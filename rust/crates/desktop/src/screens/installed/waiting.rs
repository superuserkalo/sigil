//! Changes that wait for Deadlock to close (`deadlock_core::pending`).
//!
//! While the game runs it holds its mod files open, so a delete or a switch off would fail on
//! Windows. Installed asks first ("Delete when Deadlock closes" or Cancel, with "Do this from now
//! on"), queues the change in Sigil's app data, marks the card and row, and counts the queue in
//! the status row. When the process monitor sees the game gone, the queue runs on a worker
//! (`pending::apply_all`, the only game write here) and one toast says what happened.
//!
//! These are the player's own actions on this page. No agent tool reaches this module.

use std::path::PathBuf;

use deadlock_core::import;
use deadlock_core::pending::{self, Applied, ApplyReport, PendingKind, PendingQueue};
use rune::{Color, El, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

/// The `skip_confirm` id of "Do this from now on".
pub const SKIP_ASK: &str = "installed.wait_for_close";

#[derive(Default)]
pub struct Waiting {
    pub queue: PendingQueue,
    /// The game root the queue was read for; a new root reads it again.
    pub loaded_for: Option<Option<PathBuf>>,
    /// The open "when Deadlock closes" question.
    pub ask: Option<Ask>,
    /// "Do this from now on" is ticked in the question.
    pub remember: bool,
    /// The review dialog is open.
    pub review: bool,
    pub job: Job<ApplyReport>,
    /// Run the queue the next time the game is seen closed: set at load and whenever the game
    /// is seen running.
    pub armed: bool,
}

/// A change the player asked for while the game holds the files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub kind: PendingKind,
    pub slots: Vec<u16>,
    /// The change was tried and the OS said a file was in use.
    pub after_error: bool,
}

fn queue_file() -> PathBuf {
    pending::pending_path(&crate::data::data_dir())
}

/// True while the process monitor sees Deadlock (or, before it has read the process list, the
/// launch said it started).
pub fn game_running(st: &State) -> bool {
    if st.launch.monitor.is_started() && st.launch.monitor.has_checked() {
        st.launch.monitor.running()
    } else {
        st.settings.game_running
    }
}

/// True when a delete or switch off should wait instead of running now.
pub fn must_wait(st: &State) -> bool {
    !st.demo && game_running(st)
}

/// The guard for a change that cannot wait faithfully (load order, updates, merges, solo
/// tests): while the game holds the files it does not start, and one line says why. Returns
/// true when the change has to stop.
pub fn refuse_held(st: &mut State, reason: &str) -> bool {
    if !must_wait(st) {
        return false;
    }
    st.installed.menu = None;
    crate::notify::push(
        st,
        crate::notify::Notice::warning(tr("Close Deadlock first")).detail(reason.to_string()),
    );
    true
}

