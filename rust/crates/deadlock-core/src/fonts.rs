//! Custom fonts a mod ships, and the one game file they need.
//!
//! Deadlock draws its UI with Panorama, which only sees a font once its family name is listed in
//! `game/citadel/panorama/fonts/fonts.conf`. Mods that carry a `.ttf` under `panorama/fonts/`
//! therefore work only after Sigil copies the files next to that config and adds a
//! `<fontpattern>` line for each family. This does that, inside a marker comment per mod, so a
//! second install replaces its own block and [`remove_fonts`] can take it out again.
//!
//! [`install_fonts`] and [`remove_fonts`] are game folder writes: they take `approve`, copy the
//! config before editing it into Sigil's backups, write the config through a temp file, and
//! never overwrite a font file of another mod (the report says what was skipped). The family
//! name is read from the font's own `name` table, so the entry matches what the game looks up.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, temp_path};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with, read_vpk_entry_data};

pub const CONF_FILE_NAME: &str = "fonts.conf";
/// Fonts one install may add. A UI font set is a handful of files.
pub const MAX_FONTS: usize = 32;
/// Bytes kept from one font file.
pub const MAX_FONT_BYTES: u64 = 8 * 1024 * 1024;

const SECTION_START: &str = "<!-- [SIGIL-FONTS-START:";
const SECTION_END: &str = "<!-- [SIGIL-FONTS-END:";

/// One font found in a mod.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Font {
    /// File name inside the VPK.
    pub file_name: String,
    /// The family name from the font's `name` table, or the file stem.
    pub family: String,
    pub bytes: u64,
}

/// What one install or removal did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontReport {
    pub installed: Vec<Font>,
    /// Fonts already there under another mod's name.
    pub skipped: Vec<String>,
    pub removed: Vec<String>,
    /// The config was changed.
    pub conf_patched: bool,
    pub backup: Option<PathBuf>,
}

/// The game's font folder: `game/citadel/panorama/fonts`.
pub fn fonts_dir(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("panorama").join("fonts")
}

fn is_font_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".ttf") || lower.ends_with(".otf")) && lower.contains("panorama/fonts/")
}

/// A font file name that stays inside the fonts folder on every platform: one path component,
/// no separators (`/`, `\`), no drive or stream marker (`:`), no `..`, and no `,` (the marker
/// list that records a mod's fonts is comma-separated).
pub fn is_plain_font_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains(['/', '\\', ':', ',', '\0'])
        && !name.contains("..")
        && !name.chars().any(char::is_control)
        && Path::new(name).components().count() == 1
}

/// The font files a directory VPK ships, in path order.
pub fn fonts_in_vpk(path: &Path) -> Result<Vec<Font>> {
    let directory = read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: false })?;
    let mut fonts = Vec::new();
    for entry in &directory.entries {
        if !is_font_name(&entry.path) || u64::from(entry.total_length) > MAX_FONT_BYTES {
            continue;
        }
        if !crate::vpk::is_safe_vpk_path(&entry.path) {
            continue;
        }
        let file_name = entry.path.rsplit('/').next().unwrap_or(&entry.path).to_string();
        if !is_plain_font_file_name(&file_name) {
            continue;
        }
        let family = read_vpk_entry_data(&directory, entry)
            .ok()
            .and_then(|data| family_name(&data))
            .unwrap_or_else(|| file_stem(&file_name));
        fonts.push(Font {
            file_name,
            family,
            bytes: u64::from(entry.total_length),
        });
        if fonts.len() >= MAX_FONTS {
            break;
        }
    }
    fonts.sort_by_cached_key(|font| font.file_name.to_lowercase());
    Ok(fonts)
}

