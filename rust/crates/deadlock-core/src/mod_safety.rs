//! What a mod's VPK ships besides game assets: programs, loose scripts, script-mod data and
//! Panorama code that reaches the network.
//!
//! The rules, and how Grimoire and Deadlock Mod Manager compare, are in
//! `docs/design/mod-safety.md`. In short: anything the game itself ships in the same form
//! (compiled images, layouts, styles and scripts) is a normal asset and is never listed for
//! what it is. A file is listed only for a concrete reason, and [`Finding::reason`] says which:
//!
//! - a native program (`exe`, `dll` and similar), which a mod never needs;
//! - a source script (`js`, `lua`, `cfg` and similar) the game does not run as shipped;
//! - a script-mod file, by the same rule the install check uses ([`payload::script_mod_kind`]);
//! - a Panorama layout, style or script whose text calls a web or browser API, loads a file from
//!   the internet, or builds code at runtime.
//!
//! A URL alone is not a finding: every SVG and many layouts name `http://www.w3.org/...` as an
//! XML namespace, and licence comments carry links. A URL load (`src=`, `href=`, `url=`, CSS
//! `url()`, `SetImage`) is listed wherever it appears, comments included: the scan fails closed
//! rather than guess where a comment ends. A layout, style or script too large or too broken to
//! read is listed as [`Risk::Unchecked`], never skipped.
//! Images (`vsvg_c`, `svg`, `vtex_c`, `png`) are never read. It is a warning, not a block:
//! nothing here stops an install.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::payload::{self, ScriptModKind};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with, read_vpk_entry_data};

/// Text read from one entry before the search stops. A layout or script is far smaller.
pub const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// Findings kept in a report. The rest are counted in `more`.
pub const MAX_FINDINGS: usize = 40;
/// Findings kept for one file.
pub const MAX_PER_FILE: usize = 3;

/// Why an entry is worth a look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Risk {
    /// A native program: `exe`, `dll`, `so`, `dylib`, `bat`, `cmd`, `ps1`, `msi`, `wasm`.
    Program,
    /// A source script or console config the game does not run as shipped: `js`, `vjs`, `ts`,
    /// `vts`, `lua`, `nut`, `cfg`.
    Script,
    /// A Panorama layout, style or script that calls a web or browser API or loads a URL.
    Remote,
    /// A Panorama script that builds code at runtime (`eval`, `new Function`, `atob`).
    Dynamic,
    /// A file the install check calls a script mod: a skeleton or gameplay data.
    ScriptMod,
    /// A Panorama layout, style or script the scan could not check: too large or unreadable.
    /// Listed so a file cannot pass by being big or broken.
    Unchecked,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Program => "Program",
            Risk::Script => "Source script",
            Risk::Remote => "Reaches out",
            Risk::Dynamic => "Hidden code",
            Risk::ScriptMod => "Script mod",
            Risk::Unchecked => "Not checked",
        }
    }

    /// One line on what it means for the player, when the finding has no more exact reason.
    pub fn line(self) -> &'static str {
        match self {
            Risk::Program => "A program. Deadlock never runs it, and a mod has no use for one.",
            Risk::Script => "Script source the game does not run: it only runs compiled Panorama scripts.",
            Risk::Remote => "Opens a web page or loads something from the internet while the game runs.",
            Risk::Dynamic => "Builds code while it runs, so what it does cannot be read ahead.",
            Risk::ScriptMod => "Script mod: Settings > Allow script mods covers it.",
            Risk::Unchecked => "Sigil could not read this file, so it cannot say what it does.",
        }
    }

    /// Higher sorts first in the list.
    pub fn rank(self) -> u8 {
        match self {
            Risk::Program => 5,
            Risk::Remote => 4,
            Risk::Dynamic | Risk::Unchecked => 3,
            Risk::Script => 2,
            Risk::ScriptMod => 1,
        }
    }
}

/// One thing the scan did not expect to find.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Path inside the VPK, as shipped.
    pub path: String,
    pub risk: Risk,
    /// What matched: an API name such as `$.AsyncWebRequest`, or the URL a layout loads. Empty
    /// for a file listed for its type alone.
    pub call: String,
}

impl Finding {
    /// The sentence that says exactly why this file is listed.
    pub fn reason(&self) -> String {
        if self.risk == Risk::Unchecked {
            return match self.call.as_str() {
                TOO_LARGE => format!(
                    "Too large to check: over {} MB. A layout or script this big is unusual, and Sigil cannot say what it does.",
                    MAX_TEXT_BYTES / (1024 * 1024)
                ),
                _ => "Could not be read from the VPK, so Sigil cannot say what it does.".to_string(),
            };
        }
        if !self.call.is_empty() {
            if let Some(signal) = SIGNALS.iter().find(|signal| signal.call == self.call) {
                return signal.why.to_string();
            }
            if self.risk == Risk::Remote {
                return format!("Loads {} from the internet when the game shows it.", self.call);
            }
        }
        match self.risk {
            Risk::Script if extension(&self.path) == "cfg" => {
                "Console config: the game runs its commands when a bind or another config executes it.".to_string()
            }
            Risk::Script => {
                "Script source the game does not run (it only runs compiled Panorama scripts). Unusual in a mod."
                    .to_string()
            }
            Risk::ScriptMod => payload::script_mod_kind(&self.path)
                .map(ScriptModKind::message)
                .unwrap_or(Risk::ScriptMod.line())
                .to_string(),
            risk => risk.line().to_string(),
        }
    }
}

