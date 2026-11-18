//! One folder per profile, and the `gameinfo.gi` block that mounts it.
//!
//! An isolated profile keeps its own copy of its mods in `game/citadel/addons/sigil-<slug>`.
//! Switching to it rewrites a small marked block in the `SearchPaths` section of `gameinfo.gi`:
//!
//! ```text
//! // sigil:profile-begin
//! Game citadel/addons/sigil-ranked
//! // sigil:profile-end
//! ```
//!
//! While the block is there the plain `Game citadel/addons` line is taken over by it, so mods
//! that belong to other profiles never load. Removing the block puts the plain line back. Every
//! rewrite backs `gameinfo.gi` up beside itself and writes atomically. Refuses without `approve`.
//!
//! A profile folder holds copies, never the only copy: the mods stay in the main addons folder
//! and are copied in when the profile is applied. Nothing here deletes a mod file.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{enabled_file_name, installed_identities_in, resolve_entries, EntryState, Profile};
use crate::deploy::{addons_dir, gameinfo_path, GAMEINFO_CITADEL_LINE};
use crate::error::{Error, Result};
use crate::load_order::{check_plain_name, read_slot_files, slot_of};
use crate::util::{now_iso, write_atomic};

/// Folder names this build writes start with this. A folder without it is never touched.
pub const FOLDER_PREFIX: &str = "sigil-";
pub const BLOCK_BEGIN: &str = "// sigil:profile-begin";
pub const BLOCK_END: &str = "// sigil:profile-end";

/// The folder an isolated profile lives in: `sigil-<slug>`.
pub fn folder_name(slug: &str) -> Result<String> {
    super::validate_slug(slug)?;
    Ok(format!("{FOLDER_PREFIX}{slug}"))
}

/// The profile folder inside `citadel/addons`.
pub fn folder_dir(game_root: &Path, slug: &str) -> Result<PathBuf> {
    Ok(addons_dir(game_root).join(folder_name(slug)?))
}

/// What one folder sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSync {
    pub folder: String,
    pub dir: PathBuf,
    /// Slot files copied in or refreshed.
    pub copied: Vec<String>,
    /// Profile mods that are not installed, by label.
    pub missing: Vec<String>,
    /// Copies in the folder switched off because the profile no longer has them.
    pub disabled: Vec<String>,
}

impl FolderSync {
    pub fn is_empty(&self) -> bool {
        self.copied.is_empty() && self.disabled.is_empty()
    }
}

/// Copy the profile's mods from the main addons folder into its own folder. Reads the game,
/// writes only inside `citadel/addons/<folder>`.
///
/// A copy that already matches by fingerprint is left alone. A stale copy is replaced only
/// when a fresh one comes from the main folder, which stays the source of truth.
pub fn sync_folder(game_root: &Path, workspace: &Path, profile: &Profile) -> Result<FolderSync> {
    let folder = folder_name(&profile.slug)?;
    let dir = addons_dir(game_root).join(&folder);
    fs::create_dir_all(&dir)?;
    let main = addons_dir(game_root);
    let sources = installed_identities_in(&main, workspace)?;
    let states = resolve_entries(&profile.slots, &sources);
    let groups = read_slot_files(&main)?;
    let mut sync = FolderSync {
        folder,
        dir: dir.clone(),
        ..FolderSync::default()
    };
    let mut wanted: BTreeSet<u16> = BTreeSet::new();

    for (entry, state) in profile.slots.iter().zip(states) {
        let EntryState::Ready { slot, .. } = state else {
            if entry.enabled {
                sync.missing.push(entry.label());
            }
            continue;
        };
        // An isolated profile folder holds pak01 to pak99 only; a mod in an extra addon folder
        // (slot past 99) has no place there, so it is reported rather than copied.
        if crate::addons::folder_index(slot) > 0 {
            if entry.enabled {
                sync.missing.push(format!("{} (past slot 99; isolated profiles hold 99 mods)", entry.label()));
            }
            continue;
        }
        wanted.insert(slot);
        let Some(files) = groups.get(&slot) else {
            continue;
        };
        let source = sources.iter().find(|addon| addon.slot == slot);
        let enabled_name = enabled_file_name(&entry.file);
        let target_name = if entry.enabled {
            enabled_name.clone()
        } else {
            format!("{enabled_name}{}", super::DISABLED_SUFFIX)
        };
        let target = dir.join(&target_name);
        let fresh = source
            .and_then(|source| {
                crate::profiles::fingerprint_file(&target)
                    .ok()
                    .map(|fingerprint| fingerprint == source.fingerprint)
            })
            .unwrap_or(false)
            && target.is_file();
        if fresh {
            continue;
        }
        copy_slot(&main, &dir, files, &enabled_name, &target_name)?;
        sync.copied.push(target_name.clone());
        for stale in [enabled_name.clone(), format!("{enabled_name}{}", super::DISABLED_SUFFIX)] {
            if stale != target_name {
                let _ = fs::remove_file(dir.join(stale));
            }
        }
    }

    let folder_files = read_slot_files(&dir)?;
    for (slot, files) in &folder_files {
        if wanted.contains(slot) {
            continue;
        }
        for name in files {
            if name.to_ascii_lowercase().ends_with(super::DISABLED_SUFFIX) {
                continue;
            }
            if slot_of(name).is_none() || !name.to_ascii_lowercase().ends_with("_dir.vpk") {
                continue;
            }
            check_plain_name(name)?;
            let disabled = format!("{name}{}", super::DISABLED_SUFFIX);
            if !dir.join(&disabled).exists() {
                fs::rename(dir.join(name), dir.join(&disabled))?;
                sync.disabled.push(disabled);
            }
        }
    }
    Ok(sync)
}

