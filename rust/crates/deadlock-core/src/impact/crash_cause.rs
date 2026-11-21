//! The exact file a crash was loading, and the mod that supplies it.
//!
//! Behaviour after Deadlock Mod Manager's crash resource attribution (GPL-3.0, behaviour only)
//! and Grimoire's crash evidence (MIT): the game's own words name a resource; the mod that wins
//! that path at load time is the suspect. Three sources are read, strongest first:
//!
//! 1. The dump's comment stream. A Source 2 comment reads like
//!    `error\n\nAbortMessage({FATAL ERROR: Unable to load layout file
//!    'file://{resources}/layout/hud.xml'...})\nUptime( 8.9 )\nAddons: ...\nBuild: 10725 ...`.
//!    The abort message is searched first, then the rest of the comment.
//! 2. The tail of `game/citadel/console.log` (only written with `-condebug`), when it was written
//!    around the crash: error lines about layouts, missing children and resources.
//! 3. When neither names a shipped file, the caller falls back to the newest install
//!    (`super::suspect`).
//!
//! A resource is a game path under `panorama/`, `scripts/`, `models/`, `materials/`,
//! `particles/`, `soundevents/` or `sounds/` with a compiled extension. Panorama sources
//! (`file://{resources}/layout/hud.xml`) are rewritten to the compiled path the mods ship
//! (`panorama/layout/hud.vxml_c`), `.vdata` gets its `_c`, and a path embedded in a longer word
//! or with `.`/`..` segments is not a resource. The winner of a path is the lowest slot that
//! ships it, which is the load order Sigil installs in. Two resources that point at different
//! mods name nobody: the guess must be one mod or none. Pure; the caller does the reading.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

/// How much of console.log's end is read.
pub const CONSOLE_TAIL_BYTES: u64 = 256 * 1024;
/// Console lines kept as evidence.
pub const CONSOLE_LINES: usize = 40;
/// A console.log older than the crash by more than this belongs to another session.
pub const CONSOLE_WINDOW_SECONDS: i64 = 15 * 60;

/// The fields of a Source 2 dump comment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommentFacts {
    /// The first line: `error` or `Crash`.
    pub kind: Option<String>,
    /// The text inside `AbortMessage({ ... })`.
    pub abort: Option<String>,
    pub uptime_seconds: Option<f64>,
    /// The addon names the game listed, as written.
    pub addons: Vec<String>,
    /// `Build: 10725 (revision 10577413)`, after the colon.
    pub build: Option<String>,
}

/// Read the comment's fields. Only the first occurrence of each prefix counts: the console
/// history the game appends later can echo them.
pub fn parse_comment(comment: &str) -> CommentFacts {
    let mut facts = CommentFacts {
        kind: comment.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_string),
        ..CommentFacts::default()
    };
    if let Some(start) = comment.find("AbortMessage({") {
        let rest = &comment[start + "AbortMessage({".len()..];
        let end = rest.find("})").unwrap_or(rest.len());
        let text = rest[..end].trim();
        if !text.is_empty() {
            facts.abort = Some(text.to_string());
        }
    }
    let mut seen_uptime = false;
    let mut seen_addons = false;
    let mut seen_build = false;
    for line in comment.lines().map(str::trim) {
        if !seen_uptime && let Some(rest) = line.strip_prefix("Uptime(") {
            seen_uptime = true;
            facts.uptime_seconds = rest.trim_end_matches(')').trim().parse().ok();
        } else if !seen_addons && let Some(rest) = line.strip_prefix("Addons:") {
            seen_addons = true;
            facts.addons = rest
                .split([',', ' ', '\t'])
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect();
        } else if !seen_build && let Some(rest) = line.strip_prefix("Build:") {
            seen_build = true;
            facts.build = Some(rest.trim().to_string()).filter(|build| !build.is_empty());
        }
    }
    facts
}

fn panorama_source() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"file://\{resources\}/([a-z0-9_./-]+)\.(xml|css|js)\b").unwrap())
}

fn resource() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:panorama|scripts|models|materials|particles|soundevents|sounds)/[a-z0-9_./-]+?\.(?:vxml_c|vcss_c|vjs_c|vdata_c|vdata|vmdl_c|vmat_c|vtex_c|vpcf_c|vsnd_c|vsndevts_c|vanmgrph_c|vnmgraph_c)",
        )
        .unwrap()
    })
}