/// What one mod's VPK holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub findings: Vec<Finding>,
    /// Findings beyond [`MAX_FINDINGS`].
    pub more: usize,
    /// Entries read from the VPK directory.
    pub entries: usize,
    /// The VPK could not be read.
    pub error: Option<String>,
}

impl ScanReport {
    /// Nothing unusual to show.
    pub fn is_clear(&self) -> bool {
        self.findings.is_empty() && self.error.is_none()
    }

    /// The worst risk found, if any.
    pub fn worst(&self) -> Option<Risk> {
        self.findings
            .iter()
            .map(|finding| finding.risk)
            .max_by_key(|risk| risk.rank())
    }

    /// Files to warn about on the card, at most `count`.
    pub fn paths(&self, count: usize) -> Vec<&str> {
        let mut seen = std::collections::BTreeSet::new();
        self.findings
            .iter()
            .filter(|finding| seen.insert(finding.path.as_str()))
            .map(|finding| finding.path.as_str())
            .take(count)
            .collect()
    }
}

/// What an install that should land switched on may do after [`scan_vpk`].
///
/// The decision is the scan's own rule, not the screen's: a report with any finding, or one
/// Sigil could not finish, keeps the install off. The caller applies a trust or the "Allow
/// script mods" setting before asking, so this cannot be talked into landing a flagged install
/// on by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Nothing to ask about: the install may be switched on now.
    On,
    /// The scan found something to ask about, so the install stays off with these findings.
    Off {
        /// The findings the review lists, worst first.
        findings: Vec<Finding>,
        /// Findings past [`MAX_FINDINGS`] that the review only counts.
        more: usize,
        /// Set when the whole VPK could not be read. Being unreadable is no way past the gate.
        error: Option<String>,
    },
}

impl Landing {
    /// True when the install stays switched off.
    pub fn is_off(&self) -> bool {
        matches!(self, Landing::Off { .. })
    }

    /// The findings that keep it off, empty when it may land on.
    pub fn findings(&self) -> &[Finding] {
        match self {
            Landing::On => &[],
            Landing::Off { findings, .. } => findings,
        }
    }

    /// The reason the VPK could not be read, when that is why it stays off.
    pub fn error(&self) -> Option<&str> {
        match self {
            Landing::On => None,
            Landing::Off { error, .. } => error.as_deref(),
        }
    }
}

/// Decide whether an install that should land switched on may: only a clear scan goes on.
pub fn landing(report: &ScanReport) -> Landing {
    if report.is_clear() {
        return Landing::On;
    }
    Landing::Off {
        findings: report.findings.clone(),
        more: report.more,
        error: report.error.clone(),
    }
}

/// [`landing`] for the file as written in the addon folder. Reads the VPK; run off the UI thread.
pub fn landing_at(path: &Path) -> Landing {
    landing(&scan_vpk(path))
}

/// `call` of an [`Risk::Unchecked`] finding for a file over [`MAX_TEXT_BYTES`].
pub const TOO_LARGE: &str = "too large";
/// `call` of an [`Risk::Unchecked`] finding for a file whose bytes could not be read.
pub const UNREADABLE: &str = "unreadable";

/// One API or construct worth a finding when it appears in Panorama text.
struct Signal {
    /// Shown as the finding's `call`.
    call: &'static str,
    pattern: &'static str,
    risk: Risk,
    why: &'static str,
}

