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

// --- every hero a payload names, and the text fallback ----------------------------------

/// Roster display names of every hero the mod's files name, whatever kind of file: models and
/// materials, particles under `particles/heroes*`, `particles/abilities/<codename>/` and
/// `particles/weapon_fx/<codename>/`, hero scripts, ability sounds, voice lines and
/// footsteps, sound event files, and hero portraits in `panorama/images/heroes/`. Ported from
/// Grimoire's `inferHeroesFromVpkPaths`. A sound pack or a HUD pack for one hero names that
/// hero here, unlike [`model_heroes`], which only counts bodies.
pub fn asset_heroes<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut heroes = BTreeSet::new();
    for path in paths {
        let lowered = path.to_ascii_lowercase().replace('\\', "/");
        if let Some(hero) = asset_hero(&lowered) {
            heroes.insert(hero.to_string());
        }
    }
    heroes.into_iter().collect()
}

fn asset_hero(path: &str) -> Option<&'static str> {
    if let Some(sound) =
        crate::locker::locker_sound(path).or_else(|| crate::locker::hero_sound(path))
    {
        return crate::locker::roster_hero(&sound.hero).map(|hero| hero.name);
    }
    if path.starts_with("sounds/") || path.starts_with("soundevents/") {
        return None;
    }
    for prefix in ["particles/abilities/", "particles/weapon_fx/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let (folder, _) = rest.split_once('/')?;
            return crate::locker::hero_for_codename(folder).map(|hero| hero.name);
        }
    }
    if let Some(file) = path.strip_prefix("panorama/images/heroes/") {
        let stem = file.rsplit('/').next()?.split('.').next()?;
        let mut tokens: Vec<&str> = stem.split('_').collect();
        while !tokens.is_empty() {
            if let Some(hero) = crate::locker::roster_hero(&tokens.join("_")) {
                return Some(hero.name);
            }
            tokens.pop();
        }
        return None;
    }
    let lead = path.split('/').next().unwrap_or("");
    if matches!(lead, "models" | "materials" | "particles" | "scripts") {
        return hero_of_path(path).map(|hero| hero.name);
    }
    None
}

/// What a mod's files say it is, for a mod GameBanana filed only under a broad root (Skins or
/// Model Replacement with no hero below it) or one with no category at all. Ported from
/// Grimoire's `classifyModPayload` (`fix/gb-category-drift`): conservative, never a
/// replacement for a real category.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum Payload {
    /// One hero's body, effects or portrait.
    Hero(String),
    /// A Hideout, Icon pack or HUD.
    General(crate::locker::GeneralType),
    /// Several heroes, or a skin bundled with Hideout or HUD files.
    Ambiguous,
    /// Nothing recognised.
    #[default]
    Unknown,
}

/// See [`Payload`].
pub fn classify_payload<'a>(paths: impl IntoIterator<Item = &'a str>) -> Payload {
    use crate::locker::GeneralType;
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.to_ascii_lowercase().replace('\\', "/"))
        .collect();
    let strip = |path: &str| path.strip_prefix("materials/").unwrap_or(path).to_string();
    let hero_payload: Vec<&str> = paths
        .iter()
        .filter(|path| {
            let model = strip(path);
            model.starts_with("models/heroes")
                || path.starts_with("particles/heroes")
                || path.starts_with("particles/abilities/")
                || path.starts_with("particles/weapon_fx/")
        })
        .map(String::as_str)
        .collect();
    let hideout = paths.iter().any(|path| strip(path).starts_with("models/hideout/"));
    let hud = paths.iter().any(|path| {
        ["panorama/layout/", "panorama/styles/", "panorama/images/hud/", "panorama/images/items/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
    });
    let portraits: Vec<&str> = paths
        .iter()
        .filter(|path| path.starts_with("panorama/images/heroes/"))
        .map(String::as_str)
        .collect();

    if !hero_payload.is_empty() {
        // Bundled portraits never turn a skin into an icon pack or add heroes.
        let heroes = asset_heroes(hero_payload);
        return if !hideout && !hud && heroes.len() == 1 {
            Payload::Hero(heroes[0].clone())
        } else {
            Payload::Ambiguous
        };
    }
    if hideout {
        return if hud || !portraits.is_empty() {
            Payload::Ambiguous
        } else {
            Payload::General(GeneralType::Hideout)
        };
    }
    if hud {
        return Payload::General(GeneralType::Hud);
    }
    if !portraits.is_empty() {
        let heroes = asset_heroes(portraits);
        return match heroes.len() {
            0 => Payload::Ambiguous,
            1 => Payload::Hero(heroes[0].clone()),
            _ => Payload::General(GeneralType::Icons),
        };
    }
    let heroes = asset_heroes(paths.iter().map(String::as_str));
    match heroes.len() {
        0 => Payload::Unknown,
        1 => Payload::Hero(heroes[0].clone()),
        _ => Payload::Ambiguous,
    }
}

