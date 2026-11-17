//! What Sigil does with each key an upstream config sets.
//!
//! Ported from Grimoire's pin manifest (`scripts/performance-presets.json`, MIT):
//! the excluded sections and keys, the opt-in convars and their groups, the
//! patterns that force a gameplay-shaped key to be classified before it may ride
//! in a config body, and the video.txt rules. The reasons are reworded for the
//! detail view, where a player reads them.

/// Sections never written, whatever upstream sets: engine tools, not the game.
pub const EXCLUDE_SECTIONS: &[&[&str]] = &[
    &["FileSystem"],
    &["Hammer"],
    &["pulse"],
    &["ResourceCompiler"],
    &["ContentBuilder"],
    &["SteamAudio"],
    &["MaterialSystem2", "RenderModes"],
];

/// Why a section is never written.
pub fn section_reason(path: &[String]) -> &'static str {
    match path.first().map(String::as_str) {
        Some("FileSystem") => "Load order for mods and game content. Sigil keeps yours.",
        Some("MaterialSystem2") => "A list of render modes, not settings. Editing it breaks them.",
        _ => "An engine tool section that does not change frame rate.",
    }
}

/// Keys never written, with the reason the detail view shows.
pub const EXCLUDE_KEYS: &[(&str, &str)] = &[
    (
        "AllowKeyChordBindings",
        "A key binding preference, not a performance setting.",
    ),
    (
        "PGIVersion",
        "Valve's version stamp for the file. Changing it gains nothing.",
    ),
    (
        "citadel_hideout_ball_show_juggle_count",
        "A hideout toy, not a performance setting.",
    ),
    (
        "citadel_hideout_ball_show_juggle_fx",
        "A hideout toy, not a performance setting.",
    ),
    (
        "LocalServerClientAccess",
        "Engine plumbing an older config copied into the wrong section.",
    ),
    (
        "DistanceField",
        "Crashes the current build shortly after the hideout loads.",
    ),
    (
        "r_lightmap_size",
        "Small values turn the map nearly black since the October 2026 update.",
    ),
];

/// Keys never written by pattern (case-insensitive), with the reason.
pub const EXCLUDE_PATTERNS: &[(&str, &str)] = &[
    (
        r"^(host_)?timescale$",
        "Changes game speed, not render cost.",
    ),
    (
        r"^sv_cheats$",
        "The cheat switch. No frame-rate config needs it.",
    ),
];

/// Which part of the game an opt-in convar changes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    /// Glows, outlines, health bars, tracers: what you can see.
    Visibility,
    /// Field of view, pitch and aspect ratio: how the camera frames the world.
    Camera,
    /// Debug drawing and hideout testing tools.
    Devtools,
}

impl Group {
    pub fn id(self) -> &'static str {
        match self {
            Group::Visibility => "visibility",
            Group::Camera => "camera",
            Group::Devtools => "devtools",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Group::Visibility => "Visibility",
            Group::Camera => "Camera",
            Group::Devtools => "Developer tools",
        }
    }

    /// Author values in this group are on unless the player turns them off.
    pub fn on_by_default(self) -> bool {
        !matches!(self, Group::Devtools)
    }
}

