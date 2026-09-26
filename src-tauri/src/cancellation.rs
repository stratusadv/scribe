use crate::error::{AppError, AppResult};
use crate::state::AppState;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Manager;

const STREAM_COUNT_MAX: u32 = 64;

#[derive(Debug)]
pub(crate) struct CancellationRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl CancellationRegistry {
    pub(crate) fn new() -> Self {
        Self { flags: Mutex::new(HashMap::new()) }
    }

    pub(crate) fn register(&self, stream_id: &str) -> Arc<AtomicBool> {
        debug_assert!(!stream_id.is_empty());

        let flag = Arc::new(AtomicBool::new(false));

        if let Ok(mut guard) = self.flags.lock() {
            debug_assert!(guard.len() < STREAM_COUNT_MAX as usize);

            let previous = guard.insert(stream_id.to_owned(), Arc::clone(&flag));

            debug_assert!(previous.is_none());
        }

        debug_assert!(!flag.load(Ordering::SeqCst));

        flag
    }

    pub(crate) fn trigger(&self, stream_id: &str) {
        debug_assert!(!stream_id.is_empty());

        if let Ok(guard) = self.flags.lock() {
            if let Some(flag) = guard.get(stream_id) {
                flag.store(true, Ordering::SeqCst);

                debug_assert!(flag.load(Ordering::SeqCst));
            }
        }
    }

    pub(crate) fn clear(&self, stream_id: &str) {
        debug_assert!(!stream_id.is_empty());

        if let Ok(mut guard) = self.flags.lock() {
            drop(guard.remove(stream_id));

            debug_assert!(!guard.contains_key(stream_id));
        }
    }
}

pub(crate) fn is_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

pub(crate) fn stream_id_validate(stream_id: &str) -> AppResult<()> {
    if stream_id.is_empty() {
        return Err(AppError::Config("the stream id is empty".into()));
    }

    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn task_cancel(app: tauri::AppHandle, stream_id: String) -> AppResult<()> {
    stream_id_validate(&stream_id)?;

    app.state::<AppState>().cancellation.trigger(&stream_id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_is_cancelled_only_after_its_own_stream_is_triggered() {
        let registry = CancellationRegistry::new();
        let flag = registry.register("stream-a");
        let other = registry.register("stream-b");

        registry.trigger("stream-a");

        assert!(is_cancelled(&flag));
        assert!(!is_cancelled(&other));

        registry.clear("stream-a");
        registry.trigger("stream-a");

        assert!(is_cancelled(&flag));
        assert!(!is_cancelled(&registry.register("stream-a")));
    }

    #[test]
    fn an_empty_stream_id_is_refused_at_the_boundary() {
        assert!(stream_id_validate("").is_err());
        assert!(stream_id_validate("stream-a").is_ok());
    }
}
