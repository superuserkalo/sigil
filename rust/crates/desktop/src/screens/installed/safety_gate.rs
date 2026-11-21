//! The mod safety gate (Grimoire's): a mod that ships programs, scripts or network calls waits
//! off until the player looks at what it holds.
//!
//! - Grimoire: script mods stay off until reviewed; Enable anyway, or trust this version so it
//!   never asks again.
//! - Deadlock Mod Manager: no gate.
//! - Sigil: turning such a mod on (any switch that goes through `enable::switch`) opens a
//!   question listing what `mod_safety::scan_vpk` found, each line with its reason, with Cancel,
//!   Turn on this once and Trust this version (kept in `safety-trusted.json`, by the mod and its
//!   file size). Images, layouts, styles and Panorama scripts, which skins and HUD mods ship, do
//!   not ask unless their text reaches the network. Settings > "Allow script mods" skips the
//!   gate, as it lets script mods through the install check. Rules: `docs/design/mod-safety.md`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use deadlock_core::mod_safety::{self, Risk};
use rune::{El, col, row, spacer, text};

use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, H1, MONO, SMALL, Ui};

fn trusted_path() -> PathBuf {
    crate::data::data_dir().join("safety-trusted.json")
}

fn read_trusted() -> BTreeSet<String> {
    std::fs::read_to_string(trusted_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|list| list.into_iter().collect())
        .unwrap_or_default()
}

/// The version key: the GameBanana id (or the file name) and the file size.
fn version_key(st: &State, slot: u16) -> Option<String> {
    let addon = st.addons.addons.iter().find(|a| a.slot == slot)?;
    let who = super::meta_for(st, addon)
        .and_then(|m| m.gamebanana_id.map(|id| format!("gb{id}")))
        .unwrap_or_else(|| addon.file.trim_end_matches(".disabled").to_string());
    Some(format!("{who}:{}", addon.size_bytes))
}

/// Risks that stop a switch. Every finding the scan makes is one: it lists nothing for show.
pub fn gating(risk: Risk) -> bool {
    matches!(
        risk,
        Risk::Program | Risk::Script | Risk::Remote | Risk::Dynamic | Risk::ScriptMod | Risk::Unchecked
    )
}

/// What to show before `slot` turns on, or `None` to go ahead. The scan is cached per slot file
/// and shared with "Check what is inside".
pub fn needs_review(st: &mut State, slot: u16) -> Option<Vec<String>> {
    if st.demo || rune::is_headless() || crate::screens::settings::options::get().allow_scripts {
        return None;
    }
    let key = version_key(st, slot)?;
    if read_trusted().contains(&key) {
        return None;
    }
    let addon = st.addons.addons.iter().find(|a| a.slot == slot)?.clone();
    let cache_key = (slot, addon.size_bytes);
    if !st.installed.tools.scans.contains_key(&cache_key) {
        let report = mod_safety::scan_vpk(&addon.path(&super::addons_folder(st)));
        st.installed.tools.scans.insert(cache_key, report);
    }
    let report = st.installed.tools.scans.get(&cache_key)?;
    let lines = findings(report);
    (!lines.is_empty()).then_some(lines)
}

/// An install that should land switched on (Settings > Mods > New mods start turned off is off)
/// is written switched off first; this scans the written file and turns it on only when the
/// gate would have nothing to ask (Grimoire #591: only what passed the review goes live).
/// Returns the findings when the mod stays off for the gate. Worker thread: it reads the VPK.
pub fn land_checked(game_root: &std::path::Path, slot: u16, written: &std::path::Path, gamebanana_id: Option<u64>) -> Option<Vec<String>> {
    let ask = if crate::screens::settings::options::get().allow_scripts {
        None
    } else {
        let size = std::fs::metadata(written).map(|m| m.len()).unwrap_or(0);
        let who = gamebanana_id.map(|id| format!("gb{id}")).unwrap_or_else(|| {
            written
                .file_name()
                .map(|n| n.to_string_lossy().trim_end_matches(".disabled").to_string())
                .unwrap_or_default()
        });
        if read_trusted().contains(&format!("{who}:{size}")) {
            None
        } else {
            let lines = findings(&mod_safety::scan_vpk(written));
            (!lines.is_empty()).then_some(lines)
        }
    };
    if ask.is_some() {
        return ask;
    }
    match deadlock_core::addons::set_enabled(game_root, slot, true, true) {
        Ok(_) => None,
        // Left off: the Installed switch turns it on and says why if that fails too.
        Err(error) => Some(vec![error.to_string()]),
    }
}

/// After an install that stayed off for the gate: say so and offer the review. The review is
/// the same question Installed's switch asks.
pub fn landed_off(st: &mut State, slot: u16, name: &str, findings: Vec<String>) {
    let detail = fill(
        tr("It holds files the mod safety check asks about ({}). Review them to turn it on."),
        &[&findings.len().to_string()],
    );
    crate::notify::push(
        st,
        crate::notify::Notice::warning(fill(tr("{} was installed switched off"), &[name]))
            .detail(detail)
            .action(tr("Review"), move |st: &mut State| {
                st.go(crate::model::Screen::Installed);
                st.installed.gates.open = Some(super::enable::Gate::Safety { slot, findings });
            }),
    );
}