/// Set by a worker that queued changes itself; the next poll reads the queue again.
static DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From a worker (an install with Switch variants on): queue the switch off of `slots`, whose
/// files the game holds, for when it closes. The next poll shows them.
pub fn hold_from_worker(root: &std::path::Path, library: &deadlock_core::import::Library, slots: &[u16]) {
    let file = queue_file();
    for &slot in slots {
        let meta = library.mods.iter().find(|m| m.slot == slot);
        let name = meta
            .map(|m| m.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| deadlock_core::addons::slot_label(slot));
        if let Ok(change) = pending::new_change(root, PendingKind::TurnOff, slot, &name, meta.cloned()) {
            let _ = pending::enqueue(&file, root, change);
        }
    }
    if !slots.is_empty() {
        DIRTY.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The waiting change of a slot.
pub fn for_slot(s: &State, slot: u16) -> Option<&pending::PendingChange> {
    s.installed.waiting.queue.for_slot(slot)
}

/// Ask whether `slots` should change when the game closes, or queue at once when the player
/// said so before.
pub fn offer(st: &mut State, kind: PendingKind, slots: Vec<u16>, after_error: bool) {
    st.installed.menu = None;
    st.installed.confirm = None;
    if slots.is_empty() {
        return;
    }
    if st.prefs.skip_confirm.contains(SKIP_ASK) {
        queue_now(st, kind, &slots);
        return;
    }
    st.installed.waiting.remember = false;
    st.installed.waiting.ask = Some(Ask { kind, slots, after_error });
}

/// Record the changes; nothing in the game folder moves now.
pub fn queue_now(st: &mut State, kind: PendingKind, slots: &[u16]) {
    st.installed.waiting.ask = None;
    let Some(root) = st.game_root.clone() else {
        st.status = tr("Set the game folder in Settings first.").to_string();
        return;
    };
    let file = queue_file();
    let mut queued = 0usize;
    let mut errors = Vec::new();
    for &slot in slots {
        let addon = st.addons.addons.iter().find(|a| a.slot == slot);
        let name = addon
            .map(|a| super::display_name(st, a))
            .unwrap_or_else(|| deadlock_core::addons::slot_label(slot));
        let meta = addon.and_then(|a| super::meta_for(st, a).cloned());
        match pending::new_change(&root, kind, slot, &name, meta).and_then(|change| pending::enqueue(&file, &root, change)) {
            Ok(queue) => {
                st.installed.waiting.queue = queue;
                queued += 1;
            }
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }
    let line = match (kind, queued) {
        (_, 0) => String::new(),
        (PendingKind::Delete, 1) => tr("Deletes when Deadlock closes.").to_string(),
        (PendingKind::Delete, n) => fill(tr("{} mods delete when Deadlock closes."), &[&n.to_string()]),
        (PendingKind::TurnOff, 1) => tr("Turns off when Deadlock closes.").to_string(),
        (PendingKind::TurnOff, n) => fill(tr("{} mods turn off when Deadlock closes."), &[&n.to_string()]),
    };
    st.status = if errors.is_empty() {
        line
    } else {
        format!("{line} {}", errors.join("; ")).trim().to_string()
    };
}

/// Take a change out of the queue: the player's undo.
pub fn cancel(st: &mut State, id: u64) {
    let Some(root) = st.game_root.clone() else { return };
    let name = st.installed.waiting.queue.changes.iter().find(|c| c.id == id).map(|c| c.name.clone());
    match pending::cancel(&queue_file(), &root, id) {
        Ok(queue) => {
            st.installed.waiting.queue = queue;
            if let Some(name) = name {
                st.status = fill(tr("{} no longer waits for Deadlock to close."), &[&name]);
            }
        }
        Err(error) => st.status = error.to_string(),
    }
}

fn cancel_all(st: &mut State) {
    let Some(root) = st.game_root.clone() else { return };
    match pending::cancel_all(&queue_file(), &root) {
        Ok(queue) => {
            st.installed.waiting.queue = queue;
            st.installed.waiting.review = false;
            st.status = tr("Nothing waits for Deadlock to close.").to_string();
        }
        Err(error) => st.status = error.to_string(),
    }
}

fn reload(st: &mut State) {
    let w = &mut st.installed.waiting;
    w.loaded_for = Some(st.game_root.clone());
    w.armed = true;
    let Some(root) = st.game_root.clone() else {
        w.queue = PendingQueue::default();
        return;
    };
    match pending::load(&queue_file(), &root) {
        Ok(loaded) => {
            st.installed.waiting.queue = loaded.queue;
            if !loaded.rejected.is_empty() {
                crate::notify::push(
                    st,
                    crate::notify::Notice::warning(tr("Waiting changes were dropped"))
                        .detail(loaded.rejected.join(" ")),
                );
            }
        }
        Err(error) => {
            crate::notify::push(
                st,
                crate::notify::Notice::warning(tr("Waiting changes could not be read")).detail(error.to_string()),
            );
        }
    }
}

/// Every poll: read the queue once per game root, run it when the game is seen closed, and
/// take the worker's report.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if st.demo {
        return false;
    }
    if DIRTY.swap(false, std::sync::atomic::Ordering::SeqCst) {
        let before = st.installed.waiting.queue.len();
        reload(st);
        let added = st.installed.waiting.queue.len().saturating_sub(before);
        if added > 0 {
            crate::notify::push(
                st,
                crate::notify::Notice::info(tr("Other variants turn off when Deadlock closes"))
                    .detail(fill(tr("Deadlock has their files open. {} waiting."), &[&added.to_string()])),
            );
        }
        changed = true;
    }
    if st.installed.waiting.loaded_for.as_ref() != Some(&st.game_root) {
        reload(st);
        changed = true;
    }
    if let Some(result) = st.installed.waiting.job.try_take() {
        finish(st, result);
        changed = true;
    }
    if rune::is_headless() || !st.launch.monitor.is_started() || !st.launch.monitor.has_checked() {
        return changed;
    }
    if st.launch.monitor.running() {
        st.installed.waiting.armed = true;
        return changed;
    }
    let w = &st.installed.waiting;
    if w.armed && !w.queue.is_empty() && !w.job.is_running() {
        start_apply(st);
        changed = true;
    }
    changed
}

/// Run the queue on the worker, then drop the library entries of the deleted mods, as an
/// immediate delete does.
fn start_apply(st: &mut State) {
    st.installed.waiting.armed = false;
    let Some(root) = st.game_root.clone() else { return };
    let file = queue_file();
    let trash_root = super::trash_dir();
    let library = super::library_file();
    let wake = st.wake.clone();
    st.installed.waiting.job.start(wake, move || {
        let report = pending::apply_all(&file, &root, &trash_root, true).map_err(|e| e.to_string())?;
        let gone: Vec<u16> = report
            .applied
            .iter()
            .filter_map(|(_, applied)| match applied {
                Applied::Trashed(entry) => Some(entry.manifest.slot),
                Applied::TurnedOff(_) => None,
            })
            .collect();
        if !gone.is_empty() {
            let mut store = import::read_library(&library).unwrap_or_default();
            store.mods.retain(|meta| !gone.contains(&meta.slot));
            let _ = import::write_library(&library, &store);
        }
        Ok(report)
    });
}

fn finish(st: &mut State, result: Result<ApplyReport, String>) {
    if let Some(root) = st.game_root.clone()
        && let Ok(loaded) = pending::load(&queue_file(), &root)
    {
        st.installed.waiting.queue = loaded.queue;
    }
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            crate::notify::push(
                st,
                crate::notify::Notice::warning(tr("Waiting changes did not run")).detail(error),
            );
            return;
        }
    };
    if report.is_empty() {
        return;
    }
    let deleted: Vec<PathBuf> = report
        .applied
        .iter()
        .filter_map(|(_, a)| match a {
            Applied::Trashed(entry) => Some(entry.dir.clone()),
            Applied::TurnedOff(_) => None,
        })
        .collect();
    let switched: Vec<_> = report
        .applied
        .iter()
        .filter_map(|(_, a)| match a {
            Applied::TurnedOff(result) => Some(result.clone()),
            Applied::Trashed(_) => None,
        })
        .collect();
    for (change, _) in &report.applied {
        st.installed.selected.remove(&change.slot);
    }
    let mut parts = Vec::new();
    if !deleted.is_empty() {
        parts.push(fill(tr("Deleted {}."), &[&deleted.len().to_string()]));
    }
    if !switched.is_empty() {
        parts.push(fill(tr("Turned off {}."), &[&switched.len().to_string()]));
    }
    for (_, why) in &report.skipped {
        parts.push(why.clone());
    }
    for (change, error) in &report.failed {
        parts.push(fill(tr("{} still waits: {}"), &[&change.name, error]));
    }
    parts.extend(report.rejected.iter().cloned());
    let detail = parts.join(" ");
    let clean = report.skipped.is_empty() && report.failed.is_empty() && report.rejected.is_empty();
    let title = tr("Deadlock closed: waiting changes ran");
    let notice = if clean {
        crate::notify::Notice::success(title)
    } else {
        crate::notify::Notice::warning(title)
    }
    .detail(detail);
    // One kind of change can be put back from the toast, like the immediate action.
    let undo = match (deleted.is_empty(), switched.is_empty()) {
        (false, true) => Some(super::Undo::Delete(deleted)),
        (true, false) => Some(super::Undo::Toggle(switched)),
        _ => None,
    };
    let notice = match undo {
        Some(undo) => {
            st.installed.last_undo = Some(undo.clone());
            notice.undo(move |st: &mut State| super::apply_undo(st, undo))
        }
        None => notice,
    };
    crate::notify::push(st, notice);
    super::rescan(st);
}

