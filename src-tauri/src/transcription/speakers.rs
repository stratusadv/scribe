use crate::error::{AppError, AppResult};
use polyvoice::types::{Profile, SampleRate};
use polyvoice::{ClustererKind, ModelRegistry, Pipeline, PipelineConfig};
use std::collections::HashMap;
use std::path::Path;

const WINDOW_SECONDS: u32 = 1200;
const OVERLAP_SECONDS: u32 = 120;
const WINDOW_COUNT_MAX: u32 = 64;
const SPEAKERS_MAX: u8 = 20;
const THREADS_MAX: u32 = 4;
const GHOST_SHARE_MAX: f64 = 0.02;
const TURN_SECONDS_MIN: f64 = 1.0;
const TURN_GAP_SECONDS_MAX: f64 = 1.5;
const ENVIRONMENT_CONV_THREADS: &str = "POLYVOICE_CONV_THREADS";
const ENVIRONMENT_EMBED_THREADS: &str = "POLYVOICE_EMBED_THREADS";

/// Single-worker segmentation in polyvoice packs every window of a block at once (4 GB for 20 min).
const SEGMENTATION_WORKERS_MIN: u32 = 2;

const _: () = assert!(OVERLAP_SECONDS < WINDOW_SECONDS);
const _: () = assert!(WINDOW_COUNT_MAX > 0);
const _: () = assert!(THREADS_MAX > 0);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SpeakerTurn {
    pub(crate) speaker: u16,
    pub(crate) start_seconds: f64,
    pub(crate) end_seconds: f64,
}

#[derive(Debug, Clone)]
struct WindowTurn {
    speaker_local: u32,
    start_seconds: f64,
    end_seconds: f64,
}

pub(crate) fn speaker_turns_detect(
    samples: &[f32],
    sample_rate: u32,
    models_dir: &Path,
) -> AppResult<Vec<SpeakerTurn>> {
    debug_assert!(sample_rate > 0);

    let threads = threads_budget();
    let rate = sample_rate_of(sample_rate)?;
    let window_samples = u64::from(WINDOW_SECONDS) * u64::from(sample_rate);
    let step_samples = u64::from(WINDOW_SECONDS - OVERLAP_SECONDS) * u64::from(sample_rate);
    let pipeline = pipeline_build(models_dir, rate, threads, window_samples)?;
    let sample_count = samples.len() as u64;
    let mut merged = TurnMerge { turns: Vec::new(), speaker_count: 0 };
    let mut window_start: u64 = 0;

    for _window_index in 0..WINDOW_COUNT_MAX {
        if window_start >= sample_count {
            break;
        }

        let window_end = window_start.saturating_add(window_samples).min(sample_count);
        let range = usize::try_from(window_start).unwrap_or(usize::MAX)
            ..usize::try_from(window_end).unwrap_or(usize::MAX);
        let window = samples.get(range).unwrap_or_default();
        let offset_seconds = seconds_of(window_start, sample_rate);
        let result = pipeline.run(window, rate).map_err(speakers_error)?;
        let window_turns = WindowTurn::from_result(result, offset_seconds);

        let covered_until = if window_start == 0 {
            0.0
        } else {
            offset_seconds + f64::from(OVERLAP_SECONDS)
        };

        merged.absorb(window_turns, offset_seconds, covered_until);

        if window_end == sample_count {
            break;
        }

        window_start = window_start.saturating_add(step_samples);
    }

    merged
        .turns
        .sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

    let smoothed = turns_smooth(&merged.turns);

    tracing::info!(
        target: "scribe_lib::transcription",
        "speakers: {} raw turns across {} speakers became {} turns, {} threads",
        merged.turns.len(),
        merged.speaker_count,
        smoothed.len(),
        threads
    );

    debug_assert!(smoothed.is_sorted_by(|left, right| left.start_seconds <= right.start_seconds));

    Ok(smoothed)
}

