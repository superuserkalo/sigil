//! Locker data: the playable hero roster, the installed mods attached to each
//! hero or to a General category, the user's own Locker categories, and where
//! local reference art lives on this machine.
//!
//! # Roster
//!
//! The roster is curated, as Grimoire's is: one entry per playable hero, with
//! the display name, the codenames the game and community packs file that hero
//! under, and where the face sits in the render. It does not come from raw
//! `models/heroes_*` folder names: those include old codenames (`archer`,
//! `artist`), versioned folders (`inferno_v4`, `atlas_detective_v2`), NPCs and
//! test rigs, which showed up as junk heroes and duplicate Abrams.
//!
//! The game catalog only adds facts to a roster hero: how many paths sit under
//! its folders and which materials a recolor can target. A catalog folder maps
//! to a hero through [`hero_for_codename`]: an exact codename, then the same
//! name with `_vN` suffixes dropped, then the longest codename that prefixes the
//! folder (`atlas_detective` is Abrams, `wraith_gen_man` is Wraith). Folders
//! that match nothing are ignored, never shown. Paths are always read from the
//! catalog; the table names heroes, not folders.
//!
//! # General
//!
//! An installed mod that touches no hero is classified into one of Grimoire's
//! General types (Soul Containers, HUD, Hideout, ...) from the paths it ships.
//! [`LockerCategory`] is the third, view only axis: named piles the user files
//! mods into. Filing never enables, disables or moves a mod. Categories live in
//! a JSON file the app keeps in its data directory, never in the game folder.
//!
//! # Artwork
//!
//! [`LockerAssetPaths`] resolves art directories, in order:
//!
//! 1. `DAH_LOCKER_ASSETS`, a root that contains `heroes/`, `names/` and
//!    `hero_portraits/`.
//! 2. The art installed with the app, in the same layout: `assets/locker` or
//!    `locker` beside the executable (also beside the resolved executable when
//!    it was started through a link), or `Contents/Resources/locker` in a macOS
//!    bundle.
//! 3. `assets/locker` under the working directory, for a shortcut that starts
//!    the app from its folder.
//! 4. A Grimoire checkout at `vendor/grimoire/public`.
//! 5. Nothing. Every path field is `None` and every resolver returns `None`.
//!
//! Art matches by the hero's display name or codename with a case-insensitive
//! scan: `Lady_Geist_Render.webp`, `The_Doorman_Render.png`,
//! `Mo_&_Krill_name.webp`, `haze.png`. Core never decodes an image.
//!
//! # Packaged art
//!
//! The art is in `rust/crates/desktop/assets/locker` and is built into the
//! executable (`rust/crates/desktop/build.rs`). Where it came from is recorded in
//! `THIRD_PARTY_NOTICES.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::addons::{InstalledAddon, SetManyReport};
use crate::error::{Error, Result};
use crate::game_catalog::{CatalogEntry, GameCatalog, VpkKind};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Environment variable that points at a directory holding `heroes/`, `names/`
/// and `hero_portraits/`.
pub const LOCKER_ASSETS_ENV: &str = "DAH_LOCKER_ASSETS";

/// File name of the category store inside the app data directory.
pub const LOCKER_CATEGORIES_FILE: &str = "locker-categories.json";

/// File name of the favourites, shuffle and custom card store.
pub const LOCKER_PREFS_FILE: &str = "locker-prefs.json";

/// Folder inside the app data directory that holds copied custom card art.
pub const LOCKER_CARDS_DIR: &str = "locker-cards";

/// A custom card image larger than this is refused. Card art is small.
pub const CUSTOM_CARD_BYTES: u64 = 16 * 1024 * 1024;

/// Folder under `citadel` that loads before `citadel/addons`.
pub const GLOBAL_FOLDER: &str = "grimoire";

/// The SearchPaths line that mounts the Global priority folder.
pub const GLOBAL_SEARCH_LINE: &str = "Game\t\tcitadel/grimoire";

const RENDER_SUFFIX: &str = "_render";
const NAME_SUFFIX: &str = "_name";
const RECOLOR_PATH_CAP: usize = 16;
const CATEGORY_NAME_MAX: usize = 80;
const CATEGORY_SCHEMA: u32 = 1;
const PREFS_SCHEMA: u32 = 1;
const ART_HEADER_BYTES: usize = 64;

/// Default horizontal face position, in percent of the render width.
pub const DEFAULT_FACE_X: f32 = 55.0;

/// One playable hero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RosterHero {
    /// Display name, which also names the art files.
    pub name: &'static str,
    /// Codenames for catalog and mod path matching. The first is the current
    /// game class name; the rest are older names community packs still use.
    pub codenames: &'static [&'static str],
    /// Where the face sits across the render, 0 to 100 percent. A card that
    /// crops the render keeps this point in view. From Grimoire's calibration.
    pub face_x: f32,
}

const fn hero(name: &'static str, codenames: &'static [&'static str], face_x: f32) -> RosterHero {
    RosterHero {
        name,
        codenames,
        face_x,
    }
}

/// Every playable hero, alphabetical. Ported from Grimoire's
/// `heroCodenames.ts` (codenames and legacy aliases) and `lockerUtils.ts`
/// (`HERO_FACE_POSITION`).
pub const ROSTER: [RosterHero; 44] = [
    hero("Abrams", &["atlas", "bull"], 0.0),
    hero("Apollo", &["fencer"], 55.0),
    hero("Baba", &["baba"], DEFAULT_FACE_X),
    hero("Bebop", &["bebop"], 81.0),
    hero("Billy", &["punkgoat"], 73.0),
    hero("Calico", &["nano"], 80.0),
    hero("Celeste", &["unicorn"], 55.0),
    hero("Deadman Danny", &["deadpack"], DEFAULT_FACE_X),
    hero("Doorman", &["doorman"], 40.0),
    hero("Drifter", &["drifter"], 93.0),
    hero("Dynamo", &["dynamo", "sumo"], 68.0),
    hero("Graves", &["necro"], 55.0),
    hero("Grey Talon", &["orion", "archer"], 77.0),
    hero("Haze", &["haze"], 50.0),
    hero("Holliday", &["astro"], 26.0),
    hero("Infernus", &["inferno"], 100.0),
    hero("Ivy", &["tengu"], 72.0),
    hero("Kelvin", &["kelvin"], 47.0),
    hero("Lady Geist", &["ghost", "spectre"], 87.0),
    hero("Lash", &["lash"], 54.0),
    hero("McGinnis", &["forge", "engineer"], 22.0),
    hero("Mina", &["vampirebat"], 54.0),
    hero("Mirage", &["mirage"], 65.0),
    hero("Mo & Krill", &["krill", "digger", "mokrill"], 100.0),
    hero("Nurse Harrow", &["nurse"], DEFAULT_FACE_X),
    hero("Paige", &["bookworm"], 42.0),
    hero("Paradox", &["chrono"], 59.0),
    hero("Pocket", &["synth"], 61.0),
    hero("Rat King", &["ratking"], DEFAULT_FACE_X),
    hero("Rem", &["familiar"], 55.0),
    hero("Seven", &["gigawatt"], 57.0),
    hero("Shiv", &["shiv"], 68.0),
    hero("Silver", &["werewolf"], 55.0),
    hero("Sinclair", &["magician"], 61.0),
    hero("Solomon", &["chessmaster"], DEFAULT_FACE_X),
    hero("Venator", &["priest"], 55.0),
    hero("Victor", &["frank"], 45.0),
    hero("Vindicta", &["hornet"], 83.0),
    hero("Violet", &["artist"], 55.0),
    hero("Viscous", &["viscous"], 72.0),
    hero("Vyper", &["viper"], 48.0),
    hero("Warden", &["warden"], 55.0),
    hero("Wraith", &["wraith"], 56.0),
    hero("Yamato", &["yamato"], 56.0),
];

/// The roster hero for a display name or codename, in any case or spacing:
/// `Lady Geist`, `lady_geist`, `ghost`, `The Doorman`.
pub fn roster_hero(name: &str) -> Option<&'static RosterHero> {
    let key = hero_asset_key(name);

    if key.is_empty() {
        return None;
    }

    ROSTER
        .iter()
        .find(|hero| hero_asset_key(hero.name) == key)
        .or_else(|| hero_for_codename(name))
}

/// The roster hero a catalog or mod folder name belongs to, or `None` for an
/// NPC, test rig, shared folder or anything unknown.
pub fn hero_for_codename(folder: &str) -> Option<&'static RosterHero> {
    let lowered = folder.trim().to_ascii_lowercase().replace('-', "_");
    let mut name = lowered.strip_prefix("hero_").unwrap_or(&lowered).to_string();

    if name.is_empty() {
        return None;
    }

    if let Some(hero) = exact_codename(&name) {
        return Some(hero);
    }

    // `inferno_v4`, `hornet_v3`, `yamato_v2`.
    while let Some(stripped) = strip_version_suffix(&name) {
        name = stripped;

        if let Some(hero) = exact_codename(&name) {
            return Some(hero);
        }
    }

    // A display-name folder: `lady_geist`, `mo_and_krill`, `grey_talon`.
    let key = hero_asset_key(&name);
    if let Some(hero) = ROSTER.iter().find(|hero| hero_asset_key(hero.name) == key) {
        return Some(hero);
    }

    // A variant folder: `atlas_detective`, `wraith_gen_man`, `gigawatt_prisoner`.
    ROSTER
        .iter()
        .flat_map(|hero| hero.codenames.iter().map(move |code| (hero, *code)))
        .filter(|(_, code)| {
            name.len() > code.len()
                && name.starts_with(code)
                && name.as_bytes()[code.len()] == b'_'
        })
        .max_by_key(|(_, code)| code.len())
        .map(|(hero, _)| hero)
}

fn exact_codename(name: &str) -> Option<&'static RosterHero> {
    ROSTER
        .iter()
        .find(|hero| hero.codenames.iter().any(|code| *code == name))
}

fn strip_version_suffix(name: &str) -> Option<String> {
    let (head, tail) = name.rsplit_once('_')?;
    let digits = tail.strip_prefix('v')?;

    if head.is_empty() || digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    Some(head.to_string())
}

/// Horizontal face position for a hero's render, in percent.
pub fn face_position_x(name: &str) -> f32 {
    roster_hero(name)
        .map(|hero| hero.face_x)
        .unwrap_or(DEFAULT_FACE_X)
}

/// One hero in the Locker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerHero {
    /// The hero's current codename, for example `ghost` for Lady Geist.
    pub codename: String,
    /// UI label, for example `Lady Geist`.
    pub display_name: String,
    /// Render art on disk, if the configured asset root has one.
    pub render_path: Option<PathBuf>,
    /// Name art on disk, if the configured asset root has one.
    pub name_art_path: Option<PathBuf>,
    /// Installed addon slots that ship at least one path under this hero.
    pub mod_count: usize,
    /// Catalog entries under this hero's folders.
    pub asset_count: usize,
    /// False when the catalog has no path for this hero: the game is not
    /// indexed yet, or the hero is newer than this build.
    #[serde(default = "default_true")]
    pub in_catalog: bool,
    /// Recolorable catalog paths under the hero's folders, sorted and capped.
    #[serde(default)]
    pub recolor_paths: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Grimoire's General classification: what kind of non-hero mod this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeneralType {
    SoulContainer,
    SpiritUrn,
    /// The Hidden King and the Archmother (`models/npc/patron_amber`, `patron_sapphire`).
    Patrons,
    /// The Sinner's Sacrifice vault.
    Sinners,
    /// Breakable crates and boxes.
    Boxes,
    Hideout,
    Icons,
    Hud,
    Effects,
    Animations,
    Maps,
    Announcer,
    KillstreakMusic,
}

