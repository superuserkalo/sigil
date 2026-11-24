//! Profiles: named sets of enabled mods, with the crosshair and autoexec lines that go with them.
//!
//! Ported from Grimoire's profiles (MIT, `electron/main/services/profiles.ts`,
//! `profileResolver.ts` and `portableProfile.ts`), reshaped for Sigil's slot model:
//!
//! - A profile records every mod that was on when it was saved, with its identity: the slot
//!   and file name it had, a content fingerprint, and the GameBanana mod and file ids when the
//!   mod came from GameBanana. It also records the autoexec lines and, when asked, the
//!   crosshair. It never stores VPK contents.
//! - Applying resolves each recorded mod against the addon folder by GameBanana ids first, then
//!   by fingerprint, then by slot (refusing a slot that now holds a different mod), turns every
//!   other enabled mod off, and writes the autoexec when the profile carries one.
//! - A profile can be shared as a portable `mod-profile` document: a `.modprofile.json` file or
//!   an `mp1:` share code (base64url of gzip JSON). Import resolves each entry against
//!   GameBanana, reuses mods that are already installed and can download the rest through the
//!   install pipeline.
//!
//! Profiles live in `<workspace>/.dah/profiles/<slug>.json`. Reading and writing them touches
//! the workspace only. Game install writes, each refusing without `approve`:
//! [`apply_profile`] (renames slot files in `game/citadel/addons` and writes `autoexec.cfg`
//! through [`crate::autoexec::write_autoexec`], which keeps a backup) and
//! [`install_import_entry`] (runs the approved install pipeline for one missing mod).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::autoexec::{read_autoexec, write_autoexec};
use crate::crosshair::{AUTOEXEC_BEGIN, AUTOEXEC_END, Crosshair};
use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

mod isolation;
mod snapshots;

pub(crate) use isolation::{block_folder as profile_block_folder, BLOCK_BEGIN as PROFILE_BLOCK_BEGIN};
pub use isolation::{
    folder_dir as profile_folder_dir, folder_name as profile_folder_name, mount_gameinfo,
    mounted_folder, plan_mount, sync_folder, FolderSync, MountWrite,
};
pub use snapshots::{
    capture as capture_snapshot, delete as delete_snapshot,
    delete_many as delete_snapshots, list as list_snapshots, read as read_snapshot,
    write as write_snapshot, Snapshot, SnapshotSummary, SnapshotTrigger,
};

/// Schema written by this build. Schema 1 files (slots only) still read.
pub const PROFILE_SCHEMA: u32 = 2;
const OLDEST_PROFILE_SCHEMA: u32 = 1;
pub const PROFILE_FILE_SUFFIX: &str = ".json";
pub const DISABLED_SUFFIX: &str = ".disabled";
/// Plain text file beside the profiles holding the slug of the last applied profile.
pub const ACTIVE_FILE_NAME: &str = "active";
/// Where Sigil remembers which GameBanana mod an installed VPK came from.
pub const SOURCES_FILE_NAME: &str = "sources.json";

pub const PORTABLE_FORMAT: &str = "mod-profile";
pub const PORTABLE_SCHEMA_VERSION: &str = "1.1";
pub const SHARE_CODE_PREFIX: &str = "mp1:";
pub const PORTABLE_FILE_EXTENSION: &str = ".modprofile.json";
/// Inflated size cap for a share code, the same 256 KB the format's reference reader uses.
pub const MAX_INFLATED_SHARE_CODE_BYTES: usize = 256 * 1024;
/// A profile file larger than this is not a profile.
pub const MAX_PORTABLE_FILE_BYTES: u64 = 1024 * 1024;
pub const DEADLOCK_STEAM_APP_ID: u64 = 1_422_450;
pub const DEADLOCK_GAMEBANANA_GAME_ID: u64 = 20_948;
/// This app's namespace under `extensions`.
pub const EXTENSION_KEY: &str = "sigil";
const FOREIGN_EXTENSION_KEY: &str = "grimoire";

const FINGERPRINT_WINDOW: u64 = 64 * 1024;
const MAX_PROFILE_NAME_CHARS: usize = 64;
const MAX_HINT_CHARS: usize = 200;
const MAX_AUTOEXEC_LINES: usize = 512;
const GAMEBANANA_API: &str = "https://gamebanana.com/apiv11/Mod";
const GAMEBANANA_USER_AGENT: &str = "sigil/0.1";
const API_TIMEOUT_SECONDS: u64 = 30;

// ---------------------------------------------------------------------------
// Saved profiles
// ---------------------------------------------------------------------------

/// One mod as recorded by a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSlot {
    pub slot: u16,
    /// File name as it was on disk at capture time, with `.disabled` when off.
    pub file: String,
    pub enabled: bool,
    /// Display name, when Sigil knows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamebanana_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamebanana_file_id: Option<u64>,
    /// [`fingerprint_file`] of the directory VPK at capture time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

impl ProfileSlot {
    /// What to call this mod in a list: its name, else the GameBanana id, else the slot.
    pub fn label(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|name| !name.trim().is_empty()) {
            return name.to_string();
        }
        if let Some(id) = self.gamebanana_id {
            return format!("GameBanana mod {id}");
        }
        format!("Slot {:02}", self.slot)
    }
}

/// A saved set of mods plus the settings that travel with it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub schema: u32,
    pub name: String,
    pub slug: String,
    pub created_at: String,
    pub updated_at: String,
    pub slots: Vec<ProfileSlot>,
    /// The crosshair to write on apply. Only saved when the user asks for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosshair: Option<Crosshair>,
    /// Autoexec lines outside the crosshair block. None leaves autoexec alone on apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoexec: Option<Vec<String>>,
    /// Self-reported author of an imported profile. Plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Extension namespaces from an imported profile, kept for a later export.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extensions: Map<String, Value>,
    /// The profile keeps its own mods folder: applying it swaps the addon folder's slot files
    /// with `citadel/sigil_profiles/<slug>` instead of switching mods on and off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isolated: bool,
}

/// The Locker's shuffle state a profile carries (Grimoire's profiles keep it, G #587): the
/// heroes whose skins shuffle, the skins in the pool and whether Equip adds a skin. Kept under
/// this app's extension namespace, so the profile file needs no new schema and an export,
/// which rebuilds that namespace, leaves it out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct LockerShuffle {
    /// Hero asset keys whose skins shuffle on launch.
    pub shuffle: Vec<String>,
    /// Skins opted into the shuffle pool, by the Locker's content key.
    pub shuffle_pool: Vec<String>,
    /// Equip adds a skin instead of replacing the hero's others.
    pub multi_skin: bool,
}

const LOCKER_KEY: &str = "locker";

impl Profile {
    /// The Locker shuffle saved with this profile. `None` for a profile saved before profiles
    /// carried it: applying one of those leaves the Locker as it is.
    pub fn locker_shuffle(&self) -> Option<LockerShuffle> {
        let value = self.extensions.get(EXTENSION_KEY)?.get(LOCKER_KEY)?;
        serde_json::from_value(value.clone()).ok()
    }

    /// Store the Locker shuffle in this profile, keeping anything else in the namespace.
    pub fn set_locker_shuffle(&mut self, shuffle: &LockerShuffle) {
        let entry = self
            .extensions
            .entry(EXTENSION_KEY.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        if let (Some(map), Ok(value)) = (entry.as_object_mut(), serde_json::to_value(shuffle)) {
            map.insert(LOCKER_KEY.to_string(), value);
        }
    }
}

/// Save the Locker shuffle into the profile `slug`. Writes the workspace only; returns false
/// when the profile already held exactly this state, so nothing was written.
pub fn store_locker_shuffle(workspace: &Path, slug: &str, shuffle: &LockerShuffle) -> Result<bool> {
    let mut profile = read_profile(workspace, slug)?;
    if profile.locker_shuffle().as_ref() == Some(shuffle) {
        return Ok(false);
    }
    profile.set_locker_shuffle(shuffle);
    write_profile(workspace, &profile)?;
    Ok(true)
}

/// What an approved profile apply changed and what it could not do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileApplyResult {
    /// One line per rename or copy, in slot order.
    pub changed: Vec<String>,
    /// Mods the profile names that are not installed, or whose slot now holds another mod.
    pub warnings: Vec<String>,
    /// True when `autoexec.cfg` was rewritten.
    #[serde(default)]
    pub autoexec_written: bool,
    /// The copy of `autoexec.cfg` taken before it was rewritten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoexec_backup: Option<PathBuf>,
    /// The renames made in the addons folder, newest last. Reverse them to undo.
    #[serde(default)]
    pub changes: Vec<SlotChange>,
    /// The profile that was active before this apply, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_active: Option<String>,
    /// The profile folder mounted after an isolated apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The profile folder mounted before it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_folder: Option<String>,
    /// Mods copied into a profile folder by an isolated apply.
    #[serde(default)]
    pub copied: usize,
    /// The recovery point taken just before the apply, when one could be saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
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
/// A missing profile folder is an empty list, not an error. A malformed or inconsistent profile
/// file is an error that names the file: silently skipping it would hide a broken profile.
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
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        if stem == SOURCES_FILE_NAME.trim_end_matches(".json") {
            continue;
        }

