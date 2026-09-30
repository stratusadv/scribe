mod blocking;
mod cancellation;
mod convert;
mod endpoints;
mod error;
mod export;
mod http;
mod jobs;
mod notes;
mod people;
mod recording;
mod secrets;
mod settings;
mod state;
mod transcription;
mod workspace;

use cancellation::task_cancel;
use endpoints::commands::{endpoints_list, service_api_key_set, service_status};
use error::AppResult;
use export::{notes_export, notes_print};
use jobs::commands::{
    job_audio_clip_get,
    job_audio_path_get,
    job_delete,
    job_meta_get,
    job_meta_update,
    job_transcript_engines_list,
    job_transcript_load,
    job_transcript_save,
    job_waveform_get,
    jobs_list,
    jobs_search,
};
use notes::commands::{
    notes_generate_remote_streaming,
    notes_load,
    notes_models_list,
    notes_save,
    notes_template_delete,
    notes_template_generate,
    notes_template_save,
    notes_templates_list,
    notes_text_rewrite_streaming,
    transcript_correct_streaming,
};
use people::commands::{
    group_remove,
    group_save,
    groups_list,
    people_list,
    person_remove,
    person_save,
};
use recording::{
    microphone_permission_install,
    recording_append,
    recording_discard,
    recording_finish,
    recording_start,
    recordings_directory_default,
};
use settings::commands::{settings_get, settings_update};
use state::AppState;
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::{Manager, WindowEvent};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use transcription::commands::{transcript_import, transcription_remote};

pub const APP_DATA_FOLDER: &str = "com.stratusadv.scribe";
const LOGS_FOLDER: &str = "logs";
const LOG_FILE_PREFIX: &str = "scribe";
const WINDOW_LABEL_MAIN: &str = "main";

#[cfg(target_os = "linux")] // tigerstyle-ignore: TS035
fn web_process_recovery_install(window: &tauri::WebviewWindow) {
    use webkit2gtk::WebViewExt;

    let installed = window.with_webview(|webview| {
        webview.inner().connect_web_process_terminated(|webview, reason| {
            tracing::error!("webkit web process terminated ({reason:?}), reloading the page");

            webview.reload();
        });
    });

    if let Err(error) = installed {
        tracing::warn!("web process recovery handler not installed: {error}");
    }
}

#[cfg(not(target_os = "linux"))] // tigerstyle-ignore: TS035
const fn web_process_recovery_install(_window: &tauri::WebviewWindow) {}
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

fn tracing_log_directory() -> PathBuf {
    if let Some(data_directory) = dirs::data_dir() {
        let path = data_directory.join(APP_DATA_FOLDER).join(LOGS_FOLDER);

        if std::fs::create_dir_all(&path).is_ok() {
            debug_assert!(path.ends_with(LOGS_FOLDER));

            return path;
        }
    }

    let fallback = std::env::temp_dir().join(APP_DATA_FOLDER).join(LOGS_FOLDER);

    debug_assert!(fallback.ends_with(LOGS_FOLDER));

    fallback
}

fn tracing_initialize() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("scribe_lib=info,info"));

    let log_directory = tracing_log_directory();

    debug_assert!(log_directory.is_absolute());

    let appender_file = tracing_appender::rolling::daily(&log_directory, LOG_FILE_PREFIX);
    let (writer_non_blocking, guard) = tracing_appender::non_blocking(appender_file);

    drop(LOG_GUARD.set(guard));

    let layer_stdout = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(true);

    let layer_file = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(writer_non_blocking);

    drop(
        tracing_subscriber::registry()
            .with(filter)
            .with(layer_stdout)
            .with(layer_file)
            .try_init(),
    );

    debug_assert!(LOG_GUARD.get().is_some());

    tracing::info!(
        target: "scribe_lib",
        "logging initialized, log directory = {}",
        log_directory.display()
    );
}

#[tauri::command]
async fn app_relaunch(app: tauri::AppHandle) {
    app.restart();
}

#[tauri::command(rename_all = "snake_case")]
async fn markdown_render_html(markdown: String) -> AppResult<String> {
    blocking::run(move || convert::markdown_to_html(&markdown)).await
}

fn window_event_handle(window: &tauri::Window, event: &WindowEvent) {
    debug_assert!(!window.label().is_empty());

    if window.label() != WINDOW_LABEL_MAIN { return; }

    if matches!(event, WindowEvent::Destroyed) {
        window.app_handle().exit(0);
    }
}

#[cfg(target_os = "linux")] // tigerstyle-ignore: TS035
fn realtime_cpu_limit_soften() {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    let read = unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &raw mut limit) };

    if read != 0 { return; }
    if limit.rlim_max == libc::RLIM_INFINITY { return; }
    if limit.rlim_cur < limit.rlim_max { return; }

    limit.rlim_cur = limit.rlim_max / 5 * 4;

    let written = unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &raw const limit) };

    tracing::info!(
        target: "scribe_lib",
        "realtime cpu limit softened to {} of {} microseconds (result {written})",
        limit.rlim_cur,
        limit.rlim_max
    );
}

// tigerstyle-ignore: TS020
#[expect(clippy::exit, reason = "the exit call lives inside tauri::generate_context!")]
pub fn run() {
    tracing_initialize();

    #[cfg(target_os = "linux")]
    realtime_cpu_limit_soften();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new())
        .setup(|app| {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL_MAIN) {
                microphone_permission_install(&window);
                web_process_recovery_install(&window);
            }

            Ok(())
        })
        .on_window_event(window_event_handle)
        .invoke_handler(tauri::generate_handler![
            task_cancel,
            transcription_remote,
            transcript_import,
            recording_start,
            recording_append,
            recording_finish,
            recording_discard,
            recordings_directory_default,
            endpoints_list,
            service_status,
            service_api_key_set,
            notes_templates_list,
            notes_template_save,
            notes_template_delete,
            notes_template_generate,
            notes_models_list,
            people_list,
            person_save,
            person_remove,
            groups_list,
            group_save,
            group_remove,
            notes_generate_remote_streaming,
            notes_text_rewrite_streaming,
            transcript_correct_streaming,
            notes_save,
            notes_load,
            notes_export,
            notes_print,
            jobs_list,
            jobs_search,
            job_meta_get,
            job_meta_update,
            job_delete,
            job_transcript_engines_list,
            job_transcript_load,
            job_transcript_save,
            job_waveform_get,
            job_audio_clip_get,
            job_audio_path_get,
            settings_get,
            settings_update,
            app_relaunch,
            markdown_render_html,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        tracing::error!(target: "scribe_lib", "tauri exited with an error: {}", error);
    }
}
