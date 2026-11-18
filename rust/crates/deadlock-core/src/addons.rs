//! Installed addon inventory, conflicts between enabled slots and the one
//! approved rename path that turns a slot on or off.
//!
//! [`list_installed`] and [`read_conflict_report`] are read only. They answer
//! what the manager needs to show: which slots are on disk, whether each one is
//! enabled, how many entries it ships, and which game paths more than one
//! enabled slot replaces.
//!
//! [`set_enabled`] is the only place in this crate that renames a slot file. It
//! requires an explicit approval flag, refuses a slot that is missing, already
//! in the requested state or present in both forms, and moves no other file.
//! `deploy` remains the only path that adds or replaces addon content.
//!
//! Deadlock loads `pak01_dir.vpk` through `pak99_dir.vpk` from
//! `game/citadel/addons` and lets the lowest slot win a shared path. A disabled
//! addon is the directory file renamed with a `.disabled` suffix, so the game no
//! longer sees it. Chunk files keep their names either way.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Suffix that marks a slot directory file as off.
pub const DISABLED_SUFFIX: &str = ".disabled";

/// Slots one addon folder holds: `pak01_dir.vpk` through `pak99_dir.vpk`.
pub const SLOTS_PER_FOLDER: u16 = 99;

/// The highest slot: `citadel/addons` plus the extra folders `sigil-addons2` to
/// `sigil-addons10` (see [`crate::overflow`]), 99 each.
pub const MAX_SLOT: u16 = SLOTS_PER_FOLDER * crate::overflow::MAX_FOLDERS as u16;

/// A slot is one number from 1 to [`MAX_SLOT`] across every addon folder, in load order: 1 to
/// 99 are `citadel/addons/pak01` to `pak99`, 100 to 198 are `citadel/sigil-addons2/pak01` to
/// `pak99`, and so on. The game reads the folders in that order and the `pakNN` number inside
/// each, so a lower slot still wins a shared path.
pub fn is_slot(slot: u16) -> bool {
    (1..=MAX_SLOT).contains(&slot)
}

/// Zero-based folder of a slot: 0 is `citadel/addons`.
pub fn folder_index(slot: u16) -> usize {
    usize::from(slot.max(1) - 1) / usize::from(SLOTS_PER_FOLDER)
}

/// The `pakNN` number of a slot inside its own folder, 1 to 99.
pub fn local_slot(slot: u16) -> u16 {
    (slot.max(1) - 1) % SLOTS_PER_FOLDER + 1
}

/// The slot of `pak<local>` in folder `index`.
pub fn global_slot(index: usize, local: u16) -> u16 {
    index as u16 * SLOTS_PER_FOLDER + local
}

/// The folder a slot's files live in, from the main addons folder.
pub fn slot_folder(addons: &Path, slot: u16) -> std::path::PathBuf {
    folder_at(addons, folder_index(slot))
}

/// Folder `index` beside the main addons folder: the folder itself for 0, else
/// `citadel/sigil-addons<index + 1>`.
pub fn folder_at(addons: &Path, index: usize) -> std::path::PathBuf {
    if index == 0 {
        return addons.to_path_buf();
    }
    let name = format!("{}{}", crate::overflow::FOLDER_PREFIX, index + 1);
    addons.parent().map(|citadel| citadel.join(&name)).unwrap_or_else(|| addons.join(name))
}

/// `pakNN` plus `suffix` for a slot, numbered inside its folder: `slot_name(101, "_dir.vpk")`
/// is `pak02_dir.vpk` (it lives in `sigil-addons2`).
pub fn slot_name(slot: u16, suffix: &str) -> String {
    format!("pak{:02}{suffix}", local_slot(slot))
}

/// Full path of a slot file: [`slot_folder`] joined with [`slot_name`].
pub fn slot_path(addons: &Path, slot: u16, suffix: &str) -> std::path::PathBuf {
    slot_folder(addons, slot).join(slot_name(slot, suffix))
}

/// How a slot reads to a player: `05`, or `2·05` for pak05 in the second folder.
pub fn slot_label(slot: u16) -> String {
    match folder_index(slot) {
        0 => format!("{:02}", local_slot(slot)),
        index => format!("{}·{:02}", index + 1, local_slot(slot)),
    }
}

