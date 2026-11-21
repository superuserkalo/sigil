//! Which animation clips a hero preview carries, and which one plays first.
//!
//! Ported from Grimoire (MIT):
//!
//! - `src/lib/heroAnimationCatalog.ts`: the reviewed per-hero recipes, the standing and relaxed
//!   idle fallbacks, the standalone filter (`isStandaloneHeroAnimation`), the bounded selection
//!   (`selectHeroAnimations`: a graph showcase first, then recipes, eight clips, 10,000
//!   frames), the default clip (`preferredHeroAnimation`) and the playback rule.
//! - `electron/main/services/heroRiggedClips.ts`: the UI AnimGraph2 reader that finds the clip
//!   the game's shop or hero-select screen plays (`discoverRiggedHeroClips`), and the clip file
//!   metadata (frames, duration, additive, root motion, skeleton).
//! - `electron/main/services/heroAnimationMetadata.ts`: flags for embedded clips read from the
//!   neighbouring clip file with the same timing (`PRODUCTION_CLIP_ALIASES`).
//! - `src/lib/heroViewerPlayback.ts`: friendly labels (`heroClipLabel`) and groups
//!   (`groupHeroClips`).
//!
//! The staged-pose test (`heroStagedPose.ts`), which starts on the standing idle instead of a
//! staged showcase or else drops that idle, runs on the written glb in `staged.rs`.

use std::collections::BTreeMap;

use morphic::kv3::Value;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::kv;

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
    pub(crate) fn is_idle(self) -> bool {
        matches!(self, Action::Idle | Action::RelaxedIdle)
    }
}

/// Which UI screen of the game plays a showcase clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowcaseState {
    /// The shop screen (`.../shop` or `.../hero_shop` state of the `ui` graph).
    Shop,
    /// The hero select screen (`.../select` or `.../hero_select`).
    HeroSelect,
}

/// A clip the model's compiled UI AnimGraph2 plays directly from a shop or hero-select state.
#[derive(Debug, Clone, PartialEq)]
pub struct Showcase {
    /// The compiled graph (`.vnmgraph_c`) the state lives in.
    pub graph: String,
    pub state: ShowcaseState,
    /// `loop` when the clip node allows looping, else `hold`.
    pub playback: Playback,
    /// The clip node's `m_flSpeedMultiplier`.
    pub speed: f32,
}

/// The showcase as the sidecar names it: playback and speed sit on the clip itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipShowcase {
    pub graph: String,
    pub state: ShowcaseState,
}

impl From<&Showcase> for ClipShowcase {
    fn from(showcase: &Showcase) -> Self {
        ClipShowcase {
            graph: showcase.graph.clone(),
            state: showcase.state,
        }
    }
}

/// The section of a clip picker a clip belongs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClipGroup {
    Idle,
    Movement,
    Combat,
    #[default]
    Other,
}

/// A clip the model or its clip files offer, with what the compiled data says about it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClipCandidate {
    pub name: String,
    pub frame_count: usize,
    pub fps: f32,
    pub additive: bool,
    pub hidden: bool,
    pub delta: bool,
    pub requires_base: bool,
    pub transition: bool,
    /// The clip moves the hero: average linear speed over 0.01 or turn rate over 0.001 rad/s.
    pub root_motion: bool,
    /// Compiled standalone flag, when the data carries one. `Some(true)` makes a clip
    /// eligible to fill the remaining slots of a selection.
    pub standalone: Option<bool>,
    /// Set when the UI graph plays this clip on the shop or hero-select screen.
    pub showcase: Option<Showcase>,
}

impl ClipCandidate {
    pub fn new(name: impl Into<String>, frame_count: usize, fps: f32) -> Self {
        ClipCandidate {
            name: name.into(),
            frame_count,
            fps,
            ..ClipCandidate::default()
        }
    }
}

/// A selected clip with its playback and purpose.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipChoice {
    pub name: String,
    pub playback: Playback,
    pub action: Option<Action>,
    pub showcase: Option<Showcase>,
}

