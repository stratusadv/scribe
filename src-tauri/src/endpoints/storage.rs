use super::types::{APIEndpoint, APIEndpointPurpose, ServiceStatus, SourceStatus};
use crate::error::{AppError, AppResult};
use crate::secrets::{protected_delete, protected_read, protected_write};
use crate::settings::{Settings, settings_load};
use crate::workspace;
use std::collections::HashMap;

pub(crate) const ENDPOINT_ID_TRANSCRIPTION: &str = "builtin-transcription";
pub(crate) const ENDPOINT_ID_NOTES: &str = "builtin-notes";
pub(crate) const MODEL_TRANSCRIPTION: &str = "stratus.listen";
pub(crate) const MODEL_NOTES: &str = "stratus.thinking";
const TRANSCRIBE_CHUNK_SECONDS: u32 = 30;
const KEYS_FILE: &str = "endpoint_keys.bin";
const API_KEY_CHARS_MAX: u32 = 1024;
const API_KEY_BUILTIN_TRANSCRIPTION: Option<&str> = option_env!("SCRIBE_API_KEY_TRANSCRIPTION");
const API_KEY_BUILTIN_NOTES: Option<&str> = option_env!("SCRIBE_API_KEY_NOTES");
const API_HOST_BUILTIN: Option<&str> = option_env!("SCRIBE_API_HOST");

const _: () = assert!(TRANSCRIBE_CHUNK_SECONDS > 0);

fn keys_load() -> AppResult<HashMap<String, String>> {
    let path = workspace::root_file_path(KEYS_FILE)?;

    let Some(payload) = protected_read(&path)? else {
        return Ok(HashMap::new());
    };

    let keys = serde_json::from_str(&payload).unwrap_or_else(|error| {
        tracing::warn!(
            target: "scribe_lib::endpoints",
            "the key store could not be read and will be replaced on the next save: {}",
            error
        );

        HashMap::new()
    });

    Ok(keys)
}

fn keys_save(keys: &HashMap<String, String>) -> AppResult<()> {
    let path = workspace::root_file_path(KEYS_FILE)?;

    if keys.is_empty() {
        return protected_delete(&path);
    }

    let payload = serde_json::to_string(keys)?;

    debug_assert!(payload.starts_with('{'));

    protected_write(&path, &payload)
}

const fn endpoint_id_for(purpose: APIEndpointPurpose) -> &'static str {
    match purpose {
        APIEndpointPurpose::Transcription => ENDPOINT_ID_TRANSCRIPTION,
        APIEndpointPurpose::Notes => ENDPOINT_ID_NOTES,
    }
}

fn api_key_builtin(id: &str) -> Option<&'static str> {
    let key = match id {
        ENDPOINT_ID_TRANSCRIPTION => API_KEY_BUILTIN_TRANSCRIPTION,
        ENDPOINT_ID_NOTES => API_KEY_BUILTIN_NOTES,
        _ => None,
    };

    let usable = key.map(str::trim).filter(|key| !key.is_empty());

    debug_assert!(usable.is_none_or(|key| !key.is_empty()));
    debug_assert!(usable.is_none_or(|key| key.trim() == key));

    usable
}

fn api_key_user<'a>(id: &str, keys: &'a HashMap<String, String>) -> Option<&'a str> {
    keys.get(id).map(String::as_str).filter(|key| !key.is_empty())
}

fn api_key_resolve(id: &str, keys: &HashMap<String, String>) -> String {
    api_key_user(id, keys)
        .or_else(|| api_key_builtin(id))
        .unwrap_or_default()
        .to_owned()
}

fn api_key_status(id: &str, keys: &HashMap<String, String>) -> SourceStatus {
    SourceStatus {
        builtin: api_key_builtin(id).is_some(),
        user: api_key_user(id, keys).is_some(),
    }
}

fn host_builtin() -> Option<&'static str> {
    API_HOST_BUILTIN.map(str::trim).filter(|host| !host.is_empty())
}

fn host_user(settings: &Settings) -> Option<&str> {
    settings.api_host.as_deref().map(str::trim).filter(|host| !host.is_empty())
}

fn host_resolve(settings: &Settings) -> String {
    let host = host_user(settings).map_or_else(
        || host_builtin().unwrap_or_default().to_owned(),
        str::to_owned,
    );

    debug_assert_eq!(host.trim(), host);

    host
}

fn host_status(settings: &Settings) -> SourceStatus {
    SourceStatus { builtin: host_builtin().is_some(), user: host_user(settings).is_some() }
}

fn host_validate(host: &str) -> AppResult<()> {
    if host.is_empty() {
        return Err(AppError::Config(
            "No API address has been set. Enter one under Settings, AI.".into(),
        ));
    }

    let parsed = reqwest::Url::parse(host).map_err(|error| {
        AppError::Config(format!("The API address {host} is not a valid web address ({error})."))
    })?;

    if matches!(parsed.scheme(), "http" | "https") {
        return Ok(());
    }

    Err(AppError::Config(format!(
        "The API address {host} must start with https:// or http://."
    )))
}

fn builtin(id: &str, name: &str, purpose: APIEndpointPurpose, model: &str) -> APIEndpoint {
    debug_assert!(!id.is_empty());
    debug_assert!(!model.is_empty());

    let transcribe = purpose == APIEndpointPurpose::Transcription;

    APIEndpoint {
        id: id.to_owned(),
        name: name.to_owned(),
        purpose,
        host: String::new(),
        api_key: String::new(),
        model: model.to_owned(),
        temperature: None,
        output_tokens_max: None,
        api_path_chat: None,
        api_path_transcribe: None,
        transcribe_verbose: transcribe.then_some(false),
        transcribe_chunk_seconds: transcribe.then_some(TRANSCRIBE_CHUNK_SECONDS),
        reasoning_effort: None,
        has_api_key: false,
    }
}

