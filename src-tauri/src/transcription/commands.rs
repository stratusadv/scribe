use super::pipeline::{TranscriptionResult, transcribe_remote_async, transcript_import_blocking};
use super::types::TranscribeProgress;
use crate::blocking;
use crate::cancellation::stream_id_validate;
use crate::error::AppResult;
use std::path::PathBuf;

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn transcription_remote(
    app: tauri::AppHandle,
    audio_path: String,
    endpoint_id: String,
    stream_id: Option<String>,
    transcript_reuse: bool,
) -> AppResult<TranscriptionResult> {
    if let Some(stream_id) = stream_id.as_deref() {
        stream_id_validate(stream_id)?;
    }

    let source_path = PathBuf::from(audio_path);

    let progress = stream_id.map(|id| TranscribeProgress {
        app: app.clone(),
        stream_id: id,
    });

    transcribe_remote_async(&source_path, &endpoint_id, transcript_reuse, progress).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn transcript_import(
    title: String,
    transcript_text: String,
) -> AppResult<TranscriptionResult> {
    blocking::run(move || transcript_import_blocking(&title, &transcript_text)).await
}
