//! Settings added in the parity round, kept in their own file (`<data>/options.json`) so they
//! never fight `settings.json` for a field. The same rules apply: each field reads on its own
//! with a fallback, writes go through a temporary sibling, and headless runs never touch disk.
//!
//! Other screens read a value with a getter, `options::get().locker_expanded`, which clones a
//! small struct; a handler changes one with `options::edit(st, |o| ..)`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::{Map, Value, json};

use crate::app::State;

pub const FILE_NAME: &str = "options.json";

/// Blur strengths offered for NSFW previews, in points.
pub const BLURS: [u32; 4] = [8, 18, 28, 40];
/// Download parts offered. 1 means one connection.
pub const PARTS: [u32; 3] = [1, 4, 8];
/// Mature reveals remembered at most.
pub const REVEALS_KEEP: usize = 2000;

/// One line pair Discord shows: the top line (details) and the bottom line (state).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Lines {
    pub top: String,
    pub bottom: String,
}

/// A third-party mod source: a JSON index at an https address the user added.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Source {
    pub url: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Options {
    // Game
    /// A Steam folder chosen by hand; discovery reads its libraries first.
    pub steam_root: Option<String>,
    pub block_while_running: bool,
    pub remove_paths_on_close: bool,
    pub crash_check: bool,
    pub auto_reapply: bool,
    pub fix_before_launch: bool,
    /// Add `-condebug` to the launch so the game writes console.log.
    pub condebug: bool,
    // Editor and dialogs
    /// An app or program to open text files with. None is the system default.
    pub editor: Option<String>,
    pub last_dialog_dir: Option<String>,
    // Downloads
    /// `auto` or a server id from `downloads::SERVERS`.
    pub download_server: String,
    pub mirror_first: bool,
    /// The fastest server of the last speed test on this computer.
    pub fastest_server: Option<String>,
    pub parts: u32,
    /// Where the download cache was moved to. None is inside the workspace.
    pub cache_dir: Option<String>,
    pub sources: Vec<Source>,
    // Network
    pub proxy_user: Option<String>,
    pub proxy_bypass: Vec<String>,
    // Window
    pub tray_minimize: bool,
    pub tray_close: bool,
    // NSFW content
    pub blur: u32,
    pub remember_reveals: bool,
    pub revealed: Vec<u64>,
    pub likely_nsfw: bool,
    pub reveal_on_hover: bool,
    pub always_tag: bool,
    // Page preferences
    pub locker_expanded: bool,
    pub installed_open_page: bool,
    /// Installed's sort, by its key (`installed::Sort::key`): kept between runs.
    pub installed_sort: String,
    pub confirm_profile_update: bool,
    pub prompt_requirements: bool,
    pub ignore_overlap_default: bool,
    pub switch_variants: bool,
    pub match_update_files: bool,
    pub swap_install_replace: bool,
    pub skip_replace_confirm: bool,
    pub allow_scripts: bool,
    /// Ask before a second mod for one hero is turned on (Deadlock Mod Manager's warning).
    pub hero_conflict_warning: bool,
    /// Ask at a modded launch whether to turn enabled map mods off (Deadlock Mod Manager 2.4).
    pub map_mod_warning: bool,
    /// Repair enabled mods made for an older game build before each modded launch.
    pub repair_before_launch: bool,
    // Integrations
    pub forge: bool,
    /// Read console.log for the match phase and hero.
    pub presence_game: bool,
    /// Leave the party size out of Sigil's own presence lines (DMM's switch).
    pub presence_hide_party: bool,
    /// Stats shows the match being played, read from console.log (local only).
    pub live_view: bool,
    /// The player turned on the live bridge HUD mod (off by default).
    pub live_bridge: bool,
    /// Look up the live match's players on deadlock-api (off by default).
    pub live_lookup: bool,
    /// Rebuild the live bridge by itself after a game update, without asking (off by default).
    pub live_bridge_auto: bool,
    /// The game build the live bridge was last found current for, or asked about.
    pub live_bridge_build: String,
    /// Discord lines per phase id; missing phases use the defaults.
    pub presence_lines: BTreeMap<String, Lines>,
    /// Discord lines per hero name, used in a match.
    pub hero_lines: BTreeMap<String, Lines>,
    // Experimental
    pub imprint: bool,
    // First run
    pub onboarded: bool,
    pub tips: bool,
    pub tips_seen: Vec<String>,
    pub unknown_alert_off: bool,
    // Wave 4 (shell, Settings, Browse)
    /// The sidebar shows icons only (Grimoire keeps it across launches; DMM's Cmd+B).
    pub sidebar_collapsed: bool,
    /// Pages whose "New" mark was cleared by opening them once (DMM).
    pub seen_pages: Vec<String>,
    /// Show update marks on mod cards and details (DMM, on by default).
    pub update_badges: bool,
    /// A vanilla launch writes no launch options to Steam (DMM's Vanilla Mode).
    pub vanilla_no_args: bool,
    /// Contextual feature tips (DMM): on by default, one per session.
    pub feature_hints: bool,
    /// Tips that were answered (Got it, used, or ignored twice).
    pub hints_done: Vec<String>,
    /// Small counters the tips read: Settings visits, ignores per tip.
    pub counters: BTreeMap<String, u64>,
    /// Seasonal themes by date (DMM), off by default.
    pub seasonal: bool,
    /// `<season>:<year>` turned off until it comes round again.
    pub seasonal_dismissed: Option<String>,
    /// Easter eggs (Grimoire), off by default.
    pub easter_eggs: bool,
    /// The support report carries the whole log, not only its tail (Grimoire).
    pub full_log: bool,
    /// What Copy and Ask Sigil read: `combined`, `sigil` or `dumps` (DMM's Log Source).
    pub log_source: String,
    /// The experimental tour was shown once.
    pub tour_seen: bool,
    /// Crosshair: write it to the game's hero settings (DMM). The write is the owner's step.
    pub crosshair_hero_settings: bool,
    /// Themed preset: `none`, `grid` (Blueprint), `crest` (the Sigil mark large behind the
    /// pages) or `terminal` (the grid and monospace type), off by default.
    pub themed_art: String,
    /// Seasonal full-screen effects (snow, sparks), off by default; needs seasonal themes on.
    pub seasonal_effects: bool,
    /// Servers > Regions: presets and the relay locations Sigil last saw blocked. Empty until
    /// the player presses Apply (`deadlock_core::relays`).
    pub relay_regions: deadlock_core::relays::Choices,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            steam_root: None,
            block_while_running: false,
            remove_paths_on_close: false,
            crash_check: true,
            auto_reapply: false,
            fix_before_launch: false,
            condebug: false,
            editor: None,
            last_dialog_dir: None,
            download_server: "auto".into(),
            mirror_first: false,
            fastest_server: None,
            parts: 1,
            cache_dir: None,
            sources: Vec::new(),
            proxy_user: None,
            proxy_bypass: Vec::new(),
            tray_minimize: false,
            tray_close: false,
            blur: 18,
            remember_reveals: false,
            revealed: Vec::new(),
            likely_nsfw: false,
            reveal_on_hover: false,
            always_tag: false,
            locker_expanded: false,
            installed_open_page: false,
            installed_sort: String::new(),
            confirm_profile_update: true,
            prompt_requirements: true,
            ignore_overlap_default: false,
            switch_variants: false,
            match_update_files: true,
            swap_install_replace: false,
            skip_replace_confirm: false,
            allow_scripts: false,
            hero_conflict_warning: true,
            map_mod_warning: true,
            repair_before_launch: false,
            forge: false,
            presence_game: false,
            presence_hide_party: false,
            live_view: true,
            live_bridge: false,
            live_lookup: false,
            live_bridge_auto: false,
            live_bridge_build: String::new(),
            presence_lines: BTreeMap::new(),
            hero_lines: BTreeMap::new(),
            imprint: false,
            onboarded: false,
            tips: true,
            tips_seen: Vec::new(),
            unknown_alert_off: false,
            sidebar_collapsed: false,
            seen_pages: Vec::new(),
            update_badges: true,
            vanilla_no_args: false,
            feature_hints: true,
            hints_done: Vec::new(),
            counters: BTreeMap::new(),
            seasonal: false,
            seasonal_dismissed: None,
            easter_eggs: false,
            full_log: false,
            log_source: "combined".into(),
            tour_seen: false,
            crosshair_hero_settings: false,
            themed_art: "none".into(),
            seasonal_effects: false,
            relay_regions: deadlock_core::relays::Choices::default(),
        }
    }
}

