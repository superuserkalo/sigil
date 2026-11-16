//! Locker data: the hero roster the catalog knows about, the mods and assets
//! attached to each hero, and where local reference art lives on this machine.
//!
//! # Roster
//!
//! [`discover_heroes`] walks the game catalog for folders of the form
//! `models/heroes_*/*/`. Each folder name is a codename. The display name comes
//! from a curated map for known Deadlock codenames (`punkgoat` is Billy,
//! `ghost` is Lady Geist) with a title-cased fallback for anything new.
//! `asset_count` is the number of catalog entries that live under the hero
//! folder, including `materials/` paths. `mod_count` is the number of installed
//! addon slots whose VPK ships at least one entry under that hero folder.
//!
//! # Artwork
//!
//! [`LockerAssetPaths`] resolves three directories, in order:
//!
//! 1. `DAH_LOCKER_ASSETS`, a root that contains `heroes/`, `names/` and
//!    `hero_portraits/`.
//! 2. A Grimoire checkout at `vendor/grimoire/public`, where renders live in
//!    `locker/heroes`, name art in `locker/names`, and older portraits in
//!    `heroes`.
//! 3. Nothing. Every path field is `None` and every resolver returns `None`.
//!
//! [`resolve_render`] and [`resolve_name_art`] match Grimoire file naming with a
//! case-insensitive scan: `haze.png`, `Lady_Geist_Render.png`,
//! `The_Doorman_Render.png`, `Mo_&_Krill_name.png`. Core never decodes an
//! image. It hands the UI a path and the UI decides how to draw it.
//!
//! # Licensing boundary
//!
//! These are local reference assets. They are read at runtime from a path
//! outside this repository. They are never copied into the repository, bundled
//! into a release, or redistributed. A packaged build must extract the art from
//! the user's own game install or ship artwork whose license permits
//! redistribution. The repo ships no hero art.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::addons::InstalledAddon;
use crate::game_catalog::{GameCatalog, VpkKind};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Environment variable that points at a directory holding `heroes/`, `names/`
/// and `hero_portraits/`.
pub const LOCKER_ASSETS_ENV: &str = "DAH_LOCKER_ASSETS";

const RENDER_SUFFIX: &str = "_render";
const NAME_SUFFIX: &str = "_name";

/// Folder names under `heroes_*` that hold shared art rather than a hero.
const NON_HERO_FOLDERS: [&str; 7] = [
    "common",
    "shared",
    "props",
    "particles",
    "test",
    "tests",
    "template",
];

/// One hero in the Locker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerHero {
    /// Game folder name under `models/heroes_*`, for example `lady_geist`.
    pub codename: String,
    /// UI label, for example `Lady Geist`.
    pub display_name: String,
    /// Render art on disk, if the configured asset root has one.
    pub render_path: Option<PathBuf>,
    /// Name art on disk, if the configured asset root has one.
    pub name_art_path: Option<PathBuf>,
    /// Installed addon slots that ship at least one path under this hero.
    pub mod_count: usize,
    /// Catalog entries under this hero folder.
    pub asset_count: usize,
    /// False when the hero is only known from local art, not from the game
    /// catalog. The Locker keeps these so the art set can cover a thin catalog,
    /// and every consumer must label them as art only.
    #[serde(default = "default_true")]
    pub in_catalog: bool,
    /// Recolorable catalog paths under the hero folder, sorted and capped.
    /// Empty for an art-only hero: there is no game path to edit.
    #[serde(default)]
    pub recolor_paths: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Local artwork directories. Each field is `None` when it is not present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerAssetPaths {
    /// Hero renders, Grimoire `public/locker/heroes`.
    pub heroes_dir: Option<PathBuf>,
    /// Hero name art, Grimoire `public/locker/names`.
    pub names_dir: Option<PathBuf>,
    /// Older lowercase portraits, Grimoire `public/heroes`.
    pub hero_portraits_dir: Option<PathBuf>,
}

impl LockerAssetPaths {
    /// Resolve from `DAH_LOCKER_ASSETS`, then a development Grimoire checkout,
    /// then nothing.
    pub fn discover() -> Self {
        let configured = Self::from_env();

        if !configured.is_empty() {
            return configured;
        }

        Self::from_vendor_checkout()
    }

