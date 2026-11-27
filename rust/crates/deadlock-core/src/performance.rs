//! Performance configs: community `gameinfo.gi` configs that trade visual detail
//! for frame rate, applied in place and removed exactly.
//!
//! # Source
//!
//! Configs come from their authors' GitHub repositories at runtime (`fetch`) and
//! are cached under the app data directory, so the screen works offline after
//! the first fetch. `catalog` names the repositories and folders; nothing about
//! a config's settings is bundled. Each upstream `gameinfo.gi` is diffed against
//! the stock file and classified (`generate`, ported from Grimoire, MIT).
//!
//! # What is written
//!
//! The author's performance convars and engine section edits, plus the author's
//! visibility and camera convars, each of which the player can turn off per
//! config (owner decision, 2026-11-17). Developer and hideout testing tools stay
//! off unless the player turns them on. Convars the game refuses to read from
//! `gameinfo.gi` (flagged `gameinfo_cannot_override` in Valve's convar list) are
//! not written; they would do nothing.
//!
//! # Patching
//!
//! The file is Valve KeyValues. Edits are line based so untouched lines keep their
//! exact bytes, and every change is tagged with `sigil-perf` so removal needs no
//! outside state: added lines are dropped, edited lines get their recorded value
//! back, commented-out lines are restored. The BEGIN marker records the config,
//! version and commit; a second marker line records the switches. Work happens on
//! LF text and the file's line endings are restored on write. An apply that would
//! change the number of braces is refused. A file carrying Grimoire's own
//! `grimoire-perf` markers is left alone.

pub mod as_written;
pub mod catalog;
pub mod classify;
pub mod edits;
pub mod exchange;
pub mod fetch;
pub mod generate;
pub mod hosted;
pub mod repos;
pub mod video;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{escape_regex, write_atomic};

pub use catalog::{ConfigDef, HostedRef, SourceDef, Tier};
pub use classify::Group;
pub use generate::{Body, Held, OptIn};

const MARKER: &str = "sigil-perf";
const FOREIGN_MARKER: &str = "grimoire-perf";
const BACKUP_SUFFIX: &str = "sigil-bak";

/// One key edit inside a `gameinfo.gi` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionOp {
    pub path: Vec<String>,
    pub key: String,
    pub value: Option<String>,
    #[serde(default)]
    pub remove: bool,
}

/// One video.txt file a config ships.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoSet {
    /// The upstream file name, `video.txt` or a variant such as `video(No Shadows).txt`.
    pub file: String,
    /// What the variant is, for the picker: `Default`, `No shadows`, `Shadows`.
    pub label: String,
    pub settings: Vec<(String, String)>,
}

/// How Valve's convar list says a config convar fares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvarState {
    /// Read from gameinfo.gi as written.
    Read,
    /// The game refuses this convar from gameinfo.gi; Sigil does not write it.
    Ignored,
    /// Not in the game's current convar list. Written, but may do nothing.
    Unknown,
}

/// A config ready to show and apply: its identity, the version cached, and
/// what it writes.
#[derive(Debug, Clone)]
pub struct Config {
    pub def: &'static ConfigDef,
    pub source: &'static SourceDef,
    pub version: String,
    pub commit: String,
    /// When the author last changed it, yyyy-mm-dd.
    pub date: String,
    /// Path of the file inside the author's repository.
    pub path: String,
    pub body: Body,
    pub videos: Vec<VideoSet>,
    /// Convars in the body or opt-ins the game will not read.
    pub ignored: Vec<String>,
    /// Convars not in the game's current list.
    pub unknown: Vec<String>,
}

