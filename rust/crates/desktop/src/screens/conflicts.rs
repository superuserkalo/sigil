//! Overlap: game paths that more than one enabled addon replaces.
//!
//! `read_conflict_report` does the comparison. Lower slot wins, so the winner column names the
//! slot the game actually loads for that path. This screen is where an overlap is settled: make
//! a losing mod win by swapping its slot with the winner's, turn one off, or tell Sigil to stop
//! mentioning the pair, the file or the mod. An ignore is one line in
//! `overlap-ignores.json` under the app data directory, and the restore list at the bottom of
//! the page takes any of them back.

use std::path::Path;

use deadlock_core::impact::{self, Ignores};
use rune::{El, ScrollSpec, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::{ConflictRow, Screen};
use crate::ui::{ActionKind, BODY, MONO, PAGE_TOP, PAGE_X, SMALL, SMALL_NUM, Ui};
use crate::screens::settings::i18n::{fill, tr};

const SHOWN_LIMIT: usize = 80;
/// Mono 12 runs about 7.2 points a character; this keeps a path inside the column at the
/// default window width, and the element still truncates at the end if the window is narrower.
const WINS_W: f32 = 180.0;
const LOSES_W: f32 = 120.0;
const IGNORE_W: f32 = 72.0;

/// What the page has open. Kept in `launch::View` with the rest of the area's state.
#[derive(Default)]
pub struct View {
    pub dialog: Dialog,
}

#[derive(Clone, PartialEq, Eq, Default)]
pub enum Dialog {
    #[default]
    None,
    /// One losing slot picked from a row.
    Resolve {
        path: String,
        slots: Vec<u16>,
        winner: u16,
        loser: u16,
    },
}

/// Bumped whenever the cached ignore list changes, so counts built from it know to redo.
static IGNORES_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Changes whenever the ignore list does.
pub fn ignores_generation() -> u64 {
    IGNORES_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// What an overlap check reads of the rows (paths, slots, winners), folded without
/// allocating: a rescan or a load order change that moves a winner changes it.
pub fn rows_key(conflicts: &[crate::model::ConflictRow]) -> u64 {
    conflicts.iter().fold(conflicts.len() as u64, |h, c| {
        let path = c.path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |f, b| (f ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
        let row = c.slots.iter().fold(path ^ u64::from(c.winner), |r, &slot| r.rotate_left(7) ^ u64::from(slot));
        h.rotate_left(5) ^ row.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    })
}

/// How many overlaps a player still has to look at. Drives the sidebar badge, so it runs on
/// every view of every page: the count is kept until the conflicts or the ignore list change
/// (it cloned the whole ignore list and checked every per-file row each time).
pub fn badge_count(s: &State) -> usize {
    use std::cell::Cell;
    thread_local! {
        static LAST: Cell<Option<((u64, usize, u64), usize)>> = const { Cell::new(None) };
    }
    let conflicts = &s.addons.conflicts;
    let key = (rows_key(conflicts), conflicts.len(), ignores_generation());
    if let Some((seen, count)) = LAST.with(Cell::get)
        && seen == key
    {
        return count;
    }
    let count = cache()
        .lock()
        .map(|ignores| {
            conflicts
                .iter()
                .filter(|conflict| !ignores.hides(&conflict.path, &conflict.slots, conflict.winner))
                .count()
        })
        .unwrap_or(conflicts.len());
    LAST.with(|last| last.set(Some((key, count))));
    count
}

/// The ignore list, read once per process. It lives under the app data directory and is edited
/// from this page; the cache keeps the first frame from touching the disk more than once.
///
/// This is one process-wide slot because the page has no `View` of its own in `State`; the
/// scenes replace it without writing a file (`stage_ignores`).
fn cache() -> &'static std::sync::Mutex<Ignores> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Ignores>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(impact::load_ignores(&scope_dir(scope().as_deref()))))
}

// --- ignores per profile (DMM) -------------------------------------------------------------
//
// Deadlock Mod Manager keeps the overlaps you accept per profile. With a profile active, Sigil
// keeps that profile's list in `<data>/overlap-profiles/<slug>/overlap-ignores.json`, started as
// a copy of the shared list so nothing seems to come back; with none, the shared list.

fn scope_cell() -> &'static std::sync::Mutex<Option<String>> {
    use std::sync::{Mutex, OnceLock};
    static SCOPE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SCOPE.get_or_init(|| Mutex::new(None))
}

fn scope() -> Option<String> {
    scope_cell().lock().ok().and_then(|slug| slug.clone())
}

/// The folder whose `overlap-ignores.json` the list lives in.
pub fn scope_dir(slug: Option<&str>) -> std::path::PathBuf {
    let data = crate::data::data_dir();
    match slug {
        Some(slug) if !slug.trim().is_empty() => data.join("overlap-profiles").join(deadlock_core::profiles::slug_for(slug)),
        _ => data,
    }
}

/// Follow the active profile. Called from Installed's poll; a change reads that profile's list.
pub fn sync_scope(st: &State) {
    if st.demo || rune::is_headless() {
        return;
    }
    let active = st.profiles.active.clone();
    if scope() == active {
        return;
    }
    if let Ok(mut slot) = scope_cell().lock() {
        slot.clone_from(&active);
    }
    let dir = scope_dir(active.as_deref());
    let path = impact::ignores_path(&dir);
    if active.is_some() && !path.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(impact::ignores_path(&crate::data::data_dir()), &path);
    }
    if let Ok(mut cached) = cache().lock() {
        *cached = impact::load_ignores(&dir);
        IGNORES_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Grimoire's Ignore all: every overlap shown goes to Ignored after the inline confirm, with
/// one Undo that puts the list back as it was.
static ASK_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ignore_all(st: &mut State) {
    ASK_ALL.store(false, std::sync::atomic::Ordering::Relaxed);
    let before = ignores();
    let rows: Vec<(Vec<u16>, u16)> = st
        .addons
        .conflicts
        .iter()
        .filter(|row| !before.hides(&row.path, &row.slots, row.winner))
        .map(|row| (row.slots.clone(), row.winner))
        .collect();
    edit(|list| {
        let mut changed = false;
        for (slots, winner) in &rows {
            for slot in slots.iter().filter(|slot| *slot != winner) {
                changed |= list.add_pair(*winner, *slot);
            }
        }
        changed
    });
    crate::notify::push(
        st,
        crate::notify::Notice::success(fill(tr("Ignored {} overlaps"), &[&rows.len().to_string()]))
            .detail(tr("Restore any of them at the bottom of Overlap."))
            .undo(move |_st: &mut State| {
                let before = before.clone();
                edit(move |list| {
                    *list = before;
                    true
                });
            }),
    );
}

/// The ignore list for other pages: Installed's overlap badge leaves out what it hides.
pub fn ignore_list() -> Ignores {
    ignores()
}

fn ignores() -> Ignores {
    cache().lock().map(|list| list.clone()).unwrap_or_default()
}

/// Read, edit and store the list, and keep the cache in step.
fn edit(change: impl FnOnce(&mut Ignores) -> bool) -> Ignores {
    let dir = scope_dir(scope().as_deref());
    let _ = std::fs::create_dir_all(&dir);
    let fresh = impact::edit_ignores(&dir, change).unwrap_or_else(|_| impact::load_ignores(&dir));
    if let Ok(mut cached) = cache().lock() {
        *cached = fresh.clone();
        IGNORES_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fresh
}

pub fn view(s: &State, u: &Ui) -> El {
    let query = s.conflicts_query.text.trim().to_lowercase();
    let hidden = ignores();
    let visible: Vec<&ConflictRow> = s
        .addons
        .conflicts
        .iter()
        .filter(|row| !hidden.hides(&row.path, &row.slots, row.winner))
        .filter(|row| matches_query(row, &query))
        .collect();
    let visible_count = badge_count(s);

    // "Ignore all" stays in place, disabled with one or none; its confirm is a popover under
    // the button (`dialog`), so the toolbar never swaps.
    let few = visible_count <= 1;
    let actions = vec![
        u.action(tr("Ignore all"), ActionKind::Ghost)
            .key("conflicts-ignore-all")
            .disabled(few)
            .opacity(if few { 0.45 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .on_press(|_st: &mut State| ASK_ALL.store(true, std::sync::atomic::Ordering::Relaxed)),
        u.icon_button(&icons::REFRESH_CW, tr("Rescan"))
            .key("conflicts-rescan")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                if !st.demo {
                    crate::data::start_addon_scan(st);
                }
            }),
    ];

    let count = s.addons.conflicts.len();
    let _detail = if !s.game_detected || !s.addons.scanned || visible_count == 0 {
        String::new()
    } else if !query.is_empty() {
        fill(
            tr("{} of {} shown"),
            &[&visible.len().to_string(), &visible_count.to_string()],
        )
    } else if visible_count == 1 {
        fill(
            tr("{} file shipped by more than one mod. The lower slot wins."),
            &[&visible_count.to_string()],
        )
    } else {
        fill(
            tr("{} files shipped by more than one mod. The lower slot wins."),
            &[&visible_count.to_string()],
        )
    };

    // Title with the count after it, then a fixed status row: scan warnings left, the one
    // rule right. Both stay out of the scroll.
    let lead = row()
        .gap(8.0)
        .center_y()
        .child(u.page_title(tr(Screen::Conflicts.title())))
        .child(
            text(if visible_count > 0 { visible_count.to_string() } else { String::new() })
                .font(SMALL_NUM)
                .color(u.muted),
        );
    let rule = (s.addons.scanned && count > 0)
        .then(|| text(tr("The lower slot wins")).font(SMALL).color(u.muted));
    let top = col()
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(16.0)
        .gap(8.0)
        .child(u.page_toolbar(lead, Vec::new(), Some(crate::app::page_search(s, u)), actions))
        .child(u.status_row("conflicts-status", warnings(s, u), rule));
    let page = col()
        .key("conflicts-page")
        .w_full()
        .px(PAGE_X)
        .pb(32.0)
        .gap(24.0);
    let names = names(s);

    let ignored = crate::screens::settings::options::get().ignore_overlap_default;
    let body = if ignored {
        // Settings > Mods > Ignore overlap by default.
        u.empty_state(
            "",
            tr("Overlap is ignored"),
            tr("Turn off Ignore overlap by default in Settings to compare your mods."),
            Some(
                u.action(tr("Show overlap"), ActionKind::Secondary)
                    .key("conflicts-unignore")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        crate::screens::settings::options::edit(st, |o| o.ignore_overlap_default = false)
                    }),
            ),
        )
    } else if !s.game_detected {
        u.empty_state(
            "",
            tr("Deadlock was not found"),
            tr("Set the game folder in Settings to compare your mods."),
            Some(
                u.action(tr("Open Settings"), ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else if !s.addons.scanned {
        u.empty_state(
            "",
            if s.addon_job.is_running() {
                tr("Comparing mods")
            } else {
                tr("Not compared yet")
            },
            tr("Overlap lists files that two enabled mods both replace."),
            if s.addon_job.is_running() {
                None
            } else {
                Some(
                    u.action(tr("Compare now"), ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::start_addon_scan(st)),
                )
            },
        )
    } else if count == 0 {
        u.empty_state(
            "",
            tr("No overlap"),
            if s.addons.addons.is_empty() {
                tr("No mods are installed.")
            } else {
                tr("Every enabled mod replaces different files.")
            },
            None,
        )
    } else if visible_count == 0 {
        u.empty_state(
            "",
            tr("Nothing left to settle"),
            tr("Every overlap here is ignored. Restore one below to see it again."),
            None,
        )
    } else if visible.is_empty() {
        u.empty_state(
            "",
            tr("Nothing matches"),
            tr("Try part of a path or a slot number."),
            Some(
                u.action(tr("Clear search"), ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.conflicts_query = rune::TextEdit::new("")),
            ),
        )
    } else {
        let mut list = col().w_full().child(head(u)).child(
            col()
                .w_full()
                .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
                .children(
                    visible
                        .iter()
                        .take(SHOWN_LIMIT)
                        .enumerate()
                        .map(|(index, c)| conflict_row(u, index, c, &names)),
                ),
        );
        if visible.len() > SHOWN_LIMIT {
            list = list.child(
                text(fill(
                    tr("First {} of {} shown"),
                    &[&SHOWN_LIMIT.to_string(), &visible.len().to_string()],
                ))
                .font(SMALL)
                .color(u.faint)
                .pt(12.0),
            );
        }
        list
    };

    let mut page = page.child(body);
    let ignored_list = ignores();
    if !ignored_list.is_empty() {
        page = page.child(restore_list(u, &ignored_list, names));
    }

    col()
        .key("conflicts-frame")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(top)
        .child(
            col()
                .key("conflicts-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(page),
        )
}

fn matches_query(row: &ConflictRow, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    row.path.to_lowercase().contains(query)
        || row
            .slots
            .iter()
            .any(|slot| format!("{slot:02}").contains(query) || deadlock_core::addons::slot_label(*slot).contains(query))
}

fn head(u: &Ui) -> El {
    row()
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(tr("File")).font(SMALL).color(u.muted).grow(1.0).min_w(0.0))
        .child(text(tr("Wins")).font(SMALL).color(u.muted).w(WINS_W))
        .child(text(tr("Loses")).font(SMALL).color(u.muted).w(LOSES_W))
        .child(el().w(IGNORE_W))
}

/// One overlapping path: the file, the slots that ship it, the slot that loads, and the two
/// ways to settle it.
fn conflict_row(
    u: &Ui,
    index: usize,
    conflict: &ConflictRow,
    names: &std::collections::BTreeMap<u16, String>,
) -> El {
    let path = conflict.path.clone();
    let slots = conflict.slots.clone();
    let winner = conflict.winner;
    let name_of = |slot: u16| names.get(&slot).cloned().unwrap_or_default();
    // Wins: the slot and the mod's name, so "05" means something.
    let wins = row()
        .w(WINS_W)
        .gap(8.0)
        .center_y()
        .child(text(deadlock_core::addons::slot_label(winner)).font(MONO).color(u.muted))
        .child(
            text(name_of(winner))
                .font(BODY)
                .color(u.prose)
                .truncate()
                .help_when_truncated(name_of(winner))
                .grow(1.0)
                .min_w(0.0),
        );
    // Loses: one pressable chip per losing slot, its mod name as the tooltip.
    let mut loses = row().w(LOSES_W).gap(4.0).center_y();
    for slot in conflict.slots.iter().filter(|slot| **slot != winner) {
        let loser = *slot;
        let target = path.clone();
        let all = slots.clone();
        let label = deadlock_core::addons::slot_label(loser);
        let name = name_of(loser);
        loses = loses.child(
            row()
                .key(format!("conflict-{index}-slot-{loser}"))
                .h(24.0)
                .px(6.0)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(u.over.alpha(0.06))
                .hover(u.over.alpha(0.10))
                .focusable(true)
                .role("button")
                .label(fill(tr("Settle slot {}"), &[&label]))
                .help(if name.is_empty() { fill(tr("Settle slot {}"), &[&label]) } else { name })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.launch.overlap.dialog = Dialog::Resolve {
                        path: target.clone(),
                        slots: all.clone(),
                        winner,
                        loser,
                    };
                })
                .child(text(label).font(MONO).color(u.prose).a11y_hidden()),
        );
    }
    let ignore_path = conflict.path.clone();
    let ignore_slots = conflict.slots.clone();
    row()
        .key(format!("conflict-{index}"))
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .child(
            text(conflict.path.clone())
                .font(MONO)
                .color(u.prose)
                .grow(1.0)
                .min_w(0.0)
                .truncate_middle()
                .help(conflict.path.clone())
                .selectable(true)
                .label(conflict.path.clone()),
        )
        .child(wins)
        .child(loses)
        .child(
            u.action(tr("Ignore"), ActionKind::Ghost)
                .key(format!("conflict-ignore-{index}"))
                .h(24.0)
                .px(8.0)
                .w(IGNORE_W)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    // Every losing pair, so the whole row goes rather than half of it.
                    let winner = ignore_slots.iter().copied().min().unwrap_or(0);
                    let path = ignore_path.clone();
                    let losers = ignore_slots.clone();
                    edit(|list| {
                        let mut changed = false;
                        for slot in losers.iter().filter(|slot| **slot != winner) {
                            changed |= list.add_pair(winner, *slot);
                        }
                        changed
                    });
                    crate::notify::push(
                        st,
                        crate::notify::Notice::success(tr("Ignored this file"))
                            .detail(tr("Restore it at the bottom of Overlap."))
                            .undo(move |st: &mut State| {
                                let winner = *losers.iter().min().unwrap_or(&0);
                                let losers = losers.clone();
                                edit(|list| {
                                    let mut changed = false;
                                    for slot in losers.iter().filter(|slot| **slot != winner) {
                                        changed |= list.remove_pair(winner, *slot);
                                    }
                                    changed
                                });
                                let _ = &path;
                                st.addons.scanned = false;
                            }),
                    );
                }),
        )
}

/// Scan warnings for the status row: the first one as a line, "+n" for the rest with every
/// warning in its tooltip.
fn warnings(s: &State, u: &Ui) -> Vec<El> {
    if !s.addons.scanned || s.addons.warnings.is_empty() {
        return Vec::new();
    }
    let mut out = vec![u.status_line(&icons::TRIANGLE_ALERT, &s.addons.warnings[0], crate::ui::Tone::Warn)];
    let more = s.addons.warnings.len() - 1;
    if more > 0 {
        out.push(
            u.state_chip(&icons::TRIANGLE_ALERT, &format!("+{more}"), crate::ui::Tone::Warn)
                .key("conflicts-warnings-more")
                .help(s.addons.warnings[1..].join("\n")),
        );
    }
    out
}

/// Slot names for the restore list, from the library and the addon rows.
fn names(s: &State) -> std::collections::BTreeMap<u16, String> {
    let mut names = std::collections::BTreeMap::new();
    for addon in &s.addons.addons {
        if let Some(meta) = s
            .installed
            .library
            .mods
            .iter()
            .find(|meta| meta.slot == addon.slot && !meta.name.trim().is_empty())
        {
            names.insert(addon.slot, meta.name.clone());
        }
    }
    names
}

/// The restore list: every ignore, one row each, with the action that takes it back.
fn restore_list(u: &Ui, list: &Ignores, names: std::collections::BTreeMap<u16, String>) -> El {
    let entries = impact::restore_entries(list, &names);
    let head = u.section_head(
        tr("Ignored"),
        Some(entries.len()),
        Some(
        u.action(tr("Restore all"), ActionKind::Ghost)
            .h(24.0)
            .px(8.0)
            .key("conflicts-restore-all")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                let _ = edit(|list| {
                    let changed = !list.is_empty();
                    *list = Ignores::default();
                    changed
                });
                crate::notify::push(
                    st,
                    crate::notify::Notice::success(tr("Every overlap is back")),
                );
            }),
        ),
    );
    let mut body = col().w_full().gap(0.0).child(el().pb(8.0).w_full().child(head));
    for entry in entries {
        let kind = entry.kind.clone();
        body = body.child(
            row()
                .key(format!("conflict-restore-{kind:?}"))
                .w_full()
                .h(32.0)
                .px(8.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    icon(&icons::EYE, 14.0)
                        .color(u.faint)
                        .a11y_hidden(),
                )
                .child(
                    text(entry.label.clone())
                        .font(BODY)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate()
                        .label(entry.label.clone()),
                )
                .child(
                    u.action(tr("Restore"), ActionKind::Ghost)
                        .h(24.0)
                        .px(8.0)
                        .key(format!("conflict-restore-run-{kind:?}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            let data = crate::data::data_dir();
                            let fresh = impact::restore(&data, &kind)
                                .unwrap_or_else(|_| impact::load_ignores(&data));
                            if let Ok(mut cached) = cache().lock() {
                                *cached = fresh;
                                IGNORES_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            st.addons.scanned = false;
                            crate::notify::push(
                                st,
                                crate::notify::Notice::success(tr("Overlap is back")),
                            );
                        }),
                ),
        );
    }
    body
}

