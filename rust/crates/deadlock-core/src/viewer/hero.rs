//! Hero previews: the roster, which heroes a mod touches, and the cached hero glb export.
//!
//! Ported from Grimoire's `heroPoseModels.ts` (MIT): content-addressed cache keys over the
//! resolved source VPKs, the live model entry per hero, skins without clip files taking their
//! motions from the game, and cloth data beside the model. Grimoire merges a multi-skin stack
//! into a temporary VPK; Sigil resolves across the open VPKs instead, so any number of mods
//! stack with no merge step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::cache::{self, Staging};
use super::clips::{self, Action, ClipCandidate, Playback};
use super::stack::{self, Source, SourceUsage, Stack, Stamp, normalize, source_fingerprint, stamp};
use super::{
    PIPELINE_VERSION, SCHEMA, VPKMERGE_REV, Variant, check, citadel_dir, exporter_error, kv,
};
use crate::error::{Error, Result};
use crate::import::Library;
use crate::locker::{RosterHero, hero_of_path, roster_hero};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// One playable hero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroEntry {
    /// Class codename from `scripts/heroes.vdata_c` (`inferno`, `atlas`), without `hero_`.
    pub codename: String,
    /// Roster display name (`Infernus`, `Abrams`).
    pub display_name: String,
    /// Compiled body model, `models/heroes_wip/inferno/inferno.vmdl_c`.
    pub model_entry: String,
    /// False for a vdata hero Sigil's roster does not know yet.
    pub in_roster: bool,
}

/// Grimoire's reviewed body model per hero (`MODEL_ENTRY_OVERRIDES`), used when the game's
/// `heroes.vdata_c` cannot be read or names a model the base VPK does not ship.
const MODEL_FALLBACK: &[(&str, &str)] = &[
    ("Abrams", "models/heroes_wip/abrams/abrams.vmdl_c"),
    ("McGinnis", "models/heroes_wip/mcginnis/mcginnis.vmdl_c"),
    ("Pocket", "models/heroes_wip/pocket/pocket.vmdl_c"),
    ("Ivy", "models/heroes_wip/ivy/ivy.vmdl_c"),
    ("Lady Geist", "models/heroes_wip/geist/geist.vmdl_c"),
    ("Infernus", "models/heroes_wip/inferno/inferno.vmdl_c"),
    ("Rem", "models/heroes_wip/familiar/familiar_wip.vmdl_c"),
    ("Viscous", "models/heroes_staging/viscous/viscous.vmdl_c"),
    ("Wraith", "models/heroes_wip/wraith/wraith.vmdl_c"),
    ("Mirage", "models/heroes_staging/mirage_v2/mirage.vmdl_c"),
    (
        "Seven",
        "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl_c",
    ),
    ("Vindicta", "models/heroes_staging/hornet_v3/hornet.vmdl_c"),
    ("Paradox", "models/heroes_staging/chrono/chrono.vmdl_c"),
    ("Dynamo", "models/heroes_wip/dynamo/dynamo.vmdl_c"),
    ("Kelvin", "models/heroes_staging/kelvin_v2/kelvin.vmdl_c"),
    ("Haze", "models/heroes_staging/haze/haze.vmdl_c"),
    ("Holliday", "models/heroes_staging/astro/astro.vmdl_c"),
    ("Bebop", "models/heroes_staging/bebop/bebop.vmdl_c"),
    ("Calico", "models/heroes_staging/nano/nano_v2/nano.vmdl_c"),
    ("Grey Talon", "models/heroes_staging/archer/archer.vmdl_c"),
    ("Mo & Krill", "models/heroes_staging/digger/digger.vmdl_c"),
    ("Shiv", "models/heroes_staging/shiv/shiv.vmdl_c"),
    ("Warden", "models/heroes_staging/warden/warden.vmdl_c"),
    ("Yamato", "models/heroes_staging/yamato_v2/yamato.vmdl_c"),
    ("Lash", "models/heroes_wip/lash/lash.vmdl_c"),
    ("Vyper", "models/heroes_staging/viper/viper.vmdl_c"),
    (
        "Sinclair",
        "models/heroes_staging/magician_v2/magician.vmdl_c",
    ),
    ("Mina", "models/heroes_wip/vampirebat/vampirebat.vmdl_c"),
    ("Drifter", "models/heroes_wip/drifter/drifter.vmdl_c"),
    ("Venator", "models/heroes_wip/priest/priest.vmdl_c"),
    ("Victor", "models/heroes_wip/frank/frank.vmdl_c"),
    ("Paige", "models/heroes_wip/bookworm/bookworm.vmdl_c"),
    ("Doorman", "models/heroes_wip/doorman_v2/doorman.vmdl_c"),
    ("Billy", "models/heroes_wip/punkgoat/punkgoat.vmdl_c"),
    ("Graves", "models/heroes_wip/necro/necro.vmdl_c"),
    ("Apollo", "models/heroes_wip/fencer/fencer.vmdl_c"),
    (
        "Deadman Danny",
        "models/heroes_wip/deadpack/deadpack.vmdl_c",
    ),
    ("Silver", "models/heroes_wip/werewolf/werewolf.vmdl_c"),
    ("Celeste", "models/heroes_wip/unicorn/unicorn.vmdl_c"),
    ("Rat King", "models/heroes_wip/ratking/ratking.vmdl_c"),
    (
        "Solomon",
        "models/heroes_wip/chessmaster/chessmaster.vmdl_c",
    ),
    ("Violet", "models/heroes_wip/artist/artist.vmdl_c"),
    ("Nurse Harrow", "models/heroes_wip/nurse/nurse.vmdl_c"),
    ("Baba", "models/heroes_wip/baba/baba.vmdl_c"),
];

