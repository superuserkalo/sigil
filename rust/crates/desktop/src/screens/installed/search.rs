//! Solo test, the broken mod search, and the recap after a bulk action.
//!
//! **Solo test** turns every other mod off for one launch, so a player can see whether a mod
//! works on its own. Sigil remembers what was on by slot and size, shows a line above the cards
//! while the test runs, and puts everything back when Restore is pressed. One undo arm.
//!
//! **Find a broken mod** is the halving search in `deadlock_core::bisect`: the player starts the
//! game with a set of suspects on, says whether it still breaks, and the set shrinks until one
//! mod is left. Every round is a `set_many` switch, so nothing is lost, and the end offers to
//! restore everything or leave the culprit off.
//!
//! **Recap** is the short report after a bulk action: what changed, what failed and why, and
//! what was not tried.

use std::collections::{BTreeMap, BTreeSet};

use deadlock_core::bisect::{Bisect, State as SearchState, Verdict};
use rune::{El, col, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, SMALL, Ui};

use super::{meta_for, writable_game};

/// A running solo test.
pub struct Solo {
    pub name: String,
    /// What was on, as `(slot, size, uid)`; the uid finds the slot again after a reorder.
    pub others: Vec<(u16, u64, String)>,
}

/// The broken mod search's dialog state.
#[derive(Default)]
pub struct SearchFlow {
    pub open: bool,
    pub search: Option<Bisect>,
    /// The suspects picked in the setup step.
    pub picked: BTreeSet<u16>,
    /// What was on before the search started.
    pub original: Vec<u16>,
    pub error: Option<String>,
}

/// A bulk action's report, shown once.
pub struct Recap {
    pub title: String,
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub skipped: Vec<String>,
    /// Why the run stopped before the end, when it did.
    pub stopped: Option<String>,
}

impl Recap {
    pub fn new(title: impl Into<String>) -> Self {
        Recap {
            title: title.into(),
            done: Vec::new(),
            failed: Vec::new(),
            skipped: Vec::new(),
            stopped: None,
        }
    }

    pub fn any(&self) -> bool {
        !self.done.is_empty() || !self.failed.is_empty() || !self.skipped.is_empty()
    }
}

// --- solo test ---------------------------------------------------------------

/// Turn every other mod off. The card menu's "Test this mod alone".
pub fn solo_start(st: &mut State, slot: u16) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else { return };
    let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot) else {
        return;
    };
    if !addon.enabled {
        st.status = "Turn the mod on first, then test it alone.".to_string();
        return;
    }
    let others: Vec<(u16, u64, String)> = st
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled && addon.slot != slot)
        .map(|addon| {
            let uid = super::notes::extra(st, addon).uid;
            (addon.slot, addon.size_bytes, uid)
        })
        .collect();
    if others.is_empty() {
        st.status = "This is already the only mod that is on.".to_string();
        return;
    }
    let wanted: BTreeMap<u16, bool> = others.iter().map(|(slot, _, _)| (*slot, false)).collect();
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            let name = meta_for(st, addon)
                .map(|meta| meta.name.clone())
                .unwrap_or_else(|| format!("slot {}", deadlock_core::addons::slot_label(slot)));
            st.installed.solo = Some(Solo {
                name: name.clone(),
                others,
            });
            if !report.done.is_empty() {
                st.installed.last_undo = Some(super::Undo::Toggle(report.done));
                st.installed.undo_fresh = true;
            }
            st.status = format!("Testing {name} alone. Start Deadlock, then come back.");
            super::rescan(st);
        }
        Err(error) => st.status = error.to_string(),
    }
}

/// Put back what the solo test turned off.
pub fn solo_restore(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(solo) = st.installed.solo.take() else { return };
    let Some(game_root) = writable_game(st) else {
        st.installed.solo = Some(solo);
        return;
    };
    // A uid follows its slot through a reorder; the slot and size are the fallback.
    let mut wanted: BTreeMap<u16, bool> = BTreeMap::new();
    for (slot, size, uid) in &solo.others {
        let now = st
            .installed
            .notes
            .extras
            .slot_for_uid(uid)
            .filter(|slot| st.addons.addons.iter().any(|addon| addon.slot == *slot))
            .or_else(|| {
                st.addons
                    .addons
                    .iter()
                    .find(|addon| addon.slot == *slot && addon.size_bytes == *size)
                    .map(|addon| addon.slot)
            });
        if let Some(slot) = now {
            wanted.insert(slot, true);
        }
    }
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            st.status = format!(
                "Put {} mod{} back on.",
                report.done.len(),
                if report.done.len() == 1 { "" } else { "s" }
            );
        }
        Err(error) => st.status = error.to_string(),
    }
    super::rescan(st);
}

