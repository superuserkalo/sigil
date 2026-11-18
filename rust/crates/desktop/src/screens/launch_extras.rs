//! What a modded launch does before Steam gets the game, beyond the launch options: Deadlock
//! Mod Manager 2.4's two launch-time features.
//!
//! - Random member per group. A mod list marked "One at random each launch" turns one of its
//!   members on and the others off, each launch. The pick is `mod_extras::random_picks`, which
//!   passes over the member on now and the last pick; this side keeps the pool to slots on disk
//!   and leaves map mods out (as DMM does), then records the picks (`record_picks`). The
//!   Locker's per-hero skin shuffle runs in the same step while the Locker page is on. Grimoire
//!   has only the Locker shuffle.
//! - Map mod question. Map mods replace a map and can keep you out of matchmaking. When any are
//!   on, a modded launch asks whether to turn them off first, with "Don't ask again" and a switch
//!   in Settings > Mods. Map mods are found by `mod_kinds::is_map_mod_vpk` on a worker while
//!   Installed or Browse is open; a launch only looks the answer up. Grimoire has no such check.
//!
//! Every switch here is the same rename the card switch does, behind Undo. The launch press is
//! the player's request; nothing here runs from an agent tool.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use deadlock_core::mod_extras::{self, Extras, SlotKey, random_picks};
use rune::{El, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::ui::{ActionKind, BODY, H1, MICRO, Ui};

/// Map mod answers by `(slot, size)`, from the worker.
pub type MapAnswer = ((u16, u64), bool);

/// The question a modded launch raises when map mods are on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapAsk {
    pub slots: Vec<u16>,
    /// The launch came from the Launch page, not the sidebar.
    pub page: bool,
    pub dont_ask: bool,
}

#[derive(Default)]
pub struct LaunchExtras {
    /// Is a slot file a map mod, by `(slot, size)`.
    pub maps: HashMap<(u16, u64), bool>,
    pub job: Job<Vec<MapAnswer>>,
    pub tried: HashSet<(u16, u64)>,
    pub ask: Option<MapAsk>,
    /// The launch's repair step started its worker (`installed::repair`).
    pub repairing: bool,
}

// --- random groups ------------------------------------------------------------------------

/// One slot file on disk as the roll reads it: slot, size, on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnDisk {
    pub slot: u16,
    pub size: u64,
    pub enabled: bool,
}

/// The switches the random lists ask for this launch, and the picks to record. Members are kept
/// to slot files on disk (slot and size) and map mods are never picked, by rolling over a copy
/// of the store with the rest taken out. `random_picks` passes over the member that is on now
/// and the list's last pick when another can be picked. Members of a list that are on and not
/// picked go off.
pub fn roll(
    extras: &Extras,
    disk: &[OnDisk],
    maps: &BTreeSet<u16>,
    seed: u64,
) -> (BTreeMap<u16, bool>, Vec<(String, SlotKey)>) {
    let present = |slot: u16, size: u64| disk.iter().any(|d| d.slot == slot && d.size == size);
    let on = |slot: u16| disk.iter().any(|d| d.slot == slot && d.enabled);
    let mut pool = extras.clone();
    for list in pool.lists.iter_mut().filter(|l| l.random_each_launch) {
        list.members.retain(|uid| {
            extras
                .find_uid(uid)
                .is_some_and(|m| present(m.slot, m.size_bytes) && !maps.contains(&m.slot))
        });
    }
    let picks = random_picks(&pool, seed, |key: &SlotKey| on(key.slot));
    let mut wanted = BTreeMap::new();
    for (list_id, key) in &picks {
        let Some(list) = pool.list(list_id) else { continue };
        for slot in list.members.iter().filter_map(|uid| pool.slot_for_uid(uid)) {
            if slot != key.slot && on(slot) {
                wanted.insert(slot, false);
            }
        }
        if !on(key.slot) {
            wanted.insert(key.slot, true);
        }
    }
    (wanted, picks)
}

/// The launch step: roll the random lists and the Locker's shuffle. A failure never blocks the
/// launch; it becomes a line the toast shows.
pub fn roll_before_launch(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    if st.prefs.page_on(crate::model::Screen::Locker) {
        crate::data::shuffle_before_launch(st);
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let extras = st.installed.notes.extras.clone();
    if !extras.lists.iter().any(|l| l.random_each_launch) {
        return;
    }
    let disk: Vec<OnDisk> = st
        .addons
        .addons
        .iter()
        .map(|a| OnDisk { slot: a.slot, size: a.size_bytes, enabled: a.enabled })
        .collect();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let (wanted, picks) = roll(&extras, &disk, &all_map_slots(st), seed);
    if wanted.is_empty() {
        return;
    }
    let on = wanted.values().filter(|v| **v).count();
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            if !report.done.is_empty() {
                st.installed.last_undo = Some(crate::screens::installed::Undo::Toggle(report.done));
                st.installed.undo_fresh = true;
            }
            // Remember each list's pick so the next launch passes over it.
            let path = crate::screens::installed::extras_path();
            if mod_extras::edit_extras(&path, |x| mod_extras::record_picks(x, &picks)).is_ok() {
                mod_extras::record_picks(&mut st.installed.notes.extras, &picks);
            }
            st.status = if on == 1 {
                tr("Rolled 1 random mod for this launch.").to_string()
            } else {
                fill(tr("Rolled {} random mods for this launch."), &[&on.to_string()])
            };
        }
        Err(error) => st.status = error.to_string(),
    }
    crate::data::scan_addons_now(st);
}

