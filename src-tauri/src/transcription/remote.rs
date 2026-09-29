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
const SENTENCES_PER_CHUNK_MAX: u32 = 512;
const SENTENCE_ENDS: &[char] = &['.', '?', '!'];
const CANCEL_POLL_INTERVAL_MS: u64 = 250;
const MILLISECONDS_PER_SECOND: u64 = 1000;
const SILENCE_PEAK_MAX: f32 = 0.002;
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

struct ChunkPayload {
    index: u32,
    bytes: Vec<u8>,
    name: String,
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
    debug_assert!(!endpoint.model_resolved().is_empty());

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
    debug_assert!(!progress.stream_id.is_empty());

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

fn chunk_offset_seconds(chunk_index: u32, chunk_seconds: f64) -> f64 {
    debug_assert!(chunk_seconds > 0.0);

    let offset_seconds = f64::from(chunk_index) * chunk_seconds;

    debug_assert!(offset_seconds >= 0.0);
    debug_assert!(offset_seconds.is_finite());

    offset_seconds
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

    let (chunk_payloads, chunk_indices_silent) =
        chunk_payloads_build(&samples, sample_rate, chunk_seconds)?;

    tracing::info!(
        target: "scribe_lib::transcription",
        "remote transcribe: {} chunks of {}s each, {}-way parallel, {} silent chunks skipped",
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
        chunk_seconds,
        progress,
        cancel_flag.as_deref(),
    )
    .await;

    if let Some(progress) = progress {
        progress.cancel_clear();
    }

    Ok(transcribe_chunked_assemble(completed?, f64::from(chunk_seconds)))
}

struct ChunkCollection {
    chunk_count: u32,
    chunk_seconds: u32,
    completed: Vec<(u32, APIResponse)>,
    completion_count: u32,
    index_next: u32,
    pending: BTreeMap<u32, APIResponse>,
}

impl ChunkCollection {
    fn new(chunk_count: u32, chunk_seconds: u32, chunk_indices_silent: &[u32]) -> Self {
        debug_assert!(chunk_seconds > 0);
        debug_assert!(chunk_count <= CHUNK_COUNT_MAX);

        Self {
            chunk_count,
            chunk_seconds,
            completed: Vec::with_capacity(chunk_count as usize),
            completion_count: u32::try_from(chunk_indices_silent.len()).unwrap_or(u32::MAX),
            index_next: 0,
            pending: transcribe_chunked_pending(chunk_indices_silent),
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
            f64::from(self.chunk_seconds),
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
            f64::from(self.chunk_seconds),
        );

        transcribe_chunked_complete(&self.completed, self.chunk_count as usize)?;

        debug_assert!(self.pending.is_empty());

        Ok(self.completed)
    }
}

async fn transcribe_chunked_collect(
    chunk_payloads: Vec<ChunkPayload>,
    chunk_indices_silent: Vec<u32>,
    upload: ChunkUpload<'_>,
    chunk_seconds: u32,
    progress: Option<&TranscribeProgress>,
    cancel_flag: Option<&AtomicBool>,
) -> AppResult<Vec<(u32, APIResponse)>> {
    let chunk_count = chunk_payloads.len() + chunk_indices_silent.len();
    let chunk_count_bounded = u32::try_from(chunk_count).unwrap_or(CHUNK_COUNT_MAX);
    let mut collection =
        ChunkCollection::new(chunk_count_bounded, chunk_seconds, &chunk_indices_silent);
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
                chunk_range_text(chunk.index, chunk_seconds)
            ))
        })?;

        collection.accept(chunk.index, parsed, progress);
    }

    drop(stream);

    collection.finish(progress)
}

fn chunk_range_text(chunk_index: u32, chunk_seconds: u32) -> String {
    debug_assert!(chunk_seconds > 0);

    let start = u64::from(chunk_index) * u64::from(chunk_seconds);
    let end = start + u64::from(chunk_seconds);

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
    chunk_seconds: f64,
) {
    while let Some(parsed) = pending.remove(chunk_index_next) {
        debug_assert!(*chunk_index_next < CHUNK_COUNT_MAX);

        if let Some(progress) = progress {
            chunk_progress_emit(progress, *chunk_index_next, &parsed, chunk_seconds);
        }

        completed.push((*chunk_index_next, parsed));
        *chunk_index_next += 1;
    }

    debug_assert_eq!(completed.len(), *chunk_index_next as usize);
}

