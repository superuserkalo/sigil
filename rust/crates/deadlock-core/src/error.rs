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

    /// Whether the OS refused because something holds the file: a sharing or lock violation
    /// on Windows (errors 32 and 33), a busy file, or a permission refusal.
    pub fn is_in_use(&self) -> bool {
        match self {
            Error::Io(io) => io_in_use(io),
            _ => false,
        }
    }
}

/// The sentence a game folder write adds when the OS says the file is held open. Callers that
/// only see the message (the desktop, after a worker) look for it with [`mentions_in_use`].
pub const IN_USE_HINT: &str = "Deadlock or another program has it open: close it and try again.";

/// [`Error::is_in_use`] for a plain `std::io::Error`.
pub fn io_in_use(io: &std::io::Error) -> bool {
    matches!(
        io.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
    ) || (cfg!(windows) && matches!(io.raw_os_error(), Some(32 | 33)))
}

/// True when a write's error message carries [`IN_USE_HINT`]: the file was locked, so the same
/// change can wait for the game to close.
pub fn mentions_in_use(message: &str) -> bool {
    message.contains(IN_USE_HINT)
}