// --- map mods ---------------------------------------------------------------------------

/// Enabled slots known to be map mods.
pub fn map_slots(s: &State) -> BTreeSet<u16> {
    s.addons
        .addons
        .iter()
        .filter(|a| a.enabled && is_map(s, a))
        .map(|a| a.slot)
        .collect()
}

/// Every slot known to be a map mod, on or off: a random list never picks one (A7).
pub fn all_map_slots(s: &State) -> BTreeSet<u16> {
    s.addons.addons.iter().filter(|a| is_map(s, a)).map(|a| a.slot).collect()
}

/// A map mod by its files (`mod_kinds::is_map_mod_vpk`, read on the worker) or by its
/// GameBanana category (Maps), as Deadlock Mod Manager's `isMap` does.
pub fn is_map(s: &State, addon: &crate::model::AddonRow) -> bool {
    s.launch_extras.maps.get(&(addon.slot, addon.size_bytes)).copied().unwrap_or(false)
        || crate::screens::installed::in_maps_category(s, addon)
}

/// Read enabled slot files the cache lacks, on a worker, while the warning or a random list
/// needs it. Never in the demo or a headless run.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.launch_extras.job.try_take() {
        if let Ok(answers) = result {
            st.launch_extras.maps.extend(answers);
        }
        changed = true;
    }
    if st.demo || rune::is_headless() || !st.addons.scanned || st.launch_extras.job.is_running() {
        return changed;
    }
    let wanted = options::get().map_mod_warning || st.installed.notes.extras.lists.iter().any(|l| l.random_each_launch);
    if !wanted {
        return changed;
    }
    let dir = st.addons_dir.clone().map(PathBuf::from).unwrap_or_default();
    // Off members of a random list are read too: the roll may turn one on.
    let listed: BTreeSet<u16> = st
        .installed
        .notes
        .extras
        .lists
        .iter()
        .filter(|l| l.random_each_launch)
        .flat_map(|l| l.members.iter().filter_map(|uid| st.installed.notes.extras.slot_for_uid(uid)))
        .collect();
    let todo: Vec<((u16, u64), PathBuf)> = st
        .addons
        .addons
        .iter()
        .filter(|a| (a.enabled || listed.contains(&a.slot)) && !st.launch_extras.tried.contains(&(a.slot, a.size_bytes)))
        .take(16)
        .map(|a| ((a.slot, a.size_bytes), a.path(&dir)))
        .collect();
    if todo.is_empty() {
        return changed;
    }
    for (key, _) in &todo {
        st.launch_extras.tried.insert(*key);
    }
    let wake = st.wake.clone();
    st.launch_extras.job.start(wake, move || {
        Ok(todo
            .into_iter()
            .map(|(key, path)| (key, deadlock_core::mod_kinds::is_map_mod_vpk(&path).unwrap_or(false)))
            .collect())
    });
    changed
}

/// Before a modded launch: ask when map mods are on and the warning is. True when the launch
/// waits for the answer.
pub fn ask_first(st: &mut State, page: bool) -> bool {
    if !options::get().map_mod_warning {
        return false;
    }
    let slots: Vec<u16> = map_slots(st).into_iter().collect();
    if slots.is_empty() {
        return false;
    }
    st.launch_dialog = false;
    st.launch_extras.ask = Some(MapAsk { slots, page, dont_ask: false });
    true
}

fn answer(st: &mut State, turn_off: bool) {
    let Some(ask) = st.launch_extras.ask.take() else { return };
    if ask.dont_ask {
        options::edit(st, |o| o.map_mod_warning = false);
    }
    if turn_off
        && !st.demo
        && let Some(game_root) = st.game_root.clone()
    {
        {
            let wanted: BTreeMap<u16, bool> = ask.slots.iter().map(|slot| (*slot, false)).collect();
            match deadlock_core::addons::set_many(&game_root, &wanted, true) {
                Ok(report) => {
                    if !report.done.is_empty() {
                        st.installed.last_undo = Some(crate::screens::installed::Undo::Toggle(report.done));
                        st.installed.undo_fresh = true;
                    }
                }
                Err(error) => {
                    st.status = error.to_string();
                    return;
                }
            }
            crate::data::scan_addons_now(st);
        }
    }
    crate::screens::launch_progress::begin_checked(st, false, ask.page);
}

