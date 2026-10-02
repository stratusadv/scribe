use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::fmt::Write as _;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

pub(crate) const AUDIO_HOURS_MAX: u64 = 6;
pub(crate) const AUDIO_SECONDS_MAX: u64 = AUDIO_HOURS_MAX * 60 * 60;
pub(crate) const NOTES_BYTES_MAX: u32 = 4 << 20;
pub(crate) const PCM16_SAMPLE_BYTES: u32 = 2;
const JOBS_FOLDER: &str = "jobs";
const AUDIO_FILE_NAME: &str = "audio.wav";
const CLIP_FILE_PREFIX: &str = "clip-";
const META_FILE_NAME: &str = "meta.json";
const NOTES_FILE_NAME: &str = "notes.md";
const TRANSCRIPT_FILE_PREFIX: &str = "transcript-";
const TRANSCRIPT_FILE_SUFFIX: &str = ".json";
const HASH_BUFFER_BYTES: u32 = 64 * 1024;
const JOB_ID_BYTES: usize = 16;
const JOB_ID_HEX_LENGTH: usize = JOB_ID_BYTES * 2;
const JOB_COUNT_MAX: u32 = 4096;
const ENGINE_COUNT_MAX: u32 = 16;
const META_BYTES_MAX: u32 = 64 << 10;
const TRANSCRIPT_BYTES_MAX: u32 = 64 << 20;
const SNIPPET_CHARS_MAX: u32 = 160;
const SNIPPET_CHARS_CONTEXT: u32 = 80;

const _: () = assert!(SNIPPET_CHARS_CONTEXT < SNIPPET_CHARS_MAX);
const _: () = assert!(META_BYTES_MAX < TRANSCRIPT_BYTES_MAX);
const _: () = assert!(NOTES_BYTES_MAX < TRANSCRIPT_BYTES_MAX);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JobMeta {
    pub(crate) id: String,
    pub(crate) source_path: String,
    pub(crate) source_size_bytes: u64,
    pub(crate) created_at_unix: i64,
    #[serde(default)]
    pub(crate) recorded_at_unix: Option<i64>,
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) attendees: Vec<String>,
    #[serde(default)]
    pub(crate) person_ids: Vec<String>,
    #[serde(default)]
    pub(crate) person_ids_mentioned: Vec<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) favourite: bool,
    #[serde(default)]
    pub(crate) speaker_links: Vec<SpeakerLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SpeakerLink {
    pub(crate) speaker: u16,
    pub(crate) person_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct JobListing {
    #[serde(flatten)]
    pub(crate) meta: JobMeta,
    pub(crate) has_transcript: bool,
    pub(crate) has_notes: bool,
    pub(crate) duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct JobMetaPatch {
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) attendees: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) person_ids: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) person_ids_mentioned: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) tags: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) favourite: Option<bool>,
    #[serde(default)]
    pub(crate) speaker_links: Option<Vec<SpeakerLink>>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct JobSearchHit {
    pub(crate) job_id: String,
    pub(crate) source: String,
    pub(crate) snippet: String,
}

fn speaker_links_clean(links: Vec<SpeakerLink>) -> Vec<SpeakerLink> {
    let mut cleaned: Vec<SpeakerLink> = Vec::with_capacity(links.len());

    for link in links {
        let person_id = link.person_id.trim();

        if person_id.is_empty() {
            continue;
        }

        if cleaned.iter().any(|kept| kept.speaker == link.speaker) {
            continue;
        }

        cleaned.push(SpeakerLink { speaker: link.speaker, person_id: person_id.to_owned() });
    }

    cleaned.sort_by_key(|link| link.speaker);

    debug_assert!(cleaned.is_sorted_by_key(|link| link.speaker));

    cleaned
}

pub(crate) fn meta_apply_patch(id: &str, patch: JobMetaPatch) -> AppResult<JobMeta> {
    let mut meta = meta_load(id)?
        .ok_or_else(|| AppError::Config(format!("job not found: {id}")))?;

    if meta.id != id {
        return Err(AppError::Config(format!(
            "the folder for job {id} holds metadata for job {}",
            meta.id
        )));
    }

    if let Some(title) = patch.title {
        meta.title = text_clean(&title);
    }

    if let Some(attendees) = patch.attendees {
        meta.attendees = names_clean(attendees);
    }

    if let Some(person_ids) = patch.person_ids {
        meta.person_ids = ids_clean(person_ids);
    }

    if let Some(person_ids_mentioned) = patch.person_ids_mentioned {
        meta.person_ids_mentioned = ids_clean(person_ids_mentioned);
    }

    if let Some(project) = patch.project {
        meta.project = text_clean(&project);
    }

    if let Some(tags) = patch.tags {
        meta.tags = names_clean(tags);
    }

    if let Some(favourite) = patch.favourite {
        meta.favourite = favourite;
    }

    if let Some(speaker_links) = patch.speaker_links {
        meta.speaker_links = speaker_links_clean(speaker_links);
    }

    debug_assert_eq!(meta.id, id);

    meta_save(&meta)?;

    Ok(meta)
}

fn text_clean(text: &str) -> Option<String> {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return None;
    }

    debug_assert!(!trimmed.starts_with(' '));
    debug_assert!(!trimmed.ends_with(' '));

    Some(trimmed.to_owned())
}

fn names_clean(names: Vec<String>) -> Vec<String> {
    let names_count_before = names.len();

    let cleaned: Vec<String> = names
        .into_iter()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();

    debug_assert!(cleaned.len() <= names_count_before);
    debug_assert!(cleaned.iter().all(|name| !name.is_empty()));

    cleaned
}

fn ids_clean(ids: Vec<String>) -> Vec<String> {
    let ids_count_before = ids.len();
    let mut cleaned: Vec<String> = Vec::with_capacity(ids_count_before);

    for id in ids {
        let trimmed = id.trim();

        if trimmed.is_empty() {
            continue;
        }

        if cleaned.iter().any(|kept| kept == trimmed) {
            continue;
        }

        cleaned.push(trimmed.to_owned());
    }

    debug_assert!(cleaned.len() <= ids_count_before);
    debug_assert!(cleaned.iter().all(|id| !id.is_empty()));

    cleaned
}

