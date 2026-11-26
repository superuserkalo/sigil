//! Sigil's own settings: one JSON object in `<data>/settings.json`, written atomically.
//!
//! Every field reads on its own with a fallback, so a value from an older or newer build, or a
//! hand edit gone wrong, costs that one setting and not the rest. Headless runs (snapshots,
//! scenes) never read or write the file, so a scene renders the defaults.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use deadlock_core::gamebanana::BrowseSort;
use rune::Color;
use serde_json::{Map, Value, json};

use crate::model::Screen;
use crate::screens::settings::i18n::Lang;
use crate::screens::settings::log::Level;
use crate::ui::Theme;

pub const FILE_NAME: &str = "settings.json";

/// Whether this build carries the mod agent: the Agent page and the pages under it, Models and
/// sign-in, the agent's rows in Settings and its host process. Off unless built with the cargo
/// feature `agent` (ROADMAP.md). Off, the stored `agent` switch is kept but reads false.
pub const AGENT: bool = cfg!(feature = "agent");

/// Whether this build carries the translations: German and the language choice in Settings and
/// its search entry. Off unless built with the cargo feature `languages` (ROADMAP.md). Off, the
/// app is English everywhere; a stored language choice is kept in the settings file but ignored.
pub const LANGUAGES: bool = cfg!(feature = "languages");

/// The flag id of the Agent switch in Settings > Experimental.
pub const AGENT_FLAG: &str = "agent";

/// The interface sizes offered, as a page zoom.
pub const ZOOMS: [f32; 4] = [0.9, 1.0, 1.1, 1.25];

/// A stored "Open on" page: any page the sidebar can show. One that is switched off now is kept
/// as stored ([`Prefs::opening_page`] falls back to Browse until the player picks another).
fn startup_from_key(id: &str) -> Option<Screen> {
    crate::app::PLACES
        .into_iter()
        .chain([Screen::Maps, Screen::Agent])
        .find(|screen| screen.key() == id)
}

/// What happens to Sigil once Steam has been asked to start the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OnLaunch {
    #[default]
    Stay,
    Minimize,
    Close,
}

impl OnLaunch {
    pub const ALL: [OnLaunch; 3] = [OnLaunch::Stay, OnLaunch::Minimize, OnLaunch::Close];

    pub fn id(self) -> &'static str {
        match self {
            OnLaunch::Stay => "stay",
            OnLaunch::Minimize => "minimize",
            OnLaunch::Close => "close",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OnLaunch::Stay => "Stay open",
            OnLaunch::Minimize => "Minimize",
            OnLaunch::Close => "Quit Sigil",
        }
    }
}

/// How Browse treats mods GameBanana marks as NSFW.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mature {
    Show,
    #[default]
    Blur,
    Hide,
}

impl Mature {
    pub const ALL: [Mature; 3] = [Mature::Show, Mature::Blur, Mature::Hide];

    pub fn id(self) -> &'static str {
        match self {
            Mature::Show => "show",
            Mature::Blur => "blur",
            Mature::Hide => "hide",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mature::Show => "Show",
            Mature::Blur => "Blur",
            Mature::Hide => "Hide",
        }
    }
}

pub const SORTS: [BrowseSort; 3] = [BrowseSort::Default, BrowseSort::New, BrowseSort::Updated];

pub fn sort_label(sort: BrowseSort) -> &'static str {
    match sort {
        BrowseSort::Default => "Popular now",
        BrowseSort::New => "Newest",
        BrowseSort::Updated => "Recently updated",
    }
}

/// An accent: the theme's own (ember), one of the presets, or a colour the user typed.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum Accent {
    #[default]
    Theme,
    Preset(&'static str),
    Custom(String),
}

/// Preset accents, base colour. The values follow Grimoire's accent presets (MIT).
pub const ACCENTS: [(&str, &str, &str); 8] = [
    ("ember", "Ember", ""),
    ("amber", "Amber", "#f59e0b"),
    ("crimson", "Crimson", "#ef4444"),
    ("rose", "Rose", "#ec4899"),
    ("violet", "Violet", "#8b5cf6"),
    ("azure", "Azure", "#3b82f6"),
    ("cyan", "Cyan", "#06b6d4"),
    ("emerald", "Emerald", "#10b981"),
];

impl Accent {
    pub fn id(&self) -> String {
        match self {
            Accent::Theme => "ember".to_string(),
            Accent::Preset(id) => (*id).to_string(),
            Accent::Custom(hex) => hex.clone(),
        }
    }

    fn from_id(id: &str) -> Accent {
        if id == "ember" || id.is_empty() {
            return Accent::Theme;
        }
        if let Some((key, _, _)) = ACCENTS.iter().find(|(key, _, _)| *key == id) {
            return Accent::Preset(key);
        }
        match parse_hex(id) {
            Some(_) => Accent::Custom(id.to_lowercase()),
            None => Accent::Theme,
        }
    }

    /// The colour to paint with. None means the theme's own accent.
    pub fn color(&self) -> Option<Color> {
        match self {
            Accent::Theme => None,
            Accent::Preset(id) => ACCENTS
                .iter()
                .find(|(key, _, _)| key == id)
                .and_then(|(_, _, hex)| parse_hex(hex)),
            Accent::Custom(hex) => parse_hex(hex),
        }
    }
}

