//! CLI error type. Every failure carries a short code for JSON consumers and a
//! human sentence for the terminal.

use std::fmt;

pub type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::new("io", error.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("json", error.to_string())
    }
}

impl From<deadlock_core::Error> for CliError {
    fn from(error: deadlock_core::Error) -> Self {
        Self::new("core", error.to_string())
    }
}

/// Attach the command name to a domain error so the message says what failed.
pub fn core(context: impl fmt::Display, error: deadlock_core::Error) -> CliError {
    CliError::new("core", format!("{context}: {error}"))
}