/// Panorama's web and browser surface, from the game's own binaries (GameTracking-Deadlock:
/// `panorama_strings.txt`, `client_strings.txt`) plus the browser APIs a layout script could
/// reach. See `docs/design/mod-safety.md` for the evidence behind each name.
const SIGNALS: [Signal; 14] = [
    Signal {
        call: "$.AsyncWebRequest",
        pattern: r"\bAsyncWebRequest\b",
        risk: Risk::Remote,
        why: "Sends a web request with $.AsyncWebRequest, Panorama's network call. Current game builds turn it off, but it is there to reach a server.",
    },
    Signal {
        call: "SetURL",
        pattern: r"\bSetURL(?:WithParams)?\s*\(",
        risk: Risk::Remote,
        why: "Points a browser panel at a web address (SetURL).",
    },
    Signal {
        call: "browser panel",
        pattern: r#"<\s*(?:CitadelHTMLPanel|HTML)\b|\bCreatePanel\w*\s*\(\s*["'](?:CitadelHTMLPanel|HTML)["']"#,
        risk: Risk::Remote,
        why: "Embeds a web browser panel in the game UI.",
    },
    Signal {
        call: "ExternalBrowserGoToURL",
        pattern: r"\b(?:External)?BrowserGoToURL\b",
        risk: Risk::Remote,
        why: "Opens a web page in the Steam overlay or your browser (ExternalBrowserGoToURL).",
    },
    Signal {
        call: "OpenURL",
        pattern: r"\bOpen(?:External)?(?:Browser)?URL\b",
        risk: Risk::Remote,
        why: "Opens a web page (OpenURL).",
    },
    Signal {
        call: "ExecuteSteamURL",
        pattern: r"\bExecuteSteamURL\b",
        risk: Risk::Remote,
        why: "Runs a steam:// link (ExecuteSteamURL), which can open pages or start actions in Steam.",
    },
    Signal {
        call: "fetch",
        pattern: r"\bfetch\s*\(",
        risk: Risk::Remote,
        why: "Requests a web address (fetch).",
    },
    Signal {
        call: "XMLHttpRequest",
        pattern: r"\bXMLHttpRequest\b",
        risk: Risk::Remote,
        why: "Requests a web address (XMLHttpRequest).",
    },
    Signal {
        call: "WebSocket",
        pattern: r"\bWebSocket\b",
        risk: Risk::Remote,
        why: "Opens a live connection to a server (WebSocket).",
    },
    Signal {
        call: "importScripts",
        pattern: r"\bimportScripts\s*\(",
        risk: Risk::Remote,
        why: "Loads script code from elsewhere (importScripts).",
    },
    Signal {
        call: "javascript:",
        pattern: r"(?i)\bjavascript\s*:",
        risk: Risk::Remote,
        why: "Uses a javascript: link, which runs code from an address.",
    },
    Signal {
        call: "eval",
        pattern: r#"\beval\s*\(|\bnew\s+Function\s*\(|\bFunction\s*\(\s*["'`]"#,
        risk: Risk::Dynamic,
        why: "Builds code from text while it runs (eval or new Function), so what it does cannot be read ahead.",
    },
    Signal {
        call: "atob",
        pattern: r"\batob\s*\(|\bfromCharCode\s*\(",
        risk: Risk::Dynamic,
        why: "Decodes hidden text while it runs (atob or fromCharCode), a common way to hide a web address or code.",
    },
    Signal {
        call: "RunScriptInPanelContext",
        pattern: r"\bRunScriptInPanelContext\b",
        risk: Risk::Dynamic,
        why: "Runs code inside another panel (RunScriptInPanelContext).",
    },
];

static SIGNAL_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    SIGNALS
        .iter()
        .map(|signal| Regex::new(signal.pattern).expect("signal pattern"))
        .collect()
});

/// A layout attribute, a stylesheet `url()` or an image call that loads from the internet. The
/// game fetches these (Panorama's `CLoadFileURLTask`). `file://{images}/...` and other game
/// roots are local and do not match, and the attribute anchor keeps `xmlns=`, `xlink:href` and
/// doctype URLs out. The URL is captured for the message. Comments are not exempt: a small
/// tokenizer cannot read comments the way the game's parsers do, so a commented-out load is
/// listed rather than risk hiding a real one.
static URL_LOADS: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(
        r#"(?i)(?:(?:^|[\s<;{])(?:src|url|href)\s*=\s*["']|\burl\(\s*["']?|\bSetImage\s*\(\s*["'])\s*((?:https?:)?//[^"'\s)<>]{1,200})"#,
    )
    .expect("url pattern")
});

fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The risk of shipping a path at all, by name. `None` is a game asset, or a Panorama text file
/// that is listed only when [`scan_text`] finds a reason in it.
pub fn risk_of_path(path: &str) -> Option<Risk> {
    let ext = extension(path);
    match ext.as_str() {
        "exe" | "dll" | "com" | "bat" | "cmd" | "ps1" | "vbs" | "lnk" | "msi" | "so" | "dylib"
        | "wasm" => Some(Risk::Program),
        "js" | "vjs" | "ts" | "vts" | "lua" | "nut" | "cfg" => Some(Risk::Script),
        _ => payload::script_mod_kind(path).map(|_| Risk::ScriptMod),
    }
}

/// Panorama layouts, styles and scripts the game runs, compiled or as text. Their bytes are
/// searched; compiled ones keep their source (or its strings) in the DATA block. Images
/// (`vsvg_c`, `svg`, `vsvg`) are not here: an icon has nothing to run.
fn is_panorama_text(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "vjs_c" | "vts_c" | "vxml_c" | "vcss_c" | "xml" | "vxml" | "css" | "vcss" | "html" | "htm"
    )
}

/// Search one Panorama text body. Returns at most [`MAX_PER_FILE`] `(call, risk)` pairs, worst
/// first.
pub fn scan_text(body: &[u8]) -> Vec<(String, Risk)> {
    let text = String::from_utf8_lossy(body);
    let mut found: Vec<(String, Risk)> = SIGNALS
        .iter()
        .zip(SIGNAL_PATTERNS.iter())
        .filter(|(_, pattern)| pattern.is_match(&text))
        .map(|(signal, _)| (signal.call.to_string(), signal.risk))
        .collect();
    // Every URL load counts, inside a comment or not: fail closed.
    if let Some(load) = URL_LOADS.captures(body).and_then(|caps| caps.get(1)) {
        found.push((String::from_utf8_lossy(load.as_bytes()).into_owned(), Risk::Remote));
    }
    found.sort_by_key(|(_, risk)| std::cmp::Reverse(risk.rank()));
    found.truncate(MAX_PER_FILE);
    found
}

