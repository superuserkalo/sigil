//! Design tokens for Sigil: one palette, one type scale, one set of flat primitives.
//!
//! The look is restraint: near black layers, hairlines instead of boxes, one ember accent that
//! only marks a primary action, an on state or focus. Every size sits on a 4 point grid.

use rune::{Color, El, Family, Icon, TextStyle, col, el, icon, oklch, row, rule, spacer, text};

use crate::model::{Compat, ToneKey};

pub mod personalize;

pub const SIDEBAR_W: f32 = 216.0;
pub const RAIL_W: f32 = 304.0;
pub const TITLEBAR_H: f32 = 32.0;
pub const HEADER_H: f32 = 40.0;
pub const STATUS_H: f32 = 0.0;
pub const PAGE_X: f32 = 24.0;
pub const PAGE_TOP: f32 = 12.0;

// The type scale. Rune bundles three Inter faces (Regular 400, Medium 500, SemiBold 600) and
// matches weights the CSS way, so a weight between faces draws in the next heavier one: write
// only 400, 500 and 600. 13 is body, 12 the floor for words, 11 only for badges and keycaps.
// Emphasis is one step (400 to 500); 600 is for the page title, section headers and stat values.
/// The one title a page shows, on its toolbar line; also dialog and empty-state titles.
pub const PAGE_TITLE: TextStyle = TextStyle::new(16.0, 600.0).lh(24.0).tracking(-0.1);
pub const HERO: TextStyle = PAGE_TITLE;
pub const DISPLAY: TextStyle = PAGE_TITLE;
pub const H1: TextStyle = PAGE_TITLE;
/// A section header inside a page: the only 600 weight in content, in `muted`.
pub const SECTION: TextStyle = TextStyle::new(12.0, 600.0).lh(16.0).tracking(0.1);
pub const H2: TextStyle = SECTION;
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(20.0);
pub const LEAD: TextStyle = BODY;
pub const CHAT: TextStyle = TextStyle::new(14.0, 400.0).lh(22.0);
pub const BODY_MED: TextStyle = TextStyle::new(13.0, 500.0).lh(20.0);
/// Body numbers whose digits line up: values in rows and tables.
pub const BODY_NUM: TextStyle = TextStyle::new(13.0, 400.0).lh(20.0).features("'tnum' 1");
pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
/// Small text whose digits line up: counts, sizes, versions.
pub const SMALL_NUM: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0).features("'tnum' 1");
pub const LABEL: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
pub const NAV: TextStyle = TextStyle::new(13.0, 500.0).lh(20.0);
pub const WORDMARK: TextStyle = TextStyle::new(15.0, 600.0).lh(20.0).tracking(-0.2);
/// Badges, keycaps and slot numbers only: at most four characters.
pub const MICRO: TextStyle = TextStyle::new(11.0, 500.0)
    .lh(16.0)
    .tracking(0.1)
    .features("'tnum' 1");
pub const MONO: TextStyle = TextStyle::new(12.0, 400.0)
    .lh(18.0)
    .family(Family::SystemMono);
pub const MONO_MED: TextStyle = TextStyle::new(12.0, 500.0)
    .lh(18.0)
    .family(Family::SystemMono);
pub const NUM: TextStyle = TextStyle::new(16.0, 600.0).lh(24.0).features("'tnum' 1");
/// The Browse billboard's title (`design/browse-top.md` 2): the only type this large.
pub const BILLBOARD: TextStyle = TextStyle::new(28.0, 600.0).lh(32.0).tracking(-0.3);
/// The Top 10 row's rank numeral, set on art.
pub const RANK: TextStyle = TextStyle::new(56.0, 700.0).lh(56.0).features("'tnum' 1");

/// Every shared text style, for the weight test.
#[cfg(test)]
const STYLES: [TextStyle; 15] = [
    PAGE_TITLE, SECTION, BODY, CHAT, BODY_MED, BODY_NUM, SMALL, SMALL_NUM, LABEL, NAV, WORDMARK,
    MICRO, MONO, MONO_MED, NUM,
];

/// How much room a page's toolbar line has, from the window size alone (never hover or data),
/// so the line never shifts while the pointer moves or a list loads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// 960 points or more of content width: everything labelled.
    Full,
    /// 760 to 959: labelled actions become icon buttons, search narrows to 176.
    Compact,
    /// Under 760: dropdown filters fold into one Filters menu, search 144.
    Tight,
}

impl Fit {
    pub fn from_width(content_w: f32) -> Fit {
        if content_w >= 960.0 {
            Fit::Full
        } else if content_w >= 760.0 {
            Fit::Compact
        } else {
            Fit::Tight
        }
    }

    /// The page search field's width at this fit.
    pub fn search_w(self) -> f32 {
        match self {
            Fit::Full => 240.0,
            Fit::Compact => 176.0,
            Fit::Tight => 144.0,
        }
    }
}

thread_local! {
    /// Per thread, as the view that sets it runs on the UI thread: a process-wide value let a
    /// view on one thread (a test's window) read the width another had set.
    static FIT: std::cell::Cell<Fit> = const { std::cell::Cell::new(Fit::Full) };
}

/// Set by the view each frame from the window and sidebar widths.
pub fn set_fit(fit: Fit) {
    FIT.with(|f| f.set(fit));
}

/// The toolbar fit for this frame. Pages read it to fold their own actions and filters.
pub fn fit() -> Fit {
    FIT.with(|f| f.get())
}

static WINDOW: std::sync::Mutex<(f32, f32)> = std::sync::Mutex::new((1280.0, 800.0));

/// Set by the view each frame: the window's size in points.
pub fn set_window(size: (f32, f32)) {
    if let Ok(mut w) = WINDOW.lock() {
        *w = size;
    }
}

/// The window's size for this frame, for pages that size a part to the window's height
/// (Crosshair's stage) instead of their content.
pub fn window() -> (f32, f32) {
    WINDOW.lock().map(|w| *w).unwrap_or((1280.0, 800.0))
}

/// The control heights. Every button, field, tab and menu row sits on one of these.
pub const CONTROL_S: f32 = 24.0;
pub const CONTROL_M: f32 = 28.0;
pub const CONTROL_L: f32 = 32.0;

/// The keyboard focus outline: 2 points, 1 point clear of the control.
pub const FOCUS_W: f32 = 2.0;
pub const FOCUS_GAP: f32 = 1.0;

#[derive(Clone, Copy)]
pub struct Ui {
    pub bg: Color,
    pub sidebar: Color,
    pub raised: Color,
    pub well: Color,
    pub hair: Color,
    pub hair_strong: Color,
    pub ink: Color,
    pub prose: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub accent_ink: Color,
    pub teal: Color,
    pub rose: Color,
    pub amber: Color,
    pub violet: Color,
    pub shadow: Color,
    pub glass_top: Color,
    pub glass_bottom: Color,
    pub sheen: Color,
    pub glow_cool: Color,
    pub glow_warm: Color,
    pub field: Color,
    pub hover: Color,
    pub selected: Color,
    /// The colour quiet overlays are mixed from: white on a dark theme, black on a light one.
    pub over: Color,
    /// The flat layer under a dialog.
    pub scrim: Color,
    pub light: bool,
}

/// The glass sidebar's tint: the sidebar colour at this opacity over the system backdrop.
/// Over Mica or vibrancy, which carry their own dark or light tint, and over a plain blur,
/// which shows the wallpaper as it is. `glass::tests` holds every text tier to AA over the
/// brightest (dark) or darkest (light) backdrop each can show.
pub const GLASS_TINT_DARK: f32 = 0.60;
pub const GLASS_TINT_DARK_BLUR: f32 = 0.88;
pub const GLASS_TINT_LIGHT: f32 = 0.45;
pub const GLASS_TINT_LIGHT_BLUR: f32 = 0.80;

/// The sidebar's colours on glass (`Ui::glass`).
#[derive(Clone, Copy, Debug)]
pub struct Glass {
    pub tint: Color,
    /// The 1 point inner highlight on the sidebar's right edge.
    pub edge: Color,
    pub hover: Color,
    pub selected: Color,
}

/// The palettes Settings offers. Dark is the original Sigil look; the rest keep its structure
/// (near flat layers, hairlines, one accent) and change the neutrals.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Oled,
    Midnight,
    Graphite,
    Mocha,
}

impl Theme {
    pub const ALL: [Theme; 6] = [
        Theme::Dark,
        Theme::Light,
        Theme::Oled,
        Theme::Midnight,
        Theme::Graphite,
        Theme::Mocha,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Oled => "oled",
            Theme::Midnight => "midnight",
            Theme::Graphite => "graphite",
            Theme::Mocha => "mocha",
        }
    }

    pub fn from_id(id: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|theme| theme.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Oled => "Black",
            Theme::Midnight => "Midnight",
            Theme::Graphite => "Graphite",
            Theme::Mocha => "Mocha",
        }
    }
}

/// The look the next `Ui::new` paints with. The view sets it from the saved settings at the
/// top of every run, so a change shows on the next frame everywhere, virtual rows included.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub theme: Theme,
    pub accent: Option<Color>,
    /// A palette from Personalize (a preset, a saved theme, or the builder's draft). None
    /// paints `theme`.
    pub custom: Option<Palette>,
    /// Corner radius multiplier for the shared primitives. 1 is the house grid.
    pub radius: f32,
    /// Interface density: 1 is the house size, lower is compact, higher is roomier.
    pub density: f32,
}

