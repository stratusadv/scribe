use super::pipeline::{TranscriptionResult, transcribe_remote_async, transcript_import_blocking};
use super::types::TranscribeProgress;
use crate::blocking;
use crate::cancellation::stream_id_validate;
use crate::error::AppResult;
use crate::settings::settings_load;
use std::path::PathBuf;
use tauri::Manager;
use tauri::path::BaseDirectory;

const SPEAKERS_RESOURCE_DIR: &str = "resources/speakers";

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

    let speakers_dir = speakers_dir_resolve(&app);

    let progress = stream_id.map(|id| TranscribeProgress {
        app: app.clone(),
        stream_id: id,
    });

    transcribe_remote_async(
        &source_path,
        &endpoint_id,
        transcript_reuse,
        speakers_dir,
        progress,
    )
    .await
}

fn speakers_dir_resolve(app: &tauri::AppHandle) -> Option<PathBuf> {
    let enabled = settings_load()
        .ok()
        .and_then(|settings| settings.speakers)
        .unwrap_or(true);

    if !enabled {
        return None;
    }

    let resolved = app.path().resolve(SPEAKERS_RESOURCE_DIR, BaseDirectory::Resource);

    match resolved {
        Ok(dir) if dir.is_dir() => Some(dir),
        Ok(dir) => {
            tracing::warn!(
                target: "scribe_lib::transcription",
                "speaker models missing at {}; transcribing without speakers",
                dir.display()
            );

            None
        }
        Err(error) => {
            tracing::warn!(
                target: "scribe_lib::transcription",
                "speaker models unresolved: {error}; transcribing without speakers"
            );

            None
        }
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn transcript_import(
    title: String,
    transcript_text: String,
) -> AppResult<TranscriptionResult> {
    blocking::run(move || transcript_import_blocking(&title, &transcript_text)).await
}