        let profile = read_profile_file(&path)?;
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

pub fn read_profile(workspace: &Path, slug: &str) -> Result<Profile> {
    validate_slug(slug)?;
    read_profile_file(&profile_path(workspace, slug))
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
        profile.slug == trimmed || profile.slug == slug || profile.name.eq_ignore_ascii_case(trimmed)
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

/// What the game has right now: the mods that are on, the autoexec lines and the crosshair.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentSetup {
    pub slots: Vec<ProfileSlot>,
    /// None when there is no `autoexec.cfg`.
    pub autoexec: Option<Vec<String>>,
    /// The crosshair block in `autoexec.cfg`, when there is one.
    pub crosshair: Option<Crosshair>,
}

/// Read the current setup without writing anything.
pub fn read_current_setup(game_root: &Path, workspace: &Path) -> Result<CurrentSetup> {
    let identities = installed_identities(game_root, workspace)?;
    let slots = identities
        .iter()
        .filter(|addon| addon.enabled)
        .map(|addon| ProfileSlot {
            slot: addon.slot,
            file: addon.file.clone(),
            enabled: true,
            name: addon.name.clone(),
            gamebanana_id: addon.gamebanana_id,
            gamebanana_file_id: addon.gamebanana_file_id,
            fingerprint: Some(addon.fingerprint.clone()),
        })
        .collect();
    let text = read_autoexec(game_root)?;
    let (autoexec, crosshair) = match &text {
        Some(text) => {
            let parts = split_autoexec(text);
            (Some(parts.commands), parts.crosshair)
        }
        None => (None, None),
    };
    Ok(CurrentSetup {
        slots,
        autoexec,
        crosshair,
    })
}

/// Save the mods that are on now as a new profile. Writes the workspace only.
///
/// Like Grimoire, a profile is the set of enabled mods; mods that are off are not recorded and
/// get turned off on apply. The crosshair is only saved when `include_crosshair` is set and the
/// autoexec has a crosshair block.
pub fn create_profile(
    game_root: &Path,
    workspace: &Path,
    name: &str,
    include_crosshair: bool,
) -> Result<Profile> {
    let name = clean_profile_name(name)?;
    let current = read_current_setup(game_root, workspace)?;
    let slug = unique_slug(workspace, &slugify(&name))?;
    let now = now_iso();
    let profile = Profile {
        schema: PROFILE_SCHEMA,
        name,
        slug,
        created_at: now.clone(),
        updated_at: now,
        slots: current.slots,
        crosshair: if include_crosshair {
            current.crosshair
        } else {
            None
        },
        autoexec: current.autoexec,
        author: None,
        extensions: Map::new(),
        isolated: false,
    };
    write_profile(workspace, &profile)?;
    Ok(profile)
}

/// Overwrite a profile's mods and autoexec with what is on now. Keeps its name and crosshair.
pub fn update_profile(game_root: &Path, workspace: &Path, slug: &str) -> Result<Profile> {
    let mut profile = read_profile(workspace, slug)?;
    let _ = snapshots::capture_from_profile(
        workspace,
        SnapshotTrigger::PreUpdate,
        Some(&format!("Before updating {}", profile.name)),
        &profile,
    );
    let current = read_current_setup(game_root, workspace)?;
    profile.schema = PROFILE_SCHEMA;
    profile.slots = current.slots;
    profile.autoexec = current.autoexec;
    profile.updated_at = now_iso();
    write_profile(workspace, &profile)?;
    Ok(profile)
}

/// Save under `name`, updating the profile with that slug when it exists. The CLI's `save`.
pub fn capture_profile(game_root: &Path, workspace: &Path, name: &str) -> Result<Profile> {
    let name = clean_profile_name(name)?;
    let slug = slugify(&name);
    validate_slug(&slug)?;
    if profile_path(workspace, &slug).is_file() {
        let mut profile = update_profile(game_root, workspace, &slug)?;
        if profile.name != name {
            profile.name = name;
            write_profile(workspace, &profile)?;
        }
        return Ok(profile);
    }
    create_profile(game_root, workspace, &name, false)
}

/// Give a profile a new display name. The file keeps its slug.
pub fn rename_profile(workspace: &Path, slug: &str, name: &str) -> Result<Profile> {
    let name = clean_profile_name(name)?;
    let mut profile = read_profile(workspace, slug)?;
    profile.name = name;
    profile.updated_at = now_iso();
    write_profile(workspace, &profile)?;
    Ok(profile)
}

/// Keep a profile's mods in its own `citadel/addons/<folder>` and mount that folder on apply.
/// Workspace only: the folder is filled and mounted by [`apply_profile`].
pub fn set_profile_isolated(workspace: &Path, slug: &str, isolated: bool) -> Result<Profile> {
    let mut profile = read_profile(workspace, slug)?;
    profile.isolated = isolated;
    profile.updated_at = now_iso();
    write_profile(workspace, &profile)?;
    Ok(profile)
}

/// Copy a profile under a new slug, named `<name> copy`.
pub fn duplicate_profile(workspace: &Path, slug: &str) -> Result<Profile> {
    let mut profile = read_profile(workspace, slug)?;
    let name: String = format!("{} copy", profile.name)
        .chars()
        .take(MAX_PROFILE_NAME_CHARS)
        .collect();
    profile.slug = unique_slug(workspace, &slugify(&name))?;
    profile.name = name;
    let now = now_iso();
    profile.created_at = now.clone();
    profile.updated_at = now;
    write_profile(workspace, &profile)?;
    Ok(profile)
}

/// Delete a saved profile. Clears the active marker when it pointed at this one.
pub fn delete_profile(workspace: &Path, slug: &str) -> Result<()> {
    validate_slug(slug)?;
    let path = profile_path(workspace, slug);
    fs::remove_file(&path).map_err(|error| {
        Error::msg(format!("Could not delete {}: {error}", path.display()))
    })?;
    if active_profile(workspace).as_deref() == Some(slug) {
        set_active_profile(workspace, None)?;
    }
    Ok(())
}

/// Forget the crosshair saved in a profile, leaving its mods and autoexec alone.
pub fn remove_profile_crosshair(workspace: &Path, slug: &str) -> Result<Profile> {
    let mut profile = read_profile(workspace, slug)?;
    profile.crosshair = None;
    profile.updated_at = now_iso();
    write_profile(workspace, &profile)?;
    Ok(profile)
}

/// The slug of the last applied profile, when it still exists.
pub fn active_profile(workspace: &Path) -> Option<String> {
    let text = fs::read_to_string(profile_dir(workspace).join(ACTIVE_FILE_NAME)).ok()?;
    let slug = text.trim().to_string();
    (validate_slug(&slug).is_ok() && profile_path(workspace, &slug).is_file()).then_some(slug)
}

/// Remember (or forget) the last applied profile. Workspace only.
pub fn set_active_profile(workspace: &Path, slug: Option<&str>) -> Result<()> {
    let path = profile_dir(workspace).join(ACTIVE_FILE_NAME);
    match slug {
        Some(slug) => {
            validate_slug(slug)?;
            write_atomic(&path, format!("{slug}\n").as_bytes())
        }
        None => match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

// ---------------------------------------------------------------------------
// Installed mod identity
// ---------------------------------------------------------------------------

/// One addon slot on disk with everything Sigil knows about where it came from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonIdentity {
    pub slot: u16,
    /// File name on disk, with `.disabled` when off.
    pub file: String,
    pub enabled: bool,
    pub size_bytes: u64,
    pub fingerprint: String,
    pub gamebanana_id: Option<u64>,
    pub gamebanana_file_id: Option<u64>,
    pub name: Option<String>,
}

/// A remembered source for an installed VPK, keyed by fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamebanana_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamebanana_file_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A cheap content fingerprint: SHA-256 over the size, the first 64 KiB and the last 64 KiB.
///
/// The VPK directory tree sits at the start of a directory file, so two different mods differ
/// there. Reading at most 128 KiB keeps a scan of every slot fast. Prefixed `q1:` so a full hash
/// can be told apart later.
pub fn fingerprint_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .map_err(|error| Error::msg(format!("Could not read {}: {error}", path.display())))?;
    let size = file.metadata()?.len();
    let mut hasher = Sha256::new();
    hasher.update(size.to_le_bytes());
    let mut buffer = vec![0u8; FINGERPRINT_WINDOW as usize];
    let head = read_window(&mut file, 0, size.min(FINGERPRINT_WINDOW), &mut buffer)?;
    hasher.update(&buffer[..head]);
    if size > FINGERPRINT_WINDOW {
        let start = size.saturating_sub(FINGERPRINT_WINDOW).max(FINGERPRINT_WINDOW);
        let tail = read_window(&mut file, start, size - start, &mut buffer)?;
        hasher.update(&buffer[..tail]);
    }
    Ok(format!("q1:{}", hex(&hasher.finalize())))
}

fn read_window(file: &mut File, start: u64, len: u64, buffer: &mut [u8]) -> Result<usize> {
    file.seek(SeekFrom::Start(start))?;
    let want = (len as usize).min(buffer.len());
    let mut read = 0;
    while read < want {
        let count = file.read(&mut buffer[read..want])?;
        if count == 0 {
            break;
        }
        read += count;
    }
    Ok(read)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sources_path(workspace: &Path) -> PathBuf {
    profile_dir(workspace).join(SOURCES_FILE_NAME)
}

/// Read the remembered sources. A missing or unreadable file is an empty list: it is a cache.
pub fn read_sources(workspace: &Path) -> Vec<SourceRecord> {
    fs::read_to_string(sources_path(workspace))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<SourceRecord>>(&text).ok())
        .unwrap_or_default()
}

/// Remember where an installed VPK came from. Replaces an older record with the same
/// fingerprint. Workspace only.
pub fn remember_source(workspace: &Path, record: SourceRecord) -> Result<()> {
    let mut records = read_sources(workspace);
    records.retain(|existing| existing.fingerprint != record.fingerprint);
    records.push(record);
    let mut text = serde_json::to_string_pretty(&records)?;
    text.push('\n');
    write_atomic(&sources_path(workspace), text.as_bytes())
}

/// Every addon slot with its fingerprint and, when known, its GameBanana identity.
///
/// Identity comes from [`read_sources`] (mods Sigil installed for a profile import) and from the
/// install journal (mods installed from Browse), matched by file name and size. Reads only.
pub fn installed_identities(game_root: &Path, workspace: &Path) -> Result<Vec<AddonIdentity>> {
    installed_identities_in(&addons_dir(game_root), workspace)
}

/// [`installed_identities`] for one folder: the main addons folder or a profile folder.
pub fn installed_identities_in(directory: &Path, workspace: &Path) -> Result<Vec<AddonIdentity>> {
    let sources = read_sources(workspace);
    let journal = crate::install::read_journal(workspace).unwrap_or_default();
    // The main folder and its extra folders (`sigil-addons2` and on), by global slot. A profile
    // folder has no extra folders, so this reads just that folder for it.
    let groups = crate::load_order::read_slot_files(directory)?;

    let mut identities = Vec::with_capacity(groups.len());
    for (slot, files) in groups {
        let Some(file) = files
            .iter()
            .find(|name| name.to_ascii_lowercase().ends_with("_dir.vpk")
                || name.to_ascii_lowercase().ends_with("_dir.vpk.disabled"))
            .cloned()
        else {
            continue;
        };
        let path = crate::addons::slot_folder(directory, slot).join(&file);
        let fingerprint = match fingerprint_file(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let size_bytes = path.metadata().map(|meta| meta.len()).unwrap_or(0);
        let disabled = file.to_ascii_lowercase().ends_with(DISABLED_SUFFIX);
        let mut identity = AddonIdentity {
            slot,
            file: file.clone(),
            enabled: !disabled,
            size_bytes,
            fingerprint: fingerprint.clone(),
            gamebanana_id: None,
            gamebanana_file_id: None,
            name: None,
        };

        if let Some(record) = sources.iter().find(|record| record.fingerprint == fingerprint) {
            identity.gamebanana_id = record.gamebanana_id;
            identity.gamebanana_file_id = record.gamebanana_file_id;
            identity.name = record.name.clone();
        } else {
            let enabled_name = enabled_file_name(&file);
            if let Some(entry) = journal.iter().rev().find(|entry| {
                enabled_file_name(&entry.file).eq_ignore_ascii_case(&enabled_name)
                    && entry.vpk_bytes == size_bytes
            }) {
                let (mod_id, file_id) = gamebanana_ids_from_source(&entry.source);
                let (label_mod, label_file) = gamebanana_ids_from_source(&entry.source_label);
                identity.gamebanana_id = mod_id.or(label_mod);
                identity.gamebanana_file_id = file_id.or(label_file).or_else(|| {
                    crate::install::load_plan(workspace, &entry.plan_id)
                        .ok()
                        .and_then(|plan| plan.gamebanana)
                        .map(|file| file.file_id)
                        .filter(|id| *id != 0)
                });
            }
        }

        identities.push(identity);
    }
    Ok(identities)
}

/// Read a GameBanana mod id or file id out of an install source string.
///
/// `715100`, `gamebanana:715100` and `https://gamebanana.com/mods/715100` name a mod;
/// `https://gamebanana.com/dl/1234567` names a file.
pub fn gamebanana_ids_from_source(source: &str) -> (Option<u64>, Option<u64>) {
    let text = source.trim();
    if !text.is_empty() && text.chars().all(|character| character.is_ascii_digit()) {
        return (text.parse().ok().filter(|id| *id != 0), None);
    }
    if let Some(rest) = text.strip_prefix("gamebanana:") {
        return (rest.trim().parse().ok().filter(|id| *id != 0), None);
    }
    let lower = text.to_ascii_lowercase();
    let Some(after_host) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return (None, None);
    };
    let (host, path) = after_host.split_once('/').unwrap_or((after_host, ""));
    if !matches!(host, "gamebanana.com" | "www.gamebanana.com") {
        return (None, None);
    }
    let number = |rest: &str| -> Option<u64> {
        rest.split(['/', '?', '#'])
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|id| *id != 0)
    };
    if let Some(rest) = path.strip_prefix("mods/") {
        return (number(rest), None);
    }
    if let Some(rest) = path.strip_prefix("dl/") {
        return (None, number(rest));
    }
    (None, None)
}

// ---------------------------------------------------------------------------
// Resolve and apply
// ---------------------------------------------------------------------------

/// How a recorded mod was found on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryState {
    /// Found, by GameBanana ids, fingerprint or slot.
    Ready { slot: u16, enabled_now: bool },
    /// Not installed.
    Missing,
    /// The recorded slot now holds a different mod, so it is left alone.
    SlotReused { slot: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPlan {
    /// Index into the profile's `slots`.
    pub index: usize,
    pub label: String,
    pub wants_enabled: bool,
    pub state: EntryState,
}

/// One rename an apply would make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotChange {
    pub slot: u16,
    pub from: String,
    pub to: String,
    pub label: String,
}

/// What applying a profile would do, computed without writing anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileApplyPlan {
    pub entries: Vec<EntryPlan>,
    pub turn_on: Vec<SlotChange>,
    pub turn_off: Vec<SlotChange>,
    /// The new `autoexec.cfg` text when it would change.
    pub autoexec: Option<String>,
    /// A reason the apply must refuse, for example a slot with both names on disk.
    pub blocked: Option<String>,
}

impl ProfileApplyPlan {
    pub fn missing(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.wants_enabled && !matches!(entry.state, EntryState::Ready { .. }))
            .count()
    }

    /// True when the install already matches the profile.
    pub fn in_sync(&self) -> bool {
        self.turn_on.is_empty() && self.turn_off.is_empty() && self.autoexec.is_none()
    }
}

/// Match each recorded mod to an installed addon, Grimoire's resolver order: GameBanana mod and
/// file ids, then the same GameBanana mod with another file, then the fingerprint, then the slot.
/// A slot fallback is refused when both sides know their identity and it differs, so an apply
/// never turns on a mod that only happens to sit in the old slot. Each addon is claimed once.
pub fn resolve_entries(slots: &[ProfileSlot], installed: &[AddonIdentity]) -> Vec<EntryState> {
    let mut claimed = vec![false; installed.len()];
    let mut states = vec![EntryState::Missing; slots.len()];

    let claim = |states: &mut Vec<EntryState>,
                     claimed: &mut Vec<bool>,
                     index: usize,
                     pick: &dyn Fn(&AddonIdentity) -> bool| {
        if !matches!(states[index], EntryState::Missing) {
            return;
        }
        if let Some(position) = installed
            .iter()
            .enumerate()
            .position(|(at, addon)| !claimed[at] && pick(addon))
        {
            claimed[position] = true;
            states[index] = EntryState::Ready {
                slot: installed[position].slot,
                enabled_now: installed[position].enabled,
            };
        }
    };

    for (index, entry) in slots.iter().enumerate() {
        if let (Some(mod_id), Some(file_id)) = (entry.gamebanana_id, entry.gamebanana_file_id) {
            claim(&mut states, &mut claimed, index, &|addon| {
                addon.gamebanana_id == Some(mod_id) && addon.gamebanana_file_id == Some(file_id)
            });
        }
    }
    for (index, entry) in slots.iter().enumerate() {
        if let Some(fingerprint) = &entry.fingerprint {
            claim(&mut states, &mut claimed, index, &|addon| &addon.fingerprint == fingerprint);
        }
    }
    for (index, entry) in slots.iter().enumerate() {
        if let Some(mod_id) = entry.gamebanana_id {
            claim(&mut states, &mut claimed, index, &|addon| addon.gamebanana_id == Some(mod_id));
        }
    }
    for (index, entry) in slots.iter().enumerate() {
        if !matches!(states[index], EntryState::Missing) {
            continue;
        }
        let Some(position) = installed
            .iter()
            .enumerate()
            .position(|(at, addon)| !claimed[at] && addon.slot == entry.slot)
        else {
            continue;
        };
        let addon = &installed[position];
        let fingerprint_differs = entry
            .fingerprint
            .as_ref()
            .is_some_and(|fingerprint| fingerprint != &addon.fingerprint);
        let gamebanana_differs = match (entry.gamebanana_id, addon.gamebanana_id) {
            (Some(left), Some(right)) => left != right,
            _ => false,
        };
        if fingerprint_differs || gamebanana_differs {
            states[index] = EntryState::SlotReused { slot: addon.slot };
        } else {
            claimed[position] = true;
            states[index] = EntryState::Ready {
                slot: addon.slot,
                enabled_now: addon.enabled,
            };
        }
    }
    states
}