/// Every selectable hero in the installed game, sorted by display name. The body model comes
/// from `scripts/heroes.vdata_c` (`m_strModelName`), as the game spawns it; Grimoire's
/// reviewed table fills in when the vdata is unreadable. Cached per `pak01_dir.vpk` stamp.
pub fn heroes(game_root: &Path) -> Result<Vec<HeroEntry>> {
    type Cached = (PathBuf, Stamp, Vec<HeroEntry>);
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    let base = citadel_dir(game_root).join("pak01_dir.vpk");
    if !base.is_file() {
        return Err(Error::msg(format!(
            "{} is missing: point Sigil at the Deadlock folder that holds game/citadel.",
            base.display()
        )));
    }
    let current = stamp(&base)?;
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some((path, cached_stamp, list)) = guard.as_ref()
        && *path == base
        && *cached_stamp == current
    {
        return Ok(list.clone());
    }

    let directory = stack::open_directory(&base)?;
    let has = |entry: &str| directory.by_path.contains_key(&normalize(entry));
    let mut out: Vec<HeroEntry> = Vec::new();

    if let Ok(entries) = vpkmerge_core::live_hero_entries(&base, None) {
        for entry in entries {
            let roster = crate::locker::hero_for_codename(&entry.codename);
            let fallback = roster.and_then(|hero| fallback_entry(hero.name));
            let model_entry = if has(&entry.model_entry) {
                normalize(&entry.model_entry)
            } else if let Some(fallback) = fallback.filter(|fallback| has(fallback)) {
                fallback.to_string()
            } else {
                continue;
            };
            out.push(HeroEntry {
                codename: entry.codename.to_ascii_lowercase(),
                display_name: roster
                    .map(|hero| hero.name.to_string())
                    .unwrap_or_else(|| crate::locker::display_name_for(&entry.codename)),
                model_entry,
                in_roster: roster.is_some(),
            });
        }
    }

    if out.is_empty() {
        for hero in crate::locker::ROSTER.iter() {
            let Some(model_entry) = fallback_entry(hero.name).filter(|entry| has(entry)) else {
                continue;
            };
            out.push(HeroEntry {
                codename: hero.codenames[0].to_string(),
                display_name: hero.name.to_string(),
                model_entry: model_entry.to_string(),
                in_roster: true,
            });
        }
    }

    if out.is_empty() {
        return Err(Error::msg("No hero models found in citadel/pak01_dir.vpk."));
    }
    out.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then(a.codename.cmp(&b.codename))
    });
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((base, current, out.clone()));
    }
    Ok(out)
}

fn fallback_entry(display_name: &str) -> Option<&'static str> {
    MODEL_FALLBACK
        .iter()
        .find(|(name, _)| *name == display_name)
        .map(|(_, entry)| *entry)
}