/// `#rgb` or `#rrggbb`, with or without the hash.
pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim();
    let body = s.strip_prefix('#').unwrap_or(s);
    if !(body.len() == 6 || body.len() == 3) || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    rune::color::hex(&format!("#{body}"))
}

/// How absolute dates read. The default is the European day.month.year.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DateFormat {
    #[default]
    DayDots,
    Iso,
    DayFirst,
    MonthFirst,
}

impl DateFormat {
    pub const ALL: [DateFormat; 4] =
        [DateFormat::DayDots, DateFormat::Iso, DateFormat::DayFirst, DateFormat::MonthFirst];

    pub fn id(self) -> &'static str {
        match self {
            DateFormat::DayDots => "dmy-dots",
            DateFormat::Iso => "iso",
            DateFormat::DayFirst => "dmy",
            DateFormat::MonthFirst => "mdy",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DateFormat::DayDots => "17.11.2026",
            DateFormat::Iso => "2026-11-17",
            DateFormat::DayFirst => "17/11/2026",
            DateFormat::MonthFirst => "11/17/2026",
        }
    }

    /// Format a civil date.
    pub fn format(self, y: i64, m: u32, d: u32) -> String {
        match self {
            DateFormat::DayDots => format!("{d:02}.{m:02}.{y:04}"),
            DateFormat::Iso => format!("{y:04}-{m:02}-{d:02}"),
            DateFormat::DayFirst => format!("{d:02}/{m:02}/{y:04}"),
            DateFormat::MonthFirst => format!("{m:02}/{d:02}/{y:04}"),
        }
    }
}

/// The chosen date format for code that has no `State` at hand (rows built on a worker, core
/// helpers). [`Prefs::load`] and [`edit`] keep it in step with the settings.
/// Unit tests keep it per thread, so a test that loads real settings does not change another's.
#[cfg(not(test))]
static DATE_FORMAT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
#[cfg(test)]
thread_local! {
    static DATE_FORMAT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

pub fn set_date_format(format: DateFormat) {
    let index = DateFormat::ALL.iter().position(|f| *f == format).unwrap_or(0) as u8;
    #[cfg(not(test))]
    DATE_FORMAT.store(index, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    DATE_FORMAT.with(|f| f.set(index));
}

pub fn date_format() -> DateFormat {
    #[cfg(not(test))]
    let index = DATE_FORMAT.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    let index = DATE_FORMAT.with(|f| f.get());
    DateFormat::ALL[usize::from(index) % DateFormat::ALL.len()]
}

/// A civil date in the chosen format.
pub fn show_date(y: i64, m: u32, d: u32) -> String {
    date_format().format(y, m, d)
}

/// The date of a Unix second (UTC) in the chosen format.
pub fn show_unix_date(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    show_date(y, m, d)
}

/// The date of an RFC 3339 or ISO stamp (`2026-11-17T10:22:01Z`, `2026-11-17 10:22`,
/// `2026-11-17`) in the chosen format. `None` when it does not start with a date.
pub fn show_stamp_date(stamp: &str) -> Option<String> {
    let y: i64 = stamp.get(0..4)?.parse().ok()?;
    let m: u32 = stamp.get(5..7)?.parse().ok()?;
    let d: u32 = stamp.get(8..10)?.parse().ok()?;
    let dashes = stamp.get(4..5) == Some("-") && stamp.get(7..8) == Some("-");
    (dashes && (1..=12).contains(&m) && (1..=31).contains(&d)).then(|| show_date(y, m, d))
}

/// The local calendar day of a Unix second in the chosen format; empty for 0.
pub fn show_local_date(ts: i64) -> String {
    deadlock_core::gamebanana::local_parts(ts).map(|(y, m, d, _, _)| show_date(y, m, d)).unwrap_or_default()
}

/// The local date (chosen format) and time of a Unix second; empty for 0.
pub fn show_local_stamp(ts: i64) -> String {
    deadlock_core::gamebanana::local_parts(ts)
        .map(|(y, m, d, h, min)| format!("{} {h:02}:{min:02}", show_date(y, m, d)))
        .unwrap_or_default()
}

/// [`show_stamp_date`] with the stamp's hours and minutes after it, as written (UTC for `Z`
/// stamps). A stamp that is not a date comes back as it is.
pub fn show_stamp(stamp: &str) -> String {
    let Some(date) = show_stamp_date(stamp) else {
        return stamp.to_string();
    };
    match stamp.get(11..16).filter(|t| t.as_bytes().get(2) == Some(&b':')) {
        Some(time) => format!("{date} {time}"),
        None => date,
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's civil_from_days).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Which releases the update check reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Channel {
    #[default]
    Stable,
    Preview,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Stable, Channel::Preview];

    pub fn id(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Preview => "preview",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Stable => "Stable",
            Channel::Preview => "Preview",
        }
    }
}

/// An experimental feature: id, title, one line, default.
pub struct Flag {
    pub id: &'static str,
    pub title: &'static str,
    pub line: &'static str,
    pub default: bool,
}

/// Every experimental flag Sigil reads, shown in Settings > Experimental. A flag read by id
/// that is not listed here or in `PAGES` is off.
pub const FLAGS: [Flag; 2] = [
    Flag {
        id: "agent",
        title: "Agent",
        line: "Shows the agent and its mod-building tools in the sidebar. Experimental: it runs a second Sigil process while on.",
        default: false,
    },
    Flag {
        id: "custom_maps",
        title: "Custom maps",
        line: "Adds a Maps page with GameBanana's maps, and How to play on map mods: launch a map and invite friends.",
        default: false,
    },
];

/// The pages a player can turn off one by one (Deadlock Mod Manager's Features). They share
/// the `flags` map with `FLAGS`, so the Servers switch from before keeps its saved value.
/// Mod management (Browse, Installed, Overlap, Profiles) is always on.
pub const PAGES: [(Screen, Flag); 6] = [
    (
        Screen::Locker,
        Flag {
            id: "page_locker",
            title: "Locker",
            line: "Pick a skin per hero and shuffle skins on each launch. Turning it off also stops the shuffle.",
            default: true,
        },
    ),
    (
        Screen::Crosshair,
        Flag {
            id: "page_crosshair",
            title: "Crosshair",
            line: "Design a crosshair and write it into the game's config. Turning it off takes Sigil's crosshair out of autoexec.cfg, so the game keeps the one you set in game.",
            default: true,
        },
    ),
    (
        Screen::Autoexec,
        Flag {
            id: "page_autoexec",
            title: "Autoexec",
            line: "Edit autoexec.cfg and the game's launch options.",
            default: true,
        },
    ),
    (
        Screen::Performance,
        Flag {
            id: "page_performance",
            title: "Performance",
            line: "Apply community configs that trade visual detail for frame rate.",
            default: true,
        },
    ),
    (
        Screen::Stats,
        Flag {
            id: "page_stats",
            title: "Stats",
            line: "Player stats and match history. Sends the Steam account ids you look up to deadlock-api.",
            default: true,
        },
    ),
    (
        Screen::Servers,
        Flag {
            id: "servers",
            title: "Servers",
            line: "Browse and join Deadworks community servers.",
            default: true,
        },
    ),
];

/// "Mods only" (Deadlock Mod Manager's Minimal mode): every page in `PAGES` is off while it is
/// on, and each page's own switch comes back when it is turned off.
pub const MODS_ONLY: &str = "mods_only";

/// A flag's default, from either list. Unknown ids are off.
/// Switches kept in the same `flags` map that are not experiments and so are not listed in
/// Settings > Experimental: each has its own place in the app. `docked_details` is Browse's
/// "Details beside the grid" in its View options.
pub const VIEW_FLAGS: [Flag; 1] = [Flag {
    id: "docked_details",
    title: "Details beside the grid",
    line: "Opens a mod from Browse in a panel beside the grid instead of over it.",
    default: false,
}];

/// The experimental switches this build offers: `FLAGS` without the Agent unless the build has it.
pub fn experimental_flags() -> impl Iterator<Item = &'static Flag> {
    FLAGS.iter().filter(|f| AGENT || f.id != AGENT_FLAG)
}