/// Work out what applying `profile` would change. Reads only.
pub fn plan_profile_apply(
    game_root: &Path,
    workspace: &Path,
    profile: &Profile,
) -> Result<ProfileApplyPlan> {
    let installed = installed_identities(game_root, workspace)?;
    let directory = addons_dir(game_root);
    let states = resolve_entries(&profile.slots, &installed);

    let mut wanted: BTreeMap<u16, (bool, String)> = BTreeMap::new();
    let mut entries = Vec::with_capacity(profile.slots.len());
    for (index, (entry, state)) in profile.slots.iter().zip(states).enumerate() {
        let label = installed_label(entry, &state, &installed);
        if let EntryState::Ready { slot, .. } = state {
            wanted.insert(slot, (entry.enabled, label.clone()));
        }
        entries.push(EntryPlan {
            index,
            label,
            wants_enabled: entry.enabled,
            state,
        });
    }

    let mut blocked = None;
    let mut turn_on = Vec::new();
    let mut turn_off = Vec::new();
    for addon in &installed {
        let enabled_name = enabled_file_name(&addon.file);
        let disabled_name = format!("{enabled_name}{DISABLED_SUFFIX}");
        let folder = crate::addons::slot_folder(&directory, addon.slot);
        if folder.join(&enabled_name).is_file() && folder.join(&disabled_name).is_file() {
            blocked = Some(format!(
                "Slot {:02} has both {enabled_name} and {disabled_name}. Remove one first.",
                addon.slot
            ));
        }
        let (want, label) = wanted
            .get(&addon.slot)
            .cloned()
            .unwrap_or_else(|| (false, addon_label(addon)));
        if want && !addon.enabled {
            turn_on.push(SlotChange {
                slot: addon.slot,
                from: disabled_name,
                to: enabled_name,
                label,
            });
        } else if !want && addon.enabled {
            turn_off.push(SlotChange {
                slot: addon.slot,
                from: enabled_name,
                to: disabled_name,
                label,
            });
        }
    }

    let current = read_autoexec(game_root)?;
    let autoexec = compose_autoexec(current.as_deref(), profile)
        .filter(|text| Some(normalize(text)) != current.as_deref().map(normalize));

    Ok(ProfileApplyPlan {
        entries,
        turn_on,
        turn_off,
        autoexec,
        blocked,
    })
}

/// Apply a profile to the game install. Refuses without `approve` and while Deadlock runs.
/// Game install write.
///
/// An ordinary profile turns its mods on and every other enabled mod off in the shared addons
/// folder, then writes `autoexec.cfg` when the profile carries autoexec lines or a crosshair
/// (the old file is backed up beside it). An isolated profile copies its mods into
/// `citadel/addons/<its folder>` and points the marked `gameinfo.gi` block at that folder
/// instead. A recovery point of the setup on now is taken first when one can be saved.
///
/// Every rename is checked before the first one runs, so a refusal leaves the folder as it was.
/// Mods the profile names that are not installed become warnings. Records the profile as active.
pub fn apply_profile(
    game_root: &Path,
    workspace: &Path,
    slug: &str,
    approve: bool,
) -> Result<ProfileApplyResult> {
    apply_profile_with(game_root, workspace, slug, approve, game_running())
}

/// What the app leaves out of a profile apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyChoices {
    /// Leave the profile's crosshair out of autoexec.cfg: the Crosshair page is off, so
    /// nothing in Sigil would show or clear it while the game runs it at every start (Grimoire
    /// #504). The current crosshair block, if any, stays as it is.
    pub skip_crosshair: bool,
}

/// The warning a skipped crosshair adds to the apply result.
pub const CROSSHAIR_SKIPPED: &str = "The profile's crosshair was left out because the Crosshair page is off.";

/// [`apply_profile`] with the app's choices. Game install write.
pub fn apply_profile_choosing(
    game_root: &Path,
    workspace: &Path,
    slug: &str,
    approve: bool,
    choices: ApplyChoices,
) -> Result<ProfileApplyResult> {
    apply_profile_inner(game_root, workspace, slug, approve, game_running(), choices)
}

/// [`apply_profile`] with the process check supplied, so tests and the app's poll can say.
pub fn apply_profile_with(
    game_root: &Path,
    workspace: &Path,
    slug: &str,
    approve: bool,
    running: bool,
) -> Result<ProfileApplyResult> {
    apply_profile_inner(game_root, workspace, slug, approve, running, ApplyChoices::default())
}

fn apply_profile_inner(
    game_root: &Path,
    workspace: &Path,
    slug: &str,
    approve: bool,
    running: bool,
    choices: ApplyChoices,
) -> Result<ProfileApplyResult> {
    let mut profile = read_profile(workspace, slug)?;
    let skipped_crosshair = choices.skip_crosshair && profile.crosshair.take().is_some();
    let addons = addons_dir(game_root);

    if !approve {
        return Err(Error::msg(format!(
            "Applying profile {} to {} was not approved. Nothing was written.",
            profile.name,
            addons.display()
        )));
    }
    if running {
        return Err(Error::msg(
            "Deadlock is running. Close it, then apply the profile. Nothing was changed.",
        ));
    }
    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was written.",
            addons.display()
        )));
    }

    let previous_active = active_profile(workspace);
    let previous_folder = mounted_folder(game_root);
    let snapshot = capture_snapshot(
        game_root,
        workspace,
        SnapshotTrigger::PreApplyProfile,
        Some(&format!("Before applying {}", profile.name)),
    )
    .ok()
    .map(|point| point.id);
    let mut result = if profile.isolated {
        apply_isolated(game_root, workspace, &profile)?
    } else {
        apply_shared(game_root, workspace, &profile)?
    };
    result.previous_active = previous_active;
    result.previous_folder = previous_folder;
    result.snapshot = snapshot;
    if skipped_crosshair {
        result.warnings.push(CROSSHAIR_SKIPPED.to_string());
    }
    set_active_profile(workspace, Some(&profile.slug))?;
    Ok(result)
}

/// The shared addons folder: rename slot files so only the profile's mods are on.
fn apply_shared(game_root: &Path, workspace: &Path, profile: &Profile) -> Result<ProfileApplyResult> {
    let addons = addons_dir(game_root);
    let plan = plan_profile_apply(game_root, workspace, profile)?;
    if let Some(reason) = &plan.blocked {
        return Err(Error::msg(format!("{reason} Nothing was written.")));
    }

    let mut changed = Vec::new();
    let mut changes = Vec::new();
    // Leave an isolated profile's folder first, so the shared folder is what the game reads.
    if mounted_folder(game_root).is_some() {
        isolation::mount_gameinfo(game_root, None, true)?;
    }
    // Off before on, as Grimoire does, so a swap never has both sets loaded at once.
    for change in plan.turn_off.iter().chain(plan.turn_on.iter()) {
        let folder = crate::addons::slot_folder(&addons, change.slot);
        let from = folder.join(&change.from);
        let to = folder.join(&change.to);
        if to.exists() {
            return Err(Error::msg(format!(
                "Slot {:02}: {} already exists. Stopped after {} change(s).",
                change.slot,
                change.to,
                changed.len()
            )));
        }
        fs::rename(&from, &to).map_err(|error| {
            Error::msg(format!(
                "Slot {:02}: rename {} to {} failed: {error}. Stopped after {} change(s).",
                change.slot,
                change.from,
                change.to,
                changed.len()
            ))
        })?;
        changed.push(format!("slot {:02}: {} -> {}", change.slot, change.from, change.to));
        changes.push(change.clone());
    }

    let mut warnings = missing_warnings(&plan);
    let (autoexec_written, autoexec_backup) = write_profile_autoexec(game_root, profile, &mut warnings)?;

    Ok(ProfileApplyResult {
        changed,
        warnings,
        autoexec_written,
        autoexec_backup,
        changes,
        previous_active: None,
        folder: None,
        previous_folder: None,
        copied: 0,
        snapshot: None,
    })
}

/// An isolated profile: copy its mods into its own folder and mount that folder.
fn apply_isolated(
    game_root: &Path,
    workspace: &Path,
    profile: &Profile,
) -> Result<ProfileApplyResult> {
    let sync = sync_folder(game_root, workspace, profile)?;
    let mut warnings: Vec<String> = sync
        .missing
        .iter()
        .map(|label| format!("{label} is not installed."))
        .collect();
    let (autoexec_written, autoexec_backup) =
        write_profile_autoexec(game_root, profile, &mut warnings)?;
    isolation::mount_gameinfo(game_root, Some(&sync.folder), true)?;
    Ok(ProfileApplyResult {
        changed: sync.copied.clone(),
        warnings,
        autoexec_written,
        autoexec_backup,
        changes: Vec::new(),
        previous_active: None,
        folder: Some(sync.folder),
        previous_folder: None,
        copied: sync.copied.len(),
        snapshot: None,
    })
}

fn missing_warnings(plan: &ProfileApplyPlan) -> Vec<String> {
    plan.entries
        .iter()
        .filter(|entry| entry.wants_enabled)
        .filter_map(|entry| match entry.state {
            EntryState::Missing => Some(format!("{} is not installed.", entry.label)),
            EntryState::SlotReused { slot } => Some(format!(
                "{}: slot {slot:02} now holds a different mod, so it was left alone.",
                entry.label
            )),
            EntryState::Ready { .. } => None,
        })
        .collect()
}

fn write_profile_autoexec(
    game_root: &Path,
    profile: &Profile,
    warnings: &mut Vec<String>,
) -> Result<(bool, Option<PathBuf>)> {
    let current = read_autoexec(game_root)?;
    let Some(text) = compose_autoexec(current.as_deref(), profile) else {
        return Ok((false, None));
    };
    if Some(normalize(&text)) == current.as_deref().map(normalize) {
        return Ok((false, None));
    }
    match write_autoexec(game_root, &text, true) {
        Ok(write) => Ok((true, write.backup)),
        Err(error) => {
            warnings.push(format!("The autoexec was not written: {error}"));
            Ok((false, None))
        }
    }
}

/// Undo an apply: put the slot files back, restore `autoexec.cfg` from its backup, mount the
/// previous profile folder and make the previous profile active again. GAME INSTALL WRITE.
pub fn undo_profile_apply(
    game_root: &Path,
    workspace: &Path,
    result: &ProfileApplyResult,
    approve: bool,
) -> Result<UndoReport> {
    if !approve {
        return Err(Error::msg(
            "Undoing the profile switch was not approved. Nothing was written.",
        ));
    }
    let addons = addons_dir(game_root);
    let mut report = UndoReport::default();
    for change in result.changes.iter().rev() {
        let folder = crate::addons::slot_folder(&addons, change.slot);
        let from = folder.join(&change.to);
        let to = folder.join(&change.from);
        if to.exists() {
            report.skipped.push(format!("slot {:02}: {} is in the way", change.slot, change.from));
            continue;
        }
        if !from.exists() {
            report
                .skipped
                .push(format!("slot {:02}: {} is gone", change.slot, change.to));
            continue;
        }
        fs::rename(&from, &to)?;
        report.reverted += 1;
    }
    if result.autoexec_written {
        let path = crate::autoexec::autoexec_path(game_root);
        match &result.autoexec_backup {
            Some(backup) if backup.is_file() => {
                let partial = path.with_file_name(format!(
                    ".sigil-undo-autoexec-{}",
                    std::process::id()
                ));
                fs::copy(backup, &partial)?;
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                fs::rename(&partial, &path)?;
                report.autoexec_restored = true;
            }
            _ => {
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                report.autoexec_restored = true;
            }
        }
    }
    if result.folder.is_some() || result.previous_folder.is_some() {
        let mount = isolation::mount_gameinfo(game_root, result.previous_folder.as_deref(), true)?;
        report.mount = mount.folder;
    }
    set_active_profile(workspace, result.previous_active.as_deref())?;
    Ok(report)
}

/// What [`undo_profile_apply`] put back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    pub reverted: usize,
    pub skipped: Vec<String>,
    pub autoexec_restored: bool,
    pub mount: Option<String>,
}

/// Work out what restoring a snapshot would change. Reads only.
pub fn plan_snapshot_restore(
    game_root: &Path,
    workspace: &Path,
    id: &str,
) -> Result<ProfileApplyPlan> {
    let snapshot = read_snapshot(workspace, id)?;
    plan_profile_apply(game_root, workspace, &snapshot.as_profile())
}

/// Put a recovery point back. GAME INSTALL WRITE: same rules as [`apply_profile`], plus the
/// active profile marker is cleared because the install no longer matches a saved profile.
pub fn restore_snapshot(
    game_root: &Path,
    workspace: &Path,
    id: &str,
    approve: bool,
) -> Result<ProfileApplyResult> {
    if !approve {
        return Err(Error::msg(
            "Restoring the snapshot was not approved. Nothing was written.",
        ));
    }
    if game_running() {
        return Err(Error::msg(
            "Deadlock is running. Close it, then restore the snapshot. Nothing was changed.",
        ));
    }
    let snapshot = read_snapshot(workspace, id)?;
    let profile = snapshot.as_profile();
    let previous_active = active_profile(workspace);
    let previous_folder = mounted_folder(game_root);
    let mut result = apply_shared(game_root, workspace, &profile)?;
    result.previous_active = previous_active;
    result.previous_folder = previous_folder;
    result.snapshot = Some(snapshot.id.clone());
    set_active_profile(workspace, None)?;
    Ok(result)
}

/// Is Deadlock running? Used to refuse a profile switch that would edit files mid-session.
pub fn game_running() -> bool {
    // One process check for the whole app: it skips a Deadlock stuck while closing.
    crate::launch::game_running()
}

fn installed_label(entry: &ProfileSlot, state: &EntryState, installed: &[AddonIdentity]) -> String {
    if entry.name.is_some() {
        return entry.label();
    }
    if let EntryState::Ready { slot, .. } = state
        && let Some(addon) = installed.iter().find(|addon| addon.slot == *slot)
        && addon.name.is_some()
    {
        return addon_label(addon);
    }
    entry.label()
}

fn addon_label(addon: &AddonIdentity) -> String {
    if let Some(name) = addon.name.as_deref().filter(|name| !name.trim().is_empty()) {
        return name.to_string();
    }
    if let Some(id) = addon.gamebanana_id {
        return format!("GameBanana mod {id}");
    }
    format!("Slot {:02}", addon.slot)
}

// ---------------------------------------------------------------------------
// Autoexec
// ---------------------------------------------------------------------------

/// An autoexec file split into its own lines and Sigil's crosshair block.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoexecParts {
    /// Non-empty lines outside the crosshair block, trimmed.
    pub commands: Vec<String>,
    /// The block's text, markers included, verbatim.
    pub block: Option<String>,
    pub crosshair: Option<Crosshair>,
}