/// Every resource path a text names, in order, without repeats.
pub fn resources_in(text: &str) -> Vec<String> {
    let lower = text.replace('\\', "/").to_lowercase();
    let rewritten = panorama_source().replace_all(&lower, |caps: &regex::Captures| {
        let ext = match &caps[2] {
            "xml" => "vxml_c",
            "css" => "vcss_c",
            _ => "vjs_c",
        };
        format!("panorama/{}.{ext}", &caps[1])
    });
    let bytes = rewritten.as_bytes();
    let mut out: Vec<String> = Vec::new();
    for found in resource().find_iter(&rewritten) {
        let before = found.start().checked_sub(1).map(|at| bytes[at]);
        if before.is_some_and(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b)) {
            continue;
        }
        let after = bytes.get(found.end()).copied();
        if after.is_some_and(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)) {
            continue;
        }
        let path = found.as_str();
        if path.split('/').any(|segment| segment == "." || segment == ".." || segment.is_empty()) {
            continue;
        }
        let path = if path.ends_with(".vdata") { format!("{path}_c") } else { path.to_string() };
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// Console lines worth showing next to a crash: layout and resource errors, newest first.
pub fn console_evidence(tail: &str) -> Vec<String> {
    let mut lines: Vec<String> = tail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("fatal error")
                || lower.contains("unable to find child")
                || lower.contains("unable to load layout file")
                || lower.contains("failed to load")
                || lower.contains("error loading resource")
                || lower.contains("missing resource")
                || (lower.contains("layout") && (lower.contains("failed") || lower.contains("error")))
        })
        .map(|line| line.chars().take(500).collect())
        .collect();
    lines.reverse();
    lines.dedup();
    lines.truncate(CONSOLE_LINES);
    lines
}

/// The end of console.log, from a line start, when it was written within
/// [`CONSOLE_WINDOW_SECONDS`] of `crash_secs` (0 skips the check).
pub fn read_console_tail(path: &Path, crash_secs: i64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let meta = std::fs::metadata(path).ok()?;
    if crash_secs > 0 {
        let written = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        if (written - crash_secs).abs() > CONSOLE_WINDOW_SECONDS {
            return None;
        }
    }
    let mut file = std::fs::File::open(path).ok()?;
    let len = meta.len();
    let start = len.saturating_sub(CONSOLE_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(CONSOLE_TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default()
    } else {
        text
    })
}

/// Where a named resource was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Dump,
    Console,
}

/// A resource the crash named and the mod that wins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub slot: u16,
    /// The path as the mod's VPK spells it.
    pub file: String,
    pub source: Source,
}

/// The one mod that wins the named resources. `shipped` maps each installed slot to the paths
/// it ships; lower slots win. `None` when no resource is shipped by a mod (the game's own file
/// was loading) or when two resources lead to different mods.
pub fn attribute(resources: &[(String, Source)], shipped: &BTreeMap<u16, Vec<String>>) -> Option<Attribution> {
    let mut found: Option<Attribution> = None;
    for (resource, source) in resources {
        let winner = shipped.iter().find_map(|(slot, paths)| {
            paths
                .iter()
                .find(|path| path.replace('\\', "/").eq_ignore_ascii_case(resource))
                .map(|path| (*slot, path.clone()))
        });
        let Some((slot, file)) = winner else { continue };
        match &found {
            None => found = Some(Attribution { slot, file, source: *source }),
            Some(first) if first.slot == slot => {}
            Some(_) => return None,
        }
    }
    found
}

