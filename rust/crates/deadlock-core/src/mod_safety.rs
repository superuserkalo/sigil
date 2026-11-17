//! What a mod's VPK ships besides game assets: scripts, browser panels and native programs.
//!
//! The game itself only reads compiled assets, so a mod that ships a `.js`, an `.exe` or a
//! layout that reaches the network is unusual and worth showing before it is turned on. This
//! reads the VPK directory (never the game folder) and reports what it found as a short list of
//! findings, the way Grimoire's safety scan does. It is a warning, not a gate: nothing here
//! blocks an install or a switch.
//!
//! Asset extensions are skipped without reading their bytes; text formats are read (up to
//! [`MAX_TEXT_BYTES`] each) and searched for the calls a mod would use to reach the network or
//! run code at runtime. A finding is capped per file so one minified script cannot fill the
//! dialog.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
    /// A script the game cannot run: `js`, `vjs`, `ts`, `lua`, `nut`, `cfg`.
    Script,
    /// A compiled text asset: `vjs_c`, `vts_c`, `vcss_c`, `vxml_c`, `vsvg_c`.
    CompiledText,
    /// A page or layout: `html`, `xml`, `vxml`, `svg`, `vsvg`.
    Page,
    /// A text asset with browser or network calls in it.
    Remote,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Program => "Program",
            Risk::Script => "Script",
            Risk::CompiledText => "Compiled script",
            Risk::Page => "Page",
            Risk::Remote => "Reaches out",
        }
    }

    /// One line on what it means for the player.
    pub fn line(self) -> &'static str {
        match self {
            Risk::Program => "A program is not something a mod needs. The game never runs it.",
            Risk::Script => "Scripts can run code while the game is open.",
            Risk::CompiledText => "Compiled layout script. It can run code while the game is open.",
            Risk::Page => "A page or layout that the game can show.",
            Risk::Remote => "This file can reach the network or load code at runtime.",
        }
    }

    /// Higher sorts first in the list.
    pub fn rank(self) -> u8 {
        match self {
            Risk::Program => 4,
            Risk::Remote => 3,
            Risk::CompiledText | Risk::Script => 2,
            Risk::Page => 1,
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
    /// The call that matched, for example `fetch(`. Empty for a file that is only listed.
    pub call: String,
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

/// Calls a text asset can use to reach the network, load code or read local files.
const CALLS: [(&str, Risk); 14] = [
    ("fetch(", Risk::Remote),
    ("XMLHttpRequest", Risk::Remote),
    ("WebSocket", Risk::Remote),
    ("importScripts", Risk::Remote),
    ("SetURL", Risk::Remote),
    ("OpenExternalBrowserURL", Risk::Remote),
    ("OpenURL", Risk::Remote),
    ("javascript:", Risk::Remote),
    ("http://", Risk::Remote),
    ("https://", Risk::Remote),
    ("eval(", Risk::Remote),
    ("new Function", Risk::Remote),
    ("atob(", Risk::Remote),
    ("fromCharCode", Risk::Remote),
];

fn extension(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The risk of shipping a path at all, by extension. `None` is a normal game asset.
pub fn risk_of_path(path: &str) -> Option<Risk> {
    let ext = extension(path);
    match ext.as_str() {
        "exe" | "dll" | "com" | "bat" | "cmd" | "ps1" | "vbs" | "lnk" | "msi" | "so" | "dylib"
        | "wasm" => Some(Risk::Program),
        "js" | "vjs" | "ts" | "vts" | "lua" | "nut" | "cfg" => Some(Risk::Script),
        "vjs_c" | "vts_c" | "vcss_c" | "vxml_c" | "vsvg_c" => Some(Risk::CompiledText),
        "html" | "htm" | "xml" | "vxml" | "svg" | "vsvg" | "css" | "vcss" => Some(Risk::Page),
        _ => None,
    }
}

/// Extensions whose bytes are searched. Compiled text carries its source in a DATA block, and
/// plain text formats are readable as they are.
fn is_text(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "js" | "vjs" | "ts" | "vts" | "lua" | "nut" | "cfg" | "html" | "htm" | "xml" | "vxml"
            | "svg" | "vsvg" | "css" | "vcss" | "vjs_c" | "vts_c" | "vcss_c" | "vxml_c" | "vsvg_c"
    )
}

/// Search one text body for calls. Returns at most [`MAX_PER_FILE`], worst first.
pub fn scan_text(body: &[u8]) -> Vec<&'static str> {
    let text = String::from_utf8_lossy(body);
    let mut found: Vec<&'static str> = CALLS
        .iter()
        .filter(|(needle, _)| text.contains(needle))
        .map(|(needle, _)| *needle)
        .collect();
    // One call is enough to warn; the shortest names first read better in a list.
    found.sort_by_key(|needle| needle.len());
    found.truncate(MAX_PER_FILE);
    found
}

/// Scan a directory VPK. Reads only the VPK and its chunk files.
pub fn scan_vpk(path: &Path) -> ScanReport {
    let directory = match read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: false }) {
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
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();

    // Programs and scripts first, so the cap keeps what matters.
    let mut entries: Vec<&crate::vpk::VpkEntry> = directory.entries.iter().collect();
    entries.sort_by_key(|entry| {
        let risk = risk_of_path(&entry.path);
        std::cmp::Reverse(risk.map(Risk::rank).unwrap_or(0))
    });

    for entry in entries {
        let Some(risk) = risk_of_path(&entry.path) else {
            continue;
        };
        let mut calls: Vec<&'static str> = Vec::new();
        if is_text(&entry.path)
            && u64::from(entry.total_length) <= MAX_TEXT_BYTES
            && let Ok(data) = read_vpk_entry_data(&directory, entry)
        {
            calls = scan_text(&data);
        }
        let per_file = seen.entry(entry.path.as_str()).or_insert(0);
        if calls.is_empty() {
            push_finding(&mut findings, &mut more, Finding {
                path: entry.path.clone(),
                risk,
                call: String::new(),
            });
            continue;
        }
        for call in calls {
            *per_file += 1;
            if *per_file > MAX_PER_FILE {
                break;
            }
            push_finding(&mut findings, &mut more, Finding {
                path: entry.path.clone(),
                risk: if matches!(risk, Risk::Page | Risk::Script | Risk::CompiledText) {
                    Risk::Remote
                } else {
                    risk
                },
                call: call.to_string(),
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

    #[test]
    fn flags_programs_scripts_and_network_calls() {
        let root = root("scan");
        let mod_vpk = vpk(&root, "pak01", &[
            ("materials/hero.vmat_c", b"asset"),
            ("bin/helper.exe", b"MZ"),
            ("panorama/layout/hud.xml", b"<root><script>fetch(\"https://x\")</script></root>"),
            ("scripts/hello.js", b"console.log('hi')"),
        ]);
        let report = scan_vpk(&mod_vpk);
        assert_eq!(report.entries, 4);
        assert!(report.error.is_none());
        assert!(report.findings.iter().any(|f| f.path == "bin/helper.exe" && f.risk == Risk::Program));
        assert!(report.findings.iter().any(|f| f.path == "scripts/hello.js" && f.risk == Risk::Script));
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.path.ends_with("hud.xml") && f.risk == Risk::Remote && f.call == "fetch(")
        );
        // The asset is never listed.
        assert!(report.findings.iter().all(|f| !f.path.ends_with("vmat_c")));
        assert_eq!(report.worst(), Some(Risk::Program));
        assert!(!report.is_clear());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_clean_mod_is_clear_and_a_broken_file_is_reported() {
        let root = root("clear");
        let clean = vpk(&root, "pak02", &[("materials/a.vmat_c", b"a"), ("models/b.vmdl_c", b"b")]);
        assert!(scan_vpk(&clean).is_clear());
        assert_eq!(scan_vpk(&clean).worst(), None);

        let missing = root.join("nope_dir.vpk");
        assert!(scan_vpk(&missing).error.is_some());
    }

    #[test]
    fn a_text_file_keeps_a_few_calls_and_the_report_is_capped() {
        let body = b"fetch(1); eval(2); atob(3); XMLHttpRequest";
        let calls = scan_text(body);
        assert!(!calls.is_empty());
        assert!(calls.len() <= MAX_PER_FILE);
        assert_eq!(risk_of_path("a/b.VJS_C"), Some(Risk::CompiledText));
        assert_eq!(risk_of_path("a/b.vmat_c"), None);
        assert_eq!(risk_of_path("no-extension"), None);
    }
}