// --- views ---------------------------------------------------------------------------------

/// "{n} changes waiting for Deadlock to close", first in the status row. Opens the review.
pub fn status_chip(s: &State, u: &Ui) -> Option<El> {
    let w = &s.installed.waiting;
    if w.queue.is_empty() && !w.job.is_running() {
        return None;
    }
    let n = w.queue.len();
    let label = if w.job.is_running() {
        tr("Running waiting changes").to_string()
    } else if n == 1 {
        tr("1 change waiting for Deadlock to close").to_string()
    } else {
        fill(tr("{} changes waiting for Deadlock to close"), &[&n.to_string()])
    };
    let failed = w.queue.changes.iter().any(|c| c.error.is_some());
    Some(
        u.state_chip(
            &icons::HISTORY,
            &label,
            if failed { crate::ui::Tone::Warn } else { crate::ui::Tone::Accent },
        )
        .key("installed-waiting")
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.installed.waiting.review = true),
    )
}

fn waits_line(kind: PendingKind) -> &'static str {
    match kind {
        PendingKind::Delete => tr("Deletes when Deadlock closes"),
        PendingKind::TurnOff => tr("Turns off when Deadlock closes"),
    }
}

/// The mark on a card and a list row whose slot has a waiting change. It sits first in the
/// mark row (marks are fixed 28 point slots, so nothing moves). Pressing it cancels the change.
pub fn mark(s: &State, u: &Ui, slot: u16) -> Option<El> {
    let change = for_slot(s, slot)?;
    let id = change.id;
    let help = match &change.error {
        Some(error) => format!("{} ({error}). {}", waits_line(change.kind), tr("Click to cancel.")),
        None => format!("{}. {}", waits_line(change.kind), tr("Click to cancel.")),
    };
    Some(
        el().key(format!("installed-waiting-{slot:02}"))
            .size(28.0, 28.0)
            .center()
            .radius(crate::ui::r(4.0))
            .fill(Color::BLACK.alpha(0.6))
            .hover(Color::BLACK.alpha(0.78))
            .focusable(true)
            .role("button")
            .label(waits_line(change.kind))
            .help(help)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| cancel(st, id))
            .child(
                icon(&icons::HISTORY, 14.0)
                    .color(if change.error.is_some() { u.amber } else { Color::WHITE.alpha(0.9) })
                    .a11y_hidden(),
            ),
    )
}

