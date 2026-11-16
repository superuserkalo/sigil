//! Small internal helpers shared by the domain modules.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, TimeZone, Utc};

use crate::error::Result;

pub(crate) fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn unix_seconds_to_iso(seconds: i64) -> String {
    match Utc.timestamp_opt(seconds, 0).single() {
        Some(value) => value.to_rfc3339_opts(SecondsFormat::Millis, true),
        None => String::new(),
    }
}

/// Write a file through a sibling temporary file so a crash cannot leave a half manifest.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temporary = temp_path(path);
    fs::write(&temporary, bytes)?;

    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }

    Ok(())
}

pub(crate) fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let temporary = format!("{name}.{}.tmp", std::process::id());
    path.with_file_name(temporary)
}

pub(crate) fn escape_regex(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        if matches!(
            character,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '[' | ']' | '\\' | '|'
        ) {
            output.push('\\');
        }
        output.push(character);
    }

    output
}

pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(crate) fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    match std::env::current_dir() {
        Ok(current) => current.join(path),
        Err(_) => path.to_path_buf(),
    }
}