/// The solo test as a status row chip: "Testing {name} alone" with an inline Restore.
pub fn solo_chip(s: &State, u: &Ui) -> Option<El> {
    let solo = s.installed.solo.as_ref()?;
    let label = crate::screens::settings::i18n::fill(crate::screens::settings::i18n::tr("Testing {} alone"), &[&solo.name]);
    Some(
        u.state_chip(&icons::POWER, &label, crate::ui::Tone::Accent)
            .key("installed-solo")
            .max_w(320.0)
            .help(crate::screens::settings::i18n::tr("Everything else is off until you restore it."))
            .pr(2.0)
            .child(
                row()
                    .key("installed-solo-restore")
                    .h(20.0)
                    .px(6.0)
                    .center_y()
                    .radius(crate::ui::r(4.0))
                    .hover(u.hover)
                    .focusable(true)
                    .role("button")
                    .label(crate::screens::settings::i18n::tr("Restore"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| solo_restore(st))
                    .child(text(crate::screens::settings::i18n::tr("Restore")).font(crate::ui::LABEL).color(u.ink).a11y_hidden()),
            ),
    )
}

// --- broken mod search --------------------------------------------------------

/// The enabled mods a search can suspect, in load order, leaving out the ones a search cannot
/// judge (a mod with no file is not listed anyway).
pub fn default_suspects(s: &State) -> BTreeSet<u16> {
    s.addons
        .addons
        .iter()
        .filter(|addon| addon.enabled)
        .map(|addon| addon.slot)
        .collect()
}

/// Open the setup step.
pub fn search_open(st: &mut State) {
    st.installed.search.open = true;
    st.installed.search.search = None;
    st.installed.search.error = None;
    st.installed.search.original = st
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled)
        .map(|addon| addon.slot)
        .collect();
    st.installed.search.picked = default_suspects(st);
}

/// Start the search: all suspects off, then the first round.
pub fn search_start(st: &mut State) {
    let picked: Vec<u16> = st.installed.search.picked.iter().copied().collect();
    if picked.len() < 2 {
        st.installed.search.error = Some("Pick at least two mods to search.".to_string());
        return;
    }
    if writable_game(st).is_none() {
        return;
    }
    let original = st.installed.search.original.clone();
    let search = Bisect::new(&original, &picked);
    st.installed.search.search = Some(search);
    search_apply(st);
}

/// Switch the game to the round's set: the current suspects on, everything else off.
fn search_apply(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(search) = st.installed.search.search.clone() else { return };
    let Some(game_root) = writable_game(st) else { return };
    let on: BTreeSet<u16> = search.enable_now().into_iter().collect();
    let mut wanted: BTreeMap<u16, bool> = BTreeMap::new();
    for addon in &st.addons.addons {
        wanted.insert(addon.slot, on.contains(&addon.slot));
    }
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            if !report.done.is_empty() {
                st.installed.last_undo = Some(super::Undo::Toggle(report.done));
                st.installed.undo_fresh = true;
            }
            if !report.failed.is_empty() {
                st.installed.search.error =
                    Some(format!("{} mod could not be switched.", report.failed.len()));
            }
            super::rescan(st);
        }
        Err(error) => st.installed.search.error = Some(error.to_string()),
    }
}

/// Record the player's answer and set up the next round.
pub fn search_judge(st: &mut State, verdict: Verdict) {
    let Some(mut search) = st.installed.search.search.take() else { return };
    match search.judge(verdict) {
        SearchState::Testing => {
            st.installed.search.search = Some(search);
            search_apply(st);
        }
        SearchState::Found | SearchState::Done => {
            st.installed.search.search = Some(search);
        }
    }
}

/// End the search, putting the original set back, or that set minus the culprits.
pub fn search_finish(st: &mut State, leave_culprits_off: bool) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = writable_game(st) else { return };
    let culprits: BTreeSet<u16> = st
        .installed
        .search
        .search
        .as_ref()
        .map(|search| search.culprits.iter().copied().collect())
        .unwrap_or_default();
    let original: Vec<u16> = st.installed.search.original.clone();
    let mut wanted: BTreeMap<u16, bool> = BTreeMap::new();
    for addon in &st.addons.addons {
        let was_on = original.contains(&addon.slot);
        let on = was_on && !(leave_culprits_off && culprits.contains(&addon.slot));
        if addon.enabled != on {
            wanted.insert(addon.slot, on);
        }
    }
    match deadlock_core::addons::set_many(&game_root, &wanted, true) {
        Ok(report) => {
            if !report.done.is_empty() {
                st.installed.last_undo = Some(super::Undo::Toggle(report.done));
                st.installed.undo_fresh = true;
            }
            st.status = if leave_culprits_off && !culprits.is_empty() {
                format!(
                    "Put your mods back on, {} left off.",
                    culprits.len()
                )
            } else {
                "Put your mods back on.".to_string()
            };
        }
        Err(error) => st.status = error.to_string(),
    }
    st.installed.search.open = false;
    st.installed.search.search = None;
    super::rescan(st);
}

