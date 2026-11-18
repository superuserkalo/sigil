//! Keep the Locker in step with the addon folder (issue #1).
//!
//! The Locker reads its mod list once, on its first visit. Every path that changes the addon
//! folder (delete, Clear all mods, unmerge, an update replacing a file, restore from trash,
//! profiles, a Locker write) ends in an addon rescan, and the rescan calls [`addons_changed`]:
//! rows whose file left the folder go at once, on and off follow the folder, hero counts are
//! counted again, and a fresh read starts on a worker so new mods appear too.

use crate::app::State;
use crate::model::AddonRow;

/// What the Locker cares about in one slot: its file name, size and switch.
pub type Sig = Vec<(u16, String, u64, bool)>;

pub fn sig(addons: &[AddonRow]) -> Sig {
    let mut out: Sig = addons
        .iter()
        .map(|addon| (addon.slot, addon.file.clone(), addon.size_bytes, addon.enabled))
        .collect();
    out.sort();
    out
}

/// Drop rows the folder no longer has and follow each slot's switch. Returns true when a row
/// left or changed.
pub fn prune(view: &mut super::View, addons: &[AddonRow]) -> bool {
    let mut changed = false;
    let before = view.mods.len();
    view.mods.retain(|row| {
        addons
            .iter()
            .any(|addon| addon.slot == row.slot && same_file(&addon.file, &row.file))
    });
    changed |= view.mods.len() != before;
    for row in &mut view.mods {
        if let Some(addon) = addons
            .iter()
            .find(|addon| addon.slot == row.slot && same_file(&addon.file, &row.file))
            && (addon.enabled != row.enabled || addon.file != row.file)
        {
            row.enabled = addon.enabled;
            row.file = addon.file.clone();
            changed = true;
        }
    }
    if changed {
        for hero in &mut view.heroes {
            hero.mod_count = view
                .mods
                .iter()
                .filter(|row| row.heroes.contains(&hero.display_name))
                .count();
        }
    }
    changed
}

/// A switched-off file may carry a `.disabled` suffix; it is still the same mod.
fn same_file(a: &str, b: &str) -> bool {
    a.trim_end_matches(".disabled") == b.trim_end_matches(".disabled")
}

/// Called after every addon rescan (`data::apply_addon_library`).
pub fn addons_changed(st: &mut State) {
    if st.demo || !st.locker.loaded {
        return;
    }
    let now = sig(&st.addons.addons);
    prune(&mut st.locker, &st.addons.addons);
    match &st.locker.addons_sig {
        // The first rescan after a load only records where the folder stands.
        None => st.locker.addons_sig = Some(now),
        Some(seen) if *seen == now => {}
        Some(_) => {
            st.locker.addons_sig = Some(now);
            crate::data::start_locker_load(st);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::locker::{ModRow, View};

    fn addon(slot: u16, file: &str, enabled: bool) -> AddonRow {
        AddonRow {
            slot,
            file: file.to_string(),
            size_bytes: 10,
            chunk_files: Vec::new(),
            entry_count: None,
            enabled,
        }
    }

    fn row(slot: u16, file: &str, hero: &str) -> ModRow {
        ModRow {
            slot,
            file: file.to_string(),
            enabled: true,
            key: format!("k{slot}"),
            label: file.to_string(),
            heroes: vec![hero.to_string()],
            general: None,
            sounds: Vec::new(),
            parts: Default::default(),
        }
    }

    #[test]
    fn a_deleted_mod_leaves_the_locker_and_switches_follow() {
        let mut view = View::default();
        view.mods = vec![row(1, "pak01_dir.vpk", "Haze"), row(2, "pak02_dir.vpk", "Haze")];
        // Slot 01 was deleted; slot 02 was switched off.
        let addons = vec![addon(2, "pak02_dir.vpk", false)];
        assert!(prune(&mut view, &addons));
        assert_eq!(view.mods.len(), 1);
        assert_eq!(view.mods[0].slot, 2);
        assert!(!view.mods[0].enabled);
        // Nothing left to change.
        assert!(!prune(&mut view, &addons));
    }

    #[test]
    fn a_disabled_suffix_is_the_same_mod_until_it_goes() {
        let mut view = View::default();
        view.mods = vec![row(3, "pak03_dir.vpk", "Haze")];
        assert!(prune(&mut view, &[addon(3, "pak03_dir.vpk.disabled", false)]));
        assert_eq!(view.mods.len(), 1);
        assert!(prune(&mut view, &[]));
        assert!(view.mods.is_empty());
    }
}
