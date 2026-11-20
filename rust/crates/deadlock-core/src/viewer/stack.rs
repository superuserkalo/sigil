//! The VPK stack a preview reads, in game precedence order, and a resolver over it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::{Variant, citadel_dir};
use crate::addons;
use crate::error::{Error, Result};
use crate::import::Library;
use crate::vpk::{VpkDirectory, chunk_path_for, read_vpk_directory, read_vpk_entry_data};

/// Where a VPK in the stack comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// `citadel/grimoire`, the Global priority folder, when `gameinfo.gi` mounts it.
    Global,
    /// An addon slot in `citadel/addons` or `citadel/sigil-addons2..10`.
    Addon,
    /// A map VPK (`citadel/maps/dl_hideout.vpk`).
    Map,
    /// `citadel/pak01_dir.vpk`.
    Base,
    /// `core/pak01_dir.vpk`.
    Core,
}

impl SourceKind {
    /// Mods are the layers a player installs. Everything else ships with the game.
    pub fn is_mod(self) -> bool {
        matches!(self, SourceKind::Global | SourceKind::Addon)
    }
}

/// One VPK in a preview's stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub kind: SourceKind,
    /// Global addon slot (1 to 990) for an addon, else `None`.
    pub slot: Option<u16>,
    /// Absolute path of the directory VPK (`pakNN_dir.vpk`, or `.disabled` when switched off).
    pub path: PathBuf,
    /// Path relative to `game/citadel` with forward slashes, for example
    /// `addons/pak03_dir.vpk` or `pak01_dir.vpk`. `core/pak01_dir.vpk` for core.
    pub file: String,
    /// Display name: the mod's library name when known, else the file name.
    pub name: String,
    /// False for a disabled slot included only to preview it.
    pub enabled: bool,
}

/// One source as a sidecar reports it, with how many files the export read from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUsage {
    pub slot: Option<u16>,
    pub file: String,
    pub name: String,
    pub kind: SourceKind,
    pub enabled: bool,
    /// Files this export read from this VPK. A mod at 0 changed nothing that is shown.
    pub used: u32,
}

/// The VPK stack for a preview, highest precedence first: the Global folder (Modded only),
/// enabled addon slots by load order, then `citadel/pak01_dir.vpk` and `core/pak01_dir.vpk`.
///
/// `include_disabled_slots` adds switched-off slots at the position they would load from, so a
/// player can preview a mod before turning it on. It is ignored for [`Variant::Original`].
/// Missing folders are skipped rather than reported: an empty addons folder is a stack of two.
pub fn sources(
    game_root: &Path,
    variant: Variant,
    include_disabled_slots: Option<&[u16]>,
) -> Vec<Source> {
    let citadel = citadel_dir(game_root);
    let mut out = Vec::new();

    if variant == Variant::Modded {
        out.extend(global_sources(game_root, &citadel));
        let addons_dir = crate::deploy::addons_dir(game_root);
        let extra = include_disabled_slots.unwrap_or(&[]);
        if let Ok(installed) = addons::list_installed(game_root) {
            // list_installed is sorted by slot: the order the game loads them in.
            for addon in installed {
                if !addon.enabled && !extra.contains(&addon.slot) {
                    continue;
                }
                let path = addon.path(&addons_dir);
                out.push(Source {
                    kind: SourceKind::Addon,
                    slot: Some(addon.slot),
                    file: relative_file(&citadel, &path),
                    name: addon.file.clone(),
                    path,
                    enabled: addon.enabled,
                });
            }
        }
    }

    let base = citadel.join("pak01_dir.vpk");
    if base.is_file() {
        out.push(Source {
            kind: SourceKind::Base,
            slot: None,
            file: "pak01_dir.vpk".into(),
            name: "Deadlock".into(),
            path: base,
            enabled: true,
        });
    }
    let core = game_root.join("game").join("core").join("pak01_dir.vpk");
    if core.is_file() {
        out.push(Source {
            kind: SourceKind::Core,
            slot: None,
            file: "core/pak01_dir.vpk".into(),
            name: "Deadlock core".into(),
            path: core,
            enabled: true,
        });
    }
    out
}