    /// Paths from `DAH_LOCKER_ASSETS` only. Unset or empty yields no paths.
    pub fn from_env() -> Self {
        match std::env::var_os(LOCKER_ASSETS_ENV) {
            Some(value) if !value.is_empty() => Self::from_root(Path::new(&value)),
            _ => Self::default(),
        }
    }

    /// Paths from a configured root that holds `heroes/`, `names/` and
    /// `hero_portraits/`. Missing folders stay `None`.
    pub fn from_root(root: &Path) -> Self {
        Self {
            heroes_dir: existing_dir(root.join("heroes")),
            names_dir: existing_dir(root.join("names")),
            hero_portraits_dir: existing_dir(root.join("hero_portraits")),
        }
    }

    /// Paths from a Grimoire `public/` folder.
    pub fn from_grimoire_public(public: &Path) -> Self {
        Self {
            heroes_dir: existing_dir(public.join("locker").join("heroes")),
            names_dir: existing_dir(public.join("locker").join("names")),
            hero_portraits_dir: existing_dir(public.join("heroes")),
        }
    }

    /// Look for `vendor/grimoire/public` in the current directory and its
    /// parents. This is the development fallback. It is never a packaged
    /// build's asset source.
    pub fn from_vendor_checkout() -> Self {
        let Ok(start) = std::env::current_dir() else {
            return Self::default();
        };

        for ancestor in start.ancestors().take(8) {
            let candidate = ancestor.join("vendor").join("grimoire").join("public");

            if candidate.is_dir() {
                let paths = Self::from_grimoire_public(&candidate);

                if !paths.is_empty() {
                    return paths;
                }
            }
        }

        Self::default()
    }

    /// True when no directory was found.
    pub fn is_empty(&self) -> bool {
        self.heroes_dir.is_none() && self.names_dir.is_none() && self.hero_portraits_dir.is_none()
    }
}

/// List every hero the catalog knows, with local art and mod counts attached.
///
/// Asset paths come from [`LockerAssetPaths::discover`]. Use
/// [`discover_heroes_with_assets`] when the caller already resolved a root.
pub fn discover_heroes(catalog: &GameCatalog, addons: &[InstalledAddon]) -> Vec<LockerHero> {
    let assets = LockerAssetPaths::discover();
    discover_heroes_with_assets(catalog, addons, &assets)
}

/// [`discover_heroes`] with explicit asset paths. A missing asset root is valid:
/// heroes still list, with `render_path` and `name_art_path` set to `None`.
pub fn discover_heroes_with_assets(
    catalog: &GameCatalog,
    addons: &[InstalledAddon],
    assets: &LockerAssetPaths,
) -> Vec<LockerHero> {
    let resolver = AssetResolver::build(assets);

    let mut asset_counts: BTreeMap<String, usize> = BTreeMap::new();

    for entry in &catalog.entries {
        let lowered = entry.path.to_ascii_lowercase();

        if let Some(codename) = hero_folder_code(&lowered) {
            *asset_counts.entry(codename.to_string()).or_insert(0) += 1;
        }
    }

    // Shared art lives outside `models/`, so count it for heroes already found.
    for entry in &catalog.entries {
        let lowered = entry.path.to_ascii_lowercase();

        if hero_folder_code(&lowered).is_some() {
            continue;
        }

        if let Some((codename, _)) = hero_code(&lowered) {
            if let Some(count) = asset_counts.get_mut(codename) {
                *count += 1;
            }
        }
    }

    let touched = touching_addon_slots(catalog, addons);

    let mut heroes: Vec<LockerHero> = asset_counts
        .into_iter()
        .map(|(codename, asset_count)| {
            let display_name = display_name_for(&codename);
            let render_path = resolver.render_for(&codename, &display_name);
            let name_art_path = resolver.name_art_for(&codename, &display_name);
            let mod_count = touched.get(&codename).map(BTreeSet::len).unwrap_or(0);

            let recolor_paths =
                recolor_paths_for(&catalog.entries, &codename, &display_name);
            LockerHero {
                codename,
                display_name,
                render_path,
                name_art_path,
                mod_count,
                asset_count,
                in_catalog: true,
                recolor_paths,
            }
        })
        .collect();

    // Local art can cover heroes the catalog does not list yet, which happens
    // when the index is partial or the hero is not in this build. They are
    // merged as art-only: zero catalog paths, zero mods, and `in_catalog`
    // false so the UI never presents them as game assets.
    let known: BTreeSet<String> = heroes
        .iter()
        .flat_map(|hero| {
            [
                hero_asset_key(&hero.codename),
                hero_asset_key(&hero.display_name),
            ]
        })
        .collect();
    let mut art_only: Vec<LockerHero> = resolver
        .renders
        .iter()
        .filter(|(key, _)| !known.contains(*key))
        .map(|(key, path)| LockerHero {
            codename: key.clone(),
            display_name: title_case(key),
            render_path: Some(path.clone()),
            name_art_path: resolver.names.get(key).cloned(),
            mod_count: 0,
            asset_count: 0,
            in_catalog: false,
            recolor_paths: Vec::new(),
        })
        .collect();
    heroes.append(&mut art_only);

    heroes.sort_by(|left, right| {
        left.display_name
            .to_ascii_lowercase()
            .cmp(&right.display_name.to_ascii_lowercase())
            .then_with(|| left.codename.cmp(&right.codename))
    });

    heroes
}