impl ClipChoice {
    /// Playback rate multiplier: the graph's for a showcase, else 1.
    pub fn speed(&self) -> f32 {
        self.showcase
            .as_ref()
            .map_or(1.0, |showcase| showcase.speed)
    }
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
/// A showcase longer than this is a scene, not a pose: Grimoire leaves it to the recipes.
const MAX_SHOWCASE_FRAMES: usize = 2_400;

/// Every clip name a selection for `hero` could pick from a recipe or a fallback. A clip file
/// outside this set (and not a graph showcase) is not worth decoding.
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

/// A complete full-body action: playable length; not additive, hidden, delta, base-dependent,
/// a transition or moving the hero; not flagged non-standalone; and no layer or transition
/// token in the name. Grimoire's `isStandaloneHeroAnimation`.
pub(crate) fn standalone(clip: &ClipCandidate) -> bool {
    if clip.name.trim().is_empty()
        || !clip.fps.is_finite()
        || clip.fps <= 0.0
        || clip.frame_count <= 1
    {
        return false;
    }
    let duration = (clip.frame_count - 1) as f32 / clip.fps;
    if !duration.is_finite() || duration <= 0.001 {
        return false;
    }
    if clip.standalone == Some(false)
        || clip.additive
        || clip.hidden
        || clip.delta
        || clip.requires_base
        || clip.transition
        || clip.root_motion
    {
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

/// The purpose and playback Grimoire's `heroAnimationRecipe` assigns a clip name for a hero.
pub(crate) fn recipe_for(name: &str, hero: &str) -> Option<(Action, Playback)> {
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

fn choice(clip: &ClipCandidate, hero: &str) -> ClipChoice {
    let recipe = recipe_for(&clip.name, hero);
    ClipChoice {
        name: clip.name.clone(),
        // Grimoire's `heroAnimationPlayback`: the graph's, else the recipe's, else loop.
        playback: clip
            .showcase
            .as_ref()
            .map(|showcase| showcase.playback)
            .or(recipe.map(|(_, playback)| playback))
            .unwrap_or(Playback::Loop),
        action: recipe.map(|(action, _)| action),
        showcase: clip.showcase.clone(),
    }
}

/// Choose the preview clips for `hero` (roster display name), in Grimoire's order:
///
/// 1. the UI graph's showcase (shop before hero select, at most 2,400 frames),
/// 2. the hero's recipes (the default idle recipes for an unknown hero); beside a showcase
///    only the first idle recipe hit is kept, as a standing fallback,
/// 3. beside a showcase with no idle recipe hit, the first standing or relaxed idle,
/// 4. with nothing selected, the first neutral idle,
/// 5. compiled standalone clips by name while under eight clips.
///
/// All within 10,000 frames. When nothing qualifies, the menu pose vpkmerge would bake is
/// held, so the hero is never shown in its bind pose while a pose exists (Sigil's addition).
pub fn select_clips(candidates: &[ClipCandidate], hero: &str) -> Vec<ClipChoice> {
    let safe: Vec<&ClipCandidate> = candidates.iter().filter(|clip| standalone(clip)).collect();
    // Grimoire's `byName` map keeps the last clip of a name.
    let find = |name: &str| safe.iter().rev().copied().find(|clip| clip.name == name);
    let recipes = match hero_recipes(hero) {
        [] => DEFAULT_RECIPES,
        own => own,
    };

    let mut selected: Vec<&ClipCandidate> = Vec::new();
    let mut frames = 0usize;
    let taken = |selected: &[&ClipCandidate], clip: &ClipCandidate| {
        selected.iter().any(|old| old.name == clip.name)
    };

    let showcases = || {
        safe.iter()
            .copied()
            .filter(|clip| clip.showcase.is_some() && clip.frame_count <= MAX_SHOWCASE_FRAMES)
    };
    let showcase = showcases()
        .find(|clip| {
            clip.showcase
                .as_ref()
                .is_some_and(|showcase| showcase.state == ShowcaseState::Shop)
        })
        .or_else(|| showcases().next());
    if let Some(clip) = showcase {
        selected.push(clip);
        frames += clip.frame_count;
    }

    // Beside a graph default, one standing idle only.
    let mut fallback = false;
    for recipe in recipes {
        let idle = recipe.action.is_idle();
        if showcase.is_some() && idle && fallback {
            continue;
        }
        let Some(clip) = recipe
            .names
            .iter()
            .filter_map(|name| find(name))
            .find(|clip| !taken(&selected, clip))
        else {
            continue;
        };
        if frames + clip.frame_count > MAX_FRAMES {
            continue;
        }
        selected.push(clip);
        frames += clip.frame_count;
        if showcase.is_some() && idle {
            fallback = true;
        }
    }

    // A hero's own recipes may name no standing idle (Rem's names only its shop pose).
    if showcase.is_some()
        && !fallback
        && let Some(clip) = STANDING_IDLE
            .iter()
            .chain(RELAXED_IDLE)
            .filter_map(|name| find(name))
            .find(|clip| !taken(&selected, clip))
        && frames + clip.frame_count <= MAX_FRAMES
    {
        selected.push(clip);
        frames += clip.frame_count;
    }

    // A reviewed default may disappear from a skin or game update: try a neutral idle, never
    // the longest remaining gameplay layer.
    if selected.is_empty()
        && let Some(clip) = STANDING_IDLE
            .iter()
            .chain(RELAXED_IDLE)
            .find_map(|name| find(name))
        && clip.frame_count <= MAX_FRAMES
    {
        selected.push(clip);
        frames += clip.frame_count;
    }

    let mut fill: Vec<&ClipCandidate> = safe
        .iter()
        .copied()
        .filter(|clip| clip.standalone == Some(true))
        .collect();
    fill.sort_by(|a, b| a.name.cmp(&b.name));
    for clip in fill {
        if selected.len() >= MAX_CLIPS
            || taken(&selected, clip)
            || frames + clip.frame_count > MAX_FRAMES
        {
            continue;
        }
        selected.push(clip);
        frames += clip.frame_count;
    }

    let mut out: Vec<ClipChoice> = selected.iter().map(|clip| choice(clip, hero)).collect();

    if out.is_empty()
        && let Some(clip) = vpkmerge_core::DEFAULT_POSE_CLIPS.iter().find_map(|name| {
            candidates
                .iter()
                .find(|clip| clip.name.eq_ignore_ascii_case(name) && !clip.additive)
        })
    {
        out.push(ClipChoice {
            name: clip.name.clone(),
            playback: Playback::Hold,
            action: Some(HeroPose),
            showcase: None,
        });
    }

    out.truncate(MAX_CLIPS);
    out
}

/// The clip a preview starts on (Grimoire's `preferredHeroAnimation`): the showcase, else the
/// first recipe name present, else a standing or relaxed idle, else a hero pose, else the
/// first clip.
pub(crate) fn default_clip(choices: &[ClipChoice], hero: &str) -> Option<String> {
    if let Some(showcase) = choices.iter().find(|choice| choice.showcase.is_some()) {
        return Some(showcase.name.clone());
    }
    let names: Vec<&str> = choices.iter().map(|choice| choice.name.as_str()).collect();
    for recipe in hero_recipes(hero) {
        if let Some(name) = recipe.names.iter().find(|name| names.contains(name)) {
            return Some((*name).to_string());
        }
    }
    STANDING_IDLE
        .iter()
        .chain(RELAXED_IDLE)
        .chain(&["hero_pose", "ui_hero_pose"])
        .find(|name| names.contains(name))
        .copied()
        .or_else(|| names.first().copied())
        .map(str::to_string)
}

/// The English label a clip picker shows (Grimoire's `heroClipLabel`): the recipe's action,
/// else the clip's own words.
pub fn clip_label(name: &str, hero: &str) -> String {
    match recipe_for(name, hero).map(|(action, _)| action) {
        Some(Idle) => "Idle".into(),
        Some(RelaxedIdle) => "Relaxed idle".into(),
        Some(HeroPose) => "Hero pose".into(),
        Some(Running) => "Run".into(),
        Some(Reload) => "Reload".into(),
        Some(Dash) => "Dash".into(),
        Some(HeavyMelee) => "Heavy melee".into(),
        None => humanize(name),
    }
}

/// `models/x/clips/primaryStand_idle.vnmclip_c` as `Primary Stand idle`.
fn humanize(name: &str) -> String {
    let mut base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    for extension in [".vanim_c", ".vnmclip_c", ".vanim", ".vnmclip"] {
        if lower.ends_with(extension) {
            base = &base[..base.len() - extension.len()];
            break;
        }
    }
    let mut spaced = String::with_capacity(base.len() + 4);
    let mut previous: Option<char> = None;
    for current in base.chars() {
        if previous.is_some_and(|c| c.is_ascii_lowercase()) && current.is_ascii_uppercase() {
            spaced.push(' ');
        }
        spaced.push(current);
        previous = Some(current);
    }
    let words = spaced
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let words = words.trim();
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => name.to_string(),
    }
}

/// The picker section a clip belongs in (Grimoire's `groupHeroClips`).
pub fn clip_group(name: &str, hero: &str) -> ClipGroup {
    if matches!(
        recipe_for(name, hero),
        Some((Idle | RelaxedIdle | HeroPose, _))
    ) {
        return ClipGroup::Idle;
    }
    let lower = name.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    const COMBAT: &[&str] = &[
        "attack", "fire", "shoot", "reload", "melee", "punch", "cast", "ability", "ult", "ultimate",
    ];
    const IDLE: &[&str] = &["idle", "menu", "loadout", "select"];
    const MOVEMENT: &[&str] = &[
        "walk", "run", "sprint", "jump", "land", "crouch", "dash", "slide", "climb", "move",
    ];
    let movement = |token: &str| {
        let word = token.trim_end_matches(|c: char| c.is_ascii_digit());
        MOVEMENT.contains(&word)
    };
    if tokens.iter().any(|token| COMBAT.contains(token)) {
        ClipGroup::Combat
    } else if tokens.iter().any(|token| IDLE.contains(token)) {
        ClipGroup::Idle
    } else if tokens.iter().any(|token| movement(token)) {
        ClipGroup::Movement
    } else {
        ClipGroup::Other
    }
}

// Clip file metadata --------------------------------------------------------------------------

/// What a `.vnmclip_c` says about itself, read without decoding its keyframes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClipFileMeta {
    /// `m_skeleton`, the `.vnmskel` it was authored for.
    pub skeleton: Option<String>,
    pub frame_count: usize,
    pub duration: f32,
    pub additive: bool,
    pub root_motion: bool,
}

impl ClipFileMeta {
    /// Frames per second from the duration, 0 for a single frame or no duration (which the
    /// standalone filter rejects).
    pub fn fps(&self) -> f32 {
        if self.duration > 0.0 && self.frame_count > 1 {
            (self.frame_count - 1) as f32 / self.duration
        } else {
            0.0
        }
    }

    pub fn candidate(&self, name: &str, showcase: Option<Showcase>) -> ClipCandidate {
        ClipCandidate {
            name: name.to_string(),
            frame_count: self.frame_count,
            fps: self.fps(),
            additive: self.additive,
            root_motion: self.root_motion,
            showcase,
            ..ClipCandidate::default()
        }
    }

    /// Whether this file is the same motion as a clip of `frames` frames at `fps`: equal frame
    /// counts and durations within half a frame (Grimoire's `parseHeroAnimationMetadata`).
    pub fn same_timing(&self, frames: usize, fps: f32) -> bool {
        if self.frame_count != frames || !fps.is_finite() || fps <= 0.0 || frames == 0 {
            return false;
        }
        let duration = (frames - 1) as f32 / fps;
        (self.duration - duration).abs() <= (0.5 / fps).max(0.001)
    }
}

/// The metadata of a decoded `.vnmclip_c` DATA block, or `None` when it is not a clip.
pub(crate) fn clip_file_meta(data: &Value) -> Option<ClipFileMeta> {
    let frame_count = usize::try_from(data.get("m_nNumFrames")?.as_uint()?).ok()?;
    let duration = data.get("m_flDuration")?.as_f64()?;
    if !duration.is_finite() || duration < 0.0 {
        return None;
    }
    let additive = data.get("m_bIsAdditive")?.as_bool()?;
    let root = data.get("m_rootMotion");
    let speed = |key: &str| {
        root.and_then(|root| root.get(key))
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
    };
    Some(ClipFileMeta {
        skeleton: data
            .get("m_skeleton")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_string),
        frame_count,
        duration: duration as f32,
        additive,
        root_motion: speed("m_flAverageLinearVelocity").abs() > 0.01
            || speed("m_flAverageAngularVelocityRadians").abs() > 0.001,
    })
}

/// Embedded clips whose clip file goes by another name, verified against source and timing
/// (Grimoire's `PRODUCTION_CLIP_ALIASES`).
const PRODUCTION_CLIP_ALIASES: &[(&str, &[(&str, &str)])] = &[
    (
        "models/heroes_wip/inferno/inferno.vmdl_c",
        &[("shop_menu_base", "ui_shop_idle")],
    ),
    (
        "models/heroes_staging/hornet_v3/hornet.vmdl_c",
        &[("primary_stand_idle", "weapon_stand_idle")],
    ),
    (
        "models/heroes_wip/abrams/abrams.vmdl_c",
        &[("shop_menu_base", "ui_shop_idle")],
    ),
    (
        "models/heroes_wip/wraith/wraith.vmdl_c",
        &[("primary_stand_reload", "reload_idle")],
    ),
    (
        "models/heroes_wip/mcginnis/mcginnis.vmdl_c",
        &[("primary_stand_reload", "reload_idle")],
    ),
    (
        "models/heroes_staging/chrono/chrono.vmdl_c",
        &[
            ("shop_menu_base", "ui_shop_idle"),
            ("primary_stand_reload", "reload_idle"),
        ],
    ),
    (
        "models/heroes_wip/dynamo/dynamo.vmdl_c",
        &[
            ("primary_stand_idle", "weapon_stand_idle"),
            ("primary_stand_reload", "reload_idle"),
        ],
    ),
    (
        "models/heroes_staging/haze/haze.vmdl_c",
        &[
            ("shop_menu_base", "ui_shop_idle"),
            ("primary_stand_reload", "reload_idle"),
        ],
    ),
    (
        "models/heroes_staging/bebop/bebop.vmdl_c",
        &[("primary_stand_idle", "weapon_stand_idle")],
    ),
    (
        "models/heroes_staging/nano/nano_v2/nano.vmdl_c",
        &[
            ("primary_stand_idle", "ui_shop_idle"),
            ("primary_stand_reload", "reload_idle"),
        ],
    ),
    (
        "models/heroes_staging/archer/archer.vmdl_c",
        &[
            ("shop_menu_base", "ui_shop_idle"),
            ("primary_stand_reload", "reload_idle"),
        ],
    ),
    (
        "models/heroes_staging/digger/digger.vmdl_c",
        &[("primary_stand_idle", "weapon_stand_idle")],
    ),
    (
        "models/heroes_staging/viper/viper.vmdl_c",
        &[("primary_run_n", "weapon_run_n")],
    ),
];

/// The clip files that may carry metadata for the embedded clip `name` of model `entry`: the
/// same name, the reviewed alias, and `reload_idle` for the Dynamo, Wraith and Yamato reload
/// (Grimoire's `heroAnimationMetadataPaths`).
pub(crate) fn metadata_paths(entry: &str, name: &str) -> Vec<String> {
    let folder = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut paths = vec![format!("{folder}/clips/{name}.vnmclip_c")];
    if let Some(alias) = PRODUCTION_CLIP_ALIASES
        .iter()
        .find(|(model, _)| *model == entry)
        .and_then(|(_, aliases)| aliases.iter().find(|(from, _)| *from == name))
        .map(|(_, alias)| *alias)
    {
        paths.push(format!("{folder}/clips/{alias}.vnmclip_c"));
    }
    let parts: Vec<&str> = entry.split('/').collect();
    if name == "primary_stand_reload"
        && parts.len() == 4
        && parts[0] == "models"
        && parts[1].starts_with("heroes")
        && ["dynamo", "wraith", "yamato"].contains(&parts[2])
        && parts[3].ends_with(".vmdl_c")
    {
        let reload = format!("{folder}/clips/reload_idle.vnmclip_c");
        if !paths.contains(&reload) {
            paths.push(reload);
        }
    }
    paths
}

/// Flag embedded clips from their neighbouring clip files: for the first eight recipe clips
/// by name, the first file of [`metadata_paths`] with the same timing sets `additive` and
/// `root_motion`. A missing or mistimed file invents nothing.
pub(crate) fn flag_embedded(
    entry: &str,
    hero: &str,
    clips: &mut [ClipCandidate],
    mut read: impl FnMut(&str) -> Option<ClipFileMeta>,
) {
    let safe_entry = entry.starts_with("models/heroes")
        && entry.ends_with(".vmdl_c")
        && !entry.contains("..")
        && entry
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'));
    if !safe_entry {
        return;
    }
    let mut order: Vec<usize> = (0..clips.len())
        .filter(|&index| recipe_for(&clips[index].name, hero).is_some())
        .collect();
    order.sort_by(|&a, &b| clips[a].name.cmp(&clips[b].name));
    for index in order.into_iter().take(MAX_CLIPS) {
        let clip = &mut clips[index];
        if !clip
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        for path in metadata_paths(entry, &clip.name) {
            let Some(meta) = read(&path) else {
                continue;
            };
            if !meta.same_timing(clip.frame_count, clip.fps) {
                continue;
            }
            clip.additive |= meta.additive;
            clip.root_motion |= meta.root_motion;
            break;
        }
    }
}

// UI AnimGraph2 -------------------------------------------------------------------------------

/// The animation graph and skeleton references in a model's DATA block.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ModelRefs {
    /// `m_animGraph2Refs`: identifier (`ui`, `default`, ...) and graph path.
    pub graphs: Vec<(String, String)>,
    /// `m_vecNmSkeletonRefs`.
    pub skeletons: Vec<String>,
}

