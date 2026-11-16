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

/// One addon slot found in `game/citadel/addons`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAddon {
    pub slot: u8,
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

/// A game path shipped by more than one enabled addon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonConflict {
    /// Game path with forward slashes, as stored in the VPK.
    pub path: String,
    /// Slots that ship the path, ascending.
    pub slots: Vec<u8>,
    /// Lowest slot, which the game loads first.
    pub winner: u8,
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
    pub slot: u8,
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

    let read_dir = match fs::read_dir(&directory) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let enabled_pattern =
        Regex::new(r"(?i)^pak(\d{2})_dir\.vpk$").expect("addon slot pattern is valid");
    let disabled_pattern =
        Regex::new(r"(?i)^pak(\d{2})_dir\.vpk\.disabled$").expect("disabled slot pattern is valid");
    let chunk_pattern =
        Regex::new(r"(?i)^pak(\d{2})_\d{3}\.vpk$").expect("addon chunk pattern is valid");

    let mut slots: BTreeMap<u8, (String, bool)> = BTreeMap::new();
    let mut chunks: BTreeMap<u8, Vec<String>> = BTreeMap::new();

    for entry in read_dir.flatten() {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();

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
        let path = directory.join(&file);
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
    let mut shipped: BTreeMap<String, (String, BTreeSet<u8>)> = BTreeMap::new();

    for addon in addons.iter().filter(|addon| addon.enabled) {
        let path = directory.join(&addon.file);

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
    slot: u8,
    enabled: bool,
    approve: bool,
) -> Result<AddonToggleResult> {
    if !(1..=99).contains(&slot) {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to 99. Addon slots are pak01_dir.vpk to pak99_dir.vpk."
        )));
    }

    let directory = addons_dir(game_root);
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

    let enabled_name = format!("pak{slot:02}_dir.vpk");
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

fn parse_slot(capture: Option<regex::Match<'_>>) -> Option<u8> {
    let slot = capture?.as_str().parse::<u8>().ok()?;
    (1..=99).contains(&slot).then_some(slot)
}
