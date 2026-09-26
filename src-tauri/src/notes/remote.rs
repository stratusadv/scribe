use crate::cancellation::{CancellationRegistry, is_cancelled};
use crate::endpoints::types::APIEndpoint;
use crate::error::{AppError, AppResult};
use crate::http::{
    ClientCell,
    ClientTimeouts,
    RESPONSE_BYTES_MAX,
    client_build,
    client_get,
    response_ensure_ok,
    response_json_bounded,
    retry_run,
};
use crate::state::AppState;
use crate::workspace::NOTES_BYTES_MAX;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tauri::{Emitter, Manager};

pub(super) const EVENT_NOTES_CHUNK: &str = "notes_generate_chunk";
const SSE_SEPARATOR: &[u8] = b"\n\n";
const SSE_DATA_PREFIX: &str = "data:";
const SSE_DONE: &str = "[DONE]";
const REQUEST_TIMEOUT_SECONDS: u64 = 900;
const READ_TIMEOUT_SECONDS: u64 = 120;
const CONNECT_TIMEOUT_SECONDS: u64 = 15;
const SSE_EVENT_BYTES_MAX: u32 = 1 << 20;
const REASONING_BYTES_MAX: u32 = 16 << 20;
const STREAM_CHUNK_COUNT_MAX: u32 = 1 << 22;
const API_PATH_MODELS: &str = "/v1/models";
const MODEL_COUNT_MAX: u32 = 256;
const LABEL_CHAT: &str = "chat completion";
const LABEL_MODELS: &str = "model list";

const HTTP_TIMEOUTS: ClientTimeouts = ClientTimeouts {
    connect: Duration::from_secs(CONNECT_TIMEOUT_SECONDS),
    read: Some(Duration::from_secs(READ_TIMEOUT_SECONDS)),
    request: Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
};

const _: () = assert!(!SSE_SEPARATOR.is_empty());
const _: () = assert!(SSE_EVENT_BYTES_MAX < NOTES_BYTES_MAX);
const _: () = assert!(NOTES_BYTES_MAX < REASONING_BYTES_MAX);
const _: () = assert!(CONNECT_TIMEOUT_SECONDS < READ_TIMEOUT_SECONDS);
const _: () = assert!(READ_TIMEOUT_SECONDS < REQUEST_TIMEOUT_SECONDS);

static HTTP_CLIENT: ClientCell = LazyLock::new(|| client_build(&HTTP_TIMEOUTS));

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct ChatChunk {
    pub(super) stream_id: String,
    pub(super) text: String,
    pub(super) done: bool,
}

#[derive(Debug, serde::Serialize)]
struct ChatMessage<'prompt> {
    role: &'static str,
    content: &'prompt str,
}

#[derive(Debug, serde::Serialize)]
struct ChatRequest<'prompt> {
    model: &'prompt str,
    messages: [ChatMessage<'prompt>; 2],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'prompt str>,
    chat_template_kwargs: ChatTemplateKwargs,
}

#[derive(Debug, serde::Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ReplyMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReplyChoice {
    #[serde(default)]
    message: Option<ReplyMessage>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ReplyChoice>,
}

struct StreamState {
    accumulated: String,
    byte_buffer: Vec<u8>,
    reasoning_bytes: u64,
}

pub(super) async fn chat_completion(
    endpoint: &APIEndpoint,
    system_prompt: &str,
    user_prompt: &str,
) -> AppResult<String> {
    let (url, body, api_key) = build_request(endpoint, system_prompt, user_prompt, false);
    let client = client_get(&HTTP_CLIENT)?;

    debug_assert!(url.starts_with("http"));
    debug_assert!(!body.model.is_empty());

    let parsed: ChatResponse = retry_run(|| async {
        let mut request = client.post(&url);

        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }

        let response = request.json(&body).send().await?;
        let response = response_ensure_ok(response, LABEL_CHAT).await?;

        response_json_bounded::<ChatResponse>(response, RESPONSE_BYTES_MAX).await
    })
    .await?;

    parsed
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message)
        .and_then(|message| message.content)
        .map(|content| content.trim().to_owned())
        .ok_or_else(|| AppError::API("no content in chat response".into()))
}