#[cfg(test)]
thread_local! {
    static ROOT_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) fn root() -> AppResult<PathBuf> {
    #[cfg(test)]
    if let Some(path) = ROOT_OVERRIDE.with(|root| root.borrow().clone()) {
        return Ok(path);
    }

    let data_directory = dirs::data_dir()
        .ok_or_else(|| AppError::Config("could not resolve the data directory".into()))?;

    let path = data_directory.join(crate::APP_DATA_FOLDER);

    debug_assert!(path.is_absolute());
    debug_assert!(path.ends_with(crate::APP_DATA_FOLDER));

    Ok(path)
}

pub(crate) fn root_file_path(file_name: &str) -> AppResult<PathBuf> {
    debug_assert_ne!(file_name, "");
    debug_assert!(!file_name.contains(['/', '\\']));

    let directory = root()?;

    fs::create_dir_all(&directory)?;

    let path = directory.join(file_name);

    debug_assert!(path.is_absolute());
    debug_assert!(path.ends_with(file_name));

    Ok(path)
}

pub(crate) fn jobs_directory() -> AppResult<PathBuf> {
    let path = root()?.join(JOBS_FOLDER);

    fs::create_dir_all(&path)?;

    debug_assert!(path.ends_with(JOBS_FOLDER));

    Ok(path)
}

fn job_id_from_digest(digest: &[u8]) -> String {
    debug_assert!(digest.len() >= JOB_ID_BYTES);

    let mut id = String::with_capacity(JOB_ID_HEX_LENGTH);

    for byte in digest.iter().take(JOB_ID_BYTES) {
        let _ = write!(id, "{byte:02x}");
    }

    debug_assert_eq!(id.len(), JOB_ID_HEX_LENGTH);
    debug_assert!(job_id_is_valid(&id));

    id
}