/// "Program: bin/x.exe" lines for the gating findings, with what matched when a call did:
/// "Reaches out: panorama/scripts/hud.vjs_c ($.AsyncWebRequest)".
/// A VPK the scan could not open asks too: being unreadable is no way past the gate.
pub fn findings(report: &mod_safety::ScanReport) -> Vec<String> {
    let unread = report
        .error
        .as_ref()
        .map(|error| format!("{}: {} ({error})", tr(Risk::Unchecked.label()), tr("the whole file")));
    let lines = report.findings.iter().filter(|f| gating(f.risk)).map(|f| {
        if f.call.is_empty() {
            format!("{}: {}", tr(f.risk.label()), f.path)
        } else {
            format!("{}: {} ({})", tr(f.risk.label()), f.path, f.call)
        }
    });
    unread.into_iter().chain(lines).collect()
}

fn trust(st: &mut State, slot: u16) {
    if let Some(key) = version_key(st, slot) {
        let mut list = read_trusted();
        list.insert(key);
        let list: Vec<String> = list.into_iter().collect();
        if let Ok(text) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(trusted_path(), text);
        }
    }
    super::enable::switch_reviewed(st, slot, true);
}

pub fn dialog(s: &State, u: &Ui, slot: u16, findings: &[String]) -> El {
    let name = s
        .addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| super::display_name(s, a))
        .unwrap_or_else(|| deadlock_core::addons::slot_label(slot));
    let mut list = col().w_full().gap(4.0);
    for line in findings.iter().take(8) {
        list = list.child(text(line.clone()).font(MONO).color(u.prose).truncate());
    }
    if findings.len() > 8 {
        list = list.child(text(fill(tr("and {} more"), &[&(findings.len() - 8).to_string()])).font(SMALL).color(u.faint));
    }
    let close = |st: &mut State| st.installed.gates.open = None;
    u.dialog_shell()
        .key("gate-safety")
        .role("dialog")
        .label(tr("Look before it turns on"))
        .w(520.0)
        .padding(24.0)
        .gap(14.0)
        .modal(true, close)
        .child(text(fill(tr("{} ships code"), &[&name])).font(H1).color(u.ink).wrap())
        .child(
            text(tr("Each line names a file and why it is listed: a program, a script the game does not run, a script mod, a call that opens a web page or reaches the network, or a file Sigil could not check. Images are never listed, and layouts and styles only for one of these reasons. Turn it on only when you trust where it came from."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(list)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("gate-safety-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action(tr("Turn on this once"), ActionKind::Secondary)
                        .key("gate-safety-once")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| super::enable::switch_reviewed(st, slot, true)),
                )
                .child(
                    u.action(tr("Trust this version"), ActionKind::Primary)
                        .key("gate-safety-trust")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| trust(st, slot)),
                ),
        )
}

/// `w4a-safety-gate`: turning on a mod that ships a program and a script.
pub fn stage_scene(st: &mut State) {
    st.installed.gates.open = Some(super::enable::Gate::Safety {
        slot: 3,
        findings: vec![
            "Program: bin/helper.exe".to_string(),
            "Reaches out: panorama/scripts/hud.vjs_c ($.AsyncWebRequest)".to_string(),
            "Source script: scripts/vscripts/boot.lua".to_string(),
        ],
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::mod_safety::{Finding, ScanReport};

    #[test]
    fn every_finding_asks_and_says_what_matched() {
        let finding = |path: &str, risk, call: &str| Finding { path: path.to_string(), risk, call: call.to_string() };
        let report = ScanReport {
            findings: vec![
                finding("bin/a.exe", Risk::Program, ""),
                finding("panorama/scripts/hud.vjs_c", Risk::Remote, "$.AsyncWebRequest"),
            ],
            ..ScanReport::default()
        };
        let lines = findings(&report);
        assert_eq!(lines, ["Program: bin/a.exe", "Reaches out: panorama/scripts/hud.vjs_c ($.AsyncWebRequest)"]);
        // A skin or HUD mod's scan has no findings, so nothing asks.
        assert!(findings(&ScanReport::default()).is_empty());
        // Files the scan could not check ask, and so does a VPK it could not open.
        let unchecked = ScanReport {
            findings: vec![finding("panorama/layout/big.vxml_c", Risk::Unchecked, mod_safety::TOO_LARGE)],
            ..ScanReport::default()
        };
        assert_eq!(findings(&unchecked), ["Not checked: panorama/layout/big.vxml_c (too large)"]);
        let broken = ScanReport { error: Some("bad header".into()), ..ScanReport::default() };
        assert_eq!(findings(&broken), ["Not checked: the whole file (bad header)"]);
    }

    /// GameBanana mod 679331 (Shiv "Josuke Remaster") ships Shiv's icons as `vsvg_c`. The scan
    /// used to list them as "Reaches out"; now the gate stays shut.
    #[test]
    fn a_skin_with_svg_icons_does_not_ask() {
        for path in ["panorama/images/heroes/hero_names/shiv.vsvg_c", "panorama/images/heroes/hero_names/shiv_localized.vsvg_c"] {
            assert_eq!(mod_safety::risk_of_path(path), None);
        }
    }
}