pub(super) async fn models_list(endpoint: &APIEndpoint) -> AppResult<Vec<String>> {
    let url = format!("{}{}", endpoint.host_resolved().trim_end_matches('/'), API_PATH_MODELS);
    let api_key = endpoint.api_key_resolved();
    let client = client_get(&HTTP_CLIENT)?;

    debug_assert!(url.starts_with("http"));

    let parsed: ModelsResponse = retry_run(|| async {
        let mut request = client.get(&url);

        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;
        let response = response_ensure_ok(response, LABEL_MODELS).await?;

        response_json_bounded::<ModelsResponse>(response, RESPONSE_BYTES_MAX).await
    })
    .await?;

    let mut models: Vec<String> = parsed
        .data
        .into_iter()
        .take(MODEL_COUNT_MAX as usize)
        .map(|entry| entry.id)
        .collect();

    models.sort();
    models.dedup();

    debug_assert!(models.len() <= MODEL_COUNT_MAX as usize);

    Ok(models)
}

pub(super) async fn chat_completion_streaming(
    app: tauri::AppHandle,
    stream_id: String,
    endpoint: &APIEndpoint,
    system_prompt: &str,
    user_prompt: &str,
) -> AppResult<String> {
    debug_assert!(!stream_id.is_empty());

    let (url, body, api_key) = build_request(endpoint, system_prompt, user_prompt, true);
    let client = client_get(&HTTP_CLIENT)?;

    debug_assert!(body.stream);

    let response = retry_run(|| async {
        let mut request = client.post(&url);

        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }

        let response = request.json(&body).send().await?;
        response_ensure_ok(response, LABEL_CHAT).await
    })
    .await?;

    let state = app.state::<AppState>();
    let cancellation = &state.cancellation;
    let cancel_flag = cancellation.register(&stream_id);
    let read = chat_completion_stream_read(&app, &stream_id, &cancel_flag, response).await;

    chat_completion_stream_end(&app, cancellation, stream_id);

    let accumulated = read?.trim().to_owned();

    debug_assert!(accumulated.len() <= NOTES_BYTES_MAX as usize);

    Ok(accumulated)
}

async fn chat_completion_stream_read(
    app: &tauri::AppHandle,
    stream_id: &str,
    cancel_flag: &AtomicBool,
    response: reqwest::Response,
) -> AppResult<String> {
    debug_assert!(!stream_id.is_empty());

    let mut stream = response.bytes_stream();

    let mut state = StreamState {
        accumulated: String::new(),
        byte_buffer: Vec::new(),
        reasoning_bytes: 0,
    };

    for _chunk_index in 0..STREAM_CHUNK_COUNT_MAX {
        let Some(chunk) = stream.next().await else {
            tracing::info!(
                "notes stream {stream_id} finished: {} bytes of notes after {} bytes of model \
                 reasoning",
                state.accumulated.len(),
                state.reasoning_bytes
            );

            return Ok(state.accumulated);
        };

        if is_cancelled(cancel_flag) {
            return Err(AppError::API("Cancelled".into()));
        }

        state.byte_buffer.extend_from_slice(&chunk?);

        chat_completion_stream_drain(app, stream_id, &mut state);
        chat_completion_stream_limits(&state)?;
    }

    Err(AppError::API(format!(
        "the notes service sent more than {STREAM_CHUNK_COUNT_MAX} pieces without finishing"
    )))
}

fn chat_completion_stream_limits(state: &StreamState) -> AppResult<()> {
    if state.accumulated.len() > NOTES_BYTES_MAX as usize {
        return Err(AppError::API(format!(
            "the notes service streamed more than {} MB without finishing",
            NOTES_BYTES_MAX >> 20
        )));
    }

    if state.byte_buffer.len() > SSE_EVENT_BYTES_MAX as usize {
        return Err(AppError::API(format!(
            "the notes service sent more than {} MB without ending an event",
            SSE_EVENT_BYTES_MAX >> 20
        )));
    }

    if state.reasoning_bytes > u64::from(REASONING_BYTES_MAX) {
        return Err(AppError::API(format!(
            "the notes service spent more than {} MB thinking without writing notes",
            REASONING_BYTES_MAX >> 20
        )));
    }

    Ok(())
}

fn chat_completion_stream_end(
    app: &tauri::AppHandle,
    cancellation: &CancellationRegistry,
    stream_id: String,
) {
    debug_assert!(!stream_id.is_empty());

    cancellation.clear(&stream_id);
    chunk_emit(app, ChatChunk { stream_id, text: String::new(), done: true });
}

fn chunk_emit(app: &tauri::AppHandle, chunk: ChatChunk) {
    debug_assert!(!chunk.stream_id.is_empty());

    if let Err(error) = app.emit(EVENT_NOTES_CHUNK, chunk) {
        tracing::warn!("notes chunk event could not be emitted: {}", error);
    }
}

