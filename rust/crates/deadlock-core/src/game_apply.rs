//! App actions that carry out the wave 4 plans in [`crate::game_plans`] and
//! [`crate::addon_checks`]: writes to `gameinfo.gi`, to the hero settings files that hold the
//! crosshair, and to the read-only flag of slot files.
//!
//! Each one refuses without `approve` and while Deadlock runs, re-reads every file it is about to
//! change and refuses when it no longer matches what was planned, backs up first, and writes
//! atomically. Agent tools never call these; the player triggers them.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::game_plans::{
    CrosshairWritePlan, plan_crosshair_write, plan_gameinfo_repair, read_crosshair_backup,
    save_crosshair_backup,
};
use crate::util::{now_iso, write_atomic};

/// What a `gameinfo.gi` repair did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameinfoRepairDone {
    /// Lines Sigil needs that were added (the addon mount, missing search paths).
    pub inserted: Vec<String>,
    /// The player's or another tool's mounts put back from the newest backup.
    pub restored: Vec<String>,
    /// The copy of the file as it was before this write.
    pub backup: PathBuf,
}

fn refuse_unless(approve: bool, what: &str) -> Result<()> {
    if !approve {
        return Err(Error::msg(format!("{what} was not approved. Nothing was written.")));
    }
    if crate::profiles::game_running() {
        return Err(Error::msg(format!(
            "Close Deadlock first: {what} changes files the game rewrites. Nothing was written."
        )));
    }
    Ok(())
}

fn backup_beside(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg(format!("{} has no file name.", path.display())))?;
    let stamp = now_iso().replace([':', '.'], "-");
    let backup = path.with_file_name(format!("{name}.dah-backup-{stamp}"));
    fs::copy(path, &backup)?;
    Ok(backup)
}

/// Repair `gameinfo.gi` after Steam reset it, keeping the mounts the player or another tool
/// added (DMM's behaviour), from the newest `gameinfo.gi.dah-backup-*`. Returns `None` when
/// nothing needed changing. Falls back to adding only Sigil's own lines when there is no backup.
pub fn apply_gameinfo_repair(game_root: &Path, approve: bool) -> Result<Option<GameinfoRepairDone>> {
    let gameinfo = crate::deploy::gameinfo_path(game_root);
    let current = fs::read_to_string(&gameinfo).map_err(|error| {
        Error::msg(format!(
            "{} could not be read ({error}). Verify the game files in Steam. Nothing was written.",
            gameinfo.display()
        ))
    })?;
    let previous = newest_gameinfo_backup(game_root).and_then(|path| fs::read_to_string(path).ok());
    let plan = plan_gameinfo_repair(&current, previous.as_deref())?;
    if !plan.changed {
        return Ok(None);
    }
    refuse_unless(approve, "Repairing gameinfo.gi")?;
    if crate::identify::sha256_file(&gameinfo)? != plan.before_sha256 {
        return Err(Error::msg(
            "gameinfo.gi changed while Sigil was planning the repair. Try again. Nothing was written.",
        ));
    }
    let backup = backup_beside(&gameinfo)?;
    write_atomic(&gameinfo, plan.text.as_bytes())?;
    Ok(Some(GameinfoRepairDone {
        inserted: plan.inserted,
        restored: plan.restored,
        backup,
    }))
}

fn newest_gameinfo_backup(game_root: &Path) -> Option<PathBuf> {
    let mut backups: Vec<_> = crate::backups::list(&crate::backups::BackupSources::for_game(game_root))
        .into_iter()
        .filter(|entry| entry.kind == crate::backups::BackupKind::Gameinfo)
        .collect();
    backups.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
    backups.into_iter().next().map(|entry| entry.path)
}

