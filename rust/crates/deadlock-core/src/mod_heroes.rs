//! Which heroes a mod changes, read from the model and material paths in its VPK.
//!
//! A mod replaces a game file by exact path, so a skin's own folders name the hero it is
//! for: `models/heroes_staging/inferno_v4/inferno.vmdl_c` is Infernus and
//! `materials/models/heroes_wip/haze/haze_body.vmat_c` is Haze. The Locker already maps one
//! path to a roster hero through [`crate::locker`]'s internal `hero_of_path`. This module asks
//! that question of a whole mod and returns the sorted display names, so the hero conflict
//! warning can tell that two skins replace the same hero even when neither carries a
//! GameBanana tag.
//!
//! Only `models/` and `materials/models/` count. Sounds, particles and UI art can belong to
//! one hero and still are not a model: two sound packs for Infernus do not replace the same
//! body, and a HUD pack is not a skin. The gate matches Deadlock Mod Manager: strip a leading
//! `materials/`, then require `models/heroes*`. Every extension counts, compiled (`_c`) and
//! source alike, because a source name in a mod still says which hero its author worked on.
//!
//! Read only. Nothing here writes to a game install or contacts the network. [`HeroCache`]
//! keeps the answers in a JSON file in the app data folder, keyed by the VPK's path, size and
//! modification time, so a launch reads only the mods that changed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::locker::hero_of_path;
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Roster display names of the heroes whose model or material files these game paths replace,
/// sorted and without duplicates. Sounds, particles and UI alone do not count: two sound mods
/// for one hero do not replace the same model.
pub fn model_heroes<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut heroes = BTreeSet::new();

    for path in paths {
        // VPK paths are stored with forward slashes, but a caller may hand over a path from
        // another source (a loose file, a project manifest) that kept the platform separator.
        let lowered = path.to_ascii_lowercase().replace('\\', "/");

        if !is_model_path(&lowered) {
            continue;
        }

        if let Some(hero) = hero_of_path(&lowered) {
            heroes.insert(hero.name.to_string());
        }
    }

    heroes.into_iter().collect()
}

/// [`model_heroes`] over every file in the VPK at `path`.
pub fn model_heroes_in_vpk(path: &Path) -> crate::Result<Vec<String>> {
    // The hero answer needs the directory tree only, so skip the preload bytes.
    let directory = read_vpk_directory_with(
        path,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;

    Ok(model_heroes(
        directory.entries.iter().map(|entry| entry.path.as_str()),
    ))
}

/// The cache file name, for `<data_dir>/mod-heroes.json`.
pub const HERO_CACHE_FILE: &str = "mod-heroes.json";
/// Bumped when the cache layout or the hero rule changes.
pub const HERO_CACHE_SCHEMA: u32 = 1;

/// The cached answer for one VPK, valid while its size and modification time hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedHeroes {
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub modified_nanos: u128,
    pub heroes: Vec<String>,
}

/// [`model_heroes_in_vpk`] answers kept across launches. The roster that maps folders to heroes
/// ships with the app, so a cache written by another app version is thrown away on read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroCache {
    pub schema: u32,
    /// The app version that wrote the cache.
    pub version: String,
    /// Keyed by the VPK's full path.
    pub entries: BTreeMap<String, CachedHeroes>,
    #[serde(skip)]
    dirty: bool,
}

impl Default for HeroCache {
    fn default() -> Self {
        HeroCache {
            schema: HERO_CACHE_SCHEMA,
            version: env!("CARGO_PKG_VERSION").to_string(),
            entries: BTreeMap::new(),
            dirty: false,
        }
    }
}

fn stamp_of(path: &Path) -> crate::Result<(u64, u128)> {
    let meta = std::fs::metadata(path)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok((meta.len(), modified))
}

