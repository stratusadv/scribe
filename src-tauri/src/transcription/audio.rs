use crate::error::{AppError, AppResult};
use crate::workspace::{AUDIO_HOURS_MAX, AUDIO_SECONDS_MAX, PCM16_SAMPLE_BYTES, wav_reader_open};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

pub(crate) const SAMPLE_RATE_WHISPER: u32 = 16000;
const RESAMPLE_CHUNK_SIZE: u32 = 16384;
const WAVEFORM_BAR_COUNT_MAX: u32 = 2000;
const WAVEFORM_READ_BLOCK_BYTES: u32 = 65536;
const PACKET_COUNT_MAX: u32 = 4_000_000;
const DECODE_ERROR_COUNT_MAX: u32 = 64;

const _: () = assert!(WAVEFORM_READ_BLOCK_BYTES.is_multiple_of(PCM16_SAMPLE_BYTES));
const _: () = assert!(WAVEFORM_BAR_COUNT_MAX > 0);
const _: () = assert!(DECODE_ERROR_COUNT_MAX < PACKET_COUNT_MAX);

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Waveform {
    pub(crate) peaks: Vec<f32>,
    pub(crate) duration_seconds: f64,
}

struct DecodeTrack {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    samples_capacity_hint: u64,
}

struct DecodedAudio {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
}

pub(crate) fn waveform_compute(audio_path: &Path, bar_count: u32) -> AppResult<Waveform> {
    let bar_count = bar_count.min(WAVEFORM_BAR_COUNT_MAX);
    let reader = wav_reader_open(audio_path)?;
    let spec = reader.spec();
    let frame_count = reader.duration();

    debug_assert!(spec.sample_rate > 0);

    let duration_seconds = f64::from(frame_count) / f64::from(spec.sample_rate);

    debug_assert!(duration_seconds >= 0.0);

    if bar_count == 0 {
        return Ok(Waveform { peaks: Vec::new(), duration_seconds });
    }

    if frame_count == 0 {
        return Ok(Waveform { peaks: Vec::new(), duration_seconds });
    }

    if spec.bits_per_sample != 16 {
        return Err(AppError::Audio(format!(
            "waveform expects 16-bit PCM, got {}-bit samples",
            spec.bits_per_sample
        )));
    }

    if spec.sample_format != hound::SampleFormat::Int {
        return Err(AppError::Audio(format!(
            "waveform expects integer PCM, got {:?} samples",
            spec.sample_format
        )));
    }

    let samples_total = u64::from(frame_count) * u64::from(spec.channels);
    let peaks = waveform_peaks_scan(reader.into_inner(), samples_total, bar_count)?;

    debug_assert!(peaks.len() <= bar_count as usize);
    debug_assert!(peaks.iter().all(|peak| peak.is_finite()));

    Ok(Waveform { peaks, duration_seconds })
}

