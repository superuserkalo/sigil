//! The looks to frames scale: a stepped slider over the config tiers, lighter on the left.
//!
//! The thumb only ever sits on a stop. Dragging moves it live (from `drag_tier`) and filters
//! the list once, on release; a click jumps to the nearest stop. Arrow keys step, Home and End
//! jump to the ends, Escape shows every tier again. With every tier shown the thumb rests on
//! the applied config's tier as an outline, so the control always reads as a slider. Every
//! part has a fixed size: nothing moves while dragging but the thumb and the readout text.

use deadlock_core::performance::{Catalog, Tier, catalog};
use rune::{Drag, El, Key, KeyEvent, Phase, Tx, col, el, row, text};

use crate::app::State;
use crate::ui::{BODY, BODY_MED, LABEL, SMALL, SMALL_NUM, Ui, switch_green};

use super::{fill, tr};

/// The whole control's height; the track line is centred at `LINE_Y`.
const H: f32 = 48.0;
const LINE_Y: f32 = 14.0;
const TRACK_H: f32 = 4.0;
const STOP: f32 = 8.0;
const THUMB_W: f32 = 12.0;
const THUMB_H: f32 = 20.0;
/// Room for a stop's label; the track box is padded half of it each side so end labels fit.
const LABEL_W: f32 = 80.0;
const LABEL_Y: f32 = 28.0;
/// The track's width bounds; it takes what the row leaves between them.
const TRACK_MIN: f32 = 240.0;
const TRACK_MAX: f32 = 360.0;

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

/// `on` is the tier of the applied config, its stop green like the toggles.
pub fn view(s: &State, u: &Ui, catalog: &Catalog, on: Option<Tier>, note: String) -> El {
    let tiers = tiers(catalog);
    let n = tiers.len();
    let order: Vec<Tier> = tiers.iter().map(|(t, _)| *t).collect();
    let index = |t: Option<Tier>| t.and_then(|t| order.iter().position(|x| *x == t));
    // What the thumb shows: the tier being dragged to, else the filter.
    let shown = s.performance.drag_tier.or(s.performance.tier);
    let picked = index(shown);
    // With every tier shown the thumb rests, as an outline, on the applied or recommended tier.
    let rest = index(on).or_else(|| {
        let tier = catalog.configs.iter().find(|c| c.id() == catalog::DEFAULT_ID)?.def.tier;
        index(Some(tier))
    });
    let thumb_at = picked.or(rest);

    // The track: one cell per stop across the whole width, the thumb laid over the cells.
    let mut cells = row().w_full().h(THUMB_H).a11y_hidden();
    let mut labels = row().w_full().h(16.0).a11y_hidden();
    for (i, (tier, _)) in tiers.iter().enumerate() {
        let lit_left = picked.is_some_and(|p| i <= p && i > 0);
        let lit_right = picked.is_some_and(|p| i < p);
        let filled = picked.is_some_and(|p| i < p);
        cells = cells.child(stop_cell(u, i, n, lit_left, lit_right, on == Some(*tier), filled));
        labels = labels.child(
            row().w(0.0).grow(1.0).h(16.0).center().child(
                text(tr(tier.label()).to_string())
                    .font(if picked == Some(i) { LABEL } else { SMALL })
                    .color(if picked == Some(i) { u.ink } else { u.muted })
                    .truncate(),
            ),
        );
    }
    let mut cell_col = col()
        .w_full()
        .child(el().h(LINE_Y - THUMB_H / 2.0))
        .child(cells)
        .child(el().h(LABEL_Y - LINE_Y - THUMB_H / 2.0))
        .child(labels);
    // The thumb: a stop's cell is `1/n` of the width, its centre at `(i + 0.5)/n`. Drawn as an
    // overlay row of the same cells so it lands on the stop without knowing the width.
    if let Some(at) = thumb_at {
        let solid = picked.is_some();
        let mut overlay = row().absolute(0.0, LINE_Y - THUMB_H / 2.0).w_full().h(THUMB_H).a11y_hidden();
        for i in 0..n {
            let mut c = row().w(0.0).grow(1.0).h(THUMB_H).center();
            if i == at {
                c = c.child(
                    el().size(THUMB_W, THUMB_H)
                        .radius(2.0)
                        .fill(if solid { u.ink } else { u.bg })
                        .border(1.0, if solid { u.bg } else { u.muted }),
                );
            }
            overlay = overlay.child(c);
        }
        cell_col = cell_col.child(overlay);
    }

    let drag_order = order.clone();
    let key_order = order.clone();
    let track = row()
        .key("perf-scale")
        .w(0.0)
        .grow(1.0)
        .min_w(TRACK_MIN + LABEL_W - STOP)
        .max_w(TRACK_MAX + LABEL_W)
        .h(H)
        .focusable(true)
        .role("slider")
        .label(tr("Looks to frames").to_string())
        .a11y_value(match picked {
            Some(i) => tr(tiers[i].0.label()).to_string(),
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
            let cell = d.w.max(1.0) / n as f32;
            let i = nearest(d.x - cell / 2.0, d.w - cell, n);
            if d.cancelled() {
                st.performance.drag_tier = None;
                return;
            }
            match d.phase {
                Phase::Up => {
                    st.performance.tier = Some(drag_order[i]);
                    st.performance.drag_tier = None;
                }
                _ => st.performance.drag_tier = Some(drag_order[i]),
            }
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            let len = key_order.len();
            if len == 0 {
                return false;
            }
            let now = st.performance.tier.and_then(|t| key_order.iter().position(|x| *x == t));
            let next = match ev.key {
                Key::Left | Key::Down => now.map_or(0, |i| i.saturating_sub(1)),
                Key::Right | Key::Up => now.map_or(0, |i| (i + 1).min(len - 1)),
                Key::Home => 0,
                Key::End => len - 1,
                Key::Escape if now.is_some() => {
                    st.performance.tier = None;
                    return true;
                }
                _ => return false,
            };
            st.performance.tier = Some(key_order[next]);
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

    let total = catalog.configs.len();
    let all_on = shown.is_none();
    let all = row()
        .key("perf-tier-all")
        .w(64.0)
        .h(28.0)
        .mt(LINE_Y - 14.0)
        .gap(6.0)
        .center()
        .radius(crate::ui::r(4.0))
        .border(1.0, if all_on { u.hair_strong } else { u.hair })
        .fill(if all_on { u.selected } else { rune::Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .checked(all_on)
        .label(tr("All tiers").to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.performance.tier = None)
        .child(
            text(tr("All").to_string())
                .font(BODY)
                .color(if all_on { u.ink } else { u.muted })
                .a11y_hidden(),
        );

    // The readout: what the thumb points at, in a box the text never sizes.
    // Line 2 says how fresh the configs are; the tier's blurb already heads its group below.
    let (name, count) = match picked {
        Some(i) => (tr(tiers[i].0.label()).to_string(), tiers[i].1),
        None => (tr("All tiers").to_string(), total),
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
        .child(
            row().w_full().h(16.0).child(text(note).font(SMALL).color(u.muted).truncate()),
        );

    row()
        .w_full()
        .min_w(0.0)
        .h(H)
        .child(all)
        .child(el().w(12.0))
        .child(end("Looks", 44.0, true))
        .child(el().w(8.0))
        .child(track)
        .child(el().w(8.0))
        .child(end("Frames", 48.0, false))
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
