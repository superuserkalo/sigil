//! Turn an upstream `gameinfo.gi` into what Sigil writes.
//!
//! Upstream configs ship a whole `gameinfo.gi`. Replacing the player's file with
//! one would drop their mod search paths and any change a game update made, so
//! the config is diffed against the stock file instead and classified into:
//!
//! - engine section edits (changed, added or commented-out keys outside ConVars),
//! - performance convars,
//! - opt-ins: the author's visibility, camera and developer-tool convars, each a
//!   separate switch,
//! - held back: keys Sigil never writes, with the reason.
//!
//! Ported from Grimoire's `performancePresetGen.ts` (MIT). Nothing here touches
//! the network or the disk.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::SectionOp;
use super::classify::{self, Group};

#[derive(Debug, Clone)]
struct ParsedEntry {
    path: Vec<String>,
    key: String,
    value: String,
    order: usize,
}

/// The active entries of a KeyValues file and the keys it comments out.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    entries: HashMap<String, ParsedEntry>,
    commented: Vec<String>,
}

impl Parsed {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value at `path/key`, as the engine reads it.
    pub fn value(&self, id: &str) -> Option<&str> {
        self.entries.get(id).map(|entry| entry.value.as_str())
    }
}

/// An author convar offered as its own switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptIn {
    pub key: String,
    /// What the author set.
    pub value: String,
    pub group: Group,
    /// What the stock file sets, when it sets it.
    #[serde(default)]
    pub stock: Option<String>,
}

/// A key the config sets that Sigil does not write, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    pub key: String,
    /// Where it sits: a section path, `ConVars`, or `video.txt`.
    pub place: String,
    pub reason: String,
}

/// What a config writes, generated from its upstream file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    pub section_ops: Vec<SectionOp>,
    pub convars: Vec<(String, String)>,
    pub opt_ins: Vec<OptIn>,
    pub held: Vec<Held>,
}

const UNCLASSIFIED: &str =
    "Looks like it changes what you can see, and Sigil has not reviewed it yet.";
const MISPLACED: &str =
    "A visibility or camera setting outside ConVars, where the switch cannot reach it.";

fn entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^"?([A-Za-z_][\w.]*)"?[ \t]+("[^"]*"|[^\s/]+)"#).unwrap())
}

fn entry_shape_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^"?[A-Za-z_][\w.]*"?[ \t]+\S"#).unwrap())
}

/// A commented-out entry such as `//GpuImplicitRendererManifest "1"`. Tight on
/// purpose: these files are full of prose comments.
fn commented_entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)^"?([A-Za-z_][\w.]*)"?[ \t]+(?:"([^"]*)"|(-?[0-9.]+|true|false))[ \t]*(?://.*)?$"#,
        )
        .unwrap()
    })
}

/// Parse KeyValues into active entries keyed by `path/key`, with paths relative
/// to the inside of the root `GameInfo` block. Later duplicates win: that is
/// the value the engine ends up with.
pub fn parse(text: &str) -> Parsed {
    let mut entries: HashMap<String, ParsedEntry> = HashMap::new();
    let mut commented = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    let mut order = 0usize;

    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let comment_at = raw.find("//");
        let line = match comment_at {
            Some(at) => raw[..at].trim(),
            None => raw.trim(),
        };
        if let Some(at) = comment_at
            && line.is_empty()
            && let Some(caps) = commented_entry_re().captures(raw[at + 2..].trim())
        {
            let mut id: Vec<String> = stack.iter().skip(1).cloned().collect();
            id.push(caps[1].to_string());
            let id = id.join("/");
            if !commented.contains(&id) {
                commented.push(id);
            }
        }
        if line.is_empty() {
            continue;
        }
        let mut rest = line.to_string();
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix('{') {
                stack.push(pending.take().unwrap_or_default());
                rest = after.trim().to_string();
                continue;
            }
            if let Some(after) = rest.strip_prefix('}') {
                stack.pop();
                rest = after.trim().to_string();
                continue;
            }
            let cut = [rest.find('{'), rest.find('}')].into_iter().flatten().min();
            let chunk = match cut {
                Some(at) => rest[..at].trim().to_string(),
                None => rest.trim().to_string(),
            };
            rest = match cut {
                Some(at) => rest[at..].to_string(),
                None => String::new(),
            };
            if chunk.is_empty() {
                continue;
            }
            match entry_re().captures(&chunk) {
                Some(caps) if entry_shape_re().is_match(&chunk) => {
                    let path: Vec<String> = stack.iter().skip(1).cloned().collect();
                    let key = caps[1].to_string();
                    let value = caps[2].trim_matches('"').to_string();
                    let mut id = path.clone();
                    id.push(key.clone());
                    let id = id.join("/");
                    let first = entries.get(&id).map(|entry| entry.order);
                    let entry_order = first.unwrap_or_else(|| {
                        order += 1;
                        order
                    });
                    entries.insert(
                        id,
                        ParsedEntry {
                            path,
                            key,
                            value,
                            order: entry_order,
                        },
                    );
                }
                _ => pending = Some(chunk.trim_matches('"').to_string()),
            }
        }
    }
    Parsed { entries, commented }
}