/// Resolve the render for one hero against the configured asset paths.
pub fn resolve_render(hero: &LockerHero, assets: &LockerAssetPaths) -> Option<PathBuf> {
    AssetResolver::build(assets).render_for(&hero.codename, &hero.display_name)
}

/// Resolve the name art for one hero. Same naming rules as [`resolve_render`].
pub fn resolve_name_art(hero: &LockerHero, assets: &LockerAssetPaths) -> Option<PathBuf> {
    AssetResolver::build(assets).name_art_for(&hero.codename, &hero.display_name)
}

/// Available render files keyed by [`hero_asset_key`], for a picker or a
/// presence check. Renders from `heroes_dir` win over older portraits.
pub fn read_hero_asset_index(assets: &LockerAssetPaths) -> BTreeMap<String, PathBuf> {
    AssetResolver::build(assets).renders
}

/// Normalize a codename, display name or file stem to a lookup key.
///
/// Lowercase, `&` becomes `and`, runs of anything else become one underscore,
/// and a leading `the_` is dropped. `Mo & Krill`, `mo_and_krill` and
/// `Mo_&_Krill_Render.png` all map to `mo_and_krill`.
pub fn hero_asset_key(name: &str) -> String {
    let lowered = name.trim().to_ascii_lowercase().replace('&', " and ");
    let mut key = String::with_capacity(lowered.len());
    let mut pending_separator = false;

    for character in lowered.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !key.is_empty() {
                key.push('_');
            }

            key.push(character);
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }

    let without_leading_the = key.strip_prefix("the_").unwrap_or(&key);
    without_leading_the.to_string()
}

/// Display label for a catalog codename, or a title-cased fallback.
pub fn display_name_for(codename: &str) -> String {
    let lowered = codename.trim().to_ascii_lowercase().replace('-', "_");
    let key = lowered.strip_prefix("hero_").unwrap_or(&lowered);

    if let Some(display) = curated_display_name(key) {
        return display.to_string();
    }

    title_case(key)
}

