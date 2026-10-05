use crate::cancellation::is_cancelled;
use crate::endpoints::types::APIEndpoint;
use crate::error::{AppError, AppResult};
use crate::http::{
    ClientCell,
    ClientTimeouts,
    RESPONSE_BYTES_MAX,
    RETRY_ATTEMPTS_MAX,
    client_build,
    client_get,
    response_ensure_ok,
    response_json_bounded,
    retry_run,
};
use crate::transcription::types::{
    EVENT_TRANSCRIPTION_SEGMENT,
    SegmentChunk,
    TranscribeProgress,
    Transcript,
    TranscriptSegment,
};
use crate::workspace::{
    PCM16_SAMPLE_BYTES,
    audio_wav_load,
    sample_to_pcm16,
    wav_specification_mono_pcm16,
};
use futures_util::{Stream, StreamExt};
use reqwest::multipart;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tauri::Emitter;

const REQUEST_TIMEOUT_SECONDS: u64 = 300;
const CONNECT_TIMEOUT_SECONDS: u64 = 15;
const CHUNK_CONCURRENCY_MAX: u32 = 4;
const CHUNK_COUNT_MAX: u32 = 4096;
const CHUNK_SECONDS_MIN: u32 = 2;
const CHUNK_SECONDS_TARGET: u32 = 5;
const SENTENCES_PER_CHUNK_MAX: u32 = 512;
const SENTENCE_ENDS: &[char] = &['.', '?', '!'];
const CANCEL_POLL_INTERVAL_MS: u64 = 250;
const MILLISECONDS_PER_SECOND: u64 = 1000;
const SILENCE_PEAK_MAX: f32 = 0.002;
const SPEECH_FLOOR_PERCENT: u32 = 10;
const SPEECH_FLOOR_RATIO: f32 = 1.5;
const SPEECH_FRAME_MS: u32 = 50;
const PAUSE_MS_MIN: u32 = 300;
const PAUSE_FRAMES_MIN: u32 = PAUSE_MS_MIN.div_euclid(SPEECH_FRAME_MS);
const WAV_HEADER_BYTES_ESTIMATE: u32 = 64;
const FILE_NAME_DEFAULT: &str = "audio.wav";
const LABEL_TRANSCRIBE: &str = "remote transcribe";

const HTTP_TIMEOUTS: ClientTimeouts = ClientTimeouts {
    connect: Duration::from_secs(CONNECT_TIMEOUT_SECONDS),
    read: None,
    request: Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
};

const _: () = assert!(MILLISECONDS_PER_SECOND.is_multiple_of(CANCEL_POLL_INTERVAL_MS));
const _: () = assert!(CHUNK_CONCURRENCY_MAX > 0);
const _: () = assert!(CHUNK_COUNT_MAX > CHUNK_CONCURRENCY_MAX);
const _: () = assert!(CONNECT_TIMEOUT_SECONDS < REQUEST_TIMEOUT_SECONDS);
const _: () = assert!(SPEECH_FRAME_MS > 0);
const _: () = assert!(SPEECH_FLOOR_PERCENT < 100);
const _: () = assert!(SPEECH_FLOOR_RATIO >= 1.0);
const _: () = assert!(PAUSE_FRAMES_MIN > 0);
const _: () = assert!(CHUNK_SECONDS_TARGET >= CHUNK_SECONDS_MIN);

static HTTP_CLIENT: ClientCell = LazyLock::new(|| client_build(&HTTP_TIMEOUTS));

#[derive(Debug, Deserialize)]
struct APISegment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
}

#[derive(Debug, Deserialize)]
struct APIResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Option<Vec<APISegment>>,
}

#[derive(Clone, Copy)]
struct ChunkCut {
    seconds_max: u32,
    seconds_target: u32,
}

struct ChunkPayload {
    index: u32,
    bytes: Vec<u8>,
    name: String,
    range_text: String,
}

struct SpeechFrames {
    frame_samples: u32,
    sample_count: u32,
    sample_rate: u32,
    voiced: Vec<bool>,
}

impl SpeechFrames {
    fn samples_of(&self, frames: &Range<u32>) -> Range<u32> {
        debug_assert!(frames.start <= frames.end);
        debug_assert!(frames.end as usize <= self.voiced.len());

        let sample_start = frames.start.saturating_mul(self.frame_samples);
        let sample_end = frames.end.saturating_mul(self.frame_samples).min(self.sample_count);

        sample_start..sample_end
    }
}

struct ChunkTimeline {
    frame_seconds: f64,
    offset_seconds: f64,
    seconds: f64,
    voiced_counts: Vec<u32>,
}

impl ChunkTimeline {
    fn seconds_at(&self, chars_before: u32, chars_total: u32) -> f64 {
        debug_assert!(chars_before <= chars_total);
        debug_assert_ne!(self.voiced_counts.len(), 0);

        let voiced_total = self.voiced_counts.last().copied().unwrap_or(0);

        let voiced_target = (u64::from(chars_before) * u64::from(voiced_total))
            .div_euclid(u64::from(chars_total.max(1)));

        let frame_count_before = if chars_before < chars_total {
            self.voiced_counts
                .partition_point(|count| u64::from(*count) <= voiced_target)
                .saturating_sub(1)
        } else {
            self.voiced_counts.partition_point(|count| *count < voiced_total)
        };

        let frames = u32::try_from(frame_count_before).unwrap_or(u32::MAX);
        let seconds = (f64::from(frames) * self.frame_seconds).min(self.seconds);

        debug_assert!(seconds >= 0.0);
        debug_assert!(seconds <= self.seconds);

        self.offset_seconds + seconds
    }
}

#[derive(Clone, Copy)]
struct ChunkUpload<'request> {
    url: &'request str,
    api_key: &'request str,
    model: &'request str,
    spelling_hint: &'request str,
    verbose_wanted: bool,
}

