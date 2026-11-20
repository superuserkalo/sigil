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
//! XML namespace, and licence comments carry links; comments are skipped for the URL search.
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
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Program => "Program",
            Risk::Script => "Source script",
            Risk::Remote => "Reaches out",
            Risk::Dynamic => "Hidden code",
            Risk::ScriptMod => "Script mod",
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
        }
    }

    /// Higher sorts first in the list.
    pub fn rank(self) -> u8 {
        match self {
            Risk::Program => 5,
            Risk::Remote => 4,
            Risk::Dynamic => 3,
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
/// roots are local and do not match. The URL is captured for the message.
static URL_LOADS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?:(?:^|[\s<;{])(?:src|url|href)\s*=\s*["']|\burl\(\s*["']?|\bSetImage\s*\(\s*["'])\s*((?:https?:)?//[^"'\s)<>]{1,200})"#,
    )
    .expect("url pattern")
});

/// XML and block comments, and `//` comments that start a line, removed before the URL search
/// only: licence text and credits carry links there. The API names are searched in the full
/// text, so a comment trick cannot hide a call.
static COMMENTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<!--.*?-->|/\*.*?\*/|(?m:^[ \t]*//[^\n]*)").expect("comment pattern")
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

/// Search one text body. Returns at most [`MAX_PER_FILE`] `(call, risk)` pairs, worst first.
pub fn scan_text(body: &[u8]) -> Vec<(String, Risk)> {
    let text = String::from_utf8_lossy(body);
    let mut found: Vec<(String, Risk)> = SIGNALS
        .iter()
        .zip(SIGNAL_PATTERNS.iter())
        .filter(|(_, pattern)| pattern.is_match(&text))
        .map(|(signal, _)| (signal.call.to_string(), signal.risk))
        .collect();
    let uncommented = COMMENTS.replace_all(&text, " ");
    if let Some(load) = URL_LOADS.captures(&uncommented).and_then(|caps| caps.get(1)) {
        found.push((load.as_str().to_string(), Risk::Remote));
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
        if !is_panorama_text(&entry.path) || u64::from(entry.total_length) > MAX_TEXT_BYTES {
            continue;
        }
        let Ok(data) = read_vpk_entry_data(&directory, entry) else {
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
        assert_eq!(calls(br#".A { background-image: url('//cdn.example/b.png'); }"#), ["//cdn.example/b.png"]);
        assert_eq!(calls(b"eval(atob('ZmV0Y2g='))").len(), 2);
        // A namespace, a doctype, an xlink and a credit comment are not loads.
        assert!(calls(br#"<root xmlns="http://www.w3.org/1999/xhtml" xlink:href="http://www.w3.org/1999/xlink"><!-- <Image src="https://x" /> --></root>"#).is_empty());
        // Names that only contain a signal are not it.
        assert!(calls(b"prefetch(1); MyFunction(2); evaluate(3);").is_empty());
        let url = Finding { path: "a.vxml_c".into(), risk: Risk::Remote, call: "https://cdn.example/a.png".into() };
        assert_eq!(url.reason(), "Loads https://cdn.example/a.png from the internet when the game shows it.");
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
