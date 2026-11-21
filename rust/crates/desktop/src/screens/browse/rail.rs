//! Drag to scroll the landing rails, and a scroll bar under them that drags too (DMM 2.4.0
//! `2d8ba44`). Rune draws no bar on a horizontal viewport and a mouse wheel only moves one with
//! Shift held, so a mouse user grabs the rail itself or its bar.
//!
//! The rail reports where it is scrolled through `on_scroll_offset` into live values, so the
//! thumb follows the scroll in the same frame without a view run. A press that dragged the
//! rail opens nothing when it ends.

use rune::{Live, ScrollPos};

use super::*;

/// Points the pointer travels before a press on a tile becomes a drag.
const DRAG_SLOP: f32 = 5.0;
/// The bar's hit height and the line drawn inside it.
const BAR_H: f32 = 12.0;
const LINE_H: f32 = 3.0;

pub struct Rail {
    /// `[offset, max, viewport, content]` of the hero rail, along x.
    pub metrics: Live<[f32; 4]>,
    /// Where the bar's thumb sits, in points from the track's left.
    pub thumb: Live<[f32; 2]>,
    /// A drag just scrolled the rail: the press that ends it opens nothing.
    pub dragged: bool,
}

impl Default for Rail {
    fn default() -> Self {
        Rail {
            metrics: Live::new([0.0; 4]),
            thumb: Live::new([0.0; 2]),
            dragged: false,
        }
    }
}

/// The thumb's share of the track: the part of the rail on screen, never under a tenth.
fn thumb_ratio(viewport: f32, content: f32) -> f32 {
    if content <= 0.0 {
        return 1.0;
    }
    (viewport / content).clamp(0.1, 1.0)
}

/// Where the thumb sits for a scroll position, in points from the track's left.
fn thumb_x(offset: f32, max: f32, viewport: f32) -> f32 {
    if max <= 0.0 {
        return 0.0;
    }
    let travel = viewport * (1.0 - thumb_ratio(viewport, viewport + max));
    (offset / max).clamp(0.0, 1.0) * travel
}

/// The rail offset a press at `x` on a track `width` wide asks for: the thumb centred on it.
fn bar_offset(x: f32, width: f32, ratio: f32, max: f32) -> f32 {
    let thumb = width * ratio;
    let travel = (width - thumb).max(1.0);
    ((x - thumb / 2.0) / travel).clamp(0.0, 1.0) * max.max(0.0)
}

/// A horizontal strip keyed `key` that reports its scroll and scrolls under a drag.
pub fn strip(s: &State, strip: El, key: &'static str) -> El {
    let metrics = s.browse_view.tree.rail.metrics.clone();
    let thumb = s.browse_view.tree.rail.thumb.clone();
    strip
        .on_scroll_offset(move |at: ScrollPos| {
            let [offset, max, viewport] = [at.offset[0], at.max[0], at.viewport[0]];
            metrics.set([offset, max, viewport, viewport + max]);
            thumb.set([thumb_x(offset, max, viewport), 0.0]);
        })
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| drag(st, d, tx, key))
}

/// A tile inside the strip: a drag that starts on it scrolls the strip too. Its press handler
/// calls [`took_drag`] first.
pub fn tile(tile: El, key: &'static str) -> El {
    tile.on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| drag(st, d, tx, key))
}

fn drag(st: &mut State, d: &mut Drag, tx: &mut Tx, key: &str) {
    let rail = &mut st.browse_view.tree.rail;
    match d.phase {
        Phase::Down => {
            d.data = rail.metrics.get()[0];
            rail.dragged = false;
        }
        Phase::Move => {
            if rail.dragged || d.dx.abs() >= DRAG_SLOP {
                rail.dragged = true;
                let max = rail.metrics.get()[1].max(0.0);
                tx.scroll_to_instant(key, (d.data - d.dx).clamp(0.0, max), 0.0);
            }
        }
        Phase::Up => {
            if d.cancelled() {
                tx.scroll_to_instant(key, d.data, 0.0);
            }
        }
    }
}

/// True, once, when the press that just ended was a drag of the rail.
pub fn took_drag(st: &mut State) -> bool {
    std::mem::take(&mut st.browse_view.tree.rail.dragged)
}

/// The bar under a rail: a thin track whose thumb follows the scroll. A press or drag on it
/// moves the rail there. Hidden once the rail is known to fit.
pub fn bar(s: &State, u: &Ui, key: &'static str, label: &str) -> El {
    let rail = &s.browse_view.tree.rail;
    let [_, max, viewport, content] = rail.metrics.get();
    if content > 0.0 && max < 1.0 {
        return el().h(0.0);
    }
    let ratio = if content > 0.0 { thumb_ratio(viewport, content) } else { 0.3 };
    el().key(format!("{key}-bar"))
        .w_full()
        .h(BAR_H)
        .role("scrollbar")
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| {
            if d.phase == Phase::Up {
                return;
            }
            let [_, max, viewport, content] = st.browse_view.tree.rail.metrics.get();
            let offset = bar_offset(d.x, d.w, thumb_ratio(viewport, content), max);
            tx.scroll_to_instant(key, offset, 0.0);
        })
        .child(
            el().absolute(0.0, (BAR_H - LINE_H) / 2.0)
                .w_full()
                .h(LINE_H)
                .radius(crate::ui::r(LINE_H / 2.0))
                .fill(u.over.alpha(0.06))
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, (BAR_H - LINE_H) / 2.0)
                .w_frac(ratio)
                .h(LINE_H)
                .radius(crate::ui::r(LINE_H / 2.0))
                .fill(u.accent.alpha(0.55))
                .live_offset(&rail.thumb)
                .a11y_hidden(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_tracks_the_scroll() {
        // 400 points on screen of 1600: a quarter-width thumb travelling 300 points.
        assert_eq!(thumb_ratio(400.0, 1600.0), 0.25);
        assert_eq!(thumb_x(0.0, 1200.0, 400.0), 0.0);
        assert_eq!(thumb_x(600.0, 1200.0, 400.0), 150.0);
        assert_eq!(thumb_x(1200.0, 1200.0, 400.0), 300.0);
        assert_eq!(thumb_x(50.0, 0.0, 400.0), 0.0, "a rail that fits never moves its thumb");
        assert_eq!(thumb_ratio(10.0, 10_000.0), 0.1, "a long rail keeps a thumb to grab");
    }

    #[test]
    fn a_press_on_the_bar_centres_the_thumb_there() {
        assert_eq!(bar_offset(0.0, 400.0, 0.25, 1200.0), 0.0);
        assert_eq!(bar_offset(200.0, 400.0, 0.25, 1200.0), 600.0);
        assert_eq!(bar_offset(400.0, 400.0, 0.25, 1200.0), 1200.0);
    }
}