/// Scan a directory VPK. Reads only the VPK and its chunk files.
pub fn scan_vpk(path: &Path) -> ScanReport {
    // Preload bytes are kept: a small layout can live entirely in the directory tree.
    let directory = match read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: true }) {
        Ok(directory) => directory,
        Err(error) => {
            return ScanReport {
                error: Some(error.to_string()),
                ..ScanReport::default()
            };
        }
    };
    let mut findings = Vec::new();
    let mut more = 0;
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();

    for entry in &directory.entries {
        if seen.insert(entry.path.as_str(), ()).is_some() {
            continue;
        }
        if let Some(risk) = risk_of_path(&entry.path) {
            push_finding(&mut findings, &mut more, Finding {
                path: entry.path.clone(),
                risk,
                call: String::new(),
            });
            continue;
        }
        if !is_panorama_text(&entry.path) {
            continue;
        }
        // A file the scan cannot read is listed, never skipped: size or damage must not be a
        // way past it.
        let unchecked = |call: &str| Finding {
            path: entry.path.clone(),
            risk: Risk::Unchecked,
            call: call.to_string(),
        };
        if u64::from(entry.total_length) > MAX_TEXT_BYTES {
            push_finding(&mut findings, &mut more, unchecked(TOO_LARGE));
            continue;
        }
        let Ok(data) = read_vpk_entry_data(&directory, entry) else {
            push_finding(&mut findings, &mut more, unchecked(UNREADABLE));
            continue;
        };
        for (call, risk) in scan_text(&data) {
            push_finding(&mut findings, &mut more, Finding {
                path: entry.path.clone(),
                risk,
                call,
            });
        }
    }

    findings.sort_by_key(|finding| std::cmp::Reverse(finding.risk.rank()));
    ScanReport {
        findings,
        more,
        entries: directory.entries.len(),
        error: None,
    }
}

fn push_finding(list: &mut Vec<Finding>, more: &mut usize, finding: Finding) {
    if list.len() >= MAX_FINDINGS {
        *more += 1;
    } else {
        list.push(finding);
    }
}

/// The scan result for a slot file, cached by size and modified time in the screen, not here.
pub fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

// --- the review list (Grimoire #592) -------------------------------------------------------

/// Trusts and the review list: one place that lists every flagged mod as Review or Allowed,
/// with a count for the "N mods need attention" notice and the mods Allow N script mods covers.
/// A trust is a version key (the screen builds it from the mod and its file size), so a mod
/// that changes asks again. Taking a trust back is [`revoke`]; the screen turns the mod off
/// first, so a mod is never left on without one.
pub mod review {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{Risk, ScanReport};

    /// Read the trusted version keys. A missing or broken file trusts nothing.
    pub fn read_trusted(path: &Path) -> BTreeSet<String> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .map(|list| list.into_iter().collect())
            .unwrap_or_default()
    }

    /// Write the trusted version keys, sorted, through a file beside the final name.
    pub fn write_trusted(path: &Path, keys: &BTreeSet<String>) -> crate::error::Result<()> {
        let list: Vec<&String> = keys.iter().collect();
        let text = serde_json::to_string_pretty(&list)?;
        crate::util::write_atomic(path, text.as_bytes())
    }

    /// Add one trust. True when it was new.
    pub fn trust(path: &Path, key: &str) -> crate::error::Result<bool> {
        let mut keys = read_trusted(path);
        let fresh = keys.insert(key.to_string());
        if fresh {
            write_trusted(path, &keys)?;
        }
        Ok(fresh)
    }

    /// Take one trust back (Disallow). True when it was there.
    pub fn revoke(path: &Path, key: &str) -> crate::error::Result<bool> {
        let mut keys = read_trusted(path);
        let had = keys.remove(key);
        if had {
            write_trusted(path, &keys)?;
        }
        Ok(had)
    }

    /// Where a flagged mod stands.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Standing {
        /// Nobody allowed this version yet.
        Review,
        /// The player allowed this version.
        Allowed,
    }

    /// One installed mod and what its scan found, as the screen hands it in.
    #[derive(Clone, Debug)]
    pub struct Candidate {
        pub slot: u16,
        pub name: String,
        pub enabled: bool,
        /// The version key a trust is stored under.
        pub key: String,
        pub report: ScanReport,
    }

    /// One row of the list.
    #[derive(Clone, Debug)]
    pub struct Row {
        pub slot: u16,
        pub name: String,
        pub enabled: bool,
        pub key: String,
        pub standing: Standing,
        pub report: ScanReport,
    }

    impl Row {
        /// The distinct reasons, worst first, for the row's one-line summary. A VPK the scan
        /// could not open reads as Not checked.
        pub fn risks(&self) -> Vec<Risk> {
            let mut risks: Vec<Risk> = self.report.findings.iter().map(|f| f.risk).collect();
            if self.report.error.is_some() {
                risks.push(Risk::Unchecked);
            }
            risks.sort_by_key(|risk| std::cmp::Reverse(risk.rank()));
            risks.dedup();
            risks
        }

        /// Its only findings are script-mod files, which Settings > Allow script mods covers:
        /// Allow N script mods may take it with the others.
        pub fn script_only(&self) -> bool {
            self.report.error.is_none()
                && !self.report.findings.is_empty()
                && self.report.findings.iter().all(|f| f.risk == Risk::ScriptMod)
        }
    }

    /// The flagged mods: Review first, then Allowed, each by slot. A clear scan is not listed.
    pub fn rows(candidates: Vec<Candidate>, trusted: &BTreeSet<String>) -> Vec<Row> {
        let mut rows: Vec<Row> = candidates
            .into_iter()
            .filter(|c| !c.report.is_clear())
            .map(|c| Row {
                standing: if trusted.contains(&c.key) { Standing::Allowed } else { Standing::Review },
                slot: c.slot,
                name: c.name,
                enabled: c.enabled,
                key: c.key,
                report: c.report,
            })
            .collect();
        rows.sort_by_key(|row| (row.standing == Standing::Allowed, row.slot));
        rows
    }

    /// The keys that need the player: every row in Review.
    pub fn pending(rows: &[Row]) -> BTreeSet<String> {
        rows.iter()
            .filter(|row| row.standing == Standing::Review)
            .map(|row| row.key.clone())
            .collect()
    }

    /// Grimoire's notice rule: show it while a pending key is one the player has not
    /// dismissed. A newly flagged mod brings a dismissed notice back.
    pub fn has_new(pending: &BTreeSet<String>, dismissed: &BTreeSet<String>) -> bool {
        pending.iter().any(|key| !dismissed.contains(key))
    }

    /// The Review rows Allow N script mods takes.
    pub fn script_only(rows: &[Row]) -> Vec<&Row> {
        rows.iter()
            .filter(|row| row.standing == Standing::Review && row.script_only())
            .collect()
    }
}