/// The house corner: the Block choice in Personalize. Controls, cards and dialogs get 4 points;
/// what sits inside them (knobs, chips, menu rows, segments) gets 2.
pub const BLOCK_RADIUS: f32 = 0.2;
const BLOCK_OUTER: f32 = 4.0;
const BLOCK_INNER: f32 = 2.0;
/// Radii asked for at or under this are inner parts.
const BLOCK_INNER_UP_TO: f32 = 6.0;

impl Look {
    pub const DEFAULT: Look = Look {
        theme: Theme::Graphite,
        accent: None,
        custom: None,
        radius: BLOCK_RADIUS,
        density: 1.0,
    };
}

static LOOK: std::sync::Mutex<Look> = std::sync::Mutex::new(Look::DEFAULT);

/// `Look::radius` and `Look::density` as f32 bits, beside the lock: `r` and `density` run
/// hundreds of times a build (every corner), and each took the lock.
static RADIUS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(Look::DEFAULT.radius.to_bits());
static DENSITY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(Look::DEFAULT.density.to_bits());

fn store_fast(look: &Look) {
    RADIUS.store(look.radius.to_bits(), std::sync::atomic::Ordering::Relaxed);
    DENSITY.store(look.density.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Set the theme and accent. Personalize's palette and radius stay as they are.
pub fn set_look(theme: Theme, accent: Option<Color>) {
    if let Ok(mut look) = LOOK.lock() {
        look.theme = theme;
        look.accent = accent;
    }
}

/// Set Personalize's palette (None for the theme's own) and the radius multiplier.
pub fn set_personal(custom: Option<Palette>, radius: f32) {
    if let Ok(mut look) = LOOK.lock() {
        look.custom = custom;
        look.radius = if radius.is_finite() { radius.clamp(0.0, 2.0) } else { 1.0 };
        store_fast(&look);
    }
}

pub fn look() -> Look {
    LOOK.lock().map(|look| *look).unwrap_or(Look::DEFAULT)
}

/// Personalize's density, as a scale on the whole interface. Set with the appearance pass.
pub fn set_density(scale: f32) {
    if let Ok(mut look) = LOOK.lock() {
        look.density = if scale.is_finite() { scale.clamp(0.8, 1.2) } else { 1.0 };
        store_fast(&look);
    }
}

pub fn density() -> f32 {
    f32::from_bits(DENSITY.load(std::sync::atomic::Ordering::Relaxed))
}

/// The smallest whole-window zoom (interface size times density). Every inset scales with the
/// zoom, so Interface size 90% with Compact (0.855) would put a 24 point page margin, an 18
/// point sidebar icon inset and an 8 point padding at 20.5, 15.4 and 6.8 points: content pressed
/// against the window and panel edges. At 0.9 they stay at 21.6, 16.2 and 7.2.
pub const MIN_ZOOM: f32 = 0.9;
/// The largest whole-window zoom: 125% with Spacious would leave a 1040 by 680 window about 750
/// points wide, too narrow for the page toolbars; 1.25 keeps them on one line.
pub const MAX_ZOOM: f32 = 1.25;

/// The zoom the window draws at: the interface size (`Prefs::zoom`, the zoom shortcuts) times
/// the density, kept between [`MIN_ZOOM`] and [`MAX_ZOOM`]. Compact still shrinks text and
/// controls at 100%, Spacious still grows them; only the extremes stop where insets give out.
pub fn zoom_for(interface: f32) -> f32 {
    let k = interface * density();
    if k.is_finite() { k.clamp(MIN_ZOOM, MAX_ZOOM) } else { 1.0 }
}

/// The zoom this frame draws at (`zoom_for`), as f32 bits. The view sets it before it builds,
/// so the inset floors below read the zoom the tree will be drawn with.
static ZOOM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1.0f32.to_bits());

/// Set by the view each frame, with the zoom it hands Rune.
pub fn set_zoom(zoom: f32) {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    ZOOM.store(zoom.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The zoom this frame draws at: 1 at Standard and 100%.
pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(std::sync::atomic::Ordering::Relaxed))
}

/// Density scales the space between things (gaps, rows, type); edge insets have a floor.
/// A page gutter or the sidebar's inner inset never draws under this many points.
pub const GUTTER_MIN: f32 = 12.0;
/// A panel, card, dialog or menu's inner padding never draws under these.
pub const INSET_X_MIN: f32 = 8.0;
pub const INSET_Y_MIN: f32 = 6.0;

/// `points` of inset in layout points, raised so that it draws at least `min` points on
/// screen (or `points` itself, when that is smaller) whatever the zoom. Where the zoom keeps
/// it above the floor (Standard, Spacious) it is `points` exactly, so those densities draw as
/// they always have.
pub fn floored(points: f32, min: f32) -> f32 {
    let z = zoom();
    let target = points.min(min);
    if points * z >= target - 0.001 { points } else { target / z }
}

/// A page gutter or the sidebar's inner inset (floor [`GUTTER_MIN`]).
pub fn gutter(points: f32) -> f32 {
    floored(points, GUTTER_MIN)
}

/// A panel, card, dialog or menu's left and right padding (floor [`INSET_X_MIN`]).
pub fn inset_x(points: f32) -> f32 {
    floored(points, INSET_X_MIN)
}

/// A panel, card, dialog or menu's top and bottom padding (floor [`INSET_Y_MIN`]).
pub fn inset_y(points: f32) -> f32 {
    floored(points, INSET_Y_MIN)
}

/// Inner padding for a panel, card, dialog or menu, with the density floors: `.inset(8.0, 8.0)`
/// in place of `.padding(8.0)`.
pub trait Inset {
    fn inset(self, x: f32, y: f32) -> Self;
}

impl Inset for El {
    fn inset(self, x: f32, y: f32) -> El {
        self.px(inset_x(x)).py(inset_y(y))
    }
}

/// An inset that keeps its Standard size on screen when the zoom is under 1 (the sidebar's
/// logo row, the status bar): Compact tightens what is inside them, not their edges.
pub fn standard(points: f32) -> f32 {
    let z = zoom();
    if z < 1.0 { points / z } else { points }
}

/// The selection thumb of a segmented control or tab strip: the selected part's fill and
/// border, a child placed before the label and pinned to the part's edges, so it takes no part
/// in the layout. `id` names the strip (and is the thumb's key): on a new pick the thumb glides from the
/// old part to the new one on `motion::base()`, whatever their widths, while the labels only
/// change colour. Instant under reduced motion. The part itself must not be wrapped in
/// `motion::colors`, which lands every change inside it; put that on the label instead.
///
/// Every part of the strip carries the slot, on or off, so the parts' other children keep their
/// places in the tree and nothing remounts (and fades) when the pick moves.
pub fn seg_thumb(on: bool, id: &str, fill: Color, border: Color, radius: f32) -> rune::El {
    if !on {
        return rune::el()
            .key(format!("{id}-slot"))
            .absolute(0.0, 0.0)
            .transition(crate::motion::cut())
            .a11y_hidden();
    }
    crate::motion::glides(
        rune::el()
            .key(id.to_string())
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(radius)
            .fill(fill)
            .border(1.0, border)
            .a11y_hidden(),
        id,
    )
}

/// A corner radius on the house grid, scaled by the Personalize radius choice. Block, the
/// default, maps every corner to 4 points (outer) or 2 (inner); Soft (1.0) is exact.
pub fn r(points: f32) -> f32 {
    let k = f32::from_bits(RADIUS.load(std::sync::atomic::Ordering::Relaxed));
    if k == 1.0 {
        points
    } else if k == BLOCK_RADIUS {
        if points <= 0.0 {
            0.0
        } else if points <= BLOCK_INNER_UP_TO {
            points.min(BLOCK_INNER)
        } else {
            BLOCK_OUTER
        }
    } else {
        (points * k).round()
    }
}

/// A switch that is on, in every theme and preset. Never the accent.
pub fn switch_green() -> Color {
    oklch(0.56, 0.15, 150.0)
}

/// The knob of every switch, on or off.
pub fn switch_knob() -> Color {
    oklch(0.93, 0.0, 0.0)
}

/// A floating layer's shadow colour. Only menus, dialogs, popovers and tooltips cast one.
pub fn soft(color: Color) -> Color {
    color
}

// --- palettes ---------------------------------------------------------------------

/// The colours a theme is built from, as Personalize edits them. Everything else Sigil
/// paints with (hover, faint text, strong lines, overlays) is derived.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette {
    pub background: Color,
    pub surface: Color,
    pub card: Color,
    pub field: Color,
    pub line: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_ink: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

/// One editable colour: its id in a theme file, its label and the one line under it.
pub struct Token {
    pub id: &'static str,
    pub label: &'static str,
    pub line: &'static str,
}

pub const TOKENS: [Token; 12] = [
    Token { id: "background", label: "Background", line: "The page behind everything." },
    Token { id: "surface", label: "Surface", line: "The sidebar." },
    Token { id: "card", label: "Card", line: "Dialogs, menus, popovers and cards." },
    Token { id: "field", label: "Field", line: "Inputs, search and wells." },
    Token { id: "line", label: "Line", line: "Hairlines and borders." },
    Token { id: "text", label: "Text", line: "Titles and body text." },
    Token { id: "muted", label: "Muted text", line: "Secondary lines and labels." },
    Token { id: "accent", label: "Accent", line: "The main action, switches that are on, focus." },
    Token { id: "accent_ink", label: "Accent text", line: "Text on the accent." },
    Token { id: "success", label: "Success", line: "Things that went well." },
    Token { id: "warning", label: "Warning", line: "Things that need a look." },
    Token { id: "danger", label: "Danger", line: "Errors and removals." },
];

/// WCAG 2 contrast ratio between two opaque colours (1 to 21).
pub fn contrast(a: Color, b: Color) -> f32 {
    // Colours are linear sRGB, so `luminance` is the relative luminance WCAG defines.
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// AA for body text.
pub const AA_TEXT: f32 = 4.5;

/// `#rrggbb` in sRGB, alpha dropped.
pub fn hex6(c: Color) -> String {
    let full = c.hex();
    full[..7].to_string()
}

/// `over` composited onto an opaque `under`.
fn flatten(over: Color, under: Color) -> Color {
    let mut c = under.mix(over, over.a);
    c.a = 1.0;
    c
}

impl Palette {
    pub fn get(&self, index: usize) -> Color {
        match index {
            0 => self.background,
            1 => self.surface,
            2 => self.card,
            3 => self.field,
            4 => self.line,
            5 => self.text,
            6 => self.muted,
            7 => self.accent,
            8 => self.accent_ink,
            9 => self.success,
            10 => self.warning,
            _ => self.danger,
        }
    }

    pub fn set(&mut self, index: usize, c: Color) {
        let c = c.alpha(1.0);
        match index {
            0 => self.background = c,
            1 => self.surface = c,
            2 => self.card = c,
            3 => self.field = c,
            4 => self.line = c,
            5 => self.text = c,
            6 => self.muted = c,
            7 => self.accent = c,
            8 => self.accent_ink = c,
            9 => self.success = c,
            10 => self.warning = c,
            _ => self.danger = c,
        }
    }

    /// The editable colours of a painted look, translucent lines flattened onto the page.
    pub fn from_ui(u: &Ui) -> Palette {
        Palette {
            background: u.bg.alpha(1.0),
            surface: u.sidebar.alpha(1.0),
            card: u.raised.alpha(1.0),
            field: u.well.alpha(1.0),
            line: flatten(u.hair, u.bg.alpha(1.0)),
            text: u.ink.alpha(1.0),
            muted: u.muted.alpha(1.0),
            accent: u.accent.alpha(1.0),
            accent_ink: u.accent_ink.alpha(1.0),
            success: u.teal.alpha(1.0),
            warning: u.amber.alpha(1.0),
            danger: u.rose.alpha(1.0),
        }
    }

    /// True when the page is light enough that overlays darken instead of lighten.
    pub fn is_light(&self) -> bool {
        self.background.luminance() > 0.2
    }

    /// The full set of tokens for this palette.
    pub fn to_ui(&self) -> Ui {
        let light = self.is_light();
        let mut u = if light { Ui::light_palette() } else { Ui::dark() };
        let over = if light { Color::BLACK } else { Color::WHITE };
        u.bg = self.background;
        u.sidebar = self.surface;
        u.raised = self.card;
        u.well = self.field;
        u.field = self.field;
        u.hair = self.line;
        u.hair_strong = self.line.mix(self.text, 0.12);
        u.ink = self.text;
        u.prose = self.text.mix(self.muted, 0.35);
        u.muted = self.muted;
        u.faint = self.muted.mix(self.background, 0.3);
        u.accent = self.accent;
        u.accent_ink = self.accent_ink;
        u.teal = self.success;
        u.amber = self.warning;
        u.rose = self.danger;
        u.over = over;
        u.light = light;
        u
    }

    /// Text and background pairs that fall under WCAG AA, as (foreground token, background
    /// token, ratio). Muted text is held to the body text bar because Sigil sets it at 12
    /// and 13 points.
    pub fn contrast_warnings(&self) -> Vec<(usize, usize, f32)> {
        const PAIRS: [(usize, usize); 10] = [
            (5, 0),
            (5, 1),
            (5, 2),
            (5, 3),
            (6, 0),
            (6, 2),
            (8, 7),
            (9, 0),
            (10, 0),
            (11, 0),
        ];
        PAIRS
            .iter()
            .filter_map(|&(fg, bg)| {
                let ratio = contrast(self.get(fg), self.get(bg));
                (ratio < AA_TEXT).then_some((fg, bg, ratio))
            })
            .collect()
    }
}

/// A theme Sigil ships for Personalize. Original names and palettes.
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub colors: [&'static str; 12],
}

impl Preset {
    pub fn palette(&self) -> Palette {
        let c = |i: usize| rune::color::hex(self.colors[i]).unwrap_or(Color::BLACK);
        Palette {
            background: c(0),
            surface: c(1),
            card: c(2),
            field: c(3),
            line: c(4),
            text: c(5),
            muted: c(6),
            accent: c(7),
            accent_ink: c(8),
            success: c(9),
            warning: c(10),
            danger: c(11),
        }
    }
}

/// background, surface, card, field, line, text, muted, accent, accent text, success,
/// warning, danger.
pub const PRESETS: [Preset; 10] = [
    Preset {
        id: "brass",
        name: "Brass",
        colors: [
            "#16130f", "#110f0c", "#211d17", "#1c1914", "#2c2720", "#f3ede3", "#b5ab9a",
            "#d6a44e", "#1d1608", "#82c99c", "#e8c26a", "#e8846f",
        ],
    },
    Preset {
        id: "verdigris",
        name: "Verdigris",
        colors: [
            "#0f1513", "#0b110f", "#17211e", "#141c19", "#22302b", "#e8f1ed", "#a0b6ae",
            "#4fbf9f", "#05201a", "#80d39b", "#e6c46b", "#ef8c7c",
        ],
    },
    Preset {
        id: "oxblood",
        name: "Oxblood",
        colors: [
            "#170f10", "#120b0c", "#231718", "#1d1314", "#352221", "#f4e9e8", "#c2a6a4",
            "#c9473f", "#ffffff", "#88cb9d", "#e8c06b", "#ff8d80",
        ],
    },
    Preset {
        id: "cobalt",
        name: "Cobalt",
        colors: [
            "#0f131a", "#0b0f15", "#171d28", "#131822", "#222b3a", "#e9eef7", "#a2adc2",
            "#5b8def", "#0a1530", "#7fd0a0", "#e9c46e", "#f08a7e",
        ],
    },
    Preset {
        id: "amethyst",
        name: "Amethyst",
        colors: [
            "#13111a", "#0f0d15", "#1d1a27", "#191622", "#2b2639", "#efeaf7", "#afa6c4",
            "#a78bfa", "#1a1033", "#86cfa2", "#eac56f", "#f28e8a",
        ],
    },
    Preset {
        id: "lagoon",
        name: "Lagoon",
        colors: [
            "#0d1517", "#0a1113", "#152124", "#111c1f", "#203034", "#e6f2f3", "#9eb9bd",
            "#22b8cf", "#04242a", "#7fd5a6", "#e8c66d", "#f08d80",
        ],
    },
    Preset {
        id: "sepia",
        name: "Sepia",
        colors: [
            "#1a1613", "#15120f", "#25201b", "#201b17", "#332c25", "#f2e9df", "#b9ab9b",
            "#d0814f", "#1f1006", "#8ec99a", "#e6c06c", "#ec8673",
        ],
    },
    Preset {
        id: "slate",
        name: "Slate",
        colors: [
            "#1e1f22", "#191a1c", "#28292d", "#232427", "#34353a", "#ececee", "#a9aaaf",
            "#e4e4e7", "#18181b", "#86cfa0", "#e8c46c", "#ef8b80",
        ],
    },
    Preset {
        id: "frost",
        name: "Frost",
        colors: [
            "#f4f6f9", "#e8ecf2", "#ffffff", "#edf0f5", "#d7dde6", "#18202d", "#4f5a6c",
            "#2a64d2", "#ffffff", "#1d7f50", "#8a5f00", "#c0352d",
        ],
    },
    Preset {
        id: "parchment",
        name: "Parchment",
        colors: [
            "#f6f1e7", "#ebe3d3", "#fffbf3", "#efe8d9", "#dcd1bc", "#2a2219", "#655845",
            "#8c5a26", "#ffffff", "#3a7646", "#835c00", "#ab3628",
        ],
    },
];

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// Ink that reads on top of an accent fill: near black on a light accent, white on a dark one.
pub fn ink_on(accent: Color) -> Color {
    if accent.luminance() > 0.35 {
        oklch(0.19, 0.01, 50.0)
    } else {
        Color::WHITE
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

/// The end padding a scroll viewport's content keeps where it would otherwise run to the edge,
/// so the scroll bar (`Ui::scroll_area`) sits in room of its own. It is there whether or not
/// the content overflows, so the bar appearing never changes the content's width.
pub const SCROLL_GUTTER: f32 = 12.0;

/// Rune's launch flags with scroll bars always shown while content overflows, unless the
/// command line already chose (`--scroll-bars when-scrolling`).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn scroll_bars_shown(mut args: Vec<String>) -> Vec<String> {
    if !args.iter().any(|a| a == "--scroll-bars") {
        args.extend(["--scroll-bars".to_owned(), "always".to_owned()]);
    }
    args
}

thread_local! {
    /// The scroll bar style for the last palette asked, built once per look rather than per
    /// viewport: the kit it comes from solves a whole palette.
    static SCROLL_BAR: std::cell::Cell<Option<(Color, Color, f32, rune_kit::ScrollBarStyle)>> =
        const { std::cell::Cell::new(None) };
}

impl Ui {
    /// A `rune-kit` kit on Sigil's palette: Rune's neutral tokens for this theme, with the
    /// border tokens the kit's components read taken from Sigil's hairlines and muted ink.
    pub fn kit(&self) -> rune_kit::Kit {
        let mut k = rune_kit::Kit::neutral(!self.light);
        k.c.border_muted = self.hair;
        k.c.border_base = self.hair_strong;
        k.c.border_strong = self.muted;
        k.c.text_base = self.ink;
        k.c.text_muted = self.muted;
        k.c.page = self.bg;
        k.c.bg_base = self.raised;
        k
    }

    /// Sigil's scroll bar: a graphite thumb (`hair_strong` at rest, `muted` under the pointer or
    /// while dragged), 6 points wide and 10 under the pointer, 24 long at least. Under Reduce
    /// motion it keeps its width under the pointer and only changes colour. Its ends are the
    /// kit's rounded ones: the kit draws the thumb as a pill and offers no corner of its own.
    pub fn scroll_bar_style(&self) -> rune_kit::ScrollBarStyle {
        let (rest, hover) = (self.hair_strong, self.muted);
        let hover_width = if crate::motion::reduced() { 6.0 } else { 10.0 };
        if let Some((r, h, w, style)) = SCROLL_BAR.with(std::cell::Cell::get)
            && r == rest
            && h == hover
            && w == hover_width
        {
            return style;
        }
        let style = rune_kit::ScrollBarStyle::from_kit(&self.kit())
            .thumb(rest)
            .thumb_hover(hover)
            .width(6.0)
            .hover_width(hover_width)
            .min_thumb(24.0);
        SCROLL_BAR.with(|c| c.set(Some((rest, hover, hover_width, style))));
        style
    }

    /// Every scroll viewport goes through here: `viewport` (a `.scroll(..)` column, last in its
    /// chain) with a scroll bar over its end edge. The wrapper takes the viewport's own sizing
    /// and placement, keeps its key and scroll handlers, and moves the thumb through live
    /// values, so scrolling never rebuilds the view. The bar overlays the viewport's edge and
    /// takes no layout width; content that runs to the edge keeps `SCROLL_GUTTER` of end
    /// padding. Shown whenever the content overflows on Windows and Linux (`main.rs` asks Rune
    /// for always-shown bars there); on macOS it follows "Show scroll bars". A viewport without
    /// a key is told apart by the call site, so give one built in a loop a key.
    #[track_caller]
    pub fn scroll_area(&self, viewport: El) -> El {
        use rune_kit::ScrollBar;
        viewport.scroll_bar_styled(self.scroll_bar_style())
    }

    /// `scroll_area` for a viewport that scrolls sideways (`.scroll_x()`): the bar runs along
    /// its bottom edge.
    #[track_caller]
    pub fn scroll_area_x(&self, viewport: El) -> El {
        use rune_kit::ScrollBar;
        viewport.scroll_bar_styled(self.scroll_bar_style().horizontal())
    }
}

impl Ui {
    /// The palette for the current look (see `set_look`).
    pub fn new() -> Ui {
        let look = look();
        match look.custom {
            Some(palette) => {
                let mut u = palette.to_ui();
                if let Some(accent) = look.accent {
                    u.accent = accent;
                    u.accent_ink = ink_on(accent);
                }
                u
            }
            None => Ui::themed(look.theme, look.accent),
        }
    }

    pub fn themed(theme: Theme, accent: Option<Color>) -> Ui {
        let mut u = match theme {
            Theme::Dark => Ui::dark(),
            Theme::Light => Ui::light_palette(),
            Theme::Oled => Ui::dark_variant(0.0, 0.0, 0.17, 0.12, 0.0, 286.0),
            Theme::Midnight => Ui::dark_variant(0.18, 0.155, 0.22, 0.205, 0.022, 262.0),
            Theme::Graphite => Ui::graphite(),
            Theme::Mocha => Ui::dark_variant(0.19, 0.165, 0.228, 0.212, 0.013, 55.0),
        };
        if let Some(accent) = accent {
            u.accent = accent;
            u.accent_ink = ink_on(accent);
        }
        u
    }

    /// A dark theme from four layer lightnesses and one tint.
    fn dark_variant(bg: f32, sidebar: f32, raised: f32, well: f32, chroma: f32, hue: f32) -> Ui {
        let mut u = Ui::dark();
        u.bg = oklch(bg, chroma, hue);
        u.sidebar = oklch(sidebar, chroma, hue);
        u.raised = oklch(raised, chroma * 1.2, hue);
        u.well = oklch(well, chroma, hue);
        u.field = u.well;
        u.prose = oklch(0.88, chroma.min(0.012), hue);
        u.muted = oklch(0.72, (chroma * 1.5).clamp(0.008, 0.03), hue);
        u.faint = oklch(0.65, (chroma * 1.5).clamp(0.008, 0.03), hue);
        u
    }

    /// The house look: neutral graphite in small tonal steps (well, sidebar, page, raised),
    /// 1 point low-contrast lines, shadows only on floating layers, and four text tiers that
    /// all clear AA on every layer.
    fn graphite() -> Ui {
        let mut u = Ui::dark_variant(0.225, 0.2, 0.268, 0.185, 0.0, 0.0);
        u.hair = Color::WHITE.alpha(0.085);
        u.hair_strong = Color::WHITE.alpha(0.15);
        u.ink = oklch(0.96, 0.0, 0.0);
        u.prose = oklch(0.88, 0.0, 0.0);
        u.muted = oklch(0.72, 0.0, 0.0);
        u.faint = oklch(0.65, 0.0, 0.0);
        u.hover = Color::WHITE.alpha(0.05);
        u.selected = Color::WHITE.alpha(0.09);
        u.shadow = Color::BLACK.alpha(0.40);
        u
    }

    /// Warm paper neutrals after Grimoire's light theme (MIT), with darker state colours so
    /// they read on white.
    pub(crate) fn light_palette() -> Ui {
        let black = Color::BLACK;
        Ui {
            bg: oklch(0.982, 0.004, 85.0),
            sidebar: oklch(0.952, 0.006, 85.0),
            raised: oklch(0.995, 0.002, 85.0),
            well: oklch(0.955, 0.006, 85.0),
            hair: black.alpha(0.085),
            hair_strong: black.alpha(0.13),
            ink: oklch(0.22, 0.006, 85.0),
            prose: oklch(0.32, 0.008, 85.0),
            muted: oklch(0.47, 0.01, 85.0),
            faint: oklch(0.52, 0.01, 85.0),
            accent: oklch(0.68, 0.175, 48.0),
            accent_ink: Color::WHITE,
            teal: oklch(0.52, 0.12, 155.0),
            rose: oklch(0.55, 0.19, 25.0),
            amber: oklch(0.58, 0.13, 70.0),
            violet: oklch(0.5, 0.15, 280.0),
            shadow: black.alpha(0.14),
            glass_top: Color::TRANSPARENT,
            glass_bottom: Color::TRANSPARENT,
            sheen: black.alpha(0.05),
            glow_cool: Color::TRANSPARENT,
            glow_warm: Color::TRANSPARENT,
            field: oklch(0.955, 0.006, 85.0),
            hover: black.alpha(0.045),
            selected: black.alpha(0.07),
            over: black,
            scrim: black.alpha(0.32),
            light: true,
        }
    }

    pub(crate) fn dark() -> Ui {
        Ui {
            bg: oklch(0.172, 0.002, 286.0),
            sidebar: oklch(0.145, 0.002, 286.0),
            raised: oklch(0.205, 0.003, 286.0),
            well: oklch(0.192, 0.003, 286.0),
            hair: Color::WHITE.alpha(0.065),
            hair_strong: Color::WHITE.alpha(0.10),
            ink: oklch(0.967, 0.001, 286.0),
            prose: oklch(0.87, 0.004, 286.0),
            muted: oklch(0.72, 0.012, 286.0),
            faint: oklch(0.65, 0.012, 286.0),
            accent: oklch(0.735, 0.155, 52.0),
            accent_ink: oklch(0.20, 0.04, 50.0),
            teal: oklch(0.78, 0.12, 155.0),
            rose: oklch(0.71, 0.16, 22.0),
            amber: oklch(0.84, 0.13, 85.0),
            violet: oklch(0.74, 0.10, 280.0),
            shadow: Color::BLACK.alpha(0.40),
            glass_top: Color::TRANSPARENT,
            glass_bottom: Color::TRANSPARENT,
            sheen: Color::WHITE.alpha(0.08),
            glow_cool: Color::TRANSPARENT,
            glow_warm: Color::TRANSPARENT,
            field: oklch(0.192, 0.003, 286.0),
            hover: Color::WHITE.alpha(0.045),
            selected: Color::WHITE.alpha(0.075),
            over: Color::WHITE,
            scrim: Color::BLACK.alpha(0.6),
            light: false,
        }
    }

    /// The sidebar's look over a system backdrop (Settings > Appearance > Transparent sidebar).
    /// `tinted`: the material brings its own tint (Mica, vibrancy); a plain blur does not, so
    /// the tint is denser over it.
    pub fn glass(&self, tinted: bool) -> Glass {
        let a = match (self.light, tinted) {
            (false, true) => GLASS_TINT_DARK,
            (false, false) => GLASS_TINT_DARK_BLUR,
            (true, true) => GLASS_TINT_LIGHT,
            (true, false) => GLASS_TINT_LIGHT_BLUR,
        };
        Glass {
            tint: self.sidebar.alpha(a),
            // A 1 point line inside the right edge: light catching the pane's edge on a dark
            // theme, a hairline on a light one.
            edge: if self.light { Color::BLACK.alpha(0.09) } else { Color::WHITE.alpha(0.10) },
            hover: self.over.alpha(if self.light { 0.06 } else { 0.07 }),
            selected: self.over.alpha(if self.light { 0.10 } else { 0.12 }),
        }
    }

    /// This palette for the parts drawn on glass: hover and selected fills a step stronger, so
    /// they still read over a busy backdrop.
    pub fn on_glass(&self, glass: &Glass) -> Ui {
        Ui { hover: glass.hover, selected: glass.selected, ..*self }
    }

    /// A quiet section label in sentence case. Kept for old call sites; prefer `section`.
    pub fn eyebrow(&self, s: impl Into<String>) -> El {
        self.section(&sentence(&s.into()))
    }

    /// A section header inside a page: `SECTION` (12/600) in `prose` on a 24 point row. The
    /// only 600 weight in content, so it reads as a header without being loud. Leave 24
    /// points above it (none when first on the page) and 8 below.
    pub fn section(&self, title: &str) -> El {
        self.section_head(title, None, None)
    }

    /// `section` with a count after the title (`SMALL_NUM` faint, no parentheses) and an
    /// optional trailing control (a 24 point ghost button) on the right.
    pub fn section_head(&self, title: &str, count: Option<usize>, trailing: Option<El>) -> El {
        // Full width only with a trailing control: a bare header also sits inside other rows.
        let mut line = row()
            .h(CONTROL_S)
            .gap(6.0)
            .center_y()
            .child(
                text(title.to_string())
                    .font(SECTION)
                    .color(self.prose)
                    .role("heading")
                    .level(2),
            );
        if let Some(count) = count {
            line = line.child(text(count.to_string()).font(SMALL_NUM).color(self.faint));
        }
        if let Some(trailing) = trailing {
            line = line.w_full().child(spacer()).child(trailing);
        }
        line
    }

    /// A group of rows separated by hairlines. Not a card: no fill, a 1 point border.
    pub fn panel(&self) -> El {
        col().radius(r(10.0)).border(1.0, self.hair)
    }

    /// Ambient glow is gone; the slot stays so screens keep their structure.
    pub fn glow(&self, key: &str, _center: [f32; 2], _radius: (f32, f32), _color: Color) -> El {
        el().key(key)
            .absolute(0.0, 0.0)
            .size(0.0, 0.0)
            .a11y_hidden()
    }

    pub fn glass_shell(&self) -> El {
        col()
    }

    pub fn glass_core(&self) -> El {
        col().radius(r(10.0)).border(1.0, self.hair)
    }

    /// A dialog: an opaque raised surface with one soft shadow. Floating layers are the only
    /// place a shadow appears.
    pub fn dialog_shell(&self) -> El {
        col()
            .fill(self.raised)
            .radius(r(14.0))
            .border(1.0, self.hair_strong)
            .shadow(
                24.0,
                64.0,
                soft(Color::BLACK.alpha(if self.light { 0.18 } else { 0.55 })),
            )
    }

    pub fn dialog_core(&self) -> El {
        col()
    }

    /// A full-window modal layer: a flat scrim and the dialog centred over the shell. The scrim
    /// fades; the dialog rises 8 points as it fades in and fades out in place. The layer itself
    /// has no entrance, so each part plays its own, and fades out whole.
    pub fn modal_layer(&self, key: &str, inner: El) -> El {
        let scrim = el()
            .key(format!("{key}-scrim"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(self.scrim)
            .transition(crate::motion::fade())
            .a11y_hidden();
        el().key(key)
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .transition(crate::motion::layer())
            .child(scrim)
            .child(
                el().key(format!("{key}-panel"))
                    .h_full()
                    .w_full()
                    .padding(32.0)
                    .center()
                    .transition(crate::motion::lift())
                    .child(inner),
            )
    }

    /// An inset field for a path, hash or prompt.
    pub fn well_field(&self) -> El {
        col().fill(self.well).radius(r(8.0)).border(1.0, self.hair)
    }

    /// A button. The glyph argument is ignored: buttons are text, or an icon via `icon_button`.
    pub fn pill(&self, label: &str, _glyph: &str, kind: ActionKind) -> El {
        self.action(label, kind)
    }

    pub fn divider(&self) -> El {
        rule(1.0, self.hair).w_full()
    }

    pub fn dot(&self, tone: Tone) -> El {
        el().size(6.0, 6.0)
            .radius(3.0)
            .fill(tone.fg(self))
            .a11y_hidden()
    }

    /// Glyph boxes were decoration; they now draw nothing and take no space.
    pub fn glyph_box(&self, _glyph: &str, _tone: Tone, _size: f32) -> El {
        el().size(0.0, 0.0).a11y_hidden()
    }

    /// A number and its label, no tile.
    pub fn stat(&self, value: impl Into<String>, label: &str, tone: Tone) -> El {
        col()
            .grow(1.0)
            .gap(2.0)
            .child(text(value).font(NUM).color(match tone {
                Tone::Quiet | Tone::Accent | Tone::Info => self.ink,
                other => other.fg(self),
            }))
            .child(text(sentence(label)).font(SMALL).color(self.faint))
    }

    /// A quiet inline tag. Colour only when the state is real (warn, bad).
    pub fn chip(&self, label: &str, fg: Color, _bg: Color) -> El {
        text(sentence(label)).font(SMALL).color(fg).a11y_hidden()
    }

    pub fn status_chip(&self, label: &str, tone: Tone) -> El {
        let color = match tone {
            Tone::Warn | Tone::Bad => tone.fg(self),
            _ => self.faint,
        };
        self.chip(label, color, Color::TRANSPARENT)
    }

    pub fn compat_chip(&self, c: Compat) -> El {
        self.status_chip(c.label(), c.tone())
    }

    /// A 32 point button. Primary is ember and appears once per view; secondary is a hairline
    /// button; ghost is text that lights on hover.
    pub fn action(&self, label: &str, kind: ActionKind) -> El {
        let (fill, ink, border) = self.action_colors(kind);
        row()
            .h(CONTROL_L)
            .px(12.0)
            .gap(6.0)
            .center()
            .radius(r(8.0))
            .fill(fill)
            .border(1.0, border)
            .hover(if kind == ActionKind::Primary {
                Color::WHITE.alpha(0.10)
            } else {
                self.hover
            })
            .press(self.over.alpha(0.08))
            .focusable(true)
            .role("button")
            .label(label)
            .child(
                text(label.to_string())
                    .font(BODY_MED)
                    .color(ink)
                    .a11y_hidden(),
            )
    }

    /// A 28 point button for controls inside content and on the titlebar: card actions, row
    /// actions, pickers inside panels. Same look as `action`.
    pub fn action_m(&self, label: &str, kind: ActionKind) -> El {
        self.action(label, kind).h(CONTROL_M).px(10.0)
    }

    /// `icon_action` at 28 points.
    pub fn icon_action_m(&self, glyph: &'static Icon, label: &str, kind: ActionKind) -> El {
        self.icon_action(glyph, label, kind).h(CONTROL_M).pl(8.0).pr(10.0)
    }

    /// A button with a leading icon.
    pub fn icon_action(&self, glyph: &'static Icon, label: &str, kind: ActionKind) -> El {
        let (fill, ink, border) = self.action_colors(kind);
        row()
            .h(CONTROL_L)
            .pl(10.0)
            .pr(12.0)
            .gap(8.0)
            .center()
            .radius(r(8.0))
            .fill(fill)
            .border(1.0, border)
            .hover(if kind == ActionKind::Primary {
                Color::WHITE.alpha(0.10)
            } else {
                self.hover
            })
            .press(self.over.alpha(0.08))
            .focusable(true)
            .role("button")
            .label(label)
            .child(icon(glyph, 16.0).color(ink).a11y_hidden())
            .child(
                text(label.to_string())
                    .font(BODY_MED)
                    .color(ink)
                    .a11y_hidden(),
            )
    }

    /// A square 32 point icon button, labelled for assistive tech and with its label as the
    /// tooltip (every icon-only control has one; a caller may replace it with `.help`).
    pub fn icon_button(&self, glyph: &'static Icon, label: &str) -> El {
        el().size(CONTROL_L, CONTROL_L)
            .center()
            .radius(r(8.0))
            .hover(self.hover)
            .press(self.over.alpha(0.08))
            .focusable(true)
            .role("button")
            .label(label)
            .help(label)
            .child(icon(glyph, 16.0).color(self.muted).a11y_hidden())
    }

    fn action_colors(&self, kind: ActionKind) -> (Color, Color, Color) {
        match kind {
            ActionKind::Primary => (self.accent, self.accent_ink, Color::TRANSPARENT),
            ActionKind::Secondary => (self.over.alpha(0.035), self.ink, self.hair_strong),
            ActionKind::Ghost => (Color::TRANSPARENT, self.muted, Color::TRANSPARENT),
            ActionKind::Danger => (self.rose.alpha(0.08), self.rose, self.rose.alpha(0.30)),
        }
    }

    /// The switch: a square 24 by 16 track in a 32 by 24 press target. It takes its own press,
    /// focus and name. Inside a row that is itself the switch, use `toggle_mark`.
    pub fn toggle(&self, on: bool) -> El {
        self.toggle_mark(on).role("switch").checked(on).focusable(true)
    }

    /// The switch's look with no input of its own, so a press on it reaches the row around it.
    /// That row carries the press, the "switch" role, `checked`, focus and the name.
    pub fn toggle_mark(&self, on: bool) -> El {
        const TRACK: (f32, f32) = (24.0, 16.0);
        const KNOB: f32 = 12.0;
        const INSET: f32 = 2.0;
        // The knob slides and the track's fill eases on `fast` (90 ms, no overshoot). Both boxes
        // are fixed, so only the knob's place inside the track moves.
        let motion = crate::motion::fast();
        el().key("toggle-mark").size(32.0, 24.0).center().child(
            el().key("toggle-track")
                .size(TRACK.0, TRACK.1)
                .radius(4.0)
                .fill(if on { switch_green() } else { self.switch_off() })
                .motion(motion)
                .a11y_hidden()
                .child(
                    el().key("toggle-knob")
                        .size(KNOB, KNOB)
                        .absolute(if on { TRACK.0 - KNOB - INSET } else { INSET }, INSET)
                        .radius(2.0)
                        .fill(switch_knob())
                        .motion(motion)
                        .a11y_hidden(),
                ),
        )
    }

    /// The off track: neutral, a step away from the surface in dark and light themes alike.
    pub fn switch_off(&self) -> Color {
        oklch(if self.light { 0.58 } else { 0.40 }, 0.0, 0.0)
    }

    pub fn kv(&self, key: &str, value: &str, value_color: Color) -> El {
        row()
            .w_full()
            .gap(16.0)
            .py(6.0)
            .items_start()
            .child(text(key.to_string()).font(BODY).color(self.muted).w(120.0))
            .child(
                text(value.to_string())
                    .font(BODY)
                    .color(value_color)
                    .grow(1.0)
                    .wrap_anywhere(),
            )
    }

    /// An empty state on the page's left edge: title, one line, then the action 16 below.
    /// No box, no icon tile, no centred block in a void.
    pub fn empty_state(&self, _glyph: &str, title: &str, detail: &str, action: Option<El>) -> El {
        let mut c = col()
            .w_full()
            .pt(48.0)
            .gap(4.0)
            .items_start()
            .child(text(title.to_string()).font(PAGE_TITLE).color(self.ink))
            .child(
                text(detail.to_string())
                    .font(BODY)
                    .color(self.prose)
                    .max_w(560.0)
                    .wrap(),
            );
        if let Some(a) = action {
            c = c.child(el().h(12.0)).child(a);
        }
        c
    }

    /// The page header, now the page's one toolbar line: the title, then the trailing
    /// controls, 32 points high, no subtitle, no rule. The kicker and detail arguments are
    /// ignored (a page that needs a sentence puts it in its empty state or a `note`). New code
    /// calls `page_toolbar` directly.
    pub fn page_head(&self, _kicker: &str, title: &str, _detail: &str, trailing: Option<El>) -> El {
        self.page_toolbar(self.page_title(title), Vec::new(), None, trailing.into_iter().collect())
    }

    pub fn panel_header(&self, title: &str, meta: &str) -> El {
        row()
            .w_full()
            .h(CONTROL_L)
            .px(16.0)
            .gap(12.0)
            .items_center()
            .border_sides([0.0, 0.0, 1.0, 0.0], self.hair)
            .child(text(sentence(title)).font(BODY_MED).color(self.ink))
            .child(spacer())
            .child(text(meta.to_string()).font(SMALL).color(self.faint))
    }

    pub fn table_head(&self, cols: &[(&str, f32)]) -> El {
        let mut r = row().w_full().px(16.0).h(CONTROL_M).items_center().gap(12.0);
        for (label, w) in cols {
            let e = text(sentence(label)).font(SMALL).color(self.muted);
            r = r.child(if *w > 0.0 { e.w(*w) } else { e.grow(1.0) });
        }
        r.border_sides([0.0, 0.0, 1.0, 0.0], self.hair)
    }

    /// A one line note in secondary text. No box.
    pub fn note(&self, s: impl Into<String>) -> El {
        text(s).font(SMALL).color(self.muted).wrap().w_full()
    }

    pub fn field_label(&self, s: &str) -> El {
        text(sentence(s)).font(LABEL).color(self.muted)
    }

    /// A group heading above a grid: the section header with its count, `Enabled  6`.
    pub fn grid_head(&self, label: &str, meta: &str) -> El {
        let count = meta.split_whitespace().next().and_then(|n| n.replace(',', "").parse().ok());
        self.section_head(&sentence(label), count, None)
    }

    /// Sigil's tooltip for every `El::help`: a raised graphite block with a 1 point line, 4
    /// point corners, small text, a short delay and a 90 ms fade (60 ms out), no travel. Rune
    /// draws it on macOS too instead of AppKit's yellow-grey tip.
    pub fn help_look(&self) -> rune::HelpLook {
        rune::HelpLook::system(!self.light)
            .fill(self.raised.mix(self.over, 0.06).alpha(1.0))
            .ink(self.ink)
            .ring(1.0, self.hair_strong)
            .radius(r(8.0))
            .padding(inset_x(8.0), 4.0)
            .font(LABEL)
            .max_width(Some(240.0))
            .gap(4.0)
            .margin(8.0)
            .delay(0.4)
            .skip_delay(0.3)
            .fade(crate::motion::tip_fade().0, crate::motion::tip_fade().1, 1.0)
            .keycaps(self.over.alpha(0.08), self.muted, MICRO, false)
            .keycap_shape(16.0, 4.0, 2.0, 2.0, 8.0)
            .shadows([rune::Shadow {
                x: 0.0,
                y: 4.0,
                blur: 12.0,
                spread: 0.0,
                color: Color::BLACK.alpha(if self.light { 0.12 } else { 0.35 }),
                inset: false,
            }])
    }

    /// A page's one toolbar line, 32 points high: the lead (the page title from `page_title`,
    /// or the page's tabs), then its filters, then its own search (`page_search`), then its
    /// actions on the trailing edge. Filters give way first when the window narrows.
    pub fn page_toolbar(
        &self,
        lead: El,
        filters: Vec<El>,
        search: Option<El>,
        actions: Vec<El>,
    ) -> El {
        let mut line = row()
            .key("page-toolbar")
            // With no band over the page, the toolbar's empty stretch moves the window; its
            // controls keep their presses.
            .window_drag()
            .w_full()
            .h(CONTROL_L)
            .gap(8.0)
            .center_y()
            .child(lead);
        if !filters.is_empty() {
            line = line.child(
                row()
                    .key("page-toolbar-filters")
                    .gap(8.0)
                    .center_y()
                    .shrink()
                    .children(filters),
            );
        }
        line = line.child(spacer());
        if let Some(search) = search {
            line = line.child(search);
        }
        line.children(actions)
    }

    /// The page title on its toolbar line: one per page.
    pub fn page_title(&self, title: &str) -> El {
        text(title.to_string())
            .font(PAGE_TITLE)
            .color(self.ink)
            .role("heading")
            .level(1)
    }

    /// A page's own search field for its toolbar line, 32 points high. The field comes from
    /// `rune::text_input` so the page owns its `TextEdit`.
    pub fn page_search(&self, field: El) -> El {
        self.search_shell(field).w(fit().search_w())
    }

    /// The toolbar search field. The field itself comes from `rune::text_input` so each screen
    /// owns its own `TextEdit`.
    pub fn search_shell(&self, field: El) -> El {
        row()
            .h(CONTROL_L)
            .w(280.0)
            .pl(10.0)
            .pr(10.0)
            .gap(8.0)
            .center_y()
            .radius(r(8.0))
            .fill(self.well)
            .border(1.0, self.hair)
            .child(
                icon(&crate::icons::SEARCH, 14.0)
                    .color(self.faint)
                    .a11y_hidden(),
            )
            .child(field.grow(1.0).min_w(0.0))
    }

    // --- shared rows and overlays (polish lane "rest"; also used by Installed and Launch) ---

    /// A page's status row: fixed 24 points, always drawn (empty when idle), never wraps.
    /// `left` holds state chips and result lines; `right` an optional quiet note or count.
    /// It sits outside the page scroll, 8 under the toolbar and 16 above the body.
    pub fn status_row(&self, key: &str, left: Vec<El>, right: Option<El>) -> El {
        let mut line = row()
            .key(key.to_string())
            .w_full()
            .h(CONTROL_S)
            .gap(8.0)
            .center_y()
            .child(
                row()
                    .key(format!("{key}-left"))
                    .gap(8.0)
                    .center_y()
                    .grow(1.0)
                    .min_w(0.0)
                    .children(left),
            );
        if let Some(right) = right {
            line = line.child(right);
        }
        line
    }

    /// A state chip for a status row: 24 high, 4 point corners, a quiet fill, `LABEL` text in
    /// `prose`. The tone lives in the 12 point icon only. Add `.on_press` to make it a button.
    pub fn state_chip(&self, glyph: &'static Icon, label: &str, tone: Tone) -> El {
        crate::motion::colors(row())
            .h(CONTROL_S)
            .px(8.0)
            .gap(6.0)
            .center_y()
            .radius(r(4.0))
            .fill(self.over.alpha(0.06))
            .label(label)
            .child(icon(glyph, 12.0).color(match tone {
                Tone::Quiet => self.muted,
                other => other.fg(self),
            }).a11y_hidden())
            .child(
                text(label.to_string())
                    .font(LABEL)
                    .color(self.prose)
                    .truncate()
                    .a11y_hidden(),
            )
    }

    /// A one line result for a status row: a 14 point icon and `SMALL` `prose` text, cut with
    /// the whole line in a tooltip.
    pub fn status_line(&self, glyph: &'static Icon, line: &str, tone: Tone) -> El {
        row()
            .h(CONTROL_S)
            .gap(6.0)
            .center_y()
            .min_w(0.0)
            .shrink()
            .child(icon(glyph, 14.0).color(match tone {
                Tone::Quiet => self.muted,
                other => other.fg(self),
            }).a11y_hidden())
            .child(
                text(line.to_string())
                    .font(SMALL)
                    .color(self.prose)
                    .truncate()
                    .help_when_truncated(line.to_string()),
            )
    }

    /// The bottom selection bar: an overlay centred on the content, 16 above its bottom edge,
    /// 48 high, at most 720 wide. `lead` is the close button, the count and "Select all";
    /// `actions` sit on the right. It floats, so the list under it never moves. The page keeps
    /// 72 points of bottom padding so its last row never hides under it.
    pub fn selection_bar(&self, key: &str, lead: Vec<El>, actions: Vec<El>) -> El {
        let bar = row()
            .key(format!("{key}-bar"))
            .role("toolbar")
            .w_full()
            .max_w(720.0)
            .h(48.0)
            .px(12.0)
            .gap(8.0)
            .center_y()
            .radius(r(8.0))
            .fill(self.raised)
            .border(1.0, self.hair_strong)
            .shadow(4.0, 12.0, Color::BLACK.alpha(if self.light { 0.12 } else { 0.35 }))
            .children(lead)
            .child(spacer())
            .children(actions);
        row()
            .key(key.to_string())
            .pin(Some(24.0), None, Some(24.0), Some(16.0))
            .justify_center()
            .transition(crate::motion::lift())
            .child(bar)
    }

    /// A detail drawer over the right of the content: 360 wide, full height, raised, a 1 point
    /// line on its left, its own scroll. The list under it keeps its width. `close` is the
    /// page's close button (28, it owns the press), pinned 12 from the top right: leave 32
    /// points free at the right of the body's first row. Esc is the page's to handle.
    pub fn drawer(&self, key: &str, close: El, body: El) -> El {
        col()
            .key(key.to_string())
            .role("dialog")
            .pin(None, Some(0.0), Some(0.0), Some(0.0))
            .w(360.0)
            .fill(self.raised)
            .border_sides([0.0, 0.0, 0.0, 1.0], self.hair)
            .shadow(4.0, 12.0, Color::BLACK.alpha(if self.light { 0.12 } else { 0.35 }))
            .transition(crate::motion::from_right())
            .child(
                el().pin(None, Some(12.0), Some(12.0), None)
                    .key(format!("{key}-close"))
                    .child(close),
            )
            .child(
                self.scroll_area(
                    col()
                        .key(format!("{key}-scroll"))
                        .w_full()
                        .grow(1.0)
                        .min_h(0.0)
                        .scroll(rune::ScrollSpec { stick_bottom: false })
                        .child(col().w_full().padding(16.0).gap(8.0).child(body)),
                ),
            )
    }

    /// A 28 point close button for drawers and popovers.
    pub fn close_button(&self, label: &str) -> El {
        self.icon_button(&crate::icons::X, label).size(CONTROL_M, CONTROL_M)
    }

    /// One action in a choice dialog, as a row: 48 high, title over one line, chevron. Put
    /// several in a `panel`. Long names truncate, so they never overflow the dialog.
    pub fn option_row(&self, key: &str, title: &str, line: &str) -> El {
        row()
            .key(key.to_string())
            .w_full()
            .h(48.0)
            .px(12.0)
            .gap(12.0)
            .center_y()
            .hover(self.hover)
            .press(self.over.alpha(0.08))
            .focusable(true)
            .role("button")
            .label(title)
            .cursor(rune::Cursor::Pointer)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(title.to_string()).font(BODY_MED).color(self.ink).truncate())
                    .child(text(line.to_string()).font(SMALL).color(self.muted).truncate()),
            )
            .child(icon(&crate::icons::CHEVRON_RIGHT, 12.0).color(self.muted).a11y_hidden())
    }

    /// The settings row: title (`BODY_MED` ink) and a description (`SMALL` muted, at most two
    /// lines, 400 wide) on the left, the control on the right. 56 high with one line, 72 with
    /// two: the copy sets the height, never the state. One bottom hairline.
    pub fn setting_row(&self, key: &str, title: &str, description: &str, control: El) -> El {
        let mut words = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(text(title.to_string()).font(BODY_MED).color(self.ink).truncate());
        if !description.is_empty() {
            words = words.child(
                text(description.to_string())
                    .font(SMALL)
                    .color(self.muted)
                    .max_w(400.0)
                    .max_lines(2),
            );
        }
        row()
            .key(key.to_string())
            .w_full()
            .min_h(56.0)
            .py(8.0)
            .gap(16.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], self.hair)
            .child(words)
            .child(control)
    }

    /// A dense 32 point row: label (`BODY` `prose`) on the left, value or control on the
    /// right. `label_w` fixes the label column when a slider follows (0 lets it flex).
    pub fn compact_row(&self, key: &str, label: &str, label_w: f32, control: El) -> El {
        let label = text(label.to_string()).font(BODY).color(self.prose).truncate();
        let mut line = row().key(key.to_string()).w_full().h(CONTROL_L).gap(12.0).center_y();
        line = if label_w > 0.0 {
            line.child(label.w(label_w)).child(control.grow(1.0).min_w(0.0))
        } else {
            line.child(label.grow(1.0).min_w(0.0)).child(control)
        };
        line
    }

    /// A row whose control sits full width under the title and description (swatches, tiles,
    /// multi-line fields, a path with its actions).
    pub fn stacked_row(&self, key: &str, title: &str, description: &str, control: El) -> El {
        let mut c = col()
            .key(key.to_string())
            .w_full()
            .gap(2.0)
            .pt(12.0)
            .pb(16.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], self.hair)
            .child(text(title.to_string()).font(BODY_MED).color(self.ink));
        if !description.is_empty() {
            c = c.child(
                text(description.to_string())
                    .font(SMALL)
                    .color(self.muted)
                    .max_w(400.0)
                    .max_lines(2),
            );
        }
        c.child(row().w_full().mt(6.0).child(control))
    }

    /// A select's closed box: 28 high, `well` fill, 1 point line, the value truncating and a
    /// 12 point chevron. The page adds the press that opens its menu. Default width 200.
    pub fn select_box(&self, value: &str, width: f32) -> El {
        row()
            .w(width)
            .h(CONTROL_M)
            .pl(10.0)
            .pr(8.0)
            .gap(8.0)
            .center_y()
            .radius(r(4.0))
            .fill(self.well)
            .border(1.0, self.hair)
            .hover(self.over.alpha(0.07))
            .focusable(true)
            .role("button")
            .label(value)
            .cursor(rune::Cursor::Pointer)
            .child(
                text(value.to_string())
                    .font(BODY_MED)
                    .color(self.ink)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0)
                    .a11y_hidden(),
            )
            .child(icon(&crate::icons::CHEVRON_DOWN, 12.0).color(self.muted).a11y_hidden())
    }

    /// A "..." menu: a raised list of `menu_item` rows, anchored under (or beside) the keyed
    /// button that opened it. Mount it at the root with `popover_layer`.
    pub fn menu(&self, key: &str, anchor: &str, width: f32) -> El {
        col()
            .key(key.to_string())
            .role("menu")
            .w(width)
            .padding(4.0)
            .radius(r(8.0))
            .fill(self.raised.mix(self.over, 0.04).alpha(1.0))
            .border(1.0, self.hair_strong)
            .shadow(4.0, 12.0, Color::BLACK.alpha(if self.light { 0.12 } else { 0.35 }))
            .anchored_fit(anchor.to_string(), rune::Side::Bottom, rune::Align::End, 4.0)
            .transition(crate::motion::pop(false))
    }

    /// One menu row: 32 high, a 14 point icon, the label. `danger` uses `rose` text.
    pub fn menu_item(&self, key: &str, glyph: &'static Icon, label: &str, danger: bool) -> El {
        let ink = if danger { self.rose } else { self.prose };
        row()
            .key(key.to_string())
            .w_full()
            .h(CONTROL_L)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(r(4.0))
            .hover(self.hover)
            .focusable(true)
            .role("menuitem")
            .label(label)
            .cursor(rune::Cursor::Pointer)
            .child(icon(glyph, 14.0).color(if danger { self.rose } else { self.muted }).a11y_hidden())
            .child(text(label.to_string()).font(BODY).color(ink).truncate().a11y_hidden())
    }

    /// A full-window layer for a menu or popover: a transparent `scrim` (the page gives it the
    /// press that closes) under the menu.
    pub fn popover_layer(&self, key: &str, scrim: El, menu: El) -> El {
        // The layer has no entrance and its scrim none at all: the scrim takes the closing press
        // on the first frame, and the menu plays its own `pop`.
        el().key(key.to_string())
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .transition(crate::motion::layer())
            .child(scrim.absolute(0.0, 0.0).w_full().h_full().transition(crate::motion::cut()))
            .child(menu)
    }

    /// The 28 point "..." button that opens a row's menu.
    pub fn more_button(&self, label: &str) -> El {
        self.icon_button(&crate::icons::ELLIPSIS, label).size(CONTROL_M, CONTROL_M)
    }

    /// A small colour mark (8 by 8, square) before a state word: "Equipped", "Active".
    pub fn mark(&self, color: Color) -> El {
        el().size(8.0, 8.0).radius(2.0).fill(color).a11y_hidden()
    }
}