/// Names a title or file name may use for a hero besides its display name: Grimoire's shared
/// `HERO_ALIASES` and Browse's whole-name aliases. Lowercase, `_` and `-` already spaces.
const TEXT_ALIASES: &[(&str, &[&str])] = &[
    ("Mo & Krill", &["mo & krill", "mo and krill", "mo+krill", "mokrill", "mo krill"]),
    ("Grey Talon", &["greytalon"]),
    ("Lady Geist", &["ladygeist", "geist"]),
    ("McGinnis", &["mc ginnis"]),
    ("Rat King", &["ratking"]),
    ("Doorman", &["the doorman"]),
    ("Deadman Danny", &["deadman", "danny"]),
    ("Nurse Harrow", &["harrow"]),
];

/// Names that are also everyday words in titles ("Patches of Violet on Billy"): they only
/// count when no other hero is named.
const WEAK_NAMES: &[&str] = &["Violet"];

fn cased(c: char) -> bool {
    c.is_lowercase() || c.is_uppercase()
}

fn normalize_text(text: &str) -> String {
    let spaced: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True when `alias` stands as a whole name in `text`: no cased letter right before it, and
/// none right after it once an optional possessive (`'s`, `’s` or a bare `s`) is skipped.
/// Digits do not bound a name ("Rem2"), and uncased scripts do not either ("Mina兔女郎").
fn mentions(text: &str, alias: &str) -> bool {
    for (at, _) in text.match_indices(alias) {
        if text[..at].chars().next_back().is_some_and(cased) {
            continue;
        }
        let rest = &text[at + alias.len()..];
        let after = ["'s", "\u{2019}s", "s"]
            .iter()
            .find_map(|suffix| rest.strip_prefix(suffix))
            .filter(|tail| !tail.chars().next().is_some_and(cased))
            .unwrap_or(rest);
        if !after.chars().next().is_some_and(cased) {
            return true;
        }
    }
    false
}

/// Every roster hero a title, file name or description names as a whole word, sorted. Ported
/// from Grimoire's `heroesMentionedInFileText` and `nameMentionsHero`: `_` and `-` read as
/// spaces, the match ignores case, possessives count ("Billys Blasted"), aliases count
/// ("Geist", "RatKing", "Mo and Krill"), and a longer word does not ("Remastered" is not Rem,
/// "Silverhand" is not Silver). A weak name like Violet only counts alone.
pub fn heroes_in_text(text: &str) -> Vec<String> {
    let text = normalize_text(text);
    if text.is_empty() {
        return Vec::new();
    }
    let mut found = BTreeSet::new();
    for hero in crate::locker::ROSTER.iter() {
        let name = hero.name.to_lowercase();
        let joined = name.replace(' ', "");
        let extra = TEXT_ALIASES
            .iter()
            .find(|(display, _)| *display == hero.name)
            .map(|(_, aliases)| *aliases)
            .unwrap_or(&[]);
        let hit = std::iter::once(name.as_str())
            .chain(std::iter::once(joined.as_str()))
            .chain(extra.iter().copied())
            .any(|alias| mentions(&text, alias));
        if hit {
            found.insert(hero.name.to_string());
        }
    }
    if found.len() > 1 {
        for weak in WEAK_NAMES {
            found.remove(*weak);
        }
    }
    found.into_iter().collect()
}

/// Which level of a mod answered [`guess_heroes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeroSource {
    /// The VPK's own paths.
    Files,
    /// A level of text the caller passed, by its index.
    Text(usize),
}

