//! The game's console variables: a searchable catalogue with descriptions and limits, the
//! values the player has now, and game presets that set several at once.
//!
//! The list is Valve's own, as `SteamTracking/GameTracking-Deadlock` dumps it
//! (`DumpSource2/convars.txt`: `name default (flags)` lines, each followed by an indented
//! description). Sigil ships a snapshot of it (`data/convars.txt`, the entries a retail game can
//! set: `developmentonly` and `hidden` dropped) and prefers the newer copy the Performance page
//! caches from the same file (`performance/game/convars.txt`), so no new host is contacted.
//!
//! Current values come from two places on this computer: the autoexec (what Sigil writes) and
//! `game/citadel/cfg/machine_convars.vcfg` (where the game keeps archived settings). Edits go
//! through the autoexec document like every other line (`super::place_command`), so saving keeps
//! the backup, the changed-on-disk check and the player's own lines.
//!
//! Game presets after Deadlock Mod Manager's game presets branch (GPL-3.0, behaviour only): a
//! named set of `convar value` pairs, applied with a preview of what each line adds or changes,
//! saved by the player from the convars their autoexec sets now. Built-in presets are bundles
//! of Grimoire's premade commands (MIT, `data/autoexec-presets.json`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::write_atomic;

const BUNDLED: &str = include_str!("../../data/convars.txt");

/// What a value looks like, from its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Text,
}

/// One console variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Convar {
    pub name: String,
    pub default: String,
    pub kind: Kind,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub flags: Vec<String>,
    /// Empty when Valve wrote none.
    pub description: String,
}

impl Convar {
    /// `sv_cheats` must be on for the game to take it.
    pub fn cheat(&self) -> bool {
        self.has("cheat")
    }

    /// A server variable: it only does something in a lobby the player hosts.
    pub fn server_only(&self) -> bool {
        self.has("gamedll") && !self.has("clientdll") && !self.has("replicated")
    }

    /// The game saves it itself (`machine_convars.vcfg`).
    pub fn archived(&self) -> bool {
        self.has("archive")
    }

    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|seen| seen == flag)
    }

    /// The group the catalogue lists it under.
    pub fn group(&self) -> &'static str {
        group_of(&self.name)
    }

    /// Why a value cannot be written, or `None` when it can.
    pub fn check(&self, value: &str) -> Option<Problem> {
        let value = value.trim();
        if value.is_empty() {
            return Some(Problem::Empty);
        }
        if value.contains(['"', ';', '\n', '\r']) {
            return Some(Problem::Forbidden);
        }
        let number = match self.kind {
            Kind::Bool => {
                return (!matches!(value.to_ascii_lowercase().as_str(), "0" | "1" | "true" | "false"))
                    .then_some(Problem::NotBool);
            }
            Kind::Int => match value.parse::<i64>() {
                Ok(number) => number as f64,
                Err(_) => return Some(Problem::NotWhole),
            },
            Kind::Float => match value.parse::<f64>() {
                Ok(number) if number.is_finite() => number,
                _ => return Some(Problem::NotNumber),
            },
            Kind::Text => return None,
        };
        if let Some(min) = self.min
            && number < min
        {
            return Some(Problem::BelowMin(trim_number(min)));
        }
        if let Some(max) = self.max
            && number > max
        {
            return Some(Problem::AboveMax(trim_number(max)));
        }
        None
    }
}

/// Why a value was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Empty,
    /// Quotes, `;` or a line break would break out of the line.
    Forbidden,
    NotBool,
    NotWhole,
    NotNumber,
    BelowMin(String),
    AboveMax(String),
}

fn trim_number(value: f64) -> String {
    let text = format!("{value}");
    text.strip_suffix(".0").map(str::to_string).unwrap_or(text)
}