fn name_of(s: &State, slot: u16) -> String {
    s.addons
        .addons
        .iter()
        .find(|addon| addon.slot == slot)
        .map(|addon| super::display_name(s, addon))
        .unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(slot)))
}

fn verdict_row(
    u: &Ui,
    key: &str,
    glyph: &'static rune::Icon,
    label: &str,
    verdict: Verdict,
) -> El {
    u.icon_action(glyph, label, ActionKind::Secondary)
        .key(key)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| search_judge(st, verdict))
}

/// The setup, testing and result steps.
pub fn search_dialog(s: &State, u: &Ui) -> Option<El> {
    if !s.installed.search.open {
        return None;
    }
    let flow = &s.installed.search;
    let close = |st: &mut State| {
        st.installed.search.open = false;
        st.installed.search.search = None;
    };
    let shell = |title: &str, body: &str, label: &str| {
        u.dialog_shell()
            .key("installed-search")
            .role("dialog")
            .label(label.to_string())
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(title.to_string()).font(H1).color(u.ink).wrap())
            .child(text(body.to_string()).font(BODY).color(u.muted).wrap())
    };
    let Some(search) = &flow.search else {
        // Setup: pick the suspects.
        let mut list = col().w_full().gap(2.0);
        let enabled: Vec<&crate::model::AddonRow> = s
            .addons
            .addons
            .iter()
            .filter(|addon| addon.enabled)
            .collect();
        for addon in enabled.iter().take(12) {
            let slot = addon.slot;
            let on = flow.picked.contains(&slot);
            list = list.child(
                row()
                    .key(format!("search-pick-{slot:02}"))
                    .w_full()
                    .h(32.0)
                    .gap(10.0)
                    .center_y()
                    .focusable(true)
                    .role("checkbox")
                    .checked(on)
                    .label(format!("Search {}", super::display_name(s, addon)))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if !st.installed.search.picked.remove(&slot) {
                            st.installed.search.picked.insert(slot);
                        }
                    })
                    .child(super::check_box(u, on, false))
                    .child(
                        text(super::display_name(s, addon))
                            .font(BODY)
                            .color(if on { u.ink } else { u.prose })
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(text(deadlock_core::addons::slot_label(slot)).font(MICRO).color(u.faint)),
            );
        }
        if enabled.len() > 12 {
            list = list.child(
                text(format!("and {} more", enabled.len() - 12))
                    .font(SMALL)
                    .color(u.faint)
                    .pt(6.0),
            );
        }
        let picked = flow.picked.len();
        let mut actions = row().w_full().gap(8.0).center_y();
        if let Some(error) = &flow.error {
            actions = actions.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
        }
        return Some(
            shell(
                "Find a broken mod",
                "Start Deadlock with half of these mods on. Sigil narrows the set each launch until one is left.",
                "Find a broken mod",
            )
            .child(list)
            .child(
                actions
                    .child(spacer())
                    .child(
                        u.action("Cancel", ActionKind::Ghost)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(
                        u.action(&format!("Start with {picked}"), ActionKind::Primary)
                            .key("search-start")
                            .disabled(picked < 2)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| search_start(st)),
                    ),
            ),
        );
    };

    let state = search.state();
    if state == SearchState::Testing {
        let suspects = search.suspects();
        let others = st_off_count(s, &suspects);
        let mut list = col().w_full().gap(4.0);
        for slot in suspects.iter().take(8) {
            list = list.child(
                row()
                    .w_full()
                    .h(28.0)
                    .gap(8.0)
                    .center_y()
                    .child(text(deadlock_core::addons::slot_label(*slot)).font(MICRO).color(u.faint).w(28.0))
                    .child(text(name_of(s, *slot)).font(BODY).color(u.prose).truncate()),
            );
        }
        if suspects.len() > 8 {
            list = list.child(
                text(format!("and {} more", suspects.len() - 8))
                    .font(SMALL)
                    .color(u.faint),
            );
        }
        let (title, body) = if suspects.is_empty() {
            (
                "First: every suspect off".to_string(),
                format!(
                    "Start Deadlock with all {} suspects off. If it still breaks, the cause is not one of them.",
                    search.pool_slots().len()
                ),
            )
        } else {
            (
                format!(
                    "Round {}: {} mod{} on",
                    search.rounds.len() + 1,
                    suspects.len(),
                    if suspects.len() == 1 { "" } else { "s" }
                ),
                format!(
                    "Start Deadlock now. {others} other mods are off. When the game has loaded, say what happened."
                ),
            )
        };
        return Some(
            shell(&title, &body, "Find a broken mod")
            .child(list)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        u.action("Stop and put back", ActionKind::Ghost)
                            .key("search-stop")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| search_finish(st, false)),
                    )
                    .child(spacer())
                    .child(verdict_row(u, "search-broken", &icons::TRIANGLE_ALERT, "Still broken", Verdict::Broken))
                    .child(verdict_row(u, "search-works", &icons::CHECK, "Works now", Verdict::Works))
                    .child(verdict_row(u, "search-unsure", &icons::INFO, "Not sure", Verdict::Unsure)),
            ),
        );
    }

    // Result: a culprit, or none of them.
    let culprits = search.culprits.clone();
    let mut list = col().w_full().gap(4.0);
    for slot in &culprits {
        list = list.child(
            row()
                .w_full()
                .h(36.0)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                .child(text(deadlock_core::addons::slot_label(*slot)).font(MICRO).color(u.faint).w(28.0))
                .child(text(name_of(s, *slot)).font(BODY_MED).color(u.ink).truncate()),
        );
    }
    let title = if culprits.is_empty() {
        "None of the mods on the list".to_string()
    } else {
        format!(
            "{} may be the broken one",
            culprits
                .iter()
                .map(|slot| name_of(s, *slot))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let body = if culprits.is_empty() {
        "The game ran fine with them off and on. The crash may come from something else."
    } else {
        "Leave it off next time you play, or restore everything and try again."
    };
    Some(
        shell(&title, body, "Find a broken mod")
            .child(list)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        u.action("Put everything back", ActionKind::Secondary)
                            .key("search-restore-all")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| search_finish(st, false)),
                    )
                    .child(spacer())
                    .child(
                        u.action(
                            if culprits.is_empty() { "Done" } else { "Leave them off" },
                            ActionKind::Primary,
                        )
                        .key("search-keep-off")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| search_finish(st, !culprits.is_empty())),
                    ),
            ),
    )
}