impl Config {
    pub fn id(&self) -> &'static str {
        self.def.id
    }

    pub fn name(&self) -> &'static str {
        self.def.name
    }

    /// The author's page for this config: GitHub at the cached commit, or the
    /// GameBanana mod page.
    pub fn link(&self) -> String {
        if let Some(hosted) = self.def.gamebanana {
            return format!("https://gamebanana.com/mods/{}", hosted.mod_id);
        }
        // A mod's own gameinfo: the mod page, when it came from GameBanana (`mod<id>`).
        if self.source.id == "mod" {
            return self
                .commit
                .strip_prefix("mod")
                .and_then(|id| id.parse::<u64>().ok())
                .map(|id| format!("https://gamebanana.com/mods/{id}"))
                .unwrap_or_default();
        }
        format!(
            "https://github.com/{}/blob/{}/{}",
            self.source.repo,
            self.commit,
            self.path.replace(' ', "%20")
        )
    }

    pub fn credit(&self) -> String {
        format!("{} by {}", self.source.name, self.def.author)
    }

    pub fn convar_state(&self, key: &str) -> ConvarState {
        if self.ignored.iter().any(|k| k == key) {
            ConvarState::Ignored
        } else if self.unknown.iter().any(|k| k == key) {
            ConvarState::Unknown
        } else {
            ConvarState::Read
        }
    }

    /// The convars written from the body (ignored ones dropped).
    pub fn body_convars(&self) -> impl Iterator<Item = &(String, String)> {
        self.body
            .convars
            .iter()
            .filter(|(key, _)| self.convar_state(key) != ConvarState::Ignored)
    }

    /// Opt-ins the player can switch: the game reads them.
    pub fn switchable(&self) -> impl Iterator<Item = &OptIn> {
        self.body
            .opt_ins
            .iter()
            .filter(|opt| self.convar_state(&opt.key) != ConvarState::Ignored)
    }

    /// The author's defaults: visibility and camera on, developer tools off,
    /// engine section edits on.
    pub fn default_options(&self) -> Options {
        Options {
            opt_ins: self
                .switchable()
                .filter(|opt| opt.group.on_by_default())
                .map(|opt| opt.key.clone())
                .collect(),
            engine: true,
            video: self.videos.first().map(|set| set.file.clone()),
            edits: BTreeMap::new(),
        }
    }

    /// How many lines an apply with `options` writes.
    pub fn setting_count(&self, options: &Options) -> usize {
        let engine = if options.engine {
            self.body.section_ops.len()
        } else {
            0
        };
        let opt_ins = self
            .switchable()
            .filter(|opt| options.opt_ins.contains(&opt.key))
            .count();
        let mut extra = 0;
        for key in options.edits.keys() {
            if self.convar_state(key) == ConvarState::Ignored {
                continue;
            }
            let in_body = self.body.convars.iter().any(|(k, _)| k == key);
            let in_opt = self
                .switchable()
                .find(|opt| &opt.key == key)
                .is_some_and(|_| options.opt_ins.contains(key));
            if !in_body && !in_opt {
                extra += 1;
            }
        }
        engine + self.body_convars().count() + opt_ins + extra
    }

    pub fn video(&self, file: Option<&str>) -> Option<&VideoSet> {
        match file {
            Some(file) => self.videos.iter().find(|set| set.file == file),
            None => self.videos.first(),
        }
    }

    /// Build from cached texts. `game` is Valve's convar list when known.
    pub fn from_texts(
        def: &'static ConfigDef,
        meta: &fetch::CachedConfig,
        baseline: &generate::Parsed,
        text: &str,
        videos: &[(String, String)],
        game: Option<&GameConvars>,
    ) -> Config {
        Config::from_parsed(def, meta, baseline, &generate::parse(text), videos, game)
    }

    /// `from_texts` with the config's file already parsed.
    pub fn from_parsed(
        def: &'static ConfigDef,
        meta: &fetch::CachedConfig,
        baseline: &generate::Parsed,
        parsed: &generate::Parsed,
        videos: &[(String, String)],
        game: Option<&GameConvars>,
    ) -> Config {
        let source = catalog::source(def.source);
        let mut body = generate::generate(baseline, parsed);
        let mut sets = Vec::new();
        for (file, raw) in videos {
            let (settings, held) =
                generate::generate_video(&generate::parse_video(raw), source.verbatim_video);
            for item in held {
                if !body.held.contains(&item) {
                    body.held.push(item);
                }
            }
            if !settings.is_empty() {
                sets.push(VideoSet {
                    file: file.clone(),
                    label: video_label(file, videos.len()),
                    settings,
                });
            }
        }
        let mut ignored = Vec::new();
        let mut unknown = Vec::new();
        if let Some(game) = game {
            let keys = body
                .convars
                .iter()
                .map(|(key, _)| key)
                .chain(body.opt_ins.iter().map(|opt| &opt.key));
            for key in keys {
                match game.state(key) {
                    ConvarState::Ignored => ignored.push(key.clone()),
                    ConvarState::Unknown => unknown.push(key.clone()),
                    ConvarState::Read => {}
                }
            }
        }
        Config {
            def,
            source,
            version: meta.version.clone(),
            commit: meta.commit.clone(),
            date: meta.date.clone(),
            path: meta.path.clone(),
            body,
            videos: sets,
            ignored,
            unknown,
        }
    }
}