pub(super) async fn transcribe_audio_file(
    audio_path: &Path,
    endpoint: &APIEndpoint,
    spelling_hint: &str,
    progress: Option<&TranscribeProgress>,
) -> AppResult<Transcript> {
    let path = endpoint.api_path_transcribe_resolved();
    let url = format!("{}{}", endpoint.host_resolved().trim_end_matches('/'), path);
    let chunk_seconds = endpoint.transcribe_chunk_seconds.filter(|seconds| *seconds > 0);

    debug_assert!(url.starts_with("http"));
    debug_assert_ne!(endpoint.model_resolved(), "");

    let verbose_wanted = endpoint
        .transcribe_verbose
        .unwrap_or_else(|| chunk_seconds.is_none());

    let upload = ChunkUpload {
        url: &url,
        api_key: endpoint.api_key_resolved(),
        model: endpoint.model_resolved(),
        spelling_hint,
        verbose_wanted,
    };

    tracing::info!(
        target: "scribe_lib::transcription",
        "remote transcribe: url={} model={} verbose={} chunk_seconds={:?} hint_chars={}",
        upload.url,
        upload.model,
        verbose_wanted,
        chunk_seconds,
        spelling_hint.chars().count()
    );

    if let Some(chunk_seconds) = chunk_seconds {
        return transcribe_chunked(audio_path, upload, chunk_seconds, progress).await;
    }

    let file_name = audio_path
        .file_name()
        .map_or_else(|| FILE_NAME_DEFAULT.to_owned(), |name| name.to_string_lossy().into_owned());

    let file_bytes = tokio::fs::read(audio_path).await?;
    let parsed = upload_chunk(upload, &file_bytes, &file_name).await?;

    let segments: Vec<TranscriptSegment> = parsed
        .segments
        .unwrap_or_default()
        .into_iter()
        .map(|segment| TranscriptSegment {
            text: segment.text,
            start_seconds: segment.start,
            end_seconds: segment.end,
        })
        .collect();

    if let Some(progress) = progress {
        transcribe_audio_file_emit(progress, &parsed.text, &segments);
    }

    Ok(Transcript { text: parsed.text, segments })
}

fn transcribe_audio_file_emit(
    progress: &TranscribeProgress,
    parsed_text: &str,
    segments: &[TranscriptSegment],
) {
    debug_assert_ne!(progress.stream_id, "");

    if segments.is_empty() {
        let text_trimmed = parsed_text.trim();

        if !text_trimmed.is_empty() {
            progress_emit_segment(progress, text_trimmed, 0.0, 0.0);
        }

        return;
    }

    for segment in segments {
        progress_emit_segment(
            progress,
            &segment.text,
            segment.start_seconds,
            segment.end_seconds,
        );
    }
}

async fn transcribe_chunked(
    audio_path: &Path,
    upload: ChunkUpload<'_>,
    chunk_seconds: u32,
    progress: Option<&TranscribeProgress>,
) -> AppResult<Transcript> {
    debug_assert!(chunk_seconds > 0);

    let audio_path_owned = audio_path.to_path_buf();

    let (samples, sample_rate) =
        crate::blocking::run(move || audio_wav_load(&audio_path_owned)).await?;

    if samples.is_empty() {
        return Ok(Transcript { text: String::new(), segments: Vec::new() });
    }

    let seconds_target = if upload.verbose_wanted {
        chunk_seconds
    } else {
        CHUNK_SECONDS_TARGET.min(chunk_seconds)
    };

    let cut = ChunkCut { seconds_max: chunk_seconds, seconds_target };
    let speech = speech_frames_detect(&samples, sample_rate);
    let chunk_bounds = chunk_bounds_build(&speech, cut)?;

    let (chunk_payloads, chunk_indices_silent) =
        chunk_payloads_build(&samples, &speech, &chunk_bounds)?;

    let timelines = chunk_timelines_build(&speech, &chunk_bounds);

    tracing::info!(
        target: "scribe_lib::transcription",
        "remote transcribe: {} chunks of at most {}s, {}-way parallel, {} silent chunks skipped",
        chunk_payloads.len() + chunk_indices_silent.len(),
        chunk_seconds,
        CHUNK_CONCURRENCY_MAX,
        chunk_indices_silent.len()
    );

    let cancel_flag = progress.map(TranscribeProgress::cancel_register);

    let completed = transcribe_chunked_collect(
        chunk_payloads,
        chunk_indices_silent,
        upload,
        &timelines,
        progress,
        cancel_flag.as_deref(),
    )
    .await;

    if let Some(progress) = progress {
        progress.cancel_clear();
    }

    Ok(transcribe_chunked_assemble(completed?, &timelines))
}

struct ChunkCollection<'timelines> {
    chunk_count: u32,
    completed: Vec<(u32, APIResponse)>,
    completion_count: u32,
    index_next: u32,
    pending: BTreeMap<u32, APIResponse>,
    timelines: &'timelines [ChunkTimeline],
}

impl<'timelines> ChunkCollection<'timelines> {
    fn new(
        chunk_count: u32,
        chunk_indices_silent: &[u32],
        timelines: &'timelines [ChunkTimeline],
    ) -> Self {
        debug_assert!(chunk_count <= CHUNK_COUNT_MAX);
        debug_assert_eq!(timelines.len(), chunk_count as usize);

        Self {
            chunk_count,
            completed: Vec::with_capacity(chunk_count as usize),
            completion_count: u32::try_from(chunk_indices_silent.len()).unwrap_or(u32::MAX),
            index_next: 0,
            pending: transcribe_chunked_pending(chunk_indices_silent),
            timelines,
        }
    }

    fn accept(
        &mut self,
        chunk_index: u32,
        parsed: APIResponse,
        progress: Option<&TranscribeProgress>,
    ) {
        self.completion_count += 1;

        tracing::info!(
            target: "scribe_lib::transcription",
            "chunk completed {}/{} (index={})",
            self.completion_count,
            self.chunk_count,
            chunk_index
        );

        let previous = self.pending.insert(chunk_index, parsed);

        debug_assert!(previous.is_none());

        transcribe_chunked_drain(
            &mut self.pending,
            &mut self.completed,
            &mut self.index_next,
            progress,
            self.timelines,
        );
    }