fn lines_map(value: Option<&Value>) -> BTreeMap<String, Lines> {
    value
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !k.trim().is_empty())
                .filter_map(|(k, v)| {
                    let top = v.get("top").and_then(Value::as_str)?.to_string();
                    let bottom = v.get("bottom").and_then(Value::as_str).unwrap_or("").to_string();
                    Some((k.clone(), Lines { top, bottom }))
                })
                .take(256)
                .collect()
        })
        .unwrap_or_default()
}

fn lines_json(map: &BTreeMap<String, Lines>) -> Value {
    let mut out = Map::new();
    for (k, v) in map {
        out.insert(k.clone(), json!({"top": v.top, "bottom": v.bottom}));
    }
    Value::Object(out)
}

impl Options {
    pub fn from_json(text: &str) -> Options {
        let d = Options::default();
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
            return d;
        };
        let s = |key: &str| {
            map.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let b = |key: &str, default: bool| map.get(key).and_then(Value::as_bool).unwrap_or(default);
        let n = |key: &str, allowed: &[u32], default: u32| {
            map.get(key)
                .and_then(Value::as_u64)
                .map(|v| v as u32)
                .filter(|v| allowed.contains(v))
                .unwrap_or(default)
        };
        let strings = |key: &str| -> Vec<String> {
            map.get(key)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .take(256)
                        .collect()
                })
                .unwrap_or_default()
        };
        Options {
            steam_root: s("steam_root"),
            block_while_running: b("block_while_running", d.block_while_running),
            remove_paths_on_close: b("remove_paths_on_close", d.remove_paths_on_close),
            crash_check: b("crash_check", d.crash_check),
            auto_reapply: b("auto_reapply", d.auto_reapply),
            fix_before_launch: b("fix_before_launch", d.fix_before_launch),
            condebug: b("condebug", d.condebug),
            editor: s("editor"),
            last_dialog_dir: s("last_dialog_dir"),
            download_server: s("download_server")
                .filter(|id| id == "auto" || deadlock_core::file_servers::valid_name(id))
                .unwrap_or(d.download_server),
            mirror_first: b("mirror_first", d.mirror_first),
            fastest_server: s("fastest_server").filter(|id| deadlock_core::file_servers::valid_name(id)),
            parts: n("parts", &PARTS, d.parts),
            cache_dir: s("cache_dir"),
            sources: map
                .get("sources")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|v| {
                            let url = v.get("url")?.as_str()?.trim().to_string();
                            super::sources::validate_url(&url).ok()?;
                            Some(Source {
                                name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                                enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                                url,
                            })
                        })
                        .take(32)
                        .collect()
                })
                .unwrap_or_default(),
            proxy_user: s("proxy_user"),
            proxy_bypass: strings("proxy_bypass"),
            tray_minimize: b("tray_minimize", d.tray_minimize),
            tray_close: b("tray_close", d.tray_close),
            blur: n("blur", &BLURS, d.blur),
            remember_reveals: b("remember_reveals", d.remember_reveals),
            revealed: map
                .get("revealed")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_u64).take(REVEALS_KEEP).collect())
                .unwrap_or_default(),
            likely_nsfw: b("likely_nsfw", d.likely_nsfw),
            reveal_on_hover: b("reveal_on_hover", d.reveal_on_hover),
            always_tag: b("always_tag", d.always_tag),
            locker_expanded: b("locker_expanded", d.locker_expanded),
            installed_open_page: b("installed_open_page", d.installed_open_page),
            installed_sort: s("installed_sort").unwrap_or_default(),
            confirm_profile_update: b("confirm_profile_update", d.confirm_profile_update),
            prompt_requirements: b("prompt_requirements", d.prompt_requirements),
            ignore_overlap_default: b("ignore_overlap_default", d.ignore_overlap_default),
            switch_variants: b("switch_variants", d.switch_variants),
            match_update_files: b("match_update_files", d.match_update_files),
            swap_install_replace: b("swap_install_replace", d.swap_install_replace),
            skip_replace_confirm: b("skip_replace_confirm", d.skip_replace_confirm),
            allow_scripts: b("allow_scripts", d.allow_scripts),
            hero_conflict_warning: b("hero_conflict_warning", d.hero_conflict_warning),
            map_mod_warning: b("map_mod_warning", d.map_mod_warning),
            repair_before_launch: b("repair_before_launch", d.repair_before_launch),
            forge: b("forge", d.forge),
            presence_game: b("presence_game", d.presence_game),
            presence_hide_party: b("presence_hide_party", d.presence_hide_party),
            live_view: b("live_view", d.live_view),
            live_bridge: b("live_bridge", d.live_bridge),
            live_lookup: b("live_lookup", d.live_lookup),
            live_bridge_auto: b("live_bridge_auto", d.live_bridge_auto),
            live_bridge_build: s("live_bridge_build").unwrap_or_default(),
            presence_lines: lines_map(map.get("presence_lines")),
            hero_lines: lines_map(map.get("hero_lines")),
            imprint: b("imprint", d.imprint),
            onboarded: b("onboarded", d.onboarded),
            tips: b("tips", d.tips),
            tips_seen: strings("tips_seen"),
            unknown_alert_off: b("unknown_alert_off", d.unknown_alert_off),
            sidebar_collapsed: b("sidebar_collapsed", d.sidebar_collapsed),
            seen_pages: strings("seen_pages"),
            update_badges: b("update_badges", d.update_badges),
            vanilla_no_args: b("vanilla_no_args", d.vanilla_no_args),
            feature_hints: b("feature_hints", d.feature_hints),
            hints_done: strings("hints_done"),
            counters: map
                .get("counters")
                .and_then(Value::as_object)
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                        .take(64)
                        .collect()
                })
                .unwrap_or_default(),
            seasonal: b("seasonal", d.seasonal),
            seasonal_dismissed: s("seasonal_dismissed"),
            easter_eggs: b("easter_eggs", d.easter_eggs),
            full_log: b("full_log", d.full_log),
            log_source: s("log_source")
                .filter(|v| ["combined", "sigil", "dumps"].contains(&v.as_str()))
                .unwrap_or(d.log_source),
            tour_seen: b("tour_seen", d.tour_seen),
            crosshair_hero_settings: b("crosshair_hero_settings", d.crosshair_hero_settings),
            seasonal_effects: b("seasonal_effects", d.seasonal_effects),
            themed_art: s("themed_art").filter(|v| ["none", "grid", "crest", "terminal"].contains(&v.as_str())).unwrap_or(d.themed_art),
            relay_regions: map
                .get("relay_regions")
                .cloned()
                .and_then(|v| serde_json::from_value::<deadlock_core::relays::Choices>(v).ok())
                .map(deadlock_core::relays::Choices::sanitized)
                .unwrap_or_default(),
        }
    }

    pub fn to_json(&self) -> String {
        let opt = |v: &Option<String>| v.as_ref().map_or(Value::Null, |s| json!(s));
        let mut m = Map::new();
        m.insert("schema".into(), json!(1));
        m.insert("steam_root".into(), opt(&self.steam_root));
        m.insert("block_while_running".into(), json!(self.block_while_running));
        m.insert("remove_paths_on_close".into(), json!(self.remove_paths_on_close));
        m.insert("crash_check".into(), json!(self.crash_check));
        m.insert("auto_reapply".into(), json!(self.auto_reapply));
        m.insert("fix_before_launch".into(), json!(self.fix_before_launch));
        m.insert("condebug".into(), json!(self.condebug));
        m.insert("editor".into(), opt(&self.editor));
        m.insert("last_dialog_dir".into(), opt(&self.last_dialog_dir));
        m.insert("download_server".into(), json!(self.download_server));
        m.insert("mirror_first".into(), json!(self.mirror_first));
        m.insert("fastest_server".into(), opt(&self.fastest_server));
        m.insert("parts".into(), json!(self.parts));
        m.insert("cache_dir".into(), opt(&self.cache_dir));
        m.insert(
            "sources".into(),
            Value::Array(
                self.sources
                    .iter()
                    .map(|s| json!({"url": s.url, "name": s.name, "enabled": s.enabled}))
                    .collect(),
            ),
        );
        m.insert("proxy_user".into(), opt(&self.proxy_user));
        m.insert("proxy_bypass".into(), json!(self.proxy_bypass));
        m.insert("tray_minimize".into(), json!(self.tray_minimize));
        m.insert("tray_close".into(), json!(self.tray_close));
        m.insert("blur".into(), json!(self.blur));
        m.insert("remember_reveals".into(), json!(self.remember_reveals));
        m.insert("revealed".into(), json!(self.revealed));
        m.insert("likely_nsfw".into(), json!(self.likely_nsfw));
        m.insert("reveal_on_hover".into(), json!(self.reveal_on_hover));
        m.insert("always_tag".into(), json!(self.always_tag));
        m.insert("locker_expanded".into(), json!(self.locker_expanded));
        m.insert("installed_open_page".into(), json!(self.installed_open_page));
        m.insert("installed_sort".into(), json!(self.installed_sort));
        m.insert("confirm_profile_update".into(), json!(self.confirm_profile_update));
        m.insert("prompt_requirements".into(), json!(self.prompt_requirements));
        m.insert("ignore_overlap_default".into(), json!(self.ignore_overlap_default));
        m.insert("switch_variants".into(), json!(self.switch_variants));
        m.insert("match_update_files".into(), json!(self.match_update_files));
        m.insert("swap_install_replace".into(), json!(self.swap_install_replace));
        m.insert("skip_replace_confirm".into(), json!(self.skip_replace_confirm));
        m.insert("allow_scripts".into(), json!(self.allow_scripts));
        m.insert("hero_conflict_warning".into(), json!(self.hero_conflict_warning));
        m.insert("map_mod_warning".into(), json!(self.map_mod_warning));
        m.insert("repair_before_launch".into(), json!(self.repair_before_launch));
        m.insert("forge".into(), json!(self.forge));
        m.insert("presence_game".into(), json!(self.presence_game));
        m.insert("presence_hide_party".into(), json!(self.presence_hide_party));
        m.insert("live_view".into(), json!(self.live_view));
        m.insert("live_bridge".into(), json!(self.live_bridge));
        m.insert("live_lookup".into(), json!(self.live_lookup));
        m.insert("live_bridge_auto".into(), json!(self.live_bridge_auto));
        m.insert("live_bridge_build".into(), json!(self.live_bridge_build));
        m.insert("presence_lines".into(), lines_json(&self.presence_lines));
        m.insert("hero_lines".into(), lines_json(&self.hero_lines));
        m.insert("imprint".into(), json!(self.imprint));
        m.insert("onboarded".into(), json!(self.onboarded));
        m.insert("tips".into(), json!(self.tips));
        m.insert("tips_seen".into(), json!(self.tips_seen));
        m.insert("unknown_alert_off".into(), json!(self.unknown_alert_off));
        m.insert("sidebar_collapsed".into(), json!(self.sidebar_collapsed));
        m.insert("seen_pages".into(), json!(self.seen_pages));
        m.insert("update_badges".into(), json!(self.update_badges));
        m.insert("vanilla_no_args".into(), json!(self.vanilla_no_args));
        m.insert("feature_hints".into(), json!(self.feature_hints));
        m.insert("hints_done".into(), json!(self.hints_done));
        m.insert("counters".into(), json!(self.counters));
        m.insert("seasonal".into(), json!(self.seasonal));
        m.insert("seasonal_dismissed".into(), opt(&self.seasonal_dismissed));
        m.insert("easter_eggs".into(), json!(self.easter_eggs));
        m.insert("full_log".into(), json!(self.full_log));
        m.insert("log_source".into(), json!(self.log_source));
        m.insert("tour_seen".into(), json!(self.tour_seen));
        m.insert("crosshair_hero_settings".into(), json!(self.crosshair_hero_settings));
        m.insert("themed_art".into(), json!(self.themed_art));
        m.insert("seasonal_effects".into(), json!(self.seasonal_effects));
        m.insert("relay_regions".into(), serde_json::to_value(&self.relay_regions).unwrap_or(Value::Null));
        serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default() + "\n"
    }

    /// Remember that a mature mod was revealed, newest last, capped.
    pub fn remember_reveal(&mut self, id: u64) {
        if !self.revealed.contains(&id) {
            self.revealed.push(id);
            if self.revealed.len() > REVEALS_KEEP {
                let extra = self.revealed.len() - REVEALS_KEEP;
                self.revealed.drain(..extra);
            }
        }
    }
}