pub(crate) fn model_refs(data: &Value) -> ModelRefs {
    let graphs = data
        .get("m_animGraph2Refs")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| {
            Some((
                item.get("m_sIdentifier")?.as_str()?.to_string(),
                item.get("m_hGraph")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let skeletons = data
        .get("m_vecNmSkeletonRefs")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect();
    ModelRefs { graphs, skeletons }
}

impl ModelRefs {
    /// Whether a showcase clip authored for `skeleton` may pose this model: always when the
    /// model declares no NM skeletons, else only one of them (inherited graph variations can
    /// point at another hero's skeleton).
    pub fn accepts_skeleton(&self, skeleton: Option<&str>) -> bool {
        if self.skeletons.is_empty() {
            return true;
        }
        let Some(skeleton) = skeleton else {
            return false;
        };
        let wanted = super::stack::compiled(skeleton);
        self.skeletons
            .iter()
            .any(|known| super::stack::compiled(known) == wanted)
    }
}

/// Clip files the model's graphs name, and which of them the UI graph showcases.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct GraphClips {
    /// Compiled `.vnmclip_c` paths in graph order (the `ui` graph first), present in the stack.
    pub resources: Vec<String>,
    /// The `resources` the `ui` graph names, in its order (their animation events feed the
    /// particle bundle, as Grimoire's `discoverAnimationEvents` reads them).
    pub ui: Vec<String>,
    /// Showcase per compiled clip path. A shop state wins over hero select for one clip.
    pub showcases: BTreeMap<String, Showcase>,
}

/// Read every graph of the model (the `ui` graph first) and mine the `ui` graph for showcase
/// states. `decode` returns a compiled graph's DATA block as JSON; `available` says whether a
/// compiled path is in the stack. Ported from Grimoire's `discoverRiggedHeroClips`.
pub(crate) fn read_graphs(
    refs: &ModelRefs,
    mut decode: impl FnMut(&str) -> Option<Json>,
    available: &dyn Fn(&str) -> bool,
) -> GraphClips {
    let mut out = GraphClips::default();
    let mut ordered: Vec<&(String, String)> = refs.graphs.iter().collect();
    ordered.sort_by_key(|(id, _)| !id.eq_ignore_ascii_case("ui"));
    for (id, graph) in ordered {
        let graph_path = super::stack::compiled(graph);
        let Some(data) = decode(&graph_path) else {
            continue;
        };
        for resource in kv::array(data.get("m_resources")) {
            let Some(resource) = resource.as_str() else {
                continue;
            };
            let compiled = super::stack::compiled(resource);
            if !compiled.ends_with(".vnmclip_c") || !available(&compiled) {
                continue;
            }
            if id.eq_ignore_ascii_case("ui") && !out.ui.contains(&compiled) {
                out.ui.push(compiled.clone());
            }
            if !out.resources.contains(&compiled) {
                out.resources.push(compiled);
            }
        }
        if !id.eq_ignore_ascii_case("ui") {
            continue;
        }
        for (resource, showcase) in graph_showcases(&data, &graph_path, available) {
            let replace = out
                .showcases
                .get(&resource)
                .is_none_or(|_| showcase.state == ShowcaseState::Shop);
            if replace {
                out.showcases.insert(resource, showcase);
            }
        }
    }
    out
}

fn int(value: Option<&Json>) -> Option<i64> {
    let value = value?;
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && number.fract() == 0.0)
            .map(|number| number as i64)
    })
}