    fn finish(
        mut self,
        progress: Option<&TranscribeProgress>,
    ) -> AppResult<Vec<(u32, APIResponse)>> {
        transcribe_chunked_drain(
            &mut self.pending,
            &mut self.completed,
            &mut self.index_next,
            progress,
            self.timelines,
        );

        transcribe_chunked_complete(&self.completed, self.chunk_count as usize)?;

        debug_assert_eq!(self.pending.len(), 0);

        Ok(self.completed)
    }
}

async fn transcribe_chunked_collect(
    chunk_payloads: Vec<ChunkPayload>,
    chunk_indices_silent: Vec<u32>,
    upload: ChunkUpload<'_>,
    timelines: &[ChunkTimeline],
    progress: Option<&TranscribeProgress>,
    cancel_flag: Option<&AtomicBool>,
) -> AppResult<Vec<(u32, APIResponse)>> {
    let chunk_count = chunk_payloads.len() + chunk_indices_silent.len();
    let chunk_count_bounded = u32::try_from(chunk_count).unwrap_or(CHUNK_COUNT_MAX);
    let mut collection =
        ChunkCollection::new(chunk_count_bounded, &chunk_indices_silent, timelines);
    let mut stream = transcribe_chunked_stream(chunk_payloads, upload);
    let cancel_poll = Duration::from_millis(CANCEL_POLL_INTERVAL_MS);

    for _poll in 0..transcribe_chunked_poll_count_max(chunk_count as u64) {
        if cancel_flag.is_some_and(is_cancelled) {
            return Err(AppError::Transcription("Cancelled".into()));
        }

        let (chunk, result) = match tokio::time::timeout(cancel_poll, stream.next()).await {
            Ok(Some(item)) => item,
            Ok(None) => break,
            Err(_elapsed) => continue,
        };

        let parsed = result.map_err(|error| {
            AppError::Transcription(format!(
                "the part at {} did not come back after {RETRY_ATTEMPTS_MAX} attempts: {error}",
                chunk.range_text
            ))
        })?;

        collection.accept(chunk.index, parsed, progress);
    }

    drop(stream);

    collection.finish(progress)
}

fn chunk_range_text(bound: &Range<u32>, sample_rate: u32) -> String {
    debug_assert!(sample_rate > 0);
    debug_assert!(bound.start <= bound.end);

    let start = u64::from(bound.start.div_euclid(sample_rate));
    let end = u64::from(bound.end.div_ceil(sample_rate));

    format!("{} to {}", clock_text(start), clock_text(end))
}

fn clock_text(seconds_total: u64) -> String {
    let hours = seconds_total.div_euclid(3600);
    let minutes = seconds_total.rem_euclid(3600).div_euclid(60);
    let rest = seconds_total.rem_euclid(60);

    if hours > 0 {
        return format!("{hours}:{minutes:02}:{rest:02}");
    }

    format!("{minutes}:{rest:02}")
}

fn transcribe_chunked_pending(chunk_indices_silent: &[u32]) -> BTreeMap<u32, APIResponse> {
    let mut pending: BTreeMap<u32, APIResponse> = BTreeMap::new();

    for chunk_index in chunk_indices_silent {
        let silent = APIResponse { text: String::new(), segments: None };
        let previous = pending.insert(*chunk_index, silent);

        debug_assert!(previous.is_none());
    }

    debug_assert_eq!(pending.len(), chunk_indices_silent.len());

    pending
}

fn transcribe_chunked_stream(
    chunk_payloads: Vec<ChunkPayload>,
    upload: ChunkUpload<'_>,
) -> impl Stream<Item = (ChunkPayload, AppResult<APIResponse>)> + '_ {
    debug_assert!(chunk_payloads.len() <= CHUNK_COUNT_MAX as usize);

    futures_util::stream::iter(chunk_payloads.into_iter().map(move |chunk| async move {
        let result = upload_chunk(upload, &chunk.bytes, &chunk.name).await;

        (chunk, result)
    }))
    .buffer_unordered(CHUNK_CONCURRENCY_MAX as usize)
}

fn transcribe_chunked_complete(
    completed: &[(u32, APIResponse)],
    chunk_count: usize,
) -> AppResult<()> {
    if completed.len() == chunk_count {
        return Ok(());
    }

    debug_assert!(completed.len() < chunk_count);

    Err(AppError::Transcription(format!(
        "the transcription service returned {} of {chunk_count} parts before it stopped \
         responding",
        completed.len()
    )))
}

fn transcribe_chunked_poll_count_max(chunk_count: u64) -> u64 {
    let waves = chunk_count.div_ceil(u64::from(CHUNK_CONCURRENCY_MAX));

    let attempt_seconds = (REQUEST_TIMEOUT_SECONDS + CONNECT_TIMEOUT_SECONDS)
        .saturating_mul(u64::from(RETRY_ATTEMPTS_MAX));

    let seconds = waves.saturating_mul(attempt_seconds).saturating_add(1);
    let polls_per_second = MILLISECONDS_PER_SECOND.div_euclid(CANCEL_POLL_INTERVAL_MS);
    let polls = seconds.saturating_mul(polls_per_second);

    debug_assert!(polls > 0);

    polls
}

fn transcribe_chunked_drain(
    pending: &mut BTreeMap<u32, APIResponse>,
    completed: &mut Vec<(u32, APIResponse)>,
    chunk_index_next: &mut u32,
    progress: Option<&TranscribeProgress>,
    timelines: &[ChunkTimeline],
) {
    while let Some(parsed) = pending.remove(chunk_index_next) {
        debug_assert!(*chunk_index_next < CHUNK_COUNT_MAX);

        if let Some(progress) = progress {
            if let Some(timeline) = timelines.get(*chunk_index_next as usize) {
                chunk_progress_emit(progress, &parsed, timeline);
            }
        }

        completed.push((*chunk_index_next, parsed));
        *chunk_index_next += 1;
    }

    debug_assert_eq!(completed.len(), *chunk_index_next as usize);
}

