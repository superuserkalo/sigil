//! Settings added in the parity round, kept in their own file (`<data>/options.json`) so they
//! never fight `settings.json` for a field. The same rules apply: each field reads on its own
//! with a fallback, writes go through a temporary sibling, and headless runs never touch disk.
//!
//! Other screens read a value with a getter, `options::get().locker_expanded`, which clones a
//! small struct; a handler changes one with `options::edit(st, |o| ..)`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

use serde_json::{Map, Value, json};

use crate::app::State;

pub const FILE_NAME: &str = "options.json";

/// Blur strengths offered for mature previews, in points.
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
    // Mature content
    pub blur: u32,
    pub remember_reveals: bool,
    pub revealed: Vec<u64>,
    pub likely_nsfw: bool,
    pub reveal_on_hover: bool,
    pub always_tag: bool,
    // Page preferences
    pub locker_expanded: bool,
    pub installed_open_page: bool,
    pub confirm_profile_update: bool,
    pub prompt_requirements: bool,
    pub ignore_overlap_default: bool,
    pub switch_variants: bool,
    pub match_update_files: bool,
    pub swap_install_replace: bool,
    pub skip_replace_confirm: bool,
    pub allow_scripts: bool,
    // Integrations
    pub forge: bool,
    /// Read console.log for the match phase and hero.
    pub presence_game: bool,
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
            confirm_profile_update: true,
            prompt_requirements: true,
            ignore_overlap_default: false,
            switch_variants: false,
            match_update_files: true,
            swap_install_replace: false,
            skip_replace_confirm: false,
            allow_scripts: false,
            forge: false,
            presence_game: false,
            presence_lines: BTreeMap::new(),
            hero_lines: BTreeMap::new(),
            imprint: false,
            onboarded: false,
            tips: true,
            tips_seen: Vec::new(),
            unknown_alert_off: false,
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
            confirm_profile_update: b("confirm_profile_update", d.confirm_profile_update),
            prompt_requirements: b("prompt_requirements", d.prompt_requirements),
            ignore_overlap_default: b("ignore_overlap_default", d.ignore_overlap_default),
            switch_variants: b("switch_variants", d.switch_variants),
            match_update_files: b("match_update_files", d.match_update_files),
            swap_install_replace: b("swap_install_replace", d.swap_install_replace),
            skip_replace_confirm: b("skip_replace_confirm", d.skip_replace_confirm),
            allow_scripts: b("allow_scripts", d.allow_scripts),
            forge: b("forge", d.forge),
            presence_game: b("presence_game", d.presence_game),
            presence_lines: lines_map(map.get("presence_lines")),
            hero_lines: lines_map(map.get("hero_lines")),
            imprint: b("imprint", d.imprint),
            onboarded: b("onboarded", d.onboarded),
            tips: b("tips", d.tips),
            tips_seen: strings("tips_seen"),
            unknown_alert_off: b("unknown_alert_off", d.unknown_alert_off),
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
        m.insert("confirm_profile_update".into(), json!(self.confirm_profile_update));
        m.insert("prompt_requirements".into(), json!(self.prompt_requirements));
        m.insert("ignore_overlap_default".into(), json!(self.ignore_overlap_default));
        m.insert("switch_variants".into(), json!(self.switch_variants));
        m.insert("match_update_files".into(), json!(self.match_update_files));
        m.insert("swap_install_replace".into(), json!(self.swap_install_replace));
        m.insert("skip_replace_confirm".into(), json!(self.skip_replace_confirm));
        m.insert("allow_scripts".into(), json!(self.allow_scripts));
        m.insert("forge".into(), json!(self.forge));
        m.insert("presence_game".into(), json!(self.presence_game));
        m.insert("presence_lines".into(), lines_json(&self.presence_lines));
        m.insert("hero_lines".into(), lines_json(&self.hero_lines));
        m.insert("imprint".into(), json!(self.imprint));
        m.insert("onboarded".into(), json!(self.onboarded));
        m.insert("tips".into(), json!(self.tips));
        m.insert("tips_seen".into(), json!(self.tips_seen));
        m.insert("unknown_alert_off".into(), json!(self.unknown_alert_off));
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

static STORE: RwLock<Option<Options>> = RwLock::new(None);

fn load() -> Options {
    if rune::is_headless() || cfg!(test) {
        return Options::default();
    }
    fs::read_to_string(path())
        .map(|text| Options::from_json(&text))
        .unwrap_or_default()
}

/// The current options. Loaded from disk on first use.
pub fn get() -> Options {
    if let Ok(slot) = STORE.read()
        && let Some(o) = slot.as_ref()
    {
        return o.clone();
    }
    let loaded = load();
    if let Ok(mut slot) = STORE.write() {
        slot.get_or_insert(loaded).clone()
    } else {
        Options::default()
    }
}

/// Replace the in-memory options without saving. Scenes and tests use it.
pub fn set(options: Options) {
    if let Ok(mut slot) = STORE.write() {
        *slot = Some(options);
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
    let mut options = get();
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
    let mut options = get();
    change(&mut options);
    set(options.clone());
    if let Err(error) = save(&options) {
        super::log::error(format!("options.json: {error}"));
    }
}

/// Back to the defaults, keeping only what records the user's history rather than a choice:
/// first-run state and remembered reveals go too, so Reset is a full reset.
pub fn reset() {
    let options = Options::default();
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
            confirm_profile_update: false,
            prompt_requirements: false,
            ignore_overlap_default: true,
            switch_variants: true,
            match_update_files: false,
            swap_install_replace: true,
            skip_replace_confirm: true,
            allow_scripts: true,
            forge: true,
            presence_game: true,
            presence_lines: BTreeMap::new(),
            hero_lines: BTreeMap::new(),
            imprint: true,
            onboarded: true,
            tips: false,
            tips_seen: vec!["browse".into()],
            unknown_alert_off: true,
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
