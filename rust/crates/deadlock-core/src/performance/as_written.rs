//! Apply a config "as written" (Grimoire issue #590): the author's own `ConVars` values, in
//! their order, instead of Sigil's per-value edits.
//!
//! Safety (lead review, 2026-11-21): only the author's `ConVars` section is read, and from it
//! only entries `repos::allowed` passes (a lowercase convar set to a number or true/false, never
//! a console command). The author's comments are never carried over: Sigil's undo reads markers
//! out of comments, so a comment in the game file must never be author text. Every other section of the author's file
//! (`FileSystem`, `SearchPaths`, anything else) is ignored: the player's own file stays as it
//! is around the merged `ConVars` body, so the addons folders and Sigil's slot folders keep
//! loading. Custom presets from GitHub repositories (`repos::ID_PREFIX`) never get this mode;
//! they go through the normal allowlisted apply only.
//!
//! The BEGIN and END marker lines sit above the file, so the page still knows what is applied,
//! and the stock file is kept beside it (`gameinfo.gi.sigil-stock`) so Turn off puts it back
//! exactly. The player's switches and convar edits do not apply in this mode.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use super::{
    BACKUP_SUFFIX, Config, FOREIGN_MARKER, MARKER, Patched, brace_count, fetch, find_section_by_path,
    gameinfo_path, remove_markers,
};
use crate::error::{Error, Result};
use crate::util::write_atomic;

/// The tag on the marker lines of a file applied as written.
pub const TAG: &str = "sigil-perf as-written";

const NOT_WHOLE: &str = "This config does not ship a whole gameinfo.gi, so it cannot be used as written.";
const CUSTOM: &str = "A custom preset from a GitHub repository cannot be applied as written.";

fn search_paths() -> [String; 2] {
    ["FileSystem".to_string(), "SearchPaths".to_string()]
}

fn convars() -> [String; 1] {
    ["ConVars".to_string()]
}

/// The copy of the stock file taken before an as-written apply, beside `gameinfo.gi`.
pub fn stock_path(game_root: &Path) -> PathBuf {
    gameinfo_path(game_root).with_extension("gi.sigil-stock")
}

/// Whether `text` is a whole gameinfo file: a `ConVars` and a `FileSystem` > `SearchPaths`
/// section. A ConVars block or a video-only config is not.
pub fn is_whole_file(text: &str) -> bool {
    let content = text.replace("\r\n", "\n");
    find_section_by_path(&content, &convars()).is_some() && find_section_by_path(&content, &search_paths()).is_some()
}

/// Whether `config` may be applied as written at all: never a custom preset.
pub fn offered(config: &Config) -> bool {
    !config.id().starts_with(super::repos::ID_PREFIX)
}

/// The author's file for `config`, when the cache holds a whole one and the config is not a
/// custom preset. Disk only.
pub fn author_file(data_dir: &Path, config: &Config) -> Option<String> {
    if !offered(config) {
        return None;
    }
    let text = std::fs::read_to_string(fetch::file_path(data_dir, config.id(), "gameinfo.gi")).ok()?;
    is_whole_file(&text).then_some(text)
}

/// Whether the install's `gameinfo.gi` text was applied as written.
pub fn is_applied(text: &str) -> bool {
    text.contains(TAG)
}

fn entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Key and value each fully quoted or bare; a trailing comment holds no quote or brace.
        Regex::new(r#"^[ \t]*(?:"([A-Za-z0-9_.]+)"|([A-Za-z0-9_.]+))[ \t]+(?:"([^"\n]*)"|([^\s"/{}]+))[ \t]*(?://[^{}"\n]*)?$"#).unwrap()
    })
}

/// A validated entry: key and value. A trailing comment is accepted and dropped. A line with an
/// odd number of quotes is never one.
fn entry(line: &str) -> Option<(String, String)> {
    if line.matches('"').count() % 2 != 0 {
        return None;
    }
    let caps = entry_re().captures(line)?;
    let key = caps.get(1).or_else(|| caps.get(2))?.as_str().to_string();
    let value = caps.get(3).or_else(|| caps.get(4)).map_or("", |m| m.as_str()).to_string();
    Some((key, value))
}

