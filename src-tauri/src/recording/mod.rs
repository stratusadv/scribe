use crate::blocking;
use crate::error::{AppError, AppResult};
use crate::settings;
use crate::state::AppState;
use crate::workspace::{
    self,
    AUDIO_HOURS_MAX,
    AUDIO_SECONDS_MAX,
    PCM16_SAMPLE_BYTES,
    wav_specification_mono_pcm16,
};
use std::fmt;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri::ipc::{InvokeBody, Request};

const RECORDINGS_FOLDER: &str = "scribe";
const RECORDINGS_FOLDER_FALLBACK: &str = "recordings";
const RECORDING_FILE_PREFIX: &str = "Recording ";
const RECORDING_FILE_EXTENSION: &str = "wav";
const RECORDING_FILE_TIME_FORMAT: &str = "%Y-%m-%d %H-%M-%S";
const RECORDING_SAMPLE_RATE_MIN: u32 = 8000;
const RECORDING_SAMPLE_RATE_MAX: u32 = 48000;
const RECORDING_SAMPLES_MAX: u64 = RECORDING_SAMPLE_RATE_MAX as u64 * AUDIO_SECONDS_MAX;
const RECORDING_CHUNK_BYTES_MAX: u32 = 1 << 20;

const _: () = assert!(RECORDING_SAMPLE_RATE_MIN < RECORDING_SAMPLE_RATE_MAX);
const _: () = assert!(RECORDING_CHUNK_BYTES_MAX.is_multiple_of(PCM16_SAMPLE_BYTES));

const _: () = assert!((RECORDING_SAMPLES_MAX * (PCM16_SAMPLE_BYTES as u64)) < (1u64 << 32));

pub(crate) type RecordingSlot = Arc<Mutex<Option<RecordingActive>>>;

#[cfg(target_os = "linux")] // tigerstyle-ignore: TS035
pub(crate) fn microphone_permission_install(window: &tauri::WebviewWindow) {
    use webkit2gtk::glib::ObjectExt;
    use webkit2gtk::{PermissionRequestExt, UserMediaPermissionRequest, WebViewExt};

    let installed = window.with_webview(|webview| {
        webview.inner().connect_permission_request(|_, request| {
            if request.is::<UserMediaPermissionRequest>() {
                request.allow();

                return true;
            }

            false
        });
    });

    if let Err(error) = installed {
        tracing::warn!("microphone permission handler not installed: {error}");
    }
}

#[cfg(not(target_os = "linux"))] // tigerstyle-ignore: TS035
pub(crate) const fn microphone_permission_install(_window: &tauri::WebviewWindow) {}

pub(crate) struct RecordingActive {
    path: PathBuf,
    sample_rate: u32,
    writer: hound::WavWriter<BufWriter<File>>,
}

impl fmt::Debug for RecordingActive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingActive")
            .field("path", &self.path)
            .field("sample_rate", &self.sample_rate)
            .field("samples_written", &self.writer.len())
            .finish()
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn recording_start(app: tauri::AppHandle, sample_rate: u32) -> AppResult<String> {
    let slot = Arc::clone(&app.state::<AppState>().recording);

    blocking::run(move || recording_start_blocking(&slot, sample_rate)).await
}

// tigerstyle-ignore: TS020
#[expect(clippy::needless_pass_by_value, reason = "tauri passes command arguments by value")]
#[tauri::command(async)]
pub(crate) fn recording_append(
    state: tauri::State<'_, AppState>,
    request: Request<'_>,
) -> AppResult<()> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes,
        InvokeBody::Json(_) => {
            return Err(AppError::Audio("recording chunk must be raw PCM bytes".into()));
        }
    };

    recording_append_blocking(&state.recording, bytes)
}

#[tauri::command]
pub(crate) async fn recording_finish(app: tauri::AppHandle) -> AppResult<String> {
    let slot = Arc::clone(&app.state::<AppState>().recording);

    blocking::run(move || recording_finish_blocking(&slot)).await
}