fn video_label(file: &str, count: usize) -> String {
    let lower = file.to_ascii_lowercase();
    if lower.contains("no shadows") {
        "No shadows".into()
    } else if lower.contains("shadows") || (count > 1 && lower == "video.txt") {
        "Shadows".into()
    } else {
        "Default".into()
    }
}

/// `name default ... (flags)`: the name and the last parenthesised group, which must end the
/// line and hold no parentheses. The same lines `^([A-Za-z_][\w.]*)\s.*\(([^()]*)\)\s*$` takes,
/// without a regex (the list has thousands of lines and is read each time the page opens).
fn dump_line(line: &str) -> Option<(&str, &str)> {
    let first = line.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let name_end = line
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(line.len());
    if !line[name_end..].starts_with(char::is_whitespace) {
        return None;
    }
    let rest = line.trim_end();
    let body = rest.strip_suffix(')')?;
    let open = body.rfind('(')?;
    if open < name_end || body[open + 1..].contains(')') {
        return None;
    }
    Some((&line[..name_end], &body[open + 1..]))
}

/// Valve's convar list: which convars exist and which the game refuses from
/// `gameinfo.gi`.
#[derive(Debug, Clone, Default)]
pub struct GameConvars {
    blocked: HashMap<String, bool>,
}

impl GameConvars {
    /// Parse `DumpSource2/convars.txt`: `name default (flags)` lines, each
    /// followed by an indented description.
    pub fn parse(text: &str) -> GameConvars {
        let mut blocked = HashMap::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.starts_with(char::is_whitespace) {
                continue;
            }
            if let Some((name, flags)) = dump_line(line) {
                blocked.insert(name.to_string(), flags.contains("gameinfo_cannot_override"));
            }
        }
        GameConvars { blocked }
    }

    pub fn len(&self) -> usize {
        self.blocked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
    }

    pub fn state(&self, key: &str) -> ConvarState {
        match self.blocked.get(key) {
            Some(true) => ConvarState::Ignored,
            Some(false) => ConvarState::Read,
            None => ConvarState::Unknown,
        }
    }
}

/// Every cached config and what the cache knows about its own freshness.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub configs: Vec<Config>,
    pub checked_at: Option<String>,
    /// Per config id: why the newest upstream version was not used.
    pub errors: BTreeMap<String, String>,
    /// Whether Valve's convar list is cached.
    pub game_convars: bool,
}

impl Catalog {
    pub fn get(&self, id: &str) -> Option<&Config> {
        self.configs.iter().find(|config| config.id() == id)
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }
}

/// Read the cache. Lighter tiers first, in catalog order inside a tier.
pub fn load(data_dir: &Path) -> Catalog {
    let index = fetch::read_index(data_dir);
    let mut catalog = Catalog {
        checked_at: index.checked_at.clone(),
        errors: index.errors.clone(),
        ..Catalog::default()
    };
    let Ok(baseline) = std::fs::read_to_string(fetch::file_path(
        data_dir,
        catalog::BASELINE_ID,
        "gameinfo.gi",
    )) else {
        return catalog;
    };
    // Valve's convar list parses on its own thread while the stock file and the configs parse
    // here. The configs stay in series: their parsers share compiled regexes, and on several
    // threads at once they were slower than one after another (measured).
    let (baseline, game, built) = std::thread::scope(|scope| {
        let game = scope.spawn(|| {
            std::fs::read_to_string(fetch::file_path(
                data_dir,
                fetch::CONVARS_ID,
                fetch::CONVARS_FILE,
            ))
            .ok()
            .map(|text| GameConvars::parse(&text))
            .filter(|game| game.len() > 1000)
        });
        let baseline = generate::parse(&baseline);
        let mut texts = Vec::new();
        for def in catalog::CONFIGS {
            let Some(meta) = index.configs.get(def.id) else {
                continue;
            };
            let Ok(text) =
                std::fs::read_to_string(fetch::file_path(data_dir, def.id, "gameinfo.gi"))
            else {
                continue;
            };
            let videos: Vec<(String, String)> = meta
                .videos
                .iter()
                .filter_map(|(name, _)| {
                    std::fs::read_to_string(fetch::file_path(data_dir, def.id, name))
                        .ok()
                        .map(|text| (name.clone(), text))
                })
                .collect();
            texts.push((def, meta, generate::parse(&text), videos));
        }
        let game = game.join().ok().flatten();
        let built: Vec<Config> = texts
            .into_iter()
            .map(|(def, meta, parsed, videos)| {
                Config::from_parsed(def, meta, &baseline, &parsed, &videos, game.as_ref())
            })
            .collect();
        (baseline, game, built)
    });
    catalog.game_convars = game.is_some();
    catalog.configs.extend(built);
    // Custom presets from GitHub repositories the player added (#572), after the built-ins.
    catalog.configs.extend(repos::configs(data_dir, &baseline, game.as_ref()));
    catalog.configs.sort_by_key(|config| config.def.tier);
    catalog
}

