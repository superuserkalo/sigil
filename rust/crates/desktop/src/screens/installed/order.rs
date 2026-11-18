//! The load order dialog (Deadlock Mod Manager's "Manage Mod Load Order"): the mods that are on,
//! in the order the game loads them, where the player picks a row and moves it with the keyboard
//! or the row's buttons, then applies the whole order at once.
//!
//! Keys, while the list has focus: Up and Down pick a row; Space picks the row up, and then Up
//! and Down move it and Home and End send it to the top or bottom; Space or Escape puts it down.
//! Alt (Option) with an arrow, Home or End moves the picked row without picking it up first.
//! Escape with nothing held closes the dialog without a change.
//!
//! Nothing is written until Apply. Apply plans the renames with core's two-pass plan
//! (`deadlock_core::load_order::plan_reorder`) and runs them with rollback (`apply_reorder`)
//! through Installed's `reorder`, the same write the drag and the card menu's moves make, with
//! the same Undo. The dialog itself is the confirm, so no second one opens.

use rune::{Color, El, Key, KeyEvent, ScrollSpec, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MICRO, SMALL, Ui};

use super::display_name;

/// The open dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderDialog {
    /// The enabled slots in load order when the dialog opened.
    pub start: Vec<u8>,
    /// The order being edited: the first loads first.
    pub order: Vec<u8>,
    /// The picked row, by the slot it stands for now.
    pub picked: Option<u8>,
    /// The picked row is held: arrows move it instead of picking another.
    pub held: bool,
}

/// One move of the picked row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    First,
    Last,
}

// --- pure rules -------------------------------------------------------------------------------

/// The order after `slot` makes `step`. Unchanged at an end, or when the slot is not listed.
pub fn moved(order: &[u8], slot: u8, step: Move) -> Vec<u8> {
    let Some(at) = order.iter().position(|s| *s == slot) else {
        return order.to_vec();
    };
    let last = order.len().saturating_sub(1);
    let to = match step {
        Move::Up => at.saturating_sub(1),
        Move::Down => (at + 1).min(last),
        Move::First => 0,
        Move::Last => last,
    };
    deadlock_core::load_order::move_to_index(order, slot, to)
}

/// How far a mod moved since the dialog opened: positive when it loads earlier now.
pub fn shift(start: &[u8], order: &[u8], slot: u8) -> isize {
    let from = start.iter().position(|s| *s == slot);
    let to = order.iter().position(|s| *s == slot);
    match (from, to) {
        (Some(from), Some(to)) => from as isize - to as isize,
        _ => 0,
    }
}

/// How many mods end up in a different place.
pub fn changes(start: &[u8], order: &[u8]) -> usize {
    start.iter().zip(order).filter(|(a, b)| a != b).count()
}

/// The row `delta` rows from the picked one, clamped to the list. The first row when none is
/// picked.
pub fn neighbour(order: &[u8], picked: Option<u8>, delta: isize) -> Option<u8> {
    if order.is_empty() {
        return None;
    }
    let Some(at) = picked.and_then(|slot| order.iter().position(|s| *s == slot)) else {
        return order.first().copied();
    };
    let to = (at as isize + delta).clamp(0, order.len() as isize - 1) as usize;
    order.get(to).copied()
}

// --- opening and closing -----------------------------------------------------------------------

/// Open the dialog over the mods that are on now.
pub fn open(st: &mut State) {
    st.installed.menu = None;
    let start = super::group_slots(st, true);
    st.installed.order = Some(OrderDialog { order: start.clone(), picked: start.first().copied(), start, held: false });
}

fn close(st: &mut State) {
    st.installed.order = None;
}

/// Apply the edited order: one plan, one write, one Undo.
fn apply(st: &mut State) {
    let Some(dialog) = st.installed.order.take() else { return };
    super::apply_order(st, &dialog.start, &dialog.order);
}

fn step(st: &mut State, step: Move, tx: &mut Tx) {
    let Some(dialog) = st.installed.order.as_mut() else { return };
    let Some(slot) = dialog.picked else { return };
    dialog.order = moved(&dialog.order, slot, step);
    tx.scroll_into_view(row_key(slot));
}

fn pick(st: &mut State, delta: isize, tx: &mut Tx) {
    let Some(dialog) = st.installed.order.as_mut() else { return };
    dialog.picked = neighbour(&dialog.order, dialog.picked, delta);
    if let Some(slot) = dialog.picked {
        tx.scroll_into_view(row_key(slot));
    }
}