/// Heroes for a mod GameBanana did not file under a hero, Grimoire's
/// `detectModHeroes` order: the files first, then each text level in turn (per-file
/// description, original file names, the submission title), stopping at the first level that
/// names any hero. `broad` is a mod filed directly under Skins or Model Replacement: its files
/// must point at exactly one hero ([`classify_payload`]), and a text level that names several
/// heroes does not establish any, since a broad skin is one hero's skin.
pub fn guess_heroes(
    payload: &Payload,
    files: &[String],
    broad: bool,
    texts: &[&[&str]],
) -> (Vec<String>, Option<HeroSource>) {
    if broad {
        match payload {
            Payload::Hero(hero) => return (vec![hero.clone()], Some(HeroSource::Files)),
            Payload::Unknown => {}
            _ => return (Vec::new(), None),
        }
    } else if !files.is_empty() {
        return (files.to_vec(), Some(HeroSource::Files));
    }
    for (level, texts) in texts.iter().enumerate() {
        let mut heroes = BTreeSet::new();
        for text in *texts {
            heroes.extend(heroes_in_text(text));
        }
        if heroes.is_empty() {
            continue;
        }
        if broad && heroes.len() > 1 {
            return (Vec::new(), None);
        }
        return (heroes.into_iter().collect(), Some(HeroSource::Text(level)));
    }
    (Vec::new(), None)
}

/// The cache file name, for `<data_dir>/mod-heroes.json`.
pub const HERO_CACHE_FILE: &str = "mod-heroes.json";
/// Bumped when the cache layout or the hero rule changes.
pub const HERO_CACHE_SCHEMA: u32 = 2;

