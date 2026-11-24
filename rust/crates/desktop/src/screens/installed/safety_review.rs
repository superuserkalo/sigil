//! The mod safety review list (Grimoire #592, MIT): one place for every installed mod the scan
//! flags, as Review or Allowed.
//!
//! - Grimoire: a Mod safety page with Review and Allowed rows, "N mods need attention" over
//!   every page with Review mods and a dismiss, Check installed mods, Allow N script mods, and
//!   Disallow, which turns the mod off first and returns it to Review. Its startup check also
//!   turns off enabled mods nobody approved.
//! - Deadlock Mod Manager: no scan and no list.
//! - Sigil: the list opens from the notice on Installed, from More > Mod safety and from a
//!   card's Check what is inside. The check reads every slot file on a worker the first time
//!   Installed opens (reads only), and again on Check installed mods. Nothing is switched
//!   without a press: an enabled mod nobody allowed is listed as on with Keep it off, since
//!   writing to the addon folder is the player's action. A trust is a version (the mod and its
//!   file size), so a mod that changed asks again and says so. Disallow refuses while the game
//!   runs, turns the mod off, and only then takes the trust back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::SystemTime;

use deadlock_core::mod_safety::review::{self, Candidate, Row, Standing};
use deadlock_core::mod_safety::{self, ScanReport};
use rune::{El, Key, KeyEvent, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

/// What the list remembers between frames.
#[derive(Default)]
pub struct Review {
    pub open: bool,
    /// The row showing its files and actions.
    pub expanded: Option<u16>,
    /// Scans of every slot file: `((slot, size), report)`.
    pub job: Job<Vec<((u16, u64), ScanReport)>>,
    /// The automatic check ran this session.
    pub checked: bool,
    /// The last check could not finish.
    pub failed: Option<String>,
    /// Pending keys the player dismissed the notice for (Grimoire's dismiss).
    pub dismissed: BTreeSet<String>,
    /// The trusted keys, re-read when the file changes.
    pub trusted: BTreeSet<String>,
    trusted_stamp: Option<Option<SystemTime>>,
    /// `safety_gate::trusted_writes` when the date was last looked at, and when that was.
    trusted_writes: u64,
    trusted_looked: Option<std::time::Instant>,
}

/// How often poll looks at the trusted file's date when Sigil has not written it.
const TRUSTED_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

// --- poll ---------------------------------------------------------------------------------

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.tools.review.job.try_take() {
        match result {
            Ok(reports) => {
                for (key, report) in reports {
                    st.installed.tools.scans.insert(key, report);
                }
                st.installed.tools.review.failed = None;
            }
            Err(error) => st.installed.tools.review.failed = Some(error),
        }
        changed = true;
    }
    // The trusted file: the gate writes it too, so it is read again when it changes. Its
    // date is looked at right after Sigil writes it and otherwise every two seconds, not on
    // every poll: poll runs on every animating frame, and a file stat there costs most on
    // Windows, where antivirus filters see each one.
    let review = &st.installed.tools.review;
    let writes = super::safety_gate::trusted_writes();
    if writes != review.trusted_writes || review.trusted_looked.is_none_or(|at| at.elapsed() >= TRUSTED_EVERY) {
        st.installed.tools.review.trusted_writes = writes;
        st.installed.tools.review.trusted_looked = Some(std::time::Instant::now());
        let stamp = std::fs::metadata(super::safety_gate::trusted_path()).and_then(|m| m.modified()).ok();
        if st.installed.tools.review.trusted_stamp != Some(stamp) {
            st.installed.tools.review.trusted_stamp = Some(stamp);
            st.installed.tools.review.trusted = super::safety_gate::read_trusted();
            changed = true;
        }
    }
    if st.screen == Screen::Installed && st.addons.scanned && !st.installed.tools.review.checked && active(st) {
        st.installed.tools.review.checked = true;
        start_check(st, false);
    }
    changed
}

/// The gate and the list are on: not the demo, not a scene, and Allow script mods is off.
fn active(st: &State) -> bool {
    !st.demo && !rune::is_headless() && !crate::screens::settings::options::get().allow_scripts
}

/// Scan the slot files on a worker: the ones without a report, or all of them when `fresh`.
pub fn start_check(st: &mut State, fresh: bool) {
    if st.demo || rune::is_headless() || st.installed.tools.review.job.is_running() {
        return;
    }
    let folder = super::addons_folder(st);
    let todo: Vec<((u16, u64), PathBuf)> = st
        .addons
        .addons
        .iter()
        .filter(|addon| fresh || !st.installed.tools.scans.contains_key(&(addon.slot, addon.size_bytes)))
        .map(|addon| ((addon.slot, addon.size_bytes), addon.path(&folder)))
        .collect();
    if fresh {
        st.installed.tools.scans.clear();
    }
    if todo.is_empty() {
        return;
    }
    let wake = st.wake.clone();
    st.installed.tools.review.job.start(wake, move || {
        Ok(todo
            .into_iter()
            .map(|(key, path)| (key, mod_safety::scan_vpk(&path)))
            .collect())
    });
}