/// Groups by name prefix, most specific first.
const GROUPS: &[(&str, &str)] = &[
    ("citadel_crosshair", "Crosshair"),
    ("citadel_minimap", "Minimap"),
    ("minimap", "Minimap"),
    ("citadel_hud", "HUD"),
    ("hud_", "HUD"),
    ("citadel_unit_status", "HUD"),
    ("citadel_damage", "HUD"),
    ("citadel_", "Gameplay"),
    ("deadlock_", "Gameplay"),
    ("fps_", "Performance"),
    ("engine_", "Performance"),
    ("r_", "Graphics"),
    ("mat_", "Graphics"),
    ("csm_", "Graphics"),
    ("fog_", "Graphics"),
    ("lb_", "Graphics"),
    ("snd_", "Audio"),
    ("volume", "Audio"),
    ("audio_", "Audio"),
    ("dsp_", "Audio"),
    ("voice_", "Voice"),
    ("net_", "Network"),
    ("rate", "Network"),
    ("cl_interp", "Network"),
    ("cl_updaterate", "Network"),
    ("mm_", "Matchmaking"),
    ("m_", "Mouse and input"),
    ("sensitivity", "Mouse and input"),
    ("zoom_", "Mouse and input"),
    ("joy_", "Mouse and input"),
    ("cl_", "Client"),
    ("cam_", "Camera"),
    ("spec_", "Spectating"),
    ("tv_", "Spectating"),
    ("demo_", "Spectating"),
    ("sv_", "Server"),
];

/// Every group name, in the order the catalogue offers them.
pub fn groups() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (_, group) in GROUPS {
        if !out.contains(group) {
            out.push(group);
        }
    }
    out.push("Other");
    out
}

fn group_of(name: &str) -> &'static str {
    GROUPS
        .iter()
        .find(|(prefix, _)| name.starts_with(prefix))
        .map(|(_, group)| *group)
        .unwrap_or("Other")
}

/// Parse a DumpSource2 convar list. Entries flagged `developmentonly` or `hidden` are dropped:
/// a retail game ignores them.
pub fn parse_dump(text: &str) -> Vec<Convar> {
    let mut out: Vec<Convar> = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(last) = out.last_mut() {
                let words = line.trim();
                if words != "<no description>" {
                    if !last.description.is_empty() {
                        last.description.push(' ');
                    }
                    last.description.push_str(words);
                }
            }
            continue;
        }
        let Some(open) = line.rfind('(') else { continue };
        let head = line[..open].trim_end();
        let inside = line[open + 1..].trim_end().trim_end_matches(')');
        let Some((name, default)) = head.split_once(char::is_whitespace).or(Some((head, ""))) else {
            continue;
        };
        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            continue;
        }
        let mut flags = Vec::new();
        let (mut min, mut max) = (None, None);
        for part in inside.split(',') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("min:") {
                min = value.trim().parse().ok();
                continue;
            }
            if let Some(value) = part.strip_prefix("max:") {
                let mut words = value.split_whitespace();
                max = words.next().and_then(|word| word.parse().ok());
                flags.extend(words.map(str::to_string));
                continue;
            }
            flags.extend(part.split_whitespace().map(str::to_string));
        }
        if flags.iter().any(|flag| flag == "developmentonly" || flag == "hidden") {
            continue;
        }
        let default = default.trim().to_string();
        out.push(Convar {
            kind: kind_of(&default),
            name: name.to_string(),
            default,
            min,
            max,
            flags,
            description: String::new(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

fn kind_of(default: &str) -> Kind {
    match default {
        "true" | "false" => Kind::Bool,
        _ if default.parse::<i64>().is_ok() => Kind::Int,
        _ if default.parse::<f64>().is_ok() => Kind::Float,
        _ => Kind::Text,
    }
}

/// Where the catalogue came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSource {
    /// The copy the Performance page cached from GameTracking-Deadlock.
    Cached,
    /// The snapshot Sigil ships.
    Bundled,
}

/// The convar list, newest copy first.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub convars: Vec<Convar>,
    pub source: CatalogSource,
}

impl Catalog {
    pub fn get(&self, name: &str) -> Option<&Convar> {
        let lower = name.to_ascii_lowercase();
        self.convars
            .binary_search_by(|convar| convar.name.as_str().cmp(lower.as_str()))
            .ok()
            .map(|index| &self.convars[index])
    }

    /// Convars whose name or description holds every word of `query`, names that start with it
    /// first. `group` narrows to one group; `cheats` keeps `sv_cheats` variables.
    pub fn search(&self, query: &str, group: Option<&str>, cheats: bool) -> Vec<&Convar> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut hits: Vec<(u8, &Convar)> = self
            .convars
            .iter()
            .filter(|convar| cheats || !convar.cheat())
            .filter(|convar| group.is_none_or(|group| convar.group() == group))
            .filter_map(|convar| {
                if words.is_empty() {
                    return Some((2, convar));
                }
                let description = convar.description.to_lowercase();
                if !words.iter().all(|word| convar.name.contains(word.as_str()) || description.contains(word.as_str())) {
                    return None;
                }
                let rank = if convar.name.starts_with(words[0].as_str()) {
                    0
                } else if convar.name.contains(words[0].as_str()) {
                    1
                } else {
                    2
                };
                Some((rank, convar))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
        hits.into_iter().map(|(_, convar)| convar).collect()
    }
}

/// The snapshot Sigil ships, parsed once.
pub fn bundled() -> &'static [Convar] {
    static LIST: OnceLock<Vec<Convar>> = OnceLock::new();
    LIST.get_or_init(|| parse_dump(BUNDLED))
}

/// The catalogue: the Performance cache under `data_dir` when it holds a full list, else the
/// bundled snapshot.
pub fn load_catalog(data_dir: &Path) -> Catalog {
    let cached = crate::performance::fetch::file_path(
        data_dir,
        crate::performance::fetch::CONVARS_ID,
        crate::performance::fetch::CONVARS_FILE,
    );
    if let Ok(text) = std::fs::read_to_string(&cached) {
        let convars = parse_dump(&text);
        if convars.len() >= 500 {
            return Catalog { convars, source: CatalogSource::Cached };
        }
    }
    Catalog {
        convars: bundled().to_vec(),
        source: CatalogSource::Bundled,
    }
}

// --- current values ------------------------------------------------------------------------

/// The value a command line sets: the words after the name, unquoted. `None` for a bare
/// command or a line that is not `name value`.
pub fn line_value(line: &str) -> Option<(String, String)> {
    let name = super::command_name(line)?;
    let statement = super::strip_trailing_comment(line).trim();
    let rest = statement[statement.find(char::is_whitespace)?..].trim();
    let value = rest.trim_matches('"').trim();
    (!value.is_empty()).then(|| (name, value.to_string()))
}

/// Every `name value` the autoexec sets, the last line winning (the game runs it top to bottom).
pub fn autoexec_values(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if let Some((name, value)) = line_value(line) {
            out.insert(name, value);
        }
    }
    out
}

