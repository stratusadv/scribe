use crate::blocking;
use crate::error::{AppError, AppResult};
use crate::transcription::audio::{Waveform, waveform_compute};
use crate::transcription::types::Transcript;
use crate::workspace::{self, JobListing, JobMeta, JobMetaPatch, JobSearchHit};

#[tauri::command]
pub(crate) async fn jobs_list() -> AppResult<Vec<JobListing>> {
    blocking::run(workspace::jobs_listing_all).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_meta_get(job_id: String) -> AppResult<Option<JobMeta>> {
    blocking::run(move || workspace::meta_load(&job_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_meta_update(job_id: String, patch: JobMetaPatch) -> AppResult<JobMeta> {
    blocking::run(move || workspace::meta_apply_patch(&job_id, patch)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn jobs_search(query: String) -> AppResult<Vec<JobSearchHit>> {
    blocking::run(move || workspace::jobs_search_all(&query)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_delete(job_id: String) -> AppResult<()> {
    blocking::run(move || workspace::job_delete(&job_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_transcript_engines_list(job_id: String) -> AppResult<Vec<String>> {
    blocking::run(move || workspace::engines_for_job(&job_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_transcript_load(
    job_id: String,
    engine_id: String,
) -> AppResult<Option<Transcript>> {
    blocking::run(move || workspace::transcript_load::<Transcript>(&job_id, &engine_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_transcript_save(job_id: String, transcript: Transcript) -> AppResult<()> {
    if transcript.segments.is_empty() {
        return Err(AppError::Config("a transcript needs at least one segment".into()));
    }

    blocking::run(move || workspace::transcript_save_edited(&job_id, &transcript)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_audio_path_get(job_id: String) -> AppResult<Option<String>> {
    blocking::run(move || {
        let path = workspace::job_audio_path(&job_id)?;

        if !path.exists() {
            return Ok(None);
        }

        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn job_waveform_get(
    job_id: String,
    bar_count: u32,
) -> AppResult<Option<Waveform>> {
    blocking::run(move || {
        let path = workspace::job_audio_path(&job_id)?;

        if !path.exists() {
            return Ok(None);
        }

        Ok(Some(waveform_compute(&path, bar_count)?))
    })
    .await
}