/// Fetch newer releases from GitHub into the cache. Network; run off the UI thread.
pub fn refresh(data_dir: &Path) -> Result<fetch::RefreshReport> {
    fetch::refresh(&fetch::Http::default(), data_dir, &crate::util::now_iso())
}

/// Fetch only Valve's convar list into the cache. Network; run off the UI thread.
pub fn refresh_convar_list(data_dir: &Path) -> Result<()> {
    fetch::refresh_convar_list(&fetch::Http::default(), data_dir)
}

/// The cache is empty or more than `hours` old.
pub fn needs_refresh(data_dir: &Path, hours: i64) -> bool {
    fetch::stale(&fetch::read_index(data_dir), chrono::Utc::now(), hours)
}

/// Minutes since an RFC 3339 time, for "checked 2 hours ago".
pub fn minutes_since(at: &str) -> Option<i64> {
    let then = chrono::DateTime::parse_from_rfc3339(at).ok()?;
    Some(
        chrono::Utc::now()
            .signed_duration_since(then)
            .num_minutes()
            .max(0),
    )
}

/// The switches an apply uses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// Opt-in convars that are on.
    pub opt_ins: BTreeSet<String>,
    /// Write the engine section edits.
    pub engine: bool,
    /// The video.txt variant, for configs that ship one.
    pub video: Option<String>,
    /// Values the player set in the convar editor, keyed by convar.
    pub edits: BTreeMap<String, String>,
}

/// What the file says is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Stock: no Sigil markers.
    None,
    /// A Sigil config, as its markers record it.
    Config {
        id: String,
        version: String,
        commit: String,
        /// The opt-ins that are on and whether engine edits are in. `None` for
        /// a file written before Sigil recorded switches.
        switches: Option<(BTreeSet<String>, bool)>,
    },
    /// Grimoire's own performance config is in the file.
    Foreign,
}

fn begin_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"Sigil Performance Config BEGIN \(preset=([\w-]+) v(.+?)(?: @([0-9a-f]{6,40}))?\)",
        )
        .expect("begin marker regex")
    })
}

fn switches_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"// switches: engine=(on|off) opt-ins=([\w.,]*) \[sigil-perf\]")
            .expect("switches regex")
    })
}

/// Read the applied state from `gameinfo.gi` text.
pub fn applied(text: &str) -> Applied {
    if let Some(caps) = begin_re().captures(text) {
        let switches = switches_re().captures(text).map(|s| {
            let keys = s[2]
                .split(',')
                .filter(|key| !key.is_empty())
                .map(str::to_string)
                .collect();
            (keys, &s[1] == "on")
        });
        return Applied::Config {
            id: caps[1].to_string(),
            version: caps[2].to_string(),
            commit: caps
                .get(3)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            switches,
        };
    }
    if text.contains(FOREIGN_MARKER) {
        return Applied::Foreign;
    }
    Applied::None
}

/// The result of an apply: the new text and the section edits that found no
/// section (usually a game update moved it).
#[derive(Debug, Clone)]
pub struct Patched {
    pub text: String,
    pub skipped: Vec<String>,
    pub written: usize,
}