impl WindowTurn {
    fn from_result(result: polyvoice::types::DiarizationResult, offset_seconds: f64) -> Vec<Self> {
        debug_assert!(offset_seconds >= 0.0);

        result
            .turns
            .into_iter()
            .map(|turn| Self {
                speaker_local: turn.speaker.0,
                start_seconds: turn.time.start + offset_seconds,
                end_seconds: turn.time.end + offset_seconds,
            })
            .collect()
    }
}

struct TurnMerge {
    turns: Vec<SpeakerTurn>,
    speaker_count: u16,
}

impl TurnMerge {
    fn absorb(&mut self, window_turns: Vec<WindowTurn>, overlap_start: f64, covered_until: f64) {
        debug_assert!(overlap_start <= covered_until || covered_until == 0.0);

        let mut labels = labels_link(&self.turns, &window_turns, overlap_start, covered_until);

        for turn in window_turns {
            let start_seconds = turn.start_seconds.max(covered_until);

            if turn.end_seconds <= start_seconds {
                continue;
            }

            let speaker = *labels.entry(turn.speaker_local).or_insert_with(|| {
                self.speaker_count = self.speaker_count.saturating_add(1);

                self.speaker_count - 1
            });

            self.turns.push(SpeakerTurn {
                speaker,
                start_seconds,
                end_seconds: turn.end_seconds,
            });
        }
    }
}

fn sample_rate_of(sample_rate: u32) -> AppResult<SampleRate> {
    SampleRate::new(sample_rate).ok_or_else(|| {
        AppError::Audio(format!("speaker detection cannot run at {sample_rate} Hz"))
    })
}

fn pipeline_build(
    models_dir: &Path,
    rate: SampleRate,
    threads: u32,
    window_samples: u64,
) -> AppResult<Pipeline> {
    debug_assert!(threads > 0);
    debug_assert!(window_samples > 0);

    let divisor = segmentation_divisor(cores_available(), threads);

    polyvoice_kernels::set_intra_threads(threads as usize);
    polyvoice_kernels::set_file_parallelism(divisor as usize);

    let mut config = PipelineConfig::default();

    config.profile = Profile::Balanced;
    config.sample_rate = rate;
    config.clusterer = ClustererKind::Vbx;
    config.max_speakers = SPEAKERS_MAX;
    config.embedder_pool_size = threads as usize;
    config.max_audio_samples = usize::try_from(window_samples).unwrap_or(usize::MAX);
    config.vbx_plda_dir = Some(models_dir.to_path_buf());

    let registry = ModelRegistry::with_local_dir(models_dir).map_err(speakers_error)?;

    Pipeline::builder()
        .with_models_from(registry)
        .config(config)
        .build()
        .map_err(speakers_error)
}

pub(crate) fn threads_budget_install() {
    // Runs before any other thread exists; polyvoice reads these on every call.
    unsafe {
        std::env::set_var(ENVIRONMENT_EMBED_THREADS, threads_budget().to_string());
        std::env::set_var(ENVIRONMENT_CONV_THREADS, "1");
    }
}

fn cores_available() -> u32 {
    std::thread::available_parallelism()
        .map_or(1, |cores| u32::try_from(cores.get()).unwrap_or(u32::MAX))
}

fn threads_budget() -> u32 {
    cores_available().div_ceil(2).clamp(1, THREADS_MAX)
}

fn segmentation_divisor(cores: u32, threads: u32) -> u32 {
    debug_assert!(threads > 0);

    let workers = threads.max(SEGMENTATION_WORKERS_MIN).min(cores).max(1);

    cores.div_ceil(workers).max(1)
}

fn seconds_of(sample_index: u64, sample_rate: u32) -> f64 {
    debug_assert!(sample_rate > 0);

    let index = u32::try_from(sample_index).unwrap_or(u32::MAX);
    let seconds = f64::from(index) / f64::from(sample_rate);

    debug_assert!(seconds >= 0.0);

    seconds
}

fn speakers_error(error: impl std::fmt::Display) -> AppError {
    AppError::Transcription(format!("speaker detection: {error}"))
}

