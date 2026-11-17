//! Steam launch options for Deadlock: read and write the `LaunchOptions` value Steam keeps in
//! `<Steam>/userdata/<account>/config/localconfig.vdf` under
//! `UserLocalConfigStore > Software > Valve > Steam > apps > 1422450`.
//!
//! The edit is surgical: a tokenizer finds the exact byte range of the value (or of the block
//! to insert into) and every other byte of the file stays as it was. Steam rewrites this file
//! when it exits, so a write is refused while Steam runs. The file is copied aside as
//! `localconfig.vdf.dah-backup-<stamp>` before each write, and the write is atomic.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::discovery::{DEADLOCK_APP_ID, default_steam_roots};
use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

/// The longest value accepted. Steam's own field is far shorter in practice.
pub const MAX_LAUNCH_OPTIONS_CHARS: usize = 1024;

const PATH: [&str; 6] = [
    "UserLocalConfigStore",
    "Software",
    "Valve",
    "Steam",
    "apps",
    DEADLOCK_APP_ID,
];
const KEY: &str = "LaunchOptions";

/// Every `localconfig.vdf` under the given Steam roots, most recently modified first. Pass the
/// game root too: a Steam root holding the game's library is searched as well.
pub fn find_localconfigs(game_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = default_steam_roots();
    if let Some(game_root) = game_root {
        // <root>/steamapps/common/Deadlock
        if let Some(root) = game_root.ancestors().nth(3) {
            roots.push(root.to_path_buf());
        }
    }
    find_localconfigs_in(&roots)
}

pub fn find_localconfigs_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<(PathBuf, Option<SystemTime>)> = Vec::new();
    for root in roots {
        let Ok(accounts) = fs::read_dir(root.join("userdata")) else {
            continue;
        };
        for account in accounts.flatten() {
            let path = account.path().join("config").join("localconfig.vdf");
            if let Ok(meta) = fs::metadata(&path)
                && meta.is_file()
                && !found.iter().any(|(seen, _)| *seen == path)
            {
                found.push((path, meta.modified().ok()));
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found.into_iter().map(|(path, _)| path).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Str,
    Open,
    Close,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    /// Byte range of the whole token, quotes included.
    start: usize,
    end: usize,
    text: String,
}

fn tokenize(text: &str) -> Result<Vec<Token>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                tokens.push(Token { kind: Kind::Open, start: i, end: i + 1, text: String::new() });
                i += 1;
            }
            b'}' => {
                tokens.push(Token { kind: Kind::Close, start: i, end: i + 1, text: String::new() });
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut value = Vec::new();
                loop {
                    match bytes.get(i) {
                        None => return Err(Error::msg("localconfig.vdf has an unclosed string.")),
                        Some(b'\\') => {
                            match bytes.get(i + 1) {
                                Some(&next @ (b'"' | b'\\')) => value.push(next),
                                Some(b'n') => value.push(b'\n'),
                                Some(b't') => value.push(b'\t'),
                                Some(&next) => {
                                    value.push(b'\\');
                                    value.push(next);
                                }
                                None => {
                                    return Err(Error::msg(
                                        "localconfig.vdf has an unclosed string.",
                                    ));
                                }
                            }
                            i += 2;
                        }
                        Some(b'"') => {
                            i += 1;
                            break;
                        }
                        Some(&byte) => {
                            value.push(byte);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token {
                    kind: Kind::Str,
                    start,
                    end: i,
                    text: String::from_utf8_lossy(&value).into_owned(),
                });
            }
            _ => {
                // An unquoted word: Steam does not write these, but KeyValues allows them.
                let start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'"')
                {
                    i += 1;
                }
                tokens.push(Token {
                    kind: Kind::Str,
                    start,
                    end: i,
                    text: text[start..i].to_string(),
                });
            }
        }
    }
    Ok(tokens)
}

/// Where the edit lands.
#[derive(Debug)]
struct Located {
    /// `(open brace token index, depth)` for each block of `PATH` found, outermost first.
    blocks: Vec<(usize, usize)>,
    /// The value token of `LaunchOptions` inside the app block.
    value: Option<usize>,
}

