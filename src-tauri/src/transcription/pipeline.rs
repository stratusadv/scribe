use super::audio::{SAMPLE_RATE_WHISPER, load_for_whisper};
use super::remote::transcribe_audio_file;
use super::types::{TranscribeProgress, Transcript, TranscriptSegment};
use crate::endpoints::storage::endpoint_load;
use crate::error::{AppError, AppResult};
use crate::notes::commands::title_generate;
use crate::people::storage::{Person, people_load_all};
use crate::workspace::{self, JobMeta};
use serde::Serialize;
use std::path::{Path, PathBuf};

const ENGINE_ID_IMPORTED: &str = "imported";
const ENGINE_ID_REMOTE_PREFIX: &str = "remote-";
const IMPORT_TEXT_BYTES_MAX: u32 = 16 << 20;
const LABEL_IMPORTED: &str = "Imported transcript";
const SOURCE_PATH_IMPORTED: &str = "(imported transcript)";
const SPELLING_HINT_CHARS_MAX: u32 = 600;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TranscriptionResult {
    pub(crate) job_id: String,
    pub(crate) transcript: Transcript,
}

pub(crate) fn audio_prepare_blocking(source_path: &Path) -> AppResult<(String, PathBuf)> {
    let job_id = workspace::job_id_from_source_content(source_path)?;
    let source_metadata = std::fs::metadata(source_path)?;
    let existing = workspace::meta_load(&job_id)?;

    debug_assert!(!job_id.is_empty());

    let label = source_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());

    let meta = audio_prepare_meta(&job_id, source_path, &source_metadata, label, existing);

    debug_assert_eq!(meta.id, job_id);

    workspace::meta_save(&meta)?;

    let audio_path = workspace::job_audio_path(&job_id)?;

    if !audio_path.exists() {
        let samples = load_for_whisper(source_path)?;

        workspace::audio_wav_save(&samples, SAMPLE_RATE_WHISPER, &audio_path)?;
    }

    debug_assert!(audio_path.exists());

    Ok((job_id, audio_path))
}

fn audio_prepare_meta(
    job_id: &str,
    source_path: &Path,
    source_metadata: &std::fs::Metadata,
    label: Option<String>,
    existing: Option<JobMeta>,
) -> JobMeta {
    let source_path_text = source_path.to_string_lossy().into_owned();
    let source_size_bytes = source_metadata.len();
    let recorded_at_unix = metadata_recorded_at_unix(source_metadata);

    let meta = match existing {
        None => JobMeta {
            id: job_id.to_owned(),
            source_path: source_path_text,
            source_size_bytes,
            created_at_unix: chrono::Utc::now().timestamp(),
            recorded_at_unix,
            label,
            title: None,
            attendees: Vec::new(),
            person_ids: Vec::new(),
            person_ids_mentioned: Vec::new(),
            project: None,
            tags: Vec::new(),
            favourite: false,
        },
        Some(previous) => JobMeta {
            id: job_id.to_owned(),
            source_path: source_path_text,
            source_size_bytes,
            created_at_unix: previous.created_at_unix,
            recorded_at_unix,
            label: previous.label.or(label),
            title: previous.title,
            attendees: previous.attendees,
            person_ids: previous.person_ids,
            person_ids_mentioned: previous.person_ids_mentioned,
            project: previous.project,
            tags: previous.tags,
            favourite: previous.favourite,
        },
    };

    debug_assert_eq!(meta.id, job_id);
    debug_assert_eq!(meta.source_size_bytes, source_size_bytes);

    meta
}

fn metadata_recorded_at_unix(metadata: &std::fs::Metadata) -> Option<i64> {
    let time = metadata.created().or_else(|_| metadata.modified()).ok()?;
    let elapsed = time.duration_since(std::time::UNIX_EPOCH).ok()?;

    i64::try_from(elapsed.as_secs()).ok()
}

