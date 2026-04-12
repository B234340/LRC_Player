// Shared application error types and result alias.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serde JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Decode error: {0}")]
    Decode(String),

    #[error("Other error: {0}")]
    Other(String),
}

pub type AppResult<T> = Result<T, AppError>;
