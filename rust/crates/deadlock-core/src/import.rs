//! Library metadata and imports from other mod managers.
//!
//! Three parts, kept apart so each can be checked on its own:
//!
//! 1. The library store ([`Library`], [`record_meta`]): a small JSON file under the app data
//!    directory that remembers a name, author, thumbnail URL and GameBanana id per addon slot.
//!    The addon folder only knows `pakNN_dir.vpk`; this is where the human name lives. An entry
//!    matches a slot by slot number and directory VPK size, so a file someone else puts in the
//!    slot does not inherit the name.
//! 2. Detection and planning (read only): [`detect_sources`] finds Grimoire and Deadlock Mod
//!    Manager on this machine and lists the mods they manage, [`local_mods`] turns files the
//!    user picked into the same shape, and [`plan_import`] validates each VPK, skips what is
//!    already installed and assigns free slots. [`assign_slots`] is the pure slot rule.
//! 3. [`apply_plan`]: the one function here that writes into `game/citadel/addons`.
//!
//! Layout facts about the other managers come from reading their on-disk data, not their code.
//! Grimoire (MIT) keeps `mod-metadata.json` in its Electron userData folder (`grimoire`) and
//! parks disabled mods in `citadel/addons/.disabled/`. Deadlock Mod Manager keeps `state.json`
//! in its app data folder (`dev.stormix.deadlock-mod-manager`) and a `.dmm.json` manifest in
//! the addon folder that maps each mod id to its VPK names.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::addons::DISABLED_SUFFIX;
use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::install::occupied_slots;
use crate::payload::{PayloadOptions, has_errors, validate_payload};
use crate::util::now_iso;
use crate::vpk::{ReadVpkOptions, VPK_DIRECTORY_ARCHIVE, read_vpk_directory_with};

pub const LIBRARY_SCHEMA: u32 = 1;
pub const LIBRARY_FILE_NAME: &str = "library.json";

/// Grimoire's Electron userData folder name.
pub const GRIMOIRE_DIR_NAME: &str = "grimoire";
/// Deadlock Mod Manager's Tauri identifier, which names its app data folders.
pub const DMM_IDENTIFIER: &str = "dev.stormix.deadlock-mod-manager";

// ---------------------------------------------------------------------------
// Library store
// ---------------------------------------------------------------------------

/// What Sigil remembers about one installed slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModMeta {
    pub slot: u8,
    /// Size of the directory VPK when this was recorded. Together with `slot` it is the key.
    pub size_bytes: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamebanana_id: Option<u64>,
    /// `browse`, `local`, `grimoire` or `dmm`.
    pub source: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub schema: u32,
    pub mods: Vec<ModMeta>,
}

impl Default for Library {
    fn default() -> Self {
        Library {
            schema: LIBRARY_SCHEMA,
            mods: Vec::new(),
        }
    }
}

impl Library {
    /// The entry for a slot whose directory VPK still has the recorded size.
    pub fn find(&self, slot: u8, size_bytes: u64) -> Option<&ModMeta> {
        self.mods
            .iter()
            .rev()
            .find(|meta| meta.slot == slot && meta.size_bytes == size_bytes)
    }
}

pub fn library_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LIBRARY_FILE_NAME)
}

/// Read the store. A missing file is an empty library.
pub fn read_library(path: &Path) -> Result<Library> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| Error::msg(format!("{} is not a library file: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Library::default()),
        Err(error) => Err(error.into()),
    }
}

/// Write the store atomically (temp file, then rename). App data, not the game install.
pub fn write_library(path: &Path, library: &Library) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(library)
        .map_err(|error| Error::msg(format!("Could not encode the library: {error}")))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, text)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

/// Remember `meta` for its slot, replacing whatever the slot had before.
pub fn record_meta(path: &Path, mut meta: ModMeta) -> Result<()> {
    if meta.recorded_at.is_empty() {
        meta.recorded_at = now_iso();
    }
    let mut library = read_library(path).unwrap_or_default();
    library.mods.retain(|existing| existing.slot != meta.slot);
    library.mods.push(meta);
    library.mods.sort_by_key(|entry| entry.slot);
    write_library(path, &library)
}

/// Fill in a missing preview for the mod in `slot`, only when that slot still holds the same mod
/// (GameBanana id and file size): a lookup that lands late never touches a newer install.
/// True when it wrote.
pub fn set_thumbnail(path: &Path, slot: u8, size_bytes: u64, gamebanana_id: u64, url: &str) -> Result<bool> {
    let mut library = read_library(path).unwrap_or_default();
    let Some(entry) = library.mods.iter_mut().find(|m| {
        m.slot == slot && m.size_bytes == size_bytes && m.gamebanana_id == Some(gamebanana_id)
    }) else {
        return Ok(false);
    };
    if entry.thumbnail_url.is_some() {
        return Ok(false);
    }
    entry.thumbnail_url = Some(url.to_string());
    write_library(path, &library)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Grimoire,
    Dmm,
    Local,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Grimoire => "Grimoire",
            SourceKind::Dmm => "Deadlock Mod Manager",
            SourceKind::Local => "Local files",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            SourceKind::Grimoire => "grimoire",
            SourceKind::Dmm => "dmm",
            SourceKind::Local => "local",
        }
    }
}

/// One mod another manager (or the user's file picker) knows about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignMod {
    pub name: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
    pub gamebanana_id: Option<u64>,
    /// The directory VPK, or the archive for a picked `.zip` / `.7z` / `.rar`.
    pub file: PathBuf,
    /// Chunk files that belong to `file`, in archive index order.
    pub chunks: Vec<PathBuf>,
    pub size_bytes: u64,
    pub enabled_in_source: bool,
    /// An archive goes through the install pipeline instead of a plain copy.
    pub archive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSource {
    pub kind: SourceKind,
    /// The folder the manager was found in.
    pub root: PathBuf,
    pub mods: Vec<ForeignMod>,
    pub warnings: Vec<String>,
}

