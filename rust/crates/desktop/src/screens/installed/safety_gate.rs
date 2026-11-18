//! The mod safety gate (Grimoire's): a mod that ships programs, scripts or network calls waits
//! off until the player looks at what it holds.
//!
//! - Grimoire: script mods stay off until reviewed; Enable anyway, or trust this version so it
//!   never asks again.
//! - Deadlock Mod Manager: no gate.
//! - Sigil: turning such a mod on (any switch that goes through `enable::switch`) opens a
//!   question listing what `mod_safety::scan_vpk` found, with Cancel, Turn on this once and Trust
//!   this version (kept in `safety-trusted.json`, by the mod and its file size). Compiled UI
//!   files and pages, which every HUD mod ships, do not ask. Settings > "Allow script mods" skips
//!   the gate.

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

/// Risks that stop a switch: a program, a script or a network call.
pub fn gating(risk: Risk) -> bool {
    matches!(risk, Risk::Program | Risk::Script | Risk::Remote)
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

/// "Program: bin/x.exe" lines for the gating findings.
pub fn findings(report: &mod_safety::ScanReport) -> Vec<String> {
    report
        .findings
        .iter()
        .filter(|f| gating(f.risk))
        .map(|f| format!("{}: {}", tr(f.risk.label()), f.path))
        .collect()
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
            text(tr("Deadlock does not run these files, but they are unusual in a mod. Turn it on only when you trust where it came from."))
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
        findings: vec!["Program: bin/helper.exe".to_string(), "Script: scripts/vscripts/boot.lua".to_string()],
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::mod_safety::{Finding, ScanReport};

    #[test]
    fn only_programs_scripts_and_network_calls_ask() {
        let finding = |path: &str, risk| Finding { path: path.to_string(), risk, call: String::new() };
        let report = ScanReport {
            findings: vec![
                finding("panorama/layout/hud.vxml_c", Risk::CompiledText),
                finding("x/page.html", Risk::Page),
                finding("bin/a.exe", Risk::Program),
            ],
            ..ScanReport::default()
        };
        let lines = findings(&report);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("bin/a.exe"));
        let hud_only = ScanReport { findings: vec![finding("a.vxml_c", Risk::CompiledText)], ..ScanReport::default() };
        assert!(findings(&hud_only).is_empty());
    }
}
