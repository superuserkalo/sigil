//! Which animation clips a hero preview carries, and which one plays first.
//!
//! Ported from Grimoire's `src/lib/heroAnimationCatalog.ts` (MIT): the reviewed per-hero
//! recipes, the standing and relaxed idle fallbacks, the full-body name filter and the bounded
//! selection (eight clips, 10,000 frames). Grimoire also reads the compiled UI AnimGraph2 to
//! pick the shop or hero-select state; Sigil does not port that graph reader yet, so the
//! recipes are the default here (they are Grimoire's own fallback).

use serde::{Deserialize, Serialize};

/// How a clip plays: looping, or once and held on its last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Playback {
    Loop,
    Hold,
}

/// What a selected clip is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Idle,
    RelaxedIdle,
    HeroPose,
    Running,
    Reload,
    Dash,
    HeavyMelee,
}

impl Action {
    fn is_idle(self) -> bool {
        matches!(self, Action::Idle | Action::RelaxedIdle)
    }
}

/// A clip the model or its clip files offer.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipCandidate {
    pub name: String,
    pub frame_count: usize,
    pub fps: f32,
    pub additive: bool,
}

/// A selected clip with its playback and purpose.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipChoice {
    pub name: String,
    pub playback: Playback,
    pub action: Option<Action>,
}

struct Recipe {
    action: Action,
    names: &'static [&'static str],
    playback: Playback,
}

const fn recipe(action: Action, names: &'static [&'static str]) -> Recipe {
    Recipe {
        action,
        names,
        playback: Playback::Loop,
    }
}

const fn hold(action: Action, names: &'static [&'static str]) -> Recipe {
    Recipe {
        action,
        names,
        playback: Playback::Hold,
    }
}

use Action::{Dash, HeavyMelee, HeroPose, Idle, RelaxedIdle, Reload, Running};

