//! Headless scenes for parity wave 4, UI A: toasts and Send feedback (issues #4, #5), the
//! Locker after a delete (issue #1), and the Installed, Locker and Launch additions. Registered
//! in `main.rs` next to the app's own list.

use crate::app::State;
use crate::notify::{self, Notice};

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("w4a-toasts", toasts),
    ("w4a-toasts-info", toasts_info),
    ("w4a-feedback", feedback),
    ("w4a-feedback-sent", feedback_sent),
    ("w4a-locker-after-delete", locker_after_delete),
    ("w4a-order-search", order_search),
    ("w4a-order-place", order_place),
    ("w4a-backups", backups),
    ("w4a-update-cancel", update_cancel),
    ("w4a-analyze", analyze),
    ("w4a-cursor-reapply", cursor_reapply),
    ("w4a-locker-pool", locker_pool),
    ("w4a-overlap-ignore-all", crate::screens::conflicts::scene_ignore_all),
    ("w4a-locker-parts", crate::screens::locker::scene_parts),
    ("w4a-safety-gate", safety_gate),
    ("w4a-imprint", imprint),
    ("w4a-selection-menu", selection_menu),
    ("w4a-lists", lists),
    ("w4a-sections", sections),
    ("w4a-card-style", card_style),
    ("w4a-identify-name", crate::screens::installed::tools::scene_identify_name),
    ("w4a-update-scope", update_scope),
    ("w4a-installed-sound", installed_sound),
    // Sigil's own crash at the next start, and feedback on every error and warning toast.
    ("crash-offer", crash_offer),
    ("crash-offer-open", crash_offer_open),
    ("crash-offer-unclean", crash_offer_unclean),
    ("crash-sent", crash_sent),
    ("notify-feedback", crate::notify::scene_feedback),
];

/// The crash dialog after a panic, collapsed.
pub fn crash_offer(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::settings::crash::stage_scene(s, deadlock_core::telemetry::crash::KIND_PANIC, false);
}

/// The crash dialog after a panic, with what is sent shown and Always send ticked.
pub fn crash_offer_open(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::settings::crash::stage_scene(s, deadlock_core::telemetry::crash::KIND_PANIC, true);
    s.settings.telemetry.crash.always = true;
}

/// The crash dialog after a run that left no panic (a native crash or a forced quit).
pub fn crash_offer_unclean(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::settings::crash::stage_scene(s, deadlock_core::telemetry::crash::KIND_UNCLEAN, false);
}

/// The toast a player with crash reports on sees at start.
pub fn crash_sent(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::settings::crash::stage_sent(s);
}

/// Every toast kind at once: a success with Undo, a warning that reads longer, and a sticky
/// error with its action and Send feedback.
pub fn toasts(s: &mut State) {
    crate::app::scene_demo_installed(s);
    notify::push(s, Notice::success("Turned off Ashfall").undo(|_st: &mut State| {}));
    notify::push(
        s,
        Notice::warning("2 mods were not installed")
            .detail("Install them from Browse, then apply the profile again."),
    );
    notify::push(
        s,
        Notice::error("Could not write autoexec.cfg")
            .detail("Permission denied. Close the game and try again.")
            .action("Retry", |_st: &mut State| {}),
    );
}

/// An info toast, and a status line and a push from one action collapsing into one toast.
pub fn toasts_info(s: &mut State) {
    crate::app::scene_demo_installed(s);
    notify::push(s, Notice::info("Checked 12 mods. All up to date"));
    notify::push(s, Notice::success("Installed Ashfall").detail("Slot 03. It loads the next time the game starts."));
    // The same action's status line, worded differently: it never becomes a second toast.
    s.status = "Installed Ashfall. Slot 03. It loads the next time the game starts.".to_string();
    let line = std::mem::take(&mut s.status);
    if s.notices.status_is_news(&line, notify::classify(&line)) {
        notify::push(s, notify::from_status(&line));
    }
}

pub fn feedback(s: &mut State) {
    crate::app::scene_demo_installed(s);
    notify::feedback::stage_scene(s, false);
}

pub fn feedback_sent(s: &mut State) {
    crate::app::scene_demo_installed(s);
    notify::feedback::stage_scene(s, true);
}

/// The Locker after a mod left the addon folder: Haze's list no longer shows Haze Neon (slot
/// 11), deleted in Installed (issue #1).
pub fn locker_after_delete(s: &mut State) {
    crate::screens::locker::scene_list(s);
    let addons: Vec<crate::model::AddonRow> = s
        .locker
        .mods
        .iter()
        .filter(|row| row.slot != 11)
        .map(|row| crate::model::AddonRow {
            slot: row.slot,
            file: row.file.clone(),
            size_bytes: 1,
            chunk_files: Vec::new(),
            entry_count: None,
            enabled: row.enabled,
        })
        .collect();
    crate::screens::locker::sync::prune(&mut s.locker, &addons);
}

/// The load order dialog with a search typed (Deadlock Mod Manager's search field).
pub fn order_search(s: &mut State) {
    crate::screens::installed::wave_scenes::load_order_open(s);
    if let Some(dialog) = s.installed.order.as_mut() {
        dialog.query = rune::TextEdit::new("mi");
    }
}