/// The hero for a codename (`inferno`), an older alias (`bull`) or a display name
/// (`Lady Geist`, `lady_geist`).
pub fn find_hero(game_root: &Path, query: &str) -> Result<HeroEntry> {
    let list = heroes(game_root)?;
    let wanted = query.trim().to_ascii_lowercase();
    let roster = roster_hero(query);
    list.iter()
        .find(|hero| hero.codename == wanted)
        .or_else(|| {
            roster.and_then(|roster| list.iter().find(|hero| hero.display_name == roster.name))
        })
        .or_else(|| {
            list.iter()
                .find(|hero| hero.display_name.eq_ignore_ascii_case(query.trim()))
        })
        .cloned()
        .ok_or_else(|| Error::msg(format!("No hero named \"{query}\" in this game build.")))
}

/// Codenames of the heroes whose model or material files a mod replaces, sorted. Installed
/// and mod details use it to open the right hero for a mod. Read only, directory tree only.
pub fn heroes_in_vpk(vpk: &Path) -> Result<Vec<String>> {
    let directory = read_vpk_directory_with(
        vpk,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;
    let mut out: Vec<String> = directory
        .by_path
        .keys()
        .filter(|path| is_model_path(path))
        .filter_map(|path| hero_of_path(path))
        .map(|hero| hero.codenames[0].to_string())
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// `models/heroes*` or `materials/models/heroes*`: the files that change how a hero looks.
fn is_model_path(path_lower: &str) -> bool {
    let rest = path_lower.strip_prefix("materials/").unwrap_or(path_lower);
    rest.starts_with("models/heroes")
}

/// Whether a mod can change this hero's preview: it ships the body model, anything in the
/// model's folder, or a hero model or material path that belongs to the hero.
fn touches_hero<'a>(
    mut layer_paths: impl Iterator<Item = &'a str>,
    entry: &str,
    model_dir: &str,
    roster: Option<&RosterHero>,
) -> bool {
    layer_paths.any(|path| {
        path == entry
            || path.starts_with(model_dir)
            || (is_model_path(path)
                && roster.is_some_and(|hero| {
                    hero_of_path(path).is_some_and(|found| found.name == hero.name)
                }))
    })
}

/// Where a clip's keyframes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipOrigin {
    /// Embedded in the model that is shown.
    Model,
    /// Embedded in the game's own model at the same path, bound to the shown rig by bone name.
    Base,
    /// A `.vnmclip_c` clip file, as the game's animation graph plays it.
    ClipFile,
}

/// One clip in a hero glb, as the sidecar describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMeta {
    /// The glTF animation name.
    pub name: String,
    pub duration_seconds: f32,
    pub fps: f32,
    pub frame_count: usize,
    pub playback: Playback,
    pub action: Option<Action>,
    pub origin: ClipOrigin,
    /// The `.vnmclip_c` path for a clip file.
    pub resource: Option<String>,
}

/// The JSON sidecar written beside a hero glb. See `docs/design/3d-viewer.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroSidecar {
    pub schema: u32,
    pub hero: String,
    pub display_name: String,
    /// The look the glb shows. `original` when no installed mod touches this hero, even if
    /// `modded` was asked for: the two exports then share one cache entry.
    pub variant: Variant,
    /// glTF animation names in the glb, idle first.
    pub clips: Vec<String>,
    pub idle_clip: Option<String>,
    /// The VPK stack the export read, highest precedence first. `slot` is null for game files.
    pub sources: Vec<SourceUsage>,
    pub model_entry: String,
    pub clip_info: Vec<ClipMeta>,
    /// File name of the cloth data beside the glb (`PHYS.m_pFeModel` as JSON), when the model
    /// has cloth.
    pub cloth: Option<String>,
    /// `meters`, Y up, the hero standing on y = 0 at the origin, facing +Z.
    pub units: String,
    pub up: String,
    pub forward: String,
    pub key: String,
    /// Things the preview could not show: `changesNotShown:<file>` for a mod whose files the
    /// export never read, `clipRig:<name>` for a clip file that did not fit the rig.
    pub issues: Vec<String>,
    pub pipeline: String,
}

/// A finished hero export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub glb: PathBuf,
    pub sidecar: PathBuf,
    pub cloth: Option<PathBuf>,
    /// The cache key. Equal keys for the `modded` and `original` export of a hero mean no
    /// installed mod changes how it looks.
    pub key: String,
    /// The look the glb shows (see [`HeroSidecar::variant`]).
    pub variant: Variant,
    /// True when the files came from the cache.
    pub cached: bool,
}