impl GeneralType {
    /// Rail order, as Grimoire's `GLOBAL_MOD_TYPE_ORDER`.
    pub const ALL: [GeneralType; 13] = [
        GeneralType::SoulContainer,
        GeneralType::SpiritUrn,
        GeneralType::Patrons,
        GeneralType::Sinners,
        GeneralType::Boxes,
        GeneralType::Hideout,
        GeneralType::Icons,
        GeneralType::Hud,
        GeneralType::Effects,
        GeneralType::Animations,
        GeneralType::Maps,
        GeneralType::Announcer,
        GeneralType::KillstreakMusic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GeneralType::SoulContainer => "Soul Containers",
            GeneralType::SpiritUrn => "Spirit Urns",
            GeneralType::Patrons => "Patrons",
            GeneralType::Sinners => "Sinners",
            GeneralType::Boxes => "Boxes",
            GeneralType::Hideout => "Hideout",
            GeneralType::Icons => "Icon Packs",
            GeneralType::Hud => "HUD",
            GeneralType::Effects => "Effects",
            GeneralType::Animations => "Animations",
            GeneralType::Maps => "Maps",
            GeneralType::Announcer => "Announcer / SFX",
            GeneralType::KillstreakMusic => "Killstreak Music",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            GeneralType::SoulContainer => "soul-container",
            GeneralType::SpiritUrn => "spirit-urn",
            GeneralType::Patrons => "patron",
            GeneralType::Sinners => "sinner",
            GeneralType::Boxes => "boxes",
            GeneralType::Hideout => "hideout",
            GeneralType::Icons => "icons",
            GeneralType::Hud => "hud",
            GeneralType::Effects => "effects",
            GeneralType::Animations => "animations",
            GeneralType::Maps => "maps",
            GeneralType::Announcer => "announcer",
            GeneralType::KillstreakMusic => "killstreak-music",
        }
    }
}

/// The General type a set of game paths points at, by the most common signal.
/// `None` when nothing matches: the mod stays out of the General types and is
/// only reachable through a user category.
pub fn classify_general(paths: &[String]) -> Option<GeneralType> {
    let mut votes: BTreeMap<GeneralType, usize> = BTreeMap::new();

    for path in paths {
        if let Some(kind) = general_signal(&path.to_ascii_lowercase()) {
            *votes.entry(kind).or_insert(0) += 1;
        }
    }

    // Highest count wins; ties go to the earlier rail entry.
    votes
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then(right.0.cmp(&left.0)))
        .map(|(kind, _)| kind)
}

fn general_signal(path: &str) -> Option<GeneralType> {
    // Sounds first: a Spirit Urn voice line, a vault or patron sound is a sound, not the
    // model. Grimoire's `fix/gb-category-drift` sorts these world, combat and UI sounds onto
    // Announcer / SFX; music keeps its own type.
    if path.starts_with("sounds/") || path.starts_with("soundevents/") {
        if path.starts_with("sounds/music") || path.contains("killstreak") {
            return Some(GeneralType::KillstreakMusic);
        }
        return Some(GeneralType::Announcer);
    }
    if path.contains("soul_container") || path.contains("soulcontainer") {
        return Some(GeneralType::SoulContainer);
    }
    if path.contains("spirit_urn")
        || path.contains("/idol_urn/")
        || path.contains("/urn/")
        || path.contains("/urn_")
    {
        return Some(GeneralType::SpiritUrn);
    }
    // Grimoire's `detectGeneralPropTypes`: the two patron models and the vault, models,
    // materials and texture-only overrides alike.
    if path.contains("/patron_amber") || path.contains("/patron_sapphire") {
        return Some(GeneralType::Patrons);
    }
    if path.contains("/sinners_sacrifice") {
        return Some(GeneralType::Sinners);
    }
    if is_box_path(path) {
        return Some(GeneralType::Boxes);
    }
    if path.contains("hideout") {
        return Some(GeneralType::Hideout);
    }
    if path.contains("killstreak") {
        return Some(GeneralType::KillstreakMusic);
    }
    if path.contains("announcer") {
        return Some(GeneralType::Announcer);
    }
    if path.starts_with("panorama/images/heroes/")
        || path.starts_with("panorama/images/items/")
        || path.starts_with("panorama/images/abilities/")
        || path.contains("/icons/")
    {
        return Some(GeneralType::Icons);
    }
    if path.starts_with("panorama/") {
        return Some(GeneralType::Hud);
    }
    if path.contains("animgraph") || path.ends_with(".vnmgraph_c") || path.contains("/animation") {
        return Some(GeneralType::Animations);
    }
    if path.starts_with("particles/") {
        return Some(GeneralType::Effects);
    }
    if path.starts_with("maps/") {
        return Some(GeneralType::Maps);
    }
    None
}

/// A breakable crate or box: a gameplay prop model or material whose name says so. Decor
/// crates in the street and market prop folders are map art, not the breakables.
fn is_box_path(path: &str) -> bool {
    let model = path.strip_prefix("materials/").unwrap_or(path);
    model.starts_with("models/props_gameplay/")
        && (model.contains("crate") || model.contains("breakable"))
}

/// The General type a GameBanana category name points at, as Grimoire's
/// `categoryGeneralType`: `Patrons`, `Hidden King`, `Sinner's Sacrifice`, `Crates`. Used for
/// a mod whose files name no hero and no General type, such as a box retexture filed under
/// Boxes.
pub fn general_for_category(name: &str) -> Option<GeneralType> {
    let lowered = name.trim().to_lowercase().replace('\u{2019}', "'");
    let general = match lowered.as_str() {
        "soul container" | "soul containers" => GeneralType::SoulContainer,
        "spirit urn" | "spirit urns" => GeneralType::SpiritUrn,
        "patron" | "patrons" | "hidden king" | "archmother" => GeneralType::Patrons,
        "sinners" | "sinner" | "sinner's sacrifice" => GeneralType::Sinners,
        "boxes" | "crates" => GeneralType::Boxes,
        "hideout" => GeneralType::Hideout,
        "hud" => GeneralType::Hud,
        "killstreak music" => GeneralType::KillstreakMusic,
        "animations" => GeneralType::Animations,
        "maps" => GeneralType::Maps,
        _ => return None,
    };
    Some(general)
}

/// One ability sound a mod replaces, from `sounds/abilities/<hero>/...`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerSound {
    /// Roster hero the sound belongs to.
    pub hero: String,
    /// Ability slot 1 to 3, 4 for the ultimate, or 0 when the path names none.
    pub ability: u8,
}

/// One installed addon slot as the Locker sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerMod {
    pub slot: u16,
    /// File name on disk, `.disabled` suffix included.
    pub file: String,
    pub enabled: bool,
    /// Content identity from the directory tree (paths and CRCs). It survives
    /// the enable rename and a slot move, so category membership follows it.
    pub key: String,
    pub entry_count: usize,
    /// Roster heroes whose folders the mod ships paths under.
    pub heroes: Vec<String>,
    /// General type when the mod touches no hero.
    pub general: Option<GeneralType>,
    /// Ability sounds the mod replaces, sorted and deduplicated.
    #[serde(default)]
    pub sounds: Vec<LockerSound>,
}

/// Everything the Locker screen shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerLibrary {
    /// The whole roster, alphabetical.
    pub heroes: Vec<LockerHero>,
    /// Every installed addon slot that could be read, by slot.
    pub mods: Vec<LockerMod>,
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
    /// Backdrop for the General tile and page, Grimoire
    /// `public/locker/global-bg.webp`.
    #[serde(default)]
    pub general_backdrop: Option<PathBuf>,
}

impl LockerAssetPaths {
    /// Resolve from `DAH_LOCKER_ASSETS`, then art installed with the app, then
    /// `assets/locker` under the working directory, then a development
    /// Grimoire checkout, then nothing.
    pub fn discover() -> Self {
        let configured = Self::from_env();
        if configured.has_heroes() {
            return configured;
        }

        let installed = Self::from_app_install();
        if installed.has_heroes() {
            return installed;
        }

        if let Ok(cwd) = std::env::current_dir() {
            let local = Self::from_root(&cwd.join("assets").join("locker"));
            if local.has_heroes() {
                return local;
            }
        }

        Self::from_vendor_checkout()
    }

    /// Art installed with the running executable. See [`Self::from_exe_dir`].
    pub fn from_app_install() -> Self {
        let Ok(exe) = std::env::current_exe() else {
            return Self::default();
        };

        let mut dirs = Vec::new();
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
        }
        // A symlink or a launcher shim: the art sits beside the real binary.
        if let Ok(real) = fs::canonicalize(&exe)
            && let Some(dir) = real.parent()
            && !dirs.iter().any(|known| known == dir)
        {
            dirs.push(dir.to_path_buf());
        }

        dirs.iter()
            .map(|dir| Self::from_exe_dir(dir))
            .find(Self::has_heroes)
            .unwrap_or_default()
    }

    /// Art beside an executable in `dir`: `assets/locker` (the Windows and
    /// Linux bundles), `locker`, or `../Resources/locker` (a macOS bundle).
    /// The first root with hero renders wins.
    pub fn from_exe_dir(dir: &Path) -> Self {
        [
            dir.join("assets").join("locker"),
            dir.join("locker"),
            dir.join("..").join("Resources").join("locker"),
        ]
        .iter()
        .map(|root| Self::from_root(root))
        .find(Self::has_heroes)
        .unwrap_or_default()
    }

    /// Paths from `DAH_LOCKER_ASSETS` only. Unset or empty yields no paths.
    pub fn from_env() -> Self {
        match std::env::var_os(LOCKER_ASSETS_ENV) {
            Some(value) if !value.is_empty() => Self::from_root(Path::new(&value)),
            _ => Self::default(),
        }
    }

    /// Paths from a root that holds `heroes/`, `names/`, `hero_portraits/` and
    /// optionally `global-bg.*`. Missing entries stay `None`.
    pub fn from_root(root: &Path) -> Self {
        Self {
            heroes_dir: existing_dir(root.join("heroes")),
            names_dir: existing_dir(root.join("names")),
            hero_portraits_dir: existing_dir(root.join("hero_portraits")),
            general_backdrop: first_file(root, "global-bg"),
        }
    }

    /// Paths from a Grimoire `public/` folder.
    pub fn from_grimoire_public(public: &Path) -> Self {
        Self {
            heroes_dir: existing_dir(public.join("locker").join("heroes")),
            names_dir: existing_dir(public.join("locker").join("names")),
            hero_portraits_dir: existing_dir(public.join("heroes")),
            general_backdrop: first_file(&public.join("locker"), "global-bg"),
        }
    }

    /// Look for `vendor/grimoire/public` in the current directory and its
    /// parents. This is the development fallback.
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

    /// True when a render directory exists, the one the grid needs.
    pub fn has_heroes(&self) -> bool {
        self.heroes_dir.is_some() || self.hero_portraits_dir.is_some()
    }
}

/// List every roster hero with local art and mod counts attached.
///
/// Asset paths come from [`LockerAssetPaths::discover`].
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
    let addons_dir = addons_directory(catalog);
    discover_locker(catalog, addons, addons_dir.as_deref(), assets).heroes
}