/// Convars that change what the player sees or how the camera frames the world
/// rather than how fast it renders. Each is a separate switch in the detail view.
pub const OPT_IN_KEYS: &[(&str, Group)] = &[
    ("citadel_player_outline_enemies", Group::Visibility),
    ("citadel_trooper_outline_enabled", Group::Visibility),
    ("citadel_trooper_friendly_glow_disabled", Group::Visibility),
    ("citadel_trooper_glow_disabled", Group::Visibility),
    ("citadel_player_glow_disabled", Group::Visibility),
    ("citadel_boss_glow_disabled", Group::Visibility),
    (
        "citadel_unit_status_allies_see_thru_walls",
        Group::Visibility,
    ),
    (
        "citadel_unit_status_allies_see_thru_walls_max_distance",
        Group::Visibility,
    ),
    ("citadel_unit_status_use_new", Group::Visibility),
    ("citadel_unit_status_hide_names", Group::Visibility),
    ("citadel_unit_status_use_v2", Group::Visibility),
    (
        "citadel_unit_status_use_v2_for_nonplayers",
        Group::Visibility,
    ),
    ("citadel_unit_status_single_bar_mode", Group::Visibility),
    ("citadel_unit_status_delta_decay_delay", Group::Visibility),
    ("citadel_unit_status_delta_decay_rate", Group::Visibility),
    ("citadel_unit_status_dpi", Group::Visibility),
    ("citadel_unit_status_stamina_low_pips", Group::Visibility),
    (
        "citadel_unit_status_recent_active_damage_time",
        Group::Visibility,
    ),
    ("r_particle_timescale", Group::Visibility),
    ("r_drawtracers", Group::Visibility),
    ("r_drawtracers_firstperson", Group::Visibility),
    ("cl_glow_brightness", Group::Visibility),
    ("r_citadel_outlines", Group::Visibility),
    ("r_citadel_npr_outlines", Group::Visibility),
    ("r_citadel_npr_outlines_max_dist", Group::Visibility),
    ("r_citadel_glow_health_bars", Group::Visibility),
    ("r_citadel_selection_outline2_alpha", Group::Visibility),
    ("r_citadel_selection_outline2_offset", Group::Visibility),
    ("r_citadel_selection_outline2_width", Group::Visibility),
    ("r_citadel_npr_force_solid_outline", Group::Visibility),
    (
        "citadel_player_glow_from_teammate_vision_max_range",
        Group::Visibility,
    ),
    ("r_aspectratio", Group::Camera),
    ("citadel_camera_hero_fov", Group::Camera),
    ("default_fov", Group::Camera),
    ("viewmodel_fov", Group::Camera),
    ("citadel_camera_pitch_max", Group::Camera),
    ("citadel_reduce_camera_shake", Group::Camera),
    ("citadel_camera_pitch_min", Group::Camera),
    ("citadel_camera_pitch_default", Group::Camera),
    ("citadel_hideout_enable_testing_tools", Group::Devtools),
    ("movement_stats_debug_draw", Group::Devtools),
    ("citadel_orb_debug_draw_state", Group::Devtools),
    ("music_hideout_debug_enabled", Group::Devtools),
];

/// A key matching one of these must be classified (opt-in, excluded, or allowed
/// below) before it may ride in a config body. Anything else that matches is
/// held back and listed, so an upstream update cannot slip a new outline or
/// field-of-view convar into the body.
pub const OPT_IN_PATTERNS: &[&str] = &[
    "timescale",
    "outline",
    "glow",
    "see_thru",
    "_fov$",
    "aspectratio",
    "debug_draw",
    "debug_show",
    "hideout",
    "camera_pitch",
    "citadel_unit_status_",
    "tracer",
];

/// Pattern-matching keys audited as genuine performance settings.
pub const ALLOW_IN_BODY: &[&str] = &[
    "aspectratiomode",
    "cl_phys_timescale",
    "debug_draw_enable",
    "r_citadel_glow_health_bar_debug",
    "citadel_unit_status_old_update_rate",
    "citadel_hud_objective_health_debug_show_midboss",
];

/// video.txt settings that describe the player's screen, not render cost.
/// Never written, whatever upstream ships.
pub const VIDEO_EXCLUDE_PATTERNS: &[&str] = &[
    "^defaultres",
    "^recommendedheight$",
    "^refreshrate_",
    "fullscreen|^nowindowborder$",
    "^monitor_index$",
    "^high_dpi$",
    "^aspectratiomode$",
    "^knowndevice$",
    "gamma",
    "^mat_vsync$",
    "^fps_max",
];

/// Ceilings on numeric video.txt values, for sources that do not ask for their
/// file verbatim.
pub const VIDEO_MAX: &[(&str, f64)] = &[("r_texture_stream_mip_bias", 4.0)];

pub fn opt_in_group(key: &str) -> Option<Group> {
    OPT_IN_KEYS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, group)| *group)
}

/// Why a key is never written, or `None` when it may be.
pub fn key_excluded(key: &str) -> Option<&'static str> {
    if let Some((_, why)) = EXCLUDE_KEYS.iter().find(|(name, _)| *name == key) {
        return Some(why);
    }
    EXCLUDE_PATTERNS
        .iter()
        .find(|(pattern, _)| matches_ci(pattern, key))
        .map(|(_, why)| *why)
}