fn transcribe_chunked_assemble(
    completed: Vec<(u32, APIResponse)>,
    timelines: &[ChunkTimeline],
) -> Transcript {
    debug_assert_eq!(completed.len(), timelines.len());

    let mut text = String::new();
    let mut segments: Vec<TranscriptSegment> = Vec::with_capacity(completed.len());

    for ((_chunk_index, parsed), timeline) in completed.into_iter().zip(timelines) {
        let text_trimmed = parsed.text.trim();

        if !text_trimmed.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }

            text.push_str(text_trimmed);
        }

        let api_segments = parsed.segments.unwrap_or_default();

        if api_segments.is_empty() {
            segments.extend(segments_from_text(text_trimmed, timeline));
        } else {
            for segment in api_segments {
                segments.push(TranscriptSegment {
                    text: segment.text,
                    start_seconds: segment.start + timeline.offset_seconds,
                    end_seconds: segment.end + timeline.offset_seconds,
                });
            }
        }
    }

    debug_assert!(text.is_empty() || !segments.is_empty());

    Transcript { text, segments }
}

fn samples_peak(samples: &[f32]) -> f32 {
    let peak = samples
        .iter()
        .copied()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

    debug_assert!(peak >= 0.0);

    peak
}

fn speech_threshold(peaks: &[f32]) -> f32 {
    let mut sorted = peaks.to_vec();

    sorted.sort_unstable_by(f32::total_cmp);

    let floor_index = sorted
        .len()
        .saturating_mul(SPEECH_FLOOR_PERCENT as usize)
        .div_euclid(100);

    let floor = sorted.get(floor_index).copied().unwrap_or(0.0);
    let threshold = (floor * SPEECH_FLOOR_RATIO).max(SILENCE_PEAK_MAX);

    debug_assert!(threshold >= SILENCE_PEAK_MAX);
    debug_assert!(threshold.is_finite());

    threshold
}

fn speech_frames_detect(samples: &[f32], sample_rate: u32) -> SpeechFrames {
    debug_assert!(sample_rate > 0);

    let frame_samples_wide = (u64::from(sample_rate) * u64::from(SPEECH_FRAME_MS))
        .div_euclid(MILLISECONDS_PER_SECOND)
        .max(1);

    let frame_samples = u32::try_from(frame_samples_wide).unwrap_or(u32::MAX);
    let peaks: Vec<f32> = samples.chunks(frame_samples as usize).map(samples_peak).collect();
    let threshold = speech_threshold(&peaks);
    let voiced: Vec<bool> = peaks.iter().map(|peak| *peak >= threshold).collect();

    debug_assert_eq!(voiced.len(), samples.len().div_ceil(frame_samples as usize));

    SpeechFrames {
        frame_samples,
        sample_count: u32::try_from(samples.len()).unwrap_or(u32::MAX),
        sample_rate,
        voiced,
    }
}

fn chunk_bounds_build(speech: &SpeechFrames, cut: ChunkCut) -> AppResult<Vec<Range<u32>>> {
    debug_assert!(speech.sample_rate > 0);
    debug_assert!(cut.seconds_target <= cut.seconds_max);

    let frames_max = cut
        .seconds_max
        .saturating_mul(speech.sample_rate)
        .div_euclid(speech.frame_samples);

    if frames_max == 0 {
        return Err(AppError::Config(
            "transcribe_chunk_seconds × sample_rate computed to 0".into(),
        ));
    }

    let frames_target = cut
        .seconds_target
        .saturating_mul(speech.sample_rate)
        .div_euclid(speech.frame_samples)
        .clamp(1, frames_max);

    let frame_count = u32::try_from(speech.voiced.len()).unwrap_or(u32::MAX);
    let mut bounds: Vec<Range<u32>> = Vec::new();
    let mut frame_start: u32 = 0;

    for _chunk_index in 0..CHUNK_COUNT_MAX {
        if frame_start >= frame_count {
            break;
        }

        let frame_latest = frame_start.saturating_add(frames_max).min(frame_count);
        let frame_earliest = frame_start.saturating_add(frames_target).min(frame_latest);
        let frame_cut = chunk_cut_frame(&speech.voiced, frame_earliest, frame_latest);

        debug_assert!(frame_cut > frame_start);
        debug_assert!(frame_cut <= frame_latest);

        bounds.push(frame_start..frame_cut);
        frame_start = frame_cut;
    }

    if frame_start < frame_count {
        return Err(AppError::Audio(format!(
            "this recording holds more than the {CHUNK_COUNT_MAX} parts the app transcribes in \
             one pass"
        )));
    }

    debug_assert!(bounds.len() <= CHUNK_COUNT_MAX as usize);

    Ok(bounds)
}

fn chunk_cut_frame(voiced: &[bool], frame_earliest: u32, frame_latest: u32) -> u32 {
    debug_assert!(frame_earliest <= frame_latest);
    debug_assert!(frame_latest as usize <= voiced.len());

    let frames = (frame_earliest..).zip(voiced.iter().copied().skip(frame_earliest as usize));
    let mut pause_start: Option<u32> = None;

    for (frame, is_voiced) in frames {
        match (is_voiced, pause_start) {
            (_, None) if frame >= frame_latest => return frame_latest,
            (false, None) => pause_start = Some(frame),
            (true, Some(start)) if frame - start >= PAUSE_FRAMES_MIN => {
                return start.midpoint(frame).min(frame_latest);
            }
            (true, Some(_)) => pause_start = None,
            (true, None) | (false, Some(_)) => {}
        }
    }

    frame_latest
}