fn job_id_is_valid(id: &str) -> bool {
    if id.len() != JOB_ID_HEX_LENGTH {
        return false;
    }

    id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn job_id_validate(id: &str) -> AppResult<()> {
    if job_id_is_valid(id) {
        return Ok(());
    }

    Err(AppError::Config(format!("{id:?} is not a job id")))
}

pub(crate) fn job_id_from_source_content(source: &Path) -> AppResult<String> {
    let mut file = fs::File::open(source)?;
    let block_count_max = file.metadata()?.len().div_ceil(u64::from(HASH_BUFFER_BYTES)) + 1;
    let mut block = vec![0_u8; HASH_BUFFER_BYTES as usize];
    let mut hasher = Sha256::new();
    let mut source_bytes_hashed: u64 = 0;
    let mut source_exhausted = false;

    for _block_index in 0..block_count_max {
        let read = file.read(&mut block)?;

        if read == 0 {
            source_exhausted = true;

            break;
        }

        let block_read = block
            .get(..read)
            .ok_or_else(|| AppError::IO(std::io::Error::other("read past the hash block")))?;

        hasher.update(block_read);
        source_bytes_hashed += read as u64;
    }

    if !source_exhausted {
        return Err(AppError::IO(std::io::Error::other(format!(
            "{} grew while it was being hashed",
            source.display()
        ))));
    }

    tracing::debug!(
        target: "scribe_lib::workspace",
        "hashed {} bytes of {}",
        source_bytes_hashed,
        source.display()
    );

    Ok(job_id_from_digest(&hasher.finalize()))
}

pub(crate) fn job_id_from_text(text: &str) -> String {
    let mut hasher = Sha256::new();

    hasher.update(text.as_bytes());

    job_id_from_digest(&hasher.finalize())
}

pub(crate) fn job_directory(id: &str) -> AppResult<PathBuf> {
    job_id_validate(id)?;

    let path = jobs_directory()?.join(id);

    fs::create_dir_all(&path)?;

    debug_assert!(path.ends_with(id));

    Ok(path)
}

pub(crate) fn job_audio_path(id: &str) -> AppResult<PathBuf> {
    Ok(job_directory(id)?.join(AUDIO_FILE_NAME))
}

pub(crate) fn job_meta_path(id: &str) -> AppResult<PathBuf> {
    Ok(job_directory(id)?.join(META_FILE_NAME))
}

pub(crate) fn job_transcript_path(id: &str, engine_id: &str) -> AppResult<PathBuf> {
    let engine_safe = sanitize_segment(engine_id);

    debug_assert_eq!(engine_safe.chars().count(), engine_id.chars().count());

    let file_name = format!("{TRANSCRIPT_FILE_PREFIX}{engine_safe}{TRANSCRIPT_FILE_SUFFIX}");

    Ok(job_directory(id)?.join(file_name))
}

pub(crate) fn job_notes_path(id: &str) -> AppResult<PathBuf> {
    Ok(job_directory(id)?.join(NOTES_FILE_NAME))
}

pub(crate) fn file_read_bounded(path: &Path, size_bytes_max: u32) -> AppResult<String> {
    debug_assert!(size_bytes_max > 0);

    let size = fs::metadata(path)?.len();

    if size > u64::from(size_bytes_max) {
        return Err(AppError::Config(format!(
            "{} is {} bytes, over the {size_bytes_max} bytes this app reads",
            path.display(),
            size
        )));
    }

    let contents = fs::read_to_string(path)?;

    debug_assert!(contents.len() <= size_bytes_max as usize);

    Ok(contents)
}

pub(crate) fn meta_save(meta: &JobMeta) -> AppResult<()> {
    let path = job_meta_path(&meta.id)?;
    atomic_write(&path, serde_json::to_string_pretty(meta)?.as_bytes())
}

pub(crate) fn meta_load(id: &str) -> AppResult<Option<JobMeta>> {
    let path = job_meta_path(id)?;

    if !path.exists() {
        return Ok(None);
    }

    let meta: JobMeta = serde_json::from_str(&file_read_bounded(&path, META_BYTES_MAX)?)?;

    debug_assert_ne!(meta.id, "");

    Ok(Some(meta))
}

pub(crate) fn notes_save(id: &str, markdown: &str) -> AppResult<()> {
    if markdown.len() > NOTES_BYTES_MAX as usize {
        return Err(AppError::Config(format!(
            "these notes are {} bytes, over the {NOTES_BYTES_MAX} bytes this app keeps",
            markdown.len()
        )));
    }

    let path = job_notes_path(id)?;
    atomic_write(&path, markdown.as_bytes())
}

pub(crate) fn notes_load(id: &str) -> AppResult<Option<String>> {
    let path = job_notes_path(id)?;

    if !path.exists() {
        return Ok(None);
    }

    Ok(Some(file_read_bounded(&path, NOTES_BYTES_MAX)?))
}

pub(crate) fn sample_to_pcm16(sample: f32) -> i16 {
    // tigerstyle-ignore: TS020
    #[expect(clippy::cast_possible_truncation, reason = "clamp bounds the product to i16::MAX")]
    let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;

    debug_assert!(value > i16::MIN);
    debug_assert!(value.unsigned_abs() <= i16::MAX.unsigned_abs());

    value
}

pub(crate) fn wav_specification_mono_pcm16(sample_rate: u32) -> hound::WavSpec {
    debug_assert!(sample_rate > 0);

    let specification = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    debug_assert_eq!(u32::from(specification.bits_per_sample), PCM16_SAMPLE_BYTES * 8);

    specification
}

pub(crate) fn audio_wav_save(samples: &[f32], sample_rate: u32, path: &Path) -> AppResult<()> {
    debug_assert!(sample_rate > 0);

    if samples.len() as u64 > AUDIO_SECONDS_MAX * u64::from(sample_rate) {
        return Err(AppError::Audio(format!(
            "the audio is longer than the {AUDIO_HOURS_MAX} hours this app keeps"
        )));
    }

    let mut writer = hound::WavWriter::create(path, wav_specification_mono_pcm16(sample_rate))
        .map_err(|error| AppError::Audio(format!("wav create: {error}")))?;

    for sample in samples {
        writer
            .write_sample(sample_to_pcm16(*sample))
            .map_err(|error| AppError::Audio(format!("wav write: {error}")))?;
    }

    debug_assert_eq!(writer.len() as usize, samples.len());

    writer
        .finalize()
        .map_err(|error| AppError::Audio(format!("wav finalize: {error}")))?;

    Ok(())
}

pub(crate) fn audio_clip_save(
    path_source: &Path,
    start_seconds: f64,
    end_seconds: f64,
    path_clip: &Path,
) -> AppResult<()> {
    debug_assert!(start_seconds >= 0.0);
    debug_assert!(end_seconds > start_seconds);

    let mut reader = wav_reader_open(path_source)?;
    let specification = reader.spec();
    let rate = f64::from(specification.sample_rate);
    let length = reader.len();
    let sample_start = sample_index_clamp((start_seconds * rate).floor(), length);
    let sample_end = sample_index_clamp((end_seconds * rate).ceil(), length);

    if sample_end <= sample_start {
        return Err(AppError::Audio("the clip lies past the end of the audio".into()));
    }

    reader
        .seek(sample_start)
        .map_err(|error| AppError::Audio(format!("wav seek: {error}")))?;

    let mut writer = hound::WavWriter::create(path_clip, specification)
        .map_err(|error| AppError::Audio(format!("wav create: {error}")))?;

    for sample in reader.samples::<i16>().take((sample_end - sample_start) as usize) {
        let value = sample.map_err(|error| AppError::Audio(format!("wav read: {error}")))?;

        writer
            .write_sample(value)
            .map_err(|error| AppError::Audio(format!("wav write: {error}")))?;
    }

    writer
        .finalize()
        .map_err(|error| AppError::Audio(format!("wav finalize: {error}")))?;

    Ok(())
}

pub(crate) fn job_audio_clip_path(id: &str, start_seconds: f64, end_seconds: f64) -> AppResult<PathBuf> {
    debug_assert!(start_seconds >= 0.0);
    debug_assert!(end_seconds > start_seconds);

    let start_milliseconds = milliseconds_for(start_seconds);
    let end_milliseconds = milliseconds_for(end_seconds);
    let file_name = format!("{CLIP_FILE_PREFIX}{start_milliseconds}-{end_milliseconds}.wav");

    Ok(job_directory(id)?.join(file_name))
}

pub(crate) fn job_audio_clips_remove(id: &str) -> AppResult<()> {
    for entry in fs::read_dir(job_directory(id)?)? {
        let path = entry?.path();
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned());

        if name.is_some_and(|name| name.starts_with(CLIP_FILE_PREFIX)) {
            fs::remove_file(&path)?;
        }
    }

    Ok(())
}

fn milliseconds_for(seconds: f64) -> u64 {
    let scaled = (seconds * 1000.0).round().max(0.0);

    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "rounded and floored at zero")]
    let milliseconds = scaled as u64;

    milliseconds
}

fn sample_index_clamp(value: f64, length: u32) -> u32 {
    let scaled = value.clamp(0.0, f64::from(length));

    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamp bounds the value to 0..=length")]
    let index = scaled as u32;

    index
}

pub(crate) fn wav_reader_open(path: &Path) -> AppResult<hound::WavReader<BufReader<fs::File>>> {
    let reader = hound::WavReader::open(path)
        .map_err(|error| AppError::Audio(format!("wav open: {error}")))?;

    let spec = reader.spec();

    if spec.sample_rate == 0 {
        return Err(AppError::Audio("the recording reports a sample rate of zero".into()));
    }

    if u64::from(reader.duration()) > AUDIO_SECONDS_MAX * u64::from(spec.sample_rate) {
        return Err(AppError::Audio(format!(
            "the recording is longer than the {AUDIO_HOURS_MAX} hours this app keeps"
        )));
    }

    debug_assert!(spec.sample_rate > 0);

    Ok(reader)
}