/// Apply `config` with `options` to `original`. A Sigil config already in the
/// text is removed first, so the file always goes stock to config.
pub fn apply_text(original: &str, config: &Config, options: &Options) -> Result<Patched> {
    if original.contains(FOREIGN_MARKER) {
        return Err(Error::msg(
            "Grimoire has a performance config in this file. Turn it off there first.",
        ));
    }
    let crlf = original.contains("\r\n");
    let mut content = if crlf {
        original.replace("\r\n", "\n")
    } else {
        original.to_string()
    };
    let baseline_braces = brace_count(&content);
    if content.contains(MARKER) {
        content = remove_markers(&content);
    }

    let mut skipped = Vec::new();
    let mut written = 0usize;
    if options.engine {
        for op in &config.body.section_ops {
            match apply_op(&content, op) {
                Some(next) => {
                    content = next;
                    written += 1;
                }
                None => skipped.push(format!("{}/{}", op.path.join("/"), op.key)),
            }
        }
    }

    // The player's edits ride along with the config: same block, same removal.
    // Only edits that pass the same check the editor applies reach the file, whatever path
    // put them in the options (typed, imported, or read back from the saved edits file).
    let edits: std::collections::BTreeMap<&String, &String> = options
        .edits
        .iter()
        .filter(|(key, value)| crate::performance::edits::is_valid_edit(key, value))
        .collect();
    let edited = |key: &str, value: &str| {
        edits
            .get(&key.to_string())
            .map(|v| v.to_string())
            .unwrap_or_else(|| value.to_string())
    };
    let mut convars: Vec<(String, String)> = config
        .body_convars()
        .map(|(key, value)| (key.clone(), edited(key, value)))
        .collect();
    let on: Vec<&OptIn> = config
        .switchable()
        .filter(|opt| options.opt_ins.contains(&opt.key))
        .collect();
    convars.extend(
        on.iter()
            .map(|opt| (opt.key.clone(), edited(&opt.key, &opt.value))),
    );
    // Settings a player added that the config does not set.
    let mut keys: std::collections::HashSet<String> =
        convars.iter().map(|(key, _)| key.clone()).collect();
    for (key, value) in edits.iter().map(|(k, v)| (*k, *v)) {
        if config.convar_state(key) == ConvarState::Ignored || !keys.insert(key.clone()) {
            continue;
        }
        convars.push((key.clone(), value.clone()));
    }

    let convars_path = ["ConVars".to_string()];
    let Some((start, end)) = find_section_by_path(&content, &convars_path) else {
        return Err(Error::msg(
            "gameinfo.gi has no ConVars section. Verify the game files in Steam, then try again.",
        ));
    };
    let existing: std::collections::HashSet<String> = content[start..end]
        .split('\n')
        .filter_map(entry_key)
        .collect();
    let mut inject = Vec::new();
    for (key, value) in &convars {
        if existing.contains(key) {
            let op = SectionOp {
                path: convars_path.to_vec(),
                key: key.clone(),
                value: Some(value.clone()),
                remove: false,
            };
            if let Some(next) = apply_op(&content, &op) {
                content = next;
                written += 1;
            }
        } else {
            inject.push((key.clone(), value.clone()));
        }
    }
    written += inject.len();

    let (start, end) = find_section_by_path(&content, &convars_path)
        .ok_or_else(|| Error::msg("gameinfo.gi lost its ConVars section while patching."))?;
    let indent = detect_indent(&content[start..end]);
    let width = inject.iter().map(|(key, _)| key.len()).max().unwrap_or(0) + 1;
    let commit = &config.commit[..config.commit.len().min(12)];
    let on_keys: Vec<&str> = on.iter().map(|opt| opt.key.as_str()).collect();
    let mut block = vec![
        format!(
            "{indent}// ==== Sigil Performance Config BEGIN (preset={} v{} @{commit}) ====",
            as_written::plain(config.id()),
            as_written::plain(&config.version)
        ),
        format!(
            "{indent}// {}: values from {} ({}) [{MARKER}]",
            as_written::plain(config.name()),
            as_written::plain(&config.credit()),
            as_written::plain(config.source.license)
        ),
        format!("{indent}// {} @ {commit} [{MARKER}]", as_written::plain(&config.source.url())),
        format!(
            "{indent}// switches: engine={} opt-ins={} [{MARKER}]",
            if options.engine { "on" } else { "off" },
            on_keys.join(",")
        ),
        format!("{indent}// remove or change it in Sigil > Performance [{MARKER}]"),
    ];
    for (key, value) in &inject {
        block.push(format!(
            "{indent}{key:<width$}{} // {MARKER} added",
            quote(value)
        ));
    }
    block.push(format!("{indent}// ==== Sigil Performance Config END ===="));
    let block = block.join("\n");
    // Splice after the rest of the `ConVars {` line when it is only whitespace,
    // so trailing spaces after the brace survive a round trip.
    let tail = match content[start..].find('\n') {
        Some(offset) if content[start..start + offset].trim().is_empty() => start + offset,
        _ => start,
    };
    content = format!("{}\n{}{}", &content[..tail], block, &content[tail..]);

    if brace_count(&content) != baseline_braces {
        return Err(Error::msg(
            "The change would unbalance gameinfo.gi, so nothing was written.",
        ));
    }
    let text = if crlf {
        content.replace('\n', "\r\n")
    } else {
        content
    };
    Ok(Patched {
        text,
        skipped,
        written,
    })
}

