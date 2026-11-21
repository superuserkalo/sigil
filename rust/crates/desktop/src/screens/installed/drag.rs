//! Drag to reorder in Installed: a card's grip moves it within its group (Enabled or
//! Disabled), and the drop asks for the same load order change as the card menu's Load
//! earlier and Load later. The renames are core's two-pass plan with rollback
//! (`deadlock_core::load_order::plan_reorder` and `apply_reorder`); the drag only picks the
//! order.
//!
//! Drag works in the Load order sort, where the page shows the order the game loads; in the
//! other sorts and while cards are picked the grip is hidden. With a filter on, the order moves
//! between the cards shown, and the hidden ones keep their place in between.

use rune::{Drag, El, Phase, Tx, el, icon};

use crate::app::State;
use crate::screens::settings::i18n::tr;
use crate::ui::Ui;

use super::{Sort, layout};

/// A card on the move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dragging {
    pub slot: u16,
    pub enabled: bool,
    /// Its place among the section's cards when the drag began, and where it would land.
    pub from: usize,
    pub to: usize,
}

/// Where a card dragged by `(dx, dy)` lands in a grid of `count` cards, `cols` across, one card
/// and its gap measuring `pitch`. A list is a grid of one column.
pub fn grid_target(from: usize, count: usize, cols: usize, pitch: [f32; 2], dx: f32, dy: f32) -> usize {
    if count == 0 {
        return 0;
    }
    let cols = cols.max(1) as isize;
    let from = from.min(count - 1) as isize;
    let col = from % cols;
    let row = from / cols;
    let step_x = if pitch[0] > 0.0 { (dx / pitch[0]).round() as isize } else { 0 };
    let step_y = if pitch[1] > 0.0 { (dy / pitch[1]).round() as isize } else { 0 };
    let col = (col + step_x).clamp(0, cols - 1);
    let row = (row + step_y).max(0);
    (row * cols + col).clamp(0, count as isize - 1) as usize
}

/// The load order of `group` (every slot of the section, lowest first) after the card `slot`
/// moves onto the place of `shown[to]`: after it when moving down, before it when moving up.
pub fn wanted_order(group: &[u16], shown: &[u16], slot: u16, to: usize) -> Vec<u16> {
    let Some(target) = shown.get(to).copied() else {
        return group.to_vec();
    };
    if target == slot {
        return group.to_vec();
    }
    let Some(at) = group.iter().position(|s| *s == target) else {
        return group.to_vec();
    };
    deadlock_core::load_order::move_to_index(group, slot, at)
}

/// The cards of one section in the order they are drawn, by the slot each speaks for.
pub fn order(st: &State, enabled: bool) -> Vec<u16> {
    let shown = super::shown_addons(st);
    layout::groups(st, &shown)
        .iter()
        .filter(|card| card.enabled() == enabled)
        .map(|card| card.primary().slot)
        .collect()
}

/// Whether the page lets cards be dragged now.
pub fn allowed(s: &State) -> bool {
    s.installed.sort == Sort::LoadOrder && s.installed.selected.is_empty()
}

/// The drop mark for a card: `Some(true)` when the dragged card lands after it.
pub fn drop_mark(s: &State, slot: u16, enabled: bool) -> Option<bool> {
    let d = s.installed.drag?;
    if d.enabled != enabled || d.from == d.to || d.slot == slot {
        return None;
    }
    let target = match s.installed.drag_order.get(d.to) {
        Some(target) => *target,
        // A drag staged without its order (a scene): work it out.
        None => *order(s, enabled).get(d.to)?,
    };
    (target == slot).then_some(d.to > d.from)
}

/// The grip on a card or row. `pitch` is one card and its gap; `cols` the cards across.
pub fn grip(u: &Ui, slot: u16, enabled: bool, pitch: [f32; 2], cols: usize, dragging: bool, show: bool) -> El {
    let grip = el().key(format!("installed-grip-{slot:02}"))
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(rune::Color::BLACK.alpha(0.6))
        .hover(rune::Color::BLACK.alpha(0.78))
        .press(u.over.alpha(0.08))
        .label(tr("Drag to change the load order"))
        .cursor(if dragging { rune::Cursor::Grabbing } else { rune::Cursor::Grab })
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| drag(st, slot, enabled, pitch, cols, d, tx))
        .child(icon(&GRIP_VERTICAL, 14.0).color(rune::Color::WHITE.alpha(0.9)).a11y_hidden());
    super::hover_reveal(grip, show || dragging)
}

fn drag(st: &mut State, slot: u16, enabled: bool, pitch: [f32; 2], cols: usize, d: &mut Drag, tx: &mut Tx) {
    match d.phase {
        Phase::Down => {
            // The order is worked out once here: every pointer move and every card's drop
            // mark read it, where each sorted the whole library again.
            let shown = order(st, enabled);
            let from = shown.iter().position(|s| *s == slot).unwrap_or(0);
            st.installed.drag_order = shown;
            st.installed.drag = Some(Dragging { slot, enabled, from, to: from });
        }
        Phase::Move => {
            let count = st.installed.drag_order.len();
            if let Some(drag) = st.installed.drag.as_mut() {
                let to = grid_target(drag.from, count, cols, pitch, d.dx, d.dy);
                if to == drag.to {
                    // Nothing on screen moves until the landing place does.
                    tx.live_only();
                }
                drag.to = to;
            }
        }
        Phase::Up => {
            let shown = std::mem::take(&mut st.installed.drag_order);
            let Some(drag) = st.installed.drag.take() else { return };
            if d.cancelled() || drag.from == drag.to {
                return;
            }
            let group = super::group_slots(st, enabled);
            let wanted = wanted_order(&group, &shown, slot, drag.to);
            super::ask_reorder(st, &group, &wanted);
        }
    }
}

/// Lucide grip-vertical (ISC), drawn like `icons.rs`.
static GRIP_VERTICAL: rune::Icon = rune::Icon {
    name: "sigil-installed-grip-vertical",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="12" r="1"/><circle cx="9" cy="5" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="19" r="1"/></g>"#,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_lands_where_the_pointer_points() {
        let pitch = [200.0, 150.0];
        assert_eq!(grid_target(0, 8, 4, pitch, 0.0, 0.0), 0);
        assert_eq!(grid_target(0, 8, 4, pitch, 210.0, 0.0), 1, "one card right");
        assert_eq!(grid_target(1, 8, 4, pitch, -30.0, 160.0), 5, "one row down");
        assert_eq!(grid_target(3, 8, 4, pitch, 900.0, 0.0), 3, "never past the row's end");
        assert_eq!(grid_target(5, 6, 4, pitch, 0.0, 600.0), 5, "clamped to the last card");
        assert_eq!(grid_target(2, 5, 1, [0.0, 52.0], 0.0, -110.0), 0, "a list moves by rows");
    }

    #[test]
    fn the_drop_becomes_a_load_order() {
        let group = [2, 3, 5, 9, 11];
        assert_eq!(wanted_order(&group, &group, 9, 0), vec![9, 2, 3, 5, 11], "up to the top");
        assert_eq!(wanted_order(&group, &group, 3, 3), vec![2, 5, 9, 3, 11], "down lands after the target");
        assert_eq!(wanted_order(&group, &group, 3, 1), group.to_vec(), "dropped on itself");
        // A filter shows 2, 9 and 11: dropping 11 on 9 puts it right before 9.
        assert_eq!(wanted_order(&group, &[2, 9, 11], 11, 1), vec![2, 3, 5, 11, 9]);
    }
}