/// The author's `ConVars` body cut down to what may be written: allowlisted entries and blank
/// lines, each rebuilt from validated parts (never the author's raw line). Comments, nested
/// blocks and anything else are dropped. Returns the kept lines, unindented, and the
/// keys they set.
fn allowed_lines(author_body: &str) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    for line in author_body.split('\n') {
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        if depth > 0 || opens > 0 || closes > 0 {
            depth = (depth + opens).saturating_sub(closes);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            kept.push(String::new());
            continue;
        }
        if let Some((key, value)) = entry(line)
            && super::repos::allowed(&key, &value)
        {
            kept.push(format!("\"{key}\"\t\"{value}\""));
            keys.push(key);
        }
    }
    while kept.first().is_some_and(String::is_empty) {
        kept.remove(0);
    }
    while kept.last().is_some_and(String::is_empty) {
        kept.pop();
    }
    (kept, keys)
}

/// Catalog text for a comment line: one line, no control characters.
fn plain(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The player's file with the author's allowlisted `ConVars` lines merged in and Sigil's marker
/// lines on top. `base` is the install's stock file (no Sigil edits); its line endings are kept.
/// The author's other sections are never used.
pub fn apply_text(base: &str, author: &str, config: &Config) -> Result<Patched> {
    if !offered(config) {
        return Err(Error::msg(CUSTOM));
    }
    if base.contains(FOREIGN_MARKER) {
        return Err(Error::msg(
            "Grimoire has a performance config in this file. Turn it off there first.",
        ));
    }
    let crlf = base.contains("\r\n");
    let base = base.replace("\r\n", "\n");
    let mut author = author.replace("\r\n", "\n");
    if author.contains(MARKER) || author.contains(FOREIGN_MARKER) {
        author = remove_markers(&author);
    }
    if !is_whole_file(&author) {
        return Err(Error::msg(NOT_WHOLE));
    }
    let (start, end) = find_section_by_path(&author, &convars()).ok_or_else(|| Error::msg(NOT_WHOLE))?;
    let Some((cur_start, cur_end)) = find_section_by_path(&base, &convars()) else {
        return Err(Error::msg(
            "Your gameinfo.gi has no ConVars section. Verify the game files in Steam, then try again.",
        ));
    };
    let (lines, keys) = allowed_lines(&author[start..end]);
    if keys.is_empty() {
        return Err(Error::msg("This config sets no value Sigil can write as written."));
    }
    let current_body = &base[cur_start..cur_end];
    let indent = current_body
        .split('\n')
        .find(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>())
        .unwrap_or_else(|| "\t\t".to_string());
    // The player's own entries stay unless the author sets the same key.
    let mut body: Vec<String> = current_body
        .split('\n')
        .filter(|line| entry(line).is_none_or(|(key, _)| !keys.iter().any(|k| k.eq_ignore_ascii_case(&key))))
        .map(str::to_string)
        .collect();
    while body.last().is_some_and(|line| line.trim().is_empty()) {
        body.pop();
    }
    let closing_indent = current_body.rsplit('\n').next().unwrap_or_default().to_string();
    body.push(format!("{indent}// {}, as written [{TAG}]", plain(config.name())));
    body.extend(lines.into_iter().map(|line| if line.is_empty() { line } else { format!("{indent}{line}") }));
    let merged = format!("{}\n{closing_indent}", body.join("\n"));
    let text_body = format!("{}{}{}", &base[..cur_start], merged, &base[cur_end..]);
    if brace_count(&text_body) != brace_count(&base) || find_section_by_path(&text_body, &convars()).is_none() {
        return Err(Error::msg("The change would unbalance gameinfo.gi, so nothing was written."));
    }
    let commit = &config.commit[..config.commit.len().min(12)];
    let mut text = format!(
        "// ==== Sigil Performance Config BEGIN (preset={} v{} @{commit}) ====\n\
         // {}: the author's ConVars lines, as written, from {} ({}). Only plain values are used; the rest of the file is yours. [{TAG}]\n\
         // turn it off in Sigil > Performance to put your stock file back [{TAG}]\n\
         // ==== Sigil Performance Config END ====\n",
        plain(config.id()),
        plain(&config.version),
        plain(config.name()),
        plain(&config.credit()),
        plain(config.source.license),
    );
    text.push_str(&text_body);
    let written = keys.len();
    Ok(Patched {
        text: if crlf { text.replace('\n', "\r\n") } else { text },
        skipped: Vec::new(),
        written,
    })
}

/// GAME WRITE. Apply `config` as written. Called only from the "Apply as written" confirm.
/// The stock file (Sigil's own edits taken out) is kept beside it first, unless the file on
/// disk is already an as-written apply, whose stock copy stays and is the base again.
pub fn apply_to_install(game_root: &Path, data_dir: &Path, config: &Config) -> Result<Patched> {
    if !offered(config) {
        return Err(Error::msg(CUSTOM));
    }
    let author = author_file(data_dir, config).ok_or_else(|| Error::msg(NOT_WHOLE))?;
    let path = gameinfo_path(game_root);
    let current = std::fs::read_to_string(&path)?;
    let stock = stock_path(game_root);
    let base = if is_applied(&current) {
        std::fs::read_to_string(&stock).map_err(|_| {
            Error::msg("The stock copy of gameinfo.gi is missing. Verify the game files in Steam, then try again.")
        })?
    } else {
        super::remove_text(&current)
    };
    let patched = apply_text(&base, &author, config)?;
    if !is_applied(&current) {
        write_atomic(&stock, base.as_bytes())?;
    }
    write_atomic(&path, patched.text.as_bytes())?;
    Ok(patched)
}

/// GAME WRITE, from Turn off: put the stock copy back and drop it. Falls back to the one-time
/// backup Sigil keeps (`gameinfo.gi.sigil-bak`). True when the file changed.
pub(super) fn remove_from_install(game_root: &Path) -> Result<bool> {
    let path = gameinfo_path(game_root);
    let stock = stock_path(game_root);
    let backup = gameinfo_path(game_root).with_extension(format!("gi.{BACKUP_SUFFIX}"));
    let text = match std::fs::read_to_string(&stock) {
        Ok(text) => text,
        Err(_) => std::fs::read_to_string(&backup).map_err(|_| {
            Error::msg("The stock copy of gameinfo.gi is missing. Verify the game files in Steam to get it back.")
        })?,
    };
    write_atomic(&path, text.as_bytes())?;
    let _ = std::fs::remove_file(&stock);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHOR: &str = "// Tuned by an author\n\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel\n\t\t}\n\t}\n\n\t// my favourite section\n\tConVars\n\t{\n\t\tr_shadows \"0\"   // no shadows\n\t}\n}\n";
    const CURRENT: &str = "\"GameInfo\"\r\n{\r\n\tFileSystem\r\n\t{\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tGame citadel/addons\r\n\t\t\tGame citadel/sigil-addons2\r\n\t\t\tGame citadel\r\n\t\t}\r\n\t}\r\n\tConVars\r\n\t{\r\n\t\tr_shadows \"1\"\r\n\t}\r\n}\r\n";

    fn config() -> Config {
        let def = crate::performance::catalog::config("sqooky").expect("known config");
        Config {
            def,
            source: crate::performance::catalog::source(def.source),
            version: "3.9".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            date: "2026-11-04".into(),
            path: "gameinfo.gi".into(),
            body: crate::performance::Body::default(),
            videos: Vec::new(),
            ignored: Vec::new(),
            unknown: Vec::new(),
        }
    }

    #[test]
    fn keeps_the_authors_lines_and_the_installs_search_paths() {
        let patched = apply_text(CURRENT, AUTHOR, &config()).unwrap();
        let text = patched.text.replace("\r\n", "\n");
        assert!(patched.text.contains("\r\n"), "the install's line endings are kept");
        assert!(text.contains("\"r_shadows\"\t\"0\"\n"), "rebuilt from the validated parts");
        assert!(!text.contains("no shadows"), "the author's comments are not carried over");
        assert!(!text.contains("r_shadows \"1\""), "the author's value replaces the player's");
        assert!(text.contains("Game citadel/addons\n\t\t\tGame citadel/sigil-addons2"));
        assert!(!text.contains("Tuned by an author"), "nothing outside ConVars is taken");
        assert!(is_applied(&text));
        assert!(matches!(crate::performance::applied(&text), crate::performance::Applied::Config { .. }));
    }

    /// Lead review: an author's extra SearchPaths or FileSystem block and a value outside the
    /// allowlist never reach the install.
    #[test]
    fn only_allowlisted_convars_are_taken() {
        let author = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame evil/folder\n\t\t\tGame citadel\n\t\t}\n\t}\n\tConVars\n\t{\n\t\tr_shadows \"0\"\n\t\tsv_cheats \"hello\"\n\t\texec_this \"1\"\n\t\tbind \"x\"\n\t\tSearchPaths\n\t\t{\n\t\t\tGame evil/inner\n\t\t}\n\t}\n}\n";
        let patched = apply_text(CURRENT, author, &config()).unwrap();
        let text = patched.text;
        assert!(text.contains("\"r_shadows\"\t\"0\""));
        for bad in ["evil", "sv_cheats", "exec_this", "bind"] {
            assert!(!text.contains(bad), "{bad} must not be written");
        }
        assert_eq!(text.matches("SearchPaths").count(), 1);
    }

    /// Lead review: an unbalanced quote cannot smuggle extra tokens past the checks.
    #[test]
    fn unbalanced_quotes_are_dropped() {
        let author = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel\n\t\t}\n\t}\n\tConVars\n\t{\n\t\tr_shadows \"0\"\n\t\t\"fps_max 1\n\t\tr_x \"1\" \"sv_cheats\" \"1\"\n\t\tr_y 1 \"evil\n\t\t// note \"quoted\n\t}\n}\n";
        let text = apply_text(CURRENT, author, &config()).unwrap().text;
        assert!(text.contains("\"r_shadows\"\t\"0\""));
        for bad in ["fps_max", "sv_cheats", "r_x", "r_y", "evil", "quoted"] {
            assert!(!text.contains(bad), "{bad} must not be written");
        }
        assert_eq!(text.matches('"').count() % 2, 0);
    }

    /// Security review: Sigil's undo reads markers out of comments, so an author's comment must
    /// never reach the file, or a later apply or Turn off would restore what it says.
    #[test]
    fn an_authors_comment_cannot_pose_as_a_sigil_marker() {
        let author = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel\n\t\t}\n\t}\n\tConVars\n\t{\n\t\t//sigil-perf removed: sv_cheats 1\n\t\t// sigil-perf removed: exec evil\n\t\tr_shadows 0 //sigil-perf was evil\n\t\tr_bloom 1 // // sigil-perf was \u{1b}evil\n\t}\n}\n";
        let text = apply_text(CURRENT, author, &config()).unwrap().text;
        for bad in ["sv_cheats", "exec evil", "was evil", "\u{1b}"] {
            assert!(!text.contains(bad), "{bad:?} must not be written");
        }
        assert!(!crate::performance::remove_text(&text).contains("evil"));
    }

    /// A normal apply over an as-written file starts from the stock copy, not a marker undo.
    #[test]
    fn a_normal_apply_goes_back_to_the_stock_copy() {
        let root = std::env::temp_dir().join(format!("sigil-as-written-normal-{}", std::process::id()));
        let data = root.join("data");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(gameinfo_path(&root).parent().unwrap()).unwrap();
        std::fs::write(gameinfo_path(&root), CURRENT).unwrap();
        let config = config();
        let cached = fetch::file_path(&data, config.id(), "gameinfo.gi");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, AUTHOR).unwrap();
        apply_to_install(&root, &data, &config).unwrap();

        crate::performance::apply_to_install(&root, &config, &crate::performance::Options::default()).unwrap();
        let text = std::fs::read_to_string(gameinfo_path(&root)).unwrap();
        assert!(!is_applied(&text), "the as-written mode is gone");
        assert!(!stock_path(&root).exists(), "its stock copy is used up");
        assert_eq!(crate::performance::remove_text(&text), CURRENT);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_custom_preset_is_never_applied_as_written() {
        let mut custom = config();
        let mut def = custom.def.clone();
        def.id = "custom-someone-fps";
        custom.def = Box::leak(Box::new(def));
        assert!(!offered(&custom));
        assert!(apply_text(CURRENT, AUTHOR, &custom).is_err());
        assert!(author_file(Path::new("/nonexistent"), &custom).is_none());
    }

    #[test]
    fn a_convars_block_is_not_a_whole_file() {
        assert!(is_whole_file(AUTHOR));
        assert!(!is_whole_file("ConVars\n{\n\tr_shadows \"0\"\n}\n"));
        assert!(apply_text(CURRENT, "ConVars\n{\n\tr_shadows \"0\"\n}\n", &config()).is_err());
    }

    #[test]
    fn turn_off_puts_the_stock_file_back() {
        let root = std::env::temp_dir().join(format!("sigil-as-written-{}", std::process::id()));
        let data = root.join("data");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(gameinfo_path(&root).parent().unwrap()).unwrap();
        std::fs::write(gameinfo_path(&root), CURRENT).unwrap();
        let config = config();
        let cached = fetch::file_path(&data, config.id(), "gameinfo.gi");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, AUTHOR).unwrap();

        apply_to_install(&root, &data, &config).unwrap();
        assert!(stock_path(&root).exists());
        // A second apply keeps the first stock copy.
        apply_to_install(&root, &data, &config).unwrap();
        assert_eq!(std::fs::read_to_string(stock_path(&root)).unwrap(), CURRENT);

        assert!(crate::performance::remove_from_install(&root).unwrap());
        assert_eq!(std::fs::read_to_string(gameinfo_path(&root)).unwrap(), CURRENT);
        assert!(!stock_path(&root).exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