/// The ConVars values a `gameinfo.gi` holds now, for the editor's comparison
/// with what it wrote. Keys outside the `ConVars` section are not included.
pub fn convar_values(text: &str) -> BTreeMap<String, String> {
    let content = text.replace("\r\n", "\n");
    let Some((start, end)) = find_section_by_path(&content, &["ConVars".to_string()]) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for line in content[start..end].split('\n') {
        let Some(key) = entry_key(line) else {
            continue;
        };
        let Some(entry) = match_entry_line(line, &key) else {
            continue;
        };
        out.insert(key, entry.value.trim_matches('"').to_string());
    }
    out
}

/// Undo every Sigil change in `text`. Text without markers comes back unchanged.
pub fn remove_text(text: &str) -> String {
    let crlf = text.contains("\r\n");
    let content = if crlf {
        text.replace("\r\n", "\n")
    } else {
        text.to_string()
    };
    let restored = remove_markers(&content);
    if crlf {
        restored.replace('\n', "\r\n")
    } else {
        restored
    }
}

// --- install side ----------------------------------------------------------

pub fn gameinfo_path(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("gameinfo.gi")
}

/// The one-time copy taken before the first apply.
pub fn backup_path(game_root: &Path) -> PathBuf {
    gameinfo_path(game_root).with_extension(format!("gi.{BACKUP_SUFFIX}"))
}

/// Read what the install's `gameinfo.gi` has applied.
pub fn read_status(game_root: &Path) -> Result<Applied> {
    let text = std::fs::read_to_string(gameinfo_path(game_root))?;
    Ok(applied(&text))
}

/// GAME WRITE. Apply a config to the install's `gameinfo.gi`. Called only from
/// the Apply button after its confirm. Keeps a one-time backup of a
/// structurally sound file first and writes atomically.
pub fn apply_to_install(game_root: &Path, config: &Config, options: &Options) -> Result<Patched> {
    let path = gameinfo_path(game_root);
    let on_disk = std::fs::read_to_string(&path)?;
    // A file applied as written goes back to its stock copy, never through a marker undo.
    let as_written = as_written::is_applied(&on_disk);
    let original = if as_written {
        as_written::read_stock(game_root)?
    } else {
        on_disk
    };
    let patched = apply_text(&original, config, options)?;
    let backup = backup_path(game_root);
    if !backup.exists()
        && find_section_by_path(&original.replace("\r\n", "\n"), &["ConVars".to_string()]).is_some()
    {
        write_atomic(&backup, original.as_bytes())?;
    }
    write_atomic(&path, patched.text.as_bytes())?;
    if as_written {
        let _ = std::fs::remove_file(as_written::stock_path(game_root));
    }
    Ok(patched)
}

/// What the last Apply wrote, kept in app data so the page can tell a game update or a Steam
/// verify (which put Valve's `gameinfo.gi` back) from the player turning the config off. The
/// page never trusts it for what is on: that is always read from the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    /// How many settings the apply wrote.
    pub written: usize,
    /// When, RFC 3339.
    pub at: String,
}

const APPLIED_RECORD: &str = "applied.json";

fn record_path(data_dir: &Path) -> PathBuf {
    fetch::cache_root(data_dir).join(APPLIED_RECORD)
}