/// The Ignore all popover, mounted at the root without a dimming layer.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    (s.screen == Screen::Conflicts
        && matches!(s.launch.overlap.dialog, Dialog::None)
        && ASK_ALL.load(std::sync::atomic::Ordering::Relaxed))
    .then(|| ignore_all_popover(s, u))
}

/// "Ignore all" asks under its button: one sentence, Cancel and the confirm.
fn ignore_all_popover(s: &State, u: &Ui) -> El {
    let cancel = |_st: &mut State| ASK_ALL.store(false, std::sync::atomic::Ordering::Relaxed);
    el().key("conflicts-ignore-all-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(el().absolute(0.0, 0.0).w_full().h_full().on_press(cancel))
        .child(
            col()
                .key("conflicts-ignore-all-ask")
                .role("dialog")
                .w(280.0)
                .padding(12.0)
                .gap(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.raised.mix(u.over, 0.04).alpha(1.0))
                .border(1.0, u.hair_strong)
                .shadow(4.0, 12.0, rune::Color::BLACK.alpha(if u.light { 0.12 } else { 0.35 }))
                .anchored("conflicts-ignore-all", rune::Side::Bottom, rune::Align::End, 4.0)
                .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
                    if ev.key == rune::Key::Escape {
                        cancel(st);
                        true
                    } else {
                        false
                    }
                })
                .child(
                    text(fill(tr("Ignore all {} overlaps?"), &[&badge_count(s).to_string()]))
                        .font(BODY)
                        .color(u.ink)
                        .wrap(),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(spacer())
                        .child(
                            u.action_m(tr("Cancel"), ActionKind::Ghost)
                                .key("conflicts-ignore-all-no")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(cancel),
                        )
                        .child(
                            u.action_m(tr("Ignore all"), ActionKind::Primary)
                                .key("conflicts-ignore-all-yes")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(ignore_all),
                        ),
                ),
        )
}