/// Where to look for other managers' data. `from_environment` reads the usual per-OS folders;
/// tests pass temp folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectRoots {
    /// Roaming config folders: `%APPDATA%`, `~/Library/Application Support`, `XDG_CONFIG_HOME`.
    pub config: Vec<PathBuf>,
    /// Local data folders: `%LOCALAPPDATA%`, `XDG_DATA_HOME`.
    pub data: Vec<PathBuf>,
}

impl DetectRoots {
    pub fn from_environment() -> Self {
        let mut roots = DetectRoots::default();
        let env_dir = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let home = env_dir("HOME").or_else(|| env_dir("USERPROFILE"));
        if let Some(path) = env_dir("APPDATA") {
            roots.config.push(path);
        }
        if let Some(path) = env_dir("LOCALAPPDATA") {
            roots.data.push(path);
        }
        if cfg!(target_os = "macos")
            && let Some(home) = &home
        {
            roots.config.push(home.join("Library").join("Application Support"));
        }
        if cfg!(all(unix, not(target_os = "macos"))) {
            match (env_dir("XDG_CONFIG_HOME"), &home) {
                (Some(path), _) => roots.config.push(path),
                (None, Some(home)) => roots.config.push(home.join(".config")),
                _ => {}
            }
            match (env_dir("XDG_DATA_HOME"), &home) {
                (Some(path), _) => roots.data.push(path),
                (None, Some(home)) => roots.data.push(home.join(".local").join("share")),
                _ => {}
            }
        }
        roots
    }

    fn all(&self) -> Vec<PathBuf> {
        let mut all: Vec<PathBuf> = Vec::new();
        for root in self.config.iter().chain(self.data.iter()) {
            if !all.contains(root) {
                all.push(root.clone());
            }
        }
        all
    }
}

/// Find Grimoire and Deadlock Mod Manager and list their mods. Read only.
///
/// A manager counts as found when its data file exists. Mods that live in the game folder
/// need `game_root`; without it only what the data file names is listed.
pub fn detect_sources(roots: &DetectRoots, game_root: Option<&Path>) -> Vec<DetectedSource> {
    let mut found = Vec::new();

    for root in roots.all() {
        for name in [GRIMOIRE_DIR_NAME, "Grimoire"] {
            let dir = root.join(name);
            if dir.join("mod-metadata.json").is_file() || dir.join("settings.json").is_file() {
                if found
                    .iter()
                    .any(|source: &DetectedSource| source.kind == SourceKind::Grimoire)
                {
                    continue;
                }
                found.push(read_grimoire(&dir, game_root));
            }
        }
    }

    let dmm_dirs: Vec<PathBuf> = roots
        .all()
        .into_iter()
        .map(|root| root.join(DMM_IDENTIFIER))
        .filter(|dir| dir.is_dir())
        .collect();
    let dmm_manifest = game_root
        .map(|root| addons_dir(root).join(".dmm.json"))
        .filter(|path| path.is_file());
    if dmm_dirs.iter().any(|dir| dir.join("state.json").is_file()) || dmm_manifest.is_some() {
        found.push(read_dmm(&dmm_dirs, game_root));
    }

    found
}

/// Grimoire's mods: metadata from `mod-metadata.json`, files from the game folder.
///
/// Enabled mods sit in `citadel/addons` (already Sigil's folder: listed so their names can be
/// adopted, never copied), in overflow folders `citadel/addons1..9` and in `citadel/grimoire`.
/// Disabled mods are parked in `citadel/addons/.disabled/` under free-form names.
pub fn read_grimoire(user_data: &Path, game_root: Option<&Path>) -> DetectedSource {
    let mut warnings = Vec::new();
    let metadata: BTreeMap<String, Value> =
        match fs::read_to_string(user_data.join("mod-metadata.json")) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => map.into_iter().collect(),
                Ok(_) | Err(_) => {
                    warnings.push("mod-metadata.json could not be read.".to_string());
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };

    let mut mods = Vec::new();
    if let Some(game_root) = game_root {
        let citadel = game_root.join("game").join("citadel");
        let mut folders: Vec<(PathBuf, String, bool)> = vec![
            (citadel.join("addons"), String::new(), true),
            (citadel.join("addons").join(".disabled"), String::new(), false),
            (citadel.join("grimoire"), "grimoire/".to_string(), true),
        ];
        for n in 1..=9 {
            folders.push((citadel.join(format!("addons{n}")), format!("addons{n}/"), true));
        }
        for (folder, key_prefix, enabled) in folders {
            for (file, chunks) in dir_vpks_in(&folder) {
                let file_name = file_name_of(&file);
                let meta = metadata.get(&format!("{key_prefix}{file_name}"));
                let field = |name: &str| {
                    meta.and_then(|value| value.get(name))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string)
                };
                mods.push(ForeignMod {
                    name: field("modName").unwrap_or_else(|| pretty_name(&file_name)),
                    author: field("author"),
                    thumbnail_url: field("thumbnailUrl").filter(|url| is_http(url)),
                    gamebanana_id: meta
                        .and_then(|value| value.get("gameBananaId"))
                        .and_then(Value::as_u64),
                    size_bytes: file_size(&file),
                    file,
                    chunks,
                    enabled_in_source: enabled,
                    archive: false,
                });
            }
        }
    } else {
        warnings.push("The game folder is unknown, so Grimoire's files could not be listed.".to_string());
    }

    DetectedSource {
        kind: SourceKind::Grimoire,
        root: user_data.to_path_buf(),
        mods,
        warnings,
    }
}