/// `ENABLED` becomes `Enabled`, `catalog error` stays as written. Uppercase micro labels are
/// banned in the house style, so old call sites are lowered here.
pub fn sentence(s: &str) -> String {
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        let lower = s.to_lowercase();
        let mut chars = lower.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => lower,
        }
    } else {
        s.to_string()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Bad,
    Info,
    Quiet,
    Accent,
}

impl Tone {
    pub fn fg(self, u: &Ui) -> Color {
        match self {
            Tone::Ok => u.teal,
            Tone::Warn => u.amber,
            Tone::Bad => u.rose,
            Tone::Info => u.violet,
            Tone::Quiet => u.faint,
            Tone::Accent => u.accent,
        }
    }

    pub fn bg(self, u: &Ui) -> Color {
        self.fg(u).alpha(0.11)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Primary,
    Secondary,
    Ghost,
    /// Removes something the user made. Rose text on a hairline, never ember.
    Danger,
}

impl Compat {
    pub fn tone(self) -> Tone {
        match self {
            Compat::Unchanged => Tone::Ok,
            Compat::Changed => Tone::Warn,
            Compat::Missing => Tone::Bad,
            Compat::Moved => Tone::Info,
        }
    }
}

impl ToneKey {
    pub fn tone(self) -> Tone {
        match self {
            ToneKey::Ok => Tone::Ok,
            ToneKey::Warn => Tone::Warn,
            ToneKey::Info => Tone::Info,
            ToneKey::Quiet => Tone::Quiet,
        }
    }
}

/// The look, density and language are process-wide, and tests run in parallel: every test
/// that sets them, or lays a page out and measures it, holds this lock so none interleave.
#[cfg(test)]
pub(crate) fn test_globals() -> std::sync::MutexGuard<'static, ()> {
    static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod look_tests {
    use super::*;

    fn same(a: &Ui, b: &Ui) -> bool {
        let fields = |u: &Ui| {
            [
                u.bg, u.sidebar, u.raised, u.well, u.hair, u.hair_strong, u.ink, u.prose, u.muted,
                u.faint, u.accent, u.accent_ink, u.teal, u.rose, u.amber, u.violet, u.shadow,
                u.field, u.hover, u.selected, u.over, u.scrim,
            ]
        };
        fields(a) == fields(b) && a.light == b.light
    }

    /// Graphite is neutral, and its text clears AA on every layer it sits on.
    #[test]
    fn graphite_is_neutral_and_readable() {
        let u = Ui::themed(Theme::Graphite, None);
        for layer in [u.bg, u.sidebar, u.raised, u.well] {
            assert!((layer.r - layer.g).abs() < 0.004 && (layer.g - layer.b).abs() < 0.004);
            for ink in [u.ink, u.prose, u.muted, u.faint] {
                assert!(contrast(ink, layer) >= AA_TEXT, "{} on {}", hex6(ink), hex6(layer));
            }
        }
    }

    /// The knob reads on both tracks, and the green reads on every theme's and preset's layers.
    #[test]
    fn switches_read_on_every_surface() {
        let mut looks: Vec<Ui> = Theme::ALL.into_iter().map(|t| Ui::themed(t, None)).collect();
        looks.extend(PRESETS.iter().map(|p| p.palette().to_ui()));
        for u in &looks {
            assert!(contrast(switch_knob(), switch_green()) >= 3.0);
            assert!(contrast(switch_knob(), u.switch_off()) >= 3.0);
            for layer in [u.bg, u.sidebar, u.raised, u.well] {
                let layer = layer.alpha(1.0);
                assert!(contrast(switch_green(), layer) >= 3.0, "green on {}", hex6(layer));
            }
        }
    }

    /// Rune has Inter at 400, 500 and 600 only; any other weight draws in the next heavier face.
    #[test]
    fn shared_weights_are_real_faces() {
        for style in STYLES {
            assert!(
                [400.0, 500.0, 600.0].contains(&style.weight),
                "weight {} has no face",
                style.weight
            );
        }
    }

    /// Muted and faint text clear AA on every layer of every dark theme.
    #[test]
    fn secondary_text_reads_on_every_layer() {
        for theme in Theme::ALL {
            let u = Ui::themed(theme, None);
            for layer in [u.bg, u.sidebar, u.raised, u.well] {
                for ink in [u.muted, u.faint] {
                    let ratio = contrast(ink.alpha(1.0), layer.alpha(1.0));
                    assert!(ratio >= AA_TEXT, "{theme:?} {} on {}: {ratio}", hex6(ink), hex6(layer));
                }
            }
        }
    }

    #[test]
    fn block_corners_sit_on_two_steps() {
        let _look = test_globals();
        set_personal(None, BLOCK_RADIUS);
        for points in [2.0, 4.0, 6.0] {
            assert_eq!(r(points), 2.0, "{points}");
        }
        for points in [8.0, 10.0, 14.0, 22.0] {
            assert_eq!(r(points), 4.0, "{points}");
        }
        assert_eq!(r(0.0), 0.0);
        set_personal(None, 1.0);
    }

    /// With Personalize untouched, `Ui::new` paints exactly the theme it always did and every
    /// radius keeps its number.
    #[test]
    fn default_look_is_unchanged() {
        let _look = test_globals();
        set_personal(None, 1.0);
        for theme in Theme::ALL {
            set_look(theme, None);
            assert!(same(&Ui::new(), &Ui::themed(theme, None)), "{theme:?}");
        }
        set_look(Theme::Dark, None);
        assert_eq!(r(8.0), 8.0);
        assert_eq!(r(14.0), 14.0);
        let p = crate::ui::preset("cobalt").unwrap().palette();
        set_personal(Some(p), 0.5);
        assert_eq!(Ui::new().bg, p.background);
        assert_eq!(r(8.0), 4.0);
        set_personal(None, 1.0);
        assert!(same(&Ui::new(), &Ui::themed(Theme::Dark, None)));
    }
}

#[cfg(test)]
mod inset_tests {
    use super::*;