fn builtins() -> [APIEndpoint; 2] {
    [
        builtin(
            ENDPOINT_ID_TRANSCRIPTION,
            "Transcription",
            APIEndpointPurpose::Transcription,
            MODEL_TRANSCRIPTION,
        ),
        builtin(ENDPOINT_ID_NOTES, "Notes", APIEndpointPurpose::Notes, MODEL_NOTES),
    ]
}

pub(crate) fn endpoints_load_all() -> AppResult<Vec<APIEndpoint>> {
    let keys = keys_load()?;
    let settings = settings_load()?;
    let mut result = builtins().to_vec();

    for endpoint in &mut result {
        endpoint.host = host_resolve(&settings);
        endpoint.has_api_key = !api_key_resolve(&endpoint.id, &keys).is_empty();
    }

    debug_assert_eq!(result.len(), builtins().len());
    debug_assert!(result.iter().all(|endpoint| endpoint.api_key.is_empty()));

    Ok(result)
}

pub(crate) fn endpoint_load(id: &str) -> AppResult<APIEndpoint> {
    let mut endpoint = builtins()
        .into_iter()
        .find(|endpoint| endpoint.id == id)
        .ok_or_else(|| AppError::Config(format!("endpoint not found: {id}")))?;

    let settings = settings_load()?;

    endpoint.host = host_resolve(&settings);
    endpoint.api_key = api_key_resolve(id, &keys_load()?);
    endpoint.has_api_key = !endpoint.api_key.is_empty();

    host_validate(&endpoint.host)?;

    if endpoint.purpose == APIEndpointPurpose::Notes {
        if let Some(model) = settings.notes_model.filter(|model| !model.trim().is_empty()) {
            endpoint.model = model;
        }

        endpoint.reasoning_effort = settings.notes_thinking.filter(|effort| !effort.is_empty());
    }

    debug_assert_eq!(endpoint.id, id);
    debug_assert!(!endpoint.host.is_empty());

    Ok(endpoint)
}

pub(crate) fn service_status_load() -> AppResult<ServiceStatus> {
    let keys = keys_load()?;
    let settings = settings_load()?;

    Ok(ServiceStatus {
        host: host_status(&settings),
        notes: api_key_status(ENDPOINT_ID_NOTES, &keys),
        transcription: api_key_status(ENDPOINT_ID_TRANSCRIPTION, &keys),
    })
}

pub(crate) fn api_key_set(purpose: APIEndpointPurpose, api_key: &str) -> AppResult<()> {
    let id = endpoint_id_for(purpose);
    let trimmed = api_key.trim();

    if trimmed.chars().count() > API_KEY_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "the key is too long; a key is under {API_KEY_CHARS_MAX} characters"
        )));
    }

    let mut keys = keys_load()?;

    if trimmed.is_empty() {
        drop(keys.remove(id));
    } else {
        drop(keys.insert(id.to_owned(), trimmed.to_owned()));
    }

    debug_assert!(keys.get(id).map_or(trimmed.is_empty(), |stored| stored == trimmed));
    debug_assert!(keys.len() <= builtins().len());

    keys_save(&keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_user_overrides_builtin() {
        let builtin = API_KEY_BUILTIN_NOTES.unwrap_or_default();
        let mut keys = HashMap::new();

        assert_eq!(api_key_resolve(ENDPOINT_ID_NOTES, &keys), builtin);

        drop(keys.insert(ENDPOINT_ID_NOTES.to_owned(), " user-key ".to_owned()));
        assert_eq!(api_key_resolve(ENDPOINT_ID_NOTES, &keys), " user-key ");

        assert_eq!(
            api_key_resolve(ENDPOINT_ID_TRANSCRIPTION, &keys).is_empty(),
            api_key_builtin(ENDPOINT_ID_TRANSCRIPTION).is_none(),
        );

        drop(keys.insert(ENDPOINT_ID_NOTES.to_owned(), String::new()));
        assert_eq!(api_key_resolve(ENDPOINT_ID_NOTES, &keys), builtin);
        assert!(!api_key_status(ENDPOINT_ID_NOTES, &keys).user);
    }

    #[test]
    fn host_user_overrides_builtin_and_a_blank_one_falls_through() {
        let mut settings = Settings::default();

        assert_eq!(host_resolve(&settings), host_builtin().unwrap_or_default());
        assert!(!host_status(&settings).user);

        settings.api_host = Some(" https://example.test ".to_owned());
        assert_eq!(host_resolve(&settings), "https://example.test");
        assert!(host_status(&settings).user);

        settings.api_host = Some("   ".to_owned());
        assert!(!host_status(&settings).user);
    }

    #[test]
    fn a_host_must_be_a_web_address() {
        assert!(host_validate("https://example.test").is_ok());
        assert!(host_validate("http://localhost:8000").is_ok());
        assert!(host_validate("").is_err());
        assert!(host_validate("example.test").is_err());
        assert!(host_validate("ftp://example.test").is_err());
    }

    #[test]
    fn every_builtin_endpoint_starts_without_a_key_or_a_host() {
        for endpoint in builtins() {
            assert!(endpoint.host.is_empty());
            assert!(endpoint.api_key.is_empty());
            assert!(!endpoint.has_api_key);
        }
    }
}
