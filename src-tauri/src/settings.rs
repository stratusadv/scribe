use crate::error::{AppError, AppResult};
use crate::workspace;
use serde::{Deserialize, Serialize};

pub(crate) mod commands;

const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_BYTES_MAX: u32 = 64 << 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Settings {
    #[serde(default)]
    pub(crate) notes_template_id_default: Option<String>,
    #[serde(default)]
    pub(crate) theme: Option<String>,
    #[serde(default)]
    pub(crate) palette: Option<String>,
    #[serde(default)]
    pub(crate) recordings_directory: Option<String>,
    #[serde(default)]
    pub(crate) notes_model: Option<String>,
    #[serde(default)]
    pub(crate) notes_thinking: Option<String>,
    #[serde(default)]
    pub(crate) api_host: Option<String>,
    #[serde(default)]
    pub(crate) jobs_view: Option<String>,
}

pub(crate) fn settings_load() -> AppResult<Settings> {
    let path = workspace::root_file_path(SETTINGS_FILE)?;

    if !path.exists() {
        return Ok(Settings::default());
    }

    let raw = workspace::file_read_bounded(&path, SETTINGS_BYTES_MAX)?;

    let settings = serde_json::from_str(&raw).unwrap_or_else(|error| {
        tracing::warn!(
            target: "scribe_lib::settings",
            "settings could not be read, falling back to defaults: {}",
            error
        );

        Settings::default()
    });

    Ok(settings)
}

pub(crate) fn settings_save(settings: &Settings) -> AppResult<()> {
    let path = workspace::root_file_path(SETTINGS_FILE)?;
    let encoded = serde_json::to_string_pretty(settings)?;

    if encoded.len() > SETTINGS_BYTES_MAX as usize {
        return Err(AppError::Config(format!(
            "the settings are {} bytes, over the {SETTINGS_BYTES_MAX} bytes this app keeps",
            encoded.len()
        )));
    }

    debug_assert!(encoded.starts_with('{'));
    debug_assert!(encoded.ends_with('}'));

    workspace::atomic_write(&path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_round_trip_through_json() {
        let settings = Settings {
            notes_template_id_default: Some("default-standup".to_owned()),
            theme: Some("light".to_owned()),
            palette: None,
            recordings_directory: None,
            notes_model: None,
            notes_thinking: None,
            api_host: None,
            jobs_view: None,
        };

        let raw = serde_json::to_string(&settings).expect("serialize");
        let restored: Settings = serde_json::from_str(&raw).expect("deserialize");

        assert_eq!(restored.theme.as_deref(), Some("light"));
        assert_eq!(restored.notes_template_id_default.as_deref(), Some("default-standup"));
    }

    #[test]
    fn a_settings_file_written_by_an_older_build_still_reads() {
        let restored: Settings = serde_json::from_str("{}").expect("deserialize");

        assert!(restored.theme.is_none());
        assert!(restored.notes_template_id_default.is_none());
    }
}
