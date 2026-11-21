//! Heavy scenes for measuring frame cost, not for screenshots: a library the size a long-time
//! player has. Run one headless with a pointer script to see what a commit costs:
//!
//! `dah-desktop --scene installed-stress --script sweep.json --frames 500 --fps 120`
//!
//! and read `frame cpu`, `layout nodes` and (with `RUNE_COMMIT_BUDGET_MS=4`) each slow commit.

use crate::app::State;
use crate::model::Screen;

/// Mods in a stress library: about what a player who keeps everything installed has.
/// `SIGIL_STRESS_MODS` sets another count (slots past 99 live in Sigil's extra addon folders).
pub const STRESS_MODS: u16 = 96;

fn stress_mods() -> u16 {
    std::env::var("SIGIL_STRESS_MODS").ok().and_then(|n| n.parse().ok()).unwrap_or(STRESS_MODS).clamp(1, 990)
}

pub const SCENES: [(&str, fn(&mut State)); 2] = [
    ("installed-stress", installed_stress),
    ("installed-list-stress", installed_list_stress),
];

/// The demo library grown to `STRESS_MODS` slots: every demo mod copied into the free slots,
/// with its library entry and notes, so names, art, chips and lists all have work to do.
pub fn installed_stress(s: &mut State) {
    crate::app::scene_demo_installed(s);
    grow(s, stress_mods());
}

/// The demo library grown to `mods` slots.
pub fn installed_stress_of(s: &mut State, mods: u16) {
    crate::app::scene_demo_installed(s);
    grow(s, mods);
}

/// The stress library in the list layout.
pub fn installed_list_stress(s: &mut State) {
    crate::screens::installed::scene_list(s);
    s.installed.hovered = None;
    grow(s, stress_mods());
}

fn grow(s: &mut State, mods: u16) {
    let rows = s.addons.addons.clone();
    let metas = s.installed.library.mods.clone();
    let extras = s.installed.notes.extras.mods.clone();
    if rows.is_empty() {
        return;
    }
    let taken: Vec<u16> = rows.iter().map(|row| row.slot).collect();
    let mut n = 0usize;
    for slot in 1..=mods {
        if taken.contains(&slot) {
            continue;
        }
        let source = &rows[n % rows.len()];
        n += 1;
        let size = source.size_bytes + u64::from(slot) * 4096;
        let mut row = source.clone();
        row.slot = slot;
        row.size_bytes = size;
        row.file = format!("pak{:02}_dir.vpk", slot % 100);
        row.enabled = slot % 5 != 0;
        if let Some(meta) = metas.iter().find(|m| m.slot == source.slot) {
            let mut meta = meta.clone();
            meta.slot = slot;
            meta.size_bytes = size;
            meta.name = format!("{} {slot}", meta.name);
            meta.gamebanana_id = Some(700_000 + u64::from(slot));
            s.installed.library.mods.push(meta);
        }
        if let Some(extra) = extras.iter().find(|e| e.slot == source.slot) {
            let mut extra = extra.clone();
            extra.slot = slot;
            extra.size_bytes = size;
            extra.uid = format!("stress-{slot:02}");
            extra.gamebanana_id = Some(700_000 + u64::from(slot));
            s.installed.notes.extras.mods.push(extra);
        }
        s.addons.addons.push(row);
    }
    s.addons.addons.sort_by_key(|row| row.slot);
    // Every mod has a GameBanana page and the update check has run, as for most players.
    crate::screens::installed::stage_updates_for_scene(s);
    s.screen = Screen::Installed;
}