fn locate(tokens: &[Token]) -> Result<Located> {
    let mut stack: Vec<String> = Vec::new();
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let mut value = None;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].kind {
            Kind::Str => match tokens.get(i + 1).map(|t| t.kind) {
                Some(Kind::Open) => {
                    stack.push(tokens[i].text.clone());
                    let depth = stack.len();
                    if depth <= PATH.len()
                        && stack
                            .iter()
                            .zip(PATH.iter())
                            .all(|(have, want)| have.eq_ignore_ascii_case(want))
                        && blocks.len() == depth - 1
                    {
                        blocks.push((i + 1, depth));
                    }
                    i += 2;
                }
                Some(Kind::Str) => {
                    let in_app = stack.len() == PATH.len()
                        && blocks.len() == PATH.len()
                        && stack
                            .iter()
                            .zip(PATH.iter())
                            .all(|(have, want)| have.eq_ignore_ascii_case(want));
                    if in_app
                        && tokens[i].text.eq_ignore_ascii_case(KEY)
                        && value.is_none()
                    {
                        value = Some(i + 1);
                    }
                    i += 2;
                }
                _ => return Err(Error::msg("localconfig.vdf ends after a key. Refusing to edit.")),
            },
            Kind::Close => {
                if stack.pop().is_none() {
                    return Err(Error::msg("localconfig.vdf has an extra closing brace."));
                }
                i += 1;
            }
            Kind::Open => return Err(Error::msg("localconfig.vdf has a block without a key.")),
        }
    }
    if !stack.is_empty() {
        return Err(Error::msg("localconfig.vdf has an unclosed block."));
    }
    Ok(Located { blocks, value })
}

/// The launch options set for Deadlock in this file's text. None when there are none.
pub fn read_launch_options(text: &str) -> Result<Option<String>> {
    let tokens = tokenize(text)?;
    let located = locate(&tokens)?;
    Ok(located.value.map(|index| tokens[index].text.clone()))
}

fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Clean a value typed by the user: one line, trimmed, bounded.
pub fn clean_value(value: &str) -> Result<String> {
    let value = value.replace(['\r', '\n', '\t'], " ").trim().to_string();
    if value.chars().count() > MAX_LAUNCH_OPTIONS_CHARS {
        return Err(Error::msg(format!(
            "Launch options are limited to {MAX_LAUNCH_OPTIONS_CHARS} characters."
        )));
    }
    Ok(value)
}

/// The file's text with Deadlock's launch options set to `value`. Only the value, or one
/// inserted key or block, changes. Refuses a file whose shape it does not recognise.
pub fn set_launch_options(text: &str, value: &str) -> Result<String> {
    let value = clean_value(value)?;
    let tokens = tokenize(text)?;
    let located = locate(&tokens)?;
    let nl = line_ending(text);

    if let Some(index) = located.value {
        let token = &tokens[index];
        let mut out = String::with_capacity(text.len() + value.len());
        out.push_str(&text[..token.start]);
        out.push_str(&quote(&value));
        out.push_str(&text[token.end..]);
        return Ok(out);
    }

    // Need at least the `apps` block to insert into.
    let apps_depth = PATH.len() - 1;
    if located.blocks.len() < apps_depth {
        return Err(Error::msg(
            "localconfig.vdf has no apps section to add launch options to. Start Deadlock from Steam once, then try again.",
        ));
    }
    let (open, depth) = located.blocks[located.blocks.len().min(PATH.len()) - 1];
    let at = tokens[open].end;
    let tabs = |n: usize| "\t".repeat(n);
    let insert = if located.blocks.len() == PATH.len() {
        // Inside the app block: one key.
        format!("{nl}{}\"{KEY}\"\t\t{}", tabs(depth), quote(&value))
    } else {
        // Inside apps: the app block with one key.
        format!(
            "{nl}{t}\"{DEADLOCK_APP_ID}\"{nl}{t}{{{nl}{t1}\"{KEY}\"\t\t{v}{nl}{t}}}",
            t = tabs(depth),
            t1 = tabs(depth + 1),
            v = quote(&value),
        )
    };
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..at]);
    out.push_str(&insert);
    out.push_str(&text[at..]);
    Ok(out)
}

/// Read the launch options from a localconfig.vdf on disk.
pub fn read_file(path: &Path) -> Result<Option<String>> {
    let text = fs::read_to_string(path)?;
    read_launch_options(&text)
}

#[derive(Debug, Clone)]
pub struct LaunchOptionsWrite {
    pub path: PathBuf,
    pub backup: PathBuf,
    pub changed: bool,
}