fn chat_completion_stream_drain(app: &tauri::AppHandle, stream_id: &str, state: &mut StreamState) {
    while let Some(position) = find_subsequence(&state.byte_buffer, SSE_SEPARATOR) {
        debug_assert!(position + SSE_SEPARATOR.len() <= state.byte_buffer.len());

        let event_bytes: Vec<u8> =
            state.byte_buffer.drain(..position + SSE_SEPARATOR.len()).collect();

        debug_assert!(event_bytes.len() >= SSE_SEPARATOR.len());

        let event_text = String::from_utf8_lossy(&event_bytes);

        for line in event_text.lines() {
            let Some(payload) = line.strip_prefix(SSE_DATA_PREFIX).map(str::trim) else {
                continue;
            };

            if payload == SSE_DONE {
                continue;
            }

            if payload.is_empty() {
                continue;
            }

            chat_completion_stream_event_apply(app, stream_id, payload, state);
        }
    }
}

fn chat_completion_stream_event_apply(
    app: &tauri::AppHandle,
    stream_id: &str,
    payload: &str,
    state: &mut StreamState,
) {
    debug_assert!(!payload.is_empty());

    let parsed = match serde_json::from_str::<Value>(payload) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::debug!("notes stream {stream_id} carried an event that is not JSON: {error}");

            return;
        }
    };

    let delta = parsed
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"));

    let reasoning_text = delta
        .and_then(|delta| delta.get("reasoning_content"))
        .and_then(Value::as_str);

    if let Some(text) = reasoning_text {
        state.reasoning_bytes = state.reasoning_bytes.saturating_add(text.len() as u64);
    }

    let delta_text = delta.and_then(|delta| delta.get("content")).and_then(Value::as_str);

    if let Some(text) = delta_text {
        state.accumulated.push_str(text);

        chunk_emit(
            app,
            ChatChunk {
                stream_id: stream_id.to_owned(),
                text: text.to_owned(),
                done: false,
            },
        );
    }
}

fn build_request<'prompt>(
    endpoint: &'prompt APIEndpoint,
    system_prompt: &'prompt str,
    user_prompt: &'prompt str,
    stream: bool,
) -> (String, ChatRequest<'prompt>, &'prompt str) {
    let path = endpoint.api_path_chat_resolved();
    let url = format!("{}{}", endpoint.host_resolved().trim_end_matches('/'), path);

    debug_assert!(path.starts_with('/'));
    debug_assert!(url.ends_with(&path));

    let body = ChatRequest {
        model: endpoint.model_resolved(),
        messages: [
            ChatMessage { role: "system", content: system_prompt },
            ChatMessage { role: "user", content: user_prompt },
        ],
        stream,
        temperature: endpoint.temperature,
        max_tokens: endpoint.output_tokens_max,
        reasoning_effort: endpoint.reasoning_effort.as_deref(),
        chat_template_kwargs: ChatTemplateKwargs {
            enable_thinking: endpoint.reasoning_effort.is_some(),
        },
    };

    (url, body, endpoint.api_key_resolved())
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    debug_assert!(!needle.is_empty());

    let position = haystack
        .windows(needle.len())
        .position(|window| window == needle);

    debug_assert!(position.is_none_or(|found| found + needle.len() <= haystack.len()));

    position
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(accumulated: &str, byte_buffer: &[u8], reasoning_bytes: u64) -> StreamState {
        StreamState {
            accumulated: accumulated.to_owned(),
            byte_buffer: byte_buffer.to_vec(),
            reasoning_bytes,
        }
    }

    #[test]
    fn an_event_boundary_is_found_only_on_a_full_separator() {
        assert_eq!(find_subsequence(b"data: a\n\ndata: b", SSE_SEPARATOR), Some(7));
        assert_eq!(find_subsequence(b"data: a\n", SSE_SEPARATOR), None);
        assert_eq!(find_subsequence(b"", SSE_SEPARATOR), None);
    }

    #[test]
    fn a_stream_that_never_stops_is_refused_at_its_limit() {
        let oversized = "a".repeat(NOTES_BYTES_MAX as usize + 1);

        assert!(chat_completion_stream_limits(&state_with(&oversized, b"", 0)).is_err());
        assert!(chat_completion_stream_limits(&state_with("ok", b"partial event", 0)).is_ok());

        let unended = vec![0_u8; SSE_EVENT_BYTES_MAX as usize + 1];

        assert!(chat_completion_stream_limits(&state_with("ok", &unended, 0)).is_err());
    }

    #[test]
    fn a_model_that_only_thinks_is_refused_at_its_reasoning_limit() {
        let at_limit = state_with("", b"", u64::from(REASONING_BYTES_MAX));
        let past_limit = state_with("", b"", u64::from(REASONING_BYTES_MAX) + 1);

        assert!(chat_completion_stream_limits(&at_limit).is_ok());
        assert!(chat_completion_stream_limits(&past_limit).is_err());
    }
}