/// Everything [`export_hero_with`] takes.
#[derive(Debug, Clone)]
pub struct HeroExportRequest<'a> {
    pub game_root: &'a Path,
    pub cache_dir: &'a Path,
    /// Codename or display name.
    pub hero: &'a str,
    pub variant: Variant,
    /// Disabled slots to include, to preview a mod before switching it on.
    pub extra_slots: &'a [u16],
    /// Names the mods in the sidecar.
    pub library: Option<&'a Library>,
    /// Longest texture edge embedded, `None` for full size. Default 1024 (Grimoire's preview
    /// cap: a hero stays under about 120 MB of GPU textures).
    pub max_texture_edge: Option<u32>,
}

impl<'a> HeroExportRequest<'a> {
    pub fn new(game_root: &'a Path, cache_dir: &'a Path, hero: &'a str, variant: Variant) -> Self {
        Self {
            game_root,
            cache_dir,
            hero,
            variant,
            extra_slots: &[],
            library: None,
            max_texture_edge: Some(1024),
        }
    }
}

/// Export a hero as the player would see it, or as the game ships it, into the cache.
/// A second call with unchanged inputs returns the cached files without decoding anything.
pub fn export_hero(
    game_root: &Path,
    cache_dir: &Path,
    hero: &str,
    variant: Variant,
    extra_slots: &[u16],
    cancel: &AtomicBool,
) -> Result<Exported> {
    let mut request = HeroExportRequest::new(game_root, cache_dir, hero, variant);
    request.extra_slots = extra_slots;
    export_hero_with(&request, cancel)
}

/// One heavy decode at a time, as Deadlock Mod Manager's Foundry does: a texture-heavy hero is
/// hundreds of megabytes in flight, and two at once only slow each other down.
pub(crate) fn export_slot() -> std::sync::MutexGuard<'static, ()> {
    static SLOT: Mutex<()> = Mutex::new(());
    SLOT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct HeroPlan {
    pub hero: HeroEntry,
    pub sources: Vec<Source>,
    pub variant: Variant,
    pub key: String,
}

/// Resolve the hero and the VPKs that can change it, and the cache key over them.
pub(crate) fn plan_hero(request: &HeroExportRequest<'_>) -> Result<HeroPlan> {
    let hero = find_hero(request.game_root, request.hero)?;
    let mut all = super::sources(
        request.game_root,
        request.variant,
        Some(request.extra_slots),
    );
    if let Some(library) = request.library {
        stack::name_sources(&mut all, library);
    }
    if !all
        .iter()
        .any(|source| source.kind == stack::SourceKind::Base)
    {
        return Err(Error::msg("citadel/pak01_dir.vpk is missing."));
    }

    let entry = normalize(&hero.model_entry);
    let model_dir = entry
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/"))
        .unwrap_or_default();
    let roster = roster_hero(&hero.display_name);
    let mut kept = Vec::with_capacity(all.len());
    for source in all {
        if source.kind.is_mod() {
            // An unreadable mod cannot be shown; the game would skip it too.
            let Ok(directory) = stack::open_directory(&source.path) else {
                continue;
            };
            if !touches_hero(
                directory.by_path.keys().map(String::as_str),
                &entry,
                &model_dir,
                roster,
            ) {
                continue;
            }
        }
        kept.push(source);
    }
    let variant = if kept.iter().any(|source| source.kind.is_mod()) {
        Variant::Modded
    } else {
        Variant::Original
    };

    let mut parts = vec![
        PIPELINE_VERSION.to_string(),
        VPKMERGE_REV.to_string(),
        "hero".into(),
        hero.codename.clone(),
        entry,
        format!("{:?}", request.max_texture_edge),
        variant.as_str().into(),
    ];
    parts.extend(kept.iter().map(source_fingerprint));
    let key = cache::key(&parts);
    Ok(HeroPlan {
        hero,
        sources: kept,
        variant,
        key,
    })
}