/// Is a Steam client process running? Steam rewrites localconfig.vdf on exit, so writes wait
/// until it is closed.
pub fn steam_running() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).to_lowercase().contains("steam.exe"))
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("pgrep")
            .args(["-x", "steam_osx"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("pgrep")
            .args(["-x", "steam"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }
}

/// GAME-ADJACENT WRITE. Set Deadlock's Steam launch options. Refuses without `approve` and
/// while Steam runs. Copies the file aside first and writes atomically. Called only from the
/// Save button in Settings after its confirm.
pub fn write_file(path: &Path, value: &str, approve: bool) -> Result<LaunchOptionsWrite> {
    write_file_with(path, value, approve, steam_running())
}

pub fn write_file_with(
    path: &Path,
    value: &str,
    approve: bool,
    steam_is_running: bool,
) -> Result<LaunchOptionsWrite> {
    if !approve {
        return Err(Error::msg("Saving launch options was not approved. Nothing was written."));
    }
    if steam_is_running {
        return Err(Error::msg(
            "Steam is running and would overwrite the change when it exits. Quit Steam, then save again.",
        ));
    }
    let text = fs::read_to_string(path)?;
    let next = set_launch_options(&text, value)?;
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "localconfig.vdf".to_string());
    let backup = path.with_file_name(format!(
        "{name}{}{}-{}",
        crate::backups::STAMPED_MARKER,
        now_iso().replace([':', '.'], "-"),
        std::process::id()
    ));
    if next == text {
        return Ok(LaunchOptionsWrite { path: path.to_path_buf(), backup, changed: false });
    }
    fs::copy(path, &backup)?;
    write_atomic(path, next.as_bytes())?;
    Ok(LaunchOptionsWrite { path: path.to_path_buf(), backup, changed: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITH_VALUE: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1422450\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1700000000\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t}\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-dota\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    fn without_key() -> String {
        WITH_VALUE.replace("\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n", "")
    }

    fn without_app() -> String {
        "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-dota\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n".to_string()
    }

    #[test]
    fn reads_only_deadlocks_value() {
        assert_eq!(read_launch_options(WITH_VALUE).unwrap().as_deref(), Some("-novid"));
        assert_eq!(read_launch_options(&without_key()).unwrap(), None);
        assert_eq!(read_launch_options(&without_app()).unwrap(), None);
    }

    #[test]
    fn replaces_the_value_and_nothing_else() {
        let next = set_launch_options(WITH_VALUE, "-novid +fps_max 240").unwrap();
        assert_eq!(next, WITH_VALUE.replace("\"-novid\"", "\"-novid +fps_max 240\""));
        assert_eq!(
            read_launch_options(&next).unwrap().as_deref(),
            Some("-novid +fps_max 240")
        );
        // Dota's options are untouched.
        assert!(next.contains("\"-dota\""));
    }

    #[test]
    fn inserts_the_key_into_the_app_block() {
        let next = set_launch_options(&without_key(), "-high").unwrap();
        assert_eq!(read_launch_options(&next).unwrap().as_deref(), Some("-high"));
        assert!(next.contains("\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-high\""));
        assert!(next.contains("\"LastPlayed\""));
    }

    #[test]
    fn inserts_the_app_block_case_insensitively() {
        let next = set_launch_options(&without_app(), "-novid").unwrap();
        assert_eq!(read_launch_options(&next).unwrap().as_deref(), Some("-novid"));
        assert!(next.contains("\"-dota\""));
        // Still parses as a whole.
        tokenize(&next).and_then(|tokens| locate(&tokens)).unwrap();
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        let next = set_launch_options(WITH_VALUE, "+exec \"my cfg\" C:\\x").unwrap();
        assert_eq!(
            read_launch_options(&next).unwrap().as_deref(),
            Some("+exec \"my cfg\" C:\\x")
        );
    }

    #[test]
    fn keeps_crlf_endings() {
        let crlf = without_key().replace('\n', "\r\n");
        let next = set_launch_options(&crlf, "-novid").unwrap();
        assert!(!next.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn refuses_unknown_shapes() {
        assert!(set_launch_options("\"UserLocalConfigStore\"\n{\n}\n", "-x").is_err());
        assert!(set_launch_options("\"a\"\n{\n", "-x").is_err());
        assert!(set_launch_options("\"a\" \"b\" }", "-x").is_err());
    }

    #[test]
    fn cleans_values() {
        assert_eq!(clean_value("  -novid\n-high ").unwrap(), "-novid -high");
        assert!(clean_value(&"x".repeat(MAX_LAUNCH_OPTIONS_CHARS + 1)).is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dah-launchopts-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_with_a_backup_and_refuses_while_steam_runs() {
        let root = scratch("write");
        let config = root.join("userdata").join("123").join("config");
        fs::create_dir_all(&config).unwrap();
        let path = config.join("localconfig.vdf");
        fs::write(&path, WITH_VALUE).unwrap();

        assert!(write_file_with(&path, "-high", false, false).is_err());
        assert!(write_file_with(&path, "-high", true, true).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), WITH_VALUE);

        let write = write_file_with(&path, "-high", true, false).unwrap();
        assert!(write.changed);
        assert_eq!(fs::read_to_string(&write.backup).unwrap(), WITH_VALUE);
        assert_eq!(read_file(&path).unwrap().as_deref(), Some("-high"));
        assert_eq!(
            crate::backups::target_for(&write.backup).as_deref(),
            Some(path.as_path())
        );

        assert_eq!(find_localconfigs_in(&[root.clone()]), vec![path.clone()]);
        fs::remove_dir_all(root).unwrap();
    }
}