/// The question or the review, when one is open.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(ask) = &s.installed.waiting.ask {
        return Some(ask_dialog(s, u, ask));
    }
    s.installed.waiting.review.then(|| review_dialog(s, u))
}

fn name_of(s: &State, slot: u16) -> String {
    s.addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| super::display_name(s, a))
        .unwrap_or_else(|| deadlock_core::addons::slot_label(slot))
}

fn ask_dialog(s: &State, u: &Ui, ask: &Ask) -> El {
    let close = |st: &mut State| st.installed.waiting.ask = None;
    let title = match (ask.kind, ask.slots.as_slice()) {
        (PendingKind::Delete, [one]) => fill(tr("Delete {}?"), &[&name_of(s, *one)]),
        (PendingKind::Delete, many) => fill(tr("Delete {} mods?"), &[&many.len().to_string()]),
        (PendingKind::TurnOff, [one]) => fill(tr("Turn off {}?"), &[&name_of(s, *one)]),
        (PendingKind::TurnOff, many) => fill(tr("Turn off {} mods?"), &[&many.len().to_string()]),
    };
    let body = match (ask.kind, ask.after_error) {
        (PendingKind::Delete, false) => {
            tr("Deadlock is running and has the files open. Sigil moves them to its trash as soon as the game closes.")
        }
        (PendingKind::Delete, true) => {
            tr("Deadlock or another program has the files open. Sigil moves them to its trash as soon as the game closes.")
        }
        (PendingKind::TurnOff, false) => {
            tr("Deadlock is running and has the file open. Sigil turns it off as soon as the game closes.")
        }
        (PendingKind::TurnOff, true) => {
            tr("Deadlock or another program has the file open. Sigil turns it off as soon as the game closes.")
        }
    };
    let confirm = match ask.kind {
        PendingKind::Delete => tr("Delete when Deadlock closes"),
        PendingKind::TurnOff => tr("Turn off when Deadlock closes"),
    };
    let mut list = col().w_full().gap(4.0);
    for &slot in ask.slots.iter().take(6) {
        list = list.child(
            row()
                .w_full()
                .h(28.0)
                .gap(12.0)
                .center_y()
                .child(text(deadlock_core::addons::slot_label(slot)).font(crate::ui::SMALL_NUM).color(u.faint).w(28.0))
                .child(text(name_of(s, slot)).font(BODY).color(u.prose).truncate().grow(1.0).min_w(0.0)),
        );
    }
    if ask.slots.len() > 6 {
        list = list.child(text(fill(tr("and {} more"), &[&(ask.slots.len() - 6).to_string()])).font(SMALL).color(u.faint));
    }
    let on = s.installed.waiting.remember;
    let remember = row()
        .key("installed-wait-remember")
        .h(32.0)
        .gap(8.0)
        .center_y()
        .focusable(true)
        .role("checkbox")
        .checked(on)
        .label(tr("Do this from now on"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.installed.waiting.remember = !st.installed.waiting.remember)
        .child(super::check_box(u, on, false))
        .child(text(tr("Do this from now on")).font(BODY).color(u.muted).a11y_hidden());
    let kind = ask.kind;
    let slots = ask.slots.clone();
    u.dialog_shell()
        .key("installed-wait-dialog")
        .role("dialog")
        .label(title.clone())
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .child(text(title).font(H1).color(u.ink).wrap())
        .child(text(body).font(BODY).color(u.prose).wrap().max_w(432.0))
        .child(list)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(remember)
                .child(spacer())
                .child(u.action(tr("Cancel"), ActionKind::Ghost).key("installed-wait-cancel").cursor(rune::Cursor::Pointer).on_press(close))
                .child(
                    u.action(confirm, ActionKind::Primary)
                        .key("installed-wait-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            if st.installed.waiting.remember {
                                crate::prefs::edit(st, |p| {
                                    p.skip_confirm.insert(SKIP_ASK.to_string());
                                });
                            }
                            queue_now(st, kind, &slots);
                        }),
                ),
        )
}