pub(crate) fn audio_wav_duration_seconds(path: &Path) -> AppResult<f64> {
    let reader = wav_reader_open(path)?;
    let sample_rate = reader.spec().sample_rate;

    debug_assert!(sample_rate > 0);

    let duration_seconds = f64::from(reader.duration()) / f64::from(sample_rate);

    debug_assert!(duration_seconds >= 0.0);
    debug_assert!(duration_seconds.is_finite());

    Ok(duration_seconds)
}

pub(crate) fn audio_wav_load(path: &Path) -> AppResult<(Vec<f32>, u32)> {
    let mut reader = wav_reader_open(path)?;
    let sample_rate = reader.spec().sample_rate;
    let frame_count = reader.duration();

    let samples: Result<Vec<f32>, _> = reader
        .samples::<i16>()
        .map(|sample| sample.map(|value| f32::from(value) / f32::from(i16::MAX)))
        .collect();

    let samples = samples.map_err(|error| AppError::Audio(format!("wav read: {error}")))?;

    debug_assert!(sample_rate > 0);
    let samples_max = u64::from(frame_count) * u64::from(reader.spec().channels);

    debug_assert!(samples.len() as u64 <= samples_max);

    Ok((samples, sample_rate))
}

pub(crate) fn transcript_save<T: Serialize>(
    id: &str,
    engine_id: &str,
    transcript: &T,
) -> AppResult<()> {
    let path = job_transcript_path(id, engine_id)?;
    atomic_write(&path, serde_json::to_string_pretty(transcript)?.as_bytes())
}

pub(crate) fn transcript_save_edited<T: Serialize>(id: &str, transcript: &T) -> AppResult<()> {
    let engines = engines_for_job(id)?;

    let Some(engine_id) = engines.first() else {
        return Err(AppError::Config(format!("job {id} has no transcript to replace")));
    };

    let json = serde_json::to_string_pretty(transcript)?;

    if json.len() > TRANSCRIPT_BYTES_MAX as usize {
        return Err(AppError::Config(format!(
            "this transcript is {} bytes, over the {TRANSCRIPT_BYTES_MAX} bytes this app keeps",
            json.len()
        )));
    }

    let path = job_transcript_path(id, engine_id)?;
    atomic_write(&path, json.as_bytes())
}

pub(crate) fn transcript_load<T: for<'de> Deserialize<'de>>(
    id: &str,
    engine_id: &str,
) -> AppResult<Option<T>> {
    let path = job_transcript_path(id, engine_id)?;

    if !path.exists() {
        return Ok(None);
    }

    Ok(Some(serde_json::from_str(&file_read_bounded(&path, TRANSCRIPT_BYTES_MAX)?)?))
}

pub(crate) fn jobs_search_all(query: &str) -> AppResult<Vec<JobSearchHit>> {
    let needle = query.trim().to_lowercase();

    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let jobs = jobs_list_all()?;
    let job_count = jobs.len();
    let mut hits: Vec<JobSearchHit> = Vec::with_capacity(job_count);

    for job in jobs {
        let hit = search_job(&job, &needle)?;

        if let Some(hit) = hit {
            hits.push(hit);
        }
    }

    debug_assert!(hits.len() <= job_count);

    Ok(hits)
}

fn search_job(job: &JobMeta, needle: &str) -> AppResult<Option<JobSearchHit>> {
    debug_assert_ne!(needle, "");
    debug_assert_eq!(needle, needle.to_lowercase());

    let title = job.title.as_deref().or(job.label.as_deref()).unwrap_or("");

    if let Some(snippet) = search_snippet(title, needle) {
        return Ok(Some(JobSearchHit { job_id: job.id.clone(), source: "title".into(), snippet }));
    }

    if let Some(notes) = notes_load(&job.id)? {
        if let Some(snippet) = search_snippet(&notes, needle) {
            let source = "notes".into();

            return Ok(Some(JobSearchHit { job_id: job.id.clone(), source, snippet }));
        }
    }

    let engines = engines_for_job(&job.id)?;

    let Some(engine_id) = engines.first() else {
        return Ok(None);
    };

    let transcript =
        transcript_load::<crate::transcription::types::Transcript>(&job.id, engine_id)?;

    let Some(transcript) = transcript else {
        return Ok(None);
    };

    Ok(search_snippet(&transcript.text, needle).map(|snippet| JobSearchHit {
        job_id: job.id.clone(),
        source: "transcript".into(),
        snippet,
    }))
}

fn search_snippet(haystack: &str, needle: &str) -> Option<String> {
    debug_assert_ne!(needle, "");

    let line = haystack
        .lines()
        .find(|line| line.to_lowercase().contains(needle))?
        .trim();

    if line.chars().count() <= SNIPPET_CHARS_MAX as usize {
        return Some(line.to_owned());
    }

    Some(search_snippet_window(line, needle))
}

fn search_snippet_window(line: &str, needle: &str) -> String {
    debug_assert_ne!(needle, "");
    debug_assert!(line.chars().count() > SNIPPET_CHARS_MAX as usize);

    let lowered = line.to_lowercase();
    let found_bytes = lowered.find(needle).unwrap_or(0);
    let found_chars = lowered.get(..found_bytes).map_or(0, |head| head.chars().count());
    let start = found_chars.saturating_sub(SNIPPET_CHARS_CONTEXT as usize);

    let window: String = line
        .chars()
        .skip(start)
        .take(SNIPPET_CHARS_MAX as usize)
        .collect();

    debug_assert!(window.chars().count() <= SNIPPET_CHARS_MAX as usize);

    if start > 0 {
        return format!("\u{2026}{window}");
    }

    window
}

pub(crate) fn jobs_list_all() -> AppResult<Vec<JobMeta>> {
    let directory = jobs_directory()?;
    let mut jobs: Vec<JobMeta> = Vec::new();

    for (entry_index, entry) in fs::read_dir(directory)?.enumerate() {
        if entry_index >= JOB_COUNT_MAX as usize {
            return Err(AppError::Config(format!(
                "the jobs folder holds more than the {JOB_COUNT_MAX} recordings this app lists"
            )));
        }

        let entry = entry?;

        if !entry.file_type()?.is_dir() {
            continue;
        }

        let meta_path = entry.path().join(META_FILE_NAME);

        if !meta_path.exists() {
            continue;
        }

        match jobs_list_entry_load(&meta_path) {
            Ok(meta) => jobs.push(meta),
            Err(error) => {
                tracing::warn!(path = %meta_path.display(), %error, "job metadata unreadable");
            }
        }
    }

    jobs.sort_by_key(|job| std::cmp::Reverse(job.created_at_unix));

    debug_assert!(jobs.len() <= JOB_COUNT_MAX as usize);

    Ok(jobs)
}