pub fn split_autoexec(text: &str) -> AutoexecParts {
    let (outside, block) = match text.find(AUTOEXEC_BEGIN) {
        Some(begin) => match text[begin..].find(AUTOEXEC_END) {
            Some(relative) => {
                let end = begin + relative + AUTOEXEC_END.len();
                (
                    format!("{}\n{}", &text[..begin], &text[end..]),
                    Some(text[begin..end].to_string()),
                )
            }
            None => (text.to_string(), None),
        },
        None => (text.to_string(), None),
    };
    let commands = outside
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let crosshair = block.as_deref().and_then(crosshair_from_block);
    AutoexecParts {
        commands,
        block,
        crosshair,
    }
}

/// Read the crosshair out of a managed block of `citadel_crosshair_* value` lines.
pub fn crosshair_from_block(block: &str) -> Option<Crosshair> {
    let mut fields = Map::new();
    for line in block.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(field) = key.strip_prefix("citadel_crosshair_") else {
            continue;
        };
        let value = value.trim_matches('"');
        let json = match value {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            other => match other.parse::<f64>() {
                Ok(number) => serde_json::Number::from_f64(number).map(Value::Number)?,
                Err(_) => continue,
            },
        };
        fields.insert(snake_to_camel(field), json);
    }
    if fields.is_empty() {
        return None;
    }
    crosshair_from_value(&Value::Object(fields))
}

fn snake_to_camel(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut upper = false;
    for character in value.chars() {
        if character == '_' {
            upper = true;
        } else if upper {
            output.extend(character.to_uppercase());
            upper = false;
        } else {
            output.push(character);
        }
    }
    output
}

/// Read a crosshair from camelCase settings, filling missing fields from the game defaults.
///
/// Accepts the older ten-field shape too: `pipBorder: true` becomes a one point pip outline.
/// Values out of range are refused rather than clamped.
pub fn crosshair_from_value(value: &Value) -> Option<Crosshair> {
    let object = value.as_object()?;
    let mut crosshair = Crosshair::game_defaults();
    let int = |key: &str, fallback: i32| -> i32 {
        object
            .get(key)
            .and_then(Value::as_f64)
            .map(|value| value.round() as i32)
            .unwrap_or(fallback)
    };
    let float = |key: &str, fallback: f64| -> f64 {
        object.get(key).and_then(Value::as_f64).unwrap_or(fallback)
    };
    let boolean = |key: &str, fallback: bool| -> bool {
        match object.get(key) {
            Some(Value::Bool(value)) => *value,
            Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
            _ => fallback,
        }
    };
    if let Some(Value::Bool(border)) = object.get("pipBorder")
        && !object.contains_key("pipOutlineBorder")
    {
        crosshair.pip_outline_border = i32::from(*border);
    }
    crosshair.pip_gap = int("pipGap", crosshair.pip_gap);
    crosshair.pip_gap_static = boolean("pipGapStatic", crosshair.pip_gap_static);
    crosshair.pip_height = int("pipHeight", crosshair.pip_height);
    crosshair.pip_width = int("pipWidth", crosshair.pip_width);
    crosshair.pip_opacity = float("pipOpacity", crosshair.pip_opacity);
    crosshair.pip_outline_border = int("pipOutlineBorder", crosshair.pip_outline_border);
    crosshair.pip_outline_gap = int("pipOutlineGap", crosshair.pip_outline_gap);
    crosshair.pip_outline_opacity = float("pipOutlineOpacity", crosshair.pip_outline_opacity);
    crosshair.dot_opacity = float("dotOpacity", crosshair.dot_opacity);
    crosshair.dot_size = int("dotSize", crosshair.dot_size);
    crosshair.dot_outline_border = int("dotOutlineBorder", crosshair.dot_outline_border);
    crosshair.dot_outline_gap = int("dotOutlineGap", crosshair.dot_outline_gap);
    crosshair.dot_outline_opacity = float("dotOutlineOpacity", crosshair.dot_outline_opacity);
    crosshair.color_r = int("colorR", crosshair.color_r);
    crosshair.color_g = int("colorG", crosshair.color_g);
    crosshair.color_b = int("colorB", crosshair.color_b);
    crosshair.outline_color_r = int("outlineColorR", crosshair.outline_color_r);
    crosshair.outline_color_g = int("outlineColorG", crosshair.outline_color_g);
    crosshair.outline_color_b = int("outlineColorB", crosshair.outline_color_b);
    crosshair.disable_hero_specific_crosshairs = boolean(
        "disableHeroSpecificCrosshairs",
        crosshair.disable_hero_specific_crosshairs,
    );
    crosshair.validate().ok()?;
    Some(crosshair)
}

/// The autoexec text a profile wants, or None when the profile leaves autoexec alone.
///
/// The profile's lines replace the current ones when it has any record of them; its crosshair
/// replaces the managed block when it has one, otherwise the current block is kept verbatim.
pub fn compose_autoexec(current: Option<&str>, profile: &Profile) -> Option<String> {
    if profile.autoexec.is_none() && profile.crosshair.is_none() {
        return None;
    }
    let parts = split_autoexec(current.unwrap_or(""));
    let commands = profile.autoexec.clone().unwrap_or(parts.commands);
    let block = match &profile.crosshair {
        Some(crosshair) => Some(format!(
            "{AUTOEXEC_BEGIN}\n{}\n{AUTOEXEC_END}",
            crosshair.command_text()
        )),
        None => parts.block,
    };
    let mut text = String::new();
    for line in commands {
        text.push_str(&line);
        text.push('\n');
    }
    if let Some(block) = block {
        text.push_str(&block);
        text.push('\n');
    }
    Some(text)
}

fn normalize(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Portable profiles (`.modprofile.json` and `mp1:` share codes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortableGame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_banana_game_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortableExportedBy {
    pub tool: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortableMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortableHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nsfw: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableMod {
    pub source: String,
    #[serde(rename = "ref")]
    pub reference: Value,
    pub enabled: bool,
    pub priority: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<PortableHint>,
}

impl PortableMod {
    /// `(submissionId, fileId)` for a GameBanana entry.
    pub fn gamebanana_ref(&self) -> Option<(u64, u64)> {
        if self.source != "gamebanana" {
            return None;
        }
        let submission = self.reference.get("submissionId")?.as_u64()?;
        let file = self.reference.get("fileId")?.as_u64()?;
        Some((submission, file))
    }

    pub fn label(&self) -> String {
        if let Some(name) = self
            .hint
            .as_ref()
            .and_then(|hint| hint.name.as_deref())
            .filter(|name| !name.trim().is_empty())
        {
            return name.to_string();
        }
        match self.gamebanana_ref() {
            Some((submission, _)) => format!("GameBanana mod {submission}"),
            None => format!("{} mod", self.source),
        }
    }
}

/// A portable profile document, format `mod-profile`, schema 1.x.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortableProfile {
    pub format: String,
    pub schema_version: String,
    #[serde(default)]
    pub game: PortableGame,
    /// A pack (`crate::packs`) leaves these out: they are not needed to install anything.
    #[serde(default)]
    pub exported_at: String,
    #[serde(default)]
    pub exported_by: PortableExportedBy,
    pub profile: PortableMeta,
    pub mods: Vec<PortableMod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

impl PortableProfile {
    /// The crosshair carried in the extensions: this app's namespace first, then the one the
    /// format's reference implementation writes, which uses the same settings shape.
    pub fn crosshair(&self) -> Option<Crosshair> {
        let extensions = self.extensions.as_ref()?;
        [EXTENSION_KEY, FOREIGN_EXTENSION_KEY]
            .iter()
            .find_map(|key| extensions.get(*key)?.get("crosshair"))
            .and_then(crosshair_from_value)
    }

    /// The autoexec lines carried in the extensions, cleaned of control characters.
    pub fn autoexec_commands(&self) -> Vec<String> {
        let Some(extensions) = self.extensions.as_ref() else {
            return Vec::new();
        };
        [EXTENSION_KEY, FOREIGN_EXTENSION_KEY]
            .iter()
            .find_map(|key| extensions.get(*key)?.get("autoexecCommands")?.as_array())
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|line| plain_text(line, 512))
                    .filter(|line| !line.is_empty())
                    .take(MAX_AUTOEXEC_LINES)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A built export: the document, its JSON, its share code and the mods left out.
#[derive(Debug, Clone, PartialEq)]
pub struct PortableExport {
    pub profile: PortableProfile,
    pub json: String,
    pub share_code: String,
    /// Mods with no GameBanana identity: a share would point at nothing.
    pub skipped: Vec<String>,
}

/// Build a portable document from a saved profile.
///
/// Only mods with a GameBanana mod and file id are included; local mods are listed in
/// `skipped`. An entry without ids borrows them from the installed addon it resolves to, so a
/// mod installed from Browse after the profile was saved still shares. Extension namespaces the
/// profile was imported with are kept; this app's own namespace is rewritten.
pub fn export_portable(
    profile: &Profile,
    installed: &[AddonIdentity],
    version: &str,
) -> Result<PortableExport> {
    let states = resolve_entries(&profile.slots, installed);
    let mut mods = Vec::new();
    let mut skipped = Vec::new();

    let mut order: Vec<usize> = (0..profile.slots.len()).collect();
    order.sort_by_key(|index| profile.slots[*index].slot);

    for index in order {
        let entry = &profile.slots[index];
        let addon = match states[index] {
            EntryState::Ready { slot, .. } => installed.iter().find(|addon| addon.slot == slot),
            _ => None,
        };
        let mod_id = entry
            .gamebanana_id
            .or_else(|| addon.and_then(|addon| addon.gamebanana_id));
        let file_id = entry
            .gamebanana_file_id
            .or_else(|| addon.and_then(|addon| addon.gamebanana_file_id));
        let name = entry
            .name
            .clone()
            .or_else(|| addon.and_then(|addon| addon.name.clone()));
        let (Some(mod_id), Some(file_id)) = (mod_id, file_id) else {
            skipped.push(name.unwrap_or_else(|| entry.label()));
            continue;
        };
        mods.push(PortableMod {
            source: "gamebanana".to_string(),
            reference: serde_json::json!({
                "submissionId": mod_id,
                "fileId": file_id,
                "section": "Mod",
            }),
            enabled: entry.enabled,
            priority: (mods.len() + 1) as f64,
            hint: Some(PortableHint {
                name,
                ..PortableHint::default()
            }),
        });
    }

    let mut extensions = profile.extensions.clone();
    extensions.remove(EXTENSION_KEY);
    let mut own = Map::new();
    if let Some(crosshair) = &profile.crosshair {
        own.insert("crosshair".to_string(), serde_json::to_value(crosshair)?);
    }
    if let Some(lines) = profile.autoexec.as_ref().filter(|lines| !lines.is_empty()) {
        own.insert("autoexecCommands".to_string(), serde_json::to_value(lines)?);
    }
    if !own.is_empty() {
        extensions.insert(EXTENSION_KEY.to_string(), Value::Object(own));
    }

    let portable = PortableProfile {
        format: PORTABLE_FORMAT.to_string(),
        schema_version: PORTABLE_SCHEMA_VERSION.to_string(),
        game: PortableGame {
            steam_app_id: Some(DEADLOCK_STEAM_APP_ID),
            game_banana_game_id: Some(DEADLOCK_GAMEBANANA_GAME_ID),
            name: Some("Deadlock".to_string()),
        },
        exported_at: now_iso(),
        exported_by: PortableExportedBy {
            tool: "sigil".to_string(),
            version: version.to_string(),
        },
        profile: PortableMeta {
            name: profile.name.clone(),
            description: None,
            author: profile.author.clone(),
        },
        mods,
        extensions: (!extensions.is_empty()).then_some(extensions),
    };
    let json = serde_json::to_string_pretty(&portable)?;
    let share_code = encode_share_code(&json)?;
    Ok(PortableExport {
        profile: portable,
        json,
        share_code,
        skipped,
    })
}

/// `mp1:` plus base64url (no padding) of gzip of the JSON.
pub fn encode_share_code(json: &str) -> Result<String> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(json.as_bytes())?;
    let compressed = encoder.finish()?;
    Ok(format!(
        "{SHARE_CODE_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(compressed)
    ))
}

/// Decode a share code to its JSON. Whitespace inside the code is ignored, so a code a chat app
/// wrapped still reads. The inflated size is capped at [`MAX_INFLATED_SHARE_CODE_BYTES`].
pub fn decode_share_code(code: &str) -> Result<String> {
    let body = code
        .trim()
        .strip_prefix(SHARE_CODE_PREFIX)
        .ok_or_else(|| Error::msg(format!("A share code starts with {SHARE_CODE_PREFIX}")))?;
    let body: String = body
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let body = body.trim_end_matches('=');
    let compressed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|_| Error::msg("This share code is damaged. Copy it again."))?;
    let decoder = flate2::read::GzDecoder::new(compressed.as_slice());
    let mut inflated = Vec::new();
    decoder
        .take(MAX_INFLATED_SHARE_CODE_BYTES as u64 + 1)
        .read_to_end(&mut inflated)
        .map_err(|_| Error::msg("This share code is damaged. Copy it again."))?;
    if inflated.len() > MAX_INFLATED_SHARE_CODE_BYTES {
        return Err(Error::msg(
            "This share code is too large. Share the profile as a file instead.",
        ));
    }
    String::from_utf8(inflated).map_err(|_| Error::msg("This share code is damaged. Copy it again."))
}

/// Read a portable profile from a share code, JSON text, or a path to a `.modprofile.json` file.
pub fn read_portable_input(input: &str) -> Result<PortableProfile> {
    let trimmed = input.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err(Error::msg("Paste a share code or pick a profile file first."));
    }
    let json = if trimmed.starts_with(SHARE_CODE_PREFIX) {
        decode_share_code(trimmed)?
    } else if trimmed.starts_with('{') {
        trimmed.to_string()
    } else {
        let path = Path::new(trimmed);
        if !path.is_file() {
            return Err(Error::msg(
                "That is not a share code, profile JSON or a profile file.",
            ));
        }
        read_portable_file_text(path)?
    };
    parse_portable(&json)
}

/// Read a profile file, refusing anything over [`MAX_PORTABLE_FILE_BYTES`].
pub fn read_portable_file_text(path: &Path) -> Result<String> {
    let size = fs::metadata(path)?.len();
    if size > MAX_PORTABLE_FILE_BYTES {
        return Err(Error::msg(format!(
            "{} is too large to be a profile.",
            path.display()
        )));
    }
    fs::read_to_string(path)
        .map_err(|error| Error::msg(format!("Could not read {}: {error}", path.display())))
}

/// Parse and validate a portable profile. The whole document is untrusted until this passes:
/// format and major version, game, names, every mod entry. Hint strings are reduced to plain
/// text, and thumbnail URLs are dropped unless they point at GameBanana's image host.
pub fn parse_portable(json: &str) -> Result<PortableProfile> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("This is not valid profile JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| Error::msg("This is not a profile."))?;
    if object.get("format").and_then(Value::as_str) != Some(PORTABLE_FORMAT) {
        return Err(Error::msg("This is not a mod profile."));
    }
    let version = object
        .get("schemaVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("The profile has no schema version."))?;
    if version.split('.').next() != Some("1") {
        return Err(Error::msg(format!(
            "This profile uses version {version}, which this build cannot read."
        )));
    }
    for (index, entry) in object
        .get("mods")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("The profile has no mod list."))?
        .iter()
        .enumerate()
    {
        let position = index + 1;
        if entry.get("source").and_then(Value::as_str).is_none()
            || !entry.get("ref").is_some_and(Value::is_object)
            || entry.get("enabled").and_then(Value::as_bool).is_none()
            || entry.get("priority").and_then(Value::as_f64).is_none()
        {
            return Err(Error::msg(format!("Mod {position} in the profile is incomplete.")));
        }
        if entry.get("source").and_then(Value::as_str) == Some("gamebanana") {
            let reference = &entry["ref"];
            if reference.get("submissionId").and_then(Value::as_u64).is_none()
                || reference.get("fileId").and_then(Value::as_u64).is_none()
            {
                return Err(Error::msg(format!(
                    "Mod {position} in the profile has no GameBanana ids."
                )));
            }
        }
    }

    let mut portable: PortableProfile = serde_json::from_value(value)
        .map_err(|error| Error::msg(format!("This profile is malformed: {error}")))?;

    if portable
        .game
        .steam_app_id
        .is_some_and(|id| id != DEADLOCK_STEAM_APP_ID)
        || portable
            .game
            .game_banana_game_id
            .is_some_and(|id| id != DEADLOCK_GAMEBANANA_GAME_ID)
    {
        return Err(Error::msg("This profile is for a different game."));
    }

    portable.profile.name = plain_text(&portable.profile.name, MAX_PROFILE_NAME_CHARS);
    if portable.profile.name.is_empty() {
        return Err(Error::msg("The profile has no name."));
    }
    portable.profile.description = portable
        .profile
        .description
        .as_deref()
        .map(|text| plain_text(text, 1024))
        .filter(|text| !text.is_empty());
    portable.profile.author = portable
        .profile
        .author
        .as_deref()
        .map(|text| plain_text(text, MAX_PROFILE_NAME_CHARS))
        .filter(|text| !text.is_empty());

    for entry in &mut portable.mods {
        if let Some(hint) = &mut entry.hint {
            let clean = |value: &Option<String>| {
                value
                    .as_deref()
                    .map(|text| plain_text(text, MAX_HINT_CHARS))
                    .filter(|text| !text.is_empty())
            };
            hint.name = clean(&hint.name);
            hint.category = clean(&hint.category);
            hint.file_label = clean(&hint.file_label);
            hint.original_file_name = clean(&hint.original_file_name);
            if !hint
                .thumbnail_url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://images.gamebanana.com/"))
            {
                hint.thumbnail_url = None;
            }
        }
    }
    Ok(portable)
}

/// Collapse a string to one line of plain text without control characters.
fn plain_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .map(|character| if character.is_control() { ' ' } else { character })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

/// A safe file name for a profile export: letters, digits, spaces, dots, dashes, underscores.
pub fn portable_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '_' | '.' | '-')
        })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join("_");
    let stem = joined.trim_matches('.');
    format!(
        "{}{PORTABLE_FILE_EXTENSION}",
        if stem.is_empty() { "profile" } else { stem }
    )
}