/// Export a hero with every option. See [`export_hero`].
pub fn export_hero_with(request: &HeroExportRequest<'_>, cancel: &AtomicBool) -> Result<Exported> {
    check(cancel)?;
    let plan = plan_hero(request)?;
    let parent = request.cache_dir.join("heroes");
    let entry_dir = parent.join(&plan.key);

    if let Some(found) = cached(&entry_dir, &plan)? {
        return Ok(found);
    }
    let _slot = export_slot();
    if let Some(found) = cached(&entry_dir, &plan)? {
        return Ok(found);
    }
    check(cancel)?;

    let stack = Stack::open(&plan.sources)?;
    let staging = Staging::begin(&parent, &plan.key)?;
    let built = build_hero(
        &stack,
        &plan.hero,
        request.max_texture_edge,
        cancel,
        &staging.dir,
    )?;
    check(cancel)?;

    let codename = &plan.hero.codename;
    let usage = stack.usage();
    let mut issues = built.issues;
    for source in usage
        .iter()
        .filter(|source| source.kind.is_mod() && source.used == 0)
    {
        issues.push(format!("changesNotShown:{}", source.file));
    }
    let sidecar = HeroSidecar {
        schema: SCHEMA,
        hero: codename.clone(),
        display_name: plan.hero.display_name.clone(),
        variant: plan.variant,
        clips: built.clips.iter().map(|clip| clip.name.clone()).collect(),
        idle_clip: built.idle,
        sources: usage,
        model_entry: plan.hero.model_entry.clone(),
        clip_info: built.clips,
        cloth: built.cloth.then(|| format!("{codename}.cloth.json")),
        units: "meters".into(),
        up: "+Y".into(),
        forward: "+Z".into(),
        key: plan.key.clone(),
        issues,
        pipeline: PIPELINE_VERSION.into(),
    };
    std::fs::write(
        staging.dir.join(format!("{codename}.json")),
        serde_json::to_vec_pretty(&sidecar)?,
    )?;
    let dir = staging.commit()?;
    Ok(exported(&dir, codename, &sidecar, false))
}

fn exported(dir: &Path, codename: &str, sidecar: &HeroSidecar, cached: bool) -> Exported {
    Exported {
        glb: dir.join(format!("{codename}.glb")),
        sidecar: dir.join(format!("{codename}.json")),
        cloth: sidecar.cloth.as_ref().map(|name| dir.join(name)),
        key: sidecar.key.clone(),
        variant: sidecar.variant,
        cached,
    }
}

/// A complete cache entry for the plan, with mod names refreshed from the current sources.
pub(crate) fn cached_sidecar(dir: &Path, codename: &str) -> Option<HeroSidecar> {
    let glb = dir.join(format!("{codename}.glb"));
    if !glb.is_file() {
        return None;
    }
    let text = std::fs::read(dir.join(format!("{codename}.json"))).ok()?;
    let sidecar: HeroSidecar = serde_json::from_slice(&text).ok()?;
    (sidecar.schema == SCHEMA && sidecar.pipeline == PIPELINE_VERSION).then_some(sidecar)
}

fn cached(dir: &Path, plan: &HeroPlan) -> Result<Option<Exported>> {
    let codename = &plan.hero.codename;
    let Some(mut sidecar) = cached_sidecar(dir, codename) else {
        return Ok(None);
    };
    cache::touch(dir);
    // Names are not part of the key: a renamed mod keeps its export and gets its new name.
    let names: BTreeMap<&str, &str> = plan
        .sources
        .iter()
        .map(|source| (source.file.as_str(), source.name.as_str()))
        .collect();
    let mut renamed = false;
    for source in &mut sidecar.sources {
        if let Some(name) = names.get(source.file.as_str())
            && source.name != *name
        {
            source.name = (*name).to_string();
            renamed = true;
        }
    }
    if renamed {
        cache::write_atomic(
            &dir.join(format!("{codename}.json")),
            &serde_json::to_vec_pretty(&sidecar)?,
        )?;
    }
    Ok(Some(exported(dir, codename, &sidecar, true)))
}

struct Built {
    clips: Vec<ClipMeta>,
    idle: Option<String>,
    cloth: bool,
    issues: Vec<String>,
}

enum Pick {
    Embedded(usize),
    File(usize),
}