/// The addon folders that exist on disk, with their index: always the main one first (even
/// when missing, so callers report it), then each extra folder that is a directory.
pub fn existing_folders(addons: &Path) -> Vec<(usize, std::path::PathBuf)> {
    let mut folders = vec![(0, addons.to_path_buf())];
    for index in 1..crate::overflow::MAX_FOLDERS {
        let folder = folder_at(addons, index);
        if folder.is_dir() {
            folders.push((index, folder));
        }
    }
    folders
}

/// GAME WRITE, called by the app actions that add, move or remove slot files. Makes
/// `gameinfo.gi` mount exactly the extra addon folders that hold a slot file, in the marked block
/// of [`crate::overflow`], and removes extra folders that are left empty. The folders are filled
/// before this runs, so the game never mounts a folder of half-moved files.
///
/// While an isolated profile's block is in `gameinfo.gi` nothing extra is mounted: an isolated
/// profile holds at most 99 mods, and the extra folders belong to the main library. Backs
/// `gameinfo.gi` up beside itself before changing it and writes atomically. Returns whether the
/// file changed. A missing `gameinfo.gi` changes nothing (the gameinfo banner reports it).
pub fn sync_overflow_mount(addons: &Path) -> Result<bool> {
    let files = crate::load_order::read_slot_files(addons)?;
    let used: BTreeSet<usize> = files
        .keys()
        .map(|slot| folder_index(*slot))
        .filter(|index| *index > 0)
        .collect();
    for index in 1..crate::overflow::MAX_FOLDERS {
        let folder = folder_at(addons, index);
        if !used.contains(&index)
            && folder.is_dir()
            && fs::read_dir(&folder).map(|mut entries| entries.next().is_none()).unwrap_or(false)
        {
            let _ = fs::remove_dir(&folder);
        }
    }

    let Some(citadel) = addons.parent() else {
        return Ok(false);
    };
    let gameinfo = citadel.join("gameinfo.gi");
    if !gameinfo.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(&gameinfo)?;
    let isolated = text.lines().any(|line| line.trim() == "// sigil:profile-begin");
    let folders: Vec<String> = if isolated {
        Vec::new()
    } else {
        used.iter()
            .map(|index| crate::overflow::folder_name(index + 1))
            .collect::<Result<_>>()?
    };
    if crate::overflow::overflow_folders_in(&text) == folders {
        return Ok(false);
    }
    let next = crate::overflow::gameinfo_with_overflow(&text, &folders)?;
    let stamp = crate::util::now_iso().replace([':', '.'], "-");
    fs::copy(&gameinfo, gameinfo.with_file_name(format!("gameinfo.gi.dah-backup-{stamp}")))?;
    crate::util::write_atomic(&gameinfo, next.as_bytes())?;
    Ok(true)
}

/// GAME WRITE for a vanilla launch: take the extra addon folders out of `gameinfo.gi`, so the
/// game loads none of their mods. The files stay where they are; [`sync_overflow_mount`] mounts
/// them again when the stash is restored. Backs the file up first. Returns whether it changed.
pub fn unmount_overflow(addons: &Path) -> Result<bool> {
    let Some(citadel) = addons.parent() else {
        return Ok(false);
    };
    let gameinfo = citadel.join("gameinfo.gi");
    if !gameinfo.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(&gameinfo)?;
    if crate::overflow::overflow_folders_in(&text).is_empty() {
        return Ok(false);
    }
    let next = crate::overflow::gameinfo_with_overflow(&text, &[])?;
    let stamp = crate::util::now_iso().replace([':', '.'], "-");
    fs::copy(&gameinfo, gameinfo.with_file_name(format!("gameinfo.gi.dah-backup-{stamp}")))?;
    crate::util::write_atomic(&gameinfo, next.as_bytes())?;
    Ok(true)
}

/// Every file name in every addon folder, as `(folder index, file name)`. Plain files only.
pub fn folder_files(addons: &Path) -> std::io::Result<Vec<(usize, String)>> {
    let mut files = Vec::new();
    for (index, folder) in existing_folders(addons) {
        let read = match fs::read_dir(&folder) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in read.flatten() {
            if entry.path().is_file() {
                files.push((index, entry.file_name().to_string_lossy().into_owned()));
            }
        }
    }
    Ok(files)
}