/// Fill in addon names from Sigil's library (`<data>/library.json`), when it knows the slot.
pub fn name_sources(sources: &mut [Source], library: &Library) {
    for source in sources.iter_mut() {
        let Some(slot) = source.slot else { continue };
        let size = std::fs::metadata(&source.path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        if let Some(meta) = library.find(slot, size) {
            source.name = meta.name.clone();
        }
    }
}

fn global_sources(game_root: &Path, citadel: &Path) -> Vec<Source> {
    let gameinfo = crate::deploy::gameinfo_path(game_root);
    let configured = crate::locker::read_global_path(&gameinfo)
        .map(|status| status.configured && status.folder)
        .unwrap_or(false);
    if !configured {
        return Vec::new();
    }
    let folder = crate::locker::global_dir(game_root);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut files: Vec<(u16, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let number = name.strip_prefix("pak")?.strip_suffix("_dir.vpk")?;
            Some((number.parse().ok()?, entry.path()))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|(_, path)| Source {
            kind: SourceKind::Global,
            slot: None,
            file: relative_file(citadel, &path),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path,
            enabled: true,
        })
        .collect()
}

fn relative_file(citadel: &Path, path: &Path) -> String {
    path.strip_prefix(citadel)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Size and modification time, the change detector for cache keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Stamp {
    pub size: u64,
    pub mtime_ns: u128,
}

pub(crate) fn stamp(path: &Path) -> Result<Stamp> {
    let meta = std::fs::metadata(path)?;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    Ok(Stamp {
        size: meta.len(),
        mtime_ns,
    })
}

/// Key material for one source: its kind, path and stamp, plus its chunk files for a mod.
/// The base VPKs are keyed on their directory file alone: a game update rewrites it.
pub(crate) fn source_fingerprint(source: &Source) -> String {
    let mut parts = vec![
        format!("{:?}", source.kind),
        source.path.to_string_lossy().into_owned(),
    ];
    match stamp(&source.path) {
        Ok(found) => parts.push(format!("{}:{}", found.size, found.mtime_ns)),
        Err(_) => parts.push("missing".into()),
    }
    if source.kind.is_mod() {
        for index in 0..1000u16 {
            let chunk = chunk_path_for(&source.path, index);
            let Ok(found) = stamp(&chunk) else { break };
            parts.push(format!("{index}:{}:{}", found.size, found.mtime_ns));
        }
    }
    parts.join("|")
}

/// Parsed directory trees, shared across exports while the file is unchanged. Opening
/// `pak01_dir.vpk` is most of a small export's cost; this keeps it to once per process.
pub(crate) fn open_directory(path: &Path) -> Result<Arc<VpkDirectory>> {
    type Directories = HashMap<PathBuf, (Stamp, Arc<VpkDirectory>)>;
    static CACHE: OnceLock<Mutex<Directories>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let current = stamp(path)?;
    if let Some((cached_stamp, directory)) = cache.lock().map_err(poisoned)?.get(path)
        && *cached_stamp == current
    {
        return Ok(directory.clone());
    }
    let directory = Arc::new(read_vpk_directory(path)?);
    let mut guard = cache.lock().map_err(poisoned)?;
    if guard.len() >= 48 {
        guard.clear();
    }
    guard.insert(path.to_path_buf(), (current, directory.clone()));
    Ok(directory)
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::msg("viewer VPK cache lock was poisoned")
}

/// One open VPK in a stack.
pub(crate) struct Layer {
    pub source: Source,
    pub directory: Arc<VpkDirectory>,
    used: AtomicU32,
}

impl Layer {
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.directory.by_path.keys().map(String::as_str)
    }
}

/// Open VPKs in precedence order. Reads resolve to the first layer that has the path, as the
/// engine's search paths do. Every read is counted per layer, and optionally recorded, so an
/// export can report which mods it actually used.
pub(crate) struct Stack {
    pub layers: Vec<Layer>,
    recorded: Option<Mutex<BTreeSet<String>>>,
}

impl Stack {
    pub fn open(sources: &[Source]) -> Result<Stack> {
        let mut layers = Vec::with_capacity(sources.len());
        for source in sources {
            let directory = open_directory(&source.path).map_err(|error| {
                Error::msg(format!("opening {}: {error}", source.path.display()))
            })?;
            layers.push(Layer {
                source: source.clone(),
                directory,
                used: AtomicU32::new(0),
            });
        }
        Ok(Stack {
            layers,
            recorded: None,
        })
    }

    /// Remember every path read from now on ([`Stack::recorded`]).
    pub fn record(mut self) -> Self {
        self.recorded = Some(Mutex::new(BTreeSet::new()));
        self
    }

    pub fn recorded(&self) -> BTreeSet<String> {
        self.recorded
            .as_ref()
            .and_then(|set| set.lock().ok().map(|guard| guard.clone()))
            .unwrap_or_default()
    }

    /// The bytes of `path` from the first layer at or after `start` that has it, and that layer.
    pub fn read_from(&self, start: usize, path: &str) -> Option<(Vec<u8>, usize)> {
        let lower = normalize(path);
        if let Some(set) = &self.recorded
            && let Ok(mut guard) = set.lock()
        {
            guard.insert(lower.clone());
        }
        for (index, layer) in self.layers.iter().enumerate().skip(start) {
            let Some(entry) = layer.directory.by_path.get(&lower) else {
                continue;
            };
            if let Ok(bytes) = read_vpk_entry_data(&layer.directory, entry) {
                layer.used.fetch_add(1, Ordering::Relaxed);
                return Some((bytes, index));
            }
        }
        None
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.read_from(0, path).map(|(bytes, _)| bytes)
    }

    /// Index of the first layer that ships with the game (map, base or core).
    pub fn first_game_layer(&self) -> usize {
        self.layers
            .iter()
            .position(|layer| !layer.source.kind.is_mod())
            .unwrap_or(self.layers.len())
    }

    /// Every path under `prefix` ending in `suffix`, with the layer that wins it, sorted.
    pub fn list(&self, prefix: &str, suffix: &str) -> BTreeMap<String, usize> {
        let prefix = normalize(prefix);
        let mut out = BTreeMap::new();
        for (index, layer) in self.layers.iter().enumerate() {
            for path in layer.paths() {
                if path.starts_with(&prefix) && path.ends_with(suffix) {
                    out.entry(path.to_string()).or_insert(index);
                }
            }
        }
        out
    }

    pub fn usage(&self) -> Vec<SourceUsage> {
        self.layers
            .iter()
            .map(|layer| SourceUsage {
                slot: layer.source.slot,
                file: layer.source.file.clone(),
                name: layer.source.name.clone(),
                kind: layer.source.kind,
                enabled: layer.source.enabled,
                used: layer.used.load(Ordering::Relaxed),
            })
            .collect()
    }
}

impl morphic::model::FileResolver for Stack {
    fn resolve(&self, compiled_path: &str) -> Option<Vec<u8>> {
        self.read(compiled_path)
    }
}

/// VPK paths are compared lowercased with forward slashes.
pub(crate) fn normalize(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// A compiled resource path: `foo.vmdl` becomes `foo.vmdl_c`; `foo.vmdl_c` is unchanged.
pub(crate) fn compiled(path: &str) -> String {
    let path = normalize(path);
    if path.ends_with("_c") {
        path
    } else {
        format!("{path}_c")
    }
}