fn chunk_payloads_build(
    samples: &[f32],
    speech: &SpeechFrames,
    bounds: &[Range<u32>],
) -> AppResult<(Vec<ChunkPayload>, Vec<u32>)> {
    debug_assert!(bounds.len() <= CHUNK_COUNT_MAX as usize);
    debug_assert_eq!(samples.len(), speech.sample_count as usize);

    let sample_rate = speech.sample_rate;
    let chunk_samples_min = (sample_rate as usize).saturating_mul(CHUNK_SECONDS_MIN as usize);
    let mut chunk_payloads: Vec<ChunkPayload> = Vec::with_capacity(bounds.len());
    let mut chunk_indices_silent: Vec<u32> = Vec::new();

    for (chunk_index, bound) in (0_u32..).zip(bounds) {
        let sample_range = speech.samples_of(bound);

        let chunk_samples = samples
            .get(sample_range.start as usize..sample_range.end as usize)
            .ok_or_else(|| AppError::Audio("a part reaches past the end of the audio".into()))?;

        if samples_peak(chunk_samples) < SILENCE_PEAK_MAX {
            chunk_indices_silent.push(chunk_index);

            continue;
        }

        let bytes = if chunk_samples.len() < chunk_samples_min {
            let mut padded = Vec::with_capacity(chunk_samples_min);

            padded.extend_from_slice(chunk_samples);
            padded.resize(chunk_samples_min, 0.0);

            samples_to_wav_bytes(&padded, sample_rate)?
        } else {
            samples_to_wav_bytes(chunk_samples, sample_rate)?
        };

        let name = format!("chunk_{chunk_index:04}.wav");
        let range_text = chunk_range_text(&sample_range, sample_rate);

        chunk_payloads.push(ChunkPayload { index: chunk_index, bytes, name, range_text });
    }

    debug_assert_eq!(chunk_payloads.len() + chunk_indices_silent.len(), bounds.len());

    Ok((chunk_payloads, chunk_indices_silent))
}

fn chunk_timelines_build(speech: &SpeechFrames, bounds: &[Range<u32>]) -> Vec<ChunkTimeline> {
    debug_assert!(speech.sample_rate > 0);
    debug_assert!(bounds.len() <= CHUNK_COUNT_MAX as usize);

    bounds.iter().map(|bound| chunk_timeline_build(speech, bound)).collect()
}

fn chunk_timeline_build(speech: &SpeechFrames, frames: &Range<u32>) -> ChunkTimeline {
    debug_assert!(speech.sample_rate > 0);
    debug_assert!(frames.start < frames.end);

    let frame_count = frames.end - frames.start;
    let mut voiced_counts: Vec<u32> = Vec::with_capacity(frame_count as usize + 1);
    let mut voiced_count: u32 = 0;

    let voiced = speech
        .voiced
        .get(frames.start as usize..frames.end as usize)
        .unwrap_or_default();

    voiced_counts.push(voiced_count);

    for is_voiced in voiced {
        voiced_count += u32::from(*is_voiced);
        voiced_counts.push(voiced_count);
    }

    if voiced_count == 0 {
        voiced_counts = (0..=frame_count).collect();
    }

    debug_assert_eq!(voiced_counts.len(), frame_count as usize + 1);
    debug_assert!(voiced_counts.is_sorted());

    let sample_range = speech.samples_of(frames);

    ChunkTimeline {
        frame_seconds: f64::from(speech.frame_samples) / f64::from(speech.sample_rate),
        offset_seconds: f64::from(sample_range.start) / f64::from(speech.sample_rate),
        seconds: f64::from(sample_range.end - sample_range.start) / f64::from(speech.sample_rate),
        voiced_counts,
    }
}

fn chunk_progress_emit(
    progress: &TranscribeProgress,
    parsed: &APIResponse,
    timeline: &ChunkTimeline,
) {
    match parsed.segments.as_ref() {
        Some(api_segments) if !api_segments.is_empty() => {
            for segment in api_segments {
                progress_emit_segment(
                    progress,
                    &segment.text,
                    segment.start + timeline.offset_seconds,
                    segment.end + timeline.offset_seconds,
                );
            }
        }
        _ => {
            let sentences = segments_from_text(parsed.text.trim(), timeline);

            for sentence in &sentences {
                progress_emit_segment(
                    progress,
                    &sentence.text,
                    sentence.start_seconds,
                    sentence.end_seconds,
                );
            }
        }
    }
}

fn segments_from_text(text: &str, timeline: &ChunkTimeline) -> Vec<TranscriptSegment> {
    debug_assert!(timeline.seconds >= 0.0);

    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut ended_previous = false;

    for character in text.chars() {
        if ended_previous {
            if character.is_whitespace() {
                if sentences.len() < SENTENCES_PER_CHUNK_MAX as usize - 1 {
                    sentences.push(current.trim().to_owned());
                    current.clear();
                }
            }
        }

        current.push(character);
        ended_previous = SENTENCE_ENDS.contains(&character);
    }

    if !current.trim().is_empty() {
        sentences.push(current.trim().to_owned());
    }

    sentences.retain(|sentence| !sentence.is_empty());

    let chars_total: u32 = sentences
        .iter()
        .map(|sentence| u32::try_from(sentence.chars().count()).unwrap_or(u32::MAX))
        .fold(0_u32, u32::saturating_add);

    let mut segments: Vec<TranscriptSegment> = Vec::with_capacity(sentences.len());
    let mut chars_before: u32 = 0;

    for sentence in sentences {
        let chars = u32::try_from(sentence.chars().count()).unwrap_or(u32::MAX);
        let chars_through = chars_before.saturating_add(chars);

        segments.push(TranscriptSegment {
            text: sentence,
            start_seconds: timeline.seconds_at(chars_before, chars_total),
            end_seconds: timeline.seconds_at(chars_through, chars_total),
        });

        chars_before = chars_through;
    }

    debug_assert!(segments.len() <= SENTENCES_PER_CHUNK_MAX as usize);

    segments
}

fn progress_emit_segment(
    progress: &TranscribeProgress,
    text: &str,
    start_seconds: f64,
    end_seconds: f64,
) {
    debug_assert_ne!(progress.stream_id, "");
    debug_assert!(start_seconds <= end_seconds || end_seconds == 0.0);

    let chunk = SegmentChunk {
        stream_id: progress.stream_id.clone(),
        text: text.to_owned(),
        start_seconds,
        end_seconds,
    };

    if let Err(error) = progress.app.emit(EVENT_TRANSCRIPTION_SEGMENT, chunk) {
        tracing::warn!(
            target: "scribe_lib::transcription",
            "emit transcription_segment failed: {}",
            error
        );
    }
}