pub fn read_record(data_dir: &Path) -> Option<AppliedRecord> {
    let text = std::fs::read_to_string(record_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Forget the last apply (Turn off, or the player dismissing a reset notice). App data only.
pub fn forget_record(data_dir: &Path) -> Result<()> {
    match std::fs::remove_file(record_path(data_dir)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// Lines in `text` that carry a Sigil performance change: added, changed and commented out.
/// Read from the file itself, so it says what the game will read, not what Sigil remembers.
pub fn settings_in_file(text: &str) -> usize {
    let added = format!("// {MARKER} added");
    let was = format!("// {MARKER} was ");
    let removed = format!("// {MARKER} removed: ");
    text.lines()
        .filter(|line| line.contains(&added) || line.contains(&was) || line.contains(&removed))
        .count()
}

/// The config the last Apply put in, when the file no longer has it: Valve's file came back
/// (a game update or Steam's file check) or something else rewrote it.
pub fn reset_since_apply(text: &str, record: Option<&AppliedRecord>) -> Option<AppliedRecord> {
    let record = record?;
    match applied(text) {
        Applied::None if !as_written::is_applied(text) => Some(record.clone()),
        _ => None,
    }
}

/// What an Apply did, read back from the file after the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub path: PathBuf,
    /// Settings the apply wrote.
    pub written: usize,
    /// Settings lines found in the file when it was read back.
    pub in_file: usize,
    /// Engine edits that found no section.
    pub skipped: Vec<String>,
}

/// GAME WRITE. Apply, then read the file back and check the game will see the config: the
/// marker names it and every convar line holds the value Sigil meant. The record of the apply
/// goes to app data only once the check passed. Called only from the Apply button.
pub fn apply_and_check(
    game_root: &Path,
    data_dir: &Path,
    config: &Config,
    options: &Options,
    now: &str,
) -> Result<ApplyReport> {
    let patched = apply_to_install(game_root, config, options)?;
    let path = gameinfo_path(game_root);
    let back = std::fs::read_to_string(&path)?;
    let on = matches!(applied(&back), Applied::Config { ref id, .. } if id == config.id());
    if !on {
        return Err(Error::msg(format!(
            "{} was written but does not hold {} when read back. Something else changed the file; try again.",
            path.display(),
            config.name()
        )));
    }
    let values = convar_values(&back);
    let wrong: Vec<String> = exchange::effective(config, options)
        .into_iter()
        // An edit the editor's check refuses never reaches the file (`apply_text`).
        .filter(|(key, value)| {
            !options.edits.contains_key(key) || edits::is_valid_edit(key, value)
        })
        .filter(|(key, value)| {
            values.get(key).map(|v| normalize_value(v)) != Some(normalize_value(value))
        })
        .map(|(key, _)| key)
        .collect();
    if !wrong.is_empty() {
        return Err(Error::msg(format!(
            "{} was written, but {} setting{} did not read back as set ({}). Turn the config off and apply it again.",
            path.display(),
            wrong.len(),
            if wrong.len() == 1 { "" } else { "s" },
            wrong.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    let record = AppliedRecord {
        id: config.id().to_string(),
        name: config.name().to_string(),
        version: config.version.clone(),
        written: patched.written,
        at: now.to_string(),
    };
    write_atomic(&record_path(data_dir), &serde_json::to_vec_pretty(&record)?)?;
    Ok(ApplyReport {
        path,
        written: patched.written,
        in_file: settings_in_file(&back),
        skipped: patched.skipped,
    })
}

fn normalize_value(value: &str) -> String {
    let value = value.trim().trim_matches('"').trim().to_ascii_lowercase();
    match value.as_str() {
        "true" => "1".into(),
        "false" => "0".into(),
        _ => match value.parse::<f64>() {
            Ok(number) if value.chars().any(|c| c.is_ascii_digit()) => format!("{number}"),
            _ => value,
        },
    }
}

/// GAME WRITE. Remove Sigil's config from the install's `gameinfo.gi`, restoring
/// stock values. Called only from Turn off after its confirm.
pub fn remove_from_install(game_root: &Path) -> Result<bool> {
    let path = gameinfo_path(game_root);
    let text = std::fs::read_to_string(&path)?;
    if !text.contains(MARKER) {
        return Ok(false);
    }
    // Applied as written (#590): the stock copy goes back, not a line by line undo.
    if as_written::is_applied(&text) {
        return as_written::remove_from_install(game_root);
    }
    write_atomic(&path, remove_text(&text).as_bytes())?;
    Ok(true)
}

// --- KeyValues line helpers -------------------------------------------------

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(index) => &line[..index],
        None => line,
    }
}

/// Locate `name { ... }` within `[from, to)` by balanced braces. Returns the
/// body range inside the braces. Mentions on commented lines do not count.
fn find_section(content: &str, name: &str, from: usize, to: usize) -> Option<(usize, usize)> {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let re = {
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match cache.get(name) {
            Some(re) => re.clone(),
            None => {
                let re = Regex::new(&format!(r"(^|[\s}}]){}\s*\{{", escape_regex(name))).ok()?;
                cache.insert(name.to_string(), re.clone());
                re
            }
        }
    };
    let bytes = content.as_bytes();
    let mut at = from;
    while let Some(found) = re.find_at(content, at) {
        if found.start() >= to {
            return None;
        }
        at = found.end();
        let brace = found.end() - 1;
        let line_start = content[..found.start()]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        if content[line_start..brace].contains("//") {
            continue;
        }
        let body_start = brace + 1;
        let mut depth = 1usize;
        for (index, byte) in bytes.iter().enumerate().take(to).skip(body_start) {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((body_start, index));
                    }
                }
                _ => {}
            }
        }
        return None;
    }
    None
}

fn find_section_by_path(content: &str, path: &[String]) -> Option<(usize, usize)> {
    let mut range = (0, content.len());
    for name in path {
        range = find_section(content, name, range.0, range.1)?;
    }
    Some(range)
}

fn detect_indent(body: &str) -> String {
    for line in body.split('\n') {
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !indent.is_empty() && line.len() > indent.len() {
            return indent;
        }
    }
    "\t\t".to_string()
}

struct Entry {
    prefix: String,
    value: String,
    suffix: String,
}

/// `[ws]["]key["] ws value rest`, the same shape as
/// `^([ \t]*"?key"?[ \t]+)("[^"]*"|[^\s/]+)(.*)$`, without building a regex
/// per line (an apply checks every line of a section for every key).
fn match_entry_line(line: &str, key: &str) -> Option<Entry> {
    let bytes = line.as_bytes();
    let blank = |b: u8| b == b' ' || b == b'\t';
    let mut at = 0;
    while at < bytes.len() && blank(bytes[at]) {
        at += 1;
    }
    if bytes.get(at) == Some(&b'"') {
        at += 1;
    }
    if !line[at..].starts_with(key) {
        return None;
    }
    at += key.len();
    if bytes.get(at) == Some(&b'"') {
        at += 1;
    }
    let gap = at;
    while at < bytes.len() && blank(bytes[at]) {
        at += 1;
    }
    if at == gap {
        return None;
    }
    let prefix = at;
    let quoted = (bytes.get(at) == Some(&b'"'))
        .then(|| line[at + 1..].find('"').map(|close| at + 1 + close + 1))
        .flatten();
    let end = match quoted {
        Some(end) => end,
        None => {
            let rest = &line[at..];
            let len = rest
                .char_indices()
                .find(|(_, c)| c.is_whitespace() || *c == '/')
                .map_or(rest.len(), |(index, _)| index);
            if len == 0 {
                return None;
            }
            at + len
        }
    };
    Some(Entry {
        prefix: line[..prefix].to_string(),
        value: line[prefix..end].to_string(),
        suffix: line[end..].to_string(),
    })
}

fn entry_key(line: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"^[ \t]*"?([A-Za-z_]\w*)"?[ \t]+("[^"]*"|[^\s/]+)"#).expect("entry key regex")
    });
    re.captures(strip_comment(line))
        .map(|caps| caps[1].to_string())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.trim_matches('"'))
}