/// The dialog the page opens: one losing slot, two ways to settle it, one way to ignore it.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match &s.launch.overlap.dialog {
        Dialog::None => None,
        Dialog::Resolve {
            path,
            slots,
            winner,
            loser,
        } => Some(resolve(s, u, path, slots, *winner, *loser)),
    }
}

fn resolve(s: &State, u: &Ui, path: &str, slots: &[u16], winner: u16, loser: u16) -> El {
    let close = |st: &mut State| st.launch.overlap.dialog = Dialog::None;
    let names = names(s);
    let name_for = |slot: u16| {
        names
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| format!("Slot {}", deadlock_core::addons::slot_label(slot)))
    };
    let loser_name = name_for(loser);
    let winner_name = name_for(winner);
    let blocked = s.settings.game_running;
    let make = loser;
    let swap_winner = winner;
    let off = loser;
    let ignore_path = path.to_string();
    let reason = tr("Close Deadlock first: slot numbers cannot change while it runs.");
    let dim = |e: El| if blocked { e.disabled(true).opacity(0.45) } else { e };
    let options = u
        .panel()
        .w_full()
        .clip_rounded(crate::ui::r(8.0))
        .child(dim(
            u.option_row(
                "conflicts-resolve-win",
                &fill(tr("Make {} win"), &[&loser_name]),
                &if blocked {
                    reason.to_string()
                } else {
                    fill(tr("Swaps slots {} and {}"), &[
                        &deadlock_core::addons::slot_label(loser),
                        &deadlock_core::addons::slot_label(winner),
                    ])
                },
            )
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .on_press(move |st: &mut State| {
                st.launch.overlap.dialog = Dialog::None;
                make_win(st, swap_winner, make);
            }),
        ))
        .child(dim(
            u.option_row(
                "conflicts-resolve-off",
                &fill(tr("Turn {} off"), &[&loser_name]),
                if blocked { reason } else { tr("It stays installed") },
            )
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .on_press(move |st: &mut State| {
                st.launch.overlap.dialog = Dialog::None;
                crate::data::set_addon_enabled(st, off, false);
            }),
        ))
        .child(
            u.option_row(
                "conflicts-resolve-ignore",
                tr("Ignore this file"),
                tr("Restore it at the bottom of Overlap."),
            )
            .on_press(move |st: &mut State| {
                let path = ignore_path.clone();
                let slot = make;
                edit(|list| list.add_file(slot, &path));
                st.launch.overlap.dialog = Dialog::None;
                crate::notify::push(
                    st,
                    crate::notify::Notice::success(tr("Ignored this file"))
                        .detail(tr("Restore it at the bottom of Overlap.")),
                );
            }),
        );
    let _ = slots;
    let body = col()
        .w_full()
        .gap(16.0)
        .child(text(tr("Settle this file")).font(crate::ui::PAGE_TITLE).color(u.ink))
        .child(
            text(path.to_string())
                .font(MONO)
                .color(u.prose)
                .w_full()
                .wrap_anywhere()
                .selectable(true),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(tr("Wins now")).font(SMALL).color(u.muted).w(96.0))
                .child(text(deadlock_core::addons::slot_label(winner)).font(MONO).color(u.muted))
                .child(text(winner_name.clone()).font(BODY).color(u.ink).truncate().grow(1.0).min_w(0.0)),
        )
        .child(options)
        .child(
            row().w_full().child(spacer()).child(
                u.action(tr("Cancel"), ActionKind::Ghost)
                    .key("conflicts-resolve-cancel")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            ),
        );
    body.pipe(|inner| {
            u.dialog_shell()
                .key("conflicts-resolve")
                .role("dialog")
                .label(tr("Settle overlap"))
                .w(480.0)
                .padding(24.0)
                .modal(true, close)
                .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
                    if ev.key == rune::Key::Escape {
                        st.launch.overlap.dialog = Dialog::None;
                        true
                    } else {
                        false
                    }
                })
                .child(inner)
        })
}

