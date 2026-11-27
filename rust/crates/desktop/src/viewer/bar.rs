//! The play bar's layout: which controls fit the width of the view the bar sits in, in which
//! rows, and how wide the scrubber is. Pure arithmetic on the controls' fixed widths, so the
//! strip (`super::strip`) builds exactly what this plans and a test can lay it out at any width.
//!
//! - Wide: one row spanning the view (less the side margins, up to `MAX_W`): play, the
//!   scrubber taking the spare room, the clock, the speed, the clip, then the compare buttons
//!   and the tools.
//! - Narrower: two rows in the one bar, the playback row on top and the compare buttons and
//!   tools under it. When the playback row runs short, the clip name goes first (the Controls
//!   panel has the animation picker), then the speed (the panel has it too), then the clock.
//!   Play and the scrubber always stay.
//! - Narrower still: the compare buttons and the tools take a row each, and the tools wrap.

/// The strip layer's padding: the bar's margin from the view's edges.
pub const SIDE: f32 = 12.0;
/// The widest the bar gets, however wide the view.
pub const MAX_W: f32 = 1120.0;
/// The scrubber never gets narrower than this.
pub const SEEK_MIN: f32 = 96.0;
/// The scrubber shares its row with the tools only while it keeps at least this much.
pub const SEEK_ONE_ROW: f32 = 200.0;
/// A square icon button.
pub const BUTTON: f32 = 30.0;
/// Between two controls in a row.
pub const GAP: f32 = 2.0;
/// Between two rows.
pub const ROW_GAP: f32 = 4.0;
/// The bar's padding and border on each side.
pub const CHROME: f32 = 7.0;
/// The thin rule between groups, with its margins.
pub const RULE: f32 = 9.0;
/// The scrubber's own side padding.
pub const SEEK_PAD: f32 = 6.0;
/// The clock ("0:01.0 / 0:02.5").
pub const TIME_W: f32 = 92.0;
/// The speed button ("0.25x"), fixed so the scrubber does not jump as it cycles.
pub const SPEED_W: f32 = 50.0;
/// The clip button; a longer name is cut short.
pub const CLIP_W: f32 = 132.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Play,
    Seek,
    Time,
    Speed,
    Clip,
    Rule,
    /// A compare button, by index.
    Compare(usize),
    /// A tool button, by index.
    Tool(usize),
}

/// What the strip has to show.
#[derive(Clone, Debug, Default)]
pub struct Need {
    /// The model has an animation: play, scrubber, clock and speed.
    pub animated: bool,
    /// More than one clip: the clip button.
    pub clips: bool,
    /// The compare buttons' widths (none without a before side).
    pub compare: Vec<f32>,
    /// How many tool buttons.
    pub tools: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The bar's outer width.
    pub width: f32,
    /// The scrubber's track width (0 when not animated).
    pub seek: f32,
    pub rows: Vec<Vec<Item>>,
}

impl Plan {
    pub fn has(&self, item: Item) -> bool {
        self.rows.iter().any(|r| r.contains(&item))
    }
}

/// The width `item` takes, with the scrubber at `seek`.
pub fn item_w(need: &Need, item: Item, seek: f32) -> f32 {
    match item {
        Item::Play | Item::Tool(_) => BUTTON,
        Item::Seek => seek + 2.0 * SEEK_PAD,
        Item::Time => TIME_W,
        Item::Speed => SPEED_W,
        Item::Clip => CLIP_W,
        Item::Rule => RULE,
        Item::Compare(i) => need.compare.get(i).copied().unwrap_or(0.0),
    }
}

/// The outer width of a row of `items`.
pub fn row_w(need: &Need, items: &[Item], seek: f32) -> f32 {
    let sum: f32 = items.iter().map(|i| item_w(need, *i, seek)).sum();
    sum + GAP * items.len().saturating_sub(1) as f32 + 2.0 * CHROME
}

/// The scrubber's width when `items` (with `Item::Seek` among them) fill `width`.
fn seek_in(need: &Need, items: &[Item], width: f32) -> f32 {
    width - row_w(need, items, 0.0)
}

/// Packs `groups` into rows no wider than `width`: whole groups side by side (a rule between)
/// while they fit, a group of its own split greedily when it alone does not.
fn pack(need: &Need, groups: &[Vec<Item>], width: f32) -> Vec<Vec<Item>> {
    let mut rows: Vec<Vec<Item>> = Vec::new();
    for group in groups.iter().filter(|g| !g.is_empty()) {
        if let Some(last) = rows.last_mut() {
            let mut joined = last.clone();
            joined.push(Item::Rule);
            joined.extend(group);
            if row_w(need, &joined, 0.0) <= width {
                *last = joined;
                continue;
            }
        }
        let mut row: Vec<Item> = Vec::new();
        for item in group {
            let mut next = row.clone();
            next.push(*item);
            if !row.is_empty() && row_w(need, &next, 0.0) > width {
                rows.push(std::mem::take(&mut row));
                row.push(*item);
            } else {
                row = next;
            }
        }
        rows.push(row);
    }
    rows
}