/// One addon slot found in `game/citadel/addons` or one of its extra folders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAddon {
    pub slot: u16,
    /// File name on disk, including the `.disabled` suffix when disabled.
    pub file: String,
    /// Size of the directory VPK. Chunk files are counted separately.
    pub size_bytes: u64,
    /// `pakNN_MMM.vpk` files that belong to this slot, sorted.
    pub chunk_files: Vec<String>,
    /// Directory tree entry count, or None when the VPK could not be read.
    pub entry_count: Option<usize>,
    pub enabled: bool,
}

impl InstalledAddon {
    /// The folder this addon lives in, from the main addons folder.
    pub fn folder(&self, addons: &Path) -> std::path::PathBuf {
        slot_folder(addons, self.slot)
    }

    /// Full path of the directory file (`file`), in its own folder.
    pub fn path(&self, addons: &Path) -> std::path::PathBuf {
        self.folder(addons).join(&self.file)
    }

    /// Full paths of the chunk files.
    pub fn chunk_paths(&self, addons: &Path) -> Vec<std::path::PathBuf> {
        let folder = self.folder(addons);
        self.chunk_files.iter().map(|chunk| folder.join(chunk)).collect()
    }
}

/// A game path shipped by more than one enabled addon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonConflict {
    /// Game path with forward slashes, as stored in the VPK.
    pub path: String,
    /// Slots that ship the path, ascending.
    pub slots: Vec<u16>,
    /// Lowest slot, which the game loads first.
    pub winner: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConflictReport {
    pub conflicts: Vec<AddonConflict>,
    /// Unreadable VPKs and a missing addon folder. Never fatal.
    pub warnings: Vec<String>,
    pub addons: Vec<InstalledAddon>,
}

/// One approved enable or disable rename.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonToggleResult {
    pub slot: u16,
    /// Directory file name before the rename.
    pub from: String,
    /// Directory file name after the rename.
    pub to: String,
}

/// List installed slots, enabled and disabled, sorted by slot.
///
/// A slot file named `pakNN_dir.vpk` is enabled. The same file renamed to
/// `pakNN_dir.vpk.disabled` is present but off. When both names exist the
/// enabled one is reported. Chunk files are attached to their slot whether the
/// slot is enabled or not.
pub fn list_installed(game_root: &Path) -> Result<Vec<InstalledAddon>> {
    let directory = addons_dir(game_root);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let files = folder_files(&directory)?;

    let enabled_pattern =
        Regex::new(r"(?i)^pak(\d{2})_dir\.vpk$").expect("addon slot pattern is valid");
    let disabled_pattern =
        Regex::new(r"(?i)^pak(\d{2})_dir\.vpk\.disabled$").expect("disabled slot pattern is valid");
    let chunk_pattern =
        Regex::new(r"(?i)^pak(\d{2})_\d{3}\.vpk$").expect("addon chunk pattern is valid");

    let mut slots: BTreeMap<u16, (String, bool)> = BTreeMap::new();
    let mut chunks: BTreeMap<u16, Vec<String>> = BTreeMap::new();

    for (index, name) in files {
        let parse_slot = |capture| parse_slot(capture).map(|local| global_slot(index, local));

        if let Some(captures) = chunk_pattern.captures(&name) {
            if let Some(slot) = parse_slot(captures.get(1)) {
                chunks.entry(slot).or_default().push(name);
            }
            continue;
        }

        if let Some(captures) = enabled_pattern.captures(&name) {
            if let Some(slot) = parse_slot(captures.get(1)) {
                slots
                    .entry(slot)
                    .and_modify(|existing| {
                        if !existing.1 {
                            *existing = (name.clone(), true);
                        }
                    })
                    .or_insert((name, true));
            }
            continue;
        }

        if let Some(captures) = disabled_pattern.captures(&name) {
            if let Some(slot) = parse_slot(captures.get(1)) {
                slots.entry(slot).or_insert((name, false));
            }
        }
    }

    let mut addons = Vec::with_capacity(slots.len());

    for (slot, (file, enabled)) in slots {
        let path = slot_folder(&directory, slot).join(&file);
        let size_bytes = fs::metadata(&path).map(|data| data.len()).unwrap_or(0);
        let entry_count = read_vpk_directory_with(&path, &ReadVpkOptions {
            keep_preload: false,
        })
        .ok()
        .map(|parsed| parsed.entries.len());

        let mut chunk_files = chunks.remove(&slot).unwrap_or_default();
        chunk_files.sort();

        addons.push(InstalledAddon {
            slot,
            file,
            size_bytes,
            chunk_files,
            entry_count,
            enabled,
        });
    }

    addons.sort_by_key(|addon| addon.slot);
    Ok(addons)
}