fn labels_link(
    previous: &[SpeakerTurn],
    current: &[WindowTurn],
    overlap_start: f64,
    overlap_end: f64,
) -> HashMap<u32, u16> {
    debug_assert!(overlap_start <= overlap_end);

    let mut shared: HashMap<(u32, u16), f64> = HashMap::new();

    for turn in current {
        for earlier in previous {
            let start = turn.start_seconds.max(earlier.start_seconds).max(overlap_start);
            let end = turn.end_seconds.min(earlier.end_seconds).min(overlap_end);

            if end > start {
                *shared.entry((turn.speaker_local, earlier.speaker)).or_insert(0.0) += end - start;
            }
        }
    }

    let mut pairs: Vec<((u32, u16), f64)> = shared.into_iter().collect();

    pairs.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));

    let mut labels: HashMap<u32, u16> = HashMap::new();
    let mut taken: Vec<u16> = Vec::new();

    for ((local, global), _seconds) in pairs {
        if labels.contains_key(&local) {
            continue;
        }

        if taken.contains(&global) {
            continue;
        }

        let previous = labels.insert(local, global);

        debug_assert!(previous.is_none());

        taken.push(global);
    }

    labels
}

pub(crate) fn speaker_turns_from_remote(turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    let received = turns.len();

    let mut valid: Vec<SpeakerTurn> = turns
        .into_iter()
        .filter(|turn| turn.start_seconds.is_finite() && turn.end_seconds.is_finite())
        .filter(|turn| turn.start_seconds >= 0.0 && turn.end_seconds > turn.start_seconds)
        .collect();

    valid.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

    let smoothed = turns_smooth(&valid);

    tracing::info!(
        target: "scribe_lib::transcription",
        "speakers: {} remote turns, {} valid, became {} turns",
        received,
        valid.len(),
        smoothed.len()
    );

    smoothed
}

fn turns_join(turns: &[SpeakerTurn]) -> Vec<SpeakerTurn> {
    let mut joined: Vec<SpeakerTurn> = Vec::with_capacity(turns.len());

    for turn in turns {
        if let Some(last) = joined.last_mut() {
            if last.speaker == turn.speaker {
                if turn.start_seconds - last.end_seconds <= TURN_GAP_SECONDS_MAX {
                    last.end_seconds = last.end_seconds.max(turn.end_seconds);

                    continue;
                }
            }
        }

        joined.push(turn.clone());
    }

    debug_assert!(joined.len() <= turns.len());

    joined
}

