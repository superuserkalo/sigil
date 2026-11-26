//! The one segmented selector: Browse's sections, Installed's scope, Crosshair's groups and
//! every other strip where one part of a few is picked. Every screen builds it here, so the
//! track, padding, radius, height, type, count placement, hover, focus and the sliding
//! selection box are the same everywhere (`segmented_tests` keeps screens from building their
//! own).
//!
//! The selection box is one child of the track, drawn before the parts so the labels always
//! paint over it, and it slides to a new pick on `motion::base()` by layout alone. It is never
//! a matched flight: Rune paints a flight above the whole window, which hid the labels under
//! the box for the length of the glide. Labels and counts keep their keys whichever part is
//! picked; only their colour changes.

use rune::{Color, El, Icon, el, icon, row, text};

use super::{CONTROL_L, CONTROL_M, LABEL, SMALL_NUM, Ui, r};

/// The track's inset round the parts, and the gap between them.
const PAD: f32 = 2.0;
const GAP: f32 = 2.0;
/// A part's padding each side of its content.
const PART_PX: f32 = 10.0;
/// The least padding of a part in equal columns, where the column's width centres the label
/// and a narrow menu row must still hold "Medium".
const EVEN_PX: f32 = 6.0;
/// Between a label and its count.
const COUNT_GAP: f32 = 6.0;
/// A count keeps room for this many digits at least, so 0 to 99 never moves the strip.
const COUNT_DIGITS: usize = 2;
/// An icon part's glyph.
const GLYPH: f32 = 14.0;

/// The track's height: 32 on a page toolbar, 28 in a settings row, a menu or a panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    L,
    M,
}

impl Size {
    fn track(self) -> f32 {
        match self {
            Size::L => CONTROL_L,
            Size::M => CONTROL_M,
        }
    }

    fn part(self) -> f32 {
        self.track() - 2.0 * PAD
    }
}

/// How wide the strip is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Width {
    /// Each part as wide as its content needs, measured, so nothing clips at any count.
    Natural,
    /// Equal parts sharing this width.
    Even(f32),
    /// Equal parts sharing the width the parent gives.
    Fill,
}

/// One part of the strip.
pub struct Part {
    key: String,
    label: String,
    count: Option<String>,
    glyph: Option<&'static Icon>,
    name: Option<String>,
    on: bool,
    mark: Option<Color>,
    attach: Option<Box<dyn FnOnce(El) -> El>>,
}

impl Part {
    /// A part keyed `key` showing `label`.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Part {
        Part { key: key.into(), label: label.into(), count: None, glyph: None, name: None, on: false, mark: None, attach: None }
    }

    /// The count after the label, in tabular figures, grouped by thousands (1,234).
    pub fn count(mut self, n: usize) -> Part {
        self.count = Some(grouped(n));
        self
    }

    /// An icon in place of the label, which stays the part's name and its tooltip.
    pub fn glyph(mut self, glyph: &'static Icon) -> Part {
        self.glyph = Some(glyph);
        self
    }

    /// A fuller name for assistive tech and the tooltip than the short label on screen.
    pub fn name(mut self, name: impl Into<String>) -> Part {
        self.name = Some(name.into());
        self
    }

    /// This part is the picked one.
    pub fn on(mut self, on: bool) -> Part {
        self.on = on;
        self
    }

    /// A 6 point square in the part's left padding, as a state mark ("a match is running").
    /// It sits in the padding, so it never widens the part.
    pub fn mark(mut self, color: Color) -> Part {
        self.mark = Some(color);
        self
    }

    /// The part's handlers (`|e| e.on_press(..)`). Only handlers: the look is the strip's.
    pub fn with(mut self, attach: impl FnOnce(El) -> El + 'static) -> Part {
        self.attach = Some(Box::new(attach));
        self
    }
}

/// `1234` as `1,234`.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A segmented selector keyed `key` and named `name` for assistive tech.
pub struct Segmented {
    key: String,
    name: String,
    size: Size,
    width: Width,
    role: (&'static str, &'static str),
    parts: Vec<Part>,
}

impl Segmented {
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Segmented {
        Segmented { key: key.into(), name: name.into(), size: Size::L, width: Width::Natural, role: ("radiogroup", "radio"), parts: Vec::new() }
    }

    pub fn size(mut self, size: Size) -> Segmented {
        self.size = size;
        self
    }

    pub fn width(mut self, width: Width) -> Segmented {
        self.width = width;
        self
    }

    /// The parts switch views (a tab list) rather than set a value.
    pub fn tabs(mut self) -> Segmented {
        self.role = ("tablist", "tab");
        self
    }

    pub fn part(mut self, part: Part) -> Segmented {
        self.parts.push(part);
        self
    }

    pub fn parts(mut self, parts: impl IntoIterator<Item = Part>) -> Segmented {
        self.parts.extend(parts);
        self
    }