fn file_stem(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// The family name from a TrueType or OpenType `name` table: name id 1, then 4, preferring the
/// Windows Unicode record. `None` when the file is not a font or has no name table.
pub fn family_name(data: &[u8]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }
    let tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    let mut name_offset = None;
    for index in 0..tables {
        let at = 12 + index * 16;
        let record = data.get(at..at + 16)?;
        if &record[0..4] == b"name" {
            name_offset = Some(u32::from_be_bytes([
                record[8], record[9], record[10], record[11],
            ]) as usize);
            break;
        }
    }
    let start = name_offset?;
    let count = u16::from_be_bytes([*data.get(start + 2)?, *data.get(start + 3)?]) as usize;
    let storage = start + u16::from_be_bytes([*data.get(start + 4)?, *data.get(start + 5)?]) as usize;
    let mut best: Option<(u8, String)> = None;
    for index in 0..count {
        let at = start + 6 + index * 12;
        let record = data.get(at..at + 12)?;
        let platform = u16::from_be_bytes([record[0], record[1]]);
        let name_id = u16::from_be_bytes([record[6], record[7]]);
        let length = u16::from_be_bytes([record[8], record[9]]) as usize;
        let offset = u16::from_be_bytes([record[10], record[11]]) as usize;
        if !matches!(name_id, 1 | 4) {
            continue;
        }
        let Some(raw) = data.get(storage + offset..storage + offset + length) else {
            continue;
        };
        let text = if platform == 3 || platform == 0 {
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(raw).into_owned()
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        // Family name (1) beats full name (4); the first good record wins within that.
        let score = if name_id == 1 { 2 } else { 1 };
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, text));
        }
    }
    best.map(|(_, text)| text)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn marker_start(slot: u8) -> String {
    format!("{SECTION_START}{slot:02}]")
}

fn marker_end(slot: u8) -> String {
    format!("{SECTION_END}{slot:02}]")
}

/// The config with this slot's block taken out.
pub fn strip_section(content: &str, slot: u8) -> String {
    let start_marker = marker_start(slot);
    let end_marker = marker_end(slot);
    let Some(start) = content.find(&start_marker) else {
        return content.to_string();
    };
    let Some(end_relative) = content[start..].find(&end_marker) else {
        return content.to_string();
    };
    let end = start + end_relative + end_marker.len();
    let mut out = String::with_capacity(content.len());
    out.push_str(content[..start].trim_end_matches([' ', '\t']).trim_end_matches('\n'));
    out.push_str(&content[end..]);
    out
}