fn class(node: &Json) -> Option<&str> {
    node.get("_class").and_then(Json::as_str)
}

/// The UI screen a state path belongs to: `.../shop` or `.../hero_shop`, `.../select` or
/// `.../hero_select`, any case.
fn state_role(path: &str) -> Option<ShowcaseState> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with("/shop") || lower.ends_with("/hero_shop") {
        Some(ShowcaseState::Shop)
    } else if lower.ends_with("/select") || lower.ends_with("/hero_select") {
        Some(ShowcaseState::HeroSelect)
    } else {
        None
    }
}

/// A clip node that plays one clip as is: a data slot, no reverse or reset input, a positive
/// speed. Returns the slot, looping flag and speed.
fn plain_clip_node(node: &Json) -> Option<(usize, bool, f32)> {
    if class(node) != Some("CNmClipNode::CDefinition") {
        return None;
    }
    let slot = usize::try_from(int(node.get("m_nDataSlotIdx"))?).ok()?;
    let looping = node.get("m_bAllowLooping")?.as_bool()?;
    if int(node.get("m_nPlayInReverseValueNodeIdx")) != Some(-1)
        || int(node.get("m_nResetTimeValueNodeIdx")) != Some(-1)
    {
        return None;
    }
    let speed = node.get("m_flSpeedMultiplier")?.as_f64()?;
    if !speed.is_finite() || speed <= 0.0 {
        return None;
    }
    Some((slot, looping, speed as f32))
}