/// Write an export into `directory`, never replacing an existing file. Returns the path.
/// Writes outside the game install only.
pub fn save_portable_file(directory: &Path, name: &str, json: &str) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let file_name = portable_file_name(name);
    let stem = file_name.trim_end_matches(PORTABLE_FILE_EXTENSION).to_string();
    let mut path = directory.join(&file_name);
    let mut counter = 2;
    while path.exists() {
        path = directory.join(format!("{stem}-{counter}{PORTABLE_FILE_EXTENSION}"));
        counter += 1;
    }
    write_atomic(&path, json.as_bytes())?;
    Ok(path)
}

/// Write an export to exactly `path`, adding `.modprofile.json` when the name has no extension
/// Sigil knows. The Save dialog already asked about replacing, so this writes through a sibling
/// temporary file and returns the path it used.
pub fn write_portable_to(path: &Path, json: &str) -> Result<PathBuf> {
    let mut path = path.to_path_buf();
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return Err(Error::msg("The export needs a file name."));
    }
    if !name.to_lowercase().ends_with(PORTABLE_FILE_EXTENSION) {
        let stem = name.trim_end_matches(".json").trim_end_matches(".JSON");
        path.set_file_name(format!("{stem}{PORTABLE_FILE_EXTENSION}"));
    }
    write_atomic(&path, json.as_bytes())?;
    Ok(path)
}

/// `.modprofile.json` files in the given folders, newest first, at most `limit`.
pub fn find_portable_files(directories: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for directory in directories {
        let Ok(read_dir) = fs::read_dir(directory) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !name.ends_with(PORTABLE_FILE_EXTENSION) || !path.is_file() {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|data| data.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if !found.iter().any(|(_, existing)| existing == &path) {
                found.push((modified, path));
            }
        }
    }
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().take(limit).map(|(_, path)| path).collect()
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// One downloadable file on a GameBanana mod page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    pub file_id: u64,
    pub file_name: String,
    pub download_url: String,
    pub download_count: u64,
    pub archived: bool,
}

/// A GameBanana mod's name and file list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFiles {
    pub mod_id: u64,
    pub name: String,
    pub nsfw: bool,
    pub files: Vec<ModFile>,
}

/// Parse `apiv11/Mod/<id>?_csvProperties=_idRow,_sName,_bIsNsfw,_aFiles`.
pub fn parse_mod_files(mod_id: u64, body: &str) -> Result<ModFiles> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| Error::Network("GameBanana sent something that is not JSON.".to_string()))?;
    let files = value
        .get("_aFiles")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|file| {
                    Some(ModFile {
                        file_id: file.get("_idRow")?.as_u64()?,
                        file_name: file
                            .get("_sFile")
                            .and_then(Value::as_str)
                            .unwrap_or("download")
                            .to_string(),
                        download_url: file.get("_sDownloadUrl")?.as_str()?.trim().to_string(),
                        download_count: file
                            .get("_nDownloadCount")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        archived: file
                            .get("_bIsArchived")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .filter(|file| !file.download_url.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(ModFiles {
        mod_id,
        name: value
            .get("_sName")
            .and_then(Value::as_str)
            .map(|name| plain_text(name, MAX_HINT_CHARS))
            .unwrap_or_default(),
        nsfw: value.get("_bIsNsfw").and_then(Value::as_bool).unwrap_or(false),
        files,
    })
}

/// Look a mod up on GameBanana. Network: `gamebanana.com`, on the allowlist.
pub fn fetch_mod_files(mod_id: u64) -> Result<ModFiles> {
    let url = format!("{GAMEBANANA_API}/{mod_id}?_csvProperties=_idRow,_sName,_bIsNsfw,_aFiles");
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(API_TIMEOUT_SECONDS)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(&url)
        .header("User-Agent", GAMEBANANA_USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|error| Error::Network(format!("GameBanana could not be reached: {error}")))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::Network(format!("GameBanana response failed: {error}")))?;
    if status == 404 {
        return Err(Error::Network("This mod was removed from GameBanana.".to_string()));
    }
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("GameBanana answered HTTP {status}.")));
    }
    parse_mod_files(mod_id, &body)
}

/// How one import entry resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStatus {
    /// The pinned file is available.
    Exact,
    /// The pinned file is gone or archived; the newest live file is used.
    Upgraded,
    /// Nothing can be fetched for this entry.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub index: usize,
    pub label: String,
    pub enabled: bool,
    pub nsfw: bool,
    pub mod_id: Option<u64>,
    pub pinned_file_id: Option<u64>,
    pub resolved_file_id: Option<u64>,
    pub download_url: Option<String>,
    pub file_name: Option<String>,
    pub status: ImportStatus,
    pub reason: Option<String>,
    /// The slot that already holds this mod, when it is installed.
    pub installed_slot: Option<u16>,
}

impl ImportEntry {
    /// True when the entry needs a download to be part of the imported profile.
    pub fn needs_download(&self) -> bool {
        self.installed_slot.is_none() && self.status != ImportStatus::Unavailable
    }
}

/// Resolve each entry of a portable profile: pinned file, else the newest live file of the same
/// mod, else unavailable. An entry whose pinned file is already installed resolves without a
/// lookup. `lookup` is [`fetch_mod_files`] in the app and a fixture in tests.
pub fn resolve_import(
    portable: &PortableProfile,
    installed: &[AddonIdentity],
    lookup: &mut dyn FnMut(u64) -> Result<ModFiles>,
) -> Vec<ImportEntry> {
    let mut entries = Vec::with_capacity(portable.mods.len());
    resolve_import_each(portable, installed, lookup, &mut |entry| entries.push(entry));
    entries
}

/// A placeholder row for entry `index` while its lookup runs: the shared label, unavailable
/// until [`resolve_import_each`] hands over the real one.
pub fn pending_import_entry(portable: &PortableProfile, index: usize) -> Option<ImportEntry> {
    let entry = portable.mods.get(index)?;
    Some(ImportEntry {
        index,
        label: entry.label(),
        enabled: entry.enabled,
        nsfw: entry.hint.as_ref().and_then(|hint| hint.nsfw).unwrap_or(false),
        mod_id: entry.gamebanana_ref().map(|(mod_id, _)| mod_id),
        pinned_file_id: entry.gamebanana_ref().map(|(_, file_id)| file_id),
        resolved_file_id: None,
        download_url: None,
        file_name: None,
        status: ImportStatus::Unavailable,
        reason: None,
        installed_slot: None,
    })
}

/// [`resolve_import`] one entry at a time, in order: `done` gets each entry as soon as its
/// lookup returns, so the app can start that mod's download while the next one is looked up
/// (DMM 2.4.0 starts a profile import's downloads before every mod is looked up).
pub fn resolve_import_each(
    portable: &PortableProfile,
    installed: &[AddonIdentity],
    lookup: &mut dyn FnMut(u64) -> Result<ModFiles>,
    done: &mut dyn FnMut(ImportEntry),
) {
    let mut cache: BTreeMap<u64, std::result::Result<ModFiles, String>> = BTreeMap::new();
    let mut entries = DoneSink(done);

    for (index, entry) in portable.mods.iter().enumerate() {
        let mut out = ImportEntry {
            index,
            label: entry.label(),
            enabled: entry.enabled,
            nsfw: entry.hint.as_ref().and_then(|hint| hint.nsfw).unwrap_or(false),
            mod_id: None,
            pinned_file_id: None,
            resolved_file_id: None,
            download_url: None,
            file_name: None,
            status: ImportStatus::Unavailable,
            reason: None,
            installed_slot: None,
        };
        let Some((mod_id, file_id)) = entry.gamebanana_ref() else {
            out.reason = Some(format!("Mods from {} are not supported.", entry.source));
            entries.push(out);
            continue;
        };
        out.mod_id = Some(mod_id);
        out.pinned_file_id = Some(file_id);

        let installed_pinned = installed.iter().find(|addon| {
            addon.gamebanana_id == Some(mod_id) && addon.gamebanana_file_id == Some(file_id)
        });
        if let Some(addon) = installed_pinned {
            out.status = ImportStatus::Exact;
            out.resolved_file_id = Some(file_id);
            out.installed_slot = Some(addon.slot);
            if out.label.starts_with("GameBanana mod")
                && let Some(name) = &addon.name
            {
                out.label = name.clone();
            }
            entries.push(out);
            continue;
        }

        let details = cache
            .entry(mod_id)
            .or_insert_with(|| lookup(mod_id).map_err(|error| error.to_string()));
        let details = match details {
            Ok(details) => details,
            Err(error) => {
                out.reason = Some(error.trim_start_matches("network error: ").to_string());
                entries.push(out);
                continue;
            }
        };
        if out.label.starts_with("GameBanana mod") && !details.name.is_empty() {
            out.label = details.name.clone();
        }
        out.nsfw = out.nsfw || details.nsfw;

        let pinned = details
            .files
            .iter()
            .find(|file| file.file_id == file_id && !file.archived);
        let chosen = match pinned {
            Some(file) => {
                out.status = ImportStatus::Exact;
                Some(file)
            }
            None => {
                let newest = details
                    .files
                    .iter()
                    .filter(|file| !file.archived)
                    .max_by_key(|file| file.download_count);
                if newest.is_some() {
                    out.status = ImportStatus::Upgraded;
                } else {
                    out.reason = Some(if details.files.is_empty() {
                        "No files to download.".to_string()
                    } else {
                        "Every file on this mod is archived.".to_string()
                    });
                }
                newest
            }
        };
        if let Some(file) = chosen {
            out.resolved_file_id = Some(file.file_id);
            out.download_url = Some(file.download_url.clone());
            out.file_name = Some(file.file_name.clone());
            out.installed_slot = installed
                .iter()
                .find(|addon| {
                    addon.gamebanana_id == Some(mod_id)
                        && addon.gamebanana_file_id == Some(file.file_id)
                })
                .map(|addon| addon.slot);
        }
        entries.push(out);
    }
}

/// Hands each resolved entry on as `resolve_import_each` pushes it.
struct DoneSink<'a>(&'a mut dyn FnMut(ImportEntry));

impl DoneSink<'_> {
    fn push(&mut self, entry: ImportEntry) {
        (self.0)(entry)
    }
}