/// Deadlock Mod Manager's mods for the default profile.
///
/// `state.json` names the mods (`local-config` holds `{state: {localMods}}`, as a JSON string
/// or an object). `citadel/addons/.dmm.json` maps each mod id to `currentVpks` (enabled, in its
/// shard folder `addons` or `addonsN`) and `disabledVpks` (parked in `addons` as
/// `<id>_<name>.vpk`). Mods with no file on disk fall back to the download cache
/// `<app dir>/mods/<id>/files`.
pub fn read_dmm(app_dirs: &[PathBuf], game_root: Option<&Path>) -> DetectedSource {
    let mut warnings = Vec::new();
    let root = app_dirs.first().cloned().unwrap_or_default();

    let mut records: BTreeMap<String, Value> = BTreeMap::new();
    for dir in app_dirs {
        let Ok(text) = fs::read_to_string(dir.join("state.json")) else {
            continue;
        };
        let Ok(store) = serde_json::from_str::<Value>(&text) else {
            warnings.push(format!("{} could not be read.", dir.join("state.json").display()));
            continue;
        };
        let config = match store.get("local-config") {
            Some(Value::String(inner)) => serde_json::from_str::<Value>(inner).unwrap_or(Value::Null),
            Some(other) => other.clone(),
            None => Value::Null,
        };
        if let Some(list) = config
            .pointer("/state/localMods")
            .and_then(Value::as_array)
        {
            for record in list {
                if let Some(id) = dmm_id(record.get("remoteId")) {
                    records.entry(id).or_insert_with(|| record.clone());
                }
            }
        }
    }

    let manifest: BTreeMap<String, Value> = game_root
        .map(|root| addons_dir(root).join(".dmm.json"))
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("mods").cloned())
        .and_then(|mods| match mods {
            Value::Object(map) => Some(map.into_iter().collect()),
            _ => None,
        })
        .unwrap_or_default();

    let ids: BTreeSet<String> = records.keys().chain(manifest.keys()).cloned().collect();
    let base = game_root.map(addons_dir);
    let mut mods = Vec::new();

    for id in ids {
        let record = records.get(&id);
        let entry = manifest.get(&id);
        let enabled = entry
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let shard = entry
            .and_then(|value| value.get("shard"))
            .and_then(Value::as_u64)
            .filter(|shard| *shard >= 1)
            .unwrap_or(1);
        let names = |key: &str| -> Vec<String> {
            entry
                .and_then(|value| value.get(key))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };

        let mut files: Vec<PathBuf> = Vec::new();
        if let Some(base) = &base {
            let shard_dir = if shard <= 1 {
                base.clone()
            } else {
                base.with_file_name(format!("addons{shard}"))
            };
            let (first, second) = if enabled {
                (names("currentVpks"), names("disabledVpks"))
            } else {
                (names("disabledVpks"), names("currentVpks"))
            };
            for name in first.iter().chain(second.iter()) {
                if !is_plain_file_name(name) {
                    continue;
                }
                for dir in [&shard_dir, base] {
                    let path = dir.join(name);
                    if path.is_file() && !files.contains(&path) {
                        files.push(path);
                        break;
                    }
                }
                if !files.is_empty() {
                    break;
                }
            }
        }
        if files.is_empty() {
            for dir in app_dirs {
                if let Some((file, _)) = dir_vpks_under(&dir.join("mods").join(&id).join("files"))
                    .into_iter()
                    .next()
                {
                    files.push(file);
                    break;
                }
            }
        }

        let field = |name: &str| {
            record
                .and_then(|value| value.get(name))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let name = field("name").unwrap_or_else(|| format!("Mod {id}"));

        let Some(file) = files.into_iter().next() else {
            warnings.push(format!("{name}: no VPK file was found on disk."));
            continue;
        };
        let thumbnail_url = field("thumbnailUrl").filter(|url| is_http(url)).or_else(|| {
            record
                .and_then(|value| value.get("images"))
                .and_then(Value::as_array)
                .and_then(|images| {
                    images
                        .iter()
                        .filter_map(Value::as_str)
                        .find(|url| is_http(url))
                        .map(str::to_string)
                })
        });

        mods.push(ForeignMod {
            name,
            author: field("author"),
            thumbnail_url,
            gamebanana_id: id.parse::<u64>().ok(),
            chunks: chunks_for(&file),
            size_bytes: file_size(&file),
            file,
            enabled_in_source: enabled,
            archive: false,
        });
    }

    DetectedSource {
        kind: SourceKind::Dmm,
        root,
        mods,
        warnings,
    }
}