/// `machine_convars.vcfg` next to the autoexec.
pub fn machine_convars_path(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("cfg").join("machine_convars.vcfg")
}

/// The `"name" "value"` pairs of a `.vcfg` file.
pub fn parse_vcfg(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('"') else { continue };
        let Some((key, value)) = rest.split_once('"') else { continue };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|value| value.strip_suffix('"')) else {
            continue;
        };
        out.insert(key.to_ascii_lowercase(), value.to_string());
    }
    out
}

/// Where a convar's shown value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    /// The autoexec sets it.
    Autoexec(String),
    /// The game saved it.
    Game(String),
    /// Nothing sets it: the default applies.
    Default,
}

/// The value the player has for `name`.
pub fn current(name: &str, autoexec: &BTreeMap<String, String>, game: &BTreeMap<String, String>) -> Current {
    if let Some(value) = autoexec.get(name) {
        return Current::Autoexec(value.clone());
    }
    match game.get(name) {
        Some(value) => Current::Game(value.clone()),
        None => Current::Default,
    }
}

// --- game presets ----------------------------------------------------------------------------

/// A named set of convar values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamePreset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `(convar, value)`, in the order they are written.
    pub values: Vec<(String, String)>,
}

/// What applying a preset does to one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Add { name: String, value: String },
    Change { name: String, from: String, to: String },
    Same { name: String, value: String },
}

/// The preview of a preset against the autoexec's current values.
pub fn preview(preset: &GamePreset, autoexec: &BTreeMap<String, String>) -> Vec<Change> {
    preset
        .values
        .iter()
        .map(|(name, value)| {
            let name = name.to_ascii_lowercase();
            match autoexec.get(&name) {
                None => Change::Add { name, value: value.clone() },
                Some(old) if old == value => Change::Same { name, value: value.clone() },
                Some(old) => Change::Change { name, from: old.clone(), to: value.clone() },
            }
        })
        .collect()
}