/// Download and install one missing import entry, landing it switched off, and remember where
/// it came from. Refuses without `approve`. Game install write, through
/// [`crate::install::apply_install_with`], which backs up anything it replaces.
pub fn install_import_entry(
    game_root: &Path,
    workspace: &Path,
    entry: &ImportEntry,
    approve: bool,
) -> Result<AddonIdentity> {
    if !approve {
        return Err(Error::msg(format!(
            "Installing {} was not approved. Nothing was written.",
            entry.label
        )));
    }
    let (Some(url), Some(mod_id)) = (entry.download_url.clone(), entry.mod_id) else {
        return Err(Error::msg(format!("{} has nothing to download.", entry.label)));
    };
    let request = crate::install::InstallRequest::new(url, workspace.to_path_buf())?;
    let options = crate::install::PlanOptions {
        game_root: Some(game_root.to_path_buf()),
        ..Default::default()
    };
    let plan = crate::install::plan_install(&request, &options)?;
    crate::install::store_plan(&plan)?;
    let record = crate::install::apply_install_with(
        &plan,
        true,
        crate::install::ApplyOptions { land_disabled: true },
    )?;
    let fingerprint = fingerprint_file(&record.target)?;
    remember_source(
        workspace,
        SourceRecord {
            fingerprint: fingerprint.clone(),
            gamebanana_id: Some(mod_id),
            gamebanana_file_id: entry.resolved_file_id,
            name: Some(entry.label.clone()),
        },
    )?;
    let file = record
        .target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(AddonIdentity {
        slot: plan.slot,
        enabled: !file.ends_with(DISABLED_SUFFIX),
        file,
        size_bytes: fs::metadata(&record.target).map(|data| data.len()).unwrap_or(0),
        fingerprint,
        gamebanana_id: Some(mod_id),
        gamebanana_file_id: entry.resolved_file_id,
        name: Some(entry.label.clone()),
    })
}

/// Save an imported profile from the entries the user kept, matched against what is installed
/// now. Entries that are not installed are left out, as the format's reference importer does.
/// Autoexec lines are only kept when `include_autoexec` is set. Workspace only.
pub fn create_profile_from_import(
    workspace: &Path,
    portable: &PortableProfile,
    entries: &[ImportEntry],
    keep: &[bool],
    installed: &[AddonIdentity],
    name: &str,
    include_autoexec: bool,
) -> Result<Profile> {
    let name = clean_profile_name(name)?;
    let mut slots: Vec<ProfileSlot> = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        if !keep.get(position).copied().unwrap_or(false) {
            continue;
        }
        let Some(mod_id) = entry.mod_id else {
            continue;
        };
        let addon = installed
            .iter()
            .find(|addon| {
                addon.gamebanana_id == Some(mod_id)
                    && entry.resolved_file_id.is_some()
                    && addon.gamebanana_file_id == entry.resolved_file_id
            })
            .or_else(|| {
                entry
                    .installed_slot
                    .and_then(|slot| installed.iter().find(|addon| addon.slot == slot))
            });
        let Some(addon) = addon else {
            continue;
        };
        if slots.iter().any(|slot| slot.slot == addon.slot) {
            continue;
        }
        slots.push(ProfileSlot {
            slot: addon.slot,
            file: enabled_file_name(&addon.file),
            enabled: entry.enabled,
            name: Some(entry.label.clone()),
            gamebanana_id: Some(mod_id),
            gamebanana_file_id: addon.gamebanana_file_id.or(entry.resolved_file_id),
            fingerprint: Some(addon.fingerprint.clone()),
        });
    }
    slots.sort_by_key(|slot| slot.slot);

    let commands = portable.autoexec_commands();
    let mut extensions = portable.extensions.clone().unwrap_or_default();
    extensions.remove(EXTENSION_KEY);
    let now = now_iso();
    let profile = Profile {
        schema: PROFILE_SCHEMA,
        slug: unique_slug(workspace, &slugify(&name))?,
        name,
        created_at: now.clone(),
        updated_at: now,
        slots,
        crosshair: portable.crosshair(),
        autoexec: (include_autoexec && !commands.is_empty()).then_some(commands),
        author: portable.profile.author.clone(),
        extensions,
        isolated: false,
    };
    write_profile(workspace, &profile)?;
    Ok(profile)
}

// ---------------------------------------------------------------------------
// Validation and names
// ---------------------------------------------------------------------------

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
    if !(OLDEST_PROFILE_SCHEMA..=PROFILE_SCHEMA).contains(&profile.schema) {
        return Err(Error::msg(format!(
            "Profile {:?} uses unsupported schema {}. This build reads schema {OLDEST_PROFILE_SCHEMA} to {PROFILE_SCHEMA}.",
            profile.name, profile.schema
        )));
    }
    if profile.name.trim().is_empty() {
        return Err(Error::msg("Profile name must not be empty."));
    }
    validate_slug(&profile.slug)?;

    let mut seen: Vec<u16> = Vec::with_capacity(profile.slots.len());
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
    if let Some(crosshair) = &profile.crosshair {
        crosshair.validate()?;
    }
    if let Some(lines) = &profile.autoexec
        && lines.iter().any(|line| line.contains('\0'))
    {
        return Err(Error::msg(format!(
            "Profile {:?} has an autoexec line with a NUL byte.",
            profile.name
        )));
    }
    Ok(())
}

/// Split a recorded slot file into its enabled and disabled names.
///
/// Only `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled` are accepted. A profile is data on disk,
/// so this rejects separators and any other file shape before a rename can reach outside the
/// addon folder.
fn slot_file_names(file: &str) -> Result<(String, String)> {
    let enabled = file.strip_suffix(DISABLED_SUFFIX).unwrap_or(file);
    if slot_number(enabled).is_none() {
        return Err(Error::msg(format!(
            "Profile file {file:?} must be pakNN_dir.vpk or pakNN_dir.vpk.disabled."
        )));
    }
    Ok((enabled.to_string(), format!("{enabled}{DISABLED_SUFFIX}")))
}

fn enabled_file_name(file: &str) -> String {
    file.strip_suffix(DISABLED_SUFFIX).unwrap_or(file).to_string()
}