fn curated_display_name(key: &str) -> Option<&'static str> {
    Some(match key {
        "abrams" | "atlas" | "atlas_detective" | "atlas_detective_v2" => "Abrams",
        "apollo" => "Apollo",
        "baba" => "Baba",
        "bebop" => "Bebop",
        "billy" | "punkgoat" => "Billy",
        "calico" | "nano" => "Calico",
        "celeste" => "Celeste",
        "deadman" | "deadman_danny" => "Deadman Danny",
        "doorman" | "the_doorman" => "The Doorman",
        "drifter" => "Drifter",
        "dynamo" => "Dynamo",
        "graves" => "Graves",
        "grey_talon" | "orion" => "Grey Talon",
        "haze" => "Haze",
        "holiday" | "holliday" => "Holliday",
        "infernus" => "Infernus",
        "ivy" => "Ivy",
        "kelvin" => "Kelvin",
        "ghost" | "lady_geist" => "Lady Geist",
        "lash" => "Lash",
        "mc_ginnis" | "mcginnis" => "McGinnis",
        "mina" => "Mina",
        "mirage" => "Mirage",
        "mo_and_krill" | "mo_krill" | "mokrill" => "Mo & Krill",
        "nurse_harrow" => "Nurse Harrow",
        "bookworm" | "paige" => "Paige",
        "paradox" => "Paradox",
        "pocket" | "synth" => "Pocket",
        "rat_king" => "Rat King",
        "rem" => "Rem",
        "gigawatt" | "seven" => "Seven",
        "shiv" => "Shiv",
        "silver" => "Silver",
        "magician" | "sinclair" => "Sinclair",
        "solomon" => "Solomon",
        "venator" => "Venator",
        "victor" => "Victor",
        "hornet" | "vindicta" => "Vindicta",
        "violet" => "Violet",
        "viscous" => "Viscous",
        "viper" | "vyper" => "Vyper",
        "warden" => "Warden",
        "wraith" => "Wraith",
        "yamato" => "Yamato",
        _ => return None,
    })
}

fn title_case(value: &str) -> String {
    let mut words = Vec::new();

    for word in value
        .split(|character: char| character == '_' || character == '-' || character.is_whitespace())
    {
        if word.is_empty() {
            continue;
        }

        let mut characters = word.chars();
        let first = characters
            .next()
            .expect("non-empty word has a first character");
        let mut rendered = String::new();

        rendered.extend(first.to_uppercase());
        rendered.push_str(&characters.as_str().to_lowercase());
        words.push(rendered);
    }

    if words.is_empty() {
        return value.to_string();
    }

    words.join(" ")
}

struct AssetResolver {
    renders: BTreeMap<String, PathBuf>,
    names: BTreeMap<String, PathBuf>,
}

impl AssetResolver {
    fn build(assets: &LockerAssetPaths) -> Self {
        let mut renders = BTreeMap::new();

        if let Some(directory) = &assets.heroes_dir {
            collect_files(directory, RENDER_SUFFIX, &mut renders);
        }

        if let Some(directory) = &assets.hero_portraits_dir {
            collect_files(directory, RENDER_SUFFIX, &mut renders);
        }

        let mut names = BTreeMap::new();

        if let Some(directory) = &assets.names_dir {
            collect_files(directory, NAME_SUFFIX, &mut names);
        }

        Self { renders, names }
    }

    fn render_for(&self, codename: &str, display_name: &str) -> Option<PathBuf> {
        lookup_keys(codename, display_name)
            .into_iter()
            .find_map(|key| self.renders.get(&key).cloned())
    }

    fn name_art_for(&self, codename: &str, display_name: &str) -> Option<PathBuf> {
        lookup_keys(codename, display_name)
            .into_iter()
            .find_map(|key| self.names.get(&key).cloned())
    }
}

fn lookup_keys(codename: &str, display_name: &str) -> Vec<String> {
    let mut candidates = vec![
        display_name.to_string(),
        codename.to_string(),
        title_case(codename),
    ];

    if let Some(stripped) = codename.strip_prefix("hero_") {
        candidates.push(stripped.to_string());
    }

    let mut keys = Vec::new();

    for candidate in candidates {
        let key = hero_asset_key(&candidate);

        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
        }
    }

    keys
}

fn collect_files(directory: &Path, suffix: &str, index: &mut BTreeMap<String, PathBuf>) {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();

        if !is_image_name(&name) {
            continue;
        }

        let stem = Path::new(&name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&name);
        let hero = strip_suffix_ignore_case(stem, suffix);
        let key = hero_asset_key(hero);

        if key.is_empty() {
            continue;
        }

        index.entry(key).or_insert(path);
    }
}

fn is_image_name(name: &str) -> bool {
    [".png", ".jpg", ".jpeg", ".webp", ".avif"]
        .iter()
        .any(|extension| name.to_ascii_lowercase().ends_with(extension))
}

