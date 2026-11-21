//! Apply a config "as written": the author's own `gameinfo.gi`, with its formatting, sections
//! and comments, instead of Sigil's line edits (Grimoire issue #590).
//!
//! Only a config that ships a whole file is offered (it has a `ConVars` section and a
//! `FileSystem` > `SearchPaths` section). The install's own `SearchPaths` body replaces the
//! author's, so the addons folders, Sigil's extra slot folders and a profile's own folder keep
//! loading. The BEGIN and END marker lines sit above the file, so the page still knows what is
//! applied, and the stock file is kept beside it (`gameinfo.gi.sigil-stock`) so Turn off puts it
//! back exactly. The player's switches and convar edits do not apply in this mode.

use std::path::{Path, PathBuf};

use super::{
    BACKUP_SUFFIX, Config, FOREIGN_MARKER, MARKER, Patched, brace_count, fetch, find_section_by_path,
    gameinfo_path, remove_markers,
};
use crate::error::{Error, Result};
use crate::util::write_atomic;

/// The tag on the marker lines of a file applied as written.
pub const TAG: &str = "sigil-perf as-written";

fn search_paths() -> [String; 2] {
    ["FileSystem".to_string(), "SearchPaths".to_string()]
}

/// The copy of the stock file taken before an as-written apply, beside `gameinfo.gi`.
pub fn stock_path(game_root: &Path) -> PathBuf {
    gameinfo_path(game_root).with_extension("gi.sigil-stock")
}

/// Whether `text` is a whole gameinfo file: a `ConVars` and a `FileSystem` > `SearchPaths`
/// section. A ConVars block or a video-only config is not.
pub fn is_whole_file(text: &str) -> bool {
    let content = text.replace("\r\n", "\n");
    find_section_by_path(&content, &["ConVars".to_string()]).is_some()
        && find_section_by_path(&content, &search_paths()).is_some()
}

/// The author's file for `config`, when the cache holds a whole one. Disk only.
pub fn author_file(data_dir: &Path, config: &Config) -> Option<String> {
    let text = std::fs::read_to_string(fetch::file_path(data_dir, config.id(), "gameinfo.gi")).ok()?;
    is_whole_file(&text).then_some(text)
}

/// Whether the install's `gameinfo.gi` text was applied as written.
pub fn is_applied(text: &str) -> bool {
    text.contains(TAG)
}

/// The author's file with the install's `SearchPaths` body and Sigil's marker lines on top.
/// `current` is the install's file now; its line endings are kept.
pub fn apply_text(current: &str, author: &str, config: &Config) -> Result<Patched> {
    if current.contains(FOREIGN_MARKER) {
        return Err(Error::msg(
            "Grimoire has a performance config in this file. Turn it off there first.",
        ));
    }
    let crlf = current.contains("\r\n");
    let current = current.replace("\r\n", "\n");
    let mut author = author.replace("\r\n", "\n");
    if author.contains(MARKER) || author.contains(FOREIGN_MARKER) {
        author = remove_markers(&author);
    }
    let Some((cur_start, cur_end)) = find_section_by_path(&current, &search_paths()) else {
        return Err(Error::msg(
            "Your gameinfo.gi has no SearchPaths section. Verify the game files in Steam, then try again.",
        ));
    };
    let Some((start, end)) = find_section_by_path(&author, &search_paths()) else {
        return Err(Error::msg("This config does not ship a whole gameinfo.gi, so it cannot be used as written."));
    };
    if find_section_by_path(&author, &["ConVars".to_string()]).is_none() {
        return Err(Error::msg("This config does not ship a whole gameinfo.gi, so it cannot be used as written."));
    }
    let body = format!("{}{}{}", &author[..start], &current[cur_start..cur_end], &author[end..]);
    if brace_count(&author[start..end]) % 2 != 0
        || brace_count(&current[cur_start..cur_end]) % 2 != 0
        || find_section_by_path(&body, &["ConVars".to_string()]).is_none()
    {
        return Err(Error::msg("The change would unbalance gameinfo.gi, so nothing was written."));
    }
    let commit = &config.commit[..config.commit.len().min(12)];
    let mut text = format!(
        "// ==== Sigil Performance Config BEGIN (preset={} v{} @{commit}) ====\n\
         // {}: the author's own file, as written, from {} ({}). SearchPaths are kept from your install. [{TAG}]\n\
         // turn it off in Sigil > Performance to put your stock file back [{TAG}]\n\
         // ==== Sigil Performance Config END ====\n",
        config.id(),
        config.version,
        config.name(),
        config.credit(),
        config.source.license,
    );
    text.push_str(&body);
    let written = super::convar_values(&text).len();
    Ok(Patched {
        text: if crlf { text.replace('\n', "\r\n") } else { text },
        skipped: Vec::new(),
        written,
    })
}

/// GAME WRITE. Apply `config` as written. Called only from the "Apply as written" confirm.
/// The stock file (Sigil's own edits taken out) is kept beside it first, unless the file on
/// disk is already an as-written apply, whose stock copy stays.
pub fn apply_to_install(game_root: &Path, data_dir: &Path, config: &Config) -> Result<Patched> {
    let author = author_file(data_dir, config)
        .ok_or_else(|| Error::msg("This config does not ship a whole gameinfo.gi, so it cannot be used as written."))?;
    let path = gameinfo_path(game_root);
    let current = std::fs::read_to_string(&path)?;
    let patched = apply_text(&current, &author, config)?;
    let stock = stock_path(game_root);
    if !is_applied(&current) {
        write_atomic(&stock, super::remove_text(&current).as_bytes())?;
    } else if !stock.exists() {
        return Err(Error::msg(
            "The stock copy of gameinfo.gi is missing. Verify the game files in Steam, then try again.",
        ));
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
    fn keeps_the_authors_text_and_the_installs_search_paths() {
        let patched = apply_text(CURRENT, AUTHOR, &config()).unwrap();
        let text = patched.text.replace("\r\n", "\n");
        assert!(patched.text.contains("\r\n"), "the install's line endings are kept");
        assert!(text.contains("// my favourite section"));
        assert!(text.contains("r_shadows \"0\"   // no shadows"));
        assert!(text.contains("Game citadel/addons\n\t\t\tGame citadel/sigil-addons2"));
        assert!(is_applied(&text));
        assert!(matches!(crate::performance::applied(&text), crate::performance::Applied::Config { .. }));
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