async fn upload_chunk(
    upload: ChunkUpload<'_>,
    bytes: &[u8],
    file_name: &str,
) -> AppResult<APIResponse> {
    debug_assert_ne!(file_name, "");
    debug_assert_ne!(upload.model, "");

    if bytes.is_empty() {
        return Err(AppError::Audio("the prepared audio holds no samples".into()));
    }

    let client = client_get(&HTTP_CLIENT)?;

    retry_run(|| async {
        let part = multipart::Part::bytes(bytes.to_vec())
            .file_name(file_name.to_owned())
            .mime_str("audio/wav")?;

        let mut form = multipart::Form::new()
            .text("model", upload.model.to_owned())
            .part("file", part);

        if upload.verbose_wanted {
            form = form.text("response_format", "verbose_json");
        }

        if !upload.spelling_hint.is_empty() {
            form = form.text("prompt", upload.spelling_hint.to_owned());
        }

        let mut request = client.post(upload.url);

        if !upload.api_key.is_empty() {
            request = request.bearer_auth(upload.api_key);
        }

        let response = request.multipart(form).send().await?;
        let response = response_ensure_ok(response, LABEL_TRANSCRIBE).await?;

        response_json_bounded::<APIResponse>(response, RESPONSE_BYTES_MAX).await
    })
    .await
}