/// The roster with catalog facts and art, plus every readable installed mod
/// with its heroes and General type. `addons_dir` is the folder the addon
/// files live in; `None` skips reading them.
pub fn discover_locker(
    catalog: &GameCatalog,
    addons: &[InstalledAddon],
    addons_dir: Option<&Path>,
    assets: &LockerAssetPaths,
) -> LockerLibrary {
    let resolver = AssetResolver::build(assets);
    let mods = read_locker_mods(addons, addons_dir);

    let mut asset_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut recolor: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();

    for entry in &catalog.entries {
        let lowered = entry.path.to_ascii_lowercase();
        let Some(hero) = hero_of_path(&lowered) else {
            continue;
        };

        *asset_counts.entry(hero.name).or_insert(0) += 1;

        if entry.path.ends_with(".vmat_c") || entry.path.ends_with(".vtex_c") {
            recolor
                .entry(hero.name)
                .or_default()
                .insert(entry.path.clone());
        }
    }

    let heroes = ROSTER
        .iter()
        .map(|hero| {
            let asset_count = asset_counts.get(hero.name).copied().unwrap_or(0);
            let mut recolor_paths: Vec<String> = recolor
                .get(hero.name)
                .map(|paths| paths.iter().cloned().collect())
                .unwrap_or_default();
            recolor_paths.truncate(RECOLOR_PATH_CAP);

            LockerHero {
                codename: hero.codenames[0].to_string(),
                display_name: hero.name.to_string(),
                render_path: resolver.render_for(hero),
                name_art_path: resolver.name_art_for(hero),
                mod_count: mods
                    .iter()
                    .filter(|item| item.heroes.iter().any(|name| name == hero.name))
                    .count(),
                asset_count,
                in_catalog: asset_count > 0,
                recolor_paths,
            }
        })
        .collect();

    LockerLibrary { heroes, mods }
}

fn read_locker_mods(addons: &[InstalledAddon], addons_dir: Option<&Path>) -> Vec<LockerMod> {
    let Some(directory) = addons_dir.filter(|dir| dir.is_dir()) else {
        return Vec::new();
    };

    let mut mods = Vec::new();

    for addon in addons {
        let Ok(parsed) = read_vpk_directory_with(
            &addon.path(&directory),
            &ReadVpkOptions {
                keep_preload: false,
            },
        ) else {
            continue;
        };

        let paths: Vec<String> = parsed.entries.iter().map(|entry| entry.path.clone()).collect();
        let crcs: Vec<(String, u32)> = parsed
            .entries
            .iter()
            .map(|entry| (entry.path.to_ascii_lowercase(), entry.crc32))
            .collect();
        mods.push(locker_mod(addon, &paths, &crcs));
    }

    mods.sort_by_key(|item| item.slot);
    mods
}

/// Classify one addon from its directory listing. Public for the desktop's
/// demo data and for tests.
pub fn locker_mod(addon: &InstalledAddon, paths: &[String], crcs: &[(String, u32)]) -> LockerMod {
    let mut heroes = BTreeSet::new();
    let mut sounds = BTreeSet::new();

    for path in paths {
        let lowered = path.to_ascii_lowercase();

        if let Some(hero) = hero_of_path(&lowered) {
            heroes.insert(hero.name.to_string());
        }

        if let Some(sound) = locker_sound(&lowered).or_else(|| hero_sound(&lowered)) {
            sounds.insert(sound);
        }
    }

    // Hero-less sounds (patron and newscaster lines, urn and vault sounds, shared parry and
    // hit feedback, UI) are Announcer / SFX, and so is a pack that revoices three or more
    // heroes. A sound pack for one or two heroes belongs on those heroes' sound lists.
    let sound_heroes: BTreeSet<&str> = sounds.iter().map(|sound| sound.hero.as_str()).collect();
    let general = if heroes.is_empty() {
        classify_general(paths).filter(|general| {
            *general != GeneralType::Announcer || sound_heroes.is_empty() || sound_heroes.len() >= 3
        })
    } else {
        None
    };

    LockerMod {
        slot: addon.slot,
        file: addon.file.clone(),
        enabled: addon.enabled,
        key: content_key(crcs),
        entry_count: paths.len(),
        heroes: heroes.into_iter().collect(),
        general,
        sounds: sounds.into_iter().collect(),
    }
}

/// The ability a sound path names, or `None` when it is not an ability sound.
///
/// Deadlock ability sounds live at `sounds/abilities/<codename>/...`. The slot
/// comes from an `aN` token in the rest of the path (`a4_ult/`, bare `a4/`,
/// or `_a3_`), so a pack can file one ability at a time. `ability` is 0 when
/// the path names no slot: it still belongs to the hero, just not to one ability.
pub fn locker_sound(path_lower: &str) -> Option<LockerSound> {
    let rest = path_lower.strip_prefix("sounds/abilities/")?;
    let (folder, tail) = rest.split_once('/')?;
    let hero = hero_for_codename(folder)?;

    Some(LockerSound {
        hero: hero.name.to_string(),
        ability: ability_slot(tail),
    })
}

/// A hero sound that is not filed under one ability: voice lines (`sounds/vo/<codename>/`),
/// footsteps (`sounds/player/footsteps/<codename>/`), `sounds/heroes*/<codename>/`, a
/// `hero_<codename>` folder anywhere under `sounds/`, and the hero's sound event file
/// (`soundevents/hero/hero_<codename>.vsndevts`). The ability is 0. Ported from Grimoire's
/// `INSTALLED_HERO_PATH_PATTERNS` (`fix/gb-category-drift`), so a VO pack whose title skips
/// the hero still lands on it. NPC folders (`patron`, `shopkeeper`) name no hero.
pub fn hero_sound(path_lower: &str) -> Option<LockerSound> {
    let hero = sound_hero_folder(path_lower).and_then(hero_for_codename)?;
    Some(LockerSound {
        hero: hero.name.to_string(),
        ability: 0,
    })
}

fn sound_hero_folder(path: &str) -> Option<&str> {
    if let Some(rest) = path.strip_prefix("soundevents/") {
        let (folder, file) = rest.split_once('/')?;
        if folder != "hero" && folder != "citadel" {
            return None;
        }
        let stem = file
            .strip_suffix(".vsndevts_c")
            .or_else(|| file.strip_suffix(".vsndevts"))?;
        return Some(stem.strip_prefix("hero_").unwrap_or(stem));
    }
    let rest = path.strip_prefix("sounds/")?;
    let segments: Vec<&str> = rest.split('/').collect();
    // The folder must have something inside it: a bare file names no hero.
    let folder_at = |at: usize| (segments.len() > at + 1).then(|| segments[at]);
    match segments.first().copied() {
        Some("vo") => folder_at(1),
        Some("player") if segments.get(1) == Some(&"footsteps") => folder_at(2),
        Some(first) if first.starts_with("heroes") => folder_at(1),
        _ => segments
            .iter()
            .take(segments.len().saturating_sub(1))
            .find_map(|segment| segment.strip_prefix("hero_")),
    }
}

/// The ability slot an `aN` token names: 1 to 4, else 0.
pub fn ability_slot(tail: &str) -> u8 {
    for segment in tail.split('/') {
        for token in segment.split(['_', '-', '.']) {
            let bytes = token.as_bytes();

            if bytes.len() == 2 && bytes[0].eq_ignore_ascii_case(&b'a') && (b'1'..=b'4').contains(&bytes[1])
            {
                return bytes[1] - b'0';
            }
        }
    }

    0
}

/// `vpk:<16 hex>` from the sorted lowercase paths and their CRCs (FNV-1a 64).
pub fn content_key(entries: &[(String, u32)]) -> String {
    let mut sorted: Vec<&(String, u32)> = entries.iter().collect();
    sorted.sort();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };

    for (path, crc) in sorted {
        feed(path.as_bytes());
        feed(&[0]);
        feed(&crc.to_le_bytes());
    }

    format!("vpk:{hash:016x}")
}

/// Resolve the render for one hero against the configured asset paths.
pub fn resolve_render(hero: &LockerHero, assets: &LockerAssetPaths) -> Option<PathBuf> {
    let resolver = AssetResolver::build(assets);
    match roster_hero(&hero.display_name).or_else(|| roster_hero(&hero.codename)) {
        Some(roster) => resolver.render_for(roster),
        None => resolver.lookup(&resolver.renders, &[&hero.display_name, &hero.codename]),
    }
}

/// Resolve the name art for one hero. Same naming rules as [`resolve_render`].
pub fn resolve_name_art(hero: &LockerHero, assets: &LockerAssetPaths) -> Option<PathBuf> {
    let resolver = AssetResolver::build(assets);
    match roster_hero(&hero.display_name).or_else(|| roster_hero(&hero.codename)) {
        Some(roster) => resolver.name_art_for(roster),
        None => resolver.lookup(&resolver.names, &[&hero.display_name, &hero.codename]),
    }
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

/// Display label for a codename or folder: the roster name when it maps to a
/// hero, else a title-cased fallback.
pub fn display_name_for(codename: &str) -> String {
    if let Some(hero) = hero_for_codename(codename) {
        return hero.name.to_string();
    }

    let lowered = codename.trim().to_ascii_lowercase().replace('-', "_");
    title_case(lowered.strip_prefix("hero_").unwrap_or(&lowered))
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

    fn render_for(&self, hero: &RosterHero) -> Option<PathBuf> {
        self.lookup(&self.renders, &hero_lookup_names(hero))
    }

    fn name_art_for(&self, hero: &RosterHero) -> Option<PathBuf> {
        self.lookup(&self.names, &hero_lookup_names(hero))
    }

    fn lookup(&self, index: &BTreeMap<String, PathBuf>, names: &[&str]) -> Option<PathBuf> {
        names
            .iter()
            .map(|name| hero_asset_key(name))
            .filter(|key| !key.is_empty())
            .find_map(|key| index.get(&key).cloned())
    }
}

fn hero_lookup_names(hero: &RosterHero) -> Vec<&'static str> {
    let mut names = vec![hero.name];
    names.extend(hero.codenames.iter().copied());
    names
}

fn collect_files(directory: &Path, suffix: &str, index: &mut BTreeMap<String, PathBuf>) {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return;
    };

    let mut files: Vec<PathBuf> = read_dir
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    // Stable winner when two files share a key (`Doorman` and `The_Doorman`).
    files.sort();

    for path in files {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if !is_image_name(name) {
            continue;
        }

        let stem = Path::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(name);
        let key = hero_asset_key(strip_suffix_ignore_case(stem, suffix));

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
    if value.len() >= suffix.len() && value.is_char_boundary(value.len() - suffix.len()) {
        let tail = &value[value.len() - suffix.len()..];

        if tail.eq_ignore_ascii_case(suffix) {
            return &value[..value.len() - suffix.len()];
        }
    }

    value
}

/// The roster hero a lowercase game path belongs to, through the folder after
/// any `heroes*` segment: `models/heroes_staging/inferno_v4/...`,
/// `materials/models/heroes_wip/punkgoat/...`, `sounds/heroes/haze/...`.
pub(crate) fn hero_of_path(path_lower: &str) -> Option<&'static RosterHero> {
    let segments: Vec<&str> = path_lower.split('/').collect();
    let heroes_at = segments
        .iter()
        .position(|segment| segment.starts_with("heroes"))?;
    let folder = *segments.get(heroes_at + 1)?;

    // The folder must have something inside it; a bare file is not a hero.
    if folder.is_empty() || segments.len() <= heroes_at + 2 {
        return None;
    }

    hero_for_codename(folder)
}

/// The addons folder sits beside the citadel VPK, so the catalog already knows
/// where it is.
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