fn transcribe_chunked_assemble(
    completed: Vec<(u32, APIResponse)>,
    chunk_seconds: f64,
) -> Transcript {
    debug_assert!(chunk_seconds > 0.0);

    let mut text = String::new();
    let mut segments: Vec<TranscriptSegment> = Vec::with_capacity(completed.len());

    for (chunk_index, parsed) in completed {
        let offset = chunk_offset_seconds(chunk_index, chunk_seconds);
        let text_trimmed = parsed.text.trim();

        if !text_trimmed.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }

            text.push_str(text_trimmed);
        }

        let api_segments = parsed.segments.unwrap_or_default();

        if api_segments.is_empty() {
            segments.extend(segments_from_text(text_trimmed, offset, offset + chunk_seconds));
        } else {
            for segment in api_segments {
                segments.push(TranscriptSegment {
                    text: segment.text,
                    start_seconds: segment.start + offset,
                    end_seconds: segment.end + offset,
                });
            }
        }
    }

    debug_assert!(text.is_empty() || !segments.is_empty());

    Transcript { text, segments }
}

fn chunk_payloads_build(
    samples: &[f32],
    sample_rate: u32,
    chunk_seconds: u32,
) -> AppResult<(Vec<ChunkPayload>, Vec<u32>)> {
    debug_assert!(sample_rate > 0);
    debug_assert!(chunk_seconds > 0);

    let samples_per_chunk = (chunk_seconds as usize).saturating_mul(sample_rate as usize);

    if samples_per_chunk == 0 {
        return Err(AppError::Config(
            "transcribe_chunk_seconds × sample_rate computed to 0".into(),
        ));
    }

    let chunk_count = samples.len().div_ceil(samples_per_chunk);

    if chunk_count > CHUNK_COUNT_MAX as usize {
        return Err(AppError::Audio(format!(
            "this recording holds {chunk_count} parts, over the {CHUNK_COUNT_MAX} the app \
             transcribes in one pass"
        )));
    }

    let chunk_samples_min = (sample_rate as usize).saturating_mul(CHUNK_SECONDS_MIN as usize);
    let mut chunk_payloads: Vec<ChunkPayload> = Vec::with_capacity(chunk_count);
    let mut chunk_indices_silent: Vec<u32> = Vec::new();
    let mut chunk_index: u32 = 0;

    for chunk_samples in samples.chunks(samples_per_chunk) {
        let peak = chunk_samples
            .iter()
            .copied()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

        debug_assert!(peak >= 0.0);

        if peak < SILENCE_PEAK_MAX {
            chunk_indices_silent.push(chunk_index);
            chunk_index += 1;

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

        chunk_payloads.push(ChunkPayload { index: chunk_index, bytes, name });
        chunk_index += 1;
    }

    debug_assert_eq!(chunk_index as usize, chunk_count);
    debug_assert_eq!(chunk_payloads.len() + chunk_indices_silent.len(), chunk_count);

    Ok((chunk_payloads, chunk_indices_silent))
}

fn chunk_progress_emit(
    progress: &TranscribeProgress,
    chunk_index: u32,
    parsed: &APIResponse,
    chunk_seconds: f64,
) {
    let offset = chunk_offset_seconds(chunk_index, chunk_seconds);

    match parsed.segments.as_ref() {
        Some(api_segments) if !api_segments.is_empty() => {
            for segment in api_segments {
                progress_emit_segment(
                    progress,
                    &segment.text,
                    segment.start + offset,
                    segment.end + offset,
                );
            }
        }
        _ => {
            let sentences = segments_from_text(parsed.text.trim(), offset, offset + chunk_seconds);

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

fn segments_from_text(text: &str, start_seconds: f64, end_seconds: f64) -> Vec<TranscriptSegment> {
    debug_assert!(end_seconds >= start_seconds);

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
    let span = end_seconds - start_seconds;

    for sentence in sentences {
        let chars = u32::try_from(sentence.chars().count()).unwrap_or(u32::MAX);
        let fraction_start = f64::from(chars_before) / f64::from(chars_total.max(1));
        let chars_through = chars_before.saturating_add(chars);
        let fraction_end = f64::from(chars_through) / f64::from(chars_total.max(1));

        segments.push(TranscriptSegment {
            text: sentence,
            start_seconds: span.mul_add(fraction_start, start_seconds),
            end_seconds: span.mul_add(fraction_end, start_seconds),
        });

        chars_before = chars_before.saturating_add(chars);
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
    debug_assert!(!progress.stream_id.is_empty());
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
    debug_assert!(!file_name.is_empty());
    debug_assert!(!upload.model.is_empty());

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
    debug_assert!(!samples.is_empty());

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

    #[test]
    fn assembled_chunks_are_joined_in_order_with_segment_times_shifted_by_their_offset() {
        let completed = vec![
            (0, response_with(" first ", Some(vec![("first", 0.5, 1.5)]))),
            (1, response_with("", None)),
            (2, response_with("third", None)),
            (3, response_with("  ", Some(vec![]))),
        ];

        let transcript = transcribe_chunked_assemble(completed, 30.0);

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
        let transcript = transcribe_chunked_assemble(Vec::new(), 30.0);

        assert!(transcript.text.is_empty());
        assert!(transcript.segments.is_empty());
        assert!((chunk_offset_seconds(3, 30.0) - 90.0).abs() < 1e-9);
        assert!(chunk_offset_seconds(0, 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn silent_chunks_are_pre_filled_and_a_drain_stops_at_the_first_gap() {
        let mut pending = transcribe_chunked_pending(&[0, 2]);
        let mut completed = Vec::new();
        let mut next = 0;

        assert_eq!(pending.len(), 2);
        assert!(transcribe_chunked_pending(&[]).is_empty());

        transcribe_chunked_drain(&mut pending, &mut completed, &mut next, None, 30.0);

        assert_eq!(next, 1);
        assert_eq!(completed.len(), 1);
        assert!(transcribe_chunked_complete(&completed, 3).is_err());

        drop(pending.insert(1, response_with("middle", None)));
        transcribe_chunked_drain(&mut pending, &mut completed, &mut next, None, 30.0);

        let order: Vec<u32> = completed.iter().map(|(index, _)| *index).collect();

        assert_eq!(next, 3);
        assert!(pending.is_empty());
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
        let quiet = vec![0.001_f32; sample_rate as usize * 3];
        let (payloads, silent) = chunk_payloads_build(&loud, sample_rate, 2).expect("build");
        let (none, all) = chunk_payloads_build(&quiet, sample_rate, 2).expect("build");

        assert_eq!(payloads.len(), 2);
        assert!(silent.is_empty());
        assert_eq!(payloads[0].name, "chunk_0000.wav");
        assert_eq!(payloads[1].name, "chunk_0001.wav");
        assert_eq!(payloads[1].index, 1);
        assert_eq!(payloads[0].bytes.len(), payloads[1].bytes.len());
        assert!(none.is_empty());
        assert_eq!(all, vec![0, 1]);
        assert!(chunk_payloads_build(&[], sample_rate, 2).unwrap().0.is_empty());
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

        let (payloads, silent) = chunk_payloads_build(&samples, sample_rate, 30).expect("build");
        let uploaded: Vec<u32> = payloads.iter().map(|chunk| chunk.index).collect();

        assert_eq!(uploaded, vec![0]);
        assert_eq!(silent, vec![1, 2]);
    }

    #[test]
    fn chunk_text_splits_into_sentences_with_proportional_times() {
        let segments = segments_from_text("One two. Three four? Five!", 100.0, 124.0);

        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "One two.");
        assert_eq!(segments[1].text, "Three four?");
        assert_eq!(segments[2].text, "Five!");
        assert!((segments[0].start_seconds - 100.0).abs() < f64::EPSILON);
        assert!((segments[0].end_seconds - 108.0).abs() < f64::EPSILON);
        assert!((segments[2].end_seconds - 124.0).abs() < f64::EPSILON);
        assert!(segments_from_text("   ", 0.0, 5.0).is_empty());
        assert_eq!(segments_from_text("no terminator here", 0.0, 5.0).len(), 1);
        assert_eq!(segments_from_text("It is 10.30 now. Ok.", 0.0, 5.0).len(), 2);
    }

    #[test]
    fn a_chunk_range_reads_as_a_clock_span_and_grows_hours_when_needed() {
        assert_eq!(chunk_range_text(72, 30), "36:00 to 36:30");
        assert_eq!(chunk_range_text(0, 30), "0:00 to 0:30");
        assert_eq!(chunk_range_text(120, 30), "1:00:00 to 1:00:30");
        assert_eq!(clock_text(3599), "59:59");
    }

    #[test]
    fn a_short_chunk_is_padded_up_to_the_service_minimum() {
        let sample_rate = 16000;
        let samples = vec![0.5_f32; 100];
        let (payloads, _) = chunk_payloads_build(&samples, sample_rate, 1).expect("build");
        let chunk_samples_min = sample_rate as usize * CHUNK_SECONDS_MIN as usize;
        let payload_bytes_min = chunk_samples_min * PCM16_SAMPLE_BYTES as usize;

        assert_eq!(payloads.len(), 1);
        assert!(payloads[0].bytes.len() > payload_bytes_min);
    }

    #[test]
    fn a_recording_past_the_chunk_limit_is_refused_before_any_upload() {
        let sample_rate = 16;
        let samples = vec![0.5_f32; sample_rate as usize * (CHUNK_COUNT_MAX as usize + 1)];

        assert!(chunk_payloads_build(&samples, sample_rate, 1).is_err());
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