pub(crate) async fn transcribe_remote_async(
    source_path: &Path,
    endpoint_id: &str,
    progress: Option<TranscribeProgress>,
) -> AppResult<TranscriptionResult> {
    let endpoint = endpoint_load(endpoint_id)?;
    let engine_id = format!("{ENGINE_ID_REMOTE_PREFIX}{endpoint_id}");

    debug_assert_eq!(endpoint.id, endpoint_id);
    debug_assert!(engine_id.len() > endpoint_id.len());

    if let Some(progress) = progress.as_ref() {
        progress.emit_stage("preparing_audio");
    }

    let source_owned = source_path.to_path_buf();

    let (job_id, audio_path) =
        crate::blocking::run(move || audio_prepare_blocking(&source_owned)).await?;

    if let Some(cached) = workspace::transcript_load::<Transcript>(&job_id, &engine_id)? {
        if let Some(progress) = progress.as_ref() {
            progress.emit_stage("done");
        }

        return Ok(TranscriptionResult { job_id, transcript: cached });
    }

    if let Some(progress) = progress.as_ref() {
        progress.emit_stage("transcribing");
    }

    let spelling_hint = transcribe_remote_spelling_hint();

    let transcript =
        transcribe_audio_file(&audio_path, &endpoint, &spelling_hint, progress.as_ref()).await?;

    debug_assert!(!job_id.is_empty());

    workspace::transcript_save(&job_id, &engine_id, &transcript)?;

    transcribe_remote_title_apply(&job_id, &transcript.text).await;

    if let Some(progress) = progress.as_ref() {
        progress.emit_stage("done");
    }

    Ok(TranscriptionResult { job_id, transcript })
}

fn transcribe_remote_spelling_hint() -> String {
    let hint = match people_load_all() {
        Ok(people) => spelling_hint_build(&people),
        Err(error) => {
            tracing::warn!("people unavailable for the spelling hint: {error}");

            String::new()
        }
    };

    debug_assert!(hint.chars().count() <= SPELLING_HINT_CHARS_MAX as usize);

    hint
}

fn spelling_hint_build(people: &[Person]) -> String {
    let mut hint = String::new();

    for person in people {
        let name = person.name_full();

        if name.is_empty() {
            continue;
        }

        let separator_length = if hint.is_empty() { 0 } else { 2 };
        let hint_length_next = hint.chars().count() + separator_length + name.chars().count();

        if hint_length_next > SPELLING_HINT_CHARS_MAX as usize {
            break;
        }

        if !hint.is_empty() {
            hint.push_str(", ");
        }

        hint.push_str(&name);
    }

    debug_assert!(hint.chars().count() <= SPELLING_HINT_CHARS_MAX as usize);
    debug_assert!(!hint.ends_with(','));

    hint
}

async fn transcribe_remote_title_apply(job_id: &str, transcript_text: &str) {
    debug_assert!(!job_id.is_empty());

    let mut meta = match workspace::meta_load(job_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            tracing::warn!("no meta for job {job_id}; the recording stays untitled");

            return;
        }
        Err(error) => {
            tracing::warn!("meta for job {job_id} unreadable: {error}; it stays untitled");

            return;
        }
    };

    if meta.title.as_deref().is_some_and(|title| !title.trim().is_empty()) {
        return;
    }

    let title = match title_generate(transcript_text).await {
        Ok(title) => title,
        Err(error) => {
            tracing::warn!("title generation failed for job {job_id}: {error}");

            return;
        }
    };

    debug_assert!(!title.is_empty());

    meta = match workspace::meta_load(job_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!("meta for job {job_id} unreadable after titling: {error}");

            return;
        }
    };

    if meta.title.as_deref().is_some_and(|title| !title.trim().is_empty()) {
        return;
    }

    meta.title = Some(title);

    if let Err(error) = workspace::meta_save(&meta) {
        tracing::warn!("title for job {job_id} could not be saved: {error}");
    }
}

pub(crate) fn transcript_import_blocking(
    title: &str,
    transcript_text: &str,
) -> AppResult<TranscriptionResult> {
    if transcript_text.len() > IMPORT_TEXT_BYTES_MAX as usize {
        return Err(AppError::Config(format!(
            "the pasted transcript is {} bytes, over the {IMPORT_TEXT_BYTES_MAX} bytes this app \
             imports",
            transcript_text.len()
        )));
    }

    let trimmed = transcript_text.trim();

    if trimmed.is_empty() {
        return Err(AppError::Config("transcript text is empty".into()));
    }

    let job_id = workspace::job_id_from_text(trimmed);
    let title_trimmed = title.trim();

    let title_clean = if title_trimmed.is_empty() {
        None
    } else {
        Some(title_trimmed.to_owned())
    };

    let existing = workspace::meta_load(&job_id)?;
    let meta = transcript_import_meta(&job_id, trimmed, title_clean, existing);

    debug_assert_eq!(meta.id, job_id);
    debug_assert_eq!(meta.source_size_bytes, trimmed.len() as u64);

    workspace::meta_save(&meta)?;

    let transcript = Transcript {
        text: trimmed.to_owned(),
        segments: transcript_import_segments(trimmed),
    };

    debug_assert!(!transcript.segments.is_empty());

    workspace::transcript_save(&job_id, ENGINE_ID_IMPORTED, &transcript)?;

    Ok(TranscriptionResult { job_id, transcript })
}