#[cfg(test)]
mod review_tests {
    use super::review::*;
    use super::{Finding, Risk, ScanReport};

    fn report(risks: &[Risk]) -> ScanReport {
        ScanReport {
            findings: risks
                .iter()
                .map(|risk| Finding { path: format!("x/{risk:?}"), risk: *risk, call: String::new() })
                .collect(),
            entries: 4,
            ..ScanReport::default()
        }
    }

    fn candidate(slot: u16, risks: &[Risk], enabled: bool) -> Candidate {
        Candidate { slot, name: format!("Mod {slot}"), enabled, key: format!("gb{slot}:100"), report: report(risks) }
    }

    #[test]
    fn flagged_mods_are_listed_review_first_and_clear_ones_are_not() {
        let trusted = ["gb2:100".to_string()].into_iter().collect();
        let rows = rows(
            vec![
                candidate(2, &[Risk::Program], true),
                candidate(1, &[], true),
                candidate(5, &[Risk::ScriptMod], false),
                candidate(3, &[Risk::Remote, Risk::Script], true),
            ],
            &trusted,
        );
        let order: Vec<(u16, Standing)> = rows.iter().map(|row| (row.slot, row.standing)).collect();
        assert_eq!(order, [(3, Standing::Review), (5, Standing::Review), (2, Standing::Allowed)]);
        assert_eq!(rows[0].risks(), [Risk::Remote, Risk::Script]);
        let pending = pending(&rows);
        assert_eq!(pending.len(), 2);
        assert_eq!(script_only(&rows).iter().map(|row| row.slot).collect::<Vec<_>>(), [5]);
    }

    #[test]
    fn an_unreadable_vpk_is_listed_as_not_checked() {
        let broken = Candidate {
            report: ScanReport { error: Some("bad header".into()), ..ScanReport::default() },
            ..candidate(4, &[], false)
        };
        let rows = rows(vec![broken], &Default::default());
        assert_eq!(rows[0].risks(), [Risk::Unchecked]);
        assert!(!rows[0].script_only());
    }

    #[test]
    fn a_dismissed_notice_comes_back_for_a_new_mod_only() {
        let pending: std::collections::BTreeSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
        assert!(has_new(&pending, &Default::default()));
        assert!(!has_new(&pending, &pending));
        let more: std::collections::BTreeSet<String> = ["a".into(), "b".into(), "c".into()].into_iter().collect();
        assert!(has_new(&more, &pending));
    }

