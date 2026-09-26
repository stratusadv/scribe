use crate::error::{AppError, AppResult};

pub(crate) async fn run<T, Work>(work: Work) -> AppResult<T>
where
    T: Send + 'static,
    Work: FnOnce() -> AppResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AppError::Task(format!("blocking task failed to join: {error}")))?
}