// --- rows -----------------------------------------------------------------------------------

/// Every flagged installed mod, Review first.
pub fn rows(s: &State) -> Vec<Row> {
    let candidates = s
        .addons
        .addons
        .iter()
        .filter_map(|addon| {
            let report = s.installed.tools.scans.get(&(addon.slot, addon.size_bytes))?.clone();
            Some(Candidate {
                slot: addon.slot,
                name: super::display_name(s, addon),
                enabled: addon.enabled,
                key: super::safety_gate::version_key(s, addon.slot)?,
                report,
            })
        })
        .collect();
    review::rows(candidates, &s.installed.tools.review.trusted)
}

/// A Review row whose mod was allowed at another size: it changed since the player looked.
pub fn changed_since_allowed(trusted: &BTreeSet<String>, key: &str) -> bool {
    let Some((who, _)) = key.rsplit_once(':') else { return false };
    trusted.iter().any(|other| other != key && other.rsplit_once(':').is_some_and(|(w, _)| w == who))
}

/// The one-line summary: the distinct reasons, worst first.
fn summary(row: &Row) -> String {
    row.risks().iter().map(|risk| tr(risk.label())).collect::<Vec<_>>().join(" · ")
}

// --- actions --------------------------------------------------------------------------------

pub fn open(st: &mut State) {
    st.installed.menu = None;
    st.installed.tools.scan = None;
    st.installed.tools.review.open = true;
    if active(st) && !st.installed.tools.review.job.is_running() {
        start_check(st, false);
    }
}

fn close(st: &mut State) {
    st.installed.tools.review.open = false;
    st.installed.tools.review.expanded = None;
}

/// Allow this version and turn it on, through the usual hero and requirement questions.
fn allow(st: &mut State, slot: u16) {
    super::safety_gate::trust(st, slot);
    st.installed.tools.review.trusted = super::safety_gate::read_trusted();
}

/// Keep a Review mod off: turn it off when it is on (the requirement question still asks).
fn keep_off(st: &mut State, slot: u16) {
    st.installed.tools.review.expanded = None;
    let on = st.addons.addons.iter().any(|addon| addon.slot == slot && addon.enabled);
    if on {
        super::enable::switch(st, slot, false);
    }
}

/// Grimoire's Disallow: turn the mod off, then take the trust back, so it is never left on
/// without one. Refused while the game runs, before anything changes.
pub fn disallow(st: &mut State, slot: u16) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(key) = super::safety_gate::version_key(st, slot) else { return };
    let on = st.addons.addons.iter().any(|addon| addon.slot == slot && addon.enabled);
    if on {
        super::toggle_one(st, slot, false);
        let still_on = st.addons.addons.iter().any(|addon| addon.slot == slot && addon.enabled);
        if still_on {
            // The switch failed and said why; the trust stays with the mod that is on.
            return;
        }
    }
    let revoked = review::revoke(&super::safety_gate::trusted_path(), &key);
    super::safety_gate::trusted_written();
    match revoked {
        Ok(_) => {
            st.installed.tools.review.trusted = super::safety_gate::read_trusted();
            let name = st
                .addons
                .addons
                .iter()
                .find(|addon| addon.slot == slot)
                .map(|addon| super::display_name(st, addon))
                .unwrap_or_default();
            st.status = fill(tr("{} is off and back in Review."), &[&name]);
        }
        Err(error) => st.status = error.to_string(),
    }
}

/// Allow every Review mod whose only finding is a script-mod file and turn them on in one
/// write, behind one Undo.
fn allow_scripts(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let rows = rows(st);
    let picks: Vec<(u16, String)> = review::script_only(&rows).iter().map(|row| (row.slot, row.key.clone())).collect();
    if picks.is_empty() {
        return;
    }
    let Some(game_root) = super::writable_game(st) else { return };
    let path = super::safety_gate::trusted_path();
    for (_, key) in &picks {
        let written = review::trust(&path, key);
        super::safety_gate::trusted_written();
        if let Err(error) = written {
            st.status = error.to_string();
            return;
        }
    }
    st.installed.tools.review.trusted = super::safety_gate::read_trusted();
    let wanted: BTreeMap<u16, bool> = picks.iter().map(|(slot, _)| (*slot, true)).collect();
    let line = fill(tr("Allowed and turned on {} script mods."), &[&picks.len().to_string()]);
    super::apply_switch(st, &game_root, &wanted, line);
}