fn flag_default(id: &str) -> Option<bool> {
    FLAGS
        .iter()
        .chain(VIEW_FLAGS.iter())
        .chain(PAGES.iter().map(|(_, flag)| flag))
        .find(|f| f.id == id)
        .map(|f| f.default)
        .or_else(|| (id == MODS_ONLY).then_some(false))
}

/// Intervals offered for background work, in minutes. 0 means never.
pub const CATALOG_MINUTES: [u32; 4] = [0, 15, 30, 60];
pub const UPDATE_HOURS: [u32; 3] = [0, 6, 24];
pub const SHARE_MINUTES: [u32; 3] = [15, 30, 60];

/// Custom accents remembered in Appearance.
pub const CUSTOM_ACCENTS_KEEP: usize = 6;

#[derive(Clone, PartialEq)]
pub struct Prefs {
    pub theme: Theme,
    pub accent: Accent,
    pub zoom: f32,
    pub startup: Screen,
    pub on_launch: OnLaunch,
    pub confirm_launch: bool,
    pub land_disabled: bool,
    pub mature: Mature,
    pub browse_sort: BrowseSort,
    /// A Deadlock folder chosen by hand. None means Steam discovery.
    pub game_dir: Option<String>,
    pub allow_stats: bool,
    pub allow_share_codes: bool,
    pub check_updates: bool,
    // --- round 3 ---
    pub language: Lang,
    pub date_format: DateFormat,
    pub start_minimized: bool,
    pub log_level: Level,
    pub discord: bool,
    /// Show which page and the mod count; off shows only "Using Sigil".
    pub discord_details: bool,
    pub discord_mods: bool,
    pub share_matches: bool,
    pub share_minutes: u32,
    pub offline: bool,
    pub update_hours: u32,
    pub catalog_minutes: u32,
    pub proxy: Option<String>,
    pub channel: Channel,
    pub custom_accents: Vec<String>,
    pub transparent_sidebar: bool,
    /// Settings > Appearance > Hardware acceleration (Windows): off draws on the CPU only, from
    /// the next launch (`hardware.rs`).
    pub hardware_acceleration: bool,
    /// Settings > Appearance > Reduce motion: menus, drawers and selections change at once.
    /// Off by default; the system setting reduces motion either way.
    pub reduce_motion: bool,
    /// Settings > Appearance: the Browse billboard steps to its next pick by itself every 7
    /// seconds (never under Reduce motion). On by default.
    pub browse_auto_advance: bool,
    /// The quiet bar under every page: game, catalog, downloads, profile, volume, update.
    pub status_bar: bool,
    pub single_launch: bool,
    /// Flags the user changed; the rest read their default from `FLAGS`.
    pub flags: BTreeMap<String, bool>,
    pub steam_account: Option<u32>,
    /// A GameBanana member id and the name shown for it.
    pub gamebanana: Option<(u64, String)>,
    /// Confirm dialogs the user turned off with "Don't ask again", by id (`installed.delete`).
    pub skip_confirm: std::collections::BTreeSet<String>,
    /// Settings > Appearance > Personalize. All off by default.
    pub personal: crate::ui::personalize::Personal,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: Theme::Graphite,
            accent: Accent::Theme,
            zoom: 1.0,
            startup: Screen::Browse,
            on_launch: OnLaunch::Stay,
            confirm_launch: false,
            land_disabled: true,
            mature: Mature::Blur,
            browse_sort: BrowseSort::Default,
            game_dir: None,
            allow_stats: true,
            allow_share_codes: true,
            check_updates: false,
            language: Lang::System,
            date_format: DateFormat::DayDots,
            start_minimized: false,
            log_level: Level::Info,
            discord: false,
            discord_details: true,
            discord_mods: true,
            share_matches: false,
            share_minutes: 30,
            offline: false,
            update_hours: 0,
            catalog_minutes: 0,
            proxy: None,
            channel: Channel::Stable,
            custom_accents: Vec::new(),
            transparent_sidebar: false,
            hardware_acceleration: crate::hardware::DEFAULT,
            reduce_motion: false,
            browse_auto_advance: true,
            status_bar: true,
            single_launch: false,
            flags: BTreeMap::new(),
            skip_confirm: Default::default(),
            steam_account: None,
            gamebanana: None,
            personal: Default::default(),
        }
    }
}