fn review_dialog(s: &State, u: &Ui) -> El {
    let close = |st: &mut State| st.installed.waiting.review = false;
    let w = &s.installed.waiting;
    let running = game_running(s);
    let mut list = u.panel().w_full();
    for change in &w.queue.changes {
        let id = change.id;
        let (line, color) = match &change.error {
            Some(error) => (error.clone(), u.amber),
            None => (waits_line(change.kind).to_string(), u.muted),
        };
        list = list.child(
            row()
                .key(format!("installed-wait-row-{id}"))
                .w_full()
                .h(48.0)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .child(
                    text(deadlock_core::addons::slot_label(change.slot))
                        .font(crate::ui::SMALL_NUM)
                        .color(u.faint)
                        .w(28.0),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(change.name.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(text(line.clone()).font(SMALL).color(color).truncate().help_when_truncated(line)),
                )
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key(format!("installed-wait-undo-{id}"))
                        .h(28.0)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| cancel(st, id)),
                ),
        );
    }
    let busy = w.job.is_running() || s.demo;
    let try_now = u
        .action(tr("Run now"), ActionKind::Secondary)
        .key("installed-wait-run")
        .help(if running { tr("Close Deadlock first.") } else { tr("Run the waiting changes now.") })
        .cursor(rune::Cursor::Pointer)
        .disabled(running || busy || w.queue.is_empty())
        .opacity(if running || busy || w.queue.is_empty() { 0.45 } else { 1.0 })
        .on_press(|st: &mut State| {
            if !st.demo && !must_wait(st) && !st.installed.waiting.job.is_running() {
                start_apply(st);
            }
        });
    u.dialog_shell()
        .key("installed-wait-review")
        .role("dialog")
        .label(tr("Waiting for Deadlock to close"))
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .child(text(tr("Waiting for Deadlock to close")).font(H1).color(u.ink).wrap())
        .child(
            text(tr("These run in this order when the game closes. A mod that changed since you chose it is left alone."))
                .font(BODY)
                .color(u.prose)
                .wrap()
                .max_w(432.0),
        )
        .child(list)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    u.action(tr("Cancel all"), ActionKind::Ghost)
                        .key("installed-wait-cancel-all")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(cancel_all),
                )
                .child(spacer())
                .child(try_now)
                .child(u.action(tr("Close"), ActionKind::Primary).key("installed-wait-close").cursor(rune::Cursor::Pointer).on_press(close)),
        )
}

// --- scene ---------------------------------------------------------------------------------

/// `installed-pending`: the game runs, slot 03 waits to be deleted and slot 05 to turn off, and
/// the question is open for slot 07.
pub fn scene(s: &mut State) {
    crate::app::scene_demo_installed(s);
    stage(s);
    s.installed.waiting.ask = Some(Ask {
        kind: PendingKind::Delete,
        slots: vec![7],
        after_error: false,
    });
}