/// Copy every file of one slot into the profile folder, the directory file under `target_name`.
fn copy_slot(
    source_dir: &Path,
    target_dir: &Path,
    files: &[String],
    enabled_name: &str,
    target_name: &str,
) -> Result<()> {
    for name in files {
        check_plain_name(name)?;
        let from = source_dir.join(name);
        let to = if enabled_file_name(name).eq_ignore_ascii_case(enabled_name) {
            target_dir.join(target_name)
        } else {
            target_dir.join(name)
        };
        let token = crate::load_order::new_token();
        let partial = to.with_file_name(format!(
            ".sigil-copy-{token}-{}",
            to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ));
        fs::copy(&from, &partial).map_err(|error| {
            Error::msg(format!("Could not copy {}: {error}", from.display()))
        })?;
        if to.exists() {
            fs::remove_file(&to)?;
        }
        if let Err(error) = fs::rename(&partial, &to) {
            let _ = fs::remove_file(&partial);
            return Err(error.into());
        }
    }
    Ok(())
}

/// The folder the mount block names, when there is one.
pub fn mounted_folder(game_root: &Path) -> Option<String> {
    let text = fs::read_to_string(gameinfo_path(game_root)).ok()?;
    block_folder(&text)
}

pub(crate) fn block_folder(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_BEGIN {
            inside = true;
            continue;
        }
        if trimmed == BLOCK_END {
            return None;
        }
        if inside && let Some(folder) = mount_value(line) {
            if folder.is_empty() {
                return None;
            }
            return valid_folder(&folder).then_some(folder);
        }
    }
    None
}

/// A folder name this build is willing to mount: `sigil-<slug>`.
fn valid_folder(folder: &str) -> bool {
    let Some(slug) = folder.strip_prefix(FOLDER_PREFIX) else {
        return false;
    };
    !slug.is_empty() && super::validate_slug(slug).is_ok()
}

/// The `citadel/addons` value of a `Game` line: `Some("")` for the plain folder,
/// `Some(folder)` for a subfolder, `None` for any other line.
fn mount_value(line: &str) -> Option<String> {
    let code = line.split("//").next().unwrap_or(line).trim();
    let mut parts = code.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("Game") {
        return None;
    }
    let value = parts.next()?.trim_matches('"');
    let lower = value.trim_end_matches('/').to_ascii_lowercase();
    let rest = lower.strip_prefix("citadel/addons")?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    Some(rest.trim_end_matches('/').to_string())
}

/// What a mount rewrite did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountWrite {
    /// The folder mounted after the write. None means the plain addons folder.
    pub folder: Option<String>,
    /// The folder that was mounted before.
    pub previous: Option<String>,
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