/// The lines a preset writes, `name value` each.
pub fn preset_commands(preset: &GamePreset) -> Vec<String> {
    preset.values.iter().map(|(name, value)| format!("{name} {value}")).collect()
}

/// Built-in presets: bundles of Grimoire's premade commands (MIT).
pub fn builtin_presets() -> Vec<GamePreset> {
    let pick = |ids: &[&str]| -> Vec<(String, String)> {
        ids.iter()
            .filter_map(|id| {
                super::autoexec_presets()
                    .iter()
                    .flat_map(|category| category.presets.iter())
                    .find(|preset| preset.id == *id)
            })
            .filter_map(|preset| line_value(&preset.command))
            .collect()
    };
    let mut out = vec![
        GamePreset {
            name: "Fast and responsive".to_string(),
            description: "Uncapped frame rate, low latency and the full network rate.".to_string(),
            values: pick(&["uncapFps", "lowLatencyNvidia", "engineLowLatency", "maxNetworkRate"]),
        },
        GamePreset {
            name: "Clearer minimap".to_string(),
            description: "A faster minimap with bigger player icons and thicker ziplines.".to_string(),
            values: pick(&["fasterMinimap", "largerClickRadius", "largerPlayerIcons", "largerLocalPlayerIcon", "thickerZiplines"]),
        },
        GamePreset {
            name: "Clean HUD".to_string(),
            description: "New health bars, offscreen damage arrows and no post-match survey.".to_string(),
            values: pick(&["newHealthBars", "disablePostMatchSurvey", "offscreenDamageIndicator"]),
        },
    ];
    out.retain(|preset| !preset.values.is_empty());
    out
}

/// Saved presets, under the app data directory.
pub const PRESETS_FILE: &str = "game-presets.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct PresetFile {
    #[serde(default)]
    presets: Vec<GamePreset>,
}