fn first_file(dir: &Path, stem: &str) -> Option<PathBuf> {
    ["webp", "jpg", "png"]
        .iter()
        .map(|extension| dir.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
}

/// Recolorable entries for a codename, kept for callers that hold a catalog
/// slice rather than a library.
pub fn recolor_paths_for(entries: &[CatalogEntry], hero_name: &str) -> Vec<String> {
    let Some(wanted) = roster_hero(hero_name) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = entries
        .iter()
        .filter(|entry| entry.path.ends_with(".vmat_c") || entry.path.ends_with(".vtex_c"))
        .filter(|entry| {
            hero_of_path(&entry.path.to_ascii_lowercase()).is_some_and(|hero| hero.name == wanted.name)
        })
        .map(|entry| entry.path.clone())
        .collect();
    paths.sort();
    paths.dedup();
    paths.truncate(RECOLOR_PATH_CAP);
    paths
}

// --- categories ------------------------------------------------------------

/// A user pile in the Locker's General view. Ported from Grimoire's
/// `lockerCategories.ts`: a view grouping only. Keys are [`LockerMod::key`]
/// values; keys with no installed mod are kept so a reinstall finds its pile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockerCategory {
    /// Stable across renames, derived from the first name.
    pub id: String,
    pub name: String,
    pub keys: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct CategoryStore {
    schema: u32,
    categories: Vec<LockerCategory>,
}

/// Read the category store. A missing or unreadable file is an empty list;
/// invalid entries are dropped.
pub fn read_categories(path: &Path) -> Vec<LockerCategory> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let raw: Vec<serde_json::Value> = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Array(items)) => items,
        Ok(serde_json::Value::Object(mut object)) => match object.remove("categories") {
            Some(serde_json::Value::Array(items)) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    let mut taken = BTreeSet::new();
    raw.into_iter()
        .filter_map(|value| serde_json::from_value::<LockerCategory>(value).ok())
        .filter_map(|category| {
            let id = category.id.trim().to_string();
            let name = normalize_category_name(&category.name);

            if id.is_empty() || name.is_empty() || !taken.insert(id.clone()) {
                return None;
            }

            let mut keys = Vec::new();
            for key in category.keys {
                if !key.is_empty() && !keys.contains(&key) {
                    keys.push(key);
                }
            }

            Some(LockerCategory { id, name, keys })
        })
        .collect()
}

/// Write the category store atomically: a sibling temp file, then a rename.
pub fn write_categories(path: &Path, categories: &[LockerCategory]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let store = CategoryStore {
        schema: CATEGORY_SCHEMA,
        categories: categories.to_vec(),
    };
    let text = serde_json::to_string_pretty(&store)
        .map_err(|error| Error::msg(format!("Locker categories: {error}")))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)?;
    Ok(())
}

fn normalize_category_name(raw: &str) -> String {
    raw.trim().chars().take(CATEGORY_NAME_MAX).collect()
}

fn category_token(name: &str) -> String {
    name.trim().to_lowercase()
}

fn category_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;

    for character in name.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character);
            dash = false;
        } else {
            dash = true;
        }
    }

    slug
}

/// Add a category, or return the existing one when the name is taken (case
/// insensitive). A blank name changes nothing and returns `None`.
pub fn create_category(
    categories: &[LockerCategory],
    raw_name: &str,
) -> (Vec<LockerCategory>, Option<String>) {
    let name = normalize_category_name(raw_name);
    let mut next = categories.to_vec();

    if name.is_empty() {
        return (next, None);
    }

    let token = category_token(&name);
    if let Some(existing) = categories
        .iter()
        .find(|category| category_token(&category.name) == token)
    {
        return (next, Some(existing.id.clone()));
    }

    let seed = match category_slug(&name) {
        slug if slug.is_empty() => "category".to_string(),
        slug => slug,
    };
    let taken: BTreeSet<&str> = categories.iter().map(|category| category.id.as_str()).collect();
    let id = if taken.contains(seed.as_str()) {
        (2..)
            .map(|suffix| format!("{seed}-{suffix}"))
            .find(|candidate| !taken.contains(candidate.as_str()))
            .expect("an unused suffix exists")
    } else {
        seed
    };

    next.push(LockerCategory {
        id: id.clone(),
        name,
        keys: Vec::new(),
    });
    (next, Some(id))
}

/// Rename in place. A blank name, an unknown id or a name another category
/// already uses changes nothing; the flag says whether it applied.
pub fn rename_category(
    categories: &[LockerCategory],
    id: &str,
    raw_name: &str,
) -> (Vec<LockerCategory>, bool) {
    let name = normalize_category_name(raw_name);
    let next = categories.to_vec();

    if name.is_empty() || !categories.iter().any(|category| category.id == id) {
        return (next, false);
    }

    let token = category_token(&name);
    if categories
        .iter()
        .any(|category| category.id != id && category_token(&category.name) == token)
    {
        return (next, false);
    }

    let next = categories
        .iter()
        .map(|category| {
            if category.id == id {
                LockerCategory {
                    name: name.clone(),
                    ..category.clone()
                }
            } else {
                category.clone()
            }
        })
        .collect();
    (next, true)
}

/// Remove a category. Its mods are untouched.
pub fn delete_category(categories: &[LockerCategory], id: &str) -> Vec<LockerCategory> {
    categories
        .iter()
        .filter(|category| category.id != id)
        .cloned()
        .collect()
}

/// File a mod into a category, idempotently.
pub fn add_category_membership(
    categories: &[LockerCategory],
    id: &str,
    key: &str,
) -> Vec<LockerCategory> {
    categories
        .iter()
        .map(|category| {
            let mut category = category.clone();
            if category.id == id && !key.is_empty() && !category.keys.iter().any(|k| k == key) {
                category.keys.push(key.to_string());
            }
            category
        })
        .collect()
}

/// Add or remove a mod from a category.
pub fn toggle_category_membership(
    categories: &[LockerCategory],
    id: &str,
    key: &str,
) -> Vec<LockerCategory> {
    categories
        .iter()
        .map(|category| {
            let mut category = category.clone();
            if category.id == id && !key.is_empty() {
                if category.keys.iter().any(|k| k == key) {
                    category.keys.retain(|k| k != key);
                } else {
                    category.keys.push(key.to_string());
                }
            }
            category
        })
        .collect()
}

// --- preferences: favourites, shuffle and custom cards ----------------------

/// A card image the user chose for one hero. The file is a copy in the app data
/// directory, never the original path, so moving the original changes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomCard {
    /// File name inside [`LOCKER_CARDS_DIR`]. Never a path.
    pub file: String,
    /// Horizontal face position, percent, for the crop.
    #[serde(default = "default_face_x")]
    pub focus_x: f32,
    pub width: u32,
    pub height: u32,
}

fn default_face_x() -> f32 {
    DEFAULT_FACE_X
}

/// What the Locker remembers between runs: starred heroes, the heroes whose
/// skins shuffle on launch, and custom card art.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LockerPrefs {
    pub schema: u32,
    /// Hero asset keys ([`hero_asset_key`]), in the order starred.
    pub favorites: Vec<String>,
    /// Hero asset keys whose skins shuffle on launch.
    pub shuffle: Vec<String>,
    /// Custom card art per hero asset key.
    pub cards: BTreeMap<String, CustomCard>,
    /// Skins opted into the shuffle, by [`LockerMod::key`] (survives renames and slot moves).
    /// Empty means every skin of a shuffled hero is in the pool.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shuffle_pool: Vec<String>,
    /// Equip adds a skin instead of replacing the hero's others (several skins per hero).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub multi_skin: bool,
}

impl Default for LockerPrefs {
    fn default() -> Self {
        LockerPrefs {
            schema: PREFS_SCHEMA,
            favorites: Vec::new(),
            shuffle: Vec::new(),
            cards: BTreeMap::new(),
            shuffle_pool: Vec::new(),
            multi_skin: false,
        }
    }
}

/// A store key for any hero spelling: the roster display name when the input
/// names a hero, else the normalized text. `ghost`, `Lady Geist` and
/// `lady_geist` all reach `lady_geist`.
fn prefs_hero_key(hero: &str) -> String {
    match roster_hero(hero) {
        Some(roster) => hero_asset_key(roster.name),
        None => hero_asset_key(hero),
    }
}

impl LockerPrefs {
    fn has(list: &[String], hero: &str) -> bool {
        let key = prefs_hero_key(hero);
        !key.is_empty() && list.iter().any(|item| item == &key)
    }

    fn toggle(list: &mut Vec<String>, hero: &str) -> bool {
        let key = prefs_hero_key(hero);

        if key.is_empty() {
            return false;
        }

        if let Some(at) = list.iter().position(|item| item == &key) {
            list.remove(at);
            false
        } else {
            list.push(key);
            true
        }
    }

    pub fn is_favorite(&self, hero: &str) -> bool {
        Self::has(&self.favorites, hero)
    }

    /// Star or unstar a hero. Returns the new state.
    pub fn toggle_favorite(&mut self, hero: &str) -> bool {
        Self::toggle(&mut self.favorites, hero)
    }

    pub fn shuffles(&self, hero: &str) -> bool {
        Self::has(&self.shuffle, hero)
    }

    /// Turn shuffle on or off for one hero. Returns the new state.
    pub fn toggle_shuffle(&mut self, hero: &str) -> bool {
        Self::toggle(&mut self.shuffle, hero)
    }

    pub fn card(&self, hero: &str) -> Option<&CustomCard> {
        let key = prefs_hero_key(hero);
        self.cards.get(&key)
    }

    pub fn set_card(&mut self, hero: &str, mut card: CustomCard) {
        let key = prefs_hero_key(hero);

        if !key.is_empty() && plain_file_name(&card.file) {
            card.focus_x = card.focus_x.clamp(0.0, 100.0);
            self.cards.insert(key, card);
        }
    }

    pub fn clear_card(&mut self, hero: &str) -> Option<CustomCard> {
        let key = prefs_hero_key(hero);
        self.cards.remove(&key)
    }

    pub fn any_shuffle(&self) -> bool {
        !self.shuffle.is_empty()
    }

    /// True when the mod with this content key is opted into the shuffle pool.
    pub fn in_pool(&self, key: &str) -> bool {
        self.shuffle_pool.iter().any(|item| item == key)
    }

    /// Opt a mod in or out of the shuffle pool. Returns the new state.
    pub fn toggle_pool(&mut self, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        if let Some(at) = self.shuffle_pool.iter().position(|item| item == key) {
            self.shuffle_pool.remove(at);
            false
        } else {
            self.shuffle_pool.push(key.to_string());
            true
        }
    }

    /// The slots of the pooled mods, for [`crate::locker_parts::shuffle_wanted`]. None when the
    /// pool is empty (every skin counts).
    pub fn pool_slots(&self, mods: &[LockerMod]) -> Option<BTreeSet<u16>> {
        (!self.shuffle_pool.is_empty()).then(|| {
            mods.iter().filter(|item| self.in_pool(&item.key)).map(|item| item.slot).collect()
        })
    }

    /// How equip treats a hero's other skins.
    pub fn equip_mode(&self) -> crate::locker_parts::EquipMode {
        if self.multi_skin {
            crate::locker_parts::EquipMode::Add
        } else {
            crate::locker_parts::EquipMode::Replace
        }
    }

    fn clean(mut self) -> Self {
        let mut favorites = Vec::new();
        for hero in std::mem::take(&mut self.favorites) {
            let key = prefs_hero_key(&hero);
            if !key.is_empty() && !favorites.contains(&key) {
                favorites.push(key);
            }
        }
        let mut shuffle = Vec::new();
        for hero in std::mem::take(&mut self.shuffle) {
            let key = prefs_hero_key(&hero);
            if !key.is_empty() && !shuffle.contains(&key) {
                shuffle.push(key);
            }
        }
        let mut cards = BTreeMap::new();
        for (hero, mut card) in std::mem::take(&mut self.cards) {
            let key = prefs_hero_key(&hero);
            if key.is_empty() || !plain_file_name(&card.file) {
                continue;
            }
            card.focus_x = card.focus_x.clamp(0.0, 100.0);
            if card.width == 0 || card.height == 0 {
                continue;
            }
            cards.insert(key, card);
        }
        self.schema = PREFS_SCHEMA;
        self.favorites = favorites;
        self.shuffle = shuffle;
        self.cards = cards;
        self
    }
}

/// Read the preferences store. A missing or unreadable file is the default
/// (nothing starred, no shuffle, no custom cards); junk entries are dropped.
pub fn read_prefs(path: &Path) -> LockerPrefs {
    let Ok(text) = fs::read_to_string(path) else {
        return LockerPrefs::default();
    };

    serde_json::from_str::<LockerPrefs>(&text)
        .unwrap_or_default()
        .clean()
}