/// Swap two slot numbers so the loser wins their shared files. The approved rename path is
/// `load_order::apply_reorder`; it never overwrites and rolls back on failure.
fn make_win(st: &mut State, winner: u16, loser: u16) {
    use crate::notify::{self, Notice};
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(root) = st.game_root.clone() else {
        st.status = tr("No Deadlock install was found. Nothing was renamed.").to_string();
        return;
    };
    let dir = deadlock_core::deploy::addons_dir(&root);
    let result = (|| {
        let files = deadlock_core::load_order::read_slot_files(&dir)?;
        let plan = deadlock_core::load_order::plan_reorder(
            &dir,
            &files,
            &deadlock_core::load_order::swap(winner, loser),
            &deadlock_core::load_order::new_token(),
        )?;
        deadlock_core::load_order::apply_reorder(&plan, true)?;
        Ok::<_, deadlock_core::error::Error>(plan)
    })();
    match result {
        Ok(plan) => {
            crate::data::scan_addons_now(st);
            crate::data::start_addon_scan(st);
            notify::push(
                st,
                Notice::success(fill(
                    tr("Slot {} now wins"),
                    &[&format!("{loser:02}")],
                ))
                .detail(tr("The two mods swapped slot numbers."))
                .undo(move |st: &mut State| {
                    let reverse = deadlock_core::load_order::reverse_plan(&plan);
                    if deadlock_core::load_order::apply_reorder(&reverse, true).is_ok() {
                        crate::data::scan_addons_now(st);
                        crate::data::start_addon_scan(st);
                        notify::push(st, Notice::success(tr("Slot numbers are back")));
                    }
                }),
            );
        }
        Err(error) => {
            st.status = error.to_string();
        }
    }
}