pub fn path() -> PathBuf {
    crate::data::data_dir().join(FILE_NAME)
}

impl Prefs {
    /// Read the settings file. A missing or unreadable file gives the defaults; a headless run
    /// always does.
    pub fn load() -> Prefs {
        if rune::is_headless() {
            return Prefs::default();
        }
        let prefs = match fs::read_to_string(path()) {
            Ok(text) => Prefs::from_json(&text),
            Err(_) => Prefs::default(),
        };
        set_date_format(prefs.date_format);
        prefs
    }

    pub fn from_json(text: &str) -> Prefs {
        let d = Prefs::default();
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
            return d;
        };
        let s = |key: &str| map.get(key).and_then(Value::as_str);
        let b = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        let n = |key: &str, allowed: &[u32], default: u32| {
            map.get(key)
                .and_then(Value::as_u64)
                .map(|v| v as u32)
                .filter(|v| allowed.contains(v))
                .unwrap_or(default)
        };
        // Files from before the Graphite default always wrote "dark" and a "default" radius,
        // picked or not, so that pair reads as "never chose". Any other theme, a Personalize
        // palette or a chosen radius is a real choice and keeps its exact look.
        let before_block = !map.contains_key("look");
        let mut personal = crate::ui::personalize::Personal::from_value(map.get("personalize"));
        let saved_theme = s("theme").and_then(Theme::from_id);
        let untouched = before_block
            && matches!(saved_theme, None | Some(Theme::Dark))
            && personal.theme.is_none();
        if before_block && !untouched && personal_radius_is_unset(map.get("personalize")) {
            personal.radius = crate::ui::personalize::Radius::Default;
        }
        if untouched && personal.radius == crate::ui::personalize::Radius::Default {
            personal.radius = crate::ui::personalize::Radius::Block;
        }
        Prefs {
            theme: if untouched { d.theme } else { saved_theme.unwrap_or(d.theme) },
            accent: s("accent").map(Accent::from_id).unwrap_or(d.accent),
            zoom: map
                .get("zoom")
                .and_then(Value::as_f64)
                .map(|z| z as f32)
                .filter(|z| ZOOMS.iter().any(|known| (known - z).abs() < 0.001))
                .unwrap_or(d.zoom),
            startup: s("startup")
                .and_then(startup_from_key)
                .unwrap_or(d.startup),
            on_launch: s("on_launch")
                .and_then(|id| OnLaunch::ALL.into_iter().find(|o| o.id() == id))
                .unwrap_or(d.on_launch),
            confirm_launch: b("confirm_launch", d.confirm_launch),
            land_disabled: b("land_disabled", d.land_disabled),
            mature: s("mature")
                .and_then(|id| Mature::ALL.into_iter().find(|m| m.id() == id))
                .unwrap_or(d.mature),
            browse_sort: s("browse_sort")
                .and_then(|id| SORTS.into_iter().find(|sort| sort.query() == id))
                .unwrap_or(d.browse_sort),
            game_dir: s("game_dir")
                .map(str::trim)
                .filter(|dir| !dir.is_empty())
                .map(str::to_string),
            allow_stats: b("allow_stats", d.allow_stats),
            allow_share_codes: b("allow_share_codes", d.allow_share_codes),
            check_updates: b("check_updates", d.check_updates),
            language: s("language").and_then(Lang::from_id).unwrap_or(d.language),
            date_format: s("date_format")
                .and_then(|id| DateFormat::ALL.into_iter().find(|f| f.id() == id))
                .unwrap_or(d.date_format),
            start_minimized: b("start_minimized", d.start_minimized),
            log_level: s("log_level").and_then(Level::from_id).unwrap_or(d.log_level),
            discord: b("discord", d.discord),
            discord_details: b("discord_details", d.discord_details),
            discord_mods: b("discord_mods", d.discord_mods),
            share_matches: b("share_matches", d.share_matches),
            share_minutes: n("share_minutes", &SHARE_MINUTES, d.share_minutes),
            offline: b("offline", d.offline),
            update_hours: n("update_hours", &UPDATE_HOURS, d.update_hours),
            catalog_minutes: n("catalog_minutes", &CATALOG_MINUTES, d.catalog_minutes),
            proxy: s("proxy")
                .map(str::trim)
                .filter(|p| deadlock_core::net::validate_proxy(p).is_ok())
                .map(str::to_string),
            channel: s("channel")
                .and_then(|id| Channel::ALL.into_iter().find(|c| c.id() == id))
                .unwrap_or(d.channel),
            custom_accents: map
                .get("custom_accents")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .filter(|hex| parse_hex(hex).is_some())
                        .take(CUSTOM_ACCENTS_KEEP)
                        .map(str::to_lowercase)
                        .collect()
                })
                .unwrap_or_default(),
            transparent_sidebar: b("transparent_sidebar", d.transparent_sidebar),
            hardware_acceleration: b("hardware_acceleration", d.hardware_acceleration),
            reduce_motion: b("reduce_motion", d.reduce_motion),
            browse_auto_advance: b("browse_auto_advance", d.browse_auto_advance),
            status_bar: b("status_bar", d.status_bar),
            single_launch: b("single_launch", d.single_launch),
            flags: map
                .get("flags")
                .and_then(Value::as_object)
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| flag_default(k).is_some())
                        .filter_map(|(k, v)| Some((k.clone(), v.as_bool()?)))
                        .collect()
                })
                .unwrap_or_default(),
            steam_account: map
                .get("steam_account")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0),
            gamebanana: map.get("gamebanana").and_then(|g| {
                let id = g.get("id")?.as_u64().filter(|id| *id > 0)?;
                let name = g.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                Some((id, name))
            }),
            skip_confirm: map
                .get("skip_confirm")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
            personal,
        }
    }

    /// The page Sigil opens on: the stored choice while the sidebar shows it, else Browse. The
    /// stored value is not rewritten, so a page switched back on is the start page again.
    pub fn opening_page(&self) -> Screen {
        if crate::app::sidebar_pages(self).contains(&self.startup) {
            self.startup
        } else {
            Screen::Browse
        }
    }

    /// A flag: the user's choice, else its default.
    pub fn flag(&self, id: &str) -> bool {
        // A build without the agent reads its switch as off; the stored value stays for a build
        // that has it.
        if id == AGENT_FLAG && !AGENT {
            return false;
        }
        self.flags
            .get(id)
            .copied()
            .unwrap_or_else(|| flag_default(id).unwrap_or(false))
    }

    /// Is this page shown? Pages outside `PAGES` always are, and "Mods only" hides every
    /// page that is in it.
    pub fn page_on(&self, screen: Screen) -> bool {
        // The Agent and the pages under it exist only while its switch is on (and only in a
        // build that has it).
        if crate::app::under_agent(screen) {
            return self.flag(AGENT_FLAG);
        }
        // Custom maps is an experimental flag, not a page switch: off until turned on.
        if screen == Screen::Maps {
            return self.flag("custom_maps") && !self.flag(MODS_ONLY);
        }
        match PAGES.iter().find(|(page, _)| *page == screen) {
            Some((_, flag)) => self.flag(flag.id) && !self.flag(MODS_ONLY),
            None => true,
        }
    }

    /// Flip a flag, dropping the entry when it matches the default.
    pub fn toggle_flag(&mut self, id: &str) {
        if id == AGENT_FLAG && !AGENT {
            return;
        }
        let on = !self.flag(id);
        let default = flag_default(id).unwrap_or(false);
        if on == default {
            self.flags.remove(id);
        } else {
            self.flags.insert(id.to_string(), on);
        }
    }

    /// Remember a custom accent, newest first.
    pub fn remember_accent(&mut self, hex: &str) {
        let hex = hex.to_lowercase();
        self.custom_accents.retain(|h| *h != hex);
        self.custom_accents.insert(0, hex);
        self.custom_accents.truncate(CUSTOM_ACCENTS_KEEP);
    }

    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        map.insert("schema".into(), json!(1));
        // Marks a file written since Graphite became the default: its theme is a real choice.
        map.insert("look".into(), json!(2));
        map.insert("theme".into(), json!(self.theme.id()));
        map.insert("accent".into(), json!(self.accent.id()));
        map.insert("zoom".into(), json!(self.zoom));
        map.insert("startup".into(), json!(self.startup.key()));
        map.insert("on_launch".into(), json!(self.on_launch.id()));
        map.insert("confirm_launch".into(), json!(self.confirm_launch));
        map.insert("land_disabled".into(), json!(self.land_disabled));
        map.insert("mature".into(), json!(self.mature.id()));
        map.insert("browse_sort".into(), json!(self.browse_sort.query()));
        map.insert(
            "game_dir".into(),
            self.game_dir.as_ref().map_or(Value::Null, |dir| json!(dir)),
        );
        map.insert("allow_stats".into(), json!(self.allow_stats));
        map.insert("allow_share_codes".into(), json!(self.allow_share_codes));
        map.insert("check_updates".into(), json!(self.check_updates));
        map.insert("language".into(), json!(self.language.id()));
        map.insert("date_format".into(), json!(self.date_format.id()));
        map.insert("start_minimized".into(), json!(self.start_minimized));
        map.insert("log_level".into(), json!(self.log_level.id()));
        map.insert("discord".into(), json!(self.discord));
        map.insert("discord_details".into(), json!(self.discord_details));
        map.insert("discord_mods".into(), json!(self.discord_mods));
        map.insert("share_matches".into(), json!(self.share_matches));
        map.insert("share_minutes".into(), json!(self.share_minutes));
        map.insert("offline".into(), json!(self.offline));
        map.insert("update_hours".into(), json!(self.update_hours));
        map.insert("catalog_minutes".into(), json!(self.catalog_minutes));
        map.insert(
            "proxy".into(),
            self.proxy.as_ref().map_or(Value::Null, |p| json!(p)),
        );
        map.insert("channel".into(), json!(self.channel.id()));
        map.insert("custom_accents".into(), json!(self.custom_accents));
        map.insert("transparent_sidebar".into(), json!(self.transparent_sidebar));
        map.insert("hardware_acceleration".into(), json!(self.hardware_acceleration));
        map.insert("reduce_motion".into(), json!(self.reduce_motion));
        map.insert("browse_auto_advance".into(), json!(self.browse_auto_advance));
        map.insert("status_bar".into(), json!(self.status_bar));
        map.insert("single_launch".into(), json!(self.single_launch));
        map.insert("flags".into(), json!(self.flags));
        map.insert("skip_confirm".into(), json!(self.skip_confirm));
        map.insert("personalize".into(), self.personal.to_value());
        map.insert(
            "steam_account".into(),
            self.steam_account.map_or(Value::Null, |a| json!(a)),
        );
        map.insert(
            "gamebanana".into(),
            self.gamebanana
                .as_ref()
                .map_or(Value::Null, |(id, name)| json!({"id": id, "name": name})),
        );
        serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default() + "\n"
    }

    /// Write the file through a temporary sibling. Headless runs write nothing.
    pub fn save(&self) -> Result<(), String> {
        // Unit tests drive real handlers (a settings switch saves): never into the player's own
        // settings unless a test points the data folder somewhere.
        if rune::is_headless() || (cfg!(test) && deadlock_core::app_data::override_dir().is_none()) {
            return Ok(());
        }
        let path = path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        let partial = path.with_file_name(format!("{FILE_NAME}.{}.tmp", std::process::id()));
        fs::write(&partial, self.to_json()).map_err(|error| error.to_string())?;
        fs::rename(&partial, &path).map_err(|error| {
            let _ = fs::remove_file(&partial);
            error.to_string()
        })
    }

    pub fn accent_color(&self) -> Option<Color> {
        self.accent.color()
    }
}