/// The bar for a view `view_w` points wide.
pub fn plan(need: &Need, view_w: f32) -> Plan {
    let avail = (view_w - 2.0 * SIDE).max(0.0).min(MAX_W);
    let compare: Vec<Item> = (0..need.compare.len()).map(Item::Compare).collect();
    let tools: Vec<Item> = (0..need.tools).map(Item::Tool).collect();
    if !need.animated {
        // Nothing grows: the bar hugs its buttons.
        let rows = pack(need, &[compare, tools], avail);
        let width = rows.iter().map(|r| row_w(need, r, 0.0)).fold(0.0, f32::max);
        return Plan { width, seek: 0.0, rows };
    }
    let mut playback = vec![Item::Play, Item::Seek, Item::Time, Item::Speed];
    if need.clips {
        playback.push(Item::Clip);
    }
    // One row, everything in it.
    let mut one = playback.clone();
    for group in [&compare, &tools] {
        if !group.is_empty() {
            one.push(Item::Rule);
            one.extend(group.iter());
        }
    }
    let seek = seek_in(need, &one, avail);
    if seek >= SEEK_ONE_ROW {
        return Plan { width: avail, seek, rows: vec![one] };
    }
    // The playback row on top, dropping the clip, the speed, then the clock until the scrubber
    // keeps its minimum.
    for drop in [Item::Clip, Item::Speed, Item::Time] {
        if seek_in(need, &playback, avail) >= SEEK_MIN {
            break;
        }
        playback.retain(|i| *i != drop);
    }
    let seek = seek_in(need, &playback, avail).max(SEEK_MIN.min(avail));
    let mut rows = vec![playback];
    rows.extend(pack(need, &[compare, tools], avail));
    Plan { width: avail, seek, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(compare: bool) -> Need {
        // Measured like the strip measures them: "With mods", "Without mods", "Split".
        let compare = if compare { vec![74.0, 92.0, 46.0] } else { Vec::new() };
        Need { animated: true, clips: true, compare, tools: 7 }
    }

    /// No control leaves the bar, the bar stays inside the view, and the scrubber keeps its
    /// minimum, from a phone-narrow view to a 4K one.
    #[test]
    fn the_bar_fits_its_view_at_every_width() {
        for compare in [false, true] {
            let n = need(compare);
            for view_w in [320.0, 420.0, 560.0, 720.0, 900.0, 1100.0, 1360.0, 1920.0, 3840.0] {
                let p = plan(&n, view_w);
                assert!(p.width <= view_w - 2.0 * SIDE + 0.01, "{view_w}: bar {} wider than the view", p.width);
                assert!(p.width <= MAX_W);
                assert!(p.seek >= SEEK_MIN, "{view_w}: scrubber {} under its minimum", p.seek);
                assert!(p.has(Item::Play) && p.has(Item::Seek), "{view_w}: play and the scrubber always stay");
                for row in &p.rows {
                    let w = row_w(&n, row, if row.contains(&Item::Seek) { p.seek } else { 0.0 });
                    assert!(w <= p.width + 0.01, "{view_w}: row {row:?} is {w}, the bar {}", p.width);
                }
                for i in 0..n.tools {
                    assert!(p.has(Item::Tool(i)), "{view_w}: tool {i} lost");
                }
                for i in 0..n.compare.len() {
                    assert!(p.has(Item::Compare(i)), "{view_w}: compare {i} lost");
                }
            }
        }
    }

    #[test]
    fn narrow_medium_and_wide_views() {
        let n = need(true);
        // Wide: one row spanning the view up to the maximum, the scrubber taking the room.
        let wide = plan(&n, 1920.0);
        assert_eq!((wide.rows.len(), wide.width), (1, MAX_W));
        assert!(wide.seek >= SEEK_ONE_ROW);
        // Medium: two rows, everything kept.
        let medium = plan(&n, 900.0);
        assert_eq!(medium.rows.len(), 2);
        assert!([Item::Time, Item::Speed, Item::Clip].iter().all(|i| medium.has(*i)));
        // Narrow: the clip goes first, then the speed, then the clock; play and the scrubber stay.
        let narrow = plan(&n, 420.0);
        assert!(!narrow.has(Item::Clip) && narrow.has(Item::Speed));
        let narrower = plan(&n, 320.0);
        assert!(!narrower.has(Item::Speed) && narrower.has(Item::Time));
        let narrowest = plan(&n, 240.0);
        assert!(!narrowest.has(Item::Time));
        for p in [narrow, narrower, narrowest] {
            assert_eq!(p.rows[0][..2], [Item::Play, Item::Seek]);
        }
    }

    #[test]
    fn a_still_model_hugs_its_buttons() {
        let n = Need { animated: false, clips: false, compare: Vec::new(), tools: 7 };
        let p = plan(&n, 1360.0);
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.width, row_w(&n, &p.rows[0], 0.0));
    }
}