    #[test]
    fn trust_and_revoke_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("sigil-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("safety-trusted.json");
        assert!(read_trusted(&path).is_empty());
        assert!(trust(&path, "gb1:10").unwrap());
        assert!(!trust(&path, "gb1:10").unwrap());
        assert!(trust(&path, "gb2:20").unwrap());
        assert!(revoke(&path, "gb1:10").unwrap());
        assert!(!revoke(&path, "gb1:10").unwrap());
        assert_eq!(read_trusted(&path).into_iter().collect::<Vec<_>>(), ["gb2:20"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn root(tag: &str) -> std::path::PathBuf {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/safety-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let staging = root.join(format!("staging-{name}"));
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        let packed = pack_vpk(&PackOptions::new(staging, root.join("out"), name)).unwrap();
        packed.dir_path
    }

    const SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 64 64">
  <!-- Made with Inkscape (https://inkscape.org) -->
  <path d="M0 0h64v64H0z"/>
</svg>"#;

    const LAYOUT: &[u8] = br##"<!-- Credits: https://gamebanana.com/mods/1 -->
<root>
  <styles><include src="s2r://panorama/styles/citadel_hud.vcss_c" /></styles>
  <scripts><include src="s2r://panorama/scripts/hud.vjs_c" /></scripts>
  <Panel class="Hud" onactivate="$.DispatchEvent('PlaySoundEffect', 'ui_select');">
    <Image src="file://{images}/hud/icon.vsvg" />
    <Label text="#Citadel_Hud" />
  </Panel>
</root>"##;

    const STYLE: &[u8] = br#"/* Theme by someone, see https://example.com/licence */
.Hud { background-image: url("s2r://panorama/images/hud/bg.vtex"); color: #fff; }"#;

    #[test]
    fn an_svg_with_the_w3_namespace_is_clear() {
        assert!(scan_text(SVG).is_empty());
        assert_eq!(risk_of_path("panorama/images/heroes/shiv.svg"), None);
        assert!(!is_panorama_text("panorama/images/heroes/shiv.svg"));
    }

    #[test]
    fn icons_and_images_are_never_findings() {
        for path in [
            "panorama/images/heroes/hero_names/shiv.vsvg_c",
            "panorama/images/heroes/shiv_card.vsvg",
            "panorama/images/heroes/shiv_card.svg",
            "panorama/images/heroes/shiv_card_psd.vtex_c",
            "panorama/images/hud/icon.png",
        ] {
            assert_eq!(risk_of_path(path), None, "{path}");
            assert!(!is_panorama_text(path), "{path}");
        }
    }

    #[test]
    fn plain_layouts_styles_and_scripts_are_clear() {
        assert!(scan_text(LAYOUT).is_empty(), "{:?}", scan_text(LAYOUT));
        assert!(scan_text(STYLE).is_empty(), "{:?}", scan_text(STYLE));
        let script = b"'use strict';\nfunction OnUpdate() { $('#Timer').text = Game.GetGameTime().toFixed(0); $.Schedule(0.1, OnUpdate); }\nOnUpdate();";
        assert!(scan_text(script).is_empty());
        for path in ["a/hud.vxml_c", "a/hud.vcss_c", "a/hud.vjs_c", "a/hud.vts_c"] {
            assert_eq!(risk_of_path(path), None, "{path}");
        }
    }

    #[test]
    fn a_panorama_web_request_is_flagged_with_its_reason() {
        let found = scan_text(br#"$.AsyncWebRequest("https://example.com/collect", { type: "POST", data: { id: 1 } });"#);
        assert_eq!(found[0], ("$.AsyncWebRequest".to_string(), Risk::Remote));
        let finding = Finding {
            path: "panorama/scripts/hud.vjs_c".to_string(),
            risk: Risk::Remote,
            call: found[0].0.clone(),
        };
        assert!(finding.reason().contains("$.AsyncWebRequest"));
    }

    #[test]
    fn browser_calls_and_url_loads_are_flagged_but_namespaces_are_not() {
        let calls = |body: &[u8]| scan_text(body).into_iter().map(|(call, _)| call).collect::<Vec<_>>();
        assert_eq!(calls(br#"<Button onactivate="ExternalBrowserGoToURL('https://x.example')" />"#), [
            "ExternalBrowserGoToURL"
        ]);
        assert_eq!(calls(b"$('#Web').SetURL(url);"), ["SetURL"]);
        assert_eq!(calls(br#"<root><CitadelHTMLPanel id="Web" /></root>"#), ["browser panel"]);
        assert_eq!(calls(br#"<Image src="https://cdn.example/a.png" />"#), ["https://cdn.example/a.png"]);
        let style = scan_text(br#".A { background-image: url('//cdn.example/b.png'); }"#);
        assert_eq!(style, [("//cdn.example/b.png".to_string(), Risk::Remote)]);
        assert_eq!(calls(b"eval(atob('ZmV0Y2g='))").len(), 2);
        // A namespace and an xlink are not loads.
        assert!(calls(br#"<root xmlns="http://www.w3.org/1999/xhtml" xlink:href="http://www.w3.org/1999/xlink"></root>"#).is_empty());
        // Names that only contain a signal are not it.
        assert!(calls(b"prefetch(1); MyFunction(2); evaluate(3);").is_empty());
        let url = Finding { path: "a.vxml_c".into(), risk: Risk::Remote, call: "https://cdn.example/a.png".into() };
        assert_eq!(url.reason(), "Loads https://cdn.example/a.png from the internet when the game shows it.");
    }

    #[test]
    fn every_url_load_is_listed_even_in_a_comment() {
        let loads = |body: &[u8]| scan_text(body).into_iter().filter(|(call, _)| call.contains("evil")).count();
        // Comment markers in quoted values, CSS `//` lines, processing instructions and
        // doctypes cannot hide a load, because no comment is exempt.
        assert_eq!(loads(br#"<root><Label text="<!--" /><Image src="https://evil.example/x.png" /><Label text="-->" /></root>"#), 1);
        assert_eq!(loads(br#".A { content: "/*"; background-image: url("https://evil.example/y.png"); } .B { content: "*/"; }"#), 1);
        assert_eq!(loads(b"// a /* \n.A { background: url(https://evil.example/z); } /* */"), 1);
        assert_eq!(loads(br#"<?xml version='1.0' ?><!DOCTYPE x "a'"><Image src="https://evil.example/x" />"#), 1);
        assert_eq!(loads(br#"var r = /x\/*/; $('#I').SetImage("https://evil.example/x"); var s = "*/";"#), 1);
        // A commented-out load is listed too: the scan fails closed.
        assert_eq!(loads(br#"<root><!-- <Image src="https://evil.example/credit" /> --></root>"#), 1);
        assert_eq!(loads(br#"/* url("https://evil.example/credit") */ .A {}"#), 1);
        // Namespaces, doctypes and plain links in credits are not loads.
        assert_eq!(scan_text(SVG), []);
        assert_eq!(scan_text(br#"<!-- Made by me, https://evil.example/profile --><root />"#), []);
    }

    #[test]
    fn oversized_and_unreadable_layouts_are_listed_not_skipped() {
        let root = root("unchecked");
        let mut big = b"<root>".to_vec();
        big.resize(MAX_TEXT_BYTES as usize + 16, b' ');
        big.extend_from_slice(br#"<Image src="https://evil.example/x" /></root>"#);
        let mod_vpk = vpk(&root, "pak04", &[("panorama/layout/big.vxml_c", &big), ("panorama/layout/ok.xml", b"<root />")]);
        let report = scan_vpk(&mod_vpk);
        let big = report.findings.iter().find(|f| f.path.ends_with("big.vxml_c")).expect("oversized layout listed");
        assert_eq!((big.risk, big.call.as_str()), (Risk::Unchecked, TOO_LARGE));
        assert!(big.reason().starts_with("Too large to check"));
        assert!(report.findings.iter().all(|f| !f.path.ends_with("ok.xml")));

        // Cut the archive the layout's bytes live in, so the directory still lists it.
        let small = vpk(&root, "pak05", &[("panorama/layout/hud.vxml_c", &[b'x'; 4096])]);
        let directory = read_vpk_directory_with(&small, &ReadVpkOptions { keep_preload: true }).unwrap();
        let entry = &directory.entries[0];
        let data_file = if entry.archive_index == 0x7fff {
            small.clone()
        } else {
            small.with_file_name(
                small.file_name().unwrap().to_string_lossy().replace("_dir.vpk", &format!("_{:03}.vpk", entry.archive_index)),
            )
        };
        let length = fs::metadata(&data_file).unwrap().len();
        fs::OpenOptions::new().write(true).open(&data_file).unwrap().set_len(length - 2048).unwrap();
        let report = scan_vpk(&small);
        assert!(report.error.is_none(), "{:?}", report.error);
        let broken = &report.findings[0];
        assert_eq!((broken.risk, broken.call.as_str()), (Risk::Unchecked, UNREADABLE));
        assert!(broken.reason().starts_with("Could not be read"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn programs_scripts_and_script_mods_follow_the_install_rules() {
        assert_eq!(risk_of_path("bin/helper.exe"), Some(Risk::Program));
        assert_eq!(risk_of_path("a/b.DLL"), Some(Risk::Program));
        assert_eq!(risk_of_path("scripts/vscripts/boot.lua"), Some(Risk::Script));
        assert_eq!(risk_of_path("panorama/scripts/hud.js"), Some(Risk::Script));
        assert_eq!(risk_of_path("cfg/autoexec.cfg"), Some(Risk::Script));
        assert_eq!(risk_of_path("models/heroes/shiv/shiv.vnmskel_c"), Some(Risk::ScriptMod));
        assert_eq!(risk_of_path("scripts/abilities.vdata_c"), Some(Risk::ScriptMod));
        assert_eq!(risk_of_path("a/b.vmat_c"), None);
        assert_eq!(risk_of_path("no-extension"), None);
        // The scan and the install check share one definition of a script mod.
        for path in ["models/x.vnmskel_c", "scripts/heroes.vdata_c", "scripts/a/b.vdata_c", "pak/x.vdata_c"] {
            assert_eq!(
                risk_of_path(path) == Some(Risk::ScriptMod),
                payload::script_mod_kind(path).is_some(),
                "{path}"
            );
        }
        let cfg = Finding { path: "cfg/autoexec.cfg".into(), risk: Risk::Script, call: String::new() };
        assert!(cfg.reason().starts_with("Console config"));
        let skeleton = Finding { path: "m/a.vnmskel_c".into(), risk: Risk::ScriptMod, call: String::new() };
        assert_eq!(skeleton.reason(), ScriptModKind::Skeleton.message());
    }

    #[test]
    fn flags_programs_scripts_and_network_calls() {
        let root = root("scan");
        let mod_vpk = vpk(&root, "pak01", &[
            ("materials/hero.vmat_c", b"asset"),
            ("bin/helper.exe", b"MZ"),
            ("panorama/layout/hud.xml", b"<root><script>fetch(\"https://x\")</script></root>"),
            ("panorama/scripts/net.vjs_c", b"$.AsyncWebRequest(\"https://example.com\", {});"),
            ("scripts/hello.js", b"console.log('hi')"),
        ]);
        let report = scan_vpk(&mod_vpk);
        assert_eq!(report.entries, 5);
        assert!(report.error.is_none());
        let has = |path: &str, risk: Risk| report.findings.iter().any(|f| f.path.ends_with(path) && f.risk == risk);
        assert!(has("bin/helper.exe", Risk::Program));
        assert!(has("scripts/hello.js", Risk::Script));
        assert!(report.findings.iter().any(|f| f.path.ends_with("hud.xml") && f.call == "fetch"));
        assert!(report.findings.iter().any(|f| f.path.ends_with("net.vjs_c") && f.call == "$.AsyncWebRequest"));
        assert!(report.findings.iter().all(|f| !f.path.ends_with("vmat_c")));
        assert_eq!(report.worst(), Some(Risk::Program));
        assert!(!report.is_clear());
        let _ = fs::remove_dir_all(&root);
    }

    /// GameBanana mod 679331 (Shiv "Josuke Remaster"): a hero skin with models, materials and
    /// Shiv's Panorama icons as compiled SVG. It was listed as "Reaches out" because every SVG
    /// names the w3 namespace. It ships nothing worth a warning.
    #[test]
    fn gamebanana_679331_shiv_skin_with_svg_icons_is_clear() {
        let root = root("josuke");
        let mut icon = b"\x00\x00\x00\x00\x0c\x00\x00\x00DATA".to_vec();
        icon.extend_from_slice(SVG);
        let skin = vpk(&root, "pak03", &[
            ("models/heroes_staging/shiv/shiv.vmdl_c", b"model"),
            ("materials/heroes/shiv/shiv_body.vmat_c", b"material"),
            ("materials/heroes/shiv/shiv_body_color_png.vtex_c", b"texture"),
            ("panorama/images/heroes/shiv_card.vsvg_c", &icon),
            ("panorama/images/heroes/shiv_sm.vsvg_c", &icon),
            ("panorama/images/heroes/hero_names/shiv.vsvg_c", &icon),
        ]);
        let report = scan_vpk(&skin);
        assert!(report.is_clear(), "{:?}", report.findings);
        assert_eq!(report.worst(), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_clean_mod_is_clear_and_a_broken_file_is_reported() {
        let root = root("clear");
        let clean = vpk(&root, "pak02", &[
            ("materials/a.vmat_c", b"a"),
            ("models/b.vmdl_c", b"b"),
            ("panorama/layout/hud.vxml_c", LAYOUT),
            ("panorama/styles/hud.vcss_c", STYLE),
        ]);
        assert!(scan_vpk(&clean).is_clear());
        assert_eq!(scan_vpk(&clean).worst(), None);

        let missing = root.join("nope_dir.vpk");
        assert!(scan_vpk(&missing).error.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_install_lands_on_only_when_the_scan_is_clear() {
        assert_eq!(landing(&ScanReport::default()), Landing::On);
        let flagged = ScanReport {
            findings: vec![Finding { path: "bin/x.exe".into(), risk: Risk::Program, call: String::new() }],
            ..ScanReport::default()
        };
        let decision = landing(&flagged);
        assert!(decision.is_off());
        assert_eq!(decision.findings(), flagged.findings);
        assert_eq!(decision.error(), None);
        // A VPK the scan could not open stays off too, with the reason kept.
        let broken = ScanReport { error: Some("bad header".into()), ..ScanReport::default() };
        let decision = landing(&broken);
        assert!(decision.is_off());
        assert!(decision.findings().is_empty());
        assert_eq!(decision.error(), Some("bad header"));
        // The count past the list still reaches the review.
        let many = ScanReport { more: 7, ..flagged };
        assert!(matches!(landing(&many), Landing::Off { more: 7, .. }));
    }

    #[test]
    fn landing_at_reads_the_file_as_written() {
        let root = root("landing");
        let clean = vpk(&root, "pak06", &[("materials/a.vmat_c", b"a")]);
        assert_eq!(landing_at(&clean), Landing::On);
        let flagged = vpk(&root, "pak07", &[("bin/helper.exe", b"MZ")]);
        assert!(landing_at(&flagged).is_off());
        assert!(landing_at(&root.join("missing_dir.vpk")).is_off(), "unreadable is no way past");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_text_file_keeps_a_few_calls() {
        let calls = scan_text(b"fetch(1); eval(2); atob(3); XMLHttpRequest; WebSocket");
        assert_eq!(calls.len(), MAX_PER_FILE);
        // Worst first: the three network calls outrank eval and atob.
        assert!(calls.iter().all(|(_, risk)| *risk == Risk::Remote), "{calls:?}");
    }

    /// Scan a real VPK by hand: `SIGIL_SAFETY_VPK=/path/to/mod.vpk cargo test -p deadlock-core
    /// scan_a_vpk_from_the_environment -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scan_a_vpk_from_the_environment() {
        let path = std::env::var("SIGIL_SAFETY_VPK").expect("SIGIL_SAFETY_VPK");
        let report = scan_vpk(Path::new(&path));
        println!("entries: {}, error: {:?}", report.entries, report.error);
        for finding in &report.findings {
            println!("{}: {} ({})", finding.risk.label(), finding.path, finding.reason());
        }
        println!("clear: {}", report.is_clear());
    }
}
