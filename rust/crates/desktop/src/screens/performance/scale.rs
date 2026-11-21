//! The looks to frames scale: a stepped slider over the config tiers, lighter on the left.
//!
//! The first stop is "All", then one stop per tier that has configs. The thumb always sits on
//! a real stop, so it never suggests a filter that is not on. Dragging moves it live (from
//! `drag_tier`) and filters the list once, on release; a click jumps to the nearest stop.
//! Arrow keys step, Home and End jump to the ends, Escape goes back to "All". Every part has a
//! fixed size: nothing moves while dragging but the thumb and the readout text.

use deadlock_core::performance::{Catalog, Tier};
use rune::{Drag, El, Key, KeyEvent, Phase, Tx, col, el, row, text};

use crate::app::State;
use crate::ui::{BODY_MED, LABEL, SMALL, SMALL_NUM, Ui, switch_green};

use super::{fill, tr};

/// The whole control's height; the track line is centred at `LINE_Y`.
const H: f32 = 48.0;
const LINE_Y: f32 = 14.0;
const TRACK_H: f32 = 4.0;
const STOP: f32 = 8.0;
const THUMB_W: f32 = 12.0;
const THUMB_H: f32 = 20.0;
const LABEL_Y: f32 = 28.0;
/// The track's width bounds; it takes what the row leaves between them.
const TRACK_MIN: f32 = 300.0;
const TRACK_MAX: f32 = 440.0;

/// The stop nearest to a point `x` on a track `width` wide with `n` evenly spaced stops.
fn nearest(x: f32, width: f32, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let frac = (x / width.max(1.0)).clamp(0.0, 1.0);
    ((frac * (n - 1) as f32).round() as usize).min(n - 1)
}