fn turns_smooth(turns: &[SpeakerTurn]) -> Vec<SpeakerTurn> {
    let joined = turns_join(turns);
    let turns = joined.as_slice();
    let mut totals: HashMap<u16, f64> = HashMap::new();

    for turn in turns {
        *totals.entry(turn.speaker).or_insert(0.0) += turn.end_seconds - turn.start_seconds;
    }

    let speech_total: f64 = totals.values().sum();

    let ghosts: Vec<u16> = totals
        .iter()
        .filter(|(_, total)| **total < speech_total * GHOST_SHARE_MAX)
        .map(|(speaker, _)| *speaker)
        .collect();

    let mut smoothed: Vec<SpeakerTurn> = Vec::with_capacity(turns.len());

    for turn in turns {
        let length = turn.end_seconds - turn.start_seconds;
        let absorbed = ghosts.contains(&turn.speaker) || length < TURN_SECONDS_MIN;

        if absorbed {
            if let Some(last) = smoothed.last_mut() {
                last.end_seconds = last.end_seconds.max(turn.end_seconds);
            }

            continue;
        }

        if let Some(last) = smoothed.last_mut() {
            if last.speaker == turn.speaker {
                if turn.start_seconds - last.end_seconds <= TURN_GAP_SECONDS_MAX {
                    last.end_seconds = last.end_seconds.max(turn.end_seconds);

                    continue;
                }
            }
        }

        let start_seconds = smoothed
            .last()
            .map_or(turn.start_seconds, |last| turn.start_seconds.max(last.end_seconds));

        if turn.end_seconds - start_seconds < TURN_SECONDS_MIN {
            continue;
        }

        smoothed.push(SpeakerTurn {
            speaker: turn.speaker,
            start_seconds,
            end_seconds: turn.end_seconds,
        });
    }

    debug_assert!(smoothed.len() <= turns.len());

    smoothed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(speaker: u16, start_seconds: f64, end_seconds: f64) -> SpeakerTurn {
        SpeakerTurn { speaker, start_seconds, end_seconds }
    }

    fn window_turn(speaker_local: u32, start_seconds: f64, end_seconds: f64) -> WindowTurn {
        WindowTurn { speaker_local, start_seconds, end_seconds }
    }

    #[test]
    fn smoothing_drops_ghosts_absorbs_blips_and_joins_a_speaker_across_short_gaps() {
        let raw = vec![
            turn(0, 0.0, 10.0),
            turn(0, 11.0, 20.0),
            turn(1, 20.5, 20.8),
            turn(1, 21.0, 30.0),
            turn(2, 30.2, 30.5),
            turn(0, 33.0, 40.0),
        ];

        let smoothed = turns_smooth(&raw);

        assert_eq!(smoothed, vec![turn(0, 0.0, 20.0), turn(1, 20.5, 30.5), turn(0, 33.0, 40.0)]);
    }

    #[test]
    fn a_short_reply_stays_with_the_speaker_who_keeps_talking_after_it() {
        let raw = vec![
            turn(0, 0.03, 12.3),
            turn(1, 13.21, 13.85),
            turn(1, 14.68, 18.41),
            turn(0, 25.36, 26.14),
            turn(0, 26.95, 28.94),
        ];

        let smoothed = turns_smooth(&raw);

        assert_eq!(
            smoothed,
            vec![turn(0, 0.03, 12.3), turn(1, 13.21, 18.41), turn(0, 25.36, 28.94)]
        );
    }

    #[test]
    fn remote_turns_are_validated_sorted_and_smoothed() {
        let raw = vec![
            turn(1, 13.21, 18.41),
            turn(0, 0.0, 12.0),
            turn(0, f64::NAN, 3.0),
            turn(1, 5.0, 4.0),
            turn(1, -1.0, 2.0),
        ];

        let turns = speaker_turns_from_remote(raw);

        assert_eq!(turns, vec![turn(0, 0.0, 12.0), turn(1, 13.21, 18.41)]);
        assert_eq!(speaker_turns_from_remote(Vec::new()), Vec::new());
    }

    #[test]
    fn a_turn_clipped_by_its_predecessor_keeps_the_later_start() {
        let raw = vec![turn(0, 0.0, 10.0), turn(1, 8.0, 15.0), turn(0, 15.0, 15.5)];

        let smoothed = turns_smooth(&raw);

        assert_eq!(smoothed, vec![turn(0, 0.0, 10.0), turn(1, 10.0, 15.5)]);
    }

    #[test]
    fn window_labels_follow_the_speaker_who_shares_the_most_overlap_time() {
        let previous = vec![turn(0, 1000.0, 1100.0), turn(1, 1100.0, 1200.0)];

        let current = vec![
            window_turn(7, 1080.0, 1150.0),
            window_turn(3, 1160.0, 1300.0),
            window_turn(9, 1300.0, 1400.0),
        ];

        let labels = labels_link(&previous, &current, 1080.0, 1200.0);

        assert_eq!(labels.get(&7), Some(&1));
        assert_eq!(labels.get(&3), None);
        assert_eq!(labels.get(&9), None);
        assert_eq!(labels_link(&[], &current, 0.0, 0.0).len(), 0);
    }

    #[test]
    fn the_thread_budget_stays_within_one_and_the_cap() {
        let threads = threads_budget();

        assert!(threads >= 1);
        assert!(threads <= THREADS_MAX);
        assert!((seconds_of(32000, 16000) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn segmentation_keeps_two_workers_and_never_exceeds_the_budget() {
        for cores in 1..=256_u32 {
            for threads in 1..=THREADS_MAX {
                let workers = cores.div_ceil(segmentation_divisor(cores, threads));

                assert!(workers <= threads.max(SEGMENTATION_WORKERS_MIN));
                assert!(workers >= cores.min(SEGMENTATION_WORKERS_MIN));
            }
        }
    }
}