/// Grimoire's reviewed recipes, keyed by roster display name.
static HERO_RECIPES: &[(&str, &[Recipe])] = &[
    (
        "Lash",
        &[
            recipe(Idle, &["primary_stand_idle"]),
            recipe(Dash, &["primary_airdash"]),
            recipe(HeavyMelee, &["melee_hit"]),
        ],
    ),
    (
        "Dynamo",
        &[
            recipe(Idle, &["primary_stand_idle"]),
            hold(HeroPose, &["hero_pose"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Wraith",
        &[
            recipe(Idle, &["ui_shop_idle", "primary_stand_idle"]),
            hold(HeroPose, &["ui_hero_select"]),
            recipe(Running, &["item_run_n"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Mirage",
        &[
            recipe(RelaxedIdle, &["primary_ooc_stand_idle"]),
            hold(HeroPose, &["ui_main_menu"]),
        ],
    ),
    (
        "Yamato",
        &[
            hold(HeroPose, &["ui_hero_select", "primary_stand_idle"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Viscous",
        &[recipe(Idle, &["ui_hero_select", "primary_stand_idle"])],
    ),
    (
        "Celeste",
        &[
            recipe(Idle, &["ui_shop", "out_of_combat_stand_idle"]),
            recipe(Running, &["weapon_run_n"]),
        ],
    ),
    ("Rem", &[hold(HeroPose, &["ui_shop"])]),
    ("Victor", &[recipe(Idle, &["weapon_stand_idle"])]),
    ("Graves", &[recipe(Idle, &["weapon_stand_idle"])]),
    ("Mina", &[recipe(Idle, &["weapon_stand_idle"])]),
    (
        "Silver",
        &[
            recipe(Idle, &["weapon_stand_idle"]),
            recipe(Running, &["out_of_combat_run_n"]),
        ],
    ),
    ("Infernus", &[hold(Idle, &["shop_menu_base"])]),
    ("Abrams", &[recipe(Idle, &["shop_menu_base"])]),
    (
        "McGinnis",
        &[
            recipe(Idle, &["primary_shop_menu_idle"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Paradox",
        &[
            recipe(Idle, &["shop_menu_base"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Haze",
        &[
            recipe(Idle, &["shop_menu_base"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Grey Talon",
        &[
            recipe(Idle, &["shop_menu_base"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Kelvin",
        &[
            recipe(Idle, &["primary_stand_idle"]),
            recipe(Running, &["item_run_n"]),
        ],
    ),
    (
        "Holliday",
        &[
            recipe(Idle, &["primary_stand_idle"]),
            recipe(Running, &["item_run_n"]),
        ],
    ),
    (
        "Warden",
        &[
            recipe(Idle, &["primary_stand_idle"]),
            recipe(Running, &["item_run_n"]),
        ],
    ),
    (
        "Calico",
        &[
            hold(Idle, &["primary_stand_idle"]),
            recipe(Running, &["out_of_combat_run_n"]),
            hold(Reload, &["primary_stand_reload"]),
        ],
    ),
    (
        "Vyper",
        &[
            recipe(Idle, &["primary_stand_idle"]),
            recipe(Running, &["primary_run_n"]),
        ],
    ),
    ("Seven", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Vindicta", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Lady Geist", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Bebop", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Mo & Krill", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Shiv", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Ivy", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Pocket", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Sinclair", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Doorman", &[recipe(Idle, &["primary_stand_idle"])]),
    ("Drifter", &[recipe(Idle, &["ui_shop"])]),
    ("Venator", &[recipe(Idle, &["ui_shop"])]),
    ("Apollo", &[recipe(Idle, &["ui_shop"])]),
    ("Deadman Danny", &[recipe(Idle, &["ui_shop"])]),
    ("Solomon", &[recipe(Idle, &["ui_shop"])]),
    ("Violet", &[recipe(Idle, &["ui_shop"])]),
    (
        "Paige",
        &[recipe(RelaxedIdle, &["out_of_combat_stand_idle"])],
    ),
    ("Billy", &[hold(Idle, &["primary_idle"])]),
    ("Rat King", &[recipe(Idle, &["ui_shop_idle"])]),
    ("Nurse Harrow", &[recipe(Idle, &["weapon_stand_idle1"])]),
    ("Baba", &[recipe(RelaxedIdle, &["outofcombat_stand_idle"])]),
];

fn hero_recipes(hero: &str) -> &'static [Recipe] {
    HERO_RECIPES
        .iter()
        .find(|(name, _)| *name == hero)
        .map_or(&[], |(_, recipes)| *recipes)
}

const STANDING_IDLE: &[&str] = &[
    "primary_stand_idle",
    "primary_idle",
    "weapon_stand_idle",
    "stand_idle",
    "idle_stand",
    "idle",
];
const RELAXED_IDLE: &[&str] = &[
    "primary_ooc_stand_idle",
    "out_of_combat_stand_idle",
    "ooc_stand_idle",
];
const HERO_POSES: &[&str] = &[
    "ui_hero_pose",
    "hero_pose",
    "ui_hero_select",
    "ui_main_menu",
];
const SHOP_IDLES: &[&str] = &["ui_shop", "ui_shop_idle", "shop_ui_idle"];

const DEFAULT_RECIPES: &[Recipe] = &[
    recipe(Idle, STANDING_IDLE),
    recipe(RelaxedIdle, RELAXED_IDLE),
    recipe(HeroPose, &["hero_pose", "ui_hero_pose"]),
];

const MAX_CLIPS: usize = 8;
const MAX_FRAMES: usize = 10_000;

/// Every clip name a selection for `hero` could pick. Clip files outside this set are not
/// worth decoding.
pub(crate) fn wanted_names(hero: &str) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for recipe in hero_recipes(hero).iter().chain(DEFAULT_RECIPES) {
        names.extend_from_slice(recipe.names);
    }
    names.extend_from_slice(STANDING_IDLE);
    names.extend_from_slice(RELAXED_IDLE);
    names.extend_from_slice(HERO_POSES);
    names.extend_from_slice(SHOP_IDLES);
    names.extend_from_slice(&vpkmerge_core::DEFAULT_POSE_CLIPS);
    names.sort_unstable();
    names.dedup();
    names
}

/// A complete full-body action: playable length, not additive, and no layer or transition
/// token in the name. Grimoire's `isStandaloneHeroAnimation`.
fn standalone(clip: &ClipCandidate) -> bool {
    if clip.name.trim().is_empty()
        || clip.additive
        || clip.frame_count <= 1
        || clip.fps.is_nan()
        || clip.fps <= 0.0
    {
        return false;
    }
    let duration = (clip.frame_count - 1) as f32 / clip.fps;
    if duration <= 0.001 {
        return false;
    }
    const EXCLUDED: &[&str] = &[
        "aim",
        "add",
        "additive",
        "delta",
        "layer",
        "blend",
        "upperbody",
        "lowerbody",
        "transition",
        "intro",
        "exit",
        "enter",
        "end",
        "start",
        "channel",
        "respawn",
        "rebirth",
        "death",
        "flinch",
        "retarget",
        "bindpose",
        "null",
    ];
    !clip
        .name
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| EXCLUDED.contains(&token))
}

/// The playback and purpose Grimoire assigns a clip name for a hero.
fn classify(name: &str, hero: &str) -> Option<(Action, Playback)> {
    if let Some(found) = hero_recipes(hero)
        .iter()
        .find(|recipe| recipe.names.contains(&name))
    {
        return Some((found.action, found.playback));
    }
    if STANDING_IDLE.contains(&name) || SHOP_IDLES.contains(&name) {
        return Some((Idle, Playback::Loop));
    }
    if RELAXED_IDLE.contains(&name) {
        return Some((RelaxedIdle, Playback::Loop));
    }
    if HERO_POSES.contains(&name) {
        return Some((HeroPose, Playback::Loop));
    }
    None
}

/// Choose the preview clips for `hero` (roster display name) from what the model offers, in
/// play order: the idle first. When nothing qualifies, fall back to the menu pose vpkmerge
/// would bake, held, so the hero is never shown in its bind pose while a pose exists.
pub fn select_clips(candidates: &[ClipCandidate], hero: &str) -> Vec<ClipChoice> {
    let safe: Vec<&ClipCandidate> = candidates.iter().filter(|clip| standalone(clip)).collect();
    let find = |name: &str| safe.iter().copied().find(|clip| clip.name == name);
    let recipes = match hero_recipes(hero) {
        [] => DEFAULT_RECIPES,
        own => own,
    };

    let mut selected: Vec<ClipChoice> = Vec::new();
    let mut frames = 0usize;
    for recipe in recipes {
        let Some(clip) = recipe
            .names
            .iter()
            .filter_map(|name| find(name))
            .find(|clip| !selected.iter().any(|old| old.name == clip.name))
        else {
            continue;
        };
        if frames + clip.frame_count > MAX_FRAMES {
            continue;
        }
        frames += clip.frame_count;
        selected.push(ClipChoice {
            name: clip.name.clone(),
            playback: recipe.playback,
            action: Some(recipe.action),
        });
    }

    // A reviewed default may disappear from a skin or game update: try a neutral idle.
    if selected.is_empty()
        && let Some(clip) = STANDING_IDLE
            .iter()
            .chain(RELAXED_IDLE)
            .find_map(|name| find(name))
        && clip.frame_count <= MAX_FRAMES
    {
        let (action, playback) = classify(&clip.name, hero).unwrap_or((Idle, Playback::Loop));
        selected.push(ClipChoice {
            name: clip.name.clone(),
            playback,
            action: Some(action),
        });
    }

    if selected.is_empty()
        && let Some(clip) = vpkmerge_core::DEFAULT_POSE_CLIPS.iter().find_map(|name| {
            candidates
                .iter()
                .find(|clip| clip.name.eq_ignore_ascii_case(name) && !clip.additive)
        })
    {
        selected.push(ClipChoice {
            name: clip.name.clone(),
            playback: Playback::Hold,
            action: Some(HeroPose),
        });
    }

    // Idle first: the renderer starts on the first clip unless told otherwise.
    selected.sort_by_key(|choice| !choice.action.is_some_and(Action::is_idle));
    selected.truncate(MAX_CLIPS);
    selected
}

/// The clip a preview starts on: the first idle, else the first clip.
pub(crate) fn idle_clip(choices: &[ClipChoice]) -> Option<String> {
    choices
        .iter()
        .find(|choice| choice.action.is_some_and(Action::is_idle))
        .or_else(|| choices.first())
        .map(|choice| choice.name.clone())
}