impl HeroCache {
    /// Read `path`; a missing, unreadable or stale file is an empty cache.
    pub fn read(path: &Path) -> HeroCache {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HeroCache>(&bytes).ok())
            .filter(|cache| {
                cache.schema == HERO_CACHE_SCHEMA && cache.version == env!("CARGO_PKG_VERSION")
            })
            .unwrap_or_default()
    }

    /// Write the cache when it changed since it was read. Returns whether it wrote.
    pub fn save(&mut self, path: &Path) -> crate::Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::util::write_atomic(path, &serde_json::to_vec(self)?)?;
        self.dirty = false;
        Ok(true)
    }

    /// The heroes of the VPK at `vpk`: from the cache when its size and modification time
    /// match, else read from the file and remembered.
    pub fn heroes(&mut self, vpk: &Path) -> crate::Result<Vec<String>> {
        let (size, modified_nanos) = stamp_of(vpk)?;
        let key = vpk.to_string_lossy().into_owned();
        if let Some(hit) = self.entries.get(&key)
            && hit.size == size
            && hit.modified_nanos == modified_nanos
        {
            return Ok(hit.heroes.clone());
        }
        let heroes = model_heroes_in_vpk(vpk)?;
        self.entries.insert(key, CachedHeroes { size, modified_nanos, heroes: heroes.clone() });
        self.dirty = true;
        Ok(heroes)
    }

    /// Drop entries whose file is gone, so the cache does not grow with every mod ever seen.
    pub fn prune_missing(&mut self) {
        let before = self.entries.len();
        self.entries.retain(|path, _| Path::new(path).is_file());
        self.dirty |= self.entries.len() != before;
    }
}

/// The heroes of each VPK in `vpks`, through the cache file at `cache_path` (read, filled,
/// pruned of files that are gone, saved). Files that cannot be read are left out. The one
/// call the Installed screen needs at launch in place of [`model_heroes_in_vpk`] per mod.
pub fn model_heroes_cached(cache_path: &Path, vpks: &[PathBuf]) -> BTreeMap<PathBuf, Vec<String>> {
    let mut cache = HeroCache::read(cache_path);
    let out = vpks
        .iter()
        .filter_map(|vpk| cache.heroes(vpk).ok().map(|heroes| (vpk.clone(), heroes)))
        .collect();
    cache.prune_missing();
    let _ = cache.save(cache_path);
    out
}

