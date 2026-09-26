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

const REQUEST_TIMEOUT_SECONDS: u64 = 90;
const CONNECT_TIMEOUT_SECONDS: u64 = 15;
const CHUNK_CONCURRENCY_MAX: u32 = 8;
const CHUNK_COUNT_MAX: u32 = 4096;
const CHUNK_SECONDS_MIN: u32 = 2;
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
    let parsed = upload_chunk(upload, file_bytes, file_name).await?;

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
        f64::from(chunk_seconds),
        progress,
        cancel_flag.as_deref(),
    )
    .await;

    if let Some(progress) = progress {
        progress.cancel_clear();
    }

    Ok(transcribe_chunked_assemble(completed?, f64::from(chunk_seconds)))
}

async fn transcribe_chunked_collect(
    chunk_payloads: Vec<ChunkPayload>,
    chunk_indices_silent: Vec<u32>,
    upload: ChunkUpload<'_>,
    chunk_seconds: f64,
    progress: Option<&TranscribeProgress>,
    cancel_flag: Option<&AtomicBool>,
) -> AppResult<Vec<(u32, APIResponse)>> {
    let chunk_count = chunk_payloads.len() + chunk_indices_silent.len();
    let mut completed: Vec<(u32, APIResponse)> = Vec::with_capacity(chunk_count);
    let mut pending = transcribe_chunked_pending(&chunk_indices_silent);
    let mut completion_count = chunk_indices_silent.len();
    let mut chunk_index_next: u32 = 0;
    let mut stream = transcribe_chunked_stream(chunk_payloads, upload);
    let cancel_poll = Duration::from_millis(CANCEL_POLL_INTERVAL_MS);

    for _poll in 0..transcribe_chunked_poll_count_max(chunk_count as u64) {
        if cancel_flag.is_some_and(is_cancelled) {
            return Err(AppError::Transcription("Cancelled".into()));
        }

        let result = match tokio::time::timeout(cancel_poll, stream.next()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_elapsed) => continue,
        };

        let (chunk_index, parsed) = result?;
        completion_count += 1;

        tracing::info!(
            target: "scribe_lib::transcription",
            "chunk completed {}/{} (index={})",
            completion_count,
            chunk_count,
            chunk_index
        );

        let previous = pending.insert(chunk_index, parsed);

        debug_assert!(previous.is_none());

        transcribe_chunked_drain(
            &mut pending,
            &mut completed,
            &mut chunk_index_next,
            progress,
            chunk_seconds,
        );
    }

    transcribe_chunked_drain(
        &mut pending,
        &mut completed,
        &mut chunk_index_next,
        progress,
        chunk_seconds,
    );

    transcribe_chunked_complete(&completed, chunk_count)?;

    debug_assert!(pending.is_empty());

    Ok(completed)
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
) -> impl Stream<Item = AppResult<(u32, APIResponse)>> + '_ {
    debug_assert!(chunk_payloads.len() <= CHUNK_COUNT_MAX as usize);

    futures_util::stream::iter(chunk_payloads.into_iter().map(move |chunk| async move {
        let parsed = upload_chunk(upload, chunk.bytes, chunk.name).await?;

        Ok::<(u32, APIResponse), AppError>((chunk.index, parsed))
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
            if !text_trimmed.is_empty() {
                segments.push(TranscriptSegment {
                    text: text_trimmed.to_owned(),
                    start_seconds: offset,
                    end_seconds: offset + chunk_seconds,
                });
            }
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
            let text_trimmed = parsed.text.trim();

            if !text_trimmed.is_empty() {
                progress_emit_segment(progress, text_trimmed, offset, offset + chunk_seconds);
            }
        }
    }
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
    bytes: Vec<u8>,
    file_name: String,
) -> AppResult<APIResponse> {
    debug_assert!(!file_name.is_empty());
    debug_assert!(!upload.model.is_empty());

    if bytes.is_empty() {
        return Err(AppError::Audio("the prepared audio holds no samples".into()));
    }

    let client = client_get(&HTTP_CLIENT)?;

    retry_run(|| async {
        let part = multipart::Part::bytes(bytes.clone())
            .file_name(file_name.clone())
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