    #[test]
    fn standard_and_spacious_keep_every_inset_as_written() {
        let _g = test_globals();
        for z in [1.0, 1.1, 1.25] {
            set_zoom(z);
            for p in [2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0] {
                assert_eq!(inset_x(p), p);
                assert_eq!(inset_y(p), p);
                assert_eq!(gutter(p), p);
                assert_eq!(standard(p), p);
            }
        }
        set_zoom(1.0);
    }

    #[test]
    fn compact_insets_stop_at_their_floors() {
        let _g = test_globals();
        for z in [MIN_ZOOM, 0.95] {
            set_zoom(z);
            assert!(inset_x(8.0) * z >= INSET_X_MIN - 0.001);
            assert!(inset_y(6.0) * z >= INSET_Y_MIN - 0.001);
            // An inset already under its floor at Standard keeps its Standard size, no more.
            assert!((inset_x(6.0) * z - 6.0).abs() < 0.001);
            // Larger insets still scale with the density.
            assert_eq!(inset_x(16.0), 16.0);
            assert!((standard(16.0) * z - 16.0).abs() < 0.001);
        }
        set_zoom(1.0);
    }

    #[test]
    fn page_and_sidebar_gutters_clear_the_floor_at_the_smallest_zoom() {
        // The page gutter and the sidebar's icon inset (the nav's 8 plus a row's 10) scale with
        // the zoom; the zoom's clamp keeps them over the 12 point floor.
        assert!(PAGE_X * MIN_ZOOM >= GUTTER_MIN);
        assert!((8.0 + 10.0) * MIN_ZOOM >= GUTTER_MIN);
    }
}