/// Change a setting, save, and report a failed write in the status line.
pub fn edit(st: &mut crate::app::State, change: impl FnOnce(&mut Prefs)) {
    change(&mut st.prefs);
    set_date_format(st.prefs.date_format);
    if let Err(error) = st.prefs.save() {
        st.settings.message = Some((
            crate::screens::settings::i18n::fill(
                crate::screens::settings::i18n::tr("Settings could not be saved: {}"),
                &[&error],
            ),
            true,
        ));
    }
}

/// Point Steam discovery at the chosen folder for every reader, the core's included. Called
/// at startup and when the folder changes in Settings.
pub fn apply_game_dir(prefs: &Prefs) {
    // The core reads `DAH_GAME_DIR` when no folder is passed. A user's own environment wins
    // at startup; after that Settings owns the value.
    if game_dir_from_env() {
        return;
    }
    // SAFETY: Rust's own env access is locked; the core reads this variable only through
    // `std::env::var_os`, at the start of a discovery.
    unsafe {
        match &prefs.game_dir {
            Some(dir) => std::env::set_var("DAH_GAME_DIR", dir),
            None => std::env::remove_var("DAH_GAME_DIR"),
        }
    }
}

/// True when `DAH_GAME_DIR` came from the user's environment, so Settings cannot change it.
pub fn game_dir_from_env() -> bool {
    static FROM_USER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FROM_USER.get_or_init(|| std::env::var_os("DAH_GAME_DIR").is_some())
}

