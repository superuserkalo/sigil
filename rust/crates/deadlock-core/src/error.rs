use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
    #[error("network error: {0}")]
    Network(String),
    /// The player stopped a Browse install. Nothing was written, so this reads as a state,
    /// not a failure; [`crate::install::is_cancelled`] tells a caller which one it has.
    #[error("Cancelled. Nothing was installed.")]
    Cancelled,
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