/// Write the preferences store atomically: a sibling temp file, then a rename.
pub fn write_prefs(path: &Path, prefs: &LockerPrefs) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let text = serde_json::to_string_pretty(prefs)
        .map_err(|error| Error::msg(format!("Locker preferences: {error}")))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)?;
    Ok(())
}

fn plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
}

/// Pixel size of a PNG, WebP or JPEG from its header. Core never decodes art.
pub fn art_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let le24 = |b: &[u8]| u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16;

    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return (width > 0 && height > 0).then_some((width, height));
    }

    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        let size = match &bytes[12..16] {
            b"VP8X" => (le24(&bytes[24..27]) + 1, le24(&bytes[27..30]) + 1),
            b"VP8 " => (
                u32::from(u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3fff),
                u32::from(u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3fff),
            ),
            b"VP8L" => {
                let packed = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
                ((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1)
            }
            _ => return None,
        };
        return (size.0 > 0 && size.1 > 0).then_some(size);
    }

    jpeg_dimensions(bytes)
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }

    let mut at = 2;

    while at + 9 < bytes.len() {
        if bytes[at] != 0xFF {
            at += 1;
            continue;
        }

        let marker = bytes[at + 1];

        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            at += 2;
            continue;
        }

        if marker == 0xD9 || marker == 0xDA {
            return None;
        }

        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));

        if length < 2 {
            return None;
        }

        // SOF0 to SOF15, except the huffman, arithmetic and restart tables.
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = u32::from(u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]));
            return (width > 0 && height > 0).then_some((width, height));
        }

        at += 2 + length;
    }

    None
}

fn read_header(path: &Path) -> Option<Vec<u8>> {
    let mut header = Vec::with_capacity(ART_HEADER_BYTES);
    fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut header)
        .ok()?;
    Some(header)
}

fn card_extension(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn card_slug(hero: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;

    for character in hero_asset_key(hero).chars() {
        if character.is_ascii_alphanumeric() {
            if dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character);
            dash = false;
        } else {
            dash = true;
        }
    }

    if slug.is_empty() {
        "hero".to_string()
    } else {
        slug
    }
}

/// Copy a chosen image into the custom card folder and describe it. The copy
/// is named after the hero and a hash of the source, so choosing the same file
/// twice reuses one file and nothing is ever overwritten. App data write only:
/// the game folder is never touched.
pub fn install_custom_card(
    cards_dir: &Path,
    hero: &str,
    source: &Path,
    focus_x: f32,
) -> Result<CustomCard> {
    let extension = card_extension(source).ok_or_else(|| {
        Error::msg("A custom card is a PNG, JPEG or WebP image.")
    })?;
    let meta = fs::metadata(source)
        .map_err(|error| Error::msg(format!("{}: {error}", source.display())))?;

    if !meta.is_file() {
        return Err(Error::msg(format!("{} is not a file.", source.display())));
    }

    if meta.len() == 0 || meta.len() > CUSTOM_CARD_BYTES {
        return Err(Error::msg(format!(
            "That image is {} MB. A custom card is under {} MB.",
            meta.len().div_ceil(1024 * 1024),
            CUSTOM_CARD_BYTES / (1024 * 1024)
        )));
    }

    let header = read_header(source).ok_or_else(|| {
        Error::msg(format!("{} could not be read.", source.display()))
    })?;
    let (width, height) = art_dimensions(&header)
        .ok_or_else(|| Error::msg("That file is not a PNG, JPEG or WebP image."))?;

    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source
        .to_string_lossy()
        .bytes()
        .chain(meta.len().to_le_bytes())
        .chain(mtime.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let file = format!("{}-{hash:08x}.{extension}", card_slug(hero));
    let destination = cards_dir.join(&file);
    let card = CustomCard {
        file,
        focus_x: focus_x.clamp(0.0, 100.0),
        width,
        height,
    };

    if destination.is_file() {
        return Ok(card);
    }

    fs::create_dir_all(cards_dir)?;
    let bytes = fs::read(source)?;
    let partial = cards_dir.join(format!(".{}.{}.tmp", card.file, std::process::id()));
    fs::write(&partial, &bytes)?;

    if fs::rename(&partial, &destination).is_err() {
        let _ = fs::remove_file(&partial);
        return Err(Error::msg(format!(
            "The card image could not be saved to {}.",
            cards_dir.display()
        )));
    }

    Ok(card)
}

/// Delete one copied card image. A file name with a separator is refused, so
/// only files directly in `cards_dir` can be removed. Missing is fine.
pub fn remove_custom_card(cards_dir: &Path, card: &CustomCard) -> Result<bool> {
    if !plain_file_name(&card.file) {
        return Err(Error::msg("That card file name is not a plain file name."));
    }

    match fs::remove_file(cards_dir.join(&card.file)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

// --- skins: one per hero, and the order they load in ------------------------

/// Every installed mod that touches `hero`, in load order.
pub fn hero_mods<'a>(mods: &'a [LockerMod], hero: &str) -> Vec<&'a LockerMod> {
    let key = hero_asset_key(hero);
    let mut found: Vec<&LockerMod> = mods
        .iter()
        .filter(|item| item.heroes.iter().any(|name| hero_asset_key(name) == key))
        .collect();
    found.sort_by_key(|item| item.slot);
    found
}

/// GAME WRITE. Equip one skin for a hero: turn it on and turn every other mod
/// of that hero off, so exactly one look loads. Refuses without `approve`.
pub fn equip_hero_skin(
    game_root: &Path,
    mods: &[LockerMod],
    hero: &str,
    slot: u16,
    approve: bool,
) -> Result<SetManyReport> {
    let hero_mods = hero_mods(mods, hero);

    if !hero_mods.iter().any(|item| item.slot == slot) {
        return Err(Error::msg(format!(
            "Slot {slot:02} is not one of {hero}'s installed mods. Nothing was renamed."
        )));
    }

    let wanted: BTreeMap<u16, bool> = hero_mods
        .iter()
        .map(|item| (item.slot, item.slot == slot))
        .collect();
    crate::addons::set_many(game_root, &wanted, approve)
}

/// GAME WRITE. Move one of a hero's mods to `to_index` in that hero's load
/// order. Only the hero's own slot numbers change, so unrelated mods keep
/// theirs. Refuses without `approve`.
pub fn move_hero_skin(
    game_root: &Path,
    mods: &[LockerMod],
    hero: &str,
    slot: u16,
    to_index: usize,
    approve: bool,
) -> Result<crate::load_order::ReorderReport> {
    let slots: Vec<u16> = hero_mods(mods, hero).iter().map(|item| item.slot).collect();

    if !slots.contains(&slot) {
        return Err(Error::msg(format!(
            "Slot {slot:02} is not one of {hero}'s installed mods. Nothing was renamed."
        )));
    }

    let order = crate::load_order::move_to_index(&slots, slot, to_index);
    let moves = crate::load_order::moves_for_order(&slots, &order);

    if moves.is_empty() {
        return Ok(crate::load_order::ReorderReport {
            moves: Vec::new(),
            renamed: Vec::new(),
        });
    }

    if !approve {
        return Err(Error::msg(
            "Not approved: change the skin order. Nothing was renamed.",
        ));
    }

    let directory = crate::deploy::addons_dir(game_root);
    let files = crate::load_order::read_slot_files(&directory)?;
    let plan = crate::load_order::plan_reorder(
        &directory,
        &files,
        &moves,
        &crate::load_order::new_token(),
    )?;
    crate::load_order::apply_reorder(&plan, true)
}

// --- shuffle on launch -------------------------------------------------------

fn hero_seed(seed: u64, key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let mut value = hash ^ seed;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The wanted on or off state for every mod of a shuffled hero: one pick per
/// hero, every other mod of that hero off. Pure and deterministic for a seed,
/// so the same launch picks the same skins. A mod shared by two shuffled heroes
/// stays on when either one picks it.
pub fn shuffle_picks(mods: &[LockerMod], heroes: &[String], seed: u64) -> BTreeMap<u16, bool> {
    let keys: BTreeSet<String> = heroes
        .iter()
        .map(|hero| hero_asset_key(hero))
        .filter(|key| !key.is_empty())
        .collect();
    let mut picks = BTreeSet::new();

    for key in &keys {
        let mut of: Vec<&LockerMod> = mods
            .iter()
            .filter(|item| item.heroes.iter().any(|name| hero_asset_key(name) == *key))
            .collect();

        if of.is_empty() {
            continue;
        }

        of.sort_by_key(|item| item.slot);
        let at = (hero_seed(seed, key) % of.len() as u64) as usize;
        picks.insert(of[at].slot);
    }

    mods.iter()
        .filter(|item| {
            item.heroes
                .iter()
                .any(|name| keys.contains(&hero_asset_key(name)))
        })
        .map(|item| (item.slot, picks.contains(&item.slot)))
        .collect()
}

/// GAME WRITE. Apply the launch shuffle: one random skin per shuffled hero,
/// every other mod of that hero off. Refuses without `approve`. A launch with
/// no hero in the shuffle pool writes nothing.
pub fn shuffle_for_launch(
    game_root: &Path,
    mods: &[LockerMod],
    heroes: &[String],
    seed: u64,
    approve: bool,
) -> Result<SetManyReport> {
    if !approve {
        return Err(Error::msg(
            "Not approved: shuffle the skins. Nothing was renamed.",
        ));
    }

    let wanted = shuffle_picks(mods, heroes, seed);

    if wanted.is_empty() {
        return Ok(SetManyReport::default());
    }

    crate::addons::set_many(game_root, &wanted, true)
}

// --- ability sounds ----------------------------------------------------------

/// GAME WRITE. Equip one sound for one ability of one hero: turn it on and the
/// other installed sounds for the same ability off, so exactly one plays.
/// Refuses without `approve`.
pub fn equip_ability_sound(
    game_root: &Path,
    mods: &[LockerMod],
    hero: &str,
    ability: u8,
    slot: u16,
    approve: bool,
) -> Result<SetManyReport> {
    let key = hero_asset_key(hero);
    let wanted: BTreeMap<u16, bool> = mods
        .iter()
        .filter(|item| {
            item.sounds
                .iter()
                .any(|sound| hero_asset_key(&sound.hero) == key && sound.ability == ability)
        })
        .map(|item| (item.slot, item.slot == slot))
        .collect();

    if !wanted.contains_key(&slot) {
        return Err(Error::msg(format!(
            "Slot {slot:02} has no sound for that ability. Nothing was renamed."
        )));
    }

    crate::addons::set_many(game_root, &wanted, approve)
}

// --- Global priority folder --------------------------------------------------

/// The folder that loads before `citadel/addons`: `game/citadel/grimoire`.
pub fn global_dir(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join(GLOBAL_FOLDER)
}

/// What gameinfo.gi says about the Global priority folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalPathStatus {
    /// `Game citadel/grimoire` is in SearchPaths.
    pub configured: bool,
    /// The folder exists.
    pub folder: bool,
}

/// Read the status. Never writes.
pub fn read_global_path(gameinfo_path: &Path) -> Result<GlobalPathStatus> {
    let text = match fs::read_to_string(gameinfo_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let folder = gameinfo_path
        .parent()
        .map(|citadel| citadel.join(GLOBAL_FOLDER).is_dir())
        .unwrap_or(false);

    Ok(GlobalPathStatus {
        configured: global_line_present(&text),
        folder,
    })
}

/// What [`ensure_global_path`] or [`remove_global_path`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalPathEdit {
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

fn line_is(raw: &str, key: &str, path: &str) -> bool {
    let without_ending = raw.trim_end_matches(['\r', '\n']);
    let without_comment = match without_ending.find("//") {
        Some(position) => &without_ending[..position],
        None => without_ending,
    };
    let mut parts = without_comment.split_whitespace();

    matches!(parts.next(), Some(found) if found.eq_ignore_ascii_case(key))
        && matches!(parts.next(), Some(found) if found.eq_ignore_ascii_case(path))
}

fn global_line_present(text: &str) -> bool {
    text.split_inclusive('\n')
        .any(|raw| line_is(raw, "Game", &format!("citadel/{GLOBAL_FOLDER}")))
}

fn insertion_index(text: &str) -> Option<(usize, String)> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let anchor = lines
        .iter()
        .position(|raw| line_is(raw, "Game", "citadel/addons"))
        .or_else(|| {
            lines
                .iter()
                .position(|raw| line_is(raw, "Game", "citadel"))
        })?;
    let raw = lines[anchor];
    let trimmed = raw.trim_start();
    let indent = &raw[..raw.len() - trimmed.len()];
    Some((anchor, indent.to_string()))
}

/// The text with `Game citadel/grimoire` added above the addons line, or `None`
/// when it is already there. Pure: no file access.
pub fn add_global_line(text: &str) -> Result<Option<String>> {
    if global_line_present(text) {
        return Ok(None);
    }

    let (at, indent) = insertion_index(text).ok_or_else(|| {
        Error::msg(
            "gameinfo.gi has no `Game citadel/addons` or `Game citadel` line. Verify the game files in Steam.",
        )
    })?;
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut output = String::with_capacity(text.len() + GLOBAL_SEARCH_LINE.len() + 8);

    for (index, raw) in text.split_inclusive('\n').enumerate() {
        if index == at {
            output.push_str(&indent);
            output.push_str(GLOBAL_SEARCH_LINE);
            output.push_str(ending);
        }
        output.push_str(raw);
    }

    Ok(Some(output))
}

/// The text without the `Game citadel/grimoire` line, or `None` when it is not
/// there. Pure: no file access.
pub fn remove_global_line(text: &str) -> Option<String> {
    if !global_line_present(text) {
        return None;
    }

    let mut output = String::with_capacity(text.len());

    for raw in text.split_inclusive('\n') {
        if line_is(raw, "Game", &format!("citadel/{GLOBAL_FOLDER}")) {
            continue;
        }
        output.push_str(raw);
    }

    Some(output)
}

fn global_backup(gameinfo_path: &Path) -> Result<PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let name = gameinfo_path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gameinfo.gi".to_string());
    let backup = gameinfo_path.with_file_name(format!("{name}{}{stamp}", crate::backups::STAMPED_MARKER));
    fs::copy(gameinfo_path, &backup)?;
    Ok(backup)
}