pub fn section_excluded(path: &[String]) -> bool {
    EXCLUDE_SECTIONS.iter().any(|ex| {
        ex.iter()
            .enumerate()
            .all(|(i, seg)| path.get(i).map(String::as_str) == Some(*seg))
    })
}

/// A gameplay-shaped key nobody has classified.
pub fn unclassified(key: &str) -> bool {
    !ALLOW_IN_BODY.contains(&key) && OPT_IN_PATTERNS.iter().any(|p| matches_ci(p, key))
}

pub fn video_display_key(key: &str) -> bool {
    VIDEO_EXCLUDE_PATTERNS.iter().any(|p| matches_ci(p, key))
}

fn matches_ci(pattern: &'static str, key: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<&'static str, regex::Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    let re = cache.entry(pattern).or_insert_with(|| {
        regex::Regex::new(&format!("(?i){pattern}")).expect("classification regex")
    });
    re.is_match(key)
}

/// What an opt-in convar does, in a few plain words.
pub fn opt_in_label(key: &str) -> Option<&'static str> {
    Some(match key {
        "citadel_player_outline_enemies" => "Enemy hero outlines",
        "citadel_trooper_outline_enabled" => "Trooper outlines",
        "citadel_trooper_friendly_glow_disabled" => "Hide friendly trooper glow",
        "citadel_trooper_glow_disabled" => "Hide trooper glow",
        "citadel_player_glow_disabled" => "Hide hero glow",
        "citadel_boss_glow_disabled" => "Hide boss glow",
        "citadel_unit_status_allies_see_thru_walls" => "Ally health bars through walls",
        "citadel_unit_status_allies_see_thru_walls_max_distance" => {
            "Ally health bars through walls, range"
        }
        "citadel_unit_status_use_new" => "New health bar style",
        "citadel_unit_status_hide_names" => "Hide names over health bars",
        "citadel_unit_status_use_v2" => "Health bar style v2",
        "citadel_unit_status_use_v2_for_nonplayers" => "Health bar v2 for troopers and objectives",
        "citadel_unit_status_single_bar_mode" => "Single health bar",
        "citadel_unit_status_delta_decay_delay" => "Recent damage fade delay",
        "citadel_unit_status_delta_decay_rate" => "Recent damage fade speed",
        "citadel_unit_status_dpi" => "Health bar size",
        "citadel_unit_status_stamina_low_pips" => "Show stamina below",
        "citadel_unit_status_recent_active_damage_time" => "Recent damage highlight time",
        "r_particle_timescale" => "Particle effect speed",
        "r_drawtracers" => "Bullet tracers",
        "r_drawtracers_firstperson" => "Your own bullet tracers",
        "cl_glow_brightness" => "Glow brightness",
        "r_citadel_outlines" => "Outlines",
        "r_citadel_npr_outlines" => "Stylized outlines",
        "r_citadel_npr_outlines_max_dist" => "Stylized outline distance",
        "r_citadel_glow_health_bars" => "Health bar glow",
        "r_citadel_selection_outline2_alpha" => "Selection outline opacity",
        "r_citadel_selection_outline2_offset" => "Selection outline offset",
        "r_citadel_selection_outline2_width" => "Selection outline width",
        "r_citadel_npr_force_solid_outline" => "Solid outlines",
        "citadel_player_glow_from_teammate_vision_max_range" => "Glow range from teammate vision",
        "r_aspectratio" => "Aspect ratio",
        "citadel_camera_hero_fov" => "Field of view",
        "default_fov" => "Default field of view",
        "viewmodel_fov" => "Weapon field of view",
        "citadel_camera_pitch_max" => "Highest camera angle",
        "citadel_reduce_camera_shake" => "Reduce camera shake",
        "citadel_camera_pitch_min" => "Lowest camera angle",
        "citadel_camera_pitch_default" => "Resting camera angle",
        "citadel_hideout_enable_testing_tools" => "Hideout testing tools",
        "movement_stats_debug_draw" => "Movement debug overlay",
        "citadel_orb_debug_draw_state" => "Soul orb debug overlay",
        "music_hideout_debug_enabled" => "Hideout music debug",
        _ => return None,
    })
}