#[tauri::command]
pub(crate) async fn recordings_directory_default() -> AppResult<String> {
    blocking::run(|| {
        recordings_directory_default_resolve()
            .map(|directory| directory.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub(crate) async fn recording_discard(app: tauri::AppHandle) -> AppResult<()> {
    let slot = Arc::clone(&app.state::<AppState>().recording);

    blocking::run(move || recording_discard_blocking(&slot)).await
}

fn recording_start_blocking(slot: &RecordingSlot, sample_rate: u32) -> AppResult<String> {
    if !(RECORDING_SAMPLE_RATE_MIN..=RECORDING_SAMPLE_RATE_MAX).contains(&sample_rate) {
        return Err(AppError::Audio(format!(
            "the microphone sample rate {sample_rate} Hz is outside the supported range"
        )));
    }

    let path = recording_path_new()?;

    debug_assert!(sample_rate >= RECORDING_SAMPLE_RATE_MIN);
    debug_assert!(sample_rate <= RECORDING_SAMPLE_RATE_MAX);

    recording_open(slot, sample_rate, path)
}

fn recording_open(slot: &RecordingSlot, sample_rate: u32, path: PathBuf) -> AppResult<String> {
    debug_assert!(sample_rate > 0);

    let mut guard = recording_slot_lock(slot)?;

    if guard.is_some() {
        return Err(AppError::Audio("a recording is already in progress".into()));
    }

    let writer = hound::WavWriter::create(&path, wav_specification_mono_pcm16(sample_rate))
        .map_err(|error| AppError::Audio(format!("could not create the recording: {error}")))?;

    let path_text = path.to_string_lossy().into_owned();
    *guard = Some(RecordingActive { path, sample_rate, writer });

    drop(guard);

    debug_assert!(!path_text.is_empty());

    Ok(path_text)
}

fn recording_append_blocking(slot: &RecordingSlot, bytes: &[u8]) -> AppResult<()> {
    if bytes.len() > RECORDING_CHUNK_BYTES_MAX as usize {
        return Err(AppError::Audio(format!(
            "a recording chunk of {} bytes is over the {} byte limit",
            bytes.len(),
            RECORDING_CHUNK_BYTES_MAX
        )));
    }

    if !bytes.len().is_multiple_of(PCM16_SAMPLE_BYTES as usize) {
        return Err(AppError::Audio("a recording chunk has a half sample at its end".into()));
    }

    let mut guard = recording_slot_lock(slot)?;

    let active = guard
        .as_mut()
        .ok_or_else(|| AppError::Audio("no recording is in progress".into()))?;

    let samples_chunk = u64::try_from(bytes.len().div_euclid(PCM16_SAMPLE_BYTES as usize))
        .map_err(|error| AppError::Audio(format!("recording chunk size: {error}")))?;

    let samples_max = u64::from(active.sample_rate) * AUDIO_SECONDS_MAX;
    let samples_before = u64::from(active.writer.len());

    debug_assert!(samples_max <= RECORDING_SAMPLES_MAX);

    if samples_before + samples_chunk > samples_max {
        return Err(AppError::Audio(format!(
            "the recording reached the limit of {AUDIO_HOURS_MAX} hours; stop it and start a \
             new one"
        )));
    }

    for pair in bytes.chunks_exact(PCM16_SAMPLE_BYTES as usize) {
        let &[low, high] = pair else { continue };

        active
            .writer
            .write_sample(i16::from_le_bytes([low, high]))
            .map_err(|error| AppError::Audio(format!("recording write: {error}")))?;
    }

    debug_assert_eq!(u64::from(active.writer.len()), samples_before + samples_chunk);

    drop(guard);

    Ok(())
}

fn recording_finish_blocking(slot: &RecordingSlot) -> AppResult<String> {
    let RecordingActive { path, writer, .. } = recording_take(slot)?;
    let sample_count = writer.len();

    writer
        .finalize()
        .map_err(|error| AppError::Audio(format!("recording finalize: {error}")))?;

    if sample_count == 0 {
        fs::remove_file(&path)?;

        return Err(AppError::Audio(
            "nothing was recorded; check that the microphone is plugged in and not muted".into(),
        ));
    }

    debug_assert!(sample_count > 0);
    debug_assert!(path.exists());

    Ok(path.to_string_lossy().into_owned())
}

fn recording_discard_blocking(slot: &RecordingSlot) -> AppResult<()> {
    let RecordingActive { path, writer, .. } = recording_take(slot)?;

    drop(writer.finalize());

    fs::remove_file(&path)?;

    debug_assert!(!path.exists());
    debug_assert!(path.extension().is_some_and(|extension| extension == RECORDING_FILE_EXTENSION));

    Ok(())
}

fn recording_take(slot: &RecordingSlot) -> AppResult<RecordingActive> {
    let active = recording_slot_lock(slot)?
        .take()
        .ok_or_else(|| AppError::Audio("no recording is in progress".into()))?;

    debug_assert!(active.sample_rate > 0);

    Ok(active)
}

fn recording_slot_lock(
    slot: &RecordingSlot,
) -> AppResult<std::sync::MutexGuard<'_, Option<RecordingActive>>> {
    slot.lock()
        .map_err(|_| AppError::Audio("the recording state was poisoned by a crash".into()))
}

fn recordings_directory_default_resolve() -> AppResult<PathBuf> {
    let directory = match dirs::audio_dir() {
        Some(music) => music.join(RECORDINGS_FOLDER),
        None => workspace::root()?.join(RECORDINGS_FOLDER_FALLBACK),
    };

    debug_assert!(directory.is_absolute());

    Ok(directory)
}

fn recording_path_new() -> AppResult<PathBuf> {
    let directory = match settings::settings_load()?.recordings_directory {
        Some(chosen) => PathBuf::from(chosen),
        None => recordings_directory_default_resolve()?,
    };

    fs::create_dir_all(&directory).map_err(|error| {
        AppError::Audio(format!(
            "could not open the recordings folder {}: {error}; pick another one in Settings",
            directory.display()
        ))
    })?;

    let stamp = chrono::Local::now().format(RECORDING_FILE_TIME_FORMAT);
    let file_name = format!("{RECORDING_FILE_PREFIX}{stamp}.{RECORDING_FILE_EXTENSION}");
    let path = directory.join(file_name);

    if path.exists() {
        return Err(AppError::Audio(
            "a recording from this second already exists; try again in a moment".into(),
        ));
    }

    debug_assert!(path.extension().is_some_and(|extension| extension == RECORDING_FILE_EXTENSION));
    debug_assert!(path.starts_with(&directory));

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot_new() -> RecordingSlot {
        Arc::new(Mutex::new(None))
    }

    fn path_temporary(name: &str) -> PathBuf {
        let file_name = format!("scribe-recording-test-{name}-{}.wav", std::process::id());

        std::env::temp_dir().join(file_name)
    }

    #[test]
    fn appended_chunks_land_in_the_wav_file() {
        let slot = slot_new();
        let path = recording_open(&slot, 16000, path_temporary("append")).unwrap();

        let samples: Vec<u8> = [1i16, -2, 3]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();

        recording_append_blocking(&slot, &samples).unwrap();

        let finished = recording_finish_blocking(&slot).unwrap();
        let (decoded, rate) = workspace::audio_wav_load(std::path::Path::new(&finished)).unwrap();

        assert_eq!(finished, path);
        assert_eq!(rate, 16000);
        assert_eq!(decoded.len(), 3);
        assert!(decoded[1] < 0.0);

        fs::remove_file(&finished).unwrap();
    }

    #[test]
    fn an_empty_recording_is_removed_and_reported() {
        let slot = slot_new();
        let path = recording_open(&slot, 16000, path_temporary("empty")).unwrap();

        assert!(recording_finish_blocking(&slot).is_err());
        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn a_half_sample_chunk_is_rejected() {
        let slot = slot_new();
        let path = recording_open(&slot, 16000, path_temporary("half")).unwrap();

        assert!(recording_append_blocking(&slot, &[1u8]).is_err());

        recording_discard_blocking(&slot).unwrap();

        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn a_second_recording_cannot_open_while_one_is_in_progress() {
        let slot = slot_new();

        recording_open(&slot, 16000, path_temporary("first")).unwrap();

        assert!(recording_open(&slot, 16000, path_temporary("second")).is_err());

        recording_discard_blocking(&slot).unwrap();
    }
}
