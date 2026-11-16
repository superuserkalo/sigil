//! Named sets of addon slots, captured from and applied to a game install.
//!
//! A profile records which addon slots were enabled when it was captured. It
//! never stores VPK contents, only the slot number, the file name on disk and
//! whether the slot was enabled.
//!
//! Profiles live in `<workspace>/.dah/profiles/<slug>.json`. Reading and
//! writing a profile touches the workspace only. [`apply_profile`] renames slot
//! files inside `game/citadel/addons`, and only behind an approval flag. It
//! never deletes a file, never renumbers a slot and never touches a slot the
//! profile does not record.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::addons::list_installed;
use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

pub const PROFILE_SCHEMA: u32 = 1;
pub const PROFILE_FILE_SUFFIX: &str = ".json";
pub const DISABLED_SUFFIX: &str = ".disabled";

/// One addon slot as recorded by a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSlot {
    pub slot: u8,
    /// File name as it was on disk at capture time, with `.disabled` when off.
    pub file: String,
    pub enabled: bool,
}

/// A saved set of addon slots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub schema: u32,
    pub name: String,
    pub slug: String,
    pub created_at: String,
    pub updated_at: String,
    pub slots: Vec<ProfileSlot>,
}

/// What an approved profile apply changed and what it could not do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileApplyResult {
    /// One line per rename, in slot order.
    pub changed: Vec<String>,
    /// Missing files and slots the profile could not restore. Never fatal.
    pub warnings: Vec<String>,
}

impl Profile {
    pub fn enabled_slots(&self) -> usize {
        self.slots.iter().filter(|slot| slot.enabled).count()
    }
}

pub fn profile_dir(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join("profiles")
}

pub fn profile_path(workspace: &Path, slug: &str) -> PathBuf {
    profile_dir(workspace).join(format!("{slug}{PROFILE_FILE_SUFFIX}"))
}

/// Read every profile in the workspace, sorted by name.
///
/// A missing profile folder is an empty list, not an error. A malformed or
/// inconsistent profile file is an error that names the file: silently skipping
/// it would hide a broken profile from the manager.
pub fn list_profiles(workspace: &Path) -> Result<Vec<Profile>> {
    let directory = profile_dir(workspace);

    let read_dir = match fs::read_dir(&directory) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut profiles = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let profile = read_profile_file(&path)?;
        let stem = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();

        if stem != profile.slug {
            return Err(Error::msg(format!(
                "Profile file {} declares slug {:?}. Rename the file or fix the slug.",
                path.display(),
                profile.slug
            )));
        }

        profiles.push(profile);
    }

    profiles.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(profiles)
}

/// Capture the current addon slots and save them under `name`.
///
/// An existing profile with the same slug keeps its `createdAt`; only the slot
/// list, name and `updatedAt` change.
pub fn capture_profile(game_root: &Path, workspace: &Path, name: &str) -> Result<Profile> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::msg("Profile name must not be empty."));
    }

    let slug = slugify(name);
    if slug.is_empty() {
        return Err(Error::msg(
            "Profile name must contain at least one letter or digit.",
        ));
    }

    validate_slug(&slug)?;

    let addons = list_installed(game_root)?;
    let slots: Vec<ProfileSlot> = addons
        .into_iter()
        .map(|addon| ProfileSlot {
            slot: addon.slot,
            file: addon.file,
            enabled: addon.enabled,
        })
        .collect();

    let path = profile_path(workspace, &slug);
    let created_at = if path.is_file() {
        read_profile(workspace, &slug)?.created_at
    } else {
        now_iso()
    };

    let profile = Profile {
        schema: PROFILE_SCHEMA,
        name: name.to_string(),
        slug,
        created_at,
        updated_at: now_iso(),
        slots,
    };

    write_profile(workspace, &profile)?;
    Ok(profile)
}

pub fn read_profile(workspace: &Path, slug: &str) -> Result<Profile> {
    validate_slug(slug)?;
    let path = profile_path(workspace, slug);
    read_profile_file(&path)
}