/// `installed-pending-review`: the same queue in the review dialog.
pub fn scene_review(s: &mut State) {
    crate::app::scene_demo_installed(s);
    stage(s);
    s.installed.waiting.review = true;
}

fn stage(s: &mut State) {
    s.settings.game_running = true;
    let addons = PathBuf::from("/Deadlock/game/citadel/addons");
    let mut queue = PendingQueue::default();
    for (id, kind, slot) in [(1, PendingKind::Delete, 3u16), (2, PendingKind::TurnOff, 5)] {
        let name = name_of(s, slot);
        let file = deadlock_core::addons::slot_name(slot, "_dir.vpk");
        queue.changes.push(pending::PendingChange {
            id,
            kind,
            slot,
            path: addons.join(&file),
            files: vec![pending::FileStamp { name: file, size: 1, modified_ms: 0 }],
            name,
            meta: None,
            queued_at: "2026-11-22T10:00:00.000Z".to_string(),
            error: None,
        });
    }
    queue.next_id = 3;
    s.installed.waiting.queue = queue;
    s.installed.waiting.loaded_for = Some(s.game_root.clone());
}

/// German for this module's strings. Chained into `i18n`'s maps.
pub const DE: &[(&str, &str)] = &[
    ("Deletes when Deadlock closes.", "Wird gelöscht, sobald Deadlock geschlossen ist."),
    ("{} mods delete when Deadlock closes.", "{} Mods werden gelöscht, sobald Deadlock geschlossen ist."),
    ("Turns off when Deadlock closes.", "Wird ausgeschaltet, sobald Deadlock geschlossen ist."),
    ("{} mods turn off when Deadlock closes.", "{} Mods werden ausgeschaltet, sobald Deadlock geschlossen ist."),
    ("{} no longer waits for Deadlock to close.", "{} wartet nicht mehr auf das Schließen von Deadlock."),
    ("Nothing waits for Deadlock to close.", "Nichts wartet auf das Schließen von Deadlock."),
    ("Waiting changes were dropped", "Wartende Änderungen wurden verworfen"),
    ("Waiting changes could not be read", "Wartende Änderungen konnten nicht gelesen werden"),
    ("Waiting changes did not run", "Wartende Änderungen wurden nicht ausgeführt"),
    ("Deleted {}.", "{} gelöscht."),
    ("Turned off {}.", "{} ausgeschaltet."),
    ("{} still waits: {}", "{} wartet weiter: {}"),
    ("Deadlock closed: waiting changes ran", "Deadlock geschlossen: wartende Änderungen ausgeführt"),
    ("Running waiting changes", "Wartende Änderungen laufen"),
    ("1 change waiting for Deadlock to close", "1 Änderung wartet auf das Schließen von Deadlock"),
    ("{} changes waiting for Deadlock to close", "{} Änderungen warten auf das Schließen von Deadlock"),
    ("Deletes when Deadlock closes", "Wird gelöscht, sobald Deadlock geschlossen ist"),
    ("Turns off when Deadlock closes", "Wird ausgeschaltet, sobald Deadlock geschlossen ist"),
    ("Click to cancel.", "Zum Abbrechen klicken."),
    ("Delete {}?", "{} löschen?"),
    ("Delete {} mods?", "{} Mods löschen?"),
    ("Turn off {}?", "{} ausschalten?"),
    ("Turn off {} mods?", "{} Mods ausschalten?"),
    (
        "Deadlock is running and has the files open. Sigil moves them to its trash as soon as the game closes.",
        "Deadlock läuft und hat die Dateien geöffnet. Sigil verschiebt sie in seinen Papierkorb, sobald das Spiel geschlossen ist.",
    ),
    (
        "Deadlock or another program has the files open. Sigil moves them to its trash as soon as the game closes.",
        "Deadlock oder ein anderes Programm hat die Dateien geöffnet. Sigil verschiebt sie in seinen Papierkorb, sobald das Spiel geschlossen ist.",
    ),
    (
        "Deadlock is running and has the file open. Sigil turns it off as soon as the game closes.",
        "Deadlock läuft und hat die Datei geöffnet. Sigil schaltet den Mod aus, sobald das Spiel geschlossen ist.",
    ),
    (
        "Deadlock or another program has the file open. Sigil turns it off as soon as the game closes.",
        "Deadlock oder ein anderes Programm hat die Datei geöffnet. Sigil schaltet den Mod aus, sobald das Spiel geschlossen ist.",
    ),
    ("Delete when Deadlock closes", "Löschen, wenn Deadlock geschlossen ist"),
    ("Turn off when Deadlock closes", "Ausschalten, wenn Deadlock geschlossen ist"),
    ("and {} more", "und {} weitere"),
    ("Do this from now on", "Ab jetzt immer so"),
    ("Run now", "Jetzt ausführen"),
    ("Close Deadlock first.", "Schließe zuerst Deadlock."),
    ("Run the waiting changes now.", "Die wartenden Änderungen jetzt ausführen."),
    ("Waiting for Deadlock to close", "Warten auf das Schließen von Deadlock"),
    (
        "These run in this order when the game closes. A mod that changed since you chose it is left alone.",
        "Sie laufen in dieser Reihenfolge, sobald das Spiel geschlossen ist. Ein Mod, der sich seitdem geändert hat, bleibt unberührt.",
    ),
    ("Cancel all", "Alle abbrechen"),
    ("Set the game folder in Settings first.", "Lege zuerst den Spielordner in den Einstellungen fest."),
    ("Close Deadlock first", "Schließe zuerst Deadlock"),
    ("Other variants turn off when Deadlock closes", "Andere Varianten werden ausgeschaltet, sobald Deadlock geschlossen ist"),
    ("Deadlock has their files open. {} waiting.", "Deadlock hat ihre Dateien geöffnet. {} warten."),
    (HELD_ORDER, "Die Ladereihenfolge benennt Moddateien um, die Deadlock geöffnet hat. Ändere sie, nachdem das Spiel geschlossen ist."),
    (HELD_UPDATE, "Ein Update ersetzt Moddateien, die Deadlock geöffnet hat. Aktualisiere, nachdem das Spiel geschlossen ist."),
    (HELD_MERGE, "Zusammenführen und Packen schalten Mods aus, die Deadlock geöffnet hat. Mach es, nachdem das Spiel geschlossen ist."),
    (HELD_UNMERGE, "Das Auflösen verschiebt eine Moddatei, die Deadlock geöffnet hat. Mach es, nachdem das Spiel geschlossen ist."),
    (HELD_SOLO, "Ein Einzeltest schaltet Mods aus, die Deadlock geöffnet hat. Starte ihn, nachdem das Spiel geschlossen ist."),
    (HELD_MIXED, "Das schaltet Mods gleichzeitig ein und aus, und Deadlock hat ihre Dateien geöffnet. Mach es, nachdem das Spiel geschlossen ist."),
    (HELD_SKIN, "Das Ausrüsten schaltet einen anderen Skin aus, dessen Datei Deadlock geöffnet hat. Rüste aus, nachdem das Spiel geschlossen ist."),
    (HELD_UNDO, "Rückgängig ändert Moddateien, die Deadlock geöffnet hat. Mach es, nachdem das Spiel geschlossen ist."),
    (HELD_REWRITE, "Das schreibt Moddateien neu, die Deadlock geöffnet hat. Mach es, nachdem das Spiel geschlossen ist."),
];

