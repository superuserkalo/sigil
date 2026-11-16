//! The game autoexec file: reading it, and writing it behind approval.
//!
//! Deadlock reads `game/citadel/cfg/autoexec.cfg` at launch, the same location
//! as other Source 2 titles: the `cfg` folder sits beside `gameinfo.gi` inside
//! the `citadel` game folder. The path is derived from the game root, never from
//! a working directory.
//!
//! Reading is free. [`write_autoexec`] refuses without an approval flag, copies
//! an existing file to `autoexec.cfg.dah-backup-<timestamp>` first, and rejects
//! NUL bytes so a binary file is never written as configuration.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

pub const AUTOEXEC_RELATIVE_PATH: &str = "game/citadel/cfg/autoexec.cfg";
pub const AUTOEXEC_FILE_NAME: &str = "autoexec.cfg";

/// Where an approved autoexec write landed and where the old file was copied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoexecWriteResult {
    pub path: PathBuf,
    /// Backup of the previous file. None when there was no file to back up.
    pub backup: Option<PathBuf>,
}

pub fn autoexec_path(game_root: &Path) -> PathBuf {
    game_root
        .join("game")
        .join("citadel")
        .join("cfg")
        .join(AUTOEXEC_FILE_NAME)
}

/// Read the autoexec file. None means the file does not exist.
pub fn read_autoexec(game_root: &Path) -> Result<Option<String>> {
    let path = autoexec_path(game_root);

    match fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::msg(format!(
            "Could not read {}: {error}",
            path.display()
        ))),
    }
}

/// Write the autoexec file. Refuses without `approve`.
///
/// The text is normalized to one trailing newline, the `cfg` folder is created
/// when missing, and an existing file is copied beside itself before the write.
pub fn write_autoexec(
    game_root: &Path,
    text: &str,
    approve: bool,
) -> Result<AutoexecWriteResult> {
    let path = autoexec_path(game_root);

    if !approve {
        return Err(Error::msg(format!(
            "Writing {} was not approved. Nothing was written.",
            path.display()
        )));
    }

    if text.contains('\0') {
        return Err(Error::msg(format!(
            "Refusing to write {}: the text contains a NUL byte and is not configuration.",
            path.display()
        )));
    }

    let normalized = normalize_trailing_newline(text);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let backup = if path.is_file() {
        let backup = backup_path_for(&path);
        fs::copy(&path, &backup).map_err(|error| {
            Error::msg(format!(
                "Could not back up {} to {}: {error}",
                path.display(),
                backup.display()
            ))
        })?;
        Some(backup)
    } else {
        None
    };

    write_atomic(&path, normalized.as_bytes())?;

    Ok(AutoexecWriteResult { path, backup })
}

/// The file's lines without one trailing newline artifact: `lines` already
/// drops the final empty segment.
pub fn list_lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// Collapse trailing line endings to exactly one `\n`.
///
/// Empty or whitespace-only text is written empty, which is a valid way to
/// clear the file. Interior blank lines are kept.
pub fn normalize_trailing_newline(text: &str) -> String {
    let trimmed = text.trim_end_matches(['\r', '\n']);

    if trimmed.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(trimmed.len() + 1);
    output.push_str(trimmed);
    output.push('\n');
    output
}

fn backup_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| AUTOEXEC_FILE_NAME.to_string());

    path.with_file_name(format!("{name}.dah-backup-{}", backup_stamp()))
}

fn backup_stamp() -> String {
    now_iso().replace([':', '.'], "-")
}