/// GAME WRITE. Mount `citadel/grimoire` in gameinfo.gi ahead of
/// `citadel/addons` and create the folder. That folder loads first, so a mod
/// placed there wins every path it shares with an addon. The original
/// gameinfo.gi is copied aside and the new text written atomically. A no-op
/// when the line is already there. Refuses without `approve`.
pub fn ensure_global_path(gameinfo_path: &Path, approve: bool) -> Result<GlobalPathEdit> {
    if !approve {
        return Err(Error::msg(format!(
            "Not approved: add the Global folder to {}. Nothing was written.",
            gameinfo_path.display()
        )));
    }

    let text = fs::read_to_string(gameinfo_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg("gameinfo.gi is missing. Verify the game files in Steam.")
        } else {
            error.into()
        }
    })?;
    let Some(updated) = add_global_line(&text)? else {
        return Ok(GlobalPathEdit {
            changed: false,
            backup: None,
        });
    };

    if let Some(citadel) = gameinfo_path.parent() {
        fs::create_dir_all(citadel.join(GLOBAL_FOLDER))?;
    }

    let backup = global_backup(gameinfo_path)?;
    crate::util::write_atomic(gameinfo_path, updated.as_bytes()).map_err(|error| {
        Error::msg(format!(
            "gameinfo.gi could not be written ({error}). Quit Deadlock and try again."
        ))
    })?;

    Ok(GlobalPathEdit {
        changed: true,
        backup: Some(backup),
    })
}

