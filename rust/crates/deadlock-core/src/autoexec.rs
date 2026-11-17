//! The game autoexec file: reading it, editing it line by line, and writing it behind approval.
//!
//! Deadlock reads `game/citadel/cfg/autoexec.cfg` at launch, the same location
//! as other Source 2 titles: the `cfg` folder sits beside `gameinfo.gi` inside
//! the `citadel` game folder. The path is derived from the game root, never from
//! a working directory.
//!
//! Reading is free. [`write_autoexec`] refuses without an approval flag, copies
//! an existing file to `autoexec.cfg.dah-backup-<timestamp>` first, and rejects
//! NUL bytes so a binary file is never written as configuration.
//! [`save_autoexec_if_unchanged`] adds a check that the file on disk is still the
//! one the editor opened, so a save never silently drops another writer's lines
//! (the crosshair block, a text editor).
//!
//! Editing goes through [`parse_cfg`] and [`serialise_cfg`]: every line keeps its
//! own text and line ending, so lines the editor does not understand, comments
//! and blank lines come back byte for byte. [`place_command`] decides where a
//! premade command lands: already there, over the line that sets the same
//! console variable, or at the end.
//!
//! The premade catalog ([`autoexec_presets`]) is ported from Grimoire (MIT).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

pub const AUTOEXEC_RELATIVE_PATH: &str = "game/citadel/cfg/autoexec.cfg";
pub const AUTOEXEC_FILE_NAME: &str = "autoexec.cfg";

const BOM: char = '\u{feff}';

/// Where an approved autoexec write landed and where the old file was copied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoexecWriteResult {
    pub path: PathBuf,
    /// Backup of the previous file. None when there was no file to back up.
    pub backup: Option<PathBuf>,
}

/// The outcome of [`save_autoexec_if_unchanged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoexecSave {
    Written(AutoexecWriteResult),
    /// The file on disk is not the text the editor opened. Nothing was written.
    ChangedOnDisk { current: String },
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

/// [`write_autoexec`], but only when the file on disk still matches `opened`, the text the
/// editor loaded (empty for a file that did not exist). Trailing newlines are ignored in the
/// comparison because the writer normalizes them. On a mismatch nothing is written and the
/// current text comes back so the caller can offer a reload.
pub fn save_autoexec_if_unchanged(
    game_root: &Path,
    text: &str,
    opened: &str,
    approve: bool,
) -> Result<AutoexecSave> {
    if !approve {
        return write_autoexec(game_root, text, false).map(AutoexecSave::Written);
    }
    let current = read_autoexec(game_root)?.unwrap_or_default();
    if normalize_trailing_newline(&current) != normalize_trailing_newline(opened) {
        return Ok(AutoexecSave::ChangedOnDisk { current });
    }
    write_autoexec(game_root, text, true).map(AutoexecSave::Written)
}

/// The file's lines without one trailing newline artifact: `lines` already
/// drops the final empty segment.
pub fn list_lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// Collapse trailing line endings to exactly one.
///
/// The ending is `\r\n` when the text already uses Windows line endings, else `\n`.
/// Empty or whitespace-only text is written empty, which is a valid way to
/// clear the file. Interior blank lines are kept.
pub fn normalize_trailing_newline(text: &str) -> String {
    let trimmed = text.trim_end_matches(['\r', '\n']);

    if trimmed.is_empty() {
        return String::new();
    }

    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut output = String::with_capacity(trimmed.len() + 2);
    output.push_str(trimmed);
    output.push_str(ending);
    output
}

// --- reveal --------------------------------------------------------------------------------

/// What a file manager should show for the autoexec file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevealTarget {
    /// The file exists: open its folder with the file selected.
    SelectFile(PathBuf),
    /// No file yet: open the closest folder on the way to it that exists.
    OpenFolder(PathBuf),
}

