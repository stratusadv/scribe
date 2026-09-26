use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppError {
    #[error("api error: {0}")]
    API(String),

    #[error("audio error: {0}")]
    Audio(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("export error: {0}")]
    Export(String),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("task error: {0}")]
    Task(String),

    #[error("transcription error: {0}")]
    Transcription(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub(crate) type AppResult<T> = Result<T, AppError>;