fn is_convars(path: &[String]) -> bool {
    path.len() == 1 && path[0] == "ConVars"
}

/// Root keys carry an empty path; the patcher needs a real section.
fn op_path(path: &[String]) -> Vec<String> {
    if path.is_empty() {
        vec!["GameInfo".to_string()]
    } else {
        path.to_vec()
    }
}

fn place(path: &[String]) -> String {
    if path.is_empty() {
        "GameInfo".to_string()
    } else {
        path.join("/")
    }
}

/// Diff `config` against `baseline` and classify the result. Never fails on
/// content: what it cannot place safely is held back and listed.
pub fn generate(baseline: &Parsed, config: &Parsed) -> Body {
    let mut body = Body::default();
    let mut held_sections: Vec<(Vec<String>, usize)> = Vec::new();

    let mut ordered: Vec<&ParsedEntry> = config.entries.values().collect();
    ordered.sort_by_key(|entry| entry.order);
    for entry in ordered {
        let id = id_of(&entry.path, &entry.key);
        let base = baseline.entries.get(&id);
        if base.is_some_and(|base| base.value == entry.value) {
            continue;
        }
        if classify::section_excluded(&entry.path) {
            let top: Vec<String> = entry.path.iter().take(2).cloned().collect();
            match held_sections.iter_mut().find(|(path, _)| *path == top) {
                Some((_, count)) => *count += 1,
                None => held_sections.push((top, 1)),
            }
            continue;
        }
        if let Some(reason) = classify::key_excluded(&entry.key) {
            body.held.push(Held {
                key: entry.key.clone(),
                place: place(&entry.path),
                reason: reason.to_string(),
            });
            continue;
        }
        if let Some(group) = classify::opt_in_group(&entry.key) {
            if !is_convars(&entry.path) {
                body.held.push(Held {
                    key: entry.key.clone(),
                    place: place(&entry.path),
                    reason: MISPLACED.to_string(),
                });
                continue;
            }
            body.opt_ins.push(OptIn {
                key: entry.key.clone(),
                value: entry.value.clone(),
                group,
                stock: base.map(|base| base.value.clone()),
            });
            continue;
        }
        if classify::unclassified(&entry.key) {
            body.held.push(Held {
                key: entry.key.clone(),
                place: place(&entry.path),
                reason: UNCLASSIFIED.to_string(),
            });
            continue;
        }
        if is_convars(&entry.path) {
            body.convars.push((entry.key.clone(), entry.value.clone()));
        } else {
            body.section_ops.push(SectionOp {
                path: op_path(&entry.path),
                key: entry.key.clone(),
                value: Some(entry.value.clone()),
                remove: false,
            });
        }
    }

    // A stock key the config comments out is a deliberate removal. Absence is
    // not: several configs are partial files. ConVars is additive, so a
    // commented convar there is ignored.
    for id in &config.commented {
        let Some(base) = baseline.entries.get(id) else {
            continue;
        };
        if classify::section_excluded(&base.path)
            || classify::key_excluded(&base.key).is_some()
            || is_convars(&base.path)
            || classify::opt_in_group(&base.key).is_some()
            || config.entries.contains_key(id)
        {
            continue;
        }
        if classify::unclassified(&base.key) {
            body.held.push(Held {
                key: base.key.clone(),
                place: place(&base.path),
                reason: UNCLASSIFIED.to_string(),
            });
            continue;
        }
        body.section_ops.push(SectionOp {
            path: op_path(&base.path),
            key: base.key.clone(),
            value: None,
            remove: true,
        });
    }

    body.section_ops.sort_by(|a, b| {
        a.path
            .join("/")
            .cmp(&b.path.join("/"))
            .then(a.remove.cmp(&b.remove))
            .then(a.key.cmp(&b.key))
    });

    for (path, count) in held_sections {
        body.held.push(Held {
            key: path.join("/"),
            place: format!("{count} setting{}", if count == 1 { "" } else { "s" }),
            reason: classify::section_reason(&path).to_string(),
        });
    }
    body
}