/// GAME WRITE. Point `SearchPaths` at `folder` (None puts the plain addons line back).
/// Refuses without `approve`, backs `gameinfo.gi` up before replacing it.
pub fn mount_gameinfo(game_root: &Path, folder: Option<&str>, approve: bool) -> Result<MountWrite> {
    if !approve {
        return Err(Error::msg(
            "Changing the profile mount in gameinfo.gi was not approved. Nothing was written.",
        ));
    }
    if let Some(folder) = folder
        && !valid_folder(folder)
    {
        return Err(Error::msg(format!("{folder:?} is not a profile folder.")));
    }
    let path = gameinfo_path(game_root);
    let text = fs::read_to_string(&path).map_err(|error| {
        Error::msg(format!("Could not read {}: {error}", path.display()))
    })?;
    let previous = block_folder(&text);
    let next = match plan_mount(&text, folder)? {
        Some(next) => next,
        None => {
            return Ok(MountWrite {
                folder: folder.map(str::to_string),
                previous,
                changed: false,
                backup: None,
            });
        }
    };
    let backup = backup_beside(&path)?;
    fs::copy(&path, &backup)?;
    write_atomic(&path, next.as_bytes())?;
    Ok(MountWrite {
        folder: folder.map(str::to_string),
        previous,
        changed: true,
        backup: Some(backup),
    })
}

/// The new `gameinfo.gi` text, or None when it already mounts `folder`.
///
/// The block takes the place of the plain `Game citadel/addons` line, so a game folder with no
/// block and the plain line needs no edit.
pub fn plan_mount(text: &str, folder: Option<&str>) -> Result<Option<String>> {
    if let Some(folder) = folder
        && !valid_folder(folder)
    {
        return Err(Error::msg(format!("{folder:?} is not a profile folder.")));
    }
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    fn content(line: &str) -> &str {
        line.trim_end_matches(['\r', '\n'])
    }

    let mut block_start = None;
    let mut block_end = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = content(line).trim();
        if trimmed == BLOCK_BEGIN {
            block_start = Some(index);
        } else if trimmed == BLOCK_END {
            block_end = Some(index);
            break;
        }
    }
    if block_start.is_some() != block_end.is_some()
        || block_start.is_some_and(|start| block_end.is_some_and(|end| end < start))
    {
        return Err(Error::msg(
            "gameinfo.gi has half a Sigil profile block. Fix or remove its markers.",
        ));
    }

    // An outside plain addons line, or one of our own stale mounts: both move into the block.
    let mut plain_at = None;
    let mut stale: Vec<usize> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if block_start.is_some_and(|start| index >= start) && block_end.is_some_and(|end| index <= end) {
            continue;
        }
        if let Some(value) = mount_value(content(line)) {
            if value.is_empty() {
                plain_at = Some(index);
            } else if valid_folder(&value) {
                stale.push(index);
            }
        }
    }

    let at = block_start
        .or(plain_at)
        .or_else(|| {
            lines.iter().position(|line| {
                let code = content(line);
                let trimmed = code.split("//").next().unwrap_or(code).trim();
                trimmed.eq_ignore_ascii_case(GAMEINFO_CITADEL_LINE)
            })
        })
        .ok_or_else(|| {
            Error::msg(format!(
                "gameinfo.gi has no `{GAMEINFO_CITADEL_LINE}` SearchPath line to put the profile above."
            ))
        })?;

    let mut removed: Vec<usize> = Vec::new();
    if let (Some(start), Some(end)) = (block_start, block_end) {
        removed.extend(start..=end);
    }
    if let Some(index) = plain_at {
        removed.push(index);
    }
    removed.extend(stale);
    removed.sort_unstable();
    removed.dedup();

    // With no block and no folder there is nothing of ours to rewrite: the plain line stays.
    if folder.is_none() && block_start.is_none() {
        return Ok(None);
    }

    let indent: String = {
        let source = lines
            .get(plain_at.or(block_start).unwrap_or(at))
            .map(|line| content(line))
            .unwrap_or("");
        let trimmed = source.trim_start();
        source[..source.len() - trimmed.len()].to_string()
    };
    let mut block = String::new();
    if let Some(folder) = folder {
        block.push_str(&indent);
        block.push_str(BLOCK_BEGIN);
        block.push_str(ending);
        block.push_str(&indent);
        block.push_str("Game citadel/addons/");
        block.push_str(folder);
        block.push_str(ending);
        block.push_str(&indent);
        block.push_str(BLOCK_END);
        block.push_str(ending);
    } else {
        // The folder mount is gone: leave the stock line, with no block around it.
        block.push_str(&indent);
        block.push_str("Game citadel/addons");
        block.push_str(ending);
    }

    let mut output = String::with_capacity(text.len() + block.len());
    let mut inserted = false;
    for (index, line) in lines.iter().enumerate() {
        if index == at {
            output.push_str(&block);
            inserted = true;
        }
        if removed.contains(&index) {
            continue;
        }
        output.push_str(line);
    }
    if !inserted {
        output.push_str(&block);
    }
    if output == text {
        return Ok(None);
    }
    Ok(Some(output))
}

