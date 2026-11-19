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
/// A command that never flashes a console window on Windows (`CREATE_NO_WINDOW`). Every
/// console tool Sigil starts (`tasklist`, `taskkill`, `cmd /C start`, 7-Zip, the build tools)
/// goes through this, because a black window popping up and closing looks alarming. Output
/// capture and exit codes are unaffected. On other systems it is a plain `Command`.
pub fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

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

#[cfg(test)]
mod console_tests {
    /// Every console program Sigil starts on Windows must go through [`super::hidden_command`]
    /// (or set `CREATE_NO_WINDOW` itself), or a black window flashes up. Scans both crates.
    #[test]
    fn console_tools_never_flash_a_window() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let consoles = ["\"tasklist\"", "\"taskkill\"", "\"cmd\"", "\"netsh\"", "\"reg\"", "\"powershell\"", "\"where\""];
        let mut bad = Vec::new();
        let mut stack = vec![root.join("deadlock-core/src"), root.join("desktop/src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                for (index, line) in lines.iter().enumerate() {
                    if !line.contains("Command::new(") || !consoles.iter().any(|name| line.contains(name)) {
                        continue;
                    }
                    let window = lines[index..lines.len().min(index + 12)].join("\n");
                    if !window.contains("creation_flags") {
                        bad.push(format!("{}:{}", path.display(), index + 1));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "start these through util::hidden_command: {bad:?}");
    }
}