/// GAME WRITE. Remove the `Game citadel/grimoire` line. The folder and any mods
/// in it stay. The original gameinfo.gi is copied aside. Refuses without
/// `approve`.
pub fn remove_global_path(gameinfo_path: &Path, approve: bool) -> Result<GlobalPathEdit> {
    if !approve {
        return Err(Error::msg(format!(
            "Not approved: remove the Global folder from {}. Nothing was written.",
            gameinfo_path.display()
        )));
    }

    let text = fs::read_to_string(gameinfo_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg("gameinfo.gi is missing. Verify the game files in Steam.")
        } else {
            error.into()
        }
    })?;
    let Some(updated) = remove_global_line(&text) else {
        return Ok(GlobalPathEdit {
            changed: false,
            backup: None,
        });
    };
    let backup = global_backup(gameinfo_path)?;
    crate::util::write_atomic(gameinfo_path, updated.as_bytes())?;

    Ok(GlobalPathEdit {
        changed: true,
        backup: Some(backup),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Folder names from a real Deadlock catalog, including the ones that used
    /// to show up as heroes: old codenames, versioned folders, NPCs, rigs.
    const GAME_FOLDERS: &[&str] = &[
        "archer", "archer_v2", "artist", "atlas", "atlas_detective", "atlas_detective_v2",
        "astro", "bebop", "chrono", "digger", "dynamo", "engineer", "forge", "ghost", "gigawatt",
        "gigawatt_prisoner", "haze", "hornet", "hornet_v3", "inferno_v4", "kelvin", "kelvin_v2",
        "krill", "lash", "lash_v2", "magician", "mirage", "nano", "necro", "nurse", "orion",
        "priest", "punkgoat", "shiv", "synth", "tengu", "unicorn", "vampirebat", "viper",
        "viscous", "warden", "werewolf", "wraith", "wraith_gen_man", "yamato", "yamato_v2",
        "frank", "familiar", "fencer", "chessmaster", "deadpack", "bookworm", "doorman",
        "drifter", "baba", "ratking", "kali", "trooper", "neutral_camp", "slork", "boss_tier2",
        "cadence", "genericperson", "targetdummy", "test", "common",
    ];

    fn game_catalog() -> GameCatalog {
        let mut entries = Vec::new();
        for folder in GAME_FOLDERS {
            entries.push(entry(&format!("models/heroes_staging/{folder}/{folder}.vmdl_c")));
            entries.push(entry(&format!(
                "materials/models/heroes_staging/{folder}/{folder}_color.vmat_c"
            )));
        }
        GameCatalog {
            schema: 1,
            built_at: "test".to_string(),
            sources: Vec::new(),
            entries,
        }
    }

    /// The file names the Windows bundle ships in `assets/locker`.
    const PACKED_RENDERS: &[&str] = &[
        "Abrams", "Apollo", "Baba", "Bebop", "Billy", "Calico", "Celeste", "Deadman_Danny",
        "Doorman", "The_Doorman", "Drifter", "Dynamo", "Graves", "Grey_Talon", "Haze",
        "Holliday", "Infernus", "Ivy", "Kelvin", "Lady_Geist", "Lash", "McGinnis", "Mina",
        "Mirage", "Mo_&_Krill", "Nurse_Harrow", "Paige", "Paradox", "Pocket", "Rat_King", "Rem",
        "Seven", "Shiv", "Silver", "Sinclair", "Solomon", "Venator", "Victor", "Vindicta",
        "Violet", "Viscous", "Vyper", "Warden", "Wraith", "Yamato",
    ];

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dah-locker-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn write_packed_art(locker: &Path) {
        let heroes = locker.join("heroes");
        let names = locker.join("names");
        fs::create_dir_all(&heroes).unwrap();
        fs::create_dir_all(&names).unwrap();
        for name in PACKED_RENDERS {
            fs::write(heroes.join(format!("{name}_Render.webp")), b"x").unwrap();
            if *name != "The_Doorman" {
                fs::write(names.join(format!("{name}_name.webp")), b"x").unwrap();
            }
        }
        fs::write(locker.join("global-bg.webp"), b"x").unwrap();
    }

    #[test]
    fn roster_is_one_entry_per_playable_hero() {
        let names: BTreeSet<&str> = ROSTER.iter().map(|hero| hero.name).collect();
        assert_eq!(names.len(), ROSTER.len());
        let mut sorted: Vec<&str> = ROSTER.iter().map(|hero| hero.name).collect();
        sorted.sort_by_key(|name| name.to_ascii_lowercase());
        assert_eq!(sorted, ROSTER.iter().map(|hero| hero.name).collect::<Vec<_>>());
    }

    #[test]
    fn codenames_map_to_roster_heroes() {
        let name = |folder: &str| hero_for_codename(folder).map(|hero| hero.name);
        assert_eq!(name("atlas"), Some("Abrams"));
        assert_eq!(name("atlas_detective_v2"), Some("Abrams"));
        assert_eq!(name("archer"), Some("Grey Talon"));
        assert_eq!(name("archer_v2"), Some("Grey Talon"));
        assert_eq!(name("artist"), Some("Violet"));
        assert_eq!(name("astro"), Some("Holliday"));
        assert_eq!(name("inferno_v4"), Some("Infernus"));
        assert_eq!(name("hero_hornet"), Some("Vindicta"));
        assert_eq!(name("wraith_gen_man"), Some("Wraith"));
        assert_eq!(name("gigawatt_prisoner"), Some("Seven"));
        assert_eq!(name("lady_geist"), Some("Lady Geist"));
        assert_eq!(name("mo_and_krill"), Some("Mo & Krill"));
        assert_eq!(name("kali"), None);
        assert_eq!(name("trooper"), None);
        assert_eq!(name("boss_tier2"), None);
        assert_eq!(name("common"), None);
        assert_eq!(display_name_for("punkgoat"), "Billy");
        assert_eq!(display_name_for("ghost"), "Lady Geist");
        assert_eq!(hero_asset_key("Mo & Krill"), "mo_and_krill");
        assert_eq!(hero_asset_key("The Doorman"), "doorman");
    }

    #[test]
    fn a_real_catalog_lists_the_roster_once() {
        let library =
            discover_locker(&game_catalog(), &[], None, &LockerAssetPaths::default());
        assert_eq!(library.heroes.len(), ROSTER.len());

        let names: Vec<&str> = library.heroes.iter().map(|hero| hero.display_name.as_str()).collect();
        assert_eq!(names.iter().filter(|name| **name == "Abrams").count(), 1);
        for junk in ["Archer", "Archer V2", "Artist", "Boss Tier2", "Trooper", "Kali"] {
            assert!(!names.contains(&junk), "{junk} is not a hero");
        }

        let abrams = library.heroes.iter().find(|hero| hero.display_name == "Abrams").unwrap();
        assert_eq!(abrams.codename, "atlas");
        // atlas, atlas_detective, atlas_detective_v2: two paths each.
        assert_eq!(abrams.asset_count, 6);
        assert_eq!(abrams.recolor_paths.len(), 3);
        assert!(abrams.in_catalog);

        let talon = library.heroes.iter().find(|hero| hero.display_name == "Grey Talon").unwrap();
        assert_eq!(talon.asset_count, 6);
    }

    #[test]
    fn packaged_layout_resolves_art_for_every_hero() {
        // The Windows bundle: Sigil.exe beside assets/locker, run from a folder
        // with no Grimoire checkout above it and no DAH_LOCKER_ASSETS.
        let root = temp_root("bundle");
        let locker = root.join("assets").join("locker");
        write_packed_art(&locker);

        let assets = LockerAssetPaths::from_exe_dir(&root);
        assert_eq!(assets.heroes_dir.as_deref(), Some(locker.join("heroes").as_path()));
        assert_eq!(assets.general_backdrop, Some(locker.join("global-bg.webp")));

        let library = discover_locker(&game_catalog(), &[], None, &assets);
        for hero in &library.heroes {
            assert!(hero.render_path.is_some(), "{} has no render", hero.display_name);
            assert!(hero.name_art_path.is_some(), "{} has no name art", hero.display_name);
        }

        let krill = library.heroes.iter().find(|hero| hero.display_name == "Mo & Krill").unwrap();
        assert_eq!(krill.render_path, Some(locker.join("heroes").join("Mo_&_Krill_Render.webp")));

        // `locker` and a macOS bundle's Resources resolve too.
        let flat = temp_root("flat");
        write_packed_art(&flat.join("locker"));
        assert!(LockerAssetPaths::from_exe_dir(&flat).has_heroes());
        let mac = temp_root("mac");
        write_packed_art(&mac.join("Resources").join("locker"));
        fs::create_dir_all(mac.join("MacOS")).unwrap();
        assert!(LockerAssetPaths::from_exe_dir(&mac.join("MacOS")).has_heroes());
        assert!(LockerAssetPaths::from_exe_dir(&temp_root("none")).is_empty());

        for dir in [root, flat, mac] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn older_portraits_fill_in_for_missing_renders() {
        let root = temp_root("portraits");
        fs::create_dir_all(root.join("hero_portraits")).unwrap();
        fs::write(root.join("hero_portraits").join("haze.png"), b"x").unwrap();
        let assets = LockerAssetPaths::from_root(&root);
        let library = discover_locker(&game_catalog(), &[], None, &assets);
        let haze = library.heroes.iter().find(|hero| hero.display_name == "Haze").unwrap();
        assert_eq!(haze.render_path, Some(root.join("hero_portraits").join("haze.png")));
        assert!(read_hero_asset_index(&assets).contains_key("haze"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_assets_are_not_an_error() {
        let library = discover_locker(&game_catalog(), &[], None, &LockerAssetPaths::default());
        assert_eq!(library.heroes.len(), ROSTER.len());
        assert!(library.heroes.iter().all(|hero| hero.render_path.is_none()));
        assert!(library.mods.is_empty());
    }

    fn addon(slot: u16, enabled: bool) -> InstalledAddon {
        InstalledAddon {
            slot,
            file: if enabled {
                format!("pak{slot:02}_dir.vpk")
            } else {
                format!("pak{slot:02}_dir.vpk.disabled")
            },
            size_bytes: 0,
            chunk_files: Vec::new(),
            entry_count: None,
            enabled,
        }
    }

    #[test]
    fn mods_go_to_heroes_or_a_general_type() {
        let paths = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let crcs = |list: &[&str]| list.iter().map(|p| (p.to_string(), 7)).collect::<Vec<_>>();

        let skin = ["materials/models/heroes_staging/inferno_v4/body_color_psd.vtex_c"];
        let hero_mod = locker_mod(&addon(3, true), &paths(&skin), &crcs(&skin));
        assert_eq!(hero_mod.heroes, vec!["Infernus".to_string()]);
        assert_eq!(hero_mod.general, None);

        let hud = ["panorama/layout/hud.vxml_c", "panorama/styles/hud.vcss_c"];
        assert_eq!(
            locker_mod(&addon(4, false), &paths(&hud), &crcs(&hud)).general,
            Some(GeneralType::Hud)
        );
        let music = ["sounds/music/killstreak/loop.vsnd_c"];
        assert_eq!(classify_general(&paths(&music)), Some(GeneralType::KillstreakMusic));
        let fx = ["particles/abilities/bullet_trail.vpcf_c"];
        assert_eq!(classify_general(&paths(&fx)), Some(GeneralType::Effects));
        assert_eq!(classify_general(&paths(&["scripts/x.txt"])), None);

        // The key ignores slot and the enable rename.
        let a = locker_mod(&addon(4, true), &paths(&hud), &crcs(&hud));
        let b = locker_mod(&addon(9, false), &paths(&hud), &crcs(&hud));
        assert_eq!(a.key, b.key);
        assert_ne!(a.key, hero_mod.key);
    }

    #[test]
    fn patrons_sinners_and_boxes_are_general_types() {
        let paths = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(
            classify_general(&paths(&["models/npc/patron_amber/patron_amber.vmdl_c"])),
            Some(GeneralType::Patrons)
        );
        assert_eq!(
            classify_general(&paths(&["materials/models/npc/patron_sapphire/body_color.vtex_c"])),
            Some(GeneralType::Patrons)
        );
        assert_eq!(
            classify_general(&paths(&[
                "models/props_gameplay/sinners_sacrifice_vault/sinners_sacrifice.vmdl_c"
            ])),
            Some(GeneralType::Sinners)
        );
        assert_eq!(
            classify_general(&paths(&["models/props_gameplay/wooden_crate_03/wooden_crate_03.vmdl_c"])),
            Some(GeneralType::Boxes)
        );
        // A decor crate in a map prop folder is not a breakable.
        assert_eq!(classify_general(&paths(&["models/props_street/citi_crate_01.vmdl_c"])), None);
        assert_eq!(general_for_category("Crates"), Some(GeneralType::Boxes));
        assert_eq!(general_for_category("Sinner\u{2019}s Sacrifice"), Some(GeneralType::Sinners));
        assert_eq!(general_for_category("Hidden King"), Some(GeneralType::Patrons));
        assert_eq!(general_for_category("Skins"), None);
        assert_eq!(GeneralType::ALL.len(), 13);
    }

    #[test]
    fn hero_less_sounds_are_announcer_and_hero_sounds_land_on_the_hero() {
        let paths = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let crcs = |list: &[&str]| list.iter().map(|p| (p.to_string(), 7)).collect::<Vec<_>>();
        let sorted = |list: &[&str]| locker_mod(&addon(5, true), &paths(list), &crcs(list));

        // Patron VO, urn lines and vault sounds: no hero owns them.
        let world = [
            "sounds/vo/patron/patron_taunt_01.vsnd_c",
            "sounds/vo/newscaster/intro.vsnd_c",
            "sounds/gameplay/urn/urn_pickup.vsnd_c",
            "sounds/gameplay/sinners_sacrifice/vault_open.vsnd_c",
        ];
        let world = sorted(&world);
        assert_eq!(world.general, Some(GeneralType::Announcer));
        assert!(world.sounds.is_empty());
        assert!(world.heroes.is_empty());

        // A VO pack for one hero lands on that hero's sounds, not on Announcer.
        let vo = ["sounds/vo/haze/haze_kill_01.vsnd_c", "sounds/player/footsteps/haze/step.vsnd_c"];
        let vo = sorted(&vo);
        assert_eq!(vo.general, None);
        assert_eq!(vo.sounds, vec![LockerSound { hero: "Haze".into(), ability: 0 }]);

        // The sound path codename for Mo & Krill differs from the model one.
        assert_eq!(
            hero_sound("sounds/vo/mokrill/line.vsnd_c").map(|s| s.hero),
            Some("Mo & Krill".to_string())
        );
        assert_eq!(
            hero_sound("soundevents/hero/hero_inferno.vsndevts_c").map(|s| s.hero),
            Some("Infernus".to_string())
        );
        assert_eq!(hero_sound("sounds/vo/haze.vsnd_c"), None);

        // Three or more heroes revoiced is an announcer style pack.
        let many = [
            "sounds/vo/haze/a.vsnd_c",
            "sounds/vo/inferno/a.vsnd_c",
            "sounds/vo/lash/a.vsnd_c",
        ];
        assert_eq!(sorted(&many).general, Some(GeneralType::Announcer));

        // Music keeps its own type.
        assert_eq!(
            sorted(&["sounds/music/killstreak/loop.vsnd_c"]).general,
            Some(GeneralType::KillstreakMusic)
        );
    }

    #[test]
    fn categories_create_rename_file_and_persist() {
        let (cats, id) = create_category(&[], "  My favorites ");
        let id = id.unwrap();
        assert_eq!(id, "my-favorites");
        assert_eq!(cats[0].name, "My favorites");

        // Same name in another case reuses the category.
        let (cats, again) = create_category(&cats, "my FAVORITES");
        assert_eq!(again.as_deref(), Some("my-favorites"));
        assert_eq!(cats.len(), 1);
        assert_eq!(create_category(&cats, "   ").1, None);

        let (cats, second) = create_category(&cats, "HUD bits");
        let second = second.unwrap();
        let (cats, applied) = rename_category(&cats, &second, "my favorites");
        assert!(!applied, "a duplicate name is refused");
        let (cats, applied) = rename_category(&cats, &second, "Interface");
        assert!(applied);
        assert_eq!(cats[1].id, "hud-bits");
        assert_eq!(cats[1].name, "Interface");

        let cats = add_category_membership(&cats, &id, "vpk:1");
        let cats = add_category_membership(&cats, &id, "vpk:1");
        assert_eq!(cats[0].keys, vec!["vpk:1".to_string()]);
        let cats = toggle_category_membership(&cats, &id, "vpk:2");
        let cats = toggle_category_membership(&cats, &id, "vpk:1");
        assert_eq!(cats[0].keys, vec!["vpk:2".to_string()]);

        let root = temp_root("categories");
        let path = root.join(LOCKER_CATEGORIES_FILE);
        assert!(read_categories(&path).is_empty());
        write_categories(&path, &cats).unwrap();
        assert_eq!(read_categories(&path), cats);
        assert!(!path.with_extension("json.tmp").exists());

        let cats = delete_category(&cats, &id);
        assert_eq!(cats.len(), 1);

        // A bare array (Grimoire's shape) and junk entries are tolerated.
        fs::write(&path, r#"[{"id":"a","name":"A","keys":["k","k"]},{"id":"a","name":"dup"},{"bad":1}]"#).unwrap();
        let read = read_categories(&path);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].keys, vec!["k".to_string()]);
        let _ = fs::remove_dir_all(root);
    }

    fn test_mod(slot: u16, enabled: bool, heroes: &[&str]) -> LockerMod {
        LockerMod {
            slot,
            file: if enabled {
                format!("pak{slot:02}_dir.vpk")
            } else {
                format!("pak{slot:02}_dir.vpk.disabled")
            },
            enabled,
            key: format!("vpk:{slot}"),
            entry_count: 1,
            heroes: heroes.iter().map(|name| name.to_string()).collect(),
            general: None,
            sounds: Vec::new(),
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn ability_sounds_are_read_from_their_paths() {
        assert_eq!(
            locker_sound("sounds/abilities/inferno_v4/a4_ult/fire.vsnd_c"),
            Some(LockerSound {
                hero: "Infernus".to_string(),
                ability: 4
            })
        );
        assert_eq!(
            locker_sound("sounds/abilities/bull/a2_charge/charge_01.vsnd_c"),
            Some(LockerSound {
                hero: "Abrams".to_string(),
                ability: 2
            })
        );
        assert_eq!(
            locker_sound("sounds/abilities/haze/ambient/loop.vsnd_c"),
            Some(LockerSound {
                hero: "Haze".to_string(),
                ability: 0
            })
        );
        assert_eq!(locker_sound("sounds/music/loop.vsnd_c"), None);
        assert_eq!(locker_sound("sounds/abilities/trooper/a1/x.vsnd_c"), None);
        assert_eq!(ability_slot("a3_shot/fire.vsnd_c"), 3);
        assert_eq!(ability_slot("haze_a4_ult/fire.vsnd_c"), 4);
        assert_eq!(ability_slot("ambient/loop.vsnd_c"), 0);

        let paths = vec!["sounds/abilities/haze/a2_smoke/x.vsnd_c".to_string()];
        let found = locker_mod(&addon(5, true), &paths, &[(paths[0].clone(), 1)]);
        assert_eq!(
            found.sounds,
            vec![LockerSound {
                hero: "Haze".to_string(),
                ability: 2
            }]
        );
    }

    #[test]
    fn preferences_round_trip_and_drop_junk() {
        let root = temp_root("prefs");
        let path = root.join(LOCKER_PREFS_FILE);
        assert_eq!(read_prefs(&path), LockerPrefs::default());

        let mut prefs = LockerPrefs::default();
        assert!(prefs.toggle_favorite("Lady Geist"));
        assert_eq!(prefs.favorites, vec!["lady_geist".to_string()]);
        assert!(prefs.is_favorite("ghost"));
        assert!(!prefs.toggle_favorite("Lady Geist"));
        assert!(prefs.toggle_shuffle("Mo & Krill"));
        assert!(prefs.any_shuffle());
        prefs.set_card(
            "Haze",
            CustomCard {
                file: "haze-1.png".to_string(),
                focus_x: 120.0,
                width: 800,
                height: 1000,
            },
        );
        assert_eq!(prefs.card("haze").unwrap().focus_x, 100.0);

        write_prefs(&path, &prefs).unwrap();
        assert_eq!(read_prefs(&path), prefs);
        assert!(!path.with_extension("json.tmp").exists());

        fs::write(
            &path,
            r#"{"favorites":["haze","haze",""],"shuffle":["Ghost"],"cards":{"haze":{"file":"../x.png","width":1,"height":1},"bad":{"file":"ok.png","width":0,"height":1}}}"#,
        )
        .unwrap();
        let read = read_prefs(&path);
        assert_eq!(read.favorites, vec!["haze".to_string()]);
        assert_eq!(read.shuffle, vec!["lady_geist".to_string()]);
        assert!(read.cards.is_empty());

        fs::write(&path, "not json").unwrap();
        assert_eq!(read_prefs(&path), LockerPrefs::default());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_cards_copy_once_and_remove() {
        let root = temp_root("cards");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("My Portrait.png");
        fs::write(&source, png(900, 1200)).unwrap();
        let cards = root.join(LOCKER_CARDS_DIR);

        let card = install_custom_card(&cards, "Lady Geist", &source, 42.0).unwrap();
        assert!(card.file.starts_with("lady-geist-"));
        assert!(card.file.ends_with(".png"));
        assert_eq!((card.width, card.height), (900, 1200));
        assert_eq!(card.focus_x, 42.0);
        assert!(cards.join(&card.file).is_file());

        // The same source reuses one file, and no temp file is left.
        let again = install_custom_card(&cards, "lady_geist", &source, 10.0).unwrap();
        assert_eq!(again.file, card.file);
        assert_eq!(fs::read_dir(&cards).unwrap().count(), 1);

        assert!(remove_custom_card(&cards, &card).unwrap());
        assert!(!cards.join(&card.file).is_file());
        assert!(!remove_custom_card(&cards, &card).unwrap());

        let escape = CustomCard {
            file: "../outside.png".to_string(),
            focus_x: 50.0,
            width: 1,
            height: 1,
        };
        assert!(remove_custom_card(&cards, &escape).is_err());

        fs::write(root.join("notes.txt"), b"x").unwrap();
        assert!(install_custom_card(&cards, "Haze", &root.join("notes.txt"), 50.0).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn art_dimensions_reads_png_webp_and_jpeg() {
        assert_eq!(art_dimensions(&png(640, 480)), Some((640, 480)));

        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        webp.extend_from_slice(&[0x4b, 0x04, 0x00, 0xc6, 0x03, 0x00]);
        assert_eq!(art_dimensions(&webp), Some((1100, 967)));

        // SOI, APP0, then SOF0 with 320 x 200.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00,
            0xC8, 0x01, 0x40, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01,
        ];
        assert_eq!(art_dimensions(&jpeg), Some((320, 200)));
        assert_eq!(art_dimensions(b"not an image"), None);
    }

    #[test]
    fn one_skin_is_equipped_and_the_order_moves() {
        let root = temp_root("skins");
        let addons = root.join("game").join("citadel").join("addons");
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), "one").unwrap();
        fs::write(addons.join("pak02_dir.vpk.disabled"), "two").unwrap();
        fs::write(addons.join("pak03_dir.vpk"), "three").unwrap();
    fs::write(addons.join("pak04_dir.vpk"), "four").unwrap();
        fs::write(addons.join("pak04_dir.vpk"), "four").unwrap();

        let mods = vec![
            test_mod(1, true, &["Haze"]),
            test_mod(2, false, &["Haze"]),
            test_mod(3, true, &["Haze"]),
            test_mod(4, true, &["Wraith"]),
        ];

        assert!(equip_hero_skin(&root, &mods, "Haze", 2, false).is_err());
        assert!(addons.join("pak02_dir.vpk.disabled").is_file());
        equip_hero_skin(&root, &mods, "Haze", 2, true).unwrap();
        assert!(addons.join("pak02_dir.vpk").is_file());
        assert!(addons.join("pak01_dir.vpk.disabled").is_file());
        assert!(addons.join("pak03_dir.vpk.disabled").is_file());
        // A mod of another hero is untouched.
        assert!(addons.join("pak04_dir.vpk").is_file());
        assert!(equip_hero_skin(&root, &mods, "Haze", 9, true).is_err());

        // Put 03 first: its file takes slot 01 and the others shift.
        assert!(move_hero_skin(&root, &mods, "Haze", 3, 0, false).is_err());
        let report = move_hero_skin(&root, &mods, "Haze", 3, 0, true).unwrap();
        assert!(!report.moves.is_empty());
        // The move keeps each file's on or off state: only the slot number changes.
        assert_eq!(
            fs::read_to_string(addons.join("pak01_dir.vpk.disabled")).unwrap(),
            "three"
        );
        assert_eq!(
            fs::read_to_string(addons.join("pak02_dir.vpk.disabled")).unwrap(),
            "one"
        );
        assert_eq!(fs::read_to_string(addons.join("pak03_dir.vpk")).unwrap(), "two");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shuffle_picks_one_skin_per_hero() {
        let mods = vec![
            test_mod(1, true, &["Haze"]),
            test_mod(2, false, &["Haze"]),
            test_mod(3, false, &["Haze"]),
            test_mod(4, true, &["Wraith"]),
            test_mod(5, false, &["Wraith"]),
            test_mod(6, true, &["Bebop"]),
        ];
        let heroes = ["Haze".to_string(), "Wraith".to_string()];
        let first = shuffle_picks(&mods, &heroes, 7);
        assert_eq!(shuffle_picks(&mods, &heroes, 7), first);
        assert!(first.values().any(|on| *on));
        let haze_on: Vec<u16> = first
            .iter()
            .filter(|(slot, on)| **on && (1..=3).contains(*slot))
            .map(|(slot, _)| *slot)
            .collect();
        let wraith_on: Vec<u16> = first
            .iter()
            .filter(|(slot, on)| **on && (4..=5).contains(*slot))
            .map(|(slot, _)| *slot)
            .collect();
        assert_eq!(haze_on.len(), 1);
        assert_eq!(wraith_on.len(), 1);
        // Bebop was not in the pool, so its mod is not in the wanted map.
        assert!(!first.contains_key(&6));

        // Different seeds reach different picks over a few launches.
        let picks: BTreeSet<u16> = (0..12)
            .flat_map(|seed| {
                shuffle_picks(&mods, &heroes, seed)
                    .into_iter()
                    .filter(|(slot, on)| *on && (1..=3).contains(slot))
                    .map(|(slot, _)| slot)
            })
            .collect();
        assert!(picks.len() > 1);

        let root = temp_root("shuffle");
        let addons = root.join("game").join("citadel").join("addons");
        fs::create_dir_all(&addons).unwrap();
        for slot in 1..=6 {
            fs::write(addons.join(format!("pak{slot:02}_dir.vpk")), format!("{slot}")).unwrap();
        }
        assert!(shuffle_for_launch(&root, &mods, &heroes, 7, false).is_err());
        shuffle_for_launch(&root, &mods, &heroes, 7, true).unwrap();
        let enabled = |slot: u16| addons.join(format!("pak{slot:02}_dir.vpk")).is_file();
        assert_eq!(enabled(1) as usize + enabled(2) as usize + enabled(3) as usize, 1);
        assert_eq!(enabled(4) as usize + enabled(5) as usize, 1);
        assert!(enabled(6), "a hero outside the pool keeps its mod");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ability_sounds_equip_one_per_ability() {
        let root = temp_root("sounds");
        let addons = root.join("game").join("citadel").join("addons");
        fs::create_dir_all(&addons).unwrap();
        for slot in 1..=3 {
            fs::write(addons.join(format!("pak{slot:02}_dir.vpk")), "x").unwrap();
        }
        let mut first = test_mod(1, true, &["Haze"]);
        first.sounds = vec![LockerSound { hero: "Haze".to_string(), ability: 2 }];
        let mut second = test_mod(2, true, &["Haze"]);
        second.sounds = vec![LockerSound { hero: "Haze".to_string(), ability: 2 }];
        let mut third = test_mod(3, true, &["Haze"]);
        third.sounds = vec![LockerSound { hero: "Haze".to_string(), ability: 4 }];
        let mods = vec![first, second, third];

        assert!(equip_ability_sound(&root, &mods, "Haze", 2, 2, false).is_err());
        equip_ability_sound(&root, &mods, "Haze", 2, 2, true).unwrap();
        assert!(addons.join("pak02_dir.vpk").is_file());
        assert!(addons.join("pak01_dir.vpk.disabled").is_file());
        assert!(addons.join("pak03_dir.vpk").is_file(), "another ability is untouched");
        assert!(equip_ability_sound(&root, &mods, "Haze", 2, 3, true).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn global_folder_is_added_above_addons() {
        let root = temp_root("global");
        let citadel = root.join("game").join("citadel");
        fs::create_dir_all(&citadel).unwrap();
        let gameinfo = citadel.join("gameinfo.gi");
        let before = "SearchPaths\n\t\t{\n\t\t\tGame\t\tcitadel/addons\n\t\t\tGame\t\tcitadel\n\t\t}\n";
        fs::write(&gameinfo, before).unwrap();

        let status = read_global_path(&gameinfo).unwrap();
        assert!(!status.configured);
        assert!(!status.folder);

        let added = add_global_line(before).unwrap().unwrap();
        assert!(added.contains("\t\t\tGame\t\tcitadel/grimoire\n\t\t\tGame\t\tcitadel/addons"));
        assert!(add_global_line(&added).unwrap().is_none());

        assert!(ensure_global_path(&gameinfo, false).is_err());
        let edit = ensure_global_path(&gameinfo, true).unwrap();
        assert!(edit.changed);
        let backup = edit.backup.unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), before);
        let after = fs::read_to_string(&gameinfo).unwrap();
        assert_eq!(after, added);
        assert!(citadel.join(GLOBAL_FOLDER).is_dir());
        let status = read_global_path(&gameinfo).unwrap();
        assert!(status.configured);
        assert!(status.folder);

        // Twice is a no-op and writes no second backup.
        let again = ensure_global_path(&gameinfo, true).unwrap();
        assert!(!again.changed);
        assert!(again.backup.is_none());

        assert!(remove_global_path(&gameinfo, true).unwrap().changed);
        assert_eq!(fs::read_to_string(&gameinfo).unwrap(), before);
        assert!(!remove_global_path(&gameinfo, true).unwrap().changed);

        // A file with no anchor is refused rather than guessed at.
        let bare = citadel.join("bare.gi");
        fs::write(&bare, "SearchPaths\n{\n}\n").unwrap();
        assert!(add_global_line("SearchPaths\n{\n}\n").is_err());
        assert!(ensure_global_path(&bare, true).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