/// The cached answer for one VPK, valid while its size and modification time hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedHeroes {
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub modified_nanos: u128,
    pub heroes: Vec<String>,
    /// [`asset_heroes`] of the same file.
    #[serde(default)]
    pub assets: Vec<String>,
    /// [`classify_payload`] of the same file.
    #[serde(default)]
    pub payload: Payload,
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
        self.answer(vpk).map(|answer| answer.heroes)
    }

    /// Every cached answer for the VPK at `vpk`: model heroes, asset heroes and the payload,
    /// from one read of its directory tree.
    pub fn answer(&mut self, vpk: &Path) -> crate::Result<CachedHeroes> {
        let (size, modified_nanos) = stamp_of(vpk)?;
        let key = vpk.to_string_lossy().into_owned();
        if let Some(hit) = self.entries.get(&key)
            && hit.size == size
            && hit.modified_nanos == modified_nanos
        {
            return Ok(hit.clone());
        }
        let directory = read_vpk_directory_with(vpk, &ReadVpkOptions { keep_preload: false })?;
        let paths: Vec<&str> = directory.entries.iter().map(|entry| entry.path.as_str()).collect();
        let answer = CachedHeroes {
            size,
            modified_nanos,
            heroes: model_heroes(paths.iter().copied()),
            assets: asset_heroes(paths.iter().copied()),
            payload: classify_payload(paths.iter().copied()),
        };
        self.entries.insert(key, answer.clone());
        self.dirty = true;
        Ok(answer)
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

/// [`model_heroes_cached`] with the whole answer per VPK: model heroes, asset heroes and the
/// payload. The Installed page's hero filter reads the last two for mods GameBanana did not
/// file under a hero.
pub fn heroes_cached(cache_path: &Path, vpks: &[PathBuf]) -> BTreeMap<PathBuf, CachedHeroes> {
    let mut cache = HeroCache::read(cache_path);
    let out = vpks
        .iter()
        .filter_map(|vpk| cache.answer(vpk).ok().map(|answer| (vpk.clone(), answer)))
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
    fn asset_heroes_read_every_kind_of_hero_file() {
        assert_eq!(asset_heroes(["sounds/vo/haze/kill.vsnd_c"]), vec!["Haze"]);
        assert_eq!(asset_heroes(["sounds/player/footsteps/inferno/a.vsnd_c"]), vec!["Infernus"]);
        assert_eq!(asset_heroes(["sounds/abilities/abrams/a2_charge/x.vsnd_c"]), vec!["Abrams"]);
        assert_eq!(asset_heroes(["particles/abilities/bookworm/a1.vpcf_c"]), vec!["Paige"]);
        assert_eq!(asset_heroes(["panorama/images/heroes/inferno_card_psd.vtex_c"]), vec!["Infernus"]);
        assert_eq!(asset_heroes(["soundevents/hero/hero_haze.vsndevts_c"]), vec!["Haze"]);
        // NPCs and shared sounds name nobody.
        assert!(asset_heroes(["sounds/vo/patron/a.vsnd_c", "models/npc/patron_amber/a.vmdl_c"]).is_empty());
    }

    #[test]
    fn payload_classifies_broad_skins_conservatively() {
        use crate::locker::GeneralType;
        assert_eq!(
            classify_payload(["models/heroes_wip/haze/haze.vmdl_c", "panorama/images/heroes/inferno_card.vtex_c"]),
            Payload::Hero("Haze".into())
        );
        assert_eq!(
            classify_payload(["models/heroes_wip/haze/haze.vmdl_c", "models/heroes_wip/lash/lash.vmdl_c"]),
            Payload::Ambiguous
        );
        assert_eq!(
            classify_payload(["models/heroes_wip/haze/haze.vmdl_c", "panorama/layout/hud.vxml_c"]),
            Payload::Ambiguous
        );
        assert_eq!(
            classify_payload(["models/hideout/bar/bar.vmdl_c"]),
            Payload::General(GeneralType::Hideout)
        );
        assert_eq!(
            classify_payload(["panorama/images/heroes/haze_card.vtex_c", "panorama/images/heroes/lash_card.vtex_c"]),
            Payload::General(GeneralType::Icons)
        );
        assert_eq!(classify_payload(["panorama/styles/hud.vcss_c"]), Payload::General(GeneralType::Hud));
        assert_eq!(classify_payload(["scripts/x.txt"]), Payload::Unknown);
    }

    #[test]
    fn titles_name_heroes_as_whole_words_with_aliases() {
        assert_eq!(heroes_in_text("Billys Blasted"), vec!["Billy"]);
        assert_eq!(heroes_in_text("Rem's hat"), vec!["Rem"]);
        assert!(heroes_in_text("Remastered HUD").is_empty());
        assert!(heroes_in_text("Silverhand").is_empty());
        assert_eq!(heroes_in_text("Rem2 recolor"), vec!["Rem"]);
        assert_eq!(heroes_in_text("geist_maid_outfit"), vec!["Lady Geist"]);
        assert_eq!(heroes_in_text("RatKing flag"), vec!["Rat King"]);
        assert_eq!(heroes_in_text("Mo and Krill drill"), vec!["Mo & Krill"]);
        assert_eq!(heroes_in_text("The Doorman as a bellhop"), vec!["Doorman"]);
        assert_eq!(heroes_in_text("Mina\u{5154}\u{5973}\u{90ce}"), vec!["Mina"]);
        assert!(heroes_in_text("harrowing night").is_empty());
        // A weak name only counts alone.
        assert_eq!(heroes_in_text("Patches of Violet on Billy"), vec!["Billy"]);
        assert_eq!(heroes_in_text("Violet painter"), vec!["Violet"]);
        assert_eq!(heroes_in_text("Haze and Lash duo"), vec!["Haze", "Lash"]);
    }

    #[test]
    fn guesses_stop_at_the_first_level_that_names_a_hero() {
        let title: &[&str] = &["Haze in a suit"];
        let files: &[&str] = &["lash_skin.zip"];
        // Files win over text.
        let (heroes, source) = guess_heroes(&Payload::Unknown, &["Wraith".into()], false, &[files, title]);
        assert_eq!((heroes, source), (vec!["Wraith".to_string()], Some(HeroSource::Files)));
        // The first text level that names a hero answers.
        let (heroes, source) = guess_heroes(&Payload::Unknown, &[], false, &[files, title]);
        assert_eq!((heroes, source), (vec!["Lash".to_string()], Some(HeroSource::Text(0))));
        // A broad skin takes the one hero its files point at.
        let (heroes, _) = guess_heroes(&Payload::Hero("Ivy".into()), &[], true, &[title]);
        assert_eq!(heroes, vec!["Ivy"]);
        // A broad skin whose files are ambiguous gets no hero, even from its title.
        assert!(guess_heroes(&Payload::Ambiguous, &[], true, &[title]).0.is_empty());
        // A broad skin whose title names two heroes gets none.
        let two: &[&str] = &["Haze and Lash"];
        assert!(guess_heroes(&Payload::Unknown, &[], true, &[two]).0.is_empty());
        assert_eq!(guess_heroes(&Payload::Unknown, &[], false, &[two]).0, vec!["Haze", "Lash"]);
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