/// The load order dialog with a place being typed on the third row (Grimoire's chip), and a
/// place out of range refused.
pub fn order_place(s: &mut State) {
    crate::screens::installed::wave_scenes::load_order_open(s);
    if let Some(dialog) = s.installed.order.as_mut()
        && let Some(slot) = dialog.order.get(2).copied()
    {
        dialog.picked = Some(slot);
        dialog.typing = Some((slot, rune::TextEdit::new("12")));
        dialog.typed_error = Some(crate::screens::installed::order::placed(&dialog.order, slot, "12").unwrap_err());
    }
}

/// A mod's backups, the second asking before it is put back (C10).
pub fn backups(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::backups::stage_scene(s);
}

/// An update running with two more queued: each chip and the toolbar offer Cancel (A5).
pub fn update_cancel(s: &mut State) {
    crate::screens::installed::scene_updates(s);
    s.installed.updates.queue = vec![2, 7];
}

/// Analyze the addons folder with something in every section, and an outdated HUD mod (C11).
pub fn analyze(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::checks::stage_scene(s);
}

/// Locker > Cursors after a game update put two stock files back under the active pack (C7).
pub fn cursor_reapply(s: &mut State) {
    crate::screens::wave3_scenes::locker_cursors(s);
    s.locker.cursors.reapply = Some(deadlock_core::cursors::Reapply {
        pack_id: "frost".to_string(),
        pack_name: "Frost Pointer".to_string(),
        restored_by_steam: vec!["cursor.bmp".to_string(), "cursor_wait.bmp".to_string()],
        pack_updated: false,
    });
}

/// The shuffle dialog with several skins per hero on and one of Haze's skins in the pool (C14).
pub fn locker_pool(s: &mut State) {
    crate::screens::locker::scene_shuffle(s);
    s.locker.prefs.multi_skin = true;
    s.locker.prefs.toggle_pool("vpk:haze-neon");
}

/// Turning on a mod that ships a program and a script: the safety gate.
pub fn safety_gate(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::safety_gate::stage_scene(s);
}

/// View imprint for slot 03.
pub fn imprint(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::imprint::stage_scene(s);
}

/// Three mods picked and the selection's More menu open.
pub fn selection_menu(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.selected = [2, 3, 9].into_iter().collect();
    s.locker.loaded = true;
    s.locker.categories = vec![deadlock_core::locker::LockerCategory {
        id: "favourites".to_string(),
        name: "Favourites".to_string(),
        keys: Vec::new(),
    }];
    s.installed.menu = Some(crate::screens::installed::Menu::Selection);
}

/// The Mod lists dialog with a list made, its icon and move buttons.
pub fn lists(s: &mut State) {
    crate::screens::installed::scene_lists(s);
    demo_lists(s);
}

fn demo_lists(s: &mut State) {
    use deadlock_core::mod_extras::ModList;
    let uid = |s: &State, slot: u16| {
        s.installed.notes.extras.mods.iter().find(|m| m.slot == slot).map(|m| m.uid.clone())
    };
    let ranked: Vec<String> = [1u16, 2, 3].iter().filter_map(|slot| uid(s, *slot)).collect();
    let fun: Vec<String> = [5u16, 7].iter().filter_map(|slot| uid(s, *slot)).collect();
    if !s.installed.notes.extras.lists.iter().any(|l| l.id == "ranked") {
        s.installed.notes.extras.lists.extend([
            ModList { id: "ranked".to_string(), name: "Ranked".to_string(), members: ranked, ..ModList::default() },
            ModList { id: "fun".to_string(), name: "Just for fun".to_string(), members: fun, ..ModList::default() },
        ]);
    }
}

/// Installed in sections by list.
pub fn sections(s: &mut State) {
    crate::app::scene_demo_installed(s);
    demo_lists(s);
    s.installed.by_list = true;
}

/// The View menu with the card style options, details line and slot number off.
pub fn card_style(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.installed.hide_details = true;
    s.installed.hide_slot = true;
    s.installed.menu = Some(crate::screens::installed::Menu::View);
}

/// Update all with Only enabled picked (DMM).
pub fn update_scope(s: &mut State) {
    crate::screens::installed::scene_updates(s);
    s.installed.updates.running = None;
    s.installed.update_scope = crate::screens::installed::UpdateScope::Enabled;
    let slots: Vec<u16> = s.addons.addons.iter().map(|a| a.slot).collect();
    s.installed.confirm = Some(crate::screens::installed::Confirm::UpdateAll(slots));
}

/// A sound mod's card with Play (the sample plays through Browse's player).
pub fn installed_sound(s: &mut State) {
    crate::app::scene_demo_installed(s);
    let slot = 4;
    let key = s.addons.addons.iter().find(|a| a.slot == slot).map(|a| crate::screens::installed::extras_key(s, a));
    if let Some(key) = key {
        let entry = s.installed.notes.extras.entry(key);
        entry.root_category = Some("Sound".to_string());
        entry.category = Some("Killstreak Music".to_string());
    }
}