fn transcript_import_meta(
    job_id: &str,
    trimmed: &str,
    title_clean: Option<String>,
    existing: Option<JobMeta>,
) -> JobMeta {
    debug_assert!(!trimmed.is_empty());

    let source_size_bytes = trimmed.len() as u64;

    let label = title_clean
        .clone()
        .or_else(|| Some(LABEL_IMPORTED.to_owned()));

    let meta = match existing {
        None => JobMeta {
            id: job_id.to_owned(),
            source_path: SOURCE_PATH_IMPORTED.to_owned(),
            source_size_bytes,
            created_at_unix: chrono::Utc::now().timestamp(),
            recorded_at_unix: None,
            label,
            title: title_clean,
            attendees: Vec::new(),
            person_ids: Vec::new(),
            person_ids_mentioned: Vec::new(),
            project: None,
            tags: Vec::new(),
            favourite: false,
        },
        Some(previous) => JobMeta {
            id: job_id.to_owned(),
            source_path: previous.source_path,
            source_size_bytes,
            created_at_unix: previous.created_at_unix,
            recorded_at_unix: previous.recorded_at_unix,
            label: previous.label.or(label),
            title: previous.title.or(title_clean),
            attendees: previous.attendees,
            person_ids: previous.person_ids,
            person_ids_mentioned: previous.person_ids_mentioned,
            project: previous.project,
            tags: previous.tags,
            favourite: previous.favourite,
        },
    };

    debug_assert_eq!(meta.id, job_id);
    debug_assert!(meta.label.is_some());

    meta
}

fn transcript_import_segments(trimmed: &str) -> Vec<TranscriptSegment> {
    debug_assert!(!trimmed.is_empty());

    let segments: Vec<TranscriptSegment> = trimmed
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| TranscriptSegment {
            text: line.to_owned(),
            start_seconds: 0.0,
            end_seconds: 0.0,
        })
        .collect();

    if segments.is_empty() {
        return vec![TranscriptSegment {
            text: trimmed.to_owned(),
            start_seconds: 0.0,
            end_seconds: 0.0,
        }];
    }

    debug_assert!(segments.iter().all(|segment| !segment.text.is_empty()));

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person_named(name_first: &str, name_last: &str) -> Person {
        Person {
            id: name_first.to_owned(),
            name_first: name_first.to_owned(),
            name_last: name_last.to_owned(),
            role: String::new(),
            description: String::new(),
        }
    }

    #[test]
    fn the_spelling_hint_lists_names_and_stops_at_its_limit() {
        let people = vec![person_named("Brayden", "Carlson"), person_named("Jane", "Doe")];

        assert_eq!(spelling_hint_build(&people), "Brayden Carlson, Jane Doe");
        assert_eq!(spelling_hint_build(&[]), "");

        let many: Vec<Person> = (0..200)
            .map(|index| person_named(&format!("Name{index}"), "Long"))
            .collect();

        let hint = spelling_hint_build(&many);

        assert!(hint.chars().count() <= SPELLING_HINT_CHARS_MAX as usize);
        assert!(!hint.ends_with(','));
    }

    #[test]
    fn every_non_blank_line_becomes_its_own_segment() {
        let segments = transcript_import_segments("first\n\n  second  \nthird");

        assert_eq!(segments.len(), 3);
        assert_eq!(segments[1].text, "second");
    }

    #[test]
    fn a_transcript_with_no_line_breaks_is_still_one_segment() {
        let segments = transcript_import_segments("only one line");

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "only one line");
    }

    #[test]
    fn an_import_past_the_size_limit_is_refused_before_it_is_hashed() {
        let oversized = "a".repeat(IMPORT_TEXT_BYTES_MAX as usize + 1);

        assert!(transcript_import_blocking("", &oversized).is_err());
        assert!(transcript_import_blocking("", "   \n  ").is_err());
    }

    #[test]
    fn a_fresh_file_reports_a_recorded_time_near_now() {
        let path = std::env::temp_dir()
            .join(format!("scribe-recorded-at-test-{}.wav", std::process::id()));

        std::fs::write(&path, b"x").unwrap();

        let recorded_at_unix = metadata_recorded_at_unix(&std::fs::metadata(&path).unwrap());
        let now_unix = chrono::Utc::now().timestamp();

        std::fs::remove_file(&path).unwrap();

        assert!((now_unix - recorded_at_unix.unwrap()).abs() < 60);
    }
}
