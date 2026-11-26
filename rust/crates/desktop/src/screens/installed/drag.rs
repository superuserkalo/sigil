//! Drag to reorder in Installed: a card's grip moves it within its group (Enabled or
//! Disabled), and the drop asks for the same load order change as the card menu's Load
//! earlier and Load later. The renames are core's two-pass plan with rollback
//! (`deadlock_core::load_order::plan_reorder` and `apply_reorder`); the drag only picks the
//! order.
//!
//! Drag works in the Load order sort, where the page shows the order the game loads and
//! nothing else (saved and pinned mods are not lifted there). In the other sorts the grip
//! stays, quieter, and a press on it switches back to Load order: an order the page does not
//! show is never changed by a drop. While cards are picked the grip is hidden. With a search
//! or a filter on, a card lands next to the card it is dropped on, and the hidden ones keep
//! their place in between.
//!
//! The dragged card follows the pointer over the others, the 2 point line marks where it
//! lands, and near the top or bottom edge the page scrolls under it. A multi-file card moves
//! every file of it that is in the section, together.

use std::cell::Cell;
use std::time::Duration;

use rune::{Drag, El, Live, Phase, ScrollPos, Tx, el, icon};

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

/// The page's scroll and its viewport, kept by the scroll area (`watch`) so a drag can follow
/// the cards under the pointer as the page scrolls, and scroll it near an edge.
struct Page {
    pos: Cell<ScrollPos>,
    rect: Live<[f32; 4]>,
    /// The scroll when the drag began.
    start: Cell<f32>,
    /// The dragged card's shift from its place: the pointer's travel plus the scroll since.
    follow: Live<[f32; 2]>,
}

thread_local! {
    static PAGE: Page = Page {
        pos: Cell::new(ScrollPos::default()),
        rect: Live::new([0.0; 4]),
        start: Cell::new(0.0),
        follow: Live::new([0.0; 2]),
    };
}

/// The page's scroll area key, which a drag scrolls.
const SCROLL: &str = "installed-scroll";
/// The band at the top and bottom of the page that scrolls during a drag, and the most it
/// moves in one frame there (at the very edge).
const EDGE: f32 = 56.0;
const SPEED: f32 = 14.0;
const FRAME: Duration = Duration::from_micros(16_667);

/// Has the page's scroll area report its viewport and scroll to the drag.
pub fn watch(scroll: El) -> El {
    let rect = PAGE.with(|p| p.rect.clone());
    scroll.drawn_rect_into(&rect).chain_scroll_offset(|at| PAGE.with(|p| p.pos.set(at)))
}

/// The shift the dragged card draws at, bound without a rebuild per pointer move.
pub fn follow() -> Live<[f32; 2]> {
    PAGE.with(|p| p.follow.clone())
}

fn scrolled_since_start() -> f32 {
    PAGE.with(|p| p.pos.get().offset[1] - p.start.get())
}

/// How far a pointer at `y` (window points) scrolls a viewport `[top, height]` that can move
/// `room` [up, down]: negative near the top, positive near the bottom, faster nearer the edge.
pub fn edge_speed(y: f32, viewport: [f32; 2], room: [f32; 2]) -> f32 {
    let [top, h] = viewport;
    if h <= 0.0 {
        return 0.0;
    }
    let band = EDGE.min(h / 4.0);
    let at = y - top;
    if at < band && room[0] > 0.0 {
        -SPEED * ((band - at) / band).min(1.0)
    } else if at > h - band && room[1] > 0.0 {
        SPEED * ((at - (h - band)) / band).min(1.0)
    } else {
        0.0
    }
}