/// Decode the hero, choose its clips, and write `<codename>.glb` (and cloth) into `out`.
fn build_hero(
    stack: &Stack,
    hero: &HeroEntry,
    max_texture_edge: Option<u32>,
    cancel: &AtomicBool,
    out: &Path,
) -> Result<Built> {
    let entry = normalize(&hero.model_entry);
    let (bytes, winner) = stack
        .read_from(0, &entry)
        .ok_or_else(|| Error::msg(format!("{entry} is not in any VPK of the stack.")))?;
    check(cancel)?;
    let mut model = morphic::model::decode(&bytes)
        .map_err(|error| exporter_error(format!("decoding {entry}"), error))?;
    let mut issues = Vec::new();

    // Skins: the game plays clip files through its animation graph. Copies baked into a skin's
    // model are never played and go stale, so a skin without clip files of its own takes its
    // motions from the game's model (Grimoire's recoverSkinAnimations).
    let game_start = stack.first_game_layer();
    let from_mod = winner < game_start;
    let mods_ship_clips = stack.layers[..game_start]
        .iter()
        .any(|layer| layer.paths().any(|path| path.ends_with(".vnmclip_c")));
    let mut embedded = std::mem::take(&mut model.animations);
    let mut embedded_origin = ClipOrigin::Model;
    if (from_mod && !mods_ship_clips) || embedded.is_empty() {
        let start = if from_mod { game_start } else { winner + 1 };
        if let Some((donor_bytes, _)) = stack.read_from(start, &entry)
            && let Ok(donor) = morphic::model::decode(&donor_bytes)
            && !donor.animations.is_empty()
        {
            embedded = donor
                .animations
                .iter()
                .filter_map(|clip| remap_clip(clip, &donor.skeleton, &model.skeleton))
                .collect();
            embedded_origin = ClipOrigin::Base;
        }
    }
    check(cancel)?;

    // Clip files beside the model, decoded only when a selection could pick them.
    let model_dir = entry
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default();
    let model_stem = entry
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".vmdl_c"))
        .unwrap_or_default()
        .to_string();
    let wanted = clips::wanted_names(&hero.display_name);
    let mut files: Vec<(String, String, morphic::model::NmClip)> = Vec::new();
    for path in stack
        .list(&format!("{model_dir}/clips/"), ".vnmclip_c")
        .into_keys()
    {
        let stem = path
            .rsplit('/')
            .next()
            .and_then(|name| name.strip_suffix(".vnmclip_c"))
            .unwrap_or_default();
        let Some(name) = canonical_clip_name(stem, &hero.codename, &model_stem, &wanted) else {
            continue;
        };
        if files.len() >= 32 || files.iter().any(|(existing, _, _)| *existing == name) {
            continue;
        }
        check(cancel)?;
        let Some(clip) = stack
            .read(&path)
            .and_then(|bytes| morphic::model::decode_nm_clip(&bytes).ok())
        else {
            continue;
        };
        files.push((name, path, clip));
    }

    let mut picks: BTreeMap<String, (ClipCandidate, Pick)> = BTreeMap::new();
    for (index, clip) in embedded.iter().enumerate() {
        picks.insert(
            clip.name.clone(),
            (
                ClipCandidate {
                    name: clip.name.clone(),
                    frame_count: clip.frame_count,
                    fps: clip.fps,
                    additive: false,
                },
                Pick::Embedded(index),
            ),
        );
    }
    for (index, (name, _, clip)) in files.iter().enumerate() {
        if clip.additive || clip.frame_count <= 1 {
            continue;
        }
        // A clip file wins over a same-name embedded copy: it is what the game plays.
        picks.insert(
            name.clone(),
            (
                ClipCandidate {
                    name: name.clone(),
                    frame_count: clip.frame_count as usize,
                    fps: clip.fps(),
                    additive: false,
                },
                Pick::File(index),
            ),
        );
    }
    let candidates: Vec<ClipCandidate> = picks
        .values()
        .map(|(candidate, _)| candidate.clone())
        .collect();
    let choices = clips::select_clips(&candidates, &hero.display_name);

    let root_motion = model
        .skeleton
        .bones
        .iter()
        .position(|bone| bone.name == "root_motion");
    let rig_bones = model
        .skeleton
        .bones
        .iter()
        .filter(|bone| !bone.name.starts_with("$cloth_"))
        .count();
    let mut animations = Vec::new();
    let mut metas = Vec::new();
    let mut kept_choices = Vec::new();
    for choice in &choices {
        let Some((_, pick)) = picks.get(&choice.name) else {
            continue;
        };
        let (mut clip, origin, resource) = match pick {
            Pick::Embedded(index) => (embedded[*index].clone(), embedded_origin, None),
            Pick::File(index) => {
                let (_, path, nm) = &files[*index];
                let skeleton = stack
                    .read(&stack::compiled(&nm.skeleton_ref))
                    .and_then(|bytes| morphic::model::decode_nm_skeleton(&bytes).ok());
                let Some(skeleton) = skeleton else {
                    issues.push(format!("clipSkeleton:{}", choice.name));
                    continue;
                };
                let clip =
                    morphic::model::nm_clip_to_clip(nm, &skeleton, &model.skeleton, &choice.name);
                // vpkmerge's workshop motions bar: shared tracks against the smaller rig.
                let matched = clip.tracks.len();
                if matched < 2 || matched * 2 < nm.tracks.len().min(rig_bones) {
                    issues.push(format!("clipRig:{}", choice.name));
                    continue;
                }
                (clip, ClipOrigin::ClipFile, Some(path.clone()))
            }
        };
        clip.name = choice.name.clone();
        clip.looping = choice.playback == Playback::Loop;
        // Root translation stays under the stage's control: the hero idles in place.
        for track in &mut clip.tracks {
            if Some(track.bone) == root_motion {
                track.translations = None;
            }
        }
        let duration = if clip.fps > 0.0 && clip.frame_count > 1 {
            (clip.frame_count - 1) as f32 / clip.fps
        } else {
            0.0
        };
        metas.push(ClipMeta {
            name: clip.name.clone(),
            duration_seconds: duration,
            fps: clip.fps,
            frame_count: clip.frame_count,
            playback: choice.playback,
            action: choice.action,
            origin,
            resource,
        });
        kept_choices.push(choice.clone());
        animations.push(clip);
    }
    model.animations = animations;
    check(cancel)?;

    let options = morphic::model::GlbOptions {
        max_texture_edge,
        include_outline_shells: false,
    };
    let glb = morphic::model::to_glb_textured(&model, stack, options)
        .map_err(|error| exporter_error(format!("writing the glb for {entry}"), error))?;
    std::fs::write(out.join(format!("{}.glb", hero.codename)), glb)?;

    let cloth = cloth_json(&bytes);
    if let Some(cloth) = &cloth {
        std::fs::write(
            out.join(format!("{}.cloth.json", hero.codename)),
            serde_json::to_vec(cloth)?,
        )?;
    }

    Ok(Built {
        idle: clips::idle_clip(&kept_choices),
        clips: metas,
        cloth: cloth.is_some(),
        issues,
    })
}