pub fn load_presets(data_dir: &Path) -> Vec<GamePreset> {
    std::fs::read_to_string(data_dir.join(PRESETS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<PresetFile>(&text).ok())
        .map(|file| file.presets)
        .unwrap_or_default()
}

pub fn store_presets(data_dir: &Path, presets: &[GamePreset]) -> Result<()> {
    let text = serde_json::to_string_pretty(&PresetFile { presets: presets.to_vec() })
        .map_err(|error| Error::msg(format!("Could not write the presets: {error}")))?;
    std::fs::create_dir_all(data_dir).map_err(|error| Error::msg(format!("Could not create {}: {error}", data_dir.display())))?;
    write_atomic(&data_dir.join(PRESETS_FILE), text.as_bytes())
}

/// A preset from every known convar the autoexec sets now. `None` when it sets none.
pub fn preset_from_autoexec(name: &str, autoexec: &BTreeMap<String, String>, catalog: &Catalog) -> Option<GamePreset> {
    let values: Vec<(String, String)> = autoexec
        .iter()
        .filter(|(key, _)| catalog.get(key).is_some())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    (!values.is_empty()).then(|| GamePreset {
        name: name.trim().to_string(),
        description: String::new(),
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "citadel_hud_visible true (clientdll archive)\n\tShow the HUD.\n\tSecond line.\n\nfps_max 400 (min: 0, max: 1000 release)\n\tFrame rate limiter.\n\nai_think 0.1 (min: 0.01, developmentonly gamedll)\n\t<no description>\n\nsv_cheats false (notify replicated)\n\t<no description>\n\nsv_gravity 800 (gamedll cheat)\n\t<no description>\n\nname_only (release)\n\t<no description>\n";

    #[test]
    fn the_dump_parses_names_limits_flags_and_descriptions() {
        let list = parse_dump(DUMP);
        let names: Vec<&str> = list.iter().map(|convar| convar.name.as_str()).collect();
        assert_eq!(names, vec!["citadel_hud_visible", "fps_max", "name_only", "sv_cheats", "sv_gravity"]);
        let fps = &list[1];
        assert_eq!((fps.min, fps.max, fps.kind), (Some(0.0), Some(1000.0), Kind::Int));
        assert_eq!(fps.flags, vec!["release"]);
        assert_eq!(fps.description, "Frame rate limiter.");
        assert_eq!(list[0].description, "Show the HUD. Second line.");
        assert_eq!(list[0].kind, Kind::Bool);
        assert!(list[0].archived());
        assert!(list[4].cheat() && list[4].server_only());
        assert_eq!(list[0].group(), "HUD");
        assert_eq!(list[1].group(), "Performance");
    }

    #[test]
    fn the_bundled_snapshot_has_the_retail_list() {
        let list = bundled();
        assert!(list.len() > 1000, "{}", list.len());
        assert!(list.iter().all(|convar| !convar.has("developmentonly")));
        assert!(list.iter().any(|convar| convar.name == "fps_max"));
        assert!(list.windows(2).all(|pair| pair[0].name < pair[1].name), "sorted for get()");
    }

    #[test]
    fn values_are_checked_against_kind_and_limits() {
        let list = parse_dump(DUMP);
        let fps = &list[1];
        assert_eq!(fps.check("144"), None);
        assert_eq!(fps.check("2000"), Some(Problem::AboveMax("1000".into())));
        assert!(fps.check("1.5").is_some());
        assert!(fps.check("1; quit").is_some());
        assert_eq!(list[0].check("false"), None);
        assert!(list[0].check("maybe").is_some());
    }

    #[test]
    fn search_ranks_name_prefixes_first_and_hides_cheats() {
        let catalog = Catalog { convars: parse_dump(DUMP), source: CatalogSource::Bundled };
        let hits: Vec<&str> = catalog.search("fps", None, false).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(hits, vec!["fps_max"]);
        let hits: Vec<&str> = catalog.search("limiter", None, false).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(hits, vec!["fps_max"]);
        assert!(catalog.search("sv_gravity", None, false).is_empty());
        assert_eq!(catalog.search("sv_gravity", None, true).len(), 1);
        assert_eq!(catalog.search("", Some("HUD"), false).len(), 1);
        assert_eq!(catalog.get("FPS_MAX").map(|c| c.default.as_str()), Some("400"));
    }

    #[test]
    fn current_values_come_from_the_autoexec_then_the_game() {
        let autoexec = autoexec_values("fps_max 144\n// fps_max 60\nfps_max \"240\" // later\nbind x y z\n+jump\n");
        assert_eq!(autoexec.get("fps_max").map(String::as_str), Some("240"));
        let game = parse_vcfg("\"convars\"\n{\n\t\"citadel_hud_visible\"\t\t\"false\"\n\t\"fps_max\" \"60\"\n}\n");
        assert_eq!(current("fps_max", &autoexec, &game), Current::Autoexec("240".into()));
        assert_eq!(current("citadel_hud_visible", &autoexec, &game), Current::Game("false".into()));
        assert_eq!(current("r_x", &autoexec, &game), Current::Default);
    }

    #[test]
    fn a_preset_previews_adds_changes_and_same_lines() {
        let preset = GamePreset {
            name: "x".into(),
            description: String::new(),
            values: vec![("fps_max".into(), "0".into()), ("rate".into(), "1000000".into()), ("r_a".into(), "1".into())],
        };
        let autoexec = autoexec_values("fps_max 144\nrate 1000000\n");
        assert_eq!(
            preview(&preset, &autoexec),
            vec![
                Change::Change { name: "fps_max".into(), from: "144".into(), to: "0".into() },
                Change::Same { name: "rate".into(), value: "1000000".into() },
                Change::Add { name: "r_a".into(), value: "1".into() },
            ]
        );
        assert_eq!(preset_commands(&preset)[0], "fps_max 0");
    }

    #[test]
    fn builtin_presets_resolve_their_commands() {
        let presets = builtin_presets();
        assert!(!presets.is_empty());
        for preset in &presets {
            assert!(!preset.values.is_empty(), "{}", preset.name);
        }
    }

    #[test]
    fn presets_round_trip_through_the_data_folder() {
        let dir = std::env::temp_dir().join(format!("sigil-game-presets-{}", std::process::id()));
        let catalog = Catalog { convars: parse_dump(DUMP), source: CatalogSource::Bundled };
        let preset = preset_from_autoexec("Mine", &autoexec_values("fps_max 120\nunknown_cmd 1\n"), &catalog).unwrap();
        assert_eq!(preset.values, vec![("fps_max".to_string(), "120".to_string())]);
        store_presets(&dir, std::slice::from_ref(&preset)).unwrap();
        assert_eq!(load_presets(&dir), vec![preset]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