fn jobs_list_entry_load(meta_path: &Path) -> AppResult<JobMeta> {
    debug_assert!(meta_path.ends_with(META_FILE_NAME));

    let raw = file_read_bounded(meta_path, META_BYTES_MAX)?;
    let meta: JobMeta = serde_json::from_str(&raw)?;

    debug_assert_ne!(meta.id, "");

    Ok(meta)
}

pub(crate) fn jobs_listing_all() -> AppResult<Vec<JobListing>> {
    let jobs = jobs_list_all()?;
    let job_count = jobs.len();
    let mut listings: Vec<JobListing> = Vec::with_capacity(job_count);

    for meta in jobs {
        listings.push(job_listing_build(meta)?);
    }

    debug_assert_eq!(listings.len(), job_count);

    Ok(listings)
}

fn job_listing_build(meta: JobMeta) -> AppResult<JobListing> {
    let has_transcript = !engines_for_job(&meta.id)?.is_empty();
    let notes_metadata = fs::metadata(job_notes_path(&meta.id)?);
    let has_notes = notes_metadata.is_ok_and(|metadata| metadata.len() > 0);
    let audio_path = job_audio_path(&meta.id)?;

    let duration_seconds = if audio_path.exists() {
        match audio_wav_duration_seconds(&audio_path) {
            Ok(seconds) => Some(seconds),
            Err(error) => {
                tracing::warn!(job_id = %meta.id, %error, "audio duration unreadable");
                None
            }
        }
    } else {
        None
    };

    debug_assert!(duration_seconds.is_none_or(|seconds| seconds >= 0.0));

    Ok(JobListing { meta, has_transcript, has_notes, duration_seconds })
}

pub(crate) fn job_delete(id: &str) -> AppResult<()> {
    job_id_validate(id)?;

    let directory = jobs_directory()?.join(id);

    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }

    debug_assert!(!directory.exists());

    Ok(())
}

pub(crate) fn engines_for_job(id: &str) -> AppResult<Vec<String>> {
    job_id_validate(id)?;

    let directory = jobs_directory()?.join(id);

    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut engines: Vec<String> = Vec::new();

    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();

        let Some(rest) = name_text.strip_prefix(TRANSCRIPT_FILE_PREFIX) else {
            continue;
        };

        let Some(engine) = rest.strip_suffix(TRANSCRIPT_FILE_SUFFIX) else {
            continue;
        };

        if engines.len() >= ENGINE_COUNT_MAX as usize {
            return Err(AppError::Config(format!(
                "job {id} holds more than the {ENGINE_COUNT_MAX} transcripts this app keeps"
            )));
        }

        engines.push(engine.to_owned());
    }

    engines.sort();

    debug_assert!(engines.len() <= ENGINE_COUNT_MAX as usize);

    Ok(engines)
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let path_temporary = path.with_extension(extension_temporary(path));

    debug_assert_ne!(path_temporary, path);

    fs::write(&path_temporary, contents)?;

    if let Err(error) = fs::rename(&path_temporary, path) {
        drop(fs::remove_file(&path_temporary));

        return Err(error.into());
    }

    debug_assert!(!path_temporary.exists());

    Ok(())
}

fn extension_temporary(path: &Path) -> String {
    let existing = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();

    let extension = if existing.is_empty() {
        "tmp".to_owned()
    } else {
        format!("{existing}.tmp")
    };

    debug_assert!(extension.ends_with("tmp"));

    extension
}