fn strip_suffix_ignore_case<'a>(value: &'a str, suffix: &str) -> &'a str {
    if value.len() >= suffix.len() {
        let tail = &value[value.len() - suffix.len()..];

        if tail.eq_ignore_ascii_case(suffix) {
            return &value[..value.len() - suffix.len()];
        }
    }

    value
}

/// Codename from a path that starts with `models/heroes_*/<codename>/`.
/// Materials and textures under one hero folder, the paths a recolor edits.
fn recolor_paths_for(
    entries: &[crate::game_catalog::CatalogEntry],
    codename: &str,
    display_name: &str,
) -> Vec<String> {
    let wanted = hero_asset_key(display_name);
    let mut paths: Vec<String> = entries
        .iter()
        .filter(|entry| {
            let lowered = entry.path.to_ascii_lowercase();
            // Materials live under materials/models/heroes_*, not models/, so
            // match the heroes_* segment anywhere. An alias such as `billy`
            // next to `punkgoat` is unified by the display name.
            let Some((code, _)) = hero_code(&lowered) else {
                return false;
            };
            code == codename || hero_asset_key(&display_name_for(code)) == wanted
        })
        .filter(|entry| {
            entry.path.ends_with(".vmat_c") || entry.path.ends_with(".vtex_c")
        })
        .map(|entry| entry.path.clone())
        .collect();
    paths.sort();
    paths.dedup();
    paths.truncate(16);
    paths
}

fn hero_folder_code(path_lower: &str) -> Option<&str> {
    let segments: Vec<&str> = path_lower.split('/').collect();

    if segments.len() < 4 {
        return None;
    }

    if segments[0] != "models" || !segments[1].starts_with("heroes") {
        return None;
    }

    let codename = segments[2];

    if codename.is_empty() || NON_HERO_FOLDERS.contains(&codename) || codename.contains('.') {
        return None;
    }

    Some(codename)
}

/// Codename from any path that crosses a `heroes_*` folder, including
/// `materials/models/heroes/<codename>/...`.
fn hero_code(path_lower: &str) -> Option<(&str, usize)> {
    let segments: Vec<&str> = path_lower.split('/').collect();
    let heroes_at = segments
        .iter()
        .position(|segment| segment.starts_with("heroes"))?;
    let codename = *segments.get(heroes_at + 1)?;

    if codename.is_empty() || NON_HERO_FOLDERS.contains(&codename) {
        return None;
    }

    // A sixth segment proves the codename is a folder, not a bare file.
    if segments.len() <= heroes_at + 2 {
        return None;
    }

    Some((codename, heroes_at + 2))
}

fn touching_addon_slots(
    catalog: &GameCatalog,
    addons: &[InstalledAddon],
) -> BTreeMap<String, BTreeSet<u8>> {
    let mut touched: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();

    let Some(directory) = addons_directory(catalog) else {
        return touched;
    };

    if !directory.is_dir() {
        return touched;
    }

    for addon in addons {
        let path = directory.join(&addon.file);
        let Ok(parsed) = read_vpk_directory_with(
            &path,
            &ReadVpkOptions {
                keep_preload: false,
            },
        ) else {
            continue;
        };

        for entry in parsed.entries {
            let lowered = entry.path.to_ascii_lowercase();

            if let Some((codename, _)) = hero_code(&lowered) {
                touched
                    .entry(codename.to_string())
                    .or_default()
                    .insert(addon.slot);
            }
        }
    }

    touched
}