fn samples_to_wav_bytes(samples: &[f32], sample_rate: u32) -> AppResult<Vec<u8>> {
    debug_assert!(sample_rate > 0);
    debug_assert_ne!(samples.len(), 0);

    let samples_bytes = samples.len() * PCM16_SAMPLE_BYTES as usize;
    let capacity = samples_bytes + WAV_HEADER_BYTES_ESTIMATE as usize;
    let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::with_capacity(capacity));

    {
        let specification = wav_specification_mono_pcm16(sample_rate);

        let mut writer = hound::WavWriter::new(&mut buffer, specification)
            .map_err(|error| AppError::Audio(format!("wav chunk writer: {error}")))?;

        for sample in samples {
            writer
                .write_sample(sample_to_pcm16(*sample))
                .map_err(|error| AppError::Audio(format!("wav chunk write: {error}")))?;
        }

        writer
            .finalize()
            .map_err(|error| AppError::Audio(format!("wav chunk finalize: {error}")))?;
    }

    let bytes = buffer.into_inner();

    debug_assert!(bytes.starts_with(b"RIFF"));
    debug_assert!(bytes.len() > samples_bytes);

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with(text: &str, segments: Option<Vec<(&str, f64, f64)>>) -> APIResponse {
        APIResponse {
            text: text.to_owned(),
            segments: segments.map(|segments| {
                segments
                    .into_iter()
                    .map(|(text, start, end)| APISegment { text: text.to_owned(), start, end })
                    .collect()
            }),
        }
    }

    fn cut_fixed(chunk_seconds: u32) -> ChunkCut {
        ChunkCut { seconds_max: chunk_seconds, seconds_target: chunk_seconds }
    }

    fn payloads_build(
        samples: &[f32],
        sample_rate: u32,
        chunk_seconds: u32,
    ) -> AppResult<(Vec<ChunkPayload>, Vec<u32>)> {
        let speech = speech_frames_detect(samples, sample_rate);
        let bounds = chunk_bounds_build(&speech, cut_fixed(chunk_seconds))?;

        chunk_payloads_build(samples, &speech, &bounds)
    }

    fn sample_bounds_build(samples: &[f32], sample_rate: u32, cut: ChunkCut) -> Vec<Range<u32>> {
        let speech = speech_frames_detect(samples, sample_rate);
        let bounds = chunk_bounds_build(&speech, cut).expect("bounds");

        bounds.iter().map(|bound| speech.samples_of(bound)).collect()
    }

    fn timeline_whole(samples: &[f32], sample_rate: u32) -> ChunkTimeline {
        let speech = speech_frames_detect(samples, sample_rate);
        let frame_count = u32::try_from(speech.voiced.len()).unwrap();

        chunk_timeline_build(&speech, &(0..frame_count))
    }

    fn timelines_build(
        samples: &[f32],
        sample_rate: u32,
        chunk_seconds: u32,
    ) -> Vec<ChunkTimeline> {
        let speech = speech_frames_detect(samples, sample_rate);
        let bounds = chunk_bounds_build(&speech, cut_fixed(chunk_seconds)).unwrap();

        chunk_timelines_build(&speech, &bounds)
    }

    fn timelines_even(chunk_count: u32, chunk_seconds: u32) -> Vec<ChunkTimeline> {
        let sample_rate = 100;
        let samples = vec![0.0_f32; (chunk_count * chunk_seconds * sample_rate) as usize];

        timelines_build(&samples, sample_rate, chunk_seconds)
    }

    #[test]
    fn assembled_chunks_are_joined_in_order_with_segment_times_shifted_by_their_offset() {
        let completed = vec![
            (0, response_with(" first ", Some(vec![("first", 0.5, 1.5)]))),
            (1, response_with("", None)),
            (2, response_with("third", None)),
            (3, response_with("  ", Some(vec![]))),
        ];

        let transcript = transcribe_chunked_assemble(completed, &timelines_even(4, 30));

        assert_eq!(transcript.text, "first third");
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0].text, "first");
        assert!((transcript.segments[0].start_seconds - 0.5).abs() < 1e-9);
        assert!((transcript.segments[0].end_seconds - 1.5).abs() < 1e-9);
        assert_eq!(transcript.segments[1].text, "third");
        assert!((transcript.segments[1].start_seconds - 60.0).abs() < 1e-9);
        assert!((transcript.segments[1].end_seconds - 90.0).abs() < 1e-9);
    }

    #[test]
    fn an_empty_set_of_chunks_assembles_into_an_empty_transcript() {
        let transcript = transcribe_chunked_assemble(Vec::new(), &[]);

        assert_eq!(transcript.text, "");
        assert_eq!(transcript.segments.len(), 0);
        assert!((timelines_even(4, 30)[3].offset_seconds - 90.0).abs() < 1e-9);
        assert!(timelines_even(4, 30)[0].offset_seconds.abs() < f64::EPSILON);
    }

    #[test]
    fn silent_chunks_are_pre_filled_and_a_drain_stops_at_the_first_gap() {
        let mut pending = transcribe_chunked_pending(&[0, 2]);
        let mut completed = Vec::new();
        let mut next = 0;
        let timelines = timelines_even(3, 30);

        assert_eq!(pending.len(), 2);
        assert_eq!(transcribe_chunked_pending(&[]).len(), 0);

        transcribe_chunked_drain(&mut pending, &mut completed, &mut next, None, &timelines);

        assert_eq!(next, 1);
        assert_eq!(completed.len(), 1);
        assert!(transcribe_chunked_complete(&completed, 3).is_err());

        drop(pending.insert(1, response_with("middle", None)));
        transcribe_chunked_drain(&mut pending, &mut completed, &mut next, None, &timelines);

        let order: Vec<u32> = completed.iter().map(|(index, _)| *index).collect();

        assert_eq!(next, 3);
        assert_eq!(pending.len(), 0);
        assert!(transcribe_chunked_complete(&completed, 3).is_ok());
        assert_eq!(order, vec![0, 1, 2]);
        assert!(transcribe_chunked_complete(&[], 0).is_ok());
    }

    #[test]
    fn a_chunk_payload_is_a_riff_wave_holding_exactly_its_samples() {
        let bytes = samples_to_wav_bytes(&[0.0, 0.5, -0.5], 16000).unwrap();

        assert!(bytes.starts_with(b"RIFF"));
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 3 * PCM16_SAMPLE_BYTES as usize);
    }

    #[test]
    fn chunks_are_named_by_index_and_a_wholly_silent_recording_uploads_nothing() {
        let sample_rate = 16000;
        let loud = vec![0.5_f32; sample_rate as usize * 4];
        let (payloads, silent) = payloads_build(&loud, sample_rate, 2).expect("build");

        assert_eq!(payloads.len(), 2);
        assert_eq!(silent.len(), 0);
        assert_eq!(payloads[0].name, "chunk_0000.wav");
        assert_eq!(payloads[1].name, "chunk_0001.wav");
        assert_eq!(payloads[1].index, 1);
        assert_eq!(payloads[1].range_text, "0:02 to 0:04");
        assert_eq!(payloads[0].bytes.len(), payloads[1].bytes.len());

        let quiet = vec![0.001_f32; sample_rate as usize * 3];
        let (none, all) = payloads_build(&quiet, sample_rate, 2).expect("build");

        assert_eq!(none.len(), 0);
        assert_eq!(all, vec![0, 1]);
        assert_eq!(payloads_build(&[], sample_rate, 2).unwrap().0.len(), 0);
    }

    #[test]
    fn a_service_reply_tolerates_missing_text_or_segments() {
        let text_only: APIResponse = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        let segments_only: APIResponse =
            serde_json::from_str(r#"{"segments":[{"text":"a","end":2.5}]}"#).unwrap();

        let segments = segments_only.segments.expect("segments");

        assert_eq!(text_only.text, "hi");
        assert!(text_only.segments.is_none());
        assert_eq!(segments_only.text, "");
        assert_eq!(segments[0].text, "a");
        assert!(segments[0].start.abs() < f64::EPSILON);
        assert!((segments[0].end - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn silent_chunks_are_skipped_not_uploaded() {
        let sample_rate = 16000;
        let samples_per_chunk = sample_rate as usize * 30;
        let mut samples = vec![0.0_f32; samples_per_chunk * 2 + sample_rate as usize * 9];

        for sample in &mut samples[..samples_per_chunk] {
            *sample = 0.5;
        }

        for sample in &mut samples[samples_per_chunk..samples_per_chunk * 2] {
            *sample = -0.001;
        }

        let (payloads, silent) = payloads_build(&samples, sample_rate, 30).expect("build");
        let uploaded: Vec<u32> = payloads.iter().map(|chunk| chunk.index).collect();

        assert_eq!(uploaded, vec![0]);
        assert_eq!(silent, vec![1, 2]);
    }

    #[test]
    fn chunk_text_splits_into_sentences_with_proportional_times() {
        let timelines = timelines_build(&vec![0.0_f32; 12_400], 100, 100);
        let timeline = &timelines[1];
        let segments = segments_from_text("One two. Three four? Five!", timeline);

        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "One two.");
        assert_eq!(segments[1].text, "Three four?");
        assert_eq!(segments[2].text, "Five!");
        assert!((segments[0].start_seconds - 100.0).abs() < 1e-9);
        assert!((segments[0].end_seconds - 108.0).abs() < 1e-9);
        assert!((segments[1].start_seconds - 108.0).abs() < 1e-9);
        assert!((segments[2].end_seconds - 124.0).abs() < 1e-9);
        assert_eq!(segments_from_text("   ", timeline).len(), 0);
        assert_eq!(segments_from_text("no terminator here", timeline).len(), 1);
        assert_eq!(segments_from_text("It is 10.30 now. Ok.", timeline).len(), 2);
    }

    #[test]
    fn sentence_times_skip_the_silence_inside_a_chunk() {
        let sample_rate = 100;
        let mut samples = vec![0.0_f32; 1000];

        for sample in &mut samples[..200] {
            *sample = 0.5;
        }

        for sample in &mut samples[800..] {
            *sample = -0.5;
        }

        let timeline = timeline_whole(&samples, sample_rate);
        let segments = segments_from_text("Aaaa. Bbbb.", &timeline);

        assert_eq!(segments.len(), 2);
        assert!(segments[0].start_seconds.abs() < 1e-9);
        assert!((segments[0].end_seconds - 8.0).abs() < 1e-9);
        assert!((segments[1].start_seconds - 8.0).abs() < 1e-9);
        assert!((segments[1].end_seconds - 10.0).abs() < 1e-9);

        let quiet_start = timeline_whole(&samples[200..], sample_rate);

        assert!((quiet_start.seconds_at(0, 10) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_recording_is_cut_at_the_pause_that_follows_the_target_duration() {
        let sample_rate = 100;
        let mut samples = vec![0.5_f32; 2000];

        for sample in &mut samples[300..400] {
            *sample = 0.0;
        }

        for sample in &mut samples[700..800] {
            *sample = 0.0;
        }

        for sample in &mut samples[1200..1210] {
            *sample = 0.0;
        }

        let cut = ChunkCut { seconds_max: 120, seconds_target: 5 };
        let bounds = sample_bounds_build(&samples, sample_rate, cut);
        let capped = ChunkCut { seconds_max: 6, seconds_target: 5 };
        let bounds_capped = sample_bounds_build(&samples, sample_rate, capped);
        let bounds_fixed = sample_bounds_build(&samples, sample_rate, cut_fixed(8));

        assert_eq!(bounds, vec![0..750, 750..2000]);
        assert_eq!(bounds_capped, vec![0..600, 600..1200, 1200..1800, 1800..2000]);
        assert_eq!(bounds_fixed, vec![0..800, 800..1600, 1600..2000]);
    }

    #[test]
    fn speech_is_judged_against_the_floor_of_the_recording() {
        let sample_rate = 100;
        let mut samples = vec![0.01_f32; 2000];

        for sample in &mut samples[..500] {
            *sample = 0.5;
        }

        for sample in &mut samples[800..] {
            *sample = -0.5;
        }

        let cut = ChunkCut { seconds_max: 120, seconds_target: 5 };
        let speech = speech_frames_detect(&samples, sample_rate);

        assert_eq!(speech.voiced.iter().filter(|voiced| !**voiced).count(), 60);
        assert_eq!(sample_bounds_build(&samples, sample_rate, cut), vec![0..650, 650..2000]);
        assert!((speech_threshold(&[0.0, 0.0, 0.5]) - SILENCE_PEAK_MAX).abs() < f32::EPSILON);
        assert!(speech_threshold(&[]) >= SILENCE_PEAK_MAX);

        let steady = vec![0.5_f32; 1000];
        let steady_speech = speech_frames_detect(&steady, sample_rate);
        let steady_timeline = timeline_whole(&steady, sample_rate);

        assert!(steady_speech.voiced.iter().all(|voiced| !*voiced));
        assert!((steady_timeline.seconds_at(5, 10) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn the_last_sentence_of_a_part_ends_with_its_last_sound() {
        let sample_rate = 100;
        let mut samples = vec![0.0_f32; 1000];

        for sample in &mut samples[100..400] {
            *sample = 0.5;
        }

        let timeline = timeline_whole(&samples, sample_rate);
        let segments = segments_from_text("Aaaa. Bbbb.", &timeline);

        assert_eq!(segments.len(), 2);
        assert!((segments[0].start_seconds - 1.0).abs() < 1e-9);
        assert!((segments[1].start_seconds - 2.5).abs() < 1e-9);
        assert!((segments[1].end_seconds - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_short_final_chunk_is_timed_over_its_own_duration() {
        let sample_rate = 100;
        let samples = vec![0.5_f32; 4500];
        let timelines = timelines_build(&samples, sample_rate, 30);
        let completed = vec![
            (0, response_with("One.", None)),
            (1, response_with("Two. Three.", None)),
        ];

        let transcript = transcribe_chunked_assemble(completed, &timelines);

        assert_eq!(timelines.len(), 2);
        assert!((timelines[1].offset_seconds - 30.0).abs() < 1e-9);
        assert!((timelines[1].seconds - 15.0).abs() < 1e-9);
        assert_eq!(transcript.segments.len(), 3);
        assert!((transcript.segments[0].end_seconds - 30.0).abs() < 1e-9);
        assert!((transcript.segments[1].start_seconds - 30.0).abs() < 1e-9);
        assert!((transcript.segments[2].end_seconds - 45.0).abs() < 1e-9);
    }

    #[test]
    fn a_chunk_range_reads_as_a_clock_span_and_grows_hours_when_needed() {
        assert_eq!(chunk_range_text(&(216_000..219_000), 100), "36:00 to 36:30");
        assert_eq!(chunk_range_text(&(0..3000), 100), "0:00 to 0:30");
        assert_eq!(chunk_range_text(&(360_000..363_000), 100), "1:00:00 to 1:00:30");
        assert_eq!(chunk_range_text(&(150..649), 100), "0:01 to 0:07");
        assert_eq!(clock_text(3599), "59:59");
    }

    #[test]
    fn a_short_chunk_is_padded_up_to_the_service_minimum() {
        let sample_rate = 16000;
        let samples = vec![0.5_f32; 100];
        let (payloads, _) = payloads_build(&samples, sample_rate, 1).expect("build");
        let chunk_samples_min = sample_rate as usize * CHUNK_SECONDS_MIN as usize;
        let payload_bytes_min = chunk_samples_min * PCM16_SAMPLE_BYTES as usize;

        assert_eq!(payloads.len(), 1);
        assert!(payloads[0].bytes.len() > payload_bytes_min);
    }

    #[test]
    fn a_recording_past_the_chunk_limit_is_refused_before_any_upload() {
        let sample_rate = 16;
        let samples = vec![0.5_f32; sample_rate as usize * (CHUNK_COUNT_MAX as usize + 1)];

        assert!(payloads_build(&samples, sample_rate, 1).is_err());
    }

    #[test]
    fn the_poll_budget_covers_every_wave_of_chunks() {
        let one_wave = transcribe_chunked_poll_count_max(u64::from(CHUNK_CONCURRENCY_MAX));
        let two_waves = transcribe_chunked_poll_count_max(u64::from(CHUNK_CONCURRENCY_MAX) + 1);
        let polls_per_second = MILLISECONDS_PER_SECOND.div_euclid(CANCEL_POLL_INTERVAL_MS);

        assert!(two_waves > one_wave);
        assert!(one_wave >= REQUEST_TIMEOUT_SECONDS * polls_per_second);
    }
}