/// The list's keyboard. True when the key was used.
fn on_key(st: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(dialog) = st.installed.order.as_ref() else { return false };
    let moving = dialog.held || ev.mods.alt;
    let end = dialog.order.len() as isize;
    match &ev.key {
        Key::Up if moving => step(st, Move::Up, tx),
        Key::Down if moving => step(st, Move::Down, tx),
        Key::Home if moving => step(st, Move::First, tx),
        Key::End if moving => step(st, Move::Last, tx),
        Key::Up => pick(st, -1, tx),
        Key::Down => pick(st, 1, tx),
        Key::Home => pick(st, -end, tx),
        Key::End => pick(st, end, tx),
        Key::Text(space) if space == " " && !ev.mods.cmd => {
            if let Some(dialog) = st.installed.order.as_mut()
                && dialog.picked.is_some()
            {
                dialog.held = !dialog.held;
            }
        }
        Key::Escape if dialog.held => {
            if let Some(dialog) = st.installed.order.as_mut() {
                dialog.held = false;
            }
        }
        _ => return false,
    }
    true
}

// --- the view ----------------------------------------------------------------------------------

fn row_key(slot: u8) -> String {
    format!("order-row-{slot:02}")
}

/// The toolbar button that opens the dialog. Shown while two or more mods are on.
pub fn open_button(s: &State, u: &Ui) -> Option<El> {
    let on = s.addons.addons.iter().filter(|a| a.enabled).count();
    if on < 2 {
        return None;
    }
    Some(
        u.icon_button(&ARROW_UP_DOWN, tr("Change the load order"))
            .key("installed-load-order")
            .border(1.0, u.hair_strong)
            .cursor(rune::Cursor::Pointer)
            .on_press(open),
    )
}