/// Add a block for this slot after the last existing `<fontpattern>` line.
pub fn patch_conf(content: &str, slot: u8, fonts: &[Font]) -> String {
    let content = strip_section(content, slot);
    if fonts.is_empty() {
        return content;
    }
    let patterns: String = fonts
        .iter()
        .map(|font| format!("        <fontpattern>{}</fontpattern>", escape_xml(&font.family)))
        .collect::<Vec<_>>()
        .join("\n");
    let names: String = fonts
        .iter()
        .map(|font| font.file_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let block = format!(
        "\n        {} {} -->\n{}\n        {} -->",
        marker_start(slot),
        escape_xml(&names),
        patterns,
        marker_end(slot)
    );
    let at = content
        .rfind("</fontpattern>")
        .map(|pos| pos + "</fontpattern>".len())
        .or_else(|| content.rfind("</fontconfig>"))
        .unwrap_or(content.len());
    let mut out = content;
    out.insert_str(at, &block);
    out
}

/// Copy the fonts a mod ships and register their families. Game install write.
///
/// `vpk` is the slot's directory VPK. Font files land in `game/citadel/panorama/fonts/` under
/// their own names: one that is already there with another file's bytes is skipped, so two mods
/// cannot fight over a name. `fonts.conf` is copied into `backup_dir` first, then written
/// through a temp file.
pub fn install_fonts(
    game_root: &Path,
    vpk: &Path,
    slot: u8,
    backup_dir: &Path,
    approve: bool,
) -> Result<FontReport> {
    if !approve {
        return Err(Error::msg(
            "Not approved: add this mod's fonts. Nothing was written.",
        ));
    }
    if !(1..=99).contains(&slot) {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to 99. Nothing was written."
        )));
    }
    if !game_root.join("game").join("citadel").is_dir() {
        return Err(Error::msg(format!(
            "{} no longer holds game/citadel. Nothing was written.",
            game_root.display()
        )));
    }
    let directory = read_vpk_directory_with(vpk, &ReadVpkOptions { keep_preload: false })?;
    let fonts = fonts_in_vpk(vpk)?;
    if fonts.is_empty() {
        return Err(Error::msg(
            "This mod ships no fonts under panorama/fonts. Nothing was written.",
        ));
    }
    let target_dir = fonts_dir(game_root);
    fs::create_dir_all(&target_dir)?;

    let mut report = FontReport::default();
    let mut installed = Vec::new();
    for font in &fonts {
        // Names come from the mod: write only a plain file name inside the fonts folder.
        if !is_plain_font_file_name(&font.file_name) {
            report.skipped.push(font.file_name.clone());
            continue;
        }
        let Some(entry) = directory
            .entries
            .iter()
            .find(|entry| entry.path.ends_with(&font.file_name))
        else {
            continue;
        };
        let data = read_vpk_entry_data(&directory, entry)?;
        let target = target_dir.join(&font.file_name);
        if target.exists() {
            let same = fs::read(&target).map(|old| old == data).unwrap_or(false);
            if !same {
                report.skipped.push(font.file_name.clone());
            } else {
                installed.push(font.clone());
            }
            continue;
        }
        let temporary = temp_path(&target);
        fs::write(&temporary, &data)?;
        if let Err(error) = fs::rename(&temporary, &target) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        installed.push(font.clone());
    }

    if installed.is_empty() {
        report.installed = installed;
        return Ok(report);
    }
    let conf = target_dir.join(CONF_FILE_NAME);
    let content = fs::read_to_string(&conf).unwrap_or_else(|_| default_conf());
    let patched = patch_conf(&content, slot, &installed);
    if patched != content || !conf.exists() {
        fs::create_dir_all(backup_dir)?;
        if conf.exists() {
            let backup = backup_dir.join(format!(
                "{CONF_FILE_NAME}.{}.bak",
                now_iso().replace([':', '.'], "-")
            ));
            fs::copy(&conf, &backup)?;
            report.backup = Some(backup);
        }
        let temporary = temp_path(&conf);
        fs::write(&temporary, &patched)?;
        if let Err(error) = fs::rename(&temporary, &conf) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        report.conf_patched = true;
    }
    report.installed = installed;
    Ok(report)
}