/// Write the crosshair into every hero settings file (`crosshair` Some), or put back what was
/// there before Sigil wrote (None). The restore record goes to `backup_path` (app data) before
/// any settings file changes. Every file must still be what the plan read; when a write fails
/// part way, the files already written get their original text back. Returns the files changed.
pub fn apply_crosshair_hero_settings(
    settings_paths: &[PathBuf],
    backup_path: &Path,
    crosshair: Option<&crate::crosshair::Crosshair>,
    approve: bool,
) -> Result<usize> {
    refuse_unless(approve, "Writing the crosshair to the hero settings")?;
    let backup = read_crosshair_backup(backup_path)?;
    let plan: CrosshairWritePlan = plan_crosshair_write(settings_paths, &backup, crosshair)?;
    save_crosshair_backup(backup_path, &plan.backup)?;

    for edit in &plan.edits {
        let now = match fs::read_to_string(&edit.path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if now != edit.original {
            return Err(Error::msg(format!(
                "{} changed since Sigil read it (the game or Steam wrote it). Try again. Nothing was written.",
                edit.path.display()
            )));
        }
    }

    let mut done: Vec<&crate::game_plans::SettingsEdit> = Vec::new();
    for edit in &plan.edits {
        let result = edit
            .path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .map_err(Error::from)
            .and_then(|()| write_atomic(&edit.path, edit.updated.as_bytes()));
        if let Err(error) = result {
            for written in done.iter().rev() {
                let _ = match &written.original {
                    Some(text) => write_atomic(&written.path, text.as_bytes()),
                    None => fs::remove_file(&written.path).map_err(Error::from),
                };
            }
            return Err(Error::msg(format!(
                "Writing {} failed ({error}). The files already written were put back.",
                edit.path.display()
            )));
        }
        done.push(edit);
    }
    Ok(done.len())
}

/// Clear the read-only flag on slot files that have it, the "Make writable" action for
/// [`crate::addon_checks::read_only_files`]. Only paths that check still lists are changed, so a
/// stale or foreign path is never touched. Returns the files made writable.
pub fn make_writable(addons_dir: &Path, paths: &[PathBuf], approve: bool) -> Result<usize> {
    refuse_unless(approve, "Making mod files writable")?;
    let listed = crate::addon_checks::read_only_files(addons_dir)?;
    let mut changed = 0;
    for path in paths.iter().filter(|path| listed.contains(path)) {
        let mut permissions = fs::metadata(path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(not(unix))]
        {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
        }
        fs::set_permissions(path, permissions)?;
        changed += 1;
    }
    Ok(changed)
}

/// The game config files Sigil refuses to rewrite when the new text does not parse, DMM's
/// "malformed game configs are refused instead of rewritten" rule. Matched by exact file name,
/// so stamped backups beside them are not checked.
const GUARDED_CONFIGS: [&str; 2] = ["gameinfo.gi", "video.txt"];

/// Refuse a write of `bytes` to a guarded game config (`gameinfo.gi`, `video.txt`) when the text
/// is not well-formed KeyValues. Every game config write goes through `util::write_atomic`, which
/// calls this first, so a broken edit (or an edit of a file that was already broken) never
/// replaces the game's file. Other paths pass untouched.
pub(crate) fn refuse_malformed_config(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else {
        return Ok(());
    };
    if !GUARDED_CONFIGS.contains(&name.as_str()) {
        return Ok(());
    }
    let text = String::from_utf8_lossy(bytes);
    match keyvalues_syntax_error(&text) {
        None => Ok(()),
        Some(reason) => Err(Error::msg(format!(
            "{} would not parse after this change ({reason}), so Sigil left it as it is. If the file on \
             disk is already broken, verify the game files in Steam or restore it in Settings > Backups. \
             Nothing was written.",
            path.display()
        ))),
    }
}

/// The first syntax error in a KeyValues (KV1) text such as `gameinfo.gi` or `video.txt`, or
/// `None` when it parses. Accepts quoted and bare tokens, `//` comments, `[$PLATFORM]`
/// conditionals and `#base` lines; rejects unclosed quotes, unbalanced braces, a key with no
/// value, a block with no name, and an empty file.
pub fn keyvalues_syntax_error(text: &str) -> Option<String> {
    #[derive(PartialEq)]
    enum Token {
        Word(String),
        Open,
        Close,
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut tokens: Vec<(Token, usize)> = Vec::new();
    let mut line = 1usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        line += 1;
                        break;
                    }
                }
            }
            '{' => tokens.push((Token::Open, line)),
            '}' => tokens.push((Token::Close, line)),
            '[' => {
                // A platform conditional such as `[$WIN64]`; it qualifies the token before it.
                let start = line;
                loop {
                    match chars.next() {
                        Some(']') => break,
                        Some('\n') | None => {
                            return Some(format!("line {start}: a `[` condition is never closed"));
                        }
                        Some(_) => {}
                    }
                }
            }
            '"' => {
                let start = line;
                let mut word = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\n') => {
                            line += 1;
                            word.push('\n');
                        }
                        Some(other) => word.push(other),
                        None => return Some(format!("line {start}: a quoted string is never closed")),
                    }
                }
                tokens.push((Token::Word(word), start));
            }
            other => {
                let mut word = String::from(other);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '{' | '}' | '"') {
                        break;
                    }
                    if next == '/' {
                        let mut ahead = chars.clone();
                        ahead.next();
                        if ahead.peek() == Some(&'/') {
                            break;
                        }
                    }
                    word.push(next);
                    chars.next();
                }
                tokens.push((Token::Word(word), line));
            }
        }
    }
    if tokens.is_empty() {
        return Some("the file is empty".to_string());
    }
    let mut open: Vec<usize> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (token, at) = &tokens[index];
        match token {
            Token::Close => {
                if open.pop().is_none() {
                    return Some(format!("line {at}: a closing brace has no opening brace"));
                }
                index += 1;
            }
            Token::Open => return Some(format!("line {at}: a block has no name")),
            Token::Word(key) => match tokens.get(index + 1) {
                Some((Token::Open, brace)) => {
                    open.push(*brace);
                    index += 2;
                }
                Some((Token::Word(_), _)) => index += 2,
                Some((Token::Close, _)) | None => {
                    return Some(format!("line {at}: `{key}` has no value"));
                }
            },
        }
    }
    open.last().map(|at| format!("the block opened on line {at} is never closed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sigil-game-apply-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn nothing_is_written_without_approval() {
        let root = root("approve");
        let file = root.join("citadel_hero_settings.lst");
        let backup = root.join("backup.json");
        assert!(apply_crosshair_hero_settings(&[file.clone()], &backup, None, false).is_err());
        assert!(!file.exists() && !backup.exists());
        assert!(make_writable(&root, &[], false).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keyvalues_checker_accepts_game_configs_and_names_the_error() {
        let gameinfo = "\u{feff}\"GameInfo\"\r\n{\r\n\tgame \"Deadlock\" // name\r\n\tFileSystem\r\n\t{\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tGame\t\tcitadel/addons\r\n\t\t\tGame\t\tcitadel\r\n\t\t\tGame_LowViolence  citadel_lv [$LOWVIOLENCE]\r\n\t\t}\r\n\t}\r\n\tConVars { \"r_x\" \"1\" }\r\n}\r\n";
        assert_eq!(keyvalues_syntax_error(gameinfo), None);
        assert_eq!(keyvalues_syntax_error("\"config\"\n{\n\t\"setting.fullscreen\"\t\t\"1\"\n}\n"), None);
        assert_eq!(keyvalues_syntax_error("   // only a comment\n"), Some("the file is empty".into()));
        assert!(keyvalues_syntax_error("GameInfo\n{\n game x\n").unwrap().contains("line 2"));
        assert!(keyvalues_syntax_error("a b\n}\n").unwrap().contains("closing brace"));
        assert!(keyvalues_syntax_error("a \"b\n").unwrap().contains("never closed"));
        assert!(keyvalues_syntax_error("a { b }").unwrap().contains("`b` has no value"));
        assert!(keyvalues_syntax_error("{ a b }").unwrap().contains("no name"));
    }

    #[test]
    fn malformed_game_configs_are_refused_and_left_untouched() {
        let root = root("malformed");
        let gameinfo = root.join("gameinfo.gi");
        fs::write(&gameinfo, "GameInfo { game x }").unwrap();
        let error = crate::util::write_atomic(&gameinfo, b"GameInfo { game x ").unwrap_err().to_string();
        assert!(error.contains("gameinfo.gi") && error.contains("Nothing was written"), "{error}");
        assert_eq!(fs::read_to_string(&gameinfo).unwrap(), "GameInfo { game x }");
        crate::util::write_atomic(&gameinfo, b"GameInfo { game y }").unwrap();
        // Backups and other files are not checked.
        crate::util::write_atomic(&root.join("gameinfo.gi.dah-backup-1"), b"{").unwrap();
        crate::util::write_atomic(&root.join("video.txt"), b"\"config\" { }").unwrap();
        assert!(crate::util::write_atomic(&root.join("video.txt"), b"\"config\" {").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn make_writable_touches_only_listed_slot_files() {
        use std::os::unix::fs::PermissionsExt;
        let root = root("writable");
        let addons = root.join("game").join("citadel").join("addons");
        fs::create_dir_all(&addons).unwrap();
        let slot = addons.join("pak03_dir.vpk");
        let other = addons.join("notes.txt");
        for path in [&slot, &other] {
            fs::write(path, b"x").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o444)).unwrap();
        }
        if crate::addon_checks::read_only_files(&addons).unwrap().contains(&slot) {
            assert_eq!(make_writable(&addons, &[slot.clone(), other.clone()], true).unwrap(), 1);
            assert!(!fs::metadata(&slot).unwrap().permissions().readonly());
            assert!(fs::metadata(&other).unwrap().permissions().readonly(), "not a slot file");
        }
        let _ = fs::remove_dir_all(root);
    }
}