fn waveform_peaks_scan(
    mut data: impl Read,
    samples_total: u64,
    bar_count: u32,
) -> AppResult<Vec<f32>> {
    debug_assert!(samples_total > 0);
    debug_assert!(bar_count > 0);

    let bucket_size = samples_total.div_ceil(u64::from(bar_count));
    let mut peaks: Vec<f32> = Vec::with_capacity(bar_count as usize);
    let mut bucket_peak: u16 = 0;
    let mut bucket_filled: u64 = 0;
    let mut samples_read: u64 = 0;
    let mut block = vec![0_u8; WAVEFORM_READ_BLOCK_BYTES as usize];

    debug_assert!(bucket_size > 0);

    while samples_read < samples_total {
        let samples_remaining = samples_total - samples_read;
        let read_bytes_remaining = samples_remaining.saturating_mul(u64::from(PCM16_SAMPLE_BYTES));

        let read_bytes_wanted = usize::try_from(read_bytes_remaining)
            .unwrap_or(WAVEFORM_READ_BLOCK_BYTES as usize)
            .min(WAVEFORM_READ_BLOCK_BYTES as usize);

        debug_assert!(read_bytes_wanted.is_multiple_of(PCM16_SAMPLE_BYTES as usize));

        let block_wanted = block
            .get_mut(..read_bytes_wanted)
            .ok_or_else(|| AppError::Audio("waveform read block overflow".into()))?;

        data.read_exact(block_wanted)
            .map_err(|error| AppError::Audio(format!("wav read: {error}")))?;

        for pair in block_wanted.chunks_exact(PCM16_SAMPLE_BYTES as usize) {
            let &[low, high] = pair else { continue };
            let magnitude = i16::from_le_bytes([low, high]).unsigned_abs();

            if magnitude > bucket_peak {
                bucket_peak = magnitude;
            }

            bucket_filled += 1;

            if bucket_filled == bucket_size {
                peaks.push(f32::from(bucket_peak) / f32::from(i16::MAX));
                bucket_peak = 0;
                bucket_filled = 0;
            }
        }

        samples_read += read_bytes_wanted.div_euclid(PCM16_SAMPLE_BYTES as usize) as u64;
    }

    if bucket_filled > 0 {
        peaks.push(f32::from(bucket_peak) / f32::from(i16::MAX));
    }

    waveform_peaks_normalize(&mut peaks);

    debug_assert_eq!(samples_read, samples_total);
    debug_assert!(peaks.len() <= bar_count as usize);

    Ok(peaks)
}

fn waveform_peaks_normalize(peaks: &mut [f32]) {
    let peak_max = peaks.iter().copied().fold(0.0_f32, f32::max);

    debug_assert!(peak_max >= 0.0);

    if peak_max <= 0.0 {
        return;
    }

    for peak in peaks.iter_mut() {
        *peak /= peak_max;
    }

    debug_assert!(peaks.iter().copied().fold(0.0_f32, f32::max) <= 1.0);
}

pub(crate) fn load_for_whisper(path: &Path) -> AppResult<Vec<f32>> {
    let decoded = decode_packets(decode_track_open(path)?)?;
    let mono = mix_to_mono(&decoded.samples, decoded.channels);

    debug_assert!(decoded.sample_rate > 0);
    debug_assert!(mono.len() <= decoded.samples.len());

    if decoded.sample_rate == SAMPLE_RATE_WHISPER {
        return Ok(mono);
    }

    resample(&mono, decoded.sample_rate, SAMPLE_RATE_WHISPER)
}

fn decode_track_open(path: &Path) -> AppResult<DecodeTrack> {
    let file = File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();

    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    let format = symphonia::default::get_probe()
        .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
        .map_err(|error| AppError::Audio(format!("probe: {error}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| AppError::Audio("no audio track found".into()))?;

    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| AppError::Audio("the audio track carries no codec parameters".into()))?;

    let track_id = track.id;
    let frame_count_hint = track.num_frames;
    let channels_hint = codec_params.channels.as_ref().map(Channels::count);

    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|error| AppError::Audio(format!("decoder init: {error}")))?;

    let samples_capacity_hint = match (frame_count_hint, channels_hint) {
        (Some(frames), Some(channel_count)) => frames.saturating_mul(channel_count as u64),
        _ => 0,
    };

    debug_assert!(channels_hint.is_none_or(|channel_count| channel_count > 0));

    Ok(DecodeTrack { format, decoder, track_id, samples_capacity_hint })
}

fn decode_packets(track: DecodeTrack) -> AppResult<DecodedAudio> {
    let DecodeTrack { mut format, mut decoder, track_id, samples_capacity_hint } = track;
    let capacity = usize::try_from(samples_capacity_hint).unwrap_or(0);
    let mut samples: Vec<f32> = Vec::with_capacity(capacity);
    let mut packet_samples: Vec<f32> = Vec::new();
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut decode_error_count: u32 = 0;

    for _packet_index in 0..PACKET_COUNT_MAX {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => {
                let decoded = DecodedAudio { samples, sample_rate, channels };

                return decode_packets_finish(decoded, decode_error_count);
            }
            Err(error) => return Err(AppError::Audio(format!("packet read: {error}"))),
        };

        if packet.track_id != track_id {
            continue;
        }

        let audio_buffer = match decoder.decode(&packet) {
            Ok(audio_buffer) => audio_buffer,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                decode_error_count += 1;

                decode_error_count_check(decode_error_count)?;

                continue;
            }
            Err(error) => return Err(AppError::Audio(format!("decode: {error}"))),
        };

        let spec = audio_buffer.spec();

        if channels == 0 {
            channels = u16::try_from(spec.channels().count()).map_err(|_| {
                AppError::Audio("track reports more channels than fit in u16".into())
            })?;

            sample_rate = spec.rate();
        }

        audio_buffer.copy_to_vec_interleaved(&mut packet_samples);
        samples.extend_from_slice(&packet_samples);

        decode_samples_bound_check(samples.len() as u64, sample_rate, channels)?;
    }

    Err(AppError::Audio(format!(
        "this recording holds more than the {PACKET_COUNT_MAX} packets the app decodes in one pass"
    )))
}