    /// One part per label, keyed `{key}-{index}`, the `selected` one picked; a press calls
    /// `pick` with the part's index.
    pub fn choices<S: AsRef<str>>(mut self, labels: &[S], selected: usize, pick: fn(&mut crate::app::State, usize)) -> Segmented {
        for (index, label) in labels.iter().enumerate() {
            let part = Part::new(format!("{}-{index}", self.key), label.as_ref())
                .on(index == selected)
                .with(move |e| e.on_press(move |st: &mut crate::app::State| pick(st, index)));
            self.parts.push(part);
        }
        self
    }

    /// The strip's width at `Width::Natural`, for a toolbar that plans its line before it
    /// builds.
    pub fn natural_width(&self) -> f32 {
        let widths: Vec<f32> = self.parts.iter().map(|p| part_width(p, self.size)).collect();
        track_width(&widths)
    }

    pub fn build(self, u: &Ui) -> El {
        let Segmented { key, name, size, width, role, parts } = self;
        let n = parts.len().max(1);
        let selected = parts.iter().position(|p| p.on);
        let part_h = size.part();
        let mut track = row()
            .key(key.clone())
            .h(size.track())
            .padding(PAD)
            .gap(GAP)
            .center_y()
            .radius(r(4.0))
            .fill(u.well)
            .border(1.0, u.hair)
            .role(role.0)
            .label(name);
        let widths: Vec<f32> = parts.iter().map(|p| part_width(p, size)).collect();
        // The box first, so every label paints over it.
        let thumb = |e: El| {
            crate::motion::slides(
                e.key(format!("{key}-thumb"))
                    .radius(r(8.0))
                    .fill(u.over.alpha(0.075))
                    .border(1.0, u.hair_strong)
                    .a11y_hidden(),
                crate::motion::base(),
            )
        };
        match width {
            Width::Natural => {
                track = track.w(track_width(&widths));
                if let Some(at) = selected {
                    let left = PAD + widths[..at].iter().map(|w| w + GAP).sum::<f32>();
                    track = track.child(thumb(el().absolute(left, PAD).size(widths[at], part_h)));
                }
            }
            Width::Even(_) | Width::Fill => {
                // Equal columns as wide as the widest part needs at least, so a label never
                // cuts in a narrow menu row.
                let widest = widths.iter().fold(0.0f32, |m, w| m.max(*w)) - 2.0 * (PART_PX - EVEN_PX);
                track = track.equal_columns(n).min_w(track_width(&vec![widest; n]));
                track = match width {
                    Width::Even(w) => track.w(w),
                    _ => track.w_full(),
                };
                if let Some(at) = selected {
                    track = track.child(thumb(el().grid_cell(at).h(part_h)));
                }
            }
        }
        for (index, (part, w)) in parts.into_iter().zip(widths).enumerate() {
            let px = if width == Width::Natural { PART_PX } else { EVEN_PX };
            let mut cell = part_el(u, &part, part_h, px);
            cell = match width {
                Width::Natural => cell.w(w),
                _ => cell.grid_cell(index).min_w(0.0),
            };
            cell = cell.role(role.1);
            if let Some(attach) = part.attach {
                cell = attach(cell);
            }
            track = track.child(cell);
        }
        track
    }
}

fn label_w(text: &str) -> f32 {
    width_of(text, LABEL, false)
}

/// The width a count keeps: its own, or `COUNT_DIGITS` figures when that is more.
fn count_w(count: &str) -> f32 {
    let own = width_of(count, SMALL_NUM, true);
    let least = width_of(&"0".repeat(COUNT_DIGITS), SMALL_NUM, true);
    own.max(least)
}

/// Widths measured while a view ran, by text, whether it was a count and the font epoch (a font
/// or family change measures again): a selector's natural
/// width is also asked for from handlers and polls (Browse's toolbar tier), where a measure
/// would build a second text system for the thread (every font and fallback again). Bounded:
/// cleared once it holds `WIDTHS_KEPT`.
const WIDTHS_KEPT: usize = 512;