/// The showcase clips of one decoded UI graph: each unmasked, unlayered state of the root
/// state machine whose path ends in a shop or select role and whose child is a plain clip
/// node. A graph resource list alone cannot prove a leaf is a complete body action.
pub(crate) fn graph_showcases(
    graph: &Json,
    graph_path: &str,
    available: &dyn Fn(&str) -> bool,
) -> Vec<(String, Showcase)> {
    let resources = kv::array(graph.get("m_resources"));
    let paths = kv::array(graph.get("m_nodePaths"));
    let nodes = kv::array(graph.get("m_nodes"));
    let Some(root_index) = int(graph.get("m_nRootNodeIdx")) else {
        return Vec::new();
    };
    let Some(root) = nodes.iter().find(|node| {
        class(node) == Some("CNmStateMachineNode::CDefinition")
            && int(node.get("m_nNodeIdx")) == Some(root_index)
    }) else {
        return Vec::new();
    };
    let states: Vec<i64> = kv::array(root.get("m_stateDefinitions"))
        .iter()
        .filter_map(|state| int(state.get("m_nStateNodeIdx")))
        .collect();

    let mut out: Vec<(String, Showcase)> = Vec::new();
    for node in nodes {
        if class(node) != Some("CNmStateNode::CDefinition") {
            continue;
        }
        let Some(index) = int(node.get("m_nNodeIdx")).filter(|index| *index >= 0) else {
            continue;
        };
        if !states.contains(&index) {
            continue;
        }
        let unlayered = [
            "m_nLayerWeightNodeIdx",
            "m_nLayerRootMotionWeightNodeIdx",
            "m_nLayerBoneMaskNodeIdx",
        ]
        .iter()
        .all(|key| int(node.get(key)) == Some(-1));
        if !unlayered || node.get("m_bIsOffState").and_then(Json::as_bool) != Some(false) {
            continue;
        }
        let path = usize::try_from(index)
            .ok()
            .and_then(|index| paths.get(index))
            .and_then(Json::as_str)
            .unwrap_or("");
        let Some(state) = state_role(path) else {
            continue;
        };
        let Some(child) = int(node.get("m_nChildNodeIdx")) else {
            continue;
        };
        let Some((slot, looping, speed)) = nodes
            .iter()
            .filter(|leaf| int(leaf.get("m_nNodeIdx")) == Some(child))
            .find_map(plain_clip_node)
        else {
            continue;
        };
        let Some(resource) = resources
            .get(slot)
            .and_then(Json::as_str)
            .filter(|resource| !resource.is_empty())
        else {
            continue;
        };
        let compiled = super::stack::compiled(resource);
        if !compiled.ends_with(".vnmclip_c") || !available(&compiled) {
            continue;
        }
        let showcase = Showcase {
            graph: graph_path.to_string(),
            state,
            playback: if looping {
                Playback::Loop
            } else {
                Playback::Hold
            },
            speed,
        };
        match out.iter_mut().find(|(existing, _)| *existing == compiled) {
            Some(existing) => {
                if state == ShowcaseState::Shop {
                    existing.1 = showcase;
                }
            }
            None => out.push((compiled, showcase)),
        }
    }
    out
}