/// A backup name beside the file, using the marker `backups.rs` discovers.
fn backup_beside(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg(format!("Cannot back up {}: no file name.", path.display())))?;
    let stamp = now_iso().replace([':', '.'], "-");
    Ok(path.with_file_name(format!("{name}.dah-backup-{stamp}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAMEINFO: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame_Language \"citadel_*LANGUAGE*\"\n\n\t\t\tGame citadel/addons\n\t\t\tGame \"citadel\"\n\t\t\tGame \"core\"\n\t\t}\n\t}\n}\n";

    fn mounts(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                let value = mount_value(line)?;
                value.is_empty().then(|| "citadel/addons".to_string()).or(Some(value))
            })
            .collect()
    }

    #[test]
    fn mounting_replaces_the_plain_line_and_unmount_puts_it_back() {
        let mounted = plan_mount(GAMEINFO, Some("sigil-ranked")).unwrap().unwrap();
        assert!(mounted.contains("Game citadel/addons/sigil-ranked"));
        assert!(!mounted.contains("\t\t\tGame citadel/addons\n"));
        assert_eq!(mounts(&mounted), vec!["sigil-ranked".to_string()]);
        assert_eq!(block_folder(&mounted).as_deref(), Some("sigil-ranked"));

        let restored = plan_mount(&mounted, None).unwrap().unwrap();
        assert!(!restored.contains(BLOCK_BEGIN));
        assert_eq!(mounts(&restored), vec!["citadel/addons".to_string()]);
        assert_eq!(restored, GAMEINFO);
    }

    #[test]
    fn mounting_the_same_folder_twice_is_not_a_change() {
        let mounted = plan_mount(GAMEINFO, Some("sigil-ranked")).unwrap().unwrap();
        assert_eq!(plan_mount(&mounted, Some("sigil-ranked")).unwrap(), None);
        assert_eq!(plan_mount(GAMEINFO, None).unwrap(), None);
    }

    #[test]
    fn switching_folders_keeps_one_block_and_all_other_lines() {
        let first = plan_mount(GAMEINFO, Some("sigil-ranked")).unwrap().unwrap();
        let second = plan_mount(&first, Some("sigil-casual")).unwrap().unwrap();
        assert_eq!(second.matches(BLOCK_BEGIN).count(), 1);
        assert!(!second.contains("sigil-ranked"));
        assert!(second.contains("Game \"citadel\""));
        assert!(second.contains("Game_Language"));
    }

    #[test]
    fn a_stale_mount_outside_the_block_is_folded_in() {
        let stray = GAMEINFO.replace(
            "Game citadel/addons",
            "Game citadel/addons/sigil-old\n\t\t\tGame citadel/addons",
        );
        let mounted = plan_mount(&stray, Some("sigil-new")).unwrap().unwrap();
        assert!(!mounted.contains("sigil-old"));
        assert_eq!(mounted.matches(BLOCK_BEGIN).count(), 1);
        assert_eq!(mounts(&mounted), vec!["sigil-new".to_string()]);
    }

    #[test]
    fn crlf_and_indentation_survive() {
        let crlf = GAMEINFO.replace('\n', "\r\n");
        let mounted = plan_mount(&crlf, Some("sigil-ranked")).unwrap().unwrap();
        assert!(mounted.contains("\r\n\t\t\t// sigil:profile-begin\r\n"));
        assert!(!mounted.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn half_a_block_is_refused_and_bad_names_never_reach_the_path() {
        let broken = format!("{GAMEINFO}\n{BLOCK_BEGIN}\n");
        assert!(plan_mount(&broken, Some("sigil-ranked")).is_err());
        assert!(plan_mount(GAMEINFO, Some("../escape")).is_err());
        assert!(plan_mount(GAMEINFO, Some("sigil-Not A Slug")).is_err());
        assert!(!valid_folder("other-manager"));
        assert_eq!(block_folder("// sigil:profile-begin\nGame ../escape\n// sigil:profile-end"), None);
    }
}