/// Read every enabled addon and report game paths shipped by more than one slot.
///
/// Disabled slots are ignored because the game does not load them. An
/// unreadable directory VPK becomes a warning: its paths cannot be compared, so
/// the report is honest about what it could not check.
pub fn read_conflict_report(game_root: &Path) -> Result<ConflictReport> {
    let directory = addons_dir(game_root);
    let addons = list_installed(game_root)?;
    let mut warnings = Vec::new();

    if !directory.is_dir() {
        warnings.push(format!(
            "Addon folder {} does not exist. No addons are installed.",
            directory.display()
        ));
        return Ok(ConflictReport {
            conflicts: Vec::new(),
            warnings,
            addons,
        });
    }

    // Lowercased path -> (path as shipped, slots that ship it).
    let mut shipped: BTreeMap<String, (String, BTreeSet<u16>)> = BTreeMap::new();

    for addon in addons.iter().filter(|addon| addon.enabled) {
        let path = addon.path(&directory);

        let parsed = match read_vpk_directory_with(&path, &ReadVpkOptions {
            keep_preload: false,
        }) {
            Ok(parsed) => parsed,
            Err(error) => {
                warnings.push(format!(
                    "Addon {} could not be read, so its conflicts are unknown: {error}",
                    addon.file
                ));
                continue;
            }
        };

        for entry in parsed.entries {
            shipped
                .entry(entry.path.to_ascii_lowercase())
                .or_insert_with(|| (entry.path, BTreeSet::new()))
                .1
                .insert(addon.slot);
        }
    }

    let conflicts = shipped
        .into_values()
        .filter_map(|(path, slots)| {
            if slots.len() < 2 {
                return None;
            }

            let winner = *slots.first()?;

            Some(AddonConflict {
                path,
                slots: slots.into_iter().collect(),
                winner,
            })
        })
        .collect();

    Ok(ConflictReport {
        conflicts,
        warnings,
        addons,
    })
}

/// Conflicts only. [`read_conflict_report`] keeps the warnings and inventory.
pub fn find_conflicts(game_root: &Path) -> Result<Vec<AddonConflict>> {
    read_conflict_report(game_root).map(|report| report.conflicts)
}