/// The resources a dump comment and a console tail name, dump first, the abort message first
/// within the dump.
pub fn named_resources(comment: Option<&str>, console: &[String]) -> Vec<(String, Source)> {
    let mut out: Vec<(String, Source)> = Vec::new();
    let mut push = |path: String, source: Source| {
        if !out.iter().any(|(seen, _)| *seen == path) {
            out.push((path, source));
        }
    };
    if let Some(comment) = comment {
        let facts = parse_comment(comment);
        if let Some(abort) = &facts.abort {
            for path in resources_in(abort) {
                push(path, Source::Dump);
            }
        }
        for path in resources_in(comment) {
            push(path, Source::Dump);
        }
    }
    for line in console {
        for path in resources_in(line) {
            push(path, Source::Console);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMENT: &str = "error\n\nAbortMessage({FATAL ERROR: Unable to load layout file 'file://{resources}/layout/hud.xml'.  This may indicate a problem with your local install files\n})\nUptime( 8.929353 )\nAddons: pak03_dir, pak07_dir\nCommand Line: \"deadlock.exe\" -steam\nBuild: 10725 (revision 10577413)\nUptime( 99 )\n";

    #[test]
    fn the_comment_fields_come_from_their_first_line() {
        let facts = parse_comment(COMMENT);
        assert_eq!(facts.kind.as_deref(), Some("error"));
        assert!(facts.abort.as_deref().unwrap().starts_with("FATAL ERROR: Unable to load layout file"));
        assert_eq!(facts.uptime_seconds, Some(8.929353));
        assert_eq!(facts.addons, vec!["pak03_dir", "pak07_dir"]);
        assert_eq!(facts.build.as_deref(), Some("10725 (revision 10577413)"));
    }

    #[test]
    fn resources_are_rewritten_and_bounded() {
        assert_eq!(resources_in(COMMENT), vec!["panorama/layout/hud.vxml_c"]);
        assert_eq!(resources_in("Fatal error: 'Scripts\\Test.vdata'"), vec!["scripts/test.vdata_c"]);
        assert!(resources_in("EXCEPTION_ACCESS_VIOLATION reading address 0x0").is_empty());
        assert!(resources_in("x.vmat_c.bak materials/a.vmat_c.bak").is_empty(), "a longer name is not a resource");
        assert!(resources_in("c:/materials/a.vmat_c foo/models/b.vmdl_c").is_empty(), "embedded paths are not resources");
        assert!(resources_in("models/../x.vmdl_c").is_empty());
        assert_eq!(
            resources_in("load models/heroes_wip/haze/haze.vmdl_c then models/heroes_wip/haze/haze.vmdl_c"),
            vec!["models/heroes_wip/haze/haze.vmdl_c"]
        );
        assert_eq!(resources_in("file://{resources}/styles/hud.css"), vec!["panorama/styles/hud.vcss_c"]);
    }

    #[test]
    fn console_evidence_keeps_error_lines_newest_first() {
        let tail = "boot\nUnable to find child 'NewPanel'\nfine\nFailed to load panorama/layout/hud.vxml_c\nUnable to find child 'NewPanel'\n";
        let lines = console_evidence(tail);
        assert_eq!(lines[0], "Unable to find child 'NewPanel'");
        assert_eq!(lines[1], "Failed to load panorama/layout/hud.vxml_c");
        assert_eq!(lines.len(), 3);
    }

    fn shipped() -> BTreeMap<u16, Vec<String>> {
        BTreeMap::from([
            (3, vec!["panorama/layout/HUD.vxml_c".to_string()]),
            (7, vec!["panorama/layout/hud.vxml_c".to_string(), "models/x.vmdl_c".to_string()]),
            (9, vec!["models/y.vmdl_c".to_string()]),
        ])
    }

    #[test]
    fn the_lowest_slot_wins_a_named_path() {
        let named = named_resources(Some(COMMENT), &[]);
        let hit = attribute(&named, &shipped()).unwrap();
        assert_eq!((hit.slot, hit.file.as_str(), hit.source), (3, "panorama/layout/HUD.vxml_c", Source::Dump));
    }

    #[test]
    fn two_mods_named_is_nobody() {
        let named = vec![("models/x.vmdl_c".to_string(), Source::Dump), ("models/y.vmdl_c".to_string(), Source::Dump)];
        assert_eq!(attribute(&named, &shipped()), None);
        let named = vec![("panorama/layout/other.vxml_c".to_string(), Source::Dump), ("models/y.vmdl_c".to_string(), Source::Console)];
        assert_eq!(attribute(&named, &shipped()).map(|hit| (hit.slot, hit.source)), Some((9, Source::Console)));
    }

    #[test]
    fn the_console_counts_only_after_the_dump() {
        let console = vec!["Failed to load models/y.vmdl_c".to_string()];
        let named = named_resources(Some("error\nAbortMessage({FATAL ERROR: nothing})"), &console);
        assert_eq!(named, vec![("models/y.vmdl_c".to_string(), Source::Console)]);
    }

    #[test]
    fn the_advisory_prefers_the_dump_then_the_console_then_the_newest_install() {
        use crate::impact::{suspect_with_console, DumpFacts, Reason};
        let names = BTreeMap::from([(3, "Three".to_string()), (7, "Seven".to_string()), (9, "Nine".to_string())]);
        let facts = |comment: &str| DumpFacts {
            path: "d.mdmp".into(),
            modified_secs: Some(10),
            exception: None,
            comment: Some(comment.to_string()),
        };
        let console = vec!["Failed to load models/y.vmdl_c".to_string()];
        let hit = suspect_with_console(&facts(COMMENT), &console, &[], &names, &shipped()).unwrap();
        assert_eq!((hit.slot, hit.reason), (3, Reason::DumpNamesFile));
        let hit = suspect_with_console(&facts("error\nAbortMessage({FATAL ERROR})"), &console, &[], &names, &shipped()).unwrap();
        assert_eq!((hit.slot, hit.reason, hit.file.as_deref()), (9, Reason::ConsoleNamesFile, Some("models/y.vmdl_c")));
        assert!(hit.reason.line(hit.file.as_deref(), None, 0).contains("console"));
        assert_eq!(suspect_with_console(&facts("error"), &[], &[], &names, &shipped()), None, "no install to fall back on");
    }

    #[test]
    fn an_old_console_log_is_not_read() {
        let dir = std::env::temp_dir().join(format!("sigil-console-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("console.log");
        std::fs::write(&path, "a\nb\n").unwrap();
        assert_eq!(read_console_tail(&path, 0).as_deref(), Some("a\nb\n"));
        assert_eq!(read_console_tail(&path, 1), None, "written long after 1970");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