/// Write one profile under its slug. The file is replaced atomically.
pub fn write_profile(workspace: &Path, profile: &Profile) -> Result<()> {
    validate_profile(profile)?;

    let mut text = serde_json::to_string_pretty(profile)?;
    text.push('\n');
    write_atomic(&profile_path(workspace, &profile.slug), text.as_bytes())
}

/// Resolve a profile by slug or display name, case-insensitive for the name.
pub fn find_profile(workspace: &Path, name: &str) -> Result<Profile> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::msg("Profile name must not be empty."));
    }

    let profiles = list_profiles(workspace)?;
    let slug = slugify(trimmed);

    if let Some(found) = profiles.iter().find(|profile| {
        profile.slug == trimmed
            || profile.slug == slug
            || profile.name.eq_ignore_ascii_case(trimmed)
    }) {
        return Ok(found.clone());
    }

    let available = if profiles.is_empty() {
        "none".to_string()
    } else {
        profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };

    Err(Error::msg(format!(
        "Profile {:?} does not exist in {}. Saved profiles: {available}.",
        trimmed,
        profile_dir(workspace).display()
    )))
}

/// Apply a profile to the addon folder. Refuses without `approve`.
///
/// For each recorded slot, an enabled slot is renamed from
/// `pakNN_dir.vpk.disabled` back to `pakNN_dir.vpk` and a disabled slot is
/// renamed the other way. Chunk files are left alone. A slot whose two names
/// both exist is ambiguous, so the whole apply refuses before renaming anything.
pub fn apply_profile(
    game_root: &Path,
    workspace: &Path,
    slug: &str,
    approve: bool,
) -> Result<ProfileApplyResult> {
    let profile = read_profile(workspace, slug)?;
    let addons = addons_dir(game_root);

    if !approve {
        return Err(Error::msg(format!(
            "Applying profile {} to {} was not approved. Nothing was written.",
            profile.name,
            addons.display()
        )));
    }

    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was written.",
            addons.display()
        )));
    }

    // Check every slot before renaming anything, so a refusal leaves addons as
    // they were instead of half applied.
    let mut plan: Vec<(u8, String, String, bool)> = Vec::with_capacity(profile.slots.len());

    for record in &profile.slots {
        let (enabled_name, disabled_name) = slot_file_names(&record.file)?;
        let enabled_path = addons.join(&enabled_name);
        let disabled_path = addons.join(&disabled_name);
        let has_enabled = enabled_path.is_file();
        let has_disabled = disabled_path.is_file();

        if has_enabled && has_disabled {
            return Err(Error::msg(format!(
                "Slot {:02} has both {} and {} in {}. Remove one before applying a profile. Nothing was written.",
                record.slot,
                enabled_name,
                disabled_name,
                addons.display()
            )));
        }

        plan.push((record.slot, enabled_name, disabled_name, record.enabled));
    }

    let mut changed = Vec::new();
    let mut warnings = Vec::new();

    for (slot, enabled_name, disabled_name, want_enabled) in plan {
        let enabled_path = addons.join(&enabled_name);
        let disabled_path = addons.join(&disabled_name);

        if want_enabled {
            if enabled_path.is_file() {
                continue;
            }

            if !disabled_path.is_file() {
                warnings.push(format!(
                    "slot {slot:02}: neither {enabled_name} nor {disabled_name} exists in {}.",
                    addons.display()
                ));
                continue;
            }

            fs::rename(&disabled_path, &enabled_path).map_err(|error| {
                Error::msg(format!(
                    "Could not enable slot {slot:02}: rename {} to {} failed: {error}",
                    disabled_path.display(),
                    enabled_path.display()
                ))
            })?;
            changed.push(format!("slot {slot:02}: {disabled_name} -> {enabled_name}"));
        } else {
            if disabled_path.is_file() {
                continue;
            }

            if !enabled_path.is_file() {
                warnings.push(format!(
                    "slot {slot:02}: neither {enabled_name} nor {disabled_name} exists in {}.",
                    addons.display()
                ));
                continue;
            }

            fs::rename(&enabled_path, &disabled_path).map_err(|error| {
                Error::msg(format!(
                    "Could not disable slot {slot:02}: rename {} to {} failed: {error}",
                    enabled_path.display(),
                    disabled_path.display()
                ))
            })?;
            changed.push(format!("slot {slot:02}: {enabled_name} -> {disabled_name}"));
        }
    }

    Ok(ProfileApplyResult { changed, warnings })
}