/// The autoexec file when it exists, else the nearest existing folder of
/// `game/citadel/cfg`, `game/citadel`, `game` or the game root itself.
pub fn autoexec_reveal_target(game_root: &Path) -> RevealTarget {
    let file = autoexec_path(game_root);
    if file.is_file() {
        return RevealTarget::SelectFile(file);
    }
    let mut folder = file.parent().map(Path::to_path_buf);
    while let Some(candidate) = folder {
        if candidate.is_dir() {
            return RevealTarget::OpenFolder(candidate);
        }
        if candidate == game_root {
            break;
        }
        folder = candidate.parent().map(Path::to_path_buf);
    }
    RevealTarget::OpenFolder(game_root.to_path_buf())
}

/// The single raw argument for `explorer.exe /select,"<path>"`.
///
/// Explorer parses its command line itself: a path with spaces must be quoted after the comma,
/// and forward slashes (Steam's registry stores `c:/program files (x86)/steam`) make it give up
/// and open the default folder (Documents). Pass the result through `raw_arg`, not `arg`, which
/// would wrap the whole thing in another pair of quotes.
pub fn windows_select_argument(path: &Path) -> String {
    let path = path.to_string_lossy().replace('/', "\\");
    format!("/select,\"{path}\"")
}

/// A folder path for `explorer.exe` with Windows separators.
pub fn windows_folder_argument(path: &Path) -> String {
    let path = path.to_string_lossy().replace('/', "\\");
    format!("\"{path}\"")
}

// --- parse and serialise -------------------------------------------------------------------

/// How one line ended in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
    /// The last line of a file without a final newline.
    None,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::None => "",
        }
    }
}

/// One line of a cfg file: its text without the ending, and the ending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfgLine {
    pub text: String,
    pub ending: LineEnding,
}

/// A cfg file as lines. `parse_cfg` then `serialise_cfg` gives back the input byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CfgDocument {
    /// The file started with a UTF-8 byte order mark.
    pub bom: bool,
    pub lines: Vec<CfgLine>,
}

impl CfgDocument {
    /// The ending most lines use, for new lines. `\n` for a file with none.
    pub fn preferred_ending(&self) -> LineEnding {
        let crlf = self
            .lines
            .iter()
            .filter(|line| line.ending == LineEnding::CrLf)
            .count();
        let lf = self
            .lines
            .iter()
            .filter(|line| line.ending == LineEnding::Lf)
            .count();
        if crlf > lf {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }
}

/// Split cfg text into lines. A lone `\r` stays inside the line's text.
pub fn parse_cfg(text: &str) -> CfgDocument {
    let (bom, mut rest) = match text.strip_prefix(BOM) {
        Some(stripped) => (true, stripped),
        None => (false, text),
    };
    let mut lines = Vec::new();
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(index) => {
                let (body, ending) = if index > 0 && rest.as_bytes()[index - 1] == b'\r' {
                    (&rest[..index - 1], LineEnding::CrLf)
                } else {
                    (&rest[..index], LineEnding::Lf)
                };
                lines.push(CfgLine {
                    text: body.to_string(),
                    ending,
                });
                rest = &rest[index + 1..];
            }
            None => {
                lines.push(CfgLine {
                    text: rest.to_string(),
                    ending: LineEnding::None,
                });
                rest = "";
            }
        }
    }
    CfgDocument { bom, lines }
}

/// Join lines back into text. A line other than the last that has no ending (it was the last
/// line before a reorder or an append) takes the document's preferred ending, so two lines never
/// run together.
pub fn serialise_cfg(document: &CfgDocument) -> String {
    let preferred = document.preferred_ending();
    let mut output = String::new();
    if document.bom {
        output.push(BOM);
    }
    let last = document.lines.len().saturating_sub(1);
    for (index, line) in document.lines.iter().enumerate() {
        output.push_str(&line.text);
        let ending = if index < last && line.ending == LineEnding::None {
            preferred
        } else {
            line.ending
        };
        output.push_str(ending.as_str());
    }
    output
}

/// What a line is, for display. Classification never changes the line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfgLineKind {
    Blank,
    Comment,
    Command,
}

pub fn classify_line(line: &str) -> CfgLineKind {
    let trimmed = line.trim_start();
    if trimmed.trim_end().is_empty() {
        CfgLineKind::Blank
    } else if trimmed.starts_with("//") {
        CfgLineKind::Comment
    } else {
        CfgLineKind::Command
    }
}