/// A file from before the Block radius with no radius saved had the old rounded default.
fn personal_radius_is_unset(personal: Option<&Value>) -> bool {
    personal.and_then(|p| p.get("radius")).and_then(Value::as_str).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_default_to_day_month_year_and_a_stored_choice_stays() {
        assert_eq!(Prefs::default().date_format, DateFormat::DayDots);
        assert_eq!(Prefs::from_json("{}").date_format, DateFormat::DayDots);
        assert_eq!(DateFormat::DayDots.format(2026, 11, 7), "07.11.2026");
        for format in DateFormat::ALL {
            let text = format!(r#"{{"schema":1,"date_format":"{}"}}"#, format.id());
            assert_eq!(Prefs::from_json(&text).date_format, format, "{text}");
            let mut p = Prefs::default();
            p.date_format = format;
            assert_eq!(Prefs::from_json(&p.to_json()).date_format, format);
        }
    }

    #[test]
    fn stamps_read_in_the_chosen_format() {
        assert_eq!(show_stamp_date("2026-11-07T10:22:01.000Z").as_deref(), Some("07.11.2026"));
        assert_eq!(show_stamp("2026-11-07T10:22:01Z"), "07.11.2026 10:22");
        assert_eq!(show_stamp("2026-11-07"), "07.11.2026");
        assert_eq!(show_stamp("not a date"), "not a date");
        assert_eq!(show_unix_date(1_762_473_600), "07.11.2025");
        assert_eq!(DateFormat::Iso.format(2026, 11, 7), "2026-11-07");
    }

    /// A settings file the 0.1.42 app wrote into `DeadlockAgentHarness` loads from `Sigil` after
    /// the start's move (`deadlock_core::app_data`), with the player's choices intact.
    #[test]
    fn settings_survive_the_data_folder_move() {
        use deadlock_core::app_data;
        let base = std::env::temp_dir().join(format!("sigil-prefs-move-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let old = base.join("DeadlockAgentHarness");
        fs::create_dir_all(&old).unwrap();
        let mut prefs = Prefs::default();
        prefs.theme = Theme::Midnight;
        prefs.startup = Screen::Installed;
        fs::write(old.join(FILE_NAME), prefs.to_json()).unwrap();
        let (data, outcome) = app_data::migrate_in(&base, &|_| true);
        assert!(matches!(outcome, app_data::Outcome::Moved { .. }));
        let loaded = Prefs::from_json(&fs::read_to_string(data.join(FILE_NAME)).unwrap());
        assert!(loaded.theme == Theme::Midnight);
        assert!(loaded.startup == Screen::Installed);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn the_graphite_default_keeps_real_choices() {
        use crate::ui::personalize::{Radius, ThemePick};
        // Fresh, and an old file that only ever held the old defaults.
        for text in ["{}", r#"{"schema":1,"theme":"dark","personalize":{"radius":"default"}}"#] {
            let p = Prefs::from_json(text);
            assert!(p.theme == Theme::Graphite, "{text}");
            assert_eq!(p.personal.radius, Radius::Block, "{text}");
        }
        // An old file with a real theme keeps it and its rounded corners.
        let p = Prefs::from_json(r#"{"schema":1,"theme":"midnight","personalize":{"radius":"default"}}"#);
        assert!(p.theme == Theme::Midnight);
        assert_eq!(p.personal.radius, Radius::Default);
        // A Personalize preset over the old Dark base is a choice too.
        let p = Prefs::from_json(r#"{"schema":1,"theme":"dark","personalize":{"theme":"preset:brass","radius":"default"}}"#);
        assert!(p.theme == Theme::Dark);
        assert_eq!(p.personal.theme, Some(ThemePick::Preset("brass")));
        assert_eq!(p.personal.radius, Radius::Default);
        // An old chosen radius survives the move.
        let p = Prefs::from_json(r#"{"schema":1,"theme":"dark","personalize":{"radius":"round"}}"#);
        assert!(p.theme == Theme::Graphite);
        assert_eq!(p.personal.radius, Radius::Round);
        // After the move, picking Dark and Soft sticks.
        let mut p = Prefs::default();
        p.theme = Theme::Dark;
        p.personal.radius = Radius::Default;
        let back = Prefs::from_json(&p.to_json());
        assert!(back.theme == Theme::Dark);
        assert_eq!(back.personal.radius, Radius::Default);
    }

    #[test]
    fn round_trips_every_field() {
        let prefs = Prefs {
            theme: Theme::Light,
            accent: Accent::Custom("#12ab34".into()),
            zoom: 1.1,
            startup: Screen::Locker,
            on_launch: OnLaunch::Minimize,
            confirm_launch: true,
            land_disabled: false,
            mature: Mature::Hide,
            browse_sort: BrowseSort::Updated,
            game_dir: Some("/games/Deadlock".into()),
            allow_stats: false,
            allow_share_codes: false,
            check_updates: true,
            language: Lang::German,
            date_format: DateFormat::DayFirst,
            start_minimized: true,
            log_level: Level::Debug,
            discord: true,
            discord_details: false,
            discord_mods: false,
            share_matches: true,
            share_minutes: 60,
            offline: true,
            update_hours: 24,
            catalog_minutes: 30,
            proxy: Some("http://127.0.0.1:7890".into()),
            channel: Channel::Preview,
            custom_accents: vec!["#12ab34".into(), "#abc".into()],
            transparent_sidebar: true,
            hardware_acceleration: !crate::hardware::DEFAULT,
            reduce_motion: true,
            browse_auto_advance: false,
            status_bar: false,
            single_launch: true,
            flags: [("servers".to_string(), false), ("docked_details".to_string(), true)]
                .into_iter()
                .collect(),
            steam_account: Some(81_220_464),
            gamebanana: Some((6_020_123, "Kalo".into())),
            skip_confirm: ["installed.delete".to_string()].into_iter().collect(),
            personal: {
                let mut p = crate::ui::personalize::Personal::default();
                p.theme = Some(crate::ui::personalize::ThemePick::Preset("brass"));
                p.glow.on = true;
                p.radius = crate::ui::personalize::Radius::Sharp;
                p
            },
        };
        assert!(Prefs::from_json(&prefs.to_json()) == prefs);
    }

    /// A settings.json from before Personalize loads every old field and leaves
    /// Personalize off.
    #[test]
    fn old_settings_without_personalize_still_load() {
        let old = r##"{"schema":1,"theme":"midnight","accent":"#12ab34","zoom":1.1,"transparent_sidebar":true,"custom_accents":["#12ab34"]}"##;
        let p = Prefs::from_json(old);
        assert!(p.theme == Theme::Midnight);
        assert_eq!(p.accent, Accent::Custom("#12ab34".into()));
        assert!(p.transparent_sidebar);
        // A real old theme keeps the rounded corners it was seen with.
        let expected = crate::ui::personalize::Personal {
            radius: crate::ui::personalize::Radius::Default,
            ..Default::default()
        };
        assert_eq!(p.personal, expected);
        assert!(p.personal.palette().is_none());
        let back = Prefs::from_json(&p.to_json());
        assert!(back == p);
    }

    #[test]
    fn bad_values_fall_back_one_by_one() {
        let prefs = Prefs::from_json(
            r#"{"theme":"plaid","accent":"violet","zoom":3,"startup":"settings","mature":7,"allow_stats":false}"#,
        );
        let d = Prefs::default();
        assert!(prefs.theme == d.theme);
        assert_eq!(prefs.accent, Accent::Preset("violet"));
        assert_eq!(prefs.zoom, d.zoom);
        assert!(prefs.startup == d.startup);
        assert_eq!(prefs.mature, d.mature);
        assert!(!prefs.allow_stats);
        assert!(Prefs::from_json("not json") == d);
    }

    #[test]
    fn flags_read_defaults_and_store_only_changes() {
        let mut p = Prefs::default();
        assert!(p.flag("servers"));
        assert!(!p.flag("docked_details"));
        assert!(!p.flag("no-such-flag"));
        p.toggle_flag("servers");
        assert!(!p.flag("servers"));
        p.toggle_flag("servers");
        assert!(p.flags.is_empty(), "back at the default, nothing is stored");
        let back = Prefs::from_json(r#"{"flags":{"servers":false,"bogus":true},"share_minutes":7}"#);
        assert!(!back.flag("servers"));
        assert!(!back.flags.contains_key("bogus"));
        assert_eq!(back.share_minutes, 30);
    }

    #[test]
    fn pages_turn_off_one_by_one_and_mods_only_hides_them_all() {
        let mut p = Prefs::default();
        assert!(PAGES.iter().all(|(screen, _)| p.page_on(*screen)), "every page is on by default");
        assert!(p.page_on(Screen::Browse) && p.page_on(Screen::Installed));
        p.toggle_flag("page_stats");
        assert!(!p.page_on(Screen::Stats));
        assert!(p.page_on(Screen::Locker));
        p.toggle_flag(MODS_ONLY);
        assert!(PAGES.iter().all(|(screen, _)| !p.page_on(*screen)));
        assert!(p.page_on(Screen::Installed), "mod management stays");
        p.toggle_flag(MODS_ONLY);
        assert!(p.page_on(Screen::Locker), "each page's own switch comes back");
        assert!(!p.page_on(Screen::Stats));
        // The Servers switch saved before the Pages section still reads.
        let old = Prefs::from_json(r#"{"flags":{"servers":false,"page_locker":false,"mods_only":true}}"#);
        assert!(!old.flag("servers") && !old.flag("page_locker") && old.flag(MODS_ONLY));
    }

    #[test]
    fn custom_accents_keep_the_newest_few() {
        let mut p = Prefs::default();
        for i in 0..8 {
            p.remember_accent(&format!("#00000{i}"));
        }
        p.remember_accent("#000003");
        assert_eq!(p.custom_accents.len(), CUSTOM_ACCENTS_KEEP);
        assert_eq!(p.custom_accents[0], "#000003");
    }

    #[test]
    fn hex_accepts_short_and_long_forms() {
        assert!(parse_hex("#abc").is_some());
        assert!(parse_hex("a1b2c3").is_some());
        assert!(parse_hex("#ggg").is_none());
        assert!(parse_hex("#12345").is_none());
    }
}