fn read_profile_file(path: &Path) -> Result<Profile> {
    let text = fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "Profile {} does not exist. Save it with `dah profiles save <name>` first.",
                path.display()
            ))
        } else {
            Error::msg(format!("Could not read {}: {error}", path.display()))
        }
    })?;

    let profile: Profile = serde_json::from_str(&text)
        .map_err(|error| Error::msg(format!("{} is not a valid profile: {error}", path.display())))?;

    validate_profile(&profile)?;
    Ok(profile)
}

fn validate_profile(profile: &Profile) -> Result<()> {
    if profile.schema != PROFILE_SCHEMA {
        return Err(Error::msg(format!(
            "Profile {:?} uses unsupported schema {}. This build reads schema {PROFILE_SCHEMA}.",
            profile.name, profile.schema
        )));
    }

    if profile.name.trim().is_empty() {
        return Err(Error::msg("Profile name must not be empty."));
    }

    validate_slug(&profile.slug)?;

    let mut seen: Vec<u8> = Vec::with_capacity(profile.slots.len());

    for record in &profile.slots {
        if !(1..=99).contains(&record.slot) {
            return Err(Error::msg(format!(
                "Profile {:?} records slot {}, outside 1 to 99.",
                profile.name, record.slot
            )));
        }

        if seen.contains(&record.slot) {
            return Err(Error::msg(format!(
                "Profile {:?} records slot {:02} more than once.",
                profile.name, record.slot
            )));
        }

        let (enabled_name, _) = slot_file_names(&record.file)?;
        let recorded = slot_number(&enabled_name).ok_or_else(|| {
            Error::msg(format!(
                "Profile {:?} records file {:?}, which is not pakNN_dir.vpk.",
                profile.name, record.file
            ))
        })?;

        if recorded != record.slot {
            return Err(Error::msg(format!(
                "Profile {:?} records slot {:02} but file {:?}. Fix the slot or the file.",
                profile.name, record.slot, record.file
            )));
        }

        seen.push(record.slot);
    }

    Ok(())
}

/// Split a recorded slot file into its enabled and disabled names.
///
/// Only `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled` are accepted. A profile is
/// data on disk, so this rejects separators and any other file shape before a
/// rename can reach outside the addon folder.
fn slot_file_names(file: &str) -> Result<(String, String)> {
    let enabled = match file.strip_suffix(DISABLED_SUFFIX) {
        Some(value) => value,
        None => file,
    };

    if slot_number(enabled).is_none() {
        return Err(Error::msg(format!(
            "Profile file {file:?} must be pakNN_dir.vpk or pakNN_dir.vpk.disabled."
        )));
    }

    Ok((enabled.to_string(), format!("{enabled}{DISABLED_SUFFIX}")))
}

/// The `NN` of `pakNN_dir.vpk`, or None when the name does not match.
fn slot_number(file: &str) -> Option<u8> {
    let lower = file.to_ascii_lowercase();
    let stem = lower.strip_prefix("pak")?.strip_suffix("_dir.vpk")?;

    if stem.len() != 2 || !stem.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    let slot = stem.parse::<u8>().ok()?;
    (1..=99).contains(&slot).then_some(slot)
}

fn validate_slug(slug: &str) -> Result<()> {
    let pattern = regex::Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("profile slug pattern is valid");

    if slug.len() > 64 {
        return Err(Error::msg("Profile slug must be 64 characters or fewer."));
    }

    if !pattern.is_match(slug) {
        return Err(Error::msg(format!(
            "Profile slug {slug:?} must use lowercase letters, digits and hyphens."
        )));
    }

    Ok(())
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('-');
            last_was_separator = true;
        }
    }

    slug.trim_matches('-').to_string()
}