fn id_of(path: &[String], key: &str) -> String {
    let mut id = path.to_vec();
    id.push(key.to_string());
    id.join("/")
}

// --- video.txt ---------------------------------------------------------------

fn video_entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^[ \t]*"setting\.([A-Za-z_]\w*)"[ \t]+"([^"\r\n]*)""#).unwrap())
}

/// `setting.*` entries as `(convar, value)`, first-seen order, later
/// duplicates winning.
pub fn parse_video(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let Some(caps) = video_entry_re().captures(line.trim_end_matches('\r')) else {
            continue;
        };
        match out.iter_mut().find(|(key, _)| key == &caps[1]) {
            Some(slot) => slot.1 = caps[2].to_string(),
            None => out.push((caps[1].to_string(), caps[2].to_string())),
        }
    }
    out
}

/// The video settings Sigil would write: display settings (resolution, window
/// mode, refresh, gamma, frame cap) stay the player's, excluded keys and opt-in
/// convars are dropped, and capped values are lowered. `verbatim` skips the caps
/// for a source whose author asks for their values as shipped.
pub fn generate_video(
    settings: &[(String, String)],
    verbatim: bool,
) -> (Vec<(String, String)>, Vec<Held>) {
    let mut out = Vec::new();
    let mut held = Vec::new();
    for (key, value) in settings {
        // A display setting is the player's choice, not the config's. Even a
        // verbatim source does not get to change the resolution or frame cap.
        if classify::video_display_key(key) {
            continue;
        }
        if classify::key_excluded(key).is_some() || classify::opt_in_group(key).is_some() {
            continue;
        }
        if classify::unclassified(key) {
            held.push(Held {
                key: key.clone(),
                place: "video.txt".to_string(),
                reason: UNCLASSIFIED.to_string(),
            });
            continue;
        }
        let cap = if verbatim {
            None
        } else {
            classify::VIDEO_MAX
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, max)| *max)
        };
        let value = match (cap, value.parse::<f64>()) {
            (Some(max), Ok(number)) if number > max => format_number(max),
            _ => value.clone(),
        };
        out.push((key.clone(), value));
    }
    (out, held)
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

// --- sanity gates for fetched text ---------------------------------------------

/// Structural sanity for a fetched `gameinfo.gi`. A failure is a truncated
/// download, an HTML error page, or an upstream rewrite a person should look at.
pub fn validate_gameinfo(text: &str) -> Result<(), String> {
    if text.len() < 10_000 {
        return Err(format!("is too small ({} bytes)", text.len()));
    }
    if text.len() > 2_000_000 {
        return Err(format!("is too large ({} bytes)", text.len()));
    }
    if !text.contains("GameInfo") {
        return Err("has no GameInfo block".into());
    }
    if !text.contains("ConVars") {
        return Err("has no ConVars block".into());
    }
    let mut depth: i64 = 0;
    for raw in text.split('\n') {
        let line = match raw.find("//") {
            Some(at) => &raw[..at],
            None => raw,
        };
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                return Err("has unbalanced braces".into());
            }
        }
    }
    if depth != 0 {
        return Err("has unbalanced braces".into());
    }
    Ok(())
}

/// Sanity for a generated body against the parses it came from.
pub fn validate_body(baseline: &Parsed, config: &Parsed, body: &Body) -> Result<(), String> {
    if baseline.len() < 150 {
        return Err(format!(
            "stock file parsed to only {} entries",
            baseline.len()
        ));
    }
    if config.len() < 100 {
        return Err(format!("config parsed to only {} entries", config.len()));
    }
    if body.convars.len() > 1500 {
        return Err(format!(
            "{} convars is more than any real config sets",
            body.convars.len()
        ));
    }
    if body.section_ops.len() > 600 {
        return Err(format!(
            "{} section edits is more than any real config makes",
            body.section_ops.len()
        ));
    }
    Ok(())
}

pub fn validate_video(text: &str) -> Result<(), String> {
    if text.len() > 64_000 {
        return Err(format!("video.txt is too large ({} bytes)", text.len()));
    }
    let count = parse_video(text).len();
    if !(5..=400).contains(&count) {
        return Err(format!("video.txt has {count} settings"));
    }
    Ok(())
}