/// The question.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let ask = s.launch_extras.ask.as_ref()?;
    let close = |st: &mut State| st.launch_extras.ask = None;
    let count = ask.slots.len();
    let title = if count == 1 {
        tr("A map mod is on").to_string()
    } else {
        fill(tr("{} map mods are on"), &[&count.to_string()])
    };
    let mut names = col().w_full().gap(4.0);
    for slot in &ask.slots {
        let name = s
            .addons
            .addons
            .iter()
            .find(|a| a.slot == *slot)
            .map(|a| crate::screens::installed::display_name(s, a))
            .unwrap_or_else(|| deadlock_core::addons::slot_label(*slot));
        names = names.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(deadlock_core::addons::slot_label(*slot)).font(MICRO).color(u.faint).w(28.0))
                .child(text(name).font(BODY).color(u.prose).truncate()),
        );
    }
    let dont_ask = ask.dont_ask;
    Some(
        u.dialog_shell()
            .key("launch-map-mods")
            .role("dialog")
            .label(title.clone())
            .w(480.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .child(text(title).font(H1).color(u.ink))
            .child(
                text(tr("Map mods replace a map. They can keep you out of matchmaking or break a match. Turn them off for this launch?"))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(names)
            .child(
                row()
                    .key("launch-map-dont-ask")
                    .gap(8.0)
                    .center_y()
                    .focusable(true)
                    .role("checkbox")
                    .checked(dont_ask)
                    .label(tr("Don't ask again"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        if let Some(ask) = st.launch_extras.ask.as_mut() {
                            ask.dont_ask = !ask.dont_ask;
                        }
                    })
                    .child(crate::screens::installed::check_box_el(u, dont_ask))
                    .child(text(tr("Don't ask again")).font(BODY).color(u.muted)),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key("launch-map-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(spacer())
                    .child(
                        u.action(tr("Launch anyway"), ActionKind::Secondary)
                            .key("launch-map-anyway")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| answer(st, false)),
                    )
                    .child(
                        u.action(tr("Turn them off and launch"), ActionKind::Primary)
                            .key("launch-map-off")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| answer(st, true)),
                    ),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::mod_extras::{ModExtra, ModList};

    fn extras(slots: &[u16], random: bool) -> Extras {
        let mut x = Extras::default();
        let mut uids = Vec::new();
        for slot in slots {
            let uid = format!("u{slot}");
            x.mods.push(ModExtra { uid: uid.clone(), slot: *slot, size_bytes: 100 + *slot as u64, ..ModExtra::default() });
            uids.push(uid);
        }
        x.lists.push(ModList {
            id: "skins".into(),
            name: "Skins".into(),
            members: uids,
            random_each_launch: random,
            last_pick: None,
        });
        x
    }

    fn disk(on: &[u16], all: &[u16]) -> Vec<OnDisk> {
        all.iter().map(|s| OnDisk { slot: *s, size: 100 + *s as u64, enabled: on.contains(s) }).collect()
    }

    #[test]
    fn one_member_ends_up_on_and_the_rest_off() {
        let x = extras(&[3, 5, 8], true);
        for seed in 0..32 {
            let (wanted, picks) = roll(&x, &disk(&[3, 5], &[3, 5, 8]), &BTreeSet::new(), seed);
            assert_eq!(picks.len(), 1);
            let mut on: BTreeSet<u16> = [3, 5].into_iter().collect();
            for (slot, enable) in &wanted {
                if *enable { on.insert(*slot); } else { on.remove(slot); }
            }
            assert_eq!(on.len(), 1, "seed {seed}: {wanted:?}");
        }
    }

    #[test]
    fn the_member_on_now_is_avoided_when_another_can_be_picked() {
        let x = extras(&[3, 5], true);
        for seed in 0..16 {
            let (wanted, _) = roll(&x, &disk(&[3], &[3, 5]), &BTreeSet::new(), seed);
            assert_eq!(wanted.get(&5), Some(&true), "seed {seed}");
            assert_eq!(wanted.get(&3), Some(&false));
        }
    }

    #[test]
    fn the_gamebanana_maps_category_marks_a_map_mod() {
        use crate::screens::installed::maps_category;
        assert!(maps_category(Some("Maps"), Some("Arena")));
        assert!(maps_category(None, Some("maps")));
        assert!(!maps_category(Some("Skins"), Some("Haze")));
        assert!(!maps_category(None, None));
    }

    #[test]
    fn lists_off_gone_files_and_map_mods_are_left_alone() {
        assert!(roll(&extras(&[3, 5], false), &disk(&[3], &[3, 5]), &BTreeSet::new(), 1).0.is_empty());
        // Slot 5's file is not on disk: one member is left, nothing to choose.
        assert!(roll(&extras(&[3, 5], true), &disk(&[3], &[3]), &BTreeSet::new(), 1).0.is_empty());
        // Slot 5 is a map mod: never picked and never switched.
        let maps: BTreeSet<u16> = [5].into_iter().collect();
        assert!(roll(&extras(&[3, 5], true), &disk(&[3, 5], &[3, 5]), &maps, 1).0.is_empty());
    }
}