fn st_off_count(s: &State, suspects: &[u16]) -> usize {
    s.addons
        .addons
        .iter()
        .filter(|addon| !suspects.contains(&addon.slot))
        .count()
}

// --- recap -------------------------------------------------------------------

/// The recap dialog after a bulk action.
pub fn recap_dialog(s: &State, u: &Ui) -> Option<El> {
    let recap = s.installed.recap.as_ref()?;
    let mut done = col().w_full().gap(2.0);
    for name in recap.done.iter().take(8) {
        done = done.child(
            row()
                .w_full()
                .h(24.0)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::CHECK, 12.0).color(u.muted).a11y_hidden())
                .child(text(name.clone()).font(BODY).color(u.prose).truncate()),
        );
    }
    if recap.done.len() > 8 {
        done = done.child(
            text(format!("and {} more", recap.done.len() - 8))
                .font(SMALL)
                .color(u.faint),
        );
    }
    let mut failed = col().w_full().gap(2.0);
    for (name, why) in recap.failed.iter().take(6) {
        failed = failed.child(
            row()
                .w_full()
                .h(24.0)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::TRIANGLE_ALERT, 12.0).color(u.amber).a11y_hidden())
                .child(
                    text(format!("{name}: {why}"))
                        .font(SMALL)
                        .color(u.amber)
                        .truncate(),
                ),
        );
    }
    let mut body = col().w_full().gap(12.0);
    if !recap.done.is_empty() {
        body = body.child(u.field_label(&format!("Changed ({})", recap.done.len()))).child(done);
    }
    if !recap.failed.is_empty() {
        body = body
            .child(u.field_label(&format!("Failed ({})", recap.failed.len())))
            .child(failed);
    }
    if !recap.skipped.is_empty() {
        body = body.child(
            text(format!("Not tried ({}): {}", recap.skipped.len(), recap.skipped.join(", ")))
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    }
    if let Some(stopped) = &recap.stopped {
        body = body.child(text(stopped.clone()).font(SMALL).color(u.muted).wrap());
    }
    let close = |st: &mut State| st.installed.recap = None;
    Some(
        u.dialog_shell()
            .key("installed-recap")
            .role("dialog")
            .label(recap.title.clone())
            .w(480.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(recap.title.clone()).font(H1).color(u.ink).wrap())
            .child(body)
            .child(
                row()
                    .w_full()
                    .child(spacer())
                    .child(
                        u.action("Done", ActionKind::Primary)
                            .key("recap-done")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            ),
    )
}