/// The `NN` of `pakNN_dir.vpk`, or None when the name does not match.
fn slot_number(file: &str) -> Option<u16> {
    let lower = file.to_ascii_lowercase();
    let stem = lower.strip_prefix("pak")?.strip_suffix("_dir.vpk")?;
    if stem.len() != 2 || !stem.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let slot = stem.parse::<u16>().ok()?;
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

fn clean_profile_name(name: &str) -> Result<String> {
    let name = plain_text(name, MAX_PROFILE_NAME_CHARS);
    if name.is_empty() {
        return Err(Error::msg("A profile needs a name."));
    }
    Ok(name)
}

/// A slug for `base` that no saved profile uses yet: `base`, then `base-2`, `base-3` and so on.
fn unique_slug(workspace: &Path, base: &str) -> Result<String> {
    let base: String = if base.is_empty() { "profile" } else { base }
        .chars()
        .take(56)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string();
    validate_slug(&base)?;
    if !profile_path(workspace, &base).exists() && base != "sources" && base != ACTIVE_FILE_NAME {
        return Ok(base);
    }
    for counter in 2..1000 {
        let candidate = format!("{base}-{counter}");
        if !profile_path(workspace, &candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(Error::msg("Too many profiles share this name."))
}

/// The file-name slug a profile named `name` is saved under.
pub fn slug_for(name: &str) -> String {
    slugify(name)
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


#[cfg(test)]
mod locker_shuffle_tests {
    use super::*;

    #[test]
    fn a_profile_keeps_the_locker_shuffle_and_an_old_one_has_none() {
        let workspace = std::env::temp_dir().join(format!("sigil-locker-shuffle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&workspace);
        let profile = Profile {
            schema: PROFILE_SCHEMA,
            name: "Ranked".into(),
            slug: "ranked".into(),
            created_at: String::new(),
            updated_at: String::new(),
            slots: Vec::new(),
            crosshair: None,
            autoexec: None,
            author: None,
            extensions: Map::new(),
            isolated: false,
        };
        write_profile(&workspace, &profile).unwrap();
        assert_eq!(read_profile(&workspace, "ranked").unwrap().locker_shuffle(), None);
        let state = LockerShuffle {
            shuffle: vec!["haze".into()],
            shuffle_pool: vec!["gb:1".into()],
            multi_skin: true,
        };
        assert!(store_locker_shuffle(&workspace, "ranked", &state).unwrap());
        assert!(!store_locker_shuffle(&workspace, "ranked", &state).unwrap(), "the same state writes nothing");
        let read = read_profile(&workspace, "ranked").unwrap();
        assert_eq!(read.locker_shuffle(), Some(state));
        // An empty shuffle is a state too: it turns the shuffle off when applied.
        assert!(store_locker_shuffle(&workspace, "ranked", &LockerShuffle::default()).unwrap());
        assert_eq!(read_profile(&workspace, "ranked").unwrap().locker_shuffle(), Some(LockerShuffle::default()));
        let _ = fs::remove_dir_all(&workspace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "dah-profiles-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(root.join("game/game/citadel/addons")).unwrap();
            fs::create_dir_all(root.join("workspace")).unwrap();
            Scratch { root }
        }
        fn game(&self) -> PathBuf {
            self.root.join("game")
        }
        fn workspace(&self) -> PathBuf {
            self.root.join("workspace")
        }
        fn addons(&self) -> PathBuf {
            addons_dir(&self.game())
        }
        /// A fake slot file. Contents differ per `seed`, so fingerprints differ.
        fn put(&self, file: &str, seed: &str) {
            fs::write(self.addons().join(file), format!("VPK fake payload {seed}")).unwrap();
        }
        fn exists(&self, file: &str) -> bool {
            self.addons().join(file).is_file()
        }
        fn autoexec(&self, text: &str) {
            let path = crate::autoexec::autoexec_path(&self.game());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn read_autoexec(&self) -> String {
            fs::read_to_string(crate::autoexec::autoexec_path(&self.game())).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn create_records_only_enabled_mods_with_fingerprints() {
        let s = Scratch::new("create");
        s.put("pak01_dir.vpk", "a");
        s.put("pak02_dir.vpk.disabled", "b");
        s.autoexec("fps_max 240\n");
        let profile = create_profile(&s.game(), &s.workspace(), "Ranked", false).unwrap();
        assert_eq!(profile.schema, PROFILE_SCHEMA);
        assert_eq!(profile.slug, "ranked");
        assert_eq!(profile.slots.len(), 1);
        assert_eq!(profile.slots[0].slot, 1);
        assert!(profile.slots[0].fingerprint.as_deref().unwrap().starts_with("q1:"));
        assert_eq!(profile.autoexec, Some(vec!["fps_max 240".to_string()]));
        assert!(profile.crosshair.is_none());

        // A second profile with the same name gets its own slug instead of overwriting.
        let second = create_profile(&s.game(), &s.workspace(), "Ranked", false).unwrap();
        assert_eq!(second.slug, "ranked-2");
        assert_eq!(list_profiles(&s.workspace()).unwrap().len(), 2);
    }

    #[test]
    fn create_keeps_the_crosshair_only_when_asked() {
        let s = Scratch::new("crosshair");
        let crosshair = Crosshair::default();
        let text = crate::crosshair::apply_to_autoexec("bind f1 noclip\n", &crosshair).unwrap();
        s.autoexec(&text);
        let without = create_profile(&s.game(), &s.workspace(), "Plain", false).unwrap();
        assert!(without.crosshair.is_none());
        assert_eq!(without.autoexec, Some(vec!["bind f1 noclip".to_string()]));
        let with = create_profile(&s.game(), &s.workspace(), "Aim", true).unwrap();
        assert_eq!(with.crosshair, Some(crosshair));
    }

    #[test]
    fn apply_turns_profile_mods_on_and_everything_else_off() {
        let s = Scratch::new("apply");
        s.put("pak01_dir.vpk", "a");
        s.put("pak02_dir.vpk", "b");
        s.put("pak03_dir.vpk.disabled", "c");
        let profile = create_profile(&s.game(), &s.workspace(), "Two", false).unwrap();
        assert_eq!(profile.slots.len(), 2);

        // Change the install: 02 off, 03 on.
        fs::rename(s.addons().join("pak02_dir.vpk"), s.addons().join("pak02_dir.vpk.disabled"))
            .unwrap();
        fs::rename(s.addons().join("pak03_dir.vpk.disabled"), s.addons().join("pak03_dir.vpk"))
            .unwrap();

        let refused = apply_profile_with(&s.game(), &s.workspace(), "two", false, false);
        assert!(refused.is_err());
        assert!(s.exists("pak03_dir.vpk"), "a refused apply writes nothing");

        let plan = plan_profile_apply(&s.game(), &s.workspace(), &profile).unwrap();
        assert_eq!(plan.turn_on.len(), 1);
        assert_eq!(plan.turn_off.len(), 1);
        assert!(!plan.in_sync());

        let result = apply_profile_with(&s.game(), &s.workspace(), "two", true, false).unwrap();
        assert_eq!(result.changed.len(), 2);
        assert!(s.exists("pak01_dir.vpk"));
        assert!(s.exists("pak02_dir.vpk"));
        assert!(s.exists("pak03_dir.vpk.disabled"));
        assert_eq!(active_profile(&s.workspace()).as_deref(), Some("two"));

        let again = plan_profile_apply(&s.game(), &s.workspace(), &profile).unwrap();
        assert!(again.in_sync());
    }

    #[test]
    fn apply_follows_a_mod_that_moved_slots_and_refuses_a_reused_slot() {
        let s = Scratch::new("moved");
        s.put("pak01_dir.vpk", "skin");
        s.put("pak02_dir.vpk", "hud");
        let profile = create_profile(&s.game(), &s.workspace(), "Moved", false).unwrap();

        // The skin moved from slot 01 to 05; slot 02 now holds a different mod.
        fs::rename(s.addons().join("pak01_dir.vpk"), s.addons().join("pak05_dir.vpk.disabled"))
            .unwrap();
        fs::remove_file(s.addons().join("pak02_dir.vpk")).unwrap();
        s.put("pak02_dir.vpk.disabled", "someone else");

        let plan = plan_profile_apply(&s.game(), &s.workspace(), &profile).unwrap();
        assert!(matches!(plan.entries[0].state, EntryState::Ready { slot: 5, .. }));
        assert!(matches!(plan.entries[1].state, EntryState::SlotReused { slot: 2 }));
        assert_eq!(plan.missing(), 1);

        let result = apply_profile_with(&s.game(), &s.workspace(), "moved", true, false).unwrap();
        assert!(s.exists("pak05_dir.vpk"));
        assert!(s.exists("pak02_dir.vpk.disabled"), "the other mod in slot 02 stays off");
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn apply_refuses_a_slot_with_both_names_before_renaming_anything() {
        let s = Scratch::new("ambiguous");
        s.put("pak01_dir.vpk", "a");
        let profile = create_profile(&s.game(), &s.workspace(), "One", false).unwrap();
        s.put("pak02_dir.vpk", "b");
        s.put("pak03_dir.vpk", "c");
        s.put("pak03_dir.vpk.disabled", "c2");
        let plan = plan_profile_apply(&s.game(), &s.workspace(), &profile).unwrap();
        assert!(plan.blocked.is_some());
        assert!(apply_profile_with(&s.game(), &s.workspace(), "one", true, false).is_err());
        assert!(s.exists("pak02_dir.vpk"), "nothing was renamed");
    }

    #[test]
    fn apply_writes_autoexec_with_a_backup() {
        let s = Scratch::new("autoexec");
        s.put("pak01_dir.vpk", "a");
        let crosshair = Crosshair::default();
        let text = crate::crosshair::apply_to_autoexec("fps_max 240\n", &crosshair).unwrap();
        s.autoexec(&text);
        create_profile(&s.game(), &s.workspace(), "Comp", true).unwrap();

        s.autoexec("fps_max 60\n");
        let result = apply_profile_with(&s.game(), &s.workspace(), "comp", true, false).unwrap();
        assert!(result.autoexec_written);
        let written = s.read_autoexec();
        assert!(written.starts_with("fps_max 240\n"));
        assert!(written.contains(AUTOEXEC_BEGIN));
        let parts = split_autoexec(&written);
        assert_eq!(parts.crosshair, Some(crosshair));
        let backups = fs::read_dir(s.game().join("game/citadel/cfg"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains("backup"))
            .count();
        assert_eq!(backups, 1);

        // In sync now: a second apply does not rewrite autoexec.
        let again = apply_profile_with(&s.game(), &s.workspace(), "comp", true, false).unwrap();
        assert!(!again.autoexec_written);
    }

    /// Grimoire #504: while the Crosshair page is off a profile's crosshair is not written;
    /// its autoexec lines still are, and the result says why the crosshair was left out.
    #[test]
    fn apply_skips_the_crosshair_while_its_page_is_off() {
        let s = Scratch::new("skip-crosshair");
        s.put("pak01_dir.vpk", "a");
        let text = crate::crosshair::apply_to_autoexec("fps_max 240\n", &Crosshair::default()).unwrap();
        s.autoexec(&text);
        create_profile(&s.game(), &s.workspace(), "Comp", true).unwrap();

        s.autoexec("fps_max 60\n");
        let choices = ApplyChoices { skip_crosshair: true };
        let result =
            apply_profile_inner(&s.game(), &s.workspace(), "comp", true, false, choices).unwrap();
        let written = s.read_autoexec();
        assert!(written.contains("fps_max 240"), "the autoexec lines still apply");
        assert!(!written.contains(AUTOEXEC_BEGIN), "no crosshair block was written");
        assert!(result.warnings.iter().any(|w| w == CROSSHAIR_SKIPPED));
    }

    #[test]
    fn compose_keeps_the_current_crosshair_block_when_the_profile_has_none() {
        let current = format!("old line\n{AUTOEXEC_BEGIN}\ncitadel_crosshair_pip_gap 9\n{AUTOEXEC_END}\n");
        let profile = Profile {
            schema: PROFILE_SCHEMA,
            name: "x".into(),
            slug: "x".into(),
            created_at: String::new(),
            updated_at: String::new(),
            slots: Vec::new(),
            crosshair: None,
            autoexec: Some(vec!["new line".into()]),
            author: None,
            extensions: Map::new(),
            isolated: false,
        };
        let text = compose_autoexec(Some(&current), &profile).unwrap();
        assert!(text.starts_with("new line\n"));
        assert!(text.contains("citadel_crosshair_pip_gap 9"));
        assert!(!text.contains("old line"));
    }

    #[test]
    fn schema_one_profiles_still_read_and_apply() {
        let s = Scratch::new("schema1");
        s.put("pak01_dir.vpk.disabled", "a");
        s.put("pak02_dir.vpk", "b");
        let text = r#"{"schema":1,"name":"Old","slug":"old","createdAt":"2026-01-01T00:00:00.000Z","updatedAt":"2026-01-01T00:00:00.000Z","slots":[{"slot":1,"file":"pak01_dir.vpk","enabled":true},{"slot":2,"file":"pak02_dir.vpk","enabled":false}]}"#;
        write_atomic(&profile_path(&s.workspace(), "old"), text.as_bytes()).unwrap();
        let listed = list_profiles(&s.workspace()).unwrap();
        assert_eq!(listed.len(), 1);
        apply_profile_with(&s.game(), &s.workspace(), "old", true, false).unwrap();
        assert!(s.exists("pak01_dir.vpk"));
        assert!(s.exists("pak02_dir.vpk.disabled"));
    }

    #[test]
    fn rename_duplicate_delete_and_active_marker() {
        let s = Scratch::new("manage");
        s.put("pak01_dir.vpk", "a");
        create_profile(&s.game(), &s.workspace(), "Main", false).unwrap();
        let renamed = rename_profile(&s.workspace(), "main", "  Main   setup ").unwrap();
        assert_eq!(renamed.name, "Main setup");
        assert_eq!(renamed.slug, "main");
        let copy = duplicate_profile(&s.workspace(), "main").unwrap();
        assert_eq!(copy.name, "Main setup copy");
        assert_ne!(copy.slug, "main");
        assert_eq!(copy.slots, renamed.slots);
        set_active_profile(&s.workspace(), Some("main")).unwrap();
        delete_profile(&s.workspace(), "main").unwrap();
        assert!(active_profile(&s.workspace()).is_none());
        assert_eq!(list_profiles(&s.workspace()).unwrap().len(), 1);
        assert!(rename_profile(&s.workspace(), &copy.slug, "   ").is_err());
    }

    #[test]
    fn share_code_round_trips_and_caps_inflated_size() {
        let json = r#"{"hello":"world"}"#;
        let code = encode_share_code(json).unwrap();
        assert!(code.starts_with("mp1:"));
        assert_eq!(decode_share_code(&code).unwrap(), json);
        // Wrapped by a chat app.
        let wrapped = format!("{}\n {}", &code[..10], &code[10..]);
        assert_eq!(decode_share_code(&wrapped).unwrap(), json);
        let bomb = encode_share_code(&"a".repeat(MAX_INFLATED_SHARE_CODE_BYTES + 10)).unwrap();
        assert!(decode_share_code(&bomb).is_err());
        assert!(decode_share_code("mp1:!!!").is_err());
        assert!(decode_share_code("nope").is_err());
    }

    fn identity(slot: u16, mod_id: Option<u64>, file_id: Option<u64>, name: &str) -> AddonIdentity {
        AddonIdentity {
            slot,
            file: format!("pak{slot:02}_dir.vpk"),
            enabled: true,
            size_bytes: 10,
            fingerprint: format!("q1:{slot}"),
            gamebanana_id: mod_id,
            gamebanana_file_id: file_id,
            name: Some(name.to_string()),
        }
    }

    fn sample_profile() -> Profile {
        Profile {
            schema: PROFILE_SCHEMA,
            name: "Comp Loadout".into(),
            slug: "comp-loadout".into(),
            created_at: String::new(),
            updated_at: String::new(),
            slots: vec![
                ProfileSlot {
                    slot: 1,
                    file: "pak01_dir.vpk".into(),
                    enabled: true,
                    name: Some("Dragon Skin".into()),
                    gamebanana_id: Some(123),
                    gamebanana_file_id: Some(456),
                    fingerprint: Some("q1:1".into()),
                },
                ProfileSlot {
                    slot: 2,
                    file: "pak02_dir.vpk".into(),
                    enabled: true,
                    name: None,
                    gamebanana_id: None,
                    gamebanana_file_id: None,
                    fingerprint: Some("q1:2".into()),
                },
                ProfileSlot {
                    slot: 3,
                    file: "pak03_dir.vpk".into(),
                    enabled: true,
                    name: None,
                    gamebanana_id: None,
                    gamebanana_file_id: None,
                    fingerprint: Some("q1:3".into()),
                },
            ],
            crosshair: Some(Crosshair::default()),
            autoexec: Some(vec!["fps_max 240".into()]),
            author: None,
            extensions: {
                let mut map = Map::new();
                map.insert("othertool".into(), serde_json::json!({"slot": 3}));
                map
            },
            isolated: false,
        }
    }

    #[test]
    fn export_skips_local_mods_borrows_installed_ids_and_keeps_foreign_extensions() {
        let installed = vec![
            identity(1, Some(123), Some(456), "Dragon Skin"),
            identity(2, None, None, "Local"),
            identity(3, Some(777), Some(888), "Clean HUD"),
        ];
        let export = export_portable(&sample_profile(), &installed, "0.1.0").unwrap();
        assert_eq!(export.profile.mods.len(), 2);
        assert_eq!(export.profile.mods[1].gamebanana_ref(), Some((777, 888)));
        assert_eq!(export.skipped, vec!["Local".to_string()]);
        let extensions = export.profile.extensions.as_ref().unwrap();
        assert!(extensions.contains_key("othertool"));
        assert!(extensions["sigil"]["crosshair"].is_object());

        // The share code and the file are the same document.
        let from_code = read_portable_input(&export.share_code).unwrap();
        let from_json = read_portable_input(&export.json).unwrap();
        assert_eq!(from_code, from_json);
        assert_eq!(from_code.crosshair(), Some(Crosshair::default()));
        assert_eq!(from_code.autoexec_commands(), vec!["fps_max 240".to_string()]);
    }

    #[test]
    fn parse_refuses_other_formats_versions_and_games_and_cleans_hints() {
        let base = serde_json::json!({
            "format": "mod-profile",
            "schemaVersion": "1.4",
            "game": {"steamAppId": 1422450},
            "exportedAt": "2026-05-15T18:22:01.000Z",
            "exportedBy": {"tool": "x", "version": "1"},
            "profile": {"name": "  Comp\nLoadout  ", "author": "<b>me</b>"},
            "mods": [{
                "source": "gamebanana",
                "ref": {"submissionId": 1, "fileId": 2},
                "enabled": true,
                "priority": 1,
                "hint": {"name": "Skin\u{0007}", "thumbnailUrl": "https://evil.example/x.png"}
            }]
        });
        let parsed = parse_portable(&base.to_string()).unwrap();
        assert_eq!(parsed.profile.name, "Comp Loadout");
        assert_eq!(parsed.mods[0].hint.as_ref().unwrap().name.as_deref(), Some("Skin"));
        assert!(parsed.mods[0].hint.as_ref().unwrap().thumbnail_url.is_none());

        let mut other = base.clone();
        other["format"] = "something".into();
        assert!(parse_portable(&other.to_string()).is_err());
        let mut major = base.clone();
        major["schemaVersion"] = "2.0".into();
        assert!(parse_portable(&major.to_string()).is_err());
        let mut game = base.clone();
        game["game"]["steamAppId"] = 730.into();
        assert!(parse_portable(&game.to_string()).is_err());
        let mut broken = base.clone();
        broken["mods"][0]["ref"] = serde_json::json!({"submissionId": 1});
        assert!(parse_portable(&broken.to_string()).is_err());
    }

    #[test]
    fn foreign_crosshair_shape_fills_defaults_and_maps_the_old_border_flag() {
        let value = serde_json::json!({
            "pipGap": 4, "pipHeight": 2, "pipWidth": 1, "pipOpacity": 0.85,
            "pipBorder": true, "dotOpacity": 0, "dotOutlineOpacity": 0,
            "colorR": 255, "colorG": 0, "colorB": 255
        });
        let crosshair = crosshair_from_value(&value).unwrap();
        assert_eq!(crosshair.pip_gap, 4);
        assert_eq!(crosshair.pip_outline_border, 1);
        assert_eq!(crosshair.color_g, 0);
        assert!(crosshair_from_value(&serde_json::json!({"pipGap": -40})).is_none());
    }

    fn files(mod_id: u64, name: &str, list: &[(u64, bool, u64)]) -> ModFiles {
        ModFiles {
            mod_id,
            name: name.into(),
            nsfw: false,
            files: list
                .iter()
                .map(|(id, archived, count)| ModFile {
                    file_id: *id,
                    file_name: format!("{id}.zip"),
                    download_url: format!("https://gamebanana.com/dl/{id}"),
                    download_count: *count,
                    archived: *archived,
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_marks_exact_upgraded_unavailable_and_installed() {
        let json = serde_json::json!({
            "format": "mod-profile", "schemaVersion": "1.1", "game": {"steamAppId": 1422450},
            "exportedAt": "x", "exportedBy": {"tool": "x", "version": "1"},
            "profile": {"name": "P"},
            "mods": [
                {"source": "gamebanana", "ref": {"submissionId": 1, "fileId": 10}, "enabled": true, "priority": 1},
                {"source": "gamebanana", "ref": {"submissionId": 2, "fileId": 20}, "enabled": true, "priority": 2},
                {"source": "gamebanana", "ref": {"submissionId": 3, "fileId": 30}, "enabled": true, "priority": 3},
                {"source": "gamebanana", "ref": {"submissionId": 4, "fileId": 40}, "enabled": true, "priority": 4,
                 "hint": {"name": "Gone"}},
                {"source": "nexus", "ref": {"id": 5}, "enabled": true, "priority": 5}
            ]
        });
        let portable = parse_portable(&json.to_string()).unwrap();
        let installed = vec![identity(7, Some(3), Some(30), "Already Here")];
        let mut lookups = Vec::new();
        let mut lookup = |id: u64| -> Result<ModFiles> {
            lookups.push(id);
            match id {
                1 => Ok(files(1, "Exact Mod", &[(10, false, 5)])),
                2 => Ok(files(2, "Newer Mod", &[(20, true, 50), (21, false, 3), (22, false, 9)])),
                _ => Err(Error::Network("This mod was removed from GameBanana.".into())),
            }
        };
        let entries = resolve_import(&portable, &installed, &mut lookup);
        assert_eq!(entries[0].status, ImportStatus::Exact);
        assert_eq!(entries[0].label, "Exact Mod");
        assert_eq!(entries[1].status, ImportStatus::Upgraded);
        assert_eq!(entries[1].resolved_file_id, Some(22));
        assert_eq!(entries[2].status, ImportStatus::Exact);
        assert_eq!(entries[2].installed_slot, Some(7));
        assert!(!entries[2].needs_download());
        assert_eq!(entries[3].status, ImportStatus::Unavailable);
        assert_eq!(entries[3].label, "Gone");
        assert!(entries[3].reason.as_deref().unwrap().contains("removed"));
        assert_eq!(entries[4].status, ImportStatus::Unavailable);
        assert_eq!(lookups, vec![1, 2, 4], "installed and foreign entries need no lookup");

        // One at a time: each entry is handed over before the next lookup starts.
        let events = std::cell::RefCell::new(Vec::new());
        let mut lookup = |id: u64| -> Result<ModFiles> {
            events.borrow_mut().push(format!("lookup {id}"));
            Ok(files(id, "Mod", &[(id * 10, false, 1)]))
        };
        let mut streamed = Vec::new();
        resolve_import_each(&portable, &installed, &mut lookup, &mut |entry| {
            events.borrow_mut().push(format!("done {}", entry.index));
            streamed.push(entry);
        });
        assert_eq!(events.borrow()[..4], ["lookup 1", "done 0", "lookup 2", "done 1"]);
        assert_eq!(streamed.iter().map(|e| e.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        let pending = pending_import_entry(&portable, 3).unwrap();
        assert_eq!((pending.label.as_str(), pending.mod_id, pending.installed_slot), ("Gone", Some(4), None));
        assert!(pending_import_entry(&portable, 9).is_none());
    }

    #[test]
    fn import_saves_the_kept_installed_entries_and_autoexec_only_when_asked() {
        let s = Scratch::new("import");
        let json = serde_json::json!({
            "format": "mod-profile", "schemaVersion": "1.1", "game": {"steamAppId": 1422450},
            "exportedAt": "x", "exportedBy": {"tool": "x", "version": "1"},
            "profile": {"name": "Shared", "author": "slush"},
            "mods": [
                {"source": "gamebanana", "ref": {"submissionId": 1, "fileId": 10}, "enabled": true, "priority": 1},
                {"source": "gamebanana", "ref": {"submissionId": 2, "fileId": 20}, "enabled": true, "priority": 2}
            ],
            "extensions": {"sigil": {"autoexecCommands": ["fps_max 144"]}, "othertool": {"a": 1}}
        });
        let portable = parse_portable(&json.to_string()).unwrap();
        let installed = vec![identity(4, Some(1), Some(10), "One"), identity(9, Some(2), Some(20), "Two")];
        let mut lookup = |_: u64| -> Result<ModFiles> { unreachable!() };
        let entries = resolve_import(&portable, &installed, &mut lookup);
        let profile = create_profile_from_import(
            &s.workspace(),
            &portable,
            &entries,
            &[true, false],
            &installed,
            "Shared",
            false,
        )
        .unwrap();
        assert_eq!(profile.slots.len(), 1);
        assert_eq!(profile.slots[0].slot, 4);
        assert_eq!(profile.slots[0].gamebanana_id, Some(1));
        assert!(profile.autoexec.is_none());
        assert_eq!(profile.author.as_deref(), Some("slush"));
        assert!(profile.extensions.contains_key("othertool"));

        let with = create_profile_from_import(
            &s.workspace(), &portable, &entries, &[true, true], &installed, "Shared", true,
        )
        .unwrap();
        assert_eq!(with.slug, "shared-2");
        assert_eq!(with.autoexec, Some(vec!["fps_max 144".to_string()]));
    }

    #[test]
    fn installed_identity_comes_from_remembered_sources() {
        let s = Scratch::new("sources");
        s.put("pak03_dir.vpk.disabled", "imported");
        let fingerprint = fingerprint_file(&s.addons().join("pak03_dir.vpk.disabled")).unwrap();
        remember_source(
            &s.workspace(),
            SourceRecord {
                fingerprint,
                gamebanana_id: Some(55),
                gamebanana_file_id: Some(66),
                name: Some("Imported".into()),
            },
        )
        .unwrap();
        let identities = installed_identities(&s.game(), &s.workspace()).unwrap();
        assert_eq!(identities[0].gamebanana_id, Some(55));
        assert_eq!(identities[0].name.as_deref(), Some("Imported"));
        // The sources file is not a profile.
        assert!(list_profiles(&s.workspace()).unwrap().is_empty());
    }

    #[test]
    fn install_entry_refuses_without_approval() {
        let s = Scratch::new("install");
        let entry = ImportEntry {
            index: 0,
            label: "Skin".into(),
            enabled: true,
            nsfw: false,
            mod_id: Some(1),
            pinned_file_id: Some(2),
            resolved_file_id: Some(2),
            download_url: Some("https://gamebanana.com/dl/2".into()),
            file_name: None,
            status: ImportStatus::Exact,
            reason: None,
            installed_slot: None,
        };
        assert!(install_import_entry(&s.game(), &s.workspace(), &entry, false).is_err());
        assert_eq!(fs::read_dir(s.addons()).unwrap().count(), 0);
    }

    #[test]
    fn source_strings_name_mods_and_files() {
        assert_eq!(gamebanana_ids_from_source("715100"), (Some(715100), None));
        assert_eq!(gamebanana_ids_from_source("gamebanana:12"), (Some(12), None));
        assert_eq!(
            gamebanana_ids_from_source("https://gamebanana.com/mods/715100?x=1"),
            (Some(715100), None)
        );
        assert_eq!(gamebanana_ids_from_source("https://gamebanana.com/dl/99"), (None, Some(99)));
        assert_eq!(gamebanana_ids_from_source("https://evil.com/mods/1"), (None, None));
    }

    #[test]
    fn mod_file_list_parses_archived_flags() {
        let body = r#"{"_idRow":1,"_sName":"Skin","_bIsNsfw":false,"_aFiles":[
            {"_idRow":10,"_sFile":"a.zip","_sDownloadUrl":"https://gamebanana.com/dl/10","_nDownloadCount":3,"_bIsArchived":true},
            {"_idRow":11,"_sFile":"b.zip","_sDownloadUrl":"https://gamebanana.com/dl/11","_nDownloadCount":1}]}"#;
        let parsed = parse_mod_files(1, body).unwrap();
        assert_eq!(parsed.name, "Skin");
        assert!(parsed.files[0].archived);
        assert!(!parsed.files[1].archived);
    }

    #[test]
    fn export_file_names_are_safe_and_never_overwrite() {
        assert_eq!(portable_file_name("Comp / Loadout!"), "Comp_Loadout.modprofile.json");
        assert_eq!(portable_file_name("../.."), "profile.modprofile.json");
        let s = Scratch::new("export-file");
        let first = save_portable_file(&s.workspace(), "Comp", "{}").unwrap();
        let second = save_portable_file(&s.workspace(), "Comp", "{}").unwrap();
        assert_ne!(first, second);
        let found = find_portable_files(&[s.workspace()], 5);
        assert_eq!(found.len(), 2);
    }

    const STOCK_GAMEINFO: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame_Language \"citadel_*LANGUAGE*\"\n\n\t\t\tGame citadel/addons\n\t\t\tGame \"citadel\"\n\t\t\tGame \"core\"\n\t\t}\n\t}\n}\n";

    fn write_gameinfo(s: &Scratch) {
        let path = crate::deploy::gameinfo_path(&s.game());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, STOCK_GAMEINFO).unwrap();
    }

    fn read_gameinfo(s: &Scratch) -> String {
        fs::read_to_string(crate::deploy::gameinfo_path(&s.game())).unwrap()
    }

    #[test]
    fn apply_refuses_while_the_game_runs() {
        let s = Scratch::new("running");
        s.put("pak01_dir.vpk", "a");
        create_profile(&s.game(), &s.workspace(), "One", false).unwrap();
        s.put("pak02_dir.vpk", "b");
        let error = apply_profile_with(&s.game(), &s.workspace(), "one", true, true).unwrap_err();
        assert!(error.to_string().contains("running"));
        assert!(s.exists("pak02_dir.vpk"), "nothing was renamed");
    }

    #[test]
    fn undo_puts_the_slot_files_and_autoexec_back() {
        let s = Scratch::new("undo");
        s.put("pak01_dir.vpk", "a");
        s.put("pak02_dir.vpk", "b");
        s.autoexec("fps_max 240\n");
        create_profile(&s.game(), &s.workspace(), "Two", false).unwrap();
        fs::rename(s.addons().join("pak02_dir.vpk"), s.addons().join("pak02_dir.vpk.disabled"))
            .unwrap();
        s.put("pak03_dir.vpk", "c");
        s.autoexec("fps_max 60\n");

        let result = apply_profile_with(&s.game(), &s.workspace(), "two", true, false).unwrap();
        assert_eq!(result.changes.len(), 2);
        assert!(result.autoexec_written);
        assert_eq!(result.previous_active, None);
        assert!(s.exists("pak03_dir.vpk.disabled"));
        assert_eq!(s.read_autoexec(), "fps_max 240\n");

        let undo = undo_profile_apply(&s.game(), &s.workspace(), &result, true).unwrap();
        assert_eq!(undo.reverted, 2);
        assert!(undo.autoexec_restored);
        assert!(s.exists("pak03_dir.vpk"), "the mod switched off is back on");
        assert!(s.exists("pak02_dir.vpk.disabled"), "the mod switched on is off again");
        assert_eq!(s.read_autoexec(), "fps_max 60\n");
        assert_eq!(active_profile(&s.workspace()), None);
    }

    #[test]
    fn an_isolated_profile_copies_into_its_folder_and_mounts_it() {
        let s = Scratch::new("isolated");
        write_gameinfo(&s);
        s.put("pak01_dir.vpk", "a");
        s.put("pak02_dir.vpk", "b");
        let mut profile = create_profile(&s.game(), &s.workspace(), "Ranked", false).unwrap();
        profile.isolated = true;
        write_profile(&s.workspace(), &profile).unwrap();
        s.put("pak03_dir.vpk", "other");

        let result = apply_profile_with(&s.game(), &s.workspace(), "ranked", true, false).unwrap();
        assert_eq!(result.folder.as_deref(), Some("sigil-ranked"));
        assert_eq!(result.copied, 2);
        let folder = s.addons().join("sigil-ranked");
        assert!(folder.join("pak01_dir.vpk").is_file());
        assert!(folder.join("pak02_dir.vpk").is_file());
        assert!(!folder.join("pak03_dir.vpk").exists(), "another profile's mod stays out");
        let text = read_gameinfo(&s);
        assert!(text.contains("Game citadel/addons/sigil-ranked"));
        assert!(!text.contains("\t\t\tGame citadel/addons\n"));
        assert_eq!(mounted_folder(&s.game()).as_deref(), Some("sigil-ranked"));
        assert_eq!(active_profile(&s.workspace()).as_deref(), Some("ranked"));

        // Switching back to a shared profile unmounts the folder with the managed block.
        s.put("pak04_dir.vpk", "d");
        create_profile(&s.game(), &s.workspace(), "Shared", false).unwrap();
        let back = apply_profile_with(&s.game(), &s.workspace(), "shared", true, false).unwrap();
        assert_eq!(back.previous_folder.as_deref(), Some("sigil-ranked"));
        assert_eq!(back.folder, None);
        assert_eq!(mounted_folder(&s.game()), None);
        assert!(read_gameinfo(&s).contains("\t\t\tGame citadel/addons\n"));

        // Undo puts the mount and the active profile back.
        let undo = undo_profile_apply(&s.game(), &s.workspace(), &back, true).unwrap();
        assert_eq!(undo.mount.as_deref(), Some("sigil-ranked"));
        assert_eq!(mounted_folder(&s.game()).as_deref(), Some("sigil-ranked"));
        assert_eq!(active_profile(&s.workspace()).as_deref(), Some("ranked"));
    }

    #[test]
    fn applying_and_updating_leave_a_recovery_point() {
        let s = Scratch::new("snapshots");
        s.put("pak01_dir.vpk", "a");
        create_profile(&s.game(), &s.workspace(), "One", false).unwrap();
        assert!(list_snapshots(&s.workspace()).is_empty());

        s.put("pak02_dir.vpk", "b");
        apply_profile_with(&s.game(), &s.workspace(), "one", true, false).unwrap();
        let after_apply = list_snapshots(&s.workspace());
        assert_eq!(after_apply.len(), 1);
        assert_eq!(after_apply[0].trigger, SnapshotTrigger::PreApplyProfile);
        assert_eq!(after_apply[0].mods, 2, "what was on before the apply");

        update_profile(&s.game(), &s.workspace(), "one").unwrap();
        let after_update = list_snapshots(&s.workspace());
        assert_eq!(after_update.len(), 2);
        assert!(after_update.iter().any(|point| point.trigger == SnapshotTrigger::PreUpdate));
    }

    #[test]
    fn restoring_a_snapshot_turns_the_recorded_mods_back_on() {
        let s = Scratch::new("restore");
        s.put("pak01_dir.vpk", "a");
        s.put("pak02_dir.vpk", "b");
        let point = capture_snapshot(
            &s.game(),
            &s.workspace(),
            SnapshotTrigger::Manual,
            Some("Two mods"),
        )
        .unwrap();
        fs::rename(s.addons().join("pak02_dir.vpk"), s.addons().join("pak02_dir.vpk.disabled"))
            .unwrap();
        s.put("pak03_dir.vpk", "c");

        assert!(restore_snapshot(&s.game(), &s.workspace(), &point.id, false).is_err());
        let result = restore_snapshot(&s.game(), &s.workspace(), &point.id, true).unwrap();
        assert_eq!(result.changes.len(), 2);
        assert!(s.exists("pak02_dir.vpk"));
        assert!(s.exists("pak03_dir.vpk.disabled"));
        assert_eq!(active_profile(&s.workspace()), None);
    }
}