// --- the notice -----------------------------------------------------------------------------

/// Mods to review as a status row chip; it opens the safety review.
pub fn chip(s: &State, u: &Ui) -> Option<El> {
    if crate::screens::settings::options::get().allow_scripts {
        return None;
    }
    let rows = rows(s);
    let pending = review::pending(&rows);
    if !review::has_new(&pending, &s.installed.tools.review.dismissed) {
        return None;
    }
    let label = fill(tr("{} to review"), &[&pending.len().to_string()]);
    Some(
        u.state_chip(&icons::SHIELD, &label, crate::ui::Tone::Warn)
            .key("installed-safety-notice")
            .help(tr("They ship programs, scripts or network calls. Look at each one before it runs."))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .on_press(open)
            .pr(2.0)
            .child(
                el().key("installed-safety-notice-dismiss")
                    .size(20.0, 20.0)
                    .center()
                    .radius(crate::ui::r(4.0))
                    .hover(u.hover)
                    .focusable(true)
                    .role("button")
                    .label(tr("Dismiss mod safety notice"))
                    .help(tr("Dismiss mod safety notice"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let pending = review::pending(&rows_of(st));
                        st.installed.tools.review.dismissed.extend(pending);
                    })
                    .child(icon(&icons::X, 12.0).color(u.muted).a11y_hidden()),
            ),
    )
}

fn rows_of(st: &State) -> Vec<Row> {
    rows(st)
}

// --- the dialog -----------------------------------------------------------------------------

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let view = &s.installed.tools.review;
    if !view.open {
        return None;
    }
    let rows = rows(s);
    let checking = view.job.is_running();
    let allow_all = crate::screens::settings::options::get().allow_scripts;
    let scripts = review::script_only(&rows).len();

    let mut list = col().w_full().gap(8.0);
    if rows.is_empty() {
        let line = if checking {
            tr("Checking installed mods.")
        } else if view.failed.is_some() {
            tr("The check could not finish. Check installed mods again before playing.")
        } else {
            tr("No mods need review.")
        };
        list = list.child(row().w_full().py(24.0).center().child(text(line).font(BODY).color(u.muted)));
    }
    for item in &rows {
        list = list.child(row_el(s, u, item, view.expanded == Some(item.slot), allow_all));
    }

    let mut top = row().w_full().gap(8.0).center_y().child(
        text(if allow_all {
            tr("Allow script mods is on in Settings, so nothing asks before it turns on. The list still shows what each mod ships.")
        } else {
            tr("Sigil asks before turning on a mod that ships a program, a script the game does not run, a script mod or a network call. Allow only mods you trust.")
        })
        .font(SMALL)
        .color(u.muted)
        .wrap()
        .grow(1.0)
        .min_w(0.0),
    );
    top = top.child(
        u.icon_action(&icons::REFRESH_CW, tr(if checking { "Checking" } else { "Check installed mods" }), ActionKind::Secondary)
            .key("safety-review-check")
            .disabled(checking)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| start_check(st, true)),
    );

    let mut shell = u
        .dialog_shell()
        .key("safety-review")
        .role("dialog")
        .label(tr("Mod safety"))
        .w(620.0)
        .padding(24.0)
        .gap(14.0)
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(text(tr("Mod safety")).font(H1).color(u.ink))
        .child(top);
    if let Some(error) = &view.failed {
        shell = shell.child(text(error.clone()).font(SMALL).color(u.amber).wrap().w_full());
    }
    if scripts > 1 && !allow_all {
        shell = shell.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(tr("Allows and turns on every mod below whose only finding is a script mod. Mods with other findings are allowed one at a time."))
                        .font(SMALL)
                        .color(u.muted)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    u.action(&fill(tr("Allow {} script mods"), &[&scripts.to_string()]), ActionKind::Primary)
                        .key("safety-review-allow-scripts")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(allow_scripts),
                ),
        );
    }
    Some(
        shell
            .child(u.scroll_area(col().w_full().max_h(420.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list)))
            .child(
                row().w_full().child(spacer()).child(
                    u.action(tr("Done"), ActionKind::Ghost)
                        .key("safety-review-done")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ),
            ),
    )
}