/// A comment line split into its marker (leading space, `//`, the spaces after it) and its
/// words. `prefix + body` is the line. None for a line that is not a comment.
pub fn split_comment(line: &str) -> Option<(&str, &str)> {
    if classify_line(line) != CfgLineKind::Comment {
        return None;
    }
    let start = line.len() - line.trim_start().len();
    let after_marker = start + 2;
    let body_start = after_marker + (line[after_marker..].len() - line[after_marker..].trim_start().len());
    Some((&line[..body_start], &line[body_start..]))
}

/// The console variable or command a line runs: its first word, lowercased, without quotes.
/// None for a blank line, a comment, or a line that chains several statements with `;`.
pub fn command_name(line: &str) -> Option<String> {
    if classify_line(line) != CfgLineKind::Command {
        return None;
    }
    let statement = strip_trailing_comment(line).trim();
    if statement.contains(';') {
        return None;
    }
    let word = statement
        .split(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_matches('"');
    if word.is_empty() {
        None
    } else {
        Some(word.to_ascii_lowercase())
    }
}

/// Whether a line already runs `command`: the same words, case-insensitive for the name,
/// ignoring spacing, quotes around words and a trailing `//` comment.
pub fn same_command(line: &str, command: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        strip_trailing_comment(text)
            .split_whitespace()
            .map(|word| word.trim_matches('"').to_string())
            .collect()
    };
    if classify_line(line) != CfgLineKind::Command {
        return false;
    }
    let (a, b) = (words(line), words(command));
    if a.is_empty() || a.len() != b.len() {
        return false;
    }
    a[0].eq_ignore_ascii_case(&b[0]) && a[1..] == b[1..]
}

/// Where a premade command goes in a list of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Line `n` already runs this exact command.
    Present(usize),
    /// Line `n` sets the same console variable to another value; replace it.
    Replace(usize),
    /// Nothing sets it yet; add it at the end.
    Append,
}

pub fn place_command<S: AsRef<str>>(lines: &[S], command: &str) -> Placement {
    if let Some(index) = lines
        .iter()
        .position(|line| same_command(line.as_ref(), command))
    {
        return Placement::Present(index);
    }
    let Some(name) = command_name(command) else {
        return Placement::Append;
    };
    match lines
        .iter()
        .position(|line| command_name(line.as_ref()).as_deref() == Some(name.as_str()))
    {
        Some(index) => Placement::Replace(index),
        None => Placement::Append,
    }
}

/// The text before a `//` that is not inside quotes.
fn strip_trailing_comment(line: &str) -> &str {
    let mut quoted = false;
    let bytes = line.as_bytes();
    for index in 0..bytes.len() {
        match bytes[index] {
            b'"' => quoted = !quoted,
            b'/' if !quoted && bytes.get(index + 1) == Some(&b'/') => return &line[..index],
            _ => {}
        }
    }
    line
}

// --- premade commands ----------------------------------------------------------------------

const PRESETS: &str = include_str!("../data/autoexec-presets.json");

/// One premade command: a name and a line to add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoexecPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoexecPresetCategory {
    pub id: String,
    pub name: String,
    pub presets: Vec<AutoexecPreset>,
}

#[derive(Deserialize)]
struct PresetBundle {
    categories: Vec<AutoexecPresetCategory>,
}

/// The premade catalog, grouped by what the player wants: Grimoire's full list.
pub fn autoexec_presets() -> &'static [AutoexecPresetCategory] {
    static CATALOG: OnceLock<Vec<AutoexecPresetCategory>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str::<PresetBundle>(PRESETS)
            .expect("bundled autoexec presets parse")
            .categories
    })
}

/// The premade command a line matches exactly, for a quiet label beside it.
pub fn preset_for_line(line: &str) -> Option<&'static AutoexecPreset> {
    autoexec_presets()
        .iter()
        .flat_map(|category| category.presets.iter())
        .find(|preset| same_command(line, &preset.command))
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

pub mod launch_options;

#[cfg(test)]
mod tests;