fn sanitize_segment(segment: &str) -> String {
    let sanitized: String = segment
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                return character;
            }

            if matches!(character, '-' | '_' | '.') {
                return character;
            }

            '_'
        })
        .collect();

    debug_assert_eq!(sanitized.chars().count(), segment.chars().count());
    debug_assert!(!sanitized.contains(['/', '\\']));

    sanitized
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{JobMeta, ROOT_OVERRIDE};
    use std::fs;
    use std::path::PathBuf;

    pub(crate) struct RootScope {
        path: PathBuf,
    }

    impl Drop for RootScope {
        fn drop(&mut self) {
            drop(ROOT_OVERRIDE.with(|root| root.borrow_mut().take()));
            drop(fs::remove_dir_all(&self.path));
        }
    }

    pub(crate) fn root_scoped(name: &str) -> RootScope {
        let path = std::env::temp_dir().join(format!("scribe-test-{}-{name}", std::process::id()));

        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(&path).unwrap();
        ROOT_OVERRIDE.with(|root| *root.borrow_mut() = Some(path.clone()));

        RootScope { path }
    }

    pub(crate) fn meta_with(id: &str, created_at_unix: i64) -> JobMeta {
        JobMeta {
            id: id.to_owned(),
            source_path: "/tmp/source.wav".to_owned(),
            source_size_bytes: 3,
            created_at_unix,
            recorded_at_unix: None,
            label: None,
            title: None,
            attendees: Vec::new(),
            person_ids: Vec::new(),
            person_ids_mentioned: Vec::new(),
            project: None,
            tags: Vec::new(),
            favourite: false,
            speaker_links: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{meta_with, root_scoped};
    use super::*;
    use crate::transcription::types::{Transcript, TranscriptSegment};

    fn transcript_with(text: &str) -> Transcript {
        Transcript {
            text: text.to_owned(),
            segments: vec![TranscriptSegment {
                text: text.to_owned(),
                start_seconds: 0.0,
                end_seconds: 1.0,
                speaker: None,
            }],
        }
    }

    #[test]
    fn the_data_folders_are_created_on_first_use() {
        let _root = root_scoped("folders");
        let jobs = jobs_directory().unwrap();
        let settings = root_file_path("settings.json").unwrap();

        assert!(jobs.is_dir());
        assert_eq!(jobs, root().unwrap().join(JOBS_FOLDER));
        assert!(settings.parent().unwrap().is_dir());
        assert!(!settings.exists());
    }

    #[test]
    fn metadata_survives_a_round_trip_through_its_job_folder() {
        let _root = root_scoped("meta-round-trip");
        let id = job_id_from_text("meta");
        let mut meta = meta_with(&id, 10);

        meta.title = Some("Kickoff".to_owned());
        meta.tags = vec!["weekly".to_owned()];
        meta.favourite = true;

        assert!(meta_load(&id).unwrap().is_none());

        meta_save(&meta).unwrap();

        let loaded = meta_load(&id).unwrap().expect("saved meta");

        assert_eq!(loaded.id, id);
        assert_eq!(loaded.title.as_deref(), Some("Kickoff"));
        assert_eq!(loaded.tags, vec!["weekly".to_owned()]);
        assert!(loaded.favourite);
        assert!(meta_load("not-a-job-id").is_err());
    }

    #[test]
    fn metadata_written_by_an_older_build_still_reads() {
        let raw = r#"{"id":"x","source_path":"p","source_size_bytes":1,"created_at_unix":2}"#;
        let meta: JobMeta = serde_json::from_str(raw).unwrap();
        let patch: JobMetaPatch = serde_json::from_str("{}").unwrap();

        assert!(meta.title.is_none());
        assert_eq!(meta.attendees.len(), 0);
        assert!(!meta.favourite);
        assert!(patch.title.is_none());
        assert!(patch.favourite.is_none());
    }

    #[test]
    fn a_listing_serializes_its_metadata_flat_beside_the_derived_fields() {
        let listing = JobListing {
            meta: meta_with("abc", 1),
            has_transcript: true,
            has_notes: false,
            duration_seconds: Some(1.5),
        };

        let value = serde_json::to_value(&listing).unwrap();

        assert_eq!(value["id"], "abc");
        assert_eq!(value["has_transcript"], true);
        assert_eq!(value["has_notes"], false);
        assert_eq!(value["duration_seconds"], 1.5);
    }

    #[test]
    fn notes_survive_a_round_trip_and_missing_notes_read_as_none() {
        let _root = root_scoped("notes-round-trip");
        let id = job_id_from_text("notes");

        assert!(notes_load(&id).unwrap().is_none());

        notes_save(&id, "# Notes\n\nCafé\n").unwrap();

        assert_eq!(notes_load(&id).unwrap().as_deref(), Some("# Notes\n\nCafé\n"));

        let oversized = "a".repeat(NOTES_BYTES_MAX as usize + 1);

        assert!(notes_save(&id, &oversized).is_err());
        assert_eq!(notes_load(&id).unwrap().as_deref(), Some("# Notes\n\nCafé\n"));
    }

    #[test]
    fn a_transcript_is_stored_per_engine_and_listed_by_engines_for_job() {
        let _root = root_scoped("transcript-round-trip");
        let id = job_id_from_text("transcript");

        assert_eq!(engines_for_job(&id).unwrap().len(), 0);
        assert!(transcript_load::<Transcript>(&id, "remote-a").unwrap().is_none());

        transcript_save(&id, "remote-b", &transcript_with("second")).unwrap();
        transcript_save(&id, "remote-a", &transcript_with("first")).unwrap();
        notes_save(&id, "notes").unwrap();

        let engines = engines_for_job(&id).unwrap();
        let loaded = transcript_load::<Transcript>(&id, "remote-a").unwrap().expect("saved");

        assert_eq!(engines, vec!["remote-a".to_owned(), "remote-b".to_owned()]);
        assert_eq!(loaded.text, "first");
        assert_eq!(loaded.segments.len(), 1);
        assert!(engines_for_job("nope").is_err());
    }

    #[test]
    fn an_edited_transcript_replaces_the_stored_one_and_needs_one_to_replace() {
        let _root = root_scoped("transcript-edited");
        let id = job_id_from_text("edited");

        assert!(transcript_save_edited(&id, &transcript_with("orphan")).is_err());

        transcript_save(&id, "imported", &transcript_with("before")).unwrap();
        transcript_save_edited(&id, &transcript_with("after")).unwrap();

        let loaded = transcript_load::<Transcript>(&id, "imported").unwrap().expect("saved");

        assert_eq!(loaded.text, "after");
        assert_eq!(engines_for_job(&id).unwrap(), vec!["imported".to_owned()]);
    }

    #[test]
    fn a_transcript_path_keeps_an_unsafe_engine_id_inside_the_job_folder() {
        let _root = root_scoped("transcript-path");
        let id = job_id_from_text("path");
        let path = job_transcript_path(&id, "../remote/x").unwrap();

        assert_eq!(path.file_name().unwrap(), "transcript-.._remote_x.json");
        assert_eq!(path.parent().unwrap(), jobs_directory().unwrap().join(&id));
    }

    #[test]
    fn jobs_are_listed_newest_first_and_folders_without_readable_metadata_are_skipped() {
        let _root = root_scoped("jobs-list");
        let ids = ["old", "new", "middle"].map(job_id_from_text);

        meta_save(&meta_with(&ids[0], 100)).unwrap();
        meta_save(&meta_with(&ids[1], 300)).unwrap();
        meta_save(&meta_with(&ids[2], 200)).unwrap();

        let jobs = jobs_directory().unwrap();

        fs::create_dir_all(jobs.join(job_id_from_text("no-meta"))).unwrap();
        fs::write(jobs.join("stray.txt"), b"x").unwrap();
        fs::write(job_meta_path(&job_id_from_text("broken")).unwrap(), b"{not json").unwrap();

        let listed: Vec<String> = jobs_list_all().unwrap().into_iter().map(|job| job.id).collect();

        assert_eq!(listed, vec![ids[1].clone(), ids[2].clone(), ids[0].clone()]);
    }

    #[test]
    fn a_listing_reports_transcript_notes_and_audio_duration_per_job() {
        let _root = root_scoped("jobs-listing");
        let full = job_id_from_text("full");
        let bare = job_id_from_text("bare");
        let samples = vec![0.0_f32; 8000];

        meta_save(&meta_with(&full, 2)).unwrap();
        meta_save(&meta_with(&bare, 1)).unwrap();
        transcript_save(&full, "imported", &transcript_with("hello")).unwrap();
        notes_save(&full, "notes").unwrap();
        notes_save(&bare, "").unwrap();
        audio_wav_save(&samples, 16000, &job_audio_path(&full).unwrap()).unwrap();
        fs::write(job_audio_path(&bare).unwrap(), b"not a wav").unwrap();

        let listings = jobs_listing_all().unwrap();

        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].meta.id, full);
        assert!(listings[0].has_transcript);
        assert!(listings[0].has_notes);
        assert!((listings[0].duration_seconds.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(listings[1].meta.id, bare);
        assert!(!listings[1].has_transcript);
        assert!(!listings[1].has_notes);
        assert!(listings[1].duration_seconds.is_none());
    }

    #[test]
    fn deleting_a_job_removes_its_folder_and_a_missing_one_is_not_an_error() {
        let _root = root_scoped("job-delete");
        let id = job_id_from_text("delete");

        meta_save(&meta_with(&id, 1)).unwrap();
        notes_save(&id, "notes").unwrap();

        let directory = jobs_directory().unwrap().join(&id);

        assert!(directory.exists());

        job_delete(&id).unwrap();

        assert!(!directory.exists());
        assert!(meta_load(&id).unwrap().is_none());

        job_delete(&id).unwrap();

        assert!(job_delete("../jobs").is_err());
    }

    #[test]
    fn a_patch_updates_only_the_fields_it_carries_and_is_refused_for_a_foreign_folder() {
        let _root = root_scoped("meta-patch");
        let id = job_id_from_text("patch");
        let foreign = job_id_from_text("foreign");
        let mut meta = meta_with(&id, 1);

        meta.title = Some("Before".to_owned());
        meta.project = Some("Website".to_owned());
        meta_save(&meta).unwrap();

        let patch = JobMetaPatch {
            title: Some("  After  ".to_owned()),
            tags: Some(vec![" weekly ".to_owned(), String::new()]),
            favourite: Some(true),
            ..JobMetaPatch::default()
        };

        let patched = meta_apply_patch(&id, patch).unwrap();
        let encoded = serde_json::to_string(&meta).unwrap();

        atomic_write(&job_meta_path(&foreign).unwrap(), encoded.as_bytes()).unwrap();

        assert_eq!(patched.title.as_deref(), Some("After"));
        assert_eq!(patched.project.as_deref(), Some("Website"));
        assert_eq!(patched.tags, vec!["weekly".to_owned()]);
        assert!(patched.favourite);
        assert_eq!(meta_load(&id).unwrap().unwrap().title.as_deref(), Some("After"));
        assert!(meta_apply_patch(&job_id_from_text("absent"), JobMetaPatch::default()).is_err());
        assert!(meta_apply_patch(&foreign, JobMetaPatch::default()).is_err());
    }

    #[test]
    fn a_search_reports_the_first_source_that_matches_per_job() {
        let _root = root_scoped("jobs-search");
        let titled = job_id_from_text("titled");
        let noted = job_id_from_text("noted");
        let spoken = job_id_from_text("spoken");
        let mut meta = meta_with(&titled, 3);

        meta.title = Some("Rollout plan".to_owned());
        meta_save(&meta).unwrap();
        meta_save(&meta_with(&noted, 2)).unwrap();
        meta_save(&meta_with(&spoken, 1)).unwrap();
        notes_save(&noted, "Line one.\nThe rollout slipped.\n").unwrap();
        transcript_save(&spoken, "imported", &transcript_with("we discussed the ROLLOUT")).unwrap();
        transcript_save(&titled, "imported", &transcript_with("rollout again")).unwrap();

        let hits = jobs_search_all(" Rollout ").unwrap();

        let found: Vec<(&str, &str)> = hits
            .iter()
            .map(|hit| (hit.job_id.as_str(), hit.source.as_str()))
            .collect();

        let expected = [
            (titled.as_str(), "title"),
            (noted.as_str(), "notes"),
            (spoken.as_str(), "transcript"),
        ];

        assert_eq!(found, expected);
        assert_eq!(hits[1].snippet, "The rollout slipped.");
        assert_eq!(jobs_search_all("absent").unwrap().len(), 0);
    }

    #[test]
    fn an_atomic_write_replaces_the_file_in_place_and_leaves_no_temporary_behind() {
        let _root = root_scoped("atomic-write");
        let path = root().unwrap().join("nested").join("notes.md");
        let blocked = root().unwrap().join("blocked");

        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        fs::create_dir_all(&blocked).unwrap();

        let names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(names, vec!["notes.md".to_owned()]);
        assert!(atomic_write(&blocked, b"x").is_err());
        assert!(!blocked.with_extension("tmp").exists());
    }

    #[test]
    fn audio_survives_a_round_trip_through_a_pcm16_wav_file() {
        let _root = root_scoped("audio-round-trip");
        let path = root().unwrap().join("audio.wav");
        let samples = [0.0_f32, 0.5, -0.5, 1.0, -1.0];

        audio_wav_save(&samples, 8000, &path).unwrap();

        let (loaded, sample_rate) = audio_wav_load(&path).unwrap();
        let duration_seconds = audio_wav_duration_seconds(&path).unwrap();

        assert_eq!(sample_rate, 8000);
        assert_eq!(loaded.len(), samples.len());
        assert!((duration_seconds - 5.0 / 8000.0).abs() < 1e-9);

        for (original, decoded) in samples.iter().zip(&loaded) {
            assert!((original - decoded).abs() < 1.0 / f32::from(i16::MAX));
        }
    }

    #[test]
    fn audio_past_the_length_gate_is_refused_and_a_non_wav_file_does_not_open() {
        let _root = root_scoped("audio-gates");
        let path = root().unwrap().join("audio.wav");
        let at_limit = vec![0.0_f32; usize::try_from(AUDIO_SECONDS_MAX).unwrap()];
        let past_limit = vec![0.0_f32; usize::try_from(AUDIO_SECONDS_MAX).unwrap() + 1];

        assert!(audio_wav_save(&past_limit, 1, &path).is_err());
        assert!(audio_wav_save(&at_limit, 1, &path).is_ok());
        assert!(wav_reader_open(&path).is_ok());

        fs::write(&path, b"RIFF but not really").unwrap();

        assert!(wav_reader_open(&path).is_err());
        assert!(audio_wav_duration_seconds(&path).is_err());
        assert!(audio_wav_load(&path).is_err());
    }

    #[test]
    fn a_source_file_hashes_to_the_same_id_as_its_text() {
        let _root = root_scoped("source-hash");
        let path = root().unwrap().join("source.txt");
        let multi_block = "x".repeat(HASH_BUFFER_BYTES as usize * 2 + 1);

        fs::write(&path, b"hello").unwrap();

        assert_eq!(job_id_from_source_content(&path).unwrap(), job_id_from_text("hello"));

        fs::write(&path, b"").unwrap();

        assert_eq!(job_id_from_source_content(&path).unwrap(), job_id_from_text(""));

        fs::write(&path, &multi_block).unwrap();

        assert_eq!(job_id_from_source_content(&path).unwrap(), job_id_from_text(&multi_block));
        assert!(job_id_from_source_content(&root().unwrap().join("missing")).is_err());
    }

    #[test]
    fn a_missing_file_is_an_error_and_an_empty_one_reads_as_empty() {
        let _root = root_scoped("bounded-read");
        let path = root().unwrap().join("empty.txt");

        assert!(file_read_bounded(&path, 1).is_err());

        fs::write(&path, b"").unwrap();

        assert_eq!(file_read_bounded(&path, 1).unwrap(), "");
    }

    #[test]
    fn names_keep_their_order_and_duplicates_while_ids_are_deduplicated() {
        let names = vec![" b ".to_owned(), "a".to_owned(), "  ".to_owned(), "a".to_owned()];
        let ids = vec!["b".to_owned(), "a".to_owned(), "b".to_owned()];

        assert_eq!(names_clean(names), vec!["b".to_owned(), "a".to_owned(), "a".to_owned()]);
        assert_eq!(ids_clean(ids), vec!["b".to_owned(), "a".to_owned()]);
        assert_eq!(names_clean(Vec::new()).len(), 0);
    }

    #[test]
    fn job_id_is_the_first_32_hex_chars_of_the_sha256() {
        let id = job_id_from_text("hello");

        assert_eq!(id, "2cf24dba5fb0a30e26e83b2ac5b9e29e");
        assert_eq!(id.len(), JOB_ID_HEX_LENGTH);
    }

    #[test]
    fn only_a_minted_job_id_shape_is_accepted_back() {
        assert!(job_id_validate("2cf24dba5fb0a30e26e83b2ac5b9e29e").is_ok());
        assert!(job_id_validate("2CF24DBA5FB0A30E26E83B2AC5B9E29E").is_ok());
        assert!(job_id_validate("").is_err());
        assert!(job_id_validate("../../etc").is_err());
        assert!(job_id_validate("2cf24dba5fb0a30e26e83b2ac5b9e29").is_err());
        assert!(job_id_validate("2cf24dba5fb0a30e26e83b2ac5b9e29g").is_err());
    }

    #[test]
    fn search_snippet_is_case_insensitive_and_line_scoped() {
        let haystack = "First line about hay.\nThe Rollout plan is on this line.\nThird line.";
        let snippet = search_snippet(haystack, "rollout").expect("hit");

        assert_eq!(snippet, "The Rollout plan is on this line.");
        assert!(search_snippet(haystack, "missing").is_none());
        assert_eq!(jobs_search_all("   ").expect("empty query").len(), 0);
    }

    #[test]
    fn a_long_line_is_windowed_around_the_match_on_character_boundaries() {
        let haystack = format!("{}Rollout{}", "é".repeat(300), "ü".repeat(300));
        let snippet = search_snippet(&haystack, "rollout").expect("hit");

        assert!(snippet.starts_with('\u{2026}'));
        assert!(snippet.contains("Rollout"));
        assert!(snippet.chars().count() <= SNIPPET_CHARS_MAX as usize + 1);
    }

    #[test]
    fn a_temporary_file_keeps_the_original_extension_in_its_name() {
        assert_eq!(extension_temporary(Path::new("notes.md")), "md.tmp");
        assert_eq!(extension_temporary(Path::new("meta.json")), "json.tmp");
        assert_eq!(extension_temporary(Path::new("endpoint_keys")), "tmp");
    }

    #[test]
    fn a_file_segment_keeps_only_safe_characters() {
        assert_eq!(sanitize_segment("remote-builtin_notes.v1"), "remote-builtin_notes.v1");
        assert_eq!(sanitize_segment("../escape"), ".._escape");
    }

    #[test]
    fn a_file_past_its_size_gate_is_refused_rather_than_read() {
        let path = std::env::temp_dir()
            .join(format!("scribe-bounded-read-test-{}.txt", std::process::id()));

        fs::write(&path, b"0123456789").unwrap();

        assert_eq!(file_read_bounded(&path, 10).unwrap(), "0123456789");
        assert!(file_read_bounded(&path, 9).is_err());

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_patch_trims_text_and_drops_empty_values() {
        assert_eq!(text_clean("  Kickoff "), Some("Kickoff".to_owned()));
        assert_eq!(text_clean("   "), None);
        assert_eq!(ids_clean(vec![" a ".into(), "a".into(), String::new()]), vec!["a".to_owned()]);
    }

    #[test]
    fn a_sample_is_scaled_into_the_symmetric_pcm16_range() {
        assert_eq!(sample_to_pcm16(1.0), i16::MAX);
        assert_eq!(sample_to_pcm16(-1.0), -i16::MAX);
        assert_eq!(sample_to_pcm16(0.0), 0);
        assert_eq!(sample_to_pcm16(2.0), i16::MAX);
    }
}