/// The addons folder sits beside the citadel VPK, so the catalog already knows
/// where it is. This keeps `discover_heroes` free of a game root argument.
fn addons_directory(catalog: &GameCatalog) -> Option<PathBuf> {
    catalog
        .sources
        .iter()
        .filter(|source| source.kind == VpkKind::Citadel)
        .find_map(|source| {
            let citadel = Path::new(&source.path).parent()?;
            Some(citadel.join("addons"))
        })
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_catalog::CatalogEntry;

    fn entry(path: &str) -> CatalogEntry {
        CatalogEntry {
            path: path.to_string(),
            crc32: 0,
            preload_bytes: 0,
            archive_index: 0,
            offset: 0,
            length: 1,
            vpk: "citadel".to_string(),
        }
    }

    fn catalog() -> GameCatalog {
        GameCatalog {
            schema: 1,
            built_at: "test".to_string(),
            sources: Vec::new(),
            entries: vec![
                entry("models/heroes_wip/haze/haze.vmdl_c"),
                entry("models/heroes_wip/haze/materials/haze.vmat_c"),
                entry("models/heroes_wip/lady_geist/body.vmdl_c"),
                entry("materials/models/heroes/lady_geist/skin.vmat_c"),
                entry("models/heroes_wip/doorman/body.vmdl_c"),
                entry("models/heroes_wip/punkgoat/body.vmdl_c"),
                entry("models/heroes_wip/mo_and_krill/body.vmdl_c"),
            ],
        }
    }

    #[test]
    fn display_names_and_keys() {
        assert_eq!(display_name_for("punkgoat"), "Billy");
        assert_eq!(display_name_for("ghost"), "Lady Geist");
        assert_eq!(display_name_for("barely_a_hero"), "Barely A Hero");
        assert_eq!(hero_asset_key("Mo & Krill"), "mo_and_krill");
        assert_eq!(hero_asset_key("The Doorman"), "doorman");
        assert_eq!(hero_asset_key("Mo_&_Krill_Render"), "mo_and_krill_render");
    }

    #[test]
    fn resolves_assets_and_lists_heroes() {
        let root = std::env::temp_dir().join(format!("dah-locker-{}", std::process::id()));
        let heroes = root.join("heroes");
        let names = root.join("names");
        let portraits = root.join("hero_portraits");

        fs::create_dir_all(&heroes).expect("heroes directory");
        fs::create_dir_all(&names).expect("names directory");
        fs::create_dir_all(&portraits).expect("portraits directory");

        fs::write(heroes.join("Lady_Geist_Render.png"), b"x").expect("render");
        fs::write(heroes.join("The_Doorman_Render.png"), b"x").expect("doorman render");
        fs::write(portraits.join("haze.png"), b"x").expect("portrait");
        fs::write(names.join("Lady_Geist_name.png"), b"x").expect("name art");

        let assets = LockerAssetPaths::from_root(&root);
        let discovered = discover_heroes_with_assets(&catalog(), &[], &assets);

        assert_eq!(discovered.len(), 5);

        let rhythm = discovered
            .iter()
            .find(|hero| hero.codename == "haze")
            .expect("haze is listed");
        assert_eq!(rhythm.display_name, "Haze");
        assert_eq!(rhythm.asset_count, 2);
        assert_eq!(
            rhythm.render_path.as_deref(),
            Some(portraits.join("haze.png").as_path())
        );

        let geist = discovered
            .iter()
            .find(|hero| hero.codename == "lady_geist")
            .expect("lady geist is listed");
        assert_eq!(geist.display_name, "Lady Geist");
        assert_eq!(geist.asset_count, 2);
        assert_eq!(
            geist.name_art_path.as_deref(),
            Some(names.join("Lady_Geist_name.png").as_path())
        );
        assert_eq!(
            resolve_render(geist, &assets),
            Some(heroes.join("Lady_Geist_Render.png"))
        );

        let doorman = discovered
            .iter()
            .find(|hero| hero.codename == "doorman")
            .expect("doorman is listed");
        assert_eq!(doorman.display_name, "The Doorman");
        assert_eq!(
            doorman.render_path,
            Some(heroes.join("The_Doorman_Render.png"))
        );

        let billy = discovered
            .iter()
            .find(|hero| hero.codename == "punkgoat")
            .expect("punkgoat is listed");
        assert_eq!(billy.display_name, "Billy");
        assert!(billy.render_path.is_none());

        let index = read_hero_asset_index(&assets);
        assert!(index.contains_key("lady_geist"));
        assert!(index.contains_key("haze"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_assets_are_not_an_error() {
        let assets = LockerAssetPaths::default();
        let discovered = discover_heroes_with_assets(&catalog(), &[], &assets);

        assert_eq!(discovered.len(), 5);
        assert!(discovered.iter().all(|hero| hero.render_path.is_none()));
        assert!(read_hero_asset_index(&assets).is_empty());
    }
}