/// The tiers that have configs, lighter first, with their counts.
pub fn tiers(catalog: &Catalog) -> Vec<(Tier, usize)> {
    Tier::ALL
        .into_iter()
        .map(|tier| (tier, catalog.configs.iter().filter(|c| c.def.tier == tier).count()))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// A stop's track cell: half the line to each side and the stop in the middle. Cells share the
/// track equally, so stops sit evenly whatever width the track gets.
fn stop_cell(u: &Ui, i: usize, n: usize, lit_left: bool, lit_right: bool, on: bool, filled: bool) -> El {
    let half = |show: bool, lit: bool| {
        el().w(0.0)
            .grow(1.0)
            .h(TRACK_H)
            .fill(if lit { u.muted } else { u.over.alpha(0.12) })
            .opacity(if show { 1.0 } else { 0.0 })
    };
    let stop = el()
        .size(STOP, STOP)
        .radius(1.0)
        .fill(if on {
            switch_green()
        } else if filled {
            u.muted
        } else {
            u.raised
        })
        .border(1.0, if on { switch_green() } else { u.hair_strong });
    row()
        .w(0.0)
        .grow(1.0)
        .h(THUMB_H)
        .center_y()
        .child(half(i > 0, lit_left))
        .child(row().w(STOP).h(THUMB_H).center().child(stop))
        .child(half(i + 1 < n, lit_right))
}

/// `on` is the tier of the applied config, its stop green like the toggles. `note` is the
/// readout's second line.
pub fn view(s: &State, u: &Ui, catalog: &Catalog, on: Option<Tier>, note: String) -> El {
    let tiers = tiers(catalog);
    let total = catalog.configs.len();
    // Stop 0 is "All", then the tiers lighter first.
    let stops: Vec<(Option<Tier>, usize)> =
        std::iter::once((None, total)).chain(tiers.iter().map(|(t, n)| (Some(*t), *n))).collect();
    let n = stops.len();
    let index = |t: Option<Tier>| stops.iter().position(|(x, _)| *x == t).unwrap_or(0);
    // What the thumb shows: the stop being dragged to, else the filter.
    let shown = s.performance.drag_tier.unwrap_or(s.performance.tier);
    let picked = index(shown);

    let mut cells = row().w_full().h(THUMB_H).a11y_hidden();
    let mut labels = row().w_full().h(16.0).a11y_hidden();
    for (i, (tier, _)) in stops.iter().enumerate() {
        let lit_left = i > 0 && i <= picked;
        let lit_right = i < picked;
        cells = cells.child(stop_cell(u, i, n, lit_left, lit_right, false, i < picked));
        let applied = tier.is_some() && on == *tier;
        let label = match tier {
            Some(tier) => tr(tier.label()),
            None => tr("All"),
        };
        labels = labels.child(
            // The applied config's tier carries a small green mark beside its name, like "On" in
            // the list, so it never reads as a second thumb on the track.
            row()
                .w(0.0)
                .grow(1.0)
                .h(16.0)
                .gap(4.0)
                .center()
                .children(applied.then(|| el().size(6.0, 6.0).radius(1.0).fill(switch_green())))
                .child(
                    text(label.to_string())
                        .font(if picked == i { LABEL } else { SMALL })
                        .color(if picked == i { u.ink } else { u.muted })
                        .truncate(),
                ),
        );
    }
    // The thumb, drawn as an overlay row of the same cells so it lands on its stop without
    // knowing the track's width.
    let mut overlay = row().absolute(0.0, LINE_Y - THUMB_H / 2.0).w_full().h(THUMB_H).a11y_hidden();
    for i in 0..n {
        let mut c = row().w(0.0).grow(1.0).h(THUMB_H).center();
        if i == picked {
            c = c.child(el().size(THUMB_W, THUMB_H).radius(2.0).fill(u.prose).border(1.0, u.bg));
        }
        overlay = overlay.child(c);
    }
    let cell_col = col()
        .w_full()
        .child(el().h(LINE_Y - THUMB_H / 2.0))
        .child(cells)
        .child(el().h(LABEL_Y - LINE_Y - THUMB_H / 2.0))
        .child(labels)
        .child(overlay);

    let order: Vec<Option<Tier>> = stops.iter().map(|(t, _)| *t).collect();
    let drag_order = order.clone();
    let key_order = order;
    let track = row()
        .key("perf-scale")
        .w(0.0)
        .grow(1.0)
        .min_w(TRACK_MIN)
        .max_w(TRACK_MAX)
        .h(H)
        .focusable(true)
        .role("slider")
        .label(tr("Looks to frames").to_string())
        .a11y_value(match stops[picked].0 {
            Some(tier) => tr(tier.label()).to_string(),
            None => tr("All tiers").to_string(),
        })
        .cursor(rune::Cursor::Pointer)
        .child(cell_col)
        .on_drag(move |st: &mut State, d: &mut Drag, _tx: &mut Tx| {
            // Cells of equal width: the cell under the pointer is the nearest stop.
            let n = drag_order.len();
            if n == 0 {
                return;
            }
            if d.cancelled() {
                st.performance.drag_tier = None;
                return;
            }
            let cell = d.w.max(1.0) / n as f32;
            let i = nearest(d.x - cell / 2.0, d.w - cell, n);
            match d.phase {
                Phase::Up => {
                    st.performance.tier = drag_order[i];
                    st.performance.drag_tier = None;
                }
                _ => st.performance.drag_tier = Some(drag_order[i]),
            }
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            let len = key_order.len();
            let now = key_order.iter().position(|x| *x == st.performance.tier).unwrap_or(0);
            let next = match ev.key {
                Key::Left | Key::Down => now.saturating_sub(1),
                Key::Right | Key::Up => (now + 1).min(len - 1),
                Key::Home => 0,
                Key::End => len - 1,
                Key::Escape if now > 0 => 0,
                _ => return false,
            };
            st.performance.tier = key_order[next];
            true
        });

    // Ends sit on the track line.
    let end = |label: &str, w: f32, right: bool| {
        row()
            .w(w)
            .h(H)
            .a11y_hidden()
            .child(
                row().w(w).h(16.0).mt(LINE_Y - 8.0).child(
                    text(tr(label).to_string())
                        .font(LABEL)
                        .color(u.muted)
                        .w(w)
                        .align(if right { rune::Align::End } else { rune::Align::Start }),
                ),
            )
    };

    // The readout: what the thumb points at, in a box the text never sizes. Line 2 is the note
    // (how fresh the configs are); the tier's blurb already heads its group below.
    let (name, count) = match stops[picked] {
        (Some(tier), count) => (tr(tier.label()).to_string(), count),
        (None, count) => (tr("Every tier").to_string(), count),
    };
    let readout = col()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .h(H)
        .pt(LINE_Y - 10.0)
        .a11y_hidden()
        .child(
            row()
                .h(20.0)
                .gap(8.0)
                .center_y()
                .child(text(name).font(BODY_MED).color(u.ink).truncate())
                .child(
                    text(fill(tr("{} configs"), &[&count.to_string()]))
                        .font(SMALL_NUM)
                        .color(u.muted)
                        .truncate()
                        .min_w(0.0)
                        .shrink(),
                ),
        )
        .child(row().w_full().h(16.0).child(text(note).font(SMALL).color(u.muted).truncate()));

    row()
        .w_full()
        .min_w(0.0)
        .h(H)
        .child(end("Looks", 44.0, false))
        .child(el().w(8.0))
        .child(track)
        .child(el().w(8.0))
        .child(end("Frames", 56.0, false))
        .child(el().w(16.0))
        .child(readout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_the_nearest_stop() {
        assert_eq!(nearest(-20.0, 300.0, 4), 0);
        assert_eq!(nearest(49.0, 300.0, 4), 0);
        assert_eq!(nearest(51.0, 300.0, 4), 1);
        assert_eq!(nearest(160.0, 300.0, 4), 2);
        assert_eq!(nearest(999.0, 300.0, 4), 3);
        assert_eq!(nearest(100.0, 300.0, 1), 0);
    }
}