/// The selection name for a clip file: its stem, or the stem without a `<codename>_` or
/// `<model>_` prefix (`yamato_ui_hero_select`, `vampirebat_hero_pose`), when that is a name a
/// selection can pick.
pub(crate) fn canonical_clip_name(
    stem: &str,
    codename: &str,
    model_stem: &str,
    wanted: &[&str],
) -> Option<String> {
    if wanted.contains(&stem) {
        return Some(stem.to_string());
    }
    [codename, model_stem]
        .iter()
        .filter(|prefix| !prefix.is_empty())
        .filter_map(|prefix| stem.strip_prefix(prefix)?.strip_prefix('_'))
        .find(|name| wanted.contains(name))
        .map(str::to_string)
}

/// A clip authored for `from`, bound to `to` by bone name. `None` when under two bones match.
pub(crate) fn remap_clip(
    clip: &morphic::model::Clip,
    from: &morphic::model::Skeleton,
    to: &morphic::model::Skeleton,
) -> Option<morphic::model::Clip> {
    let mut out = clip.clone();
    out.tracks = clip
        .tracks
        .iter()
        .filter_map(|track| {
            let name = &from.bones.get(track.bone)?.name;
            let bone = to.find_bone(name)?;
            let mut track = track.clone();
            track.bone = bone;
            Some(track)
        })
        .collect();
    (out.tracks.len() >= 2).then_some(out)
}

/// The model's cloth simulation (`PHYS.m_pFeModel`) as JSON, as vpkmerge's
/// `export_femodel_json` writes it, or `None` for a model without cloth.
fn cloth_json(model_bytes: &[u8]) -> Option<serde_json::Value> {
    let resource = morphic::resource::Resource::parse(model_bytes).ok()?;
    let phys = resource.find_block(*b"PHYS")?;
    let value = morphic::kv3::decode(phys).ok()?;
    Some(kv::to_json(value.get("m_pFeModel")?))
}