/// `materials/models/heroes/.../body.vmat_c`: keep the start and the file name, drop the
/// middle, so the part that tells two paths apart stays visible.
#[cfg(test)]
fn middle(path: &str, max: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max {
        return path.to_string();
    }
    let tail = (max * 3) / 5;
    let headn = max - tail - 1;
    let start: String = chars[..headn].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}\u{2026}{end}")
}

/// A tiny pipe so the dialog's body can be built in one expression.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

/// Unused in some builds; keeps the import of `Path` honest where the helpers move.
#[allow(dead_code)]
fn _addons_dir(root: &Path) -> std::path::PathBuf {
    deadlock_core::deploy::addons_dir(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_keeps_the_start_and_the_file_name() {
        let path = "materials/models/heroes/billy/billy_body.vmat_c";
        let short = middle(path, 24);
        assert!(short.chars().count() <= 24);
        assert!(short.ends_with("body.vmat_c"));
        assert_eq!(middle("short.vmat_c", 96), "short.vmat_c");
    }

    #[test]
    fn a_restore_entry_labels_what_it_hides() {
        use deadlock_core::impact::RestoreKind;
        let mut list = Ignores::default();
        list.add_pair(1, 3);
        let names = std::collections::BTreeMap::from([(1, "Alpha".to_string())]);
        let entries = impact::restore_entries(&list, &names);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].label.contains("Alpha"));
        assert_eq!(entries[0].kind, RestoreKind::Pair(1, 3));
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Replace the cached list without touching disk, for the scenes only.
fn stage_ignores(list: Ignores) {
    if let Ok(mut cached) = cache().lock() {
        *cached = list;
        IGNORES_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn scene_base(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Conflicts;
    stage_ignores(Ignores::default());
    // Names for the dialog and the restore list, as a real library read would give.
    st.installed.library.mods = [
        (1, "infernus glow"),
        (2, "hud pack"),
        (3, "flame trail"),
        (4, "fire loop"),
        (5, "billy recolor"),
        (7, "scoreboard"),
    ]
    .into_iter()
    .map(|(slot, name)| deadlock_core::import::ModMeta {
        slot,
        name: name.to_string(),
        ..deadlock_core::import::ModMeta::default()
    })
    .collect();
}

/// `conflicts-resolve`: one losing slot picked, with both fixes and Ignore.
pub fn scene_resolve(st: &mut State) {
    scene_base(st);
    let Some(conflict) = st.addons.conflicts.first() else {
        return;
    };
    let loser = conflict
        .slots
        .iter()
        .copied()
        .find(|slot| *slot != conflict.winner)
        .unwrap_or(conflict.winner);
    st.launch.overlap.dialog = Dialog::Resolve {
        path: conflict.path.clone(),
        slots: conflict.slots.clone(),
        winner: conflict.winner,
        loser,
    };
}

/// `conflicts-ignored`: the restore list at the bottom of the page.
pub fn scene_ignored(st: &mut State) {
    scene_base(st);
    let mut list = Ignores::default();
    list.add_mod(5);
    list.add_pair(2, 7);
    list.add_file(4, "sounds/heroes/infernus/infernus_fire_loop.vsnd_c");
    stage_ignores(list);
}

/// `w4a-overlap-ignore-all`: Ignore all pressed, asking before every overlap goes to Ignored.
pub fn scene_ignore_all(st: &mut State) {
    scene_base(st);
    ASK_ALL.store(true, std::sync::atomic::Ordering::Relaxed);
}