fn row_el(s: &State, u: &Ui, item: &Row, expanded: bool, allow_all: bool) -> El {
    let slot = item.slot;
    let allowed = item.standing == Standing::Allowed;
    let changed = !allowed && changed_since_allowed(&s.installed.tools.review.trusted, &item.key);
    let state = if allowed { tr("Allowed") } else { tr("Review") };
    let mut head_line = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(text(item.name.clone()).font(BODY_MED).color(u.ink).truncate().min_w(0.0))
        .child(text(state).font(SMALL).color(if allowed { u.teal } else { u.amber }));
    if item.enabled {
        head_line = head_line.child(text(tr("On")).font(SMALL).color(u.faint));
    }
    let head = row()
        .key(format!("safety-review-row-{slot}"))
        .w_full()
        .gap(10.0)
        .center_y()
        .focusable(true)
        .role("button")
        .label(fill(tr("View risks and actions for {}"), &[&item.name]))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let view = &mut st.installed.tools.review;
            view.expanded = if view.expanded == Some(slot) { None } else { Some(slot) };
        })
        .child(icon(&icons::SHIELD, 16.0).color(if allowed { u.teal } else { u.amber }).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(head_line)
                .child(text(summary(item)).font(SMALL).color(u.muted).truncate()),
        )
        .child(icon(if expanded { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 14.0).color(u.faint).a11y_hidden());
    let mut card = col()
        .key(format!("safety-review-card-{slot}"))
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair)
        .child(head);
    if !expanded {
        return card;
    }
    let hint = if allowed {
        tr("You allowed this version. Disallow turns it off and asks again before it turns on.")
    } else if changed {
        tr("This mod changed since you allowed it. Look at it again before allowing this version.")
    } else if item.enabled {
        tr("This mod is on, and nobody allowed this version. Keep it off, or allow it if you trust where it came from.")
    } else {
        tr("Allowing this version turns the mod on with these findings.")
    };
    card = card.child(text(hint).font(BODY).color(u.prose).wrap().w_full());
    let lines = super::safety_gate::findings(&item.report);
    let mut files = col().w_full().gap(4.0);
    for line in lines.iter().take(10) {
        files = files.child(text(line.clone()).font(MONO).color(u.muted).truncate());
    }
    if lines.len() + item.report.more > 10 {
        files = files.child(
            text(fill(tr("and {} more"), &[&(lines.len() + item.report.more - 10).to_string()]))
                .font(SMALL)
                .color(u.faint),
        );
    }
    card = card.child(files);
    let mut buttons = row().w_full().gap(8.0).center_y().child(spacer());
    if allowed {
        buttons = buttons.child(
            u.action(tr("Disallow"), ActionKind::Secondary)
                .key(format!("safety-review-disallow-{slot}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| disallow(st, slot)),
        );
    } else {
        if item.enabled {
            buttons = buttons.child(
                u.action(tr("Keep it off"), ActionKind::Secondary)
                    .key(format!("safety-review-off-{slot}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| keep_off(st, slot)),
            );
        }
        if !allow_all {
            buttons = buttons.child(
                u.action(tr("Allow this version"), ActionKind::Primary)
                    .key(format!("safety-review-allow-{slot}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| allow(st, slot)),
            );
        }
    }
    card.child(buttons)
}

/// `w4a-safety-review`: two mods in Review (one changed since it was allowed) and one Allowed,
/// expanded to its Disallow.
pub fn stage_scene(st: &mut State) {
    use deadlock_core::mod_safety::{Finding, Risk};
    let finding = |path: &str, risk, call: &str| Finding { path: path.into(), risk, call: call.into() };
    let reports = [
        (vec![finding("bin/helper.exe", Risk::Program, ""), finding("panorama/scripts/hud.vjs_c", Risk::Remote, "$.AsyncWebRequest")], 3u16),
        (vec![finding("scripts/abilities.vdata_c", Risk::ScriptMod, "")], 5),
        (vec![finding("scripts/heroes.vdata_c", Risk::ScriptMod, "")], 9),
    ];
    let mut trusted = BTreeSet::new();
    for (findings, slot) in reports {
        let Some(addon) = st.addons.addons.iter().find(|addon| addon.slot == slot).cloned() else { continue };
        st.installed.tools.scans.insert(
            (slot, addon.size_bytes),
            ScanReport { findings, entries: 12, ..ScanReport::default() },
        );
        if slot == 9
            && let Some(key) = super::safety_gate::version_key(st, slot)
        {
            trusted.insert(key);
        }
    }
    st.installed.tools.review.trusted = trusted;
    st.installed.tools.review.trusted_stamp = Some(None);
    st.installed.tools.review.checked = true;
    st.installed.tools.review.open = true;
    st.installed.tools.review.expanded = Some(9);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trust_for_another_size_means_the_mod_changed() {
        let trusted: BTreeSet<String> = ["gb7:100".to_string()].into_iter().collect();
        assert!(changed_since_allowed(&trusted, "gb7:120"));
        assert!(!changed_since_allowed(&trusted, "gb7:100"));
        assert!(!changed_since_allowed(&trusted, "gb8:120"));
    }
}
