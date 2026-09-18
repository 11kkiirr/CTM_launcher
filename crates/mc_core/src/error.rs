use thiserror::Error;

/// Convenience alias used throughout `mc_core`.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Domain-level error type for the launcher engine.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to (de)serialize data: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("archive error: {0}")]
    Archive(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("instance error: {0}")]
    Instance(String),

    #[error("installer error: {0}")]
    Install(String),

    #[error("modpack error: {0}")]
    Modpack(String),

    #[error("modrinth error: {0}")]
    Modrinth(String),

    #[error("launch error: {0}")]
    Launch(String),

    #[error("skin error: {0}")]
    Skin(String),

    #[error("integrity check failed for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn other(msg: impl Into<String>) -> Self {
        CoreError::Other(msg.into())
    }
}