/// Files the user picked: a directory VPK (its chunks are found next to it) or an archive.
pub fn local_mods(paths: &[PathBuf]) -> Vec<ForeignMod> {
    paths
        .iter()
        .filter(|path| path.is_file())
        .map(|path| {
            let name = file_name_of(path);
            let lower = name.to_ascii_lowercase();
            let archive = [".zip", ".7z", ".rar"]
                .iter()
                .any(|ext| lower.ends_with(ext));
            ForeignMod {
                name: pretty_name(&name),
                author: None,
                thumbnail_url: None,
                gamebanana_id: None,
                file: path.clone(),
                chunks: if archive { Vec::new() } else { chunks_for(path) },
                size_bytes: file_size(path),
                enabled_in_source: false,
                archive,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImportAction {
    /// Copy into this free slot as `pakNN_dir.vpk.disabled`.
    Copy { slot: u8 },
    /// Install the archive into this slot through the install pipeline, disabled.
    Archive { slot: u8 },
    /// Already in the addon folder at this slot: only the name and preview are recorded.
    Adopt { slot: u8 },
    Skip { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedImport {
    pub source: ForeignMod,
    pub action: ImportAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub kind: SourceKind,
    pub addons_dir: PathBuf,
    pub items: Vec<PlannedImport>,
}

impl ImportPlan {
    /// Items that write a file into the addon folder.
    pub fn copies(&self) -> usize {
        self.items
            .iter()
            .filter(|item| {
                matches!(
                    item.action,
                    ImportAction::Copy { .. } | ImportAction::Archive { .. }
                )
            })
            .count()
    }
}

/// The lowest free slots, one per request, in order. `None` once 99 is passed. Pure.
pub fn assign_slots(used: &BTreeSet<u8>, count: usize) -> Vec<Option<u8>> {
    let mut free = (1..=99u8).filter(|slot| !used.contains(slot));
    (0..count).map(|_| free.next()).collect()
}

/// Validate, deduplicate and slot every mod. Reads the addon folder and each VPK; writes nothing.
///
/// A mod whose directory VPK already sits in `game/citadel/addons` under a slot name is adopted
/// (metadata only). A mod whose bytes equal an installed slot is adopted for that slot. A VPK
/// that fails the payload rules, or references a chunk that is not next to it, is skipped with
/// the reason.
pub fn plan_import(kind: SourceKind, addons: &Path, mods: Vec<ForeignMod>) -> Result<ImportPlan> {
    let used = occupied_slots(addons)?;
    let installed = installed_dir_files(addons);
    let mut items = Vec::with_capacity(mods.len());
    let mut needs_slot = Vec::new();

    for source in mods {
        let action = if source.archive {
            None
        } else if let Some(slot) = slot_in_folder(&source.file, addons) {
            Some(ImportAction::Adopt { slot })
        } else if let Some(slot) = same_bytes_slot(&source, &installed) {
            Some(ImportAction::Adopt { slot })
        } else {
            check_vpk(&source).err().map(|reason| ImportAction::Skip { reason })
        };
        if action.is_none() {
            needs_slot.push(items.len());
        }
        items.push(PlannedImport {
            source,
            action: action.unwrap_or(ImportAction::Skip {
                reason: String::new(),
            }),
        });
    }

    let slots = assign_slots(&used, needs_slot.len());
    for (index, slot) in needs_slot.into_iter().zip(slots) {
        let archive = items[index].source.archive;
        items[index].action = match slot {
            Some(slot) if archive => ImportAction::Archive { slot },
            Some(slot) => ImportAction::Copy { slot },
            None => ImportAction::Skip {
                reason: "Every addon slot from 01 to 99 is taken.".to_string(),
            },
        };
    }

    Ok(ImportPlan {
        kind,
        addons_dir: addons.to_path_buf(),
        items,
    })
}

fn check_vpk(source: &ForeignMod) -> std::result::Result<(), String> {
    let directory = read_vpk_directory_with(&source.file, &ReadVpkOptions {
        keep_preload: false,
    })
    .map_err(|error| format!("Not a readable VPK: {error}"))?;
    let entries: Vec<String> = directory.entries.iter().map(|entry| entry.path.clone()).collect();
    let issues = validate_payload(&entries, &PayloadOptions::default());
    if has_errors(&issues) {
        let first = issues
            .iter()
            .find(|issue| issue.severity == crate::payload::Severity::Error)
            .map(|issue| issue.message.clone())
            .unwrap_or_default();
        return Err(format!("Fails the payload rules: {first}"));
    }
    let indexes: BTreeSet<u16> = directory
        .entries
        .iter()
        .map(|entry| entry.archive_index)
        .filter(|index| *index != VPK_DIRECTORY_ARCHIVE)
        .collect();
    for index in indexes {
        if chunk_index_path(source, index).is_none() {
            return Err(format!("Chunk {index:03} is missing next to the VPK."));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// What `apply_plan` needs besides the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyContext {
    pub game_root: PathBuf,
    /// Sigil's workspace, for archives that go through the install pipeline.
    pub workspace: PathBuf,
    /// The library store, see [`library_path`].
    pub library: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// `(slot, name)` for every mod written into the addon folder.
    pub imported: Vec<(u8, String)>,
    /// `(slot, name)` for mods already installed whose name and preview were recorded.
    pub adopted: Vec<(u8, String)>,
    /// `(name, reason)`.
    pub failed: Vec<(String, String)>,
}

/// Copy a confirmed import plan into `game/citadel/addons`. Game install write.
///
/// Call it only from an explicit user confirmation. For each `Copy` item it checks that the
/// source still has its planned size and that the slot is still free (no `pakNN_dir.vpk`, no
/// `.disabled`, no chunk), copies the chunks renamed to `pakNN_MMM.vpk`, then the directory
/// VPK as `pakNN_dir.vpk.disabled`, each through a temp file and a rename. The mod is off
/// until the user enables it. If any file of an item fails, the files written for that item
/// are removed and the next item continues. Nothing that exists is overwritten, so there is
/// nothing to back up. `Archive` items run [`crate::install::plan_install`] and
/// [`crate::install::apply_install_with`] with `land_disabled`, which validates the payload.
/// `Adopt` items write nothing to the game. Every success records its metadata in the library.
pub fn apply_plan(plan: &ImportPlan, context: &ApplyContext) -> Result<ImportReport> {
    let addons = addons_dir(&context.game_root);
    if addons != plan.addons_dir {
        return Err(Error::msg(format!(
            "The plan was made for {}, not {}. Nothing was written.",
            plan.addons_dir.display(),
            addons.display()
        )));
    }
    if !context.game_root.join("game").join("citadel").is_dir() {
        return Err(Error::msg(format!(
            "{} no longer holds game/citadel. Nothing was written.",
            context.game_root.display()
        )));
    }
    fs::create_dir_all(&addons)?;

    let mut report = ImportReport::default();
    let source_key = plan.kind.key();

    for item in &plan.items {
        let name = item.source.name.clone();
        let result = match &item.action {
            ImportAction::Skip { .. } => continue,
            ImportAction::Adopt { slot } => {
                let size = slot_dir_size(&addons, *slot);
                record(context, &item.source, *slot, size, source_key)
                    .map(|_| report.adopted.push((*slot, name.clone())))
            }
            ImportAction::Copy { slot } => copy_item(&addons, &item.source, *slot).and_then(|size| {
                record(context, &item.source, *slot, size, source_key)
                    .map(|_| report.imported.push((*slot, name.clone())))
            }),
            ImportAction::Archive { slot } => {
                install_archive(context, &item.source, *slot).and_then(|size| {
                    record(context, &item.source, *slot, size, source_key)
                        .map(|_| report.imported.push((*slot, name.clone())))
                })
            }
        };
        if let Err(error) = result {
            report.failed.push((name, error.to_string()));
        }
    }

    Ok(report)
}

fn copy_item(addons: &Path, source: &ForeignMod, slot: u8) -> Result<u64> {
    if file_size(&source.file) != source.size_bytes || !source.file.is_file() {
        return Err(Error::msg(format!(
            "{} changed since the plan. Nothing was copied.",
            source.file.display()
        )));
    }
    if occupied_slots(addons)?.contains(&slot) {
        return Err(Error::msg(format!(
            "Slot {slot:02} was taken since the plan. Nothing was copied."
        )));
    }

    let mut written: Vec<PathBuf> = Vec::new();
    let outcome = (|| -> Result<u64> {
        for chunk in &source.chunks {
            let index = chunk_index_of(chunk).ok_or_else(|| {
                Error::msg(format!("{} is not named like a VPK chunk.", chunk.display()))
            })?;
            let target = addons.join(format!("pak{slot:02}_{index:03}.vpk"));
            copy_new(chunk, &target)?;
            written.push(target);
        }
        let target = addons.join(format!("pak{slot:02}_dir.vpk{DISABLED_SUFFIX}"));
        copy_new(&source.file, &target)?;
        written.push(target.clone());
        Ok(file_size(&target))
    })();

    if outcome.is_err() {
        for path in &written {
            let _ = fs::remove_file(path);
        }
    }
    outcome
}

fn install_archive(context: &ApplyContext, source: &ForeignMod, slot: u8) -> Result<u64> {
    use crate::install::{
        ApplyOptions, InstallRequest, PlanOptions, apply_install_with, plan_install,
    };
    let request = InstallRequest::new(source.file.display().to_string(), context.workspace.clone())?
        .with_slot(Some(slot));
    let options = PlanOptions {
        game_root: Some(context.game_root.clone()),
        ..PlanOptions::default()
    };
    let plan = plan_install(&request, &options)?;
    let record = apply_install_with(&plan, true, ApplyOptions { land_disabled: true })?;
    Ok(record.entry.vpk_bytes)
}

fn record(
    context: &ApplyContext,
    source: &ForeignMod,
    slot: u8,
    size_bytes: u64,
    source_key: &str,
) -> Result<()> {
    record_meta(&context.library, ModMeta {
        slot,
        size_bytes,
        name: source.name.clone(),
        author: source.author.clone(),
        thumbnail_url: source.thumbnail_url.clone(),
        gamebanana_id: source.gamebanana_id,
        source: source_key.to_string(),
        recorded_at: now_iso(),
    })
}

/// Copy to a path that must not exist yet, through a temp file and a rename.
fn copy_new(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Err(Error::msg(format!(
            "{} already exists. Nothing was overwritten.",
            destination.display()
        )));
    }
    let temporary = destination.with_file_name(format!("{}.sigil-tmp", file_name_of(destination)));
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn is_http(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".."
}

fn dmm_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `glamorous_geist_dir.vpk` becomes `Glamorous geist`; `pak03_dir.vpk` stays `pak03_dir.vpk`.
fn pretty_name(file_name: &str) -> String {
    let lower = file_name.to_ascii_lowercase();
    if lower.starts_with("pak") && lower.len() >= 13 && lower[3..5].chars().all(|c| c.is_ascii_digit()) {
        return file_name.to_string();
    }
    let mut stem = file_name.to_string();
    for suffix in [".disabled", ".vpk", ".zip", ".7z", ".rar", "_dir"] {
        if stem.to_ascii_lowercase().ends_with(suffix) {
            stem.truncate(stem.len() - suffix.len());
        }
    }
    let spaced: String = stem
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let trimmed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => file_name.to_string(),
    }
}

/// `<stem>_dir.vpk` files in one folder (not recursive), each with its `<stem>_NNN.vpk` chunks.
fn dir_vpks_in(folder: &Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, Vec<PathBuf>)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && file_name_of(path).to_ascii_lowercase().ends_with("_dir.vpk"))
        .map(|path| {
            let chunks = chunks_for(&path);
            (path, chunks)
        })
        .collect();
    found.sort();
    found
}

/// `dir_vpks_in` one level deep as well, for download caches.
fn dir_vpks_under(folder: &Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut found = dir_vpks_in(folder);
    if let Ok(entries) = fs::read_dir(folder) {
        let mut nested: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        nested.sort();
        for dir in nested {
            found.extend(dir_vpks_in(&dir));
        }
    }
    found
}

/// Chunk files next to a directory VPK: `<stem>_dir.vpk` owns `<stem>_000.vpk` and up.
fn chunks_for(dir_file: &Path) -> Vec<PathBuf> {
    let name = file_name_of(dir_file);
    let lower = name.to_ascii_lowercase();
    let Some(stem_len) = lower.find("_dir.vpk") else {
        return Vec::new();
    };
    let stem = lower[..stem_len].to_string();
    let Some(folder) = dir_file.parent() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut chunks: Vec<(u16, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let other = file_name_of(&path).to_ascii_lowercase();
            let rest = other.strip_prefix(&stem)?.strip_prefix('_')?.strip_suffix(".vpk")?;
            if rest.len() != 3 || !rest.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            rest.parse::<u16>().ok().map(|index| (index, path))
        })
        .collect();
    chunks.sort();
    chunks.into_iter().map(|(_, path)| path).collect()
}

fn chunk_index_of(chunk: &Path) -> Option<u16> {
    let name = file_name_of(chunk).to_ascii_lowercase();
    let rest = name.strip_suffix(".vpk")?;
    let digits = rest.rsplit('_').next()?;
    (digits.len() == 3 && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

fn chunk_index_path(source: &ForeignMod, index: u16) -> Option<&PathBuf> {
    source
        .chunks
        .iter()
        .find(|chunk| chunk_index_of(chunk) == Some(index))
}

/// The slot of a file that already sits directly in the addon folder under a slot name.
fn slot_in_folder(file: &Path, addons: &Path) -> Option<u8> {
    let parent = file.parent()?;
    let same = match (fs::canonicalize(parent), fs::canonicalize(addons)) {
        (Ok(a), Ok(b)) => a == b,
        _ => parent == addons,
    };
    if !same {
        return None;
    }
    let name = file_name_of(file).to_ascii_lowercase();
    let rest = name.strip_prefix("pak")?;
    let slot: u8 = rest.get(..2)?.parse().ok()?;
    let tail = rest.get(2..)?;
    ((1..=99).contains(&slot) && (tail == "_dir.vpk" || tail == "_dir.vpk.disabled")).then_some(slot)
}

/// `(slot, path, size)` for every directory VPK in the addon folder, either name.
fn installed_dir_files(addons: &Path) -> Vec<(u8, PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(addons) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let slot = slot_in_folder(&path, addons)?;
            let size = file_size(&path);
            Some((slot, path, size))
        })
        .collect()
}

/// An installed slot whose directory VPK has the same bytes as `source`.
fn same_bytes_slot(source: &ForeignMod, installed: &[(u8, PathBuf, u64)]) -> Option<u8> {
    let candidates: Vec<&(u8, PathBuf, u64)> = installed
        .iter()
        .filter(|(_, _, size)| *size == source.size_bytes && *size > 0)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let wanted = fs::read(&source.file).ok()?;
    candidates
        .into_iter()
        .find(|(_, path, _)| fs::read(path).map(|bytes| bytes == wanted).unwrap_or(false))
        .map(|(slot, _, _)| *slot)
}

fn slot_dir_size(addons: &Path, slot: u8) -> u64 {
    let enabled = addons.join(format!("pak{slot:02}_dir.vpk"));
    if enabled.is_file() {
        return file_size(&enabled);
    }
    file_size(&addons.join(format!("pak{slot:02}_dir.vpk{DISABLED_SUFFIX}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("sigil-import-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn fake_game(root: &Path) -> PathBuf {
        let game = root.join("Deadlock");
        fs::create_dir_all(game.join("game").join("citadel").join("addons")).unwrap();
        game
    }

    /// A real directory VPK through the crate writer, renamed to `name` in `folder`.
    fn make_vpk(folder: &Path, name: &str, paths: &[(&str, &[u8])]) -> PathBuf {
        let work = folder.join(format!(".work-{name}"));
        let staging = work.join("staging");
        for (path, data) in paths {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        let packed = pack_vpk(&PackOptions::new(staging, work.join("out"), "pak01")).unwrap();
        fs::create_dir_all(folder).unwrap();
        let target = folder.join(name);
        fs::copy(&packed.dir_path, &target).unwrap();
        let stem = name.trim_end_matches("dir.vpk");
        for chunk in &packed.chunk_paths {
            let index = chunk_index_of(chunk).unwrap();
            fs::copy(chunk, folder.join(format!("{stem}{index:03}.vpk"))).unwrap();
        }
        let _ = fs::remove_dir_all(&work);
        target
    }

    #[test]
    fn assigns_the_lowest_free_slots_in_order() {
        let used: BTreeSet<u8> = [1, 2, 4].into_iter().collect();
        assert_eq!(assign_slots(&used, 3), vec![Some(3), Some(5), Some(6)]);
        let full: BTreeSet<u8> = (1..=98).collect();
        assert_eq!(assign_slots(&full, 2), vec![Some(99), None]);
    }

    #[test]
    fn library_round_trips_and_matches_on_slot_and_size() {
        let root = scratch("library");
        let path = library_path(&root);
        assert_eq!(read_library(&path).unwrap(), Library::default());

        let meta = ModMeta {
            slot: 3,
            size_bytes: 120,
            name: "Violet recolor".to_string(),
            thumbnail_url: Some("https://images.gamebanana.com/x.jpg".to_string()),
            gamebanana_id: Some(715100),
            source: "browse".to_string(),
            ..ModMeta::default()
        };
        record_meta(&path, meta.clone()).unwrap();
        record_meta(&path, ModMeta { name: "Replaced".to_string(), ..meta.clone() }).unwrap();

        let library = read_library(&path).unwrap();
        assert_eq!(library.mods.len(), 1);
        assert_eq!(library.find(3, 120).unwrap().name, "Replaced");
        assert!(library.find(3, 121).is_none());
        assert!(library.find(4, 120).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_grimoire_and_lists_its_parked_and_overflow_mods() {
        let root = scratch("grimoire");
        let game = fake_game(&root);
        let citadel = game.join("game").join("citadel");
        let config = root.join("config");
        let user_data = config.join(GRIMOIRE_DIR_NAME);
        fs::create_dir_all(&user_data).unwrap();
        fs::write(
            user_data.join("mod-metadata.json"),
            r#"{
                "pak01_dir.vpk": {"modName": "Enabled one", "gameBananaId": 11},
                "glamorous_geist_dir.vpk": {"modName": "Glamorous Geist", "author": "ash",
                    "thumbnailUrl": "https://images.gamebanana.com/g.jpg", "gameBananaId": 22},
                "addons2/pak01_dir.vpk": {"modName": "Overflow"}
            }"#,
        )
        .unwrap();
        let payload: &[(&str, &[u8])] = &[("materials/a.vmat_c", b"a")];
        make_vpk(&citadel.join("addons"), "pak01_dir.vpk", payload);
        make_vpk(&citadel.join("addons").join(".disabled"), "glamorous_geist_dir.vpk", &[("materials/b.vmat_c", b"b")]);
        make_vpk(&citadel.join("addons2"), "pak01_dir.vpk", &[("materials/c.vmat_c", b"c")]);

        let roots = DetectRoots { config: vec![config], data: Vec::new() };
        let sources = detect_sources(&roots, Some(&game));
        assert_eq!(sources.len(), 1);
        let source = &sources[0];
        assert_eq!(source.kind, SourceKind::Grimoire);
        let names: Vec<&str> = source.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Enabled one", "Glamorous Geist", "Overflow"]);
        let geist = &source.mods[1];
        assert_eq!(geist.author.as_deref(), Some("ash"));
        assert_eq!(geist.gamebanana_id, Some(22));
        assert!(!geist.enabled_in_source);
        assert_eq!(geist.chunks.len(), 1);

        // The enabled one is already in addons: adopted, never copied.
        let addons = addons_dir(&game);
        let plan = plan_import(SourceKind::Grimoire, &addons, source.mods.clone()).unwrap();
        assert_eq!(plan.items[0].action, ImportAction::Adopt { slot: 1 });
        assert_eq!(plan.items[1].action, ImportAction::Copy { slot: 2 });
        assert_eq!(plan.items[2].action, ImportAction::Copy { slot: 3 });
        assert_eq!(plan.copies(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_dmm_from_state_and_manifest() {
        let root = scratch("dmm");
        let game = fake_game(&root);
        let addons = addons_dir(&game);
        let data = root.join("data");
        let app = data.join(DMM_IDENTIFIER);
        fs::create_dir_all(&app).unwrap();
        let inner = serde_json::json!({
            "state": {"localMods": [
                {"remoteId": "715100", "name": "Asa Mitaka over Paige", "author": "toast",
                 "images": ["https://images.gamebanana.com/a.jpg"]},
                {"remoteId": "local-0f8fad5b-d9cb-469f-a165-70867728950e", "name": "My local"}
            ]},
            "version": 3
        });
        fs::write(
            app.join("state.json"),
            serde_json::json!({"local-config": inner.to_string()}).to_string(),
        )
        .unwrap();
        make_vpk(&addons, "715100_pak01_dir.vpk", &[("materials/a.vmat_c", b"a")]);
        fs::write(
            addons.join(".dmm.json"),
            r#"{"version":3,"mods":{
                "715100":{"enabled":false,"shard":1,"currentVpks":[],"disabledVpks":["715100_pak01_dir.vpk"]},
                "local-0f8fad5b-d9cb-469f-a165-70867728950e":{"enabled":false,"currentVpks":[],"disabledVpks":["gone.vpk"]}
            }}"#,
        )
        .unwrap();

        let roots = DetectRoots { config: Vec::new(), data: vec![data] };
        let sources = detect_sources(&roots, Some(&game));
        assert_eq!(sources.len(), 1);
        let source = &sources[0];
        assert_eq!(source.kind, SourceKind::Dmm);
        assert_eq!(source.mods.len(), 1, "{:?}", source.warnings);
        let asa = &source.mods[0];
        assert_eq!(asa.name, "Asa Mitaka over Paige");
        assert_eq!(asa.gamebanana_id, Some(715100));
        assert_eq!(asa.thumbnail_url.as_deref(), Some("https://images.gamebanana.com/a.jpg"));
        assert_eq!(asa.chunks.len(), 1);
        assert!(source.warnings.iter().any(|w| w.contains("My local")));

        // The parked file is in addons but not under a slot name: it is copied.
        let plan = plan_import(SourceKind::Dmm, &addons, source.mods.clone()).unwrap();
        assert_eq!(plan.items[0].action, ImportAction::Copy { slot: 1 });
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_is_detected_on_a_clean_machine() {
        let root = scratch("clean");
        let game = fake_game(&root);
        let roots = DetectRoots { config: vec![root.join("c")], data: vec![root.join("d")] };
        assert!(detect_sources(&roots, Some(&game)).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_skips_denied_payloads_and_missing_chunks() {
        let root = scratch("plan-skip");
        let game = fake_game(&root);
        let addons = addons_dir(&game);
        let picked = root.join("picked");
        let bad = make_vpk(&picked, "bad_dir.vpk", &[("scripts/abilities.vdata_c", b"x")]);
        let chunked = make_vpk(&picked, "chunked_dir.vpk", &[("materials/a.vmat_c", b"a")]);
        fs::remove_file(picked.join("chunked_000.vpk")).unwrap();

        let plan = plan_import(SourceKind::Local, &addons, local_mods(&[bad, chunked])).unwrap();
        assert!(matches!(&plan.items[0].action, ImportAction::Skip { reason } if reason.contains("payload")));
        assert!(matches!(&plan.items[1].action, ImportAction::Skip { reason } if reason.contains("Chunk 000")));
        assert_eq!(plan.copies(), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_copies_disabled_into_free_slots_and_records_metadata() {
        let root = scratch("apply");
        let game = fake_game(&root);
        let addons = addons_dir(&game);
        // Slot 01 is the user's own disabled mod; it must stay untouched.
        fs::write(addons.join("pak01_dir.vpk.disabled"), b"mine").unwrap();
        let picked = root.join("picked");
        let one = make_vpk(&picked, "violet_dir.vpk", &[("materials/a.vmat_c", b"a")]);
        let same_as_installed = make_vpk(&picked, "copy_dir.vpk", &[("materials/z.vmat_c", b"z")]);
        fs::copy(&same_as_installed, addons.join("pak05_dir.vpk")).unwrap();

        let mut mods = local_mods(&[one, same_as_installed]);
        mods[0].thumbnail_url = Some("https://images.gamebanana.com/v.jpg".to_string());
        let plan = plan_import(SourceKind::Local, &addons, mods).unwrap();
        assert_eq!(plan.items[0].action, ImportAction::Copy { slot: 2 });
        assert_eq!(plan.items[1].action, ImportAction::Adopt { slot: 5 });

        let context = ApplyContext {
            game_root: game.clone(),
            workspace: root.join("workspace"),
            library: library_path(&root.join("app")),
        };
        let report = apply_plan(&plan, &context).unwrap();
        assert_eq!(report.imported, vec![(2, "Violet".to_string())]);
        assert_eq!(report.adopted, vec![(5, "Copy".to_string())]);
        assert!(report.failed.is_empty());

        assert!(addons.join("pak02_dir.vpk.disabled").is_file());
        assert!(!addons.join("pak02_dir.vpk").exists());
        assert!(addons.join("pak02_000.vpk").is_file());
        assert_eq!(fs::read(addons.join("pak01_dir.vpk.disabled")).unwrap(), b"mine");
        assert!(fs::read_dir(&addons).unwrap().flatten().all(|e| !file_name_of(&e.path()).ends_with(".sigil-tmp")));

        // The slot shows up as an off addon, and the switch turns it on.
        let listed = crate::addons::list_installed(&game).unwrap();
        let two = listed.iter().find(|addon| addon.slot == 2).unwrap();
        assert!(!two.enabled);
        assert_eq!(two.entry_count, Some(1));
        crate::addons::set_enabled(&game, 2, true, true).unwrap();

        let library = read_library(&context.library).unwrap();
        let size = file_size(&addons.join("pak02_dir.vpk"));
        let meta = library.find(2, size).unwrap();
        assert_eq!(meta.name, "Violet");
        assert_eq!(meta.source, "local");
        assert!(library.find(5, file_size(&addons.join("pak05_dir.vpk"))).is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_refuses_a_slot_taken_after_planning_and_leaves_no_partial_files() {
        let root = scratch("apply-race");
        let game = fake_game(&root);
        let addons = addons_dir(&game);
        let picked = root.join("picked");
        let one = make_vpk(&picked, "late_dir.vpk", &[("materials/a.vmat_c", b"a")]);
        let plan = plan_import(SourceKind::Local, &addons, local_mods(&[one])).unwrap();
        assert_eq!(plan.items[0].action, ImportAction::Copy { slot: 1 });

        // Something else lands in slot 01 between plan and apply.
        fs::write(addons.join("pak01_dir.vpk"), b"someone else").unwrap();
        let context = ApplyContext {
            game_root: game.clone(),
            workspace: root.join("workspace"),
            library: library_path(&root.join("app")),
        };
        let report = apply_plan(&plan, &context).unwrap();
        assert!(report.imported.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(fs::read(addons.join("pak01_dir.vpk")).unwrap(), b"someone else");
        assert!(!addons.join("pak01_000.vpk").exists());
        assert!(!addons.join("pak01_dir.vpk.disabled").exists());

        // A plan for another game folder is refused outright.
        let other = ApplyContext { game_root: root.join("Elsewhere"), ..context };
        assert!(apply_plan(&plan, &other).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pretty_names_from_file_names() {
        assert_eq!(pretty_name("glamorous_geist_dir.vpk"), "Glamorous geist");
        assert_eq!(pretty_name("pak03_dir.vpk"), "pak03_dir.vpk");
        assert_eq!(pretty_name("cool-mod.zip"), "Cool mod");
    }
}

#[cfg(test)]
mod thumbnail_tests {
    use super::*;

    #[test]
    fn a_late_preview_only_fills_the_same_mod_in_its_slot() {
        let dir = std::env::temp_dir().join(format!("sigil-thumb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.json");
        let meta = |slot: u8, size: u64, id: u64| ModMeta {
            slot,
            size_bytes: size,
            name: "A mod".into(),
            author: None,
            thumbnail_url: None,
            gamebanana_id: Some(id),
            source: "browse".into(),
            recorded_at: String::new(),
        };
        record_meta(&path, meta(3, 100, 42)).unwrap();
        // Another mod took slot 3 since the lookup started: nothing is written.
        assert!(!set_thumbnail(&path, 3, 100, 7, "https://images.gamebanana.com/a.jpg").unwrap());
        assert!(!set_thumbnail(&path, 3, 999, 42, "https://images.gamebanana.com/a.jpg").unwrap());
        // The same mod: filled once, then left alone.
        assert!(set_thumbnail(&path, 3, 100, 42, "https://images.gamebanana.com/a.jpg").unwrap());
        assert!(!set_thumbnail(&path, 3, 100, 42, "https://images.gamebanana.com/b.jpg").unwrap());
        let library = read_library(&path).unwrap();
        assert_eq!(library.mods[0].thumbnail_url.as_deref(), Some("https://images.gamebanana.com/a.jpg"));
        let _ = fs::remove_dir_all(&dir);
    }
}