/// One line per change that is blocked while the game holds the files.
pub const HELD_ORDER: &str = "The load order renames mod files Deadlock has open. Change it after the game closes.";
pub const HELD_UPDATE: &str = "An update replaces mod files Deadlock has open. Update after the game closes.";
pub const HELD_MERGE: &str = "Merging and packing turn off mods Deadlock has open. Do it after the game closes.";
pub const HELD_UNMERGE: &str = "Taking a merge apart moves a mod file Deadlock has open. Do it after the game closes.";
pub const HELD_SOLO: &str = "A solo test turns off mods Deadlock has open. Start it after the game closes.";
pub const HELD_MIXED: &str = "This turns mods on and off together, and Deadlock has their files open. Do it after the game closes.";
pub const HELD_SKIN: &str = "Equipping turns off another skin whose file Deadlock has open. Equip after the game closes.";
pub const HELD_UNDO: &str = "Undo changes mod files Deadlock has open. Do it after the game closes.";
pub const HELD_REWRITE: &str = "This rewrites mod files Deadlock has open. Do it after the game closes.";

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(scene_fn: fn(&mut State)) -> Vec<String> {
        let mut s = crate::data::init_state();
        scene_fn(&mut s);
        let mut rt = rune::Runtime::new(s, crate::app::view, (1280.0, 800.0), rune::Env::new(true, 1.0));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        rt.elements().into_iter().filter(|e| !e.exiting).filter_map(|e| e.key).collect()
    }

    #[test]
    fn the_question_and_the_waiting_marks_render() {
        let keys = keys(scene);
        for want in ["installed-wait-dialog", "installed-wait-confirm", "installed-wait-cancel", "installed-wait-remember", "installed-waiting"] {
            assert!(keys.iter().any(|k| k == want), "{want} missing");
        }
        assert!(keys.iter().any(|k| k == "installed-waiting-03"), "the delete mark on slot 03");
        assert!(keys.iter().any(|k| k == "installed-waiting-05"), "the switch off mark on slot 05");
        assert!(!keys.iter().any(|k| k == "installed-waiting-04"));
    }

    #[test]
    fn the_review_lists_each_change_with_a_cancel() {
        let keys = keys(scene_review);
        for want in ["installed-wait-review", "installed-wait-undo-1", "installed-wait-undo-2", "installed-wait-cancel-all", "installed-wait-run"] {
            assert!(keys.iter().any(|k| k == want), "{want} missing");
        }
    }

    #[test]
    fn a_demo_never_waits_and_never_runs_the_queue() {
        let mut s = crate::data::init_state();
        scene(&mut s);
        assert!(!must_wait(&s));
        assert!(!poll(&mut s));
        assert_eq!(s.installed.waiting.queue.len(), 2);
    }

    /// A live library (not the demo) with the game running and no game folder, so nothing
    /// can reach a disk.
    fn running() -> State {
        let mut s = crate::data::init_state();
        crate::app::scene_demo_installed(&mut s);
        s.demo = false;
        s.game_root = None;
        s.settings.game_running = true;
        s
    }

    fn told_to_close(s: &State) -> bool {
        s.notices.visible().any(|t| t.notice.title == "Close Deadlock first")
    }

    #[test]
    fn changes_that_cannot_wait_are_refused_up_front() {
        let mut s = running();
        assert!(must_wait(&s));
        super::super::ask(&mut s, super::super::Confirm::UpdateAll(vec![2]));
        assert!(s.installed.confirm.is_none(), "no confirm opens");
        assert!(told_to_close(&s));

        let mut s = running();
        super::super::apply_undo(&mut s, super::super::Undo::FontsInstalled(2));
        assert!(told_to_close(&s));

        // A mix of on and off cannot wait: refused, nothing asked.
        let mut s = running();
        let on = s.addons.addons.iter().find(|a| a.enabled).unwrap().slot;
        let off = s.addons.addons.iter().find(|a| !a.enabled).unwrap().slot;
        let wanted = [(on, false), (off, true)].into_iter().collect();
        super::super::apply_switch(&mut s, std::path::Path::new("/none"), &wanted, String::new());
        assert!(told_to_close(&s));
        assert!(s.installed.waiting.ask.is_none());

        // Once the game is gone nothing is refused.
        let mut s = running();
        s.settings.game_running = false;
        assert!(!refuse_held(&mut s, HELD_ORDER));
        assert!(!told_to_close(&s));
    }

    #[test]
    fn turning_several_off_asks_to_wait_for_each() {
        let mut s = running();
        let ons: Vec<u16> = s.addons.addons.iter().filter(|a| a.enabled).map(|a| a.slot).take(2).collect();
        let wanted = ons.iter().map(|slot| (*slot, false)).collect();
        super::super::apply_switch(&mut s, std::path::Path::new("/none"), &wanted, String::new());
        let ask = s.installed.waiting.ask.clone().expect("the question opens");
        assert_eq!(ask.kind, PendingKind::TurnOff);
        assert_eq!(ask.slots, ons);
        // Bulk turn off from the selection bar asks the same.
        let mut s = running();
        let targets = ons.iter().map(|slot| (*slot, String::new())).collect();
        assert!(super::super::switch_slots(&mut s, targets, false).is_empty());
        assert_eq!(s.installed.waiting.ask.as_ref().map(|a| a.slots.clone()), Some(ons));
    }

    #[test]
    fn german_keeps_the_holes() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in DE {
            assert!(seen.insert(*en), "duplicate {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'));
        }
    }
}