/// Take a mod's fonts out again: its config block goes, and the files that block named are
/// moved to Sigil's backups. Game install write; refuses without `approve`.
pub fn remove_fonts(
    game_root: &Path,
    slot: u8,
    backup_dir: &Path,
    approve: bool,
) -> Result<FontReport> {
    if !approve {
        return Err(Error::msg(
            "Not approved: remove this mod's fonts. Nothing was written.",
        ));
    }
    let target_dir = fonts_dir(game_root);
    let conf = target_dir.join(CONF_FILE_NAME);
    let mut report = FontReport::default();
    let Ok(content) = fs::read_to_string(&conf) else {
        return Err(Error::msg(format!(
            "{} has no fonts.conf to change. Nothing was written.",
            target_dir.display()
        )));
    };
    let start = marker_start(slot);
    let names: Vec<String> = content
        .find(&start)
        .and_then(|at| {
            let head = content[at..].split("-->").next()?;
            head.strip_prefix(&start).map(|rest| {
                rest.split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default();
    let stripped = strip_section(&content, slot);
    if stripped == content {
        return Err(Error::msg(format!(
            "Slot {slot:02} has no fonts in fonts.conf. Nothing was written."
        )));
    }
    fs::create_dir_all(backup_dir)?;
    let backup = backup_dir.join(format!(
        "{CONF_FILE_NAME}.{}.bak",
        now_iso().replace([':', '.'], "-")
    ));
    fs::copy(&conf, &backup)?;
    let temporary = temp_path(&conf);
    fs::write(&temporary, &stripped)?;
    if let Err(error) = fs::rename(&temporary, &conf) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    report.conf_patched = true;
    report.backup = Some(backup);
    for name in names {
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            continue;
        }
        let file = target_dir.join(&name);
        if file.is_file() {
            let stash = backup_dir.join(format!("fonts-{slot:02}-{name}"));
            if fs::rename(&file, &stash).is_ok() {
                report.removed.push(name);
            }
        }
    }
    Ok(report)
}

/// The config Sigil writes when the game has none: the wrapping element world of a fontconfig
/// file, which Panorama reads.
fn default_conf() -> String {
    "<?xml version=\"1.0\"?>\n<fontconfig>\n</fontconfig>\n".to_string()
}

/// Whether the game's `fonts.conf` already has a block for this slot.
pub fn has_block(game_root: &Path, slot: u8) -> bool {
    fs::read_to_string(fonts_dir(game_root).join(CONF_FILE_NAME))
        .map(|content| content.contains(&marker_start(slot)))
        .unwrap_or(false)
}

/// The fonts folder exists and the game has a config to edit.
pub fn conf_exists(game_root: &Path) -> bool {
    fonts_dir(game_root).join(CONF_FILE_NAME).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn root(tag: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/fonts-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn game(root: &Path) -> PathBuf {
        let game = root.join("Deadlock");
        fs::create_dir_all(game.join("game/citadel/panorama/fonts")).unwrap();
        game
    }

    /// A minimal TrueType file with one `name` record (family name, Windows Unicode).
    fn ttf(family: &str) -> Vec<u8> {
        let units: Vec<u8> = family
            .encode_utf16()
            .flat_map(|unit| unit.to_be_bytes())
            .collect();
        let mut name = Vec::new();
        name.extend_from_slice(&0u16.to_be_bytes()); // format
        name.extend_from_slice(&1u16.to_be_bytes()); // count
        name.extend_from_slice(&18u16.to_be_bytes()); // string offset
        name.extend_from_slice(&3u16.to_be_bytes()); // platform: Windows
        name.extend_from_slice(&1u16.to_be_bytes()); // encoding: Unicode BMP
        name.extend_from_slice(&0x0409u16.to_be_bytes()); // language
        name.extend_from_slice(&1u16.to_be_bytes()); // name id: family
        name.extend_from_slice(&(units.len() as u16).to_be_bytes());
        name.extend_from_slice(&0u16.to_be_bytes());
        name.extend_from_slice(&units);

        let mut file = Vec::new();
        file.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        file.extend_from_slice(&1u16.to_be_bytes());
        file.extend_from_slice(&16u16.to_be_bytes());
        file.extend_from_slice(&0u16.to_be_bytes());
        file.extend_from_slice(&0u16.to_be_bytes());
        file.extend_from_slice(b"name");
        file.extend_from_slice(&0u32.to_be_bytes());
        file.extend_from_slice(&28u32.to_be_bytes());
        file.extend_from_slice(&(name.len() as u32).to_be_bytes());
        file.extend_from_slice(&name);
        file
    }

    fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let staging = root.join(format!("staging-{name}"));
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        pack_vpk(&PackOptions::new(staging, root.join("out"), name))
            .unwrap()
            .dir_path
    }

    #[test]
    fn reads_the_family_from_a_font_name_table() {
        assert_eq!(family_name(&ttf("Brotherhood")), Some("Brotherhood".to_string()));
        assert_eq!(family_name(b"not a font"), None);
        assert_eq!(family_name(&[]), None);
    }

    #[test]
    fn finds_fonts_installs_them_and_registers_the_family() {
        let root = root("install");
        let game = game(&root);
        let font = ttf("Brotherhood");
        let mod_vpk = vpk(&root, "pak01", &[
            ("panorama/fonts/Brotherhood.ttf", &font),
            ("materials/a.vmat_c", b"asset"),
        ]);
        let fonts = fonts_in_vpk(&mod_vpk).unwrap();
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].family, "Brotherhood");

        let backups = root.join("backups");
        assert!(install_fonts(&game, &mod_vpk, 3, &backups, false).is_err());
        let report = install_fonts(&game, &mod_vpk, 3, &backups, true).unwrap();
        assert_eq!(report.installed.len(), 1);
        assert!(report.conf_patched);
        let conf = fs::read_to_string(fonts_dir(&game).join(CONF_FILE_NAME)).unwrap();
        assert!(conf.contains("<fontpattern>Brotherhood</fontpattern>"));
        assert!(conf.contains("[SIGIL-FONTS-START:03]"));
        assert!(fonts_dir(&game).join("Brotherhood.ttf").is_file());

        // A second install of the same mod is idempotent: one block, not two.
        let again = install_fonts(&game, &mod_vpk, 3, &backups, true).unwrap();
        assert!(again.skipped.is_empty());
        let conf = fs::read_to_string(fonts_dir(&game).join(CONF_FILE_NAME)).unwrap();
        assert_eq!(conf.matches("<fontpattern>Brotherhood</fontpattern>").count(), 1);

        // Removal takes the block and the file out, keeping a backup of the config.
        let removed = remove_fonts(&game, 3, &backups, true).unwrap();
        assert_eq!(removed.removed, vec!["Brotherhood.ttf".to_string()]);
        assert!(!fonts_dir(&game).join("Brotherhood.ttf").exists());
        let conf = fs::read_to_string(fonts_dir(&game).join(CONF_FILE_NAME)).unwrap();
        assert!(!conf.contains("Brotherhood"));
        assert!(backups.read_dir().unwrap().count() >= 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mod_without_fonts_says_so_and_never_overwrites_another_mods_file() {
        let root = root("refuse");
        let game = game(&root);
        let plain = vpk(&root, "pak02", &[("materials/a.vmat_c", b"a")]);
        let backups = root.join("backups");
        assert!(
            install_fonts(&game, &plain, 1, &backups, true)
                .unwrap_err()
                .to_string()
                .contains("no fonts")
        );

        let other = ttf("Other");
        fs::write(fonts_dir(&game).join("Shared.ttf"), &other).unwrap();
        let mine = vpk(&root, "pak03", &[("panorama/fonts/Shared.ttf", &ttf("Mine"))]);
        let report = install_fonts(&game, &mine, 2, &backups, true).unwrap();
        assert_eq!(report.skipped, vec!["Shared.ttf".to_string()]);
        assert_eq!(fs::read(fonts_dir(&game).join("Shared.ttf")).unwrap(), other);
        assert!(!fonts_dir(&game).join(CONF_FILE_NAME).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn config_patching_keeps_other_lines_and_strips_only_its_own_block() {
        let base = "<fontconfig>\n        <fontpattern>Existing</fontpattern>\n</fontconfig>\n";
        let fonts = vec![Font {
            file_name: "a.ttf".into(),
            family: "Mine".into(),
            bytes: 1,
        }];
        let once = patch_conf(base, 4, &fonts);
        assert!(once.contains("<fontpattern>Existing</fontpattern>"));
        assert!(once.contains("SIGIL-FONTS-START:04"));
        let twice = patch_conf(&once, 4, &fonts);
        assert_eq!(twice.matches("SIGIL-FONTS-START").count(), 1);
        let gone = strip_section(&twice, 4);
        assert!(!gone.contains("SIGIL"));
        assert!(gone.contains("Existing"));
    }
}

#[cfg(test)]
mod font_name_tests {
    use super::is_plain_font_file_name;

    #[test]
    fn font_names_that_could_leave_the_fonts_folder_are_refused() {
        assert!(is_plain_font_file_name("Radiance-Bold.ttf"));
        for bad in ["..\\evil.ttf", "../x.ttf", "C:x.ttf", "a,b.ttf", "sub/x.ttf", "", "..", "x\u{0}.ttf"] {
            assert!(!is_plain_font_file_name(bad), "{bad:?} must be refused");
        }
    }
}