/// Enable or disable one addon slot with a single rename. Refuses without
/// `approve`.
///
/// Enabling renames `pakNN_dir.vpk.disabled` back to `pakNN_dir.vpk`. Disabling
/// renames `pakNN_dir.vpk` to `pakNN_dir.vpk.disabled`. The call refuses when
/// the slot number is outside 1 to 99, when the addon folder is missing, when
/// both names already exist, when neither name exists, or when the slot is
/// already in the requested state. Nothing is deleted, renumbered or renamed
/// around, and chunk files are never touched: they stay attached to the slot
/// while it is off.
pub fn set_enabled(
    game_root: &Path,
    slot: u16,
    enabled: bool,
    approve: bool,
) -> Result<AddonToggleResult> {
    if !is_slot(slot) {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to {MAX_SLOT}. Nothing was written."
        )));
    }

    let directory = slot_folder(&addons_dir(game_root), slot);
    let action = if enabled { "enable" } else { "disable" };

    if !approve {
        return Err(Error::msg(format!(
            "Not approved: {action} slot {slot:02} in {}. Nothing was written. Re-run with --approve.",
            directory.display()
        )));
    }

    if !directory.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Slot {slot:02} cannot be {action}d. Nothing was written.",
            directory.display()
        )));
    }

    let enabled_name = slot_name(slot, "_dir.vpk");
    let disabled_name = format!("{enabled_name}{DISABLED_SUFFIX}");
    let enabled_path = directory.join(&enabled_name);
    let disabled_path = directory.join(&disabled_name);
    let has_enabled = enabled_path.is_file();
    let has_disabled = disabled_path.is_file();

    if has_enabled && has_disabled {
        return Err(Error::msg(format!(
            "Slot {slot:02} has both {enabled_name} and {disabled_name} in {}. Remove one before changing its state. Nothing was written.",
            directory.display()
        )));
    }

    let (from_name, to_name, from_path, to_path) = if enabled {
        (&disabled_name, &enabled_name, &disabled_path, &enabled_path)
    } else {
        (&enabled_name, &disabled_name, &enabled_path, &disabled_path)
    };

    if to_path.is_file() {
        return Err(Error::msg(format!(
            "Slot {slot:02} is already {state}: {to_name} exists in {}. Nothing was written.",
            directory.display(),
            state = if enabled { "enabled" } else { "disabled" }
        )));
    }

    if !from_path.is_file() {
        return Err(Error::msg(format!(
            "Slot {slot:02} is not installed: neither {enabled_name} nor {disabled_name} exists in {}. Nothing was written.",
            directory.display()
        )));
    }

    fs::rename(from_path, to_path).map_err(|error| {
        Error::msg(format!(
            "Could not {action} slot {slot:02}: rename {} to {} failed: {error}",
            from_path.display(),
            to_path.display()
        ))
    })?;

    Ok(AddonToggleResult {
        slot,
        from: from_name.clone(),
        to: to_name.clone(),
    })
}

/// Undo a set of enable or disable renames: each `to` name goes back to its `from` name.
/// Game install write; refuses without `approve`. Every rename is checked first (the `to`
/// file is there, the `from` name is free, both are directory names of the result's slot), so
/// nothing moves when the folder changed since. Returns the reverted results.
pub fn revert_toggles(
    game_root: &Path,
    results: &[AddonToggleResult],
    approve: bool,
) -> Result<Vec<AddonToggleResult>> {
    if !approve {
        return Err(Error::msg("Not approved: undo the switch. Nothing was renamed."));
    }
    let addons = addons_dir(game_root);
    let enabled_name = |slot: u16| slot_name(slot, "_dir.vpk");
    for result in results {
        let directory = slot_folder(&addons, result.slot);
        let names = [enabled_name(result.slot), format!("{}{DISABLED_SUFFIX}", enabled_name(result.slot))];
        if !names.contains(&result.from) || !names.contains(&result.to) || result.from == result.to {
            return Err(Error::msg(format!(
                "{} to {} is not a switch of slot {:02}. Nothing was renamed.",
                result.from, result.to, result.slot
            )));
        }
        if !directory.join(&result.to).is_file() || directory.join(&result.from).exists() {
            return Err(Error::msg(format!(
                "Slot {:02} changed since the switch. Nothing was renamed.",
                result.slot
            )));
        }
    }
    let mut reverted = Vec::new();
    for result in results {
        let directory = slot_folder(&addons, result.slot);
        fs::rename(directory.join(&result.to), directory.join(&result.from))?;
        reverted.push(AddonToggleResult {
            slot: result.slot,
            from: result.to.clone(),
            to: result.from.clone(),
        });
    }
    Ok(reverted)
}

/// What [`set_many`] did: the renames made (for an undo) and the slots that failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetManyReport {
    pub done: Vec<AddonToggleResult>,
    pub failed: Vec<(u16, String)>,
}

/// GAME WRITE. Put several slots in a wanted state, each one the single rename of
/// [`set_enabled`]. Slots already in that state are skipped quietly; a slot that fails stops
/// nothing else. Refuses everything without `approve`. Used by solo test, the broken mod
/// search, variant pickers and mod lists.
pub fn set_many(game_root: &Path, wanted: &BTreeMap<u16, bool>, approve: bool) -> Result<SetManyReport> {
    if !approve {
        return Err(Error::msg("Not approved: switch several mods. Nothing was renamed."));
    }
    let installed = list_installed(game_root)?;
    let mut report = SetManyReport::default();
    for (&slot, &enable) in wanted {
        match installed.iter().find(|addon| addon.slot == slot) {
            None => report.failed.push((slot, format!("Slot {slot:02} is not installed."))),
            Some(addon) if addon.enabled == enable => {}
            Some(_) => match set_enabled(game_root, slot, enable, true) {
                Ok(result) => report.done.push(result),
                Err(error) => report.failed.push((slot, error.to_string())),
            },
        }
    }
    Ok(report)
}