fn decode_error_count_check(decode_error_count: u32) -> AppResult<()> {
    debug_assert!(decode_error_count > 0);

    if decode_error_count > DECODE_ERROR_COUNT_MAX {
        return Err(AppError::Audio(format!(
            "more than {DECODE_ERROR_COUNT_MAX} parts of this recording could not be decoded; \
             the file is damaged or in a format the app does not read"
        )));
    }

    Ok(())
}

fn decode_samples_bound_check(sample_count: u64, sample_rate: u32, channels: u16) -> AppResult<()> {
    debug_assert!(sample_rate > 0);
    debug_assert!(channels > 0);

    let samples_max = AUDIO_SECONDS_MAX * u64::from(sample_rate) * u64::from(channels);

    if sample_count > samples_max {
        return Err(AppError::Audio(format!(
            "this recording is longer than the {AUDIO_HOURS_MAX} hours this app transcribes"
        )));
    }

    Ok(())
}

fn decode_packets_finish(
    decoded: DecodedAudio,
    decode_error_count: u32,
) -> AppResult<DecodedAudio> {
    if decoded.channels == 0 {
        return Err(AppError::Audio("no audio decoded from track".into()));
    }

    if decoded.sample_rate == 0 {
        return Err(AppError::Audio("no audio decoded from track".into()));
    }

    if decode_error_count > 0 {
        tracing::warn!(
            target: "scribe_lib::transcription",
            "{decode_error_count} packets could not be decoded and were skipped"
        );
    }

    debug_assert!(decoded.sample_rate > 0);
    debug_assert!(decoded.channels > 0);
    debug_assert!(decode_error_count <= DECODE_ERROR_COUNT_MAX);

    Ok(decoded)
}

fn mix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    debug_assert!(channels > 0);

    if channels == 1 {
        return interleaved.to_vec();
    }

    let channel_count = channels as usize;

    let mono: Vec<f32> = interleaved
        .chunks_exact(channel_count)
        .map(|frame| frame.iter().sum::<f32>() / f32::from(channels))
        .collect();

    debug_assert_eq!(mono.len(), interleaved.len().div_euclid(channel_count));

    mono
}

fn resample_output_length(input_length: u64, source_rate: u32, target_rate: u32) -> u64 {
    debug_assert!(source_rate > 0);
    debug_assert!(target_rate > 0);

    let output_length = input_length
        .saturating_mul(u64::from(target_rate))
        .div_ceil(u64::from(source_rate));

    debug_assert!(output_length <= input_length.saturating_mul(u64::from(target_rate)));

    output_length
}