/// One frame of edge scrolling, and a request for the next while the pointer stays there.
fn edge_scroll(d: &Drag, tx: &mut Tx) {
    PAGE.with(|p| {
        let (mut at, [_, y, _, h]) = (p.pos.get(), p.rect.get());
        let room = [at.offset[1], (at.max[1] - at.offset[1]).max(0.0)];
        let speed = edge_speed(d.window_point()[1], [y, h], room);
        if speed == 0.0 {
            return;
        }
        let to = (at.offset[1] + speed).clamp(0.0, at.max[1]);
        at.offset[1] = to;
        p.pos.set(at);
        tx.scroll_to_instant(SCROLL, 0.0, to);
        tx.drag_repeat_after(FRAME);
    });
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
#[cfg(test)]
pub fn wanted_order(group: &[u16], shown: &[u16], slot: u16, to: usize) -> Vec<u16> {
    wanted_block(group, shown, slot, &[slot], to)
}

/// [`wanted_order`] for a card that speaks for several slots (`members`, a multi-file mod):
/// they move as one block, in their own order, to the place of `shown[to]`. `shown` holds one
/// slot per card, `card` the dragged one's.
pub fn wanted_block(group: &[u16], shown: &[u16], card: u16, members: &[u16], to: usize) -> Vec<u16> {
    let Some(target) = shown.get(to).copied() else {
        return group.to_vec();
    };
    let Some(from) = shown.iter().position(|s| *s == card) else {
        return group.to_vec();
    };
    if target == card || members.contains(&target) || !group.contains(&target) {
        return group.to_vec();
    }
    let block: Vec<u16> = group.iter().copied().filter(|s| members.contains(s)).collect();
    let mut rest: Vec<u16> = group.iter().copied().filter(|s| !members.contains(s)).collect();
    let Some(at) = rest.iter().position(|s| *s == target) else {
        return group.to_vec();
    };
    let at = if to > from { at + 1 } else { at };
    rest.splice(at..at, block);
    rest
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

/// The slots of the card that speaks for `slot` which sit in its section (a multi-file card
/// can hold files of both).
fn members(st: &State, slot: u16, enabled: bool) -> Vec<u16> {
    let shown = super::shown_addons(st);
    layout::groups(st, &shown)
        .iter()
        .find(|card| card.primary().slot == slot)
        .map(|card| card.addons.iter().filter(|a| a.enabled == enabled).map(|a| a.slot).collect())
        .unwrap_or_else(|| vec![slot])
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
/// In a sort other than Load order it is the way back there: the grip, quieter, whose press
/// sorts by load order.
pub fn grip(s: &State, u: &Ui, slot: u16, enabled: bool, pitch: [f32; 2], cols: usize, dragging: bool, show: bool) -> El {
    if !s.installed.selected.is_empty() {
        return el().size(28.0, 28.0).a11y_hidden();
    }
    if s.installed.sort != Sort::LoadOrder {
        let off = el()
            .key(format!("installed-grip-{slot:02}"))
            .size(28.0, 28.0)
            .center()
            .radius(crate::ui::r(8.0))
            .fill(rune::Color::BLACK.alpha(0.45))
            .hover(rune::Color::BLACK.alpha(0.7))
            .focusable(true)
            .role("button")
            .label(tr("Sort by load order to drag"))
            .help(tr("Sort by load order to drag"))
            .cursor(rune::Cursor::Pointer)
            .on_press(super::sort_by_load_order)
            .child(icon(&GRIP_VERTICAL, 14.0).color(rune::Color::WHITE.alpha(0.4)).a11y_hidden());
        return super::hover_reveal(off, show);
    }
    let grip = el().key(format!("installed-grip-{slot:02}"))
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(rune::Color::BLACK.alpha(0.6))
        .hover(rune::Color::BLACK.alpha(0.78))
        .press(u.over.alpha(0.08))
        .label(tr("Drag to change the load order"))
        .help(tr("Drag to change the load order"))
        .cursor(if dragging { rune::Cursor::Grabbing } else { rune::Cursor::Grab })
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| drag(st, slot, enabled, pitch, cols, d, tx))
        .child(icon(&GRIP_VERTICAL, 14.0).color(rune::Color::WHITE.alpha(0.9)).a11y_hidden());
    super::hover_reveal(grip, show || dragging)
}

/// The dragged card's look: it follows the pointer above the others, solid (a see-through
/// card mixed its name with the one under it) and with a shadow; its own place stays empty and
/// the line on the target says where it lands.
pub fn lift(card: El, dragging: bool) -> El {
    if !dragging {
        return card;
    }
    card.live_offset(&follow())
        .raise()
        .shadow(8.0, 24.0, rune::Color::BLACK.alpha(0.45))
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
            PAGE.with(|p| {
                p.start.set(p.pos.get().offset[1]);
                p.follow.set([0.0; 2]);
            });
        }
        Phase::Move => {
            edge_scroll(d, tx);
            let count = st.installed.drag_order.len();
            // The cards moved up by what the page scrolled since the press.
            let dy = d.dy + scrolled_since_start();
            follow().set([d.dx, dy]);
            if let Some(drag) = st.installed.drag.as_mut() {
                let to = grid_target(drag.from, count, cols, pitch, d.dx, dy);
                if to == drag.to {
                    // Nothing but the dragged card moves until the landing place does.
                    tx.live_only();
                }
                drag.to = to;
            }
        }
        Phase::Up => {
            follow().set([0.0; 2]);
            let shown = std::mem::take(&mut st.installed.drag_order);
            let Some(drag) = st.installed.drag.take() else { return };
            if d.cancelled() || drag.from == drag.to {
                return;
            }
            let group = super::group_slots(st, enabled);
            let members = members(st, slot, enabled);
            let wanted = wanted_block(&group, &shown, slot, &members, drag.to);
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

    #[test]
    fn a_multi_file_card_moves_as_one_block() {
        // Cards 2, {5, 11} and 9 (the card of 5 also holds 11): its files move together.
        let group = [2, 3, 5, 9, 11];
        let shown = [2, 3, 5, 9];
        assert_eq!(wanted_block(&group, &shown, 5, &[5, 11], 0), vec![5, 11, 2, 3, 9], "to the top");
        assert_eq!(wanted_block(&group, &shown, 5, &[5, 11], 3), vec![2, 3, 9, 5, 11], "after 9");
        assert_eq!(wanted_block(&group, &shown, 9, &[9], 0), vec![9, 2, 3, 5, 11]);
        assert_eq!(wanted_block(&group, &shown, 5, &[5, 11], 2), group.to_vec(), "on itself");
    }

    #[test]
    fn the_page_scrolls_only_near_an_edge_that_has_more() {
        let viewport = [100.0, 600.0];
        assert_eq!(edge_speed(400.0, viewport, [500.0, 500.0]), 0.0, "the middle stays");
        assert!(edge_speed(105.0, viewport, [500.0, 500.0]) < 0.0, "near the top scrolls up");
        assert!(edge_speed(695.0, viewport, [500.0, 500.0]) > 0.0, "near the bottom scrolls down");
        assert_eq!(edge_speed(105.0, viewport, [0.0, 500.0]), 0.0, "already at the top");
        assert_eq!(edge_speed(695.0, viewport, [500.0, 0.0]), 0.0, "already at the bottom");
        assert!(edge_speed(100.0, viewport, [500.0, 500.0]) < edge_speed(140.0, viewport, [500.0, 500.0]), "faster at the edge");
        assert_eq!(edge_speed(20.0, viewport, [500.0, 500.0]), -SPEED, "past the edge, full speed");
    }
}

/// The page driven headless: a real drag of a real card through the grip.
#[cfg(test)]
mod page_tests {
    use rune::{Env, Runtime};

    use crate::app::State;
    use crate::screens::installed::{Confirm, Sort, layout};

    fn page(prepare: impl FnOnce(&mut State)) -> Runtime<State> {
        let mut s = crate::data::init_state();
        s.prefs.reduce_motion = true;
        s.prefs.skip_confirm.clear();
        crate::app::scene_demo_installed(&mut s);
        prepare(&mut s);
        let mut rt = Runtime::new(s, crate::app::view, (1280.0, 900.0), Env::new(true, 1.0));
        rt.check_live_only(false);
        settle(&mut rt);
        rt
    }

    fn settle(rt: &mut Runtime<State>) {
        for _ in 0..60 {
            if !rt.frame(1.0 / 60.0) {
                break;
            }
        }
    }

    fn centre(rt: &Runtime<State>, key: &str) -> (f32, f32) {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .find(|e| e.key.as_deref() == Some(key))
            .map(|e| (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0))
            .unwrap_or_else(|| panic!("{key} is not on screen"))
    }

    /// Press the grip of `slot`, move in steps to the centre of `onto`'s card, let go.
    fn drag_onto(rt: &mut Runtime<State>, card: &str, slot: u16, onto: &str) {
        let (cx, cy) = centre(rt, &format!("{card}{slot:02}"));
        rt.pointer_move(cx, cy);
        settle(rt);
        let (gx, gy) = centre(rt, &format!("installed-grip-{slot:02}"));
        let (tx, ty) = centre(rt, onto);
        // The pointer keeps its offset inside the card, so it lands on the target's centre.
        let (ex, ey) = (gx + (tx - cx), gy + (ty - cy));
        rt.pointer_move(gx, gy);
        rt.pointer_down(gx, gy);
        rt.frame(1.0 / 60.0);
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            rt.pointer_move(gx + (ex - gx) * t, gy + (ey - gy) * t);
            rt.frame(1.0 / 60.0);
        }
        rt.pointer_up(ex, ey);
        settle(rt);
    }

    fn enabled_slots(s: &State) -> Vec<u16> {
        let mut slots: Vec<u16> = s.addons.addons.iter().filter(|a| a.enabled).map(|a| a.slot).collect();
        slots.sort_unstable();
        slots
    }

    fn save(s: &mut State, slot: u16) {
        let addon = s.addons.addons.iter().find(|a| a.slot == slot).cloned().expect("slot");
        let key = crate::screens::installed::extras_key(s, &addon);
        s.installed.notes.extras.entry(key).favorite = true;
    }

    #[test]
    fn load_order_shows_saved_mods_in_their_place() {
        let mut s = crate::data::init_state();
        crate::app::scene_demo_installed(&mut s);
        let on = enabled_slots(&s);
        let last = *on.last().expect("enabled mods");
        save(&mut s, last);
        s.installed.sort = Sort::LoadOrder;
        assert_eq!(super::order(&s, true), on, "the page is the load order, saved or not");
        s.installed.sort = Sort::Name;
        assert_eq!(super::order(&s, true).first(), Some(&last), "other sorts still lift saved mods");
    }

    #[test]
    fn a_drop_on_the_first_card_loads_first_even_with_a_saved_mod() {
        // The bug: a saved mod sat first in Load order, and dropping a card on it put the card
        // after it in slot terms, at the bottom.
        let mut on = Vec::new();
        let mut rt = page(|s| {
            s.installed.layout = layout::Layout::List;
            on = enabled_slots(s);
            let last = *on.last().expect("enabled mods");
            save(s, last);
        });
        // Drop on the top row of the section, whatever the page draws there.
        let top = super::order(&rt.state, true)[0];
        let (first, moving) = (on[0], on[2]);
        drag_onto(&mut rt, "addon-row-", moving, &format!("addon-row-{top:02}"));
        let Some(Confirm::Reorder(plan)) = rt.state.installed.confirm.clone() else {
            panic!("the drop asks to change the load order; status: {}", rt.state.status);
        };
        let wanted = deadlock_core::load_order::move_to_index(&on, moving, 0);
        let mut got: Vec<(u16, u16)> = plan.moves.iter().map(|m| (m.from, m.to)).collect();
        let mut expected: Vec<(u16, u16)> =
            deadlock_core::load_order::moves_for_order(&on, &wanted).iter().map(|m| (m.from, m.to)).collect();
        got.sort_unstable();
        expected.sort_unstable();
        assert_eq!(got, expected);
        assert!(got.contains(&(moving, first)), "{moving} takes slot {first}");
    }

    #[test]
    fn a_grid_drop_lands_on_the_card_under_the_pointer() {
        let mut on = Vec::new();
        let mut rt = page(|s| on = enabled_slots(s));
        let (moving, onto) = (on[0], on[2]);
        drag_onto(&mut rt, "addon-", moving, &format!("addon-{onto:02}"));
        let Some(Confirm::Reorder(plan)) = rt.state.installed.confirm.clone() else {
            panic!("the drop asks to change the load order; status: {}", rt.state.status);
        };
        // Moving later lands after the target: the first card takes the third place.
        assert!(plan.moves.iter().any(|m| m.from == moving && m.to == onto), "{:?}", plan.moves);
        assert_eq!(rt.state.installed.drag, None, "the drag ends with the drop");
    }

    #[test]
    fn outside_load_order_the_grip_switches_back_and_never_reorders() {
        let mut on = Vec::new();
        let mut rt = page(|s| {
            s.installed.layout = layout::Layout::List;
            s.installed.sort = Sort::Name;
            on = enabled_slots(s);
        });
        let slot = on[1];
        let (x, y) = centre(&rt, &format!("addon-row-{slot:02}"));
        rt.pointer_move(x, y);
        settle(&mut rt);
        let (gx, gy) = centre(&rt, &format!("installed-grip-{slot:02}"));
        rt.pointer_move(gx, gy);
        rt.pointer_down(gx, gy);
        rt.frame(1.0 / 60.0);
        rt.pointer_move(gx, gy + 140.0);
        rt.frame(1.0 / 60.0);
        rt.pointer_up(gx, gy + 140.0);
        settle(&mut rt);
        assert!(rt.state.installed.confirm.is_none(), "no reorder from a sort the page does not show");
        // A press on the quiet grip is the way back.
        rt.pointer_move(gx, gy);
        settle(&mut rt);
        let (gx, gy) = centre(&rt, &format!("installed-grip-{slot:02}"));
        rt.pointer_down(gx, gy);
        rt.frame(1.0 / 60.0);
        rt.pointer_up(gx, gy);
        settle(&mut rt);
        assert_eq!(rt.state.installed.sort, Sort::LoadOrder, "the press sorted by load order");
        assert!(rt.state.installed.confirm.is_none());
    }

    #[test]
    fn the_sort_is_kept_by_its_key() {
        for sort in Sort::ALL {
            assert_eq!(Sort::from_key(sort.key()), sort);
        }
        assert_eq!(Sort::from_key(""), Sort::LoadOrder, "nothing saved is load order");
        assert_eq!(Sort::from_key("bogus"), Sort::LoadOrder);
    }
}