fn apply_op(content: &str, op: &SectionOp) -> Option<String> {
    let (start, end) = find_section_by_path(content, &op.path)?;
    let mut lines: Vec<String> = content[start..end]
        .split('\n')
        .map(str::to_string)
        .collect();
    let mut edited = false;
    for line in lines.iter_mut() {
        let Some(entry) = match_entry_line(line, &op.key) else {
            continue;
        };
        if op.remove {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            *line = format!("{indent}// {MARKER} removed: {}", line.trim());
        } else {
            let value = op.value.as_deref().unwrap_or_default();
            *line = format!(
                "{}{}{} // {MARKER} was {}",
                entry.prefix,
                quote(value),
                entry.suffix,
                entry.value
            );
        }
        edited = true;
    }
    if edited {
        return Some(format!(
            "{}{}{}",
            &content[..start],
            lines.join("\n"),
            &content[end..]
        ));
    }
    if op.remove {
        return Some(content.to_string());
    }
    let indent = detect_indent(&content[start..end]);
    let value = op.value.as_deref().unwrap_or_default();
    Some(format!(
        "{}\n{indent}{} {} // {MARKER} added{}",
        &content[..start],
        op.key,
        quote(value),
        &content[start..]
    ))
}

fn remove_markers(content: &str) -> String {
    let was =
        Regex::new(&format!(r#"^(.*?) // {MARKER} was ("[^"]*"|\S+)\s*$"#)).expect("was regex");
    let removed =
        Regex::new(&format!(r"^([ \t]*)// {MARKER} removed: (.*)$")).expect("removed regex");
    let mut out = Vec::new();
    for line in content.split('\n') {
        if line.contains(&format!("// {MARKER} added"))
            || line.contains(&format!("[{MARKER}]"))
            || line.contains("Sigil Performance Config BEGIN")
            || line.contains("Sigil Performance Config END")
        {
            continue;
        }
        if let Some(caps) = was.captures(line) {
            let head = &caps[1];
            if let Some(key) = entry_key(head)
                && let Some(entry) = match_entry_line(head, &key)
            {
                out.push(format!("{}{}{}", entry.prefix, &caps[2], entry.suffix));
                continue;
            }
        }
        if let Some(caps) = removed.captures(line) {
            out.push(format!("{}{}", &caps[1], &caps[2]));
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

fn brace_count(content: &str) -> usize {
    content
        .bytes()
        .filter(|byte| *byte == b'{' || *byte == b'}')
        .count()
}

#[cfg(test)]
mod tests;