/// The dialog, mounted with Installed's dialogs.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let dialog = s.installed.order.as_ref()?;
    let changed = changes(&dialog.start, &dialog.order);
    let mut list = col()
        .key("order-list")
        .w_full()
        .shrink()
        .min_h(0.0)
        .max_h(420.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair)
        .focusable(true)
        .autofocus()
        .focus_within_ring(1.0, -1.0, u.accent.alpha(0.6))
        .role("listbox")
        .label(tr("Mods in load order"))
        .on_key(on_key);
    // The slots stay the ones the mods have now; Apply hands them out in the new order.
    for (index, slot) in dialog.order.iter().enumerate() {
        let new_slot = dialog.start.get(index).copied().unwrap_or(*slot);
        list = list.child(order_row(s, u, dialog, index, *slot, new_slot));
    }
    let hint = if dialog.held {
        tr("Up and Down move it. Home and End send it to the top or bottom. Space puts it down.")
    } else {
        tr("Up and Down pick a mod. Space picks it up to move it, or hold Alt with an arrow.")
    };
    let mut foot = row().w_full().gap(8.0).center_y().pt(4.0);
    if changed > 0 {
        foot = foot
            .child(
                u.action(tr("Undo changes"), ActionKind::Ghost)
                    .key("order-reset")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        if let Some(dialog) = st.installed.order.as_mut() {
                            dialog.order = dialog.start.clone();
                            dialog.held = false;
                        }
                    }),
            )
            .child(
                text(if changed == 1 {
                    tr("1 mod moves").to_string()
                } else {
                    fill(tr("{} mods move"), &[&changed.to_string()])
                })
                .font(SMALL)
                .color(u.faint),
            );
    }
    foot = foot
        .child(spacer())
        .child(
            u.action(tr("Cancel"), ActionKind::Ghost)
                .key("order-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        )
        .child(
            u.action(tr("Apply"), ActionKind::Primary)
                .key("order-apply")
                .disabled(changed == 0)
                .cursor(rune::Cursor::Pointer)
                .on_press(apply),
        );
    Some(
        u.dialog_shell()
            .key("order-dialog")
            .role("dialog")
            .label(tr("Load order"))
            .w(560.0)
            .max_h(640.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .on_key(|st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(text(tr("Load order")).font(H1).color(u.ink))
            .child(
                text(tr("Mods higher in this list load first. When two mods change the same file, the higher one wins."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(list)
            .child(text(hint).font(SMALL).color(u.faint).wrap())
            .child(foot),
    )
}

fn order_row(s: &State, u: &Ui, dialog: &OrderDialog, index: usize, slot: u8, new_slot: u8) -> El {
    let picked = dialog.picked == Some(slot);
    let held = picked && dialog.held;
    let name = s
        .addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| display_name(s, a))
        .unwrap_or_else(|| format!("{slot:02}"));
    let mut line = row()
        .key(row_key(slot))
        .w_full()
        .h(40.0)
        .px(12.0)
        .gap(10.0)
        .center_y()
        .role("option")
        .checked(picked)
        .label(name.clone())
        .fill(if held {
            u.accent.alpha(0.16)
        } else if picked {
            u.selected
        } else {
            Color::TRANSPARENT
        })
        .border_sides(if index == 0 { [0.0; 4] } else { [1.0, 0.0, 0.0, 0.0] }, u.hair)
        .hover(if picked { u.selected } else { u.hover })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            if let Some(dialog) = st.installed.order.as_mut() {
                if dialog.picked != Some(slot) {
                    dialog.held = false;
                }
                dialog.picked = Some(slot);
            }
        })
        .child(text(format!("{new_slot:02}")).font(MICRO).color(if picked { u.ink } else { u.faint }).w(20.0))
        .child(
            text(name.clone())
                .font(if picked { BODY_MED } else { BODY })
                .color(u.ink)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
    let moved_by = shift(&dialog.start, &dialog.order, slot);
    if moved_by != 0 {
        let up = moved_by > 0;
        let words = if up {
            fill(tr("Up {}"), &[&moved_by.unsigned_abs().to_string()])
        } else {
            fill(tr("Down {}"), &[&moved_by.unsigned_abs().to_string()])
        };
        line = line.child(
            row()
                .h(20.0)
                .px(6.0)
                .gap(2.0)
                .center_y()
                .radius(crate::ui::r(5.0))
                .fill(u.accent.alpha(0.12))
                .label(words)
                .child(icon(if up { &crate::icons::ARROW_UP } else { &ARROW_DOWN }, 11.0).color(u.accent).a11y_hidden())
                .child(text(moved_by.unsigned_abs().to_string()).font(MICRO).color(u.accent).a11y_hidden()),
        );
    }
    if picked {
        let last = dialog.order.len().saturating_sub(1);
        for (key, glyph, words, step_to, can) in [
            ("first", &ARROW_UP_TO_LINE, tr("Move to top"), Move::First, index > 0),
            ("last", &ARROW_DOWN_TO_LINE, tr("Move to bottom"), Move::Last, index < last),
        ] {
            let b = u.icon_button(glyph, words).key(format!("order-{key}-{slot:02}"));
            line = line.child(if can {
                b.cursor(rune::Cursor::Pointer).on_press_tx(move |st: &mut State, tx: &mut Tx| step(st, step_to, tx))
            } else {
                b.disabled(true).opacity(0.35)
            });
        }
    } else {
        line = line.child(el().w(4.0));
    }
    line
}

/// Lucide arrow-up-down (ISC), drawn like `icons.rs`.
static ARROW_UP_DOWN: rune::Icon = rune::Icon {
    name: "sigil-order-arrow-up-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="m21 16-4 4-4-4"/><path d="M17 20V4"/><path d="m3 8 4-4 4 4"/><path d="M7 4v16"/></g>"#,
};

/// Lucide arrow-down (ISC).
static ARROW_DOWN: rune::Icon = rune::Icon {
    name: "sigil-order-arrow-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></g>"#,
};

/// Lucide arrow-up-to-line (ISC).
static ARROW_UP_TO_LINE: rune::Icon = rune::Icon {
    name: "sigil-order-arrow-up-to-line",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3h14"/><path d="m18 13-6-6-6 6"/><path d="M12 7v14"/></g>"#,
};

/// Lucide arrow-down-to-line (ISC).
static ARROW_DOWN_TO_LINE: rune::Icon = rune::Icon {
    name: "sigil-order-arrow-down-to-line",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17V3"/><path d="m6 11 6 6 6-6"/><path d="M19 21H5"/></g>"#,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_row_moves_and_stops_at_the_ends() {
        let order = [2, 3, 5, 9];
        assert_eq!(moved(&order, 5, Move::Up), vec![2, 5, 3, 9]);
        assert_eq!(moved(&order, 5, Move::Down), vec![2, 3, 9, 5]);
        assert_eq!(moved(&order, 5, Move::First), vec![5, 2, 3, 9]);
        assert_eq!(moved(&order, 3, Move::Last), vec![2, 5, 9, 3]);
        assert_eq!(moved(&order, 2, Move::Up), order.to_vec(), "the top stays the top");
        assert_eq!(moved(&order, 9, Move::Down), order.to_vec(), "the bottom stays the bottom");
        assert_eq!(moved(&order, 7, Move::First), order.to_vec(), "a slot that is not listed");
    }

    #[test]
    fn the_dialog_counts_what_moved() {
        let start = [2, 3, 5, 9];
        let order = moved(&start, 9, Move::First);
        assert_eq!(shift(&start, &order, 9), 3, "up three places");
        assert_eq!(shift(&start, &order, 2), -1);
        assert_eq!(changes(&start, &order), 4);
        assert_eq!(changes(&start, &start), 0);
    }

    #[test]
    fn arrows_pick_the_next_row() {
        let order = [2, 3, 5];
        assert_eq!(neighbour(&order, None, 1), Some(2), "nothing picked starts at the top");
        assert_eq!(neighbour(&order, Some(3), 1), Some(5));
        assert_eq!(neighbour(&order, Some(5), 1), Some(5), "stops at the end");
        assert_eq!(neighbour(&order, Some(3), -9), Some(2));
        assert_eq!(neighbour(&[], Some(3), 1), None);
    }

    #[test]
    fn the_edited_order_becomes_the_same_moves_a_drag_makes() {
        // Apply hands the order to core's planner: 9 to the top is three renames.
        let start = [2, 3, 5, 9];
        let order = moved(&start, 9, Move::First);
        let moves = deadlock_core::load_order::moves_for_order(&start, &order);
        assert_eq!(moves.len(), 4);
        assert!(moves.iter().any(|m| m.from == 9 && m.to == 2));
    }
}