fn resample(input: &[f32], source_rate: u32, target_rate: u32) -> AppResult<Vec<f32>> {
    debug_assert!(source_rate > 0);
    debug_assert!(target_rate > 0);
    debug_assert_ne!(source_rate, target_rate);

    let ratio = f64::from(target_rate) / f64::from(source_rate);

    let mut resampler = Async::<f32>::new_poly(
        ratio,
        1.0,
        PolynomialDegree::Cubic,
        RESAMPLE_CHUNK_SIZE as usize,
        1,
        FixedAsync::Input,
    )
    .map_err(|error| AppError::Audio(format!("resampler init: {error}")))?;

    let buffer_in = InterleavedSlice::new(input, 1, input.len())
        .map_err(|error| AppError::Audio(format!("resampler input: {error}")))?;

    let output = resampler
        .process_all(&buffer_in, input.len(), None)
        .map_err(|error| AppError::Audio(format!("resample: {error}")))?
        .take_data();

    let output_length = resample_output_length(input.len() as u64, source_rate, target_rate);

    debug_assert!((output.len() as u64).abs_diff(output_length) <= 1);

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::test_support::root_scoped;
    use crate::workspace::{audio_wav_save, root};
    use std::fs;

    #[test]
    fn a_waveform_has_one_peak_per_bucket_and_none_for_empty_audio_or_zero_bars() {
        let _root = root_scoped("waveform");
        let path = root().unwrap().join("audio.wav");
        let samples = [0.0_f32, 0.25, 0.5, 0.25, 1.0, 0.0];

        audio_wav_save(&samples, 16000, &path).unwrap();

        let waveform = waveform_compute(&path, 3).unwrap();
        let capped = waveform_compute(&path, u32::MAX).unwrap();

        assert_eq!(waveform.peaks.len(), 3);
        assert!((waveform.peaks[0] - 0.25).abs() < 1e-4);
        assert!((waveform.peaks[1] - 0.5).abs() < 1e-4);
        assert!((waveform.peaks[2] - 1.0).abs() < 1e-6);
        assert!((waveform.duration_seconds - 6.0 / 16000.0).abs() < 1e-9);
        assert!(waveform_compute(&path, 0).unwrap().peaks.is_empty());
        assert_eq!(capped.peaks.len(), samples.len());

        audio_wav_save(&[], 16000, &path).unwrap();

        assert!(waveform_compute(&path, 3).unwrap().peaks.is_empty());
        assert!(waveform_compute(&path, 3).unwrap().duration_seconds.abs() < f64::EPSILON);
    }

    #[test]
    fn a_waveform_needs_16_bit_integer_samples() {
        let _root = root_scoped("waveform-8bit");
        let path = root().unwrap().join("audio.wav");

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(&path, spec).unwrap();

        writer.write_sample(1_i8).unwrap();
        writer.finalize().unwrap();

        assert!(waveform_compute(&path, 1).is_err());
    }

    #[test]
    fn a_partial_last_bucket_still_yields_a_peak() {
        let samples: Vec<u8> = [0x1000_i16, 0x2000, 0x0800]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();

        let peaks = waveform_peaks_scan(samples.as_slice(), 3, 2).expect("scan");

        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 1.0).abs() < 1e-6);
        assert!((peaks[1] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn stereo_frames_with_a_dangling_sample_drop_it() {
        assert_eq!(mix_to_mono(&[1.0, 1.0, 1.0], 2), vec![1.0]);
        assert!(mix_to_mono(&[], 2).is_empty());
        assert!(mix_to_mono(&[], 1).is_empty());
    }

    #[test]
    fn resampling_halves_the_length_and_keeps_a_steady_signal_level() {
        let input = vec![0.5_f32; 32000];
        let output = resample(&input, 32000, 16000).unwrap();

        assert!(output.len().abs_diff(16000) <= 1);
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!((output[8000] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_wav_loads_for_whisper_at_its_native_rate_or_resampled_to_it() {
        let _root = root_scoped("load-for-whisper");
        let native = root().unwrap().join("native.wav");
        let slow = root().unwrap().join("slow.wav");
        let samples = vec![0.25_f32; 8000];

        audio_wav_save(&samples, SAMPLE_RATE_WHISPER, &native).unwrap();
        audio_wav_save(&samples, 8000, &slow).unwrap();

        let loaded_native = load_for_whisper(&native).unwrap();
        let loaded_slow = load_for_whisper(&slow).unwrap();

        assert_eq!(loaded_native.len(), samples.len());
        assert!((loaded_native[100] - 0.25).abs() < 1e-3);
        assert!(loaded_slow.len().abs_diff(16000) <= 1);
        assert!((loaded_slow[8000] - 0.25).abs() < 1e-2);

        fs::write(&native, b"not audio").unwrap();

        assert!(load_for_whisper(&native).is_err());
        assert!(load_for_whisper(&root().unwrap().join("missing.wav")).is_err());
    }

    #[test]
    fn a_stereo_file_is_mixed_down_before_it_is_returned() {
        let _root = root_scoped("load-stereo");
        let path = root().unwrap().join("stereo.wav");

        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SAMPLE_RATE_WHISPER,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(&path, spec).unwrap();

        for _frame in 0..100 {
            writer.write_sample(i16::MAX).unwrap();
            writer.write_sample(0_i16).unwrap();
        }

        writer.finalize().unwrap();

        let mono = load_for_whisper(&path).unwrap();

        assert_eq!(mono.len(), 100);
        assert!((mono[50] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn peaks_are_normalised_against_the_loudest_bucket() {
        let mut peaks = vec![0.25_f32, 0.5, 0.125];

        waveform_peaks_normalize(&mut peaks);

        assert_eq!(peaks, vec![0.5, 1.0, 0.25]);
    }

    #[test]
    fn silence_is_left_alone_rather_than_divided_by_zero() {
        let mut peaks = vec![0.0_f32, 0.0];

        waveform_peaks_normalize(&mut peaks);

        assert_eq!(peaks, vec![0.0, 0.0]);
    }

    #[test]
    fn one_bar_per_sample_gives_one_peak_per_sample() {
        let samples: Vec<u8> = vec![0x00, 0x40, 0x00, 0x20];
        let peaks = waveform_peaks_scan(samples.as_slice(), 2, 2).expect("scan");

        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks, vec![1.0, 0.5]);
    }

    #[test]
    fn a_decode_that_runs_past_the_audio_limit_is_refused() {
        let within = AUDIO_SECONDS_MAX * 16000;

        assert!(decode_samples_bound_check(within, 16000, 1).is_ok());
        assert!(decode_samples_bound_check(within + 1, 16000, 1).is_err());
        assert!(decode_samples_bound_check(within * 2, 16000, 2).is_ok());
    }

    #[test]
    fn a_mostly_undecodable_file_is_refused_rather_than_read_as_silence() {
        assert!(decode_error_count_check(1).is_ok());
        assert!(decode_error_count_check(DECODE_ERROR_COUNT_MAX).is_ok());
        assert!(decode_error_count_check(DECODE_ERROR_COUNT_MAX + 1).is_err());
    }

    #[test]
    fn stereo_frames_are_averaged_into_one_channel() {
        assert_eq!(mix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(mix_to_mono(&[0.25, 0.75], 1), vec![0.25, 0.75]);
    }

    #[test]
    fn the_resampled_length_follows_the_rate_ratio() {
        assert_eq!(resample_output_length(48000, 48000, 16000), 16000);
        assert_eq!(resample_output_length(100, 16000, 16000), 100);
        assert_eq!(resample_output_length(1, 44100, 16000), 1);
        assert_eq!(resample_output_length(0, 44100, 16000), 0);
    }
}