thread_local! {
    static WIDTHS: std::cell::RefCell<std::collections::HashMap<(String, bool, u64), f32>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `text`'s width in `style`: measured with the window's text system while a view runs and kept;
/// outside a view, what a view measured, else an estimate (a view measures it for real the next
/// time it builds the control). Tests measure directly.
fn width_of(text: &str, style: rune::TextStyle, count: bool) -> f32 {
    let key = (text.to_string(), count, rune::text::font_epoch());
    if let Some(w) = WIDTHS.with_borrow(|m| m.get(&key).copied()) {
        return w;
    }
    if !rune::text::in_view() && !cfg!(test) {
        return text.chars().count() as f32 * style.size * 0.6;
    }
    let w = rune::text::measure(text, &[], style, None).0;
    WIDTHS.with_borrow_mut(|m| {
        if m.len() >= WIDTHS_KEPT {
            m.clear();
        }
        m.insert(key, w);
    });
    w
}

fn part_width(part: &Part, size: Size) -> f32 {
    if part.glyph.is_some() {
        return size.part();
    }
    let mut w = label_w(&part.label) + 2.0 * PART_PX;
    if let Some(count) = &part.count {
        w += COUNT_GAP + count_w(count);
    }
    w.ceil()
}

fn track_width(widths: &[f32]) -> f32 {
    2.0 * PAD + widths.iter().sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32
}

/// One part: the label (and count) centred, the same padding on both sides. Its children keep
/// their keys whichever part is picked, and only their colour eases.
fn part_el(u: &Ui, part: &Part, h: f32, px: f32) -> El {
    let on = part.on;
    let base = part.name.clone().unwrap_or_else(|| part.label.clone());
    let name = match &part.count {
        Some(count) => format!("{base} {count}"),
        None => base,
    };
    let mut cell = row()
        .key(part.key.clone())
        .h(h)
        .px(if part.glyph.is_some() { 0.0 } else { px })
        .gap(COUNT_GAP)
        .center()
        .radius(r(8.0))
        .hover(if on { Color::TRANSPARENT } else { u.hover })
        .press(u.over.alpha(0.08))
        .focusable(true)
        // A press picks without taking focus, so it costs one commit, not a focus commit
        // and then the pick; Tab and the arrow keys still reach every part.
        .keeps_text_selection()
        .checked(on)
        .label(name)
        .cursor(rune::Cursor::Pointer);
    if let Some(full) = &part.name {
        cell = cell.help(full.clone());
    }
    if let Some(glyph) = part.glyph {
        return cell
            .help(part.label.clone())
            .child(crate::motion::colors(
                icon(glyph, GLYPH).key(format!("{}-glyph", part.key)).color(if on { u.ink } else { u.muted }).a11y_hidden(),
            ));
    }
    if let Some(color) = part.mark {
        cell = cell.child(el().absolute(2.0, (h - 6.0) / 2.0).size(6.0, 6.0).radius(1.0).fill(color).a11y_hidden());
    }
    cell = cell.child(crate::motion::colors(
        text(part.label.clone())
            .key(format!("{}-label", part.key))
            .font(LABEL)
            .color(if on { u.ink } else { u.muted })
            .truncate()
            .min_w(0.0)
            .a11y_hidden(),
    ));
    if let Some(count) = &part.count {
        cell = cell.child(crate::motion::colors(
            text(count.clone())
                .key(format!("{}-count", part.key))
                .font(SMALL_NUM)
                .color(if on { u.prose } else { u.muted })
                .a11y_hidden(),
        ));
    }
    cell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_group_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1234), "1,234");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn a_counted_part_holds_its_label_and_count_inside_even_padding() {
        for n in [0, 9, 99, 1234] {
            let part = Part::new("p", "Disabled").count(n);
            let w = part_width(&part, Size::L);
            let need = label_w("Disabled") + COUNT_GAP + rune::text::measure(&grouped(n), &[], SMALL_NUM, None).0;
            assert!(w >= need + 2.0 * PART_PX - 0.01, "{n}: {w} < {need} plus padding");
        }
        // Within the reserved digits the width never moves.
        let w = |n| part_width(&Part::new("p", "All").count(n), Size::L);
        assert_eq!(w(0), w(9));
        assert_eq!(w(9), w(99));
    }

    /// Every source file under `src`, as (path from `src`, text), tests left out.
    fn sources() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut out = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "tests.rs" {
                    continue;
                }
                let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&path).unwrap_or_default()));
            }
        }
        out
    }

    /// There is one segmented selector. A screen that builds its own strip (a track of picks
    /// on the well fill, a sliding thumb or indicator of its own, a matched selection flight, or
    /// a grid of tab chips keyed `-tabs`) fails here: build it with `ui::Segmented` instead. The
    /// Settings section list is a vertical navigation list, not a segmented selector: its box
    /// slides by layout under the labels as `Segmented`'s does.
    #[test]
    fn screens_build_no_segmented_selector_of_their_own() {
        let mut wrong = Vec::new();
        for (path, text) in sources() {
            if path == "ui/segmented.rs" {
                continue;
            }
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            // Split into functions, roughly: each `fn ` starts a new body.
            for body in code.split("fn ").skip(1) {
                let name: String = body.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                // A selection that flies as a matched element paints over the labels it slides
                // under (the old `seg_thumb`).
                let thumb = body.contains("seg_thumb(") || (body.contains(".matched(") && body.contains("-thumb"));
                let track = body.contains(".fill(u.well)") && (body.contains(".role(\"radiogroup\")") || body.contains(".role(\"tablist\")"));
                let indicator = body.contains(".equal_columns(") && body.contains("-indicator");
                // Tab chips laid out in equal columns (the Discord lines dialog's phases were).
                let chips = body.contains(".equal_columns(") && body.contains("-tabs\")");
                if thumb || track || indicator || chips {
                    wrong.push(format!("{path}: fn {name}"));
                }
            }
        }
        assert!(wrong.is_empty(), "segmented selectors built outside ui::Segmented:\n{}", wrong.join("\n"));
    }
}