/// True for a path under `models/heroes*` or `materials/models/heroes*`.
///
/// This is the folder pair Deadlock Mod Manager reads a hero from. Requiring `models` keeps a
/// material anyway: `materials/models/heroes_wip/haze/...` loses its leading `materials/` and
/// then looks like a model path.
fn is_model_path(path_lower: &str) -> bool {
    let without_materials = path_lower.strip_prefix("materials/").unwrap_or(path_lower);

    without_materials.starts_with("models/heroes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use crate::vpk_write::{PackOptions, pack_vpk};

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dah-mod-heroes-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let staging = root.join(format!("staging-{name}"));

        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }

        let packed = pack_vpk(&PackOptions::new(staging, root.join("out"), name)).unwrap();
        packed.dir_path
    }

    #[test]
    fn a_model_or_material_path_names_its_hero() {
        assert_eq!(
            model_heroes(["models/heroes_staging/inferno_v4/inferno.vmdl_c"]),
            vec!["Infernus"]
        );
        assert_eq!(
            model_heroes(["materials/models/heroes_wip/haze/haze_body.vmat_c"]),
            vec!["Haze"]
        );
        // Both a compiled and a source name count.
        assert_eq!(
            model_heroes(["models/heroes_wip/billy/billy.vmdl"]),
            vec!["Billy"]
        );
        assert!(model_heroes([]).is_empty());
    }

    #[test]
    fn two_heroes_come_back_sorted_and_without_duplicates() {
        let heroes = model_heroes([
            "models/heroes_staging/inferno_v4/inferno.vmdl_c",
            "materials/models/heroes_wip/haze/haze_body.vmat_c",
            "models/heroes_staging/inferno_v4/inferno_skin.vmdl_c",
            "materials/models/heroes_staging/inferno_v4/materials/inferno_body.vmat_c",
        ]);

        assert_eq!(heroes, vec!["Haze", "Infernus"]);
    }

    #[test]
    fn sounds_particles_and_ui_alone_name_no_hero() {
        assert!(
            model_heroes([
                "sounds/abilities/inferno/a1_fire.vsnd_c",
                "sounds/heroes/haze/vo_line.vsnd_c",
                "particles/heroes/inferno/fire.vpcf_c",
                "panorama/images/heroes/inferno_card.vtex_c",
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_bare_file_in_the_heroes_folder_names_no_hero() {
        assert!(model_heroes(["models/heroes/inferno.vmdl_c"]).is_empty());
        assert!(model_heroes(["materials/models/heroes/haze.vmat_c"]).is_empty());
    }

    #[test]
    fn backslashes_and_upper_case_work() {
        let heroes = model_heroes([
            "Models\\Heroes_Staging\\Inferno_V4\\Inferno.VMDL_C",
            "MATERIALS\\MODELS\\HEROES_WIP\\HAZE\\HAZE.VMAT_C",
        ]);

        assert_eq!(heroes, vec!["Haze", "Infernus"]);
    }

    #[test]
    fn reads_every_file_in_a_packed_vpk() {
        let root = temp_root("vpk");
        let path = vpk(
            &root,
            "pak01",
            &[
                (
                    "models/heroes_staging/inferno_v4/inferno.vmdl_c",
                    b"infernus",
                ),
                ("materials/models/heroes_wip/haze/haze_body.vmat_c", b"haze"),
                ("sounds/abilities/inferno/a1_fire.vsnd_c", b"sound"),
                ("particles/heroes/haze/fire.vpcf_c", b"particle"),
            ],
        );

        assert_eq!(
            model_heroes_in_vpk(&path).unwrap(),
            vec!["Haze", "Infernus"]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_vpk_is_an_error_not_a_panic() {
        let root = temp_root("missing");
        let missing = root.join("pak42_dir.vpk");

        assert!(model_heroes_in_vpk(&missing).is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cache_answers_until_the_file_changes() {
        let root = temp_root("cache");
        let staging = root.join("staging");
        let model = staging.join("models/heroes_wip/haze/haze.vmdl_c");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&model, b"m").unwrap();
        let vpk = pack_vpk(&PackOptions::new(staging, root.join("out"), "pak01")).unwrap().dir_path;
        let cache_path = root.join("data").join(HERO_CACHE_FILE);

        let first = model_heroes_cached(&cache_path, std::slice::from_ref(&vpk));
        assert_eq!(first[&vpk], vec!["Haze".to_string()]);
        assert!(cache_path.is_file());

        // A cached answer is served without reading the VPK: plant a fake answer to prove it.
        let mut cache = HeroCache::read(&cache_path);
        cache.entries.get_mut(&vpk.to_string_lossy().into_owned()).unwrap().heroes = vec!["Cached".into()];
        cache.dirty = true;
        cache.save(&cache_path).unwrap();
        assert_eq!(model_heroes_cached(&cache_path, std::slice::from_ref(&vpk))[&vpk], vec!["Cached".to_string()]);

        // A changed size invalidates the entry.
        let mut bytes = fs::read(&vpk).unwrap();
        bytes.push(0);
        fs::write(&vpk, bytes).unwrap();
        assert_eq!(model_heroes_cached(&cache_path, std::slice::from_ref(&vpk))[&vpk], vec!["Haze".to_string()]);

        // A missing file is left out and pruned.
        fs::remove_file(&vpk).unwrap();
        assert!(model_heroes_cached(&cache_path, std::slice::from_ref(&vpk)).is_empty());
        assert!(HeroCache::read(&cache_path).entries.is_empty());

        // Another schema or app version reads as empty.
        fs::write(&cache_path, br#"{"schema":0,"version":"x","entries":{}}"#).unwrap();
        assert_eq!(HeroCache::read(&cache_path), HeroCache::default());
        let _ = fs::remove_dir_all(&root);
    }
}