fn parse_slot(capture: Option<regex::Match<'_>>) -> Option<u16> {
    let slot = capture?.as_str().parse::<u16>().ok()?;
    (1..=99).contains(&slot).then_some(slot)
}

#[cfg(test)]
mod revert_tests {
    use super::*;

    #[test]
    fn revert_toggles_undoes_a_bulk_switch() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/addons-test")
            .join(format!("revert-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let addons = addons_dir(&root);
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), "a").unwrap();
        fs::write(addons.join("pak02_dir.vpk.disabled"), "b").unwrap();
        let done = vec![
            set_enabled(&root, 1, false, true).unwrap(),
            set_enabled(&root, 2, true, true).unwrap(),
        ];
        assert!(revert_toggles(&root, &done, false).is_err());
        let bogus = vec![AddonToggleResult { slot: 1, from: "../x".into(), to: "pak01_dir.vpk.disabled".into() }];
        assert!(revert_toggles(&root, &bogus, true).is_err());
        revert_toggles(&root, &done, true).unwrap();
        assert!(addons.join("pak01_dir.vpk").is_file());
        assert!(addons.join("pak02_dir.vpk.disabled").is_file());
        // A second undo finds the folder changed and does nothing.
        assert!(revert_toggles(&root, &done, true).is_err());
    }

    #[test]
    fn set_many_switches_only_what_differs() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/addons-test")
            .join(format!("many-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let addons = addons_dir(&root);
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), "a").unwrap();
        fs::write(addons.join("pak02_dir.vpk.disabled"), "b").unwrap();
        fs::write(addons.join("pak03_dir.vpk"), "c").unwrap();
        let wanted: BTreeMap<u16, bool> = [(1, false), (2, true), (3, true), (9, true)].into();
        assert!(set_many(&root, &wanted, false).is_err());
        assert!(addons.join("pak01_dir.vpk").is_file());
        let report = set_many(&root, &wanted, true).unwrap();
        assert_eq!(report.done.len(), 2);
        assert_eq!(report.failed.len(), 1);
        assert!(addons.join("pak01_dir.vpk.disabled").is_file());
        assert!(addons.join("pak02_dir.vpk").is_file());
        revert_toggles(&root, &report.done, true).unwrap();
        assert!(addons.join("pak01_dir.vpk").is_file());
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use crate::load_order::{SlotMove, apply_reorder, plan_reorder, read_slot_files};

    const GAMEINFO: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel/addons\n\t\t\tGame \"citadel\"\n\t\t\tGame \"core\"\n\t\t}\n\t}\n}\n";

    /// `<tmp>/game/citadel` with an addons folder and a stock gameinfo.gi.
    fn game(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("sigil-overflow-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let citadel = root.join("game").join("citadel");
        fs::create_dir_all(citadel.join("addons")).unwrap();
        fs::write(citadel.join("gameinfo.gi"), GAMEINFO).unwrap();
        root
    }

    #[test]
    fn slots_map_to_folders_and_back() {
        assert_eq!((folder_index(1), local_slot(1)), (0, 1));
        assert_eq!((folder_index(99), local_slot(99)), (0, 99));
        assert_eq!((folder_index(100), local_slot(100)), (1, 1));
        assert_eq!((folder_index(198), local_slot(198)), (1, 99));
        assert_eq!((folder_index(MAX_SLOT), local_slot(MAX_SLOT)), (9, 99));
        assert_eq!(global_slot(1, 5), 104);
        assert_eq!(slot_name(104, "_dir.vpk"), "pak05_dir.vpk");
        assert_eq!(slot_label(5), "05");
        assert_eq!(slot_label(104), "2·05");
        let addons = Path::new("/g/game/citadel/addons");
        assert_eq!(slot_folder(addons, 104), Path::new("/g/game/citadel/sigil-addons2"));
        assert!(!is_slot(0) && !is_slot(MAX_SLOT + 1));
    }

    #[test]
    fn mods_in_an_extra_folder_are_listed_with_their_global_slot() {
        let root = game("list");
        let addons = addons_dir(&root);
        fs::write(addons.join("pak05_dir.vpk"), b"main").unwrap();
        let extra = slot_folder(&addons, 100);
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("pak05_dir.vpk.disabled"), b"extra").unwrap();
        fs::write(extra.join("pak05_000.vpk"), b"chunk").unwrap();

        let listed = list_installed(&root).unwrap();
        let slots: Vec<(u16, bool)> = listed.iter().map(|a| (a.slot, a.enabled)).collect();
        assert_eq!(slots, vec![(5, true), (104, false)]);
        assert_eq!(listed[1].path(&addons), extra.join("pak05_dir.vpk.disabled"));
        assert_eq!(listed[1].chunk_files, vec!["pak05_000.vpk".to_string()]);

        set_enabled(&root, 104, true, true).unwrap();
        assert!(extra.join("pak05_dir.vpk").is_file());
        assert!(addons.join("pak05_dir.vpk").is_file(), "the main folder's pak05 is untouched");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_reorder_across_folders_moves_files_and_mounts_the_folder() {
        let root = game("reorder");
        let addons = addons_dir(&root);
        let gameinfo = addons.parent().unwrap().join("gameinfo.gi");
        fs::write(addons.join("pak07_dir.vpk"), b"seven").unwrap();
        fs::write(addons.join("pak07_000.vpk"), b"chunk").unwrap();

        let files = read_slot_files(&addons).unwrap();
        let plan = plan_reorder(&addons, &files, &[SlotMove { from: 7, to: 120 }], "t").unwrap();
        apply_reorder(&plan, true).unwrap();
        let extra = slot_folder(&addons, 120);
        assert_eq!(fs::read(extra.join("pak21_dir.vpk")).unwrap(), b"seven");
        assert!(extra.join("pak21_000.vpk").is_file());
        assert!(!addons.join("pak07_dir.vpk").exists());
        let text = fs::read_to_string(&gameinfo).unwrap();
        assert_eq!(crate::overflow::overflow_folders_in(&text), vec!["sigil-addons2".to_string()]);

        // Moving it back empties the extra folder: the mount and the folder go away.
        let files = read_slot_files(&addons).unwrap();
        let plan = plan_reorder(&addons, &files, &[SlotMove { from: 120, to: 7 }], "u").unwrap();
        apply_reorder(&plan, true).unwrap();
        assert!(addons.join("pak07_dir.vpk").is_file());
        assert!(!extra.exists(), "an empty extra folder is removed");
        let text = fs::read_to_string(&gameinfo).unwrap();
        assert!(crate::overflow::overflow_folders_in(&text).is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_isolated_profile_gets_no_extra_mounts_and_vanilla_unmounts() {
        let root = game("mounts");
        let addons = addons_dir(&root);
        let gameinfo = addons.parent().unwrap().join("gameinfo.gi");
        let extra = slot_folder(&addons, 150);
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join(slot_name(150, "_dir.vpk")), b"x").unwrap();

        assert!(sync_overflow_mount(&addons).unwrap());
        assert!(!sync_overflow_mount(&addons).unwrap(), "a second sync changes nothing");
        assert!(unmount_overflow(&addons).unwrap());
        assert!(crate::overflow::overflow_folders_in(&fs::read_to_string(&gameinfo).unwrap()).is_empty());
        assert!(extra.join(slot_name(150, "_dir.vpk")).is_file(), "vanilla keeps the files");

        let isolated = GAMEINFO.replace(
            "\t\t\tGame citadel/addons\n",
            "\t\t\t// sigil:profile-begin\n\t\t\tGame citadel/addons/sigil-ranked\n\t\t\t// sigil:profile-end\n",
        );
        fs::write(&gameinfo, isolated).unwrap();
        assert!(!sync_overflow_mount(&addons).unwrap(), "nothing extra mounts beside an isolated profile");
        let _ = fs::remove_dir_all(root);
    }
}