pub fn path() -> PathBuf {
    crate::data::data_dir().join(FILE_NAME)
}

static STORE: RwLock<Option<Arc<Options>>> = RwLock::new(None);

fn load() -> Options {
    if rune::is_headless() || cfg!(test) {
        return Options::default();
    }
    fs::read_to_string(path())
        .map(|text| Options::from_json(&text))
        .unwrap_or_default()
}

/// The current options. Loaded from disk on first use. A shared snapshot: callers read
/// fields through it, and `set` swaps in a new one. It was a full copy of every option (maps
/// and lists included) on each of the 10 to 15 calls a poll makes.
pub fn get() -> Arc<Options> {
    if let Ok(slot) = STORE.read()
        && let Some(o) = slot.as_ref()
    {
        return o.clone();
    }
    let loaded = Arc::new(load());
    if let Ok(mut slot) = STORE.write() {
        slot.get_or_insert(loaded).clone()
    } else {
        Arc::new(Options::default())
    }
}

/// A copy to change and save.
pub fn get_owned() -> Options {
    (*get()).clone()
}

/// Replace the in-memory options without saving. Scenes and tests use it.
pub fn set(options: Options) {
    if let Ok(mut slot) = STORE.write() {
        *slot = Some(Arc::new(options));
    }
}

fn save(options: &Options) -> Result<(), String> {
    if rune::is_headless() || cfg!(test) {
        return Ok(());
    }
    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let partial = path.with_file_name(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    fs::write(&partial, options.to_json()).map_err(|e| e.to_string())?;
    fs::rename(&partial, &path).map_err(|e| {
        let _ = fs::remove_file(&partial);
        e.to_string()
    })
}

/// Change options, save, and report a failed write in the settings status line.
pub fn edit(st: &mut State, change: impl FnOnce(&mut Options)) {
    let mut options = get_owned();
    change(&mut options);
    set(options.clone());
    if let Err(error) = save(&options) {
        st.settings.message = Some((
            super::i18n::fill(super::i18n::tr("Settings could not be saved: {}"), &[&error]),
            true,
        ));
    }
}

/// Change options outside a handler (a worker result). Errors only reach the log.
pub fn edit_quiet(change: impl FnOnce(&mut Options)) {
    let mut options = get_owned();
    change(&mut options);
    set(options.clone());
    if let Err(error) = save(&options) {
        super::log::error(format!("options.json: {error}"));
    }
}

/// Back to the defaults, keeping only what records the user's history rather than a choice:
/// first-run state and remembered reveals go too, so Reset is a full reset.
pub fn reset() {
    // The relay rules stay in Windows Firewall after a reset, so the record of them stays too:
    // the status bar keeps saying regions are blocked until Servers > Regions resets them.
    let kept = get().relay_regions.clone();
    let mut options = Options::default();
    options.relay_regions.applied = kept.applied;
    options.relay_regions.pending = kept.pending;
    set(options.clone());
    if let Err(error) = save(&options) {
        super::log::error(format!("options.json: {error}"));
    }
}

// --- getters other screens read ------------------------------------------------------------

/// Locker: show every card expanded.
pub fn locker_expanded() -> bool {
    get().locker_expanded
}

/// Installed: a press on a mod with several files opens its GameBanana page.
pub fn installed_open_page() -> bool {
    get().installed_open_page
}

/// Replacing a mod installs without asking first.
pub fn skip_replace_confirm() -> bool {
    get().skip_replace_confirm
}

/// Updates: put Replace before Install.
pub fn swap_install_replace() -> bool {
    get().swap_install_replace
}

/// Updates: pair the remaining files of a multi-file mod by name.
pub fn match_update_files() -> bool {
    get().match_update_files
}

/// Installing another file of a mod turns the old one off.
pub fn switch_variants() -> bool {
    get().switch_variants
}

/// Turning on a second mod for a hero asks first.
pub fn hero_conflict_warning() -> bool {
    get().hero_conflict_warning
}

/// Requirements are checked when a mod is turned on or off.
pub fn prompt_requirements() -> bool {
    get().prompt_requirements
}

/// Changes to the addons folder wait while the game runs.
pub fn block_while_running() -> bool {
    get().block_while_running
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_field() {
        let mut o = Options {
            steam_root: Some("D:/Steam".into()),
            block_while_running: true,
            remove_paths_on_close: true,
            crash_check: false,
            auto_reapply: true,
            fix_before_launch: true,
            condebug: true,
            editor: Some("/Applications/Zed.app".into()),
            last_dialog_dir: Some("/tmp".into()),
            download_server: "filecache9".into(),
            mirror_first: true,
            fastest_server: Some("filecache3".into()),
            parts: 4,
            cache_dir: Some("/Volumes/Big/cache".into()),
            sources: vec![Source {
                url: "https://example.org/sigil-source.json".into(),
                name: "Example".into(),
                enabled: false,
            }],
            proxy_user: Some("me".into()),
            proxy_bypass: vec!["localhost".into(), "*.lan".into()],
            tray_minimize: true,
            tray_close: true,
            blur: 40,
            remember_reveals: true,
            revealed: vec![3, 1, 2],
            likely_nsfw: true,
            reveal_on_hover: true,
            always_tag: true,
            locker_expanded: true,
            installed_open_page: true,
            installed_sort: "name".into(),
            confirm_profile_update: false,
            prompt_requirements: false,
            ignore_overlap_default: true,
            switch_variants: true,
            match_update_files: false,
            swap_install_replace: true,
            skip_replace_confirm: true,
            allow_scripts: true,
            hero_conflict_warning: false,
            map_mod_warning: false,
            repair_before_launch: true,
            forge: true,
            presence_game: true,
            presence_hide_party: true,
            live_view: false,
            live_bridge: true,
            live_lookup: true,
            live_bridge_auto: true,
            live_bridge_build: "6123:1a2b".into(),
            presence_lines: BTreeMap::new(),
            hero_lines: BTreeMap::new(),
            imprint: true,
            onboarded: true,
            tips: false,
            tips_seen: vec!["browse".into()],
            unknown_alert_off: true,
            sidebar_collapsed: true,
            seen_pages: vec!["stats".into()],
            update_badges: false,
            vanilla_no_args: true,
            feature_hints: false,
            hints_done: vec!["settings-search".into()],
            counters: BTreeMap::from([("settings_visits".to_string(), 4)]),
            seasonal: true,
            seasonal_dismissed: Some("halloween:2026".into()),
            easter_eggs: true,
            full_log: true,
            log_source: "dumps".into(),
            tour_seen: true,
            crosshair_hero_settings: true,
            themed_art: "grid".into(),
            seasonal_effects: true,
            relay_regions: deadlock_core::relays::Choices {
                applied: vec!["hkg".into(), "tyo".into()],
                presets: vec![deadlock_core::relays::Preset { name: "EU only".into(), blocked: vec!["iad".into()] }],
                pending: true,
                health_bar: true,
            },
        };
        o.presence_lines.insert(
            "match".into(),
            Lines {
                top: "Playing {hero}".into(),
                bottom: "{mods} mods".into(),
            },
        );
        o.hero_lines.insert(
            "Haze".into(),
            Lines {
                top: "Haze time".into(),
                bottom: String::new(),
            },
        );
        assert_eq!(Options::from_json(&o.to_json()), o);
    }

    #[test]
    fn bad_values_fall_back_one_by_one() {
        let o = Options::from_json(
            r#"{"parts":3,"blur":"x","download_server":"nowhere","sources":[{"url":"http://plain.example"}],"tray_close":true}"#,
        );
        let d = Options::default();
        assert_eq!(o.parts, d.parts);
        assert_eq!(o.blur, d.blur);
        assert_eq!(o.download_server, "auto");
        assert!(o.sources.is_empty(), "plain http sources are dropped");
        assert!(o.tray_close);
        assert_eq!(Options::from_json("not json"), d);
    }

    #[test]
    fn reveals_are_capped_oldest_first() {
        let mut o = Options::default();
        for id in 0..(REVEALS_KEEP as u64 + 5) {
            o.remember_reveal(id);
        }
        o.remember_reveal(10);
        assert_eq!(o.revealed.len(), REVEALS_KEEP);
        assert_eq!(o.revealed[0], 5);
    }
}
