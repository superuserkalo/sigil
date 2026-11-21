//! Design tokens for Sigil: one palette, one type scale, one set of flat primitives.
//!
//! The look is restraint: near black layers, hairlines instead of boxes, one ember accent that
//! only marks a primary action, an on state or focus. Every size sits on a 4 point grid.

use rune::{Color, El, Family, Icon, TextStyle, col, el, icon, oklch, row, rule, spacer, text};

use crate::model::{Compat, ToneKey};

pub mod personalize;

pub const SIDEBAR_W: f32 = 232.0;
pub const RAIL_W: f32 = 304.0;
pub const TITLEBAR_H: f32 = 40.0;
pub const HEADER_H: f32 = 40.0;
pub const STATUS_H: f32 = 0.0;
pub const PAGE_X: f32 = 32.0;
pub const PAGE_TOP: f32 = 24.0;

pub const HERO: TextStyle = TextStyle::new(28.0, 600.0).lh(34.0).tracking(-0.4);
pub const DISPLAY: TextStyle = TextStyle::new(22.0, 600.0).lh(28.0).tracking(-0.25);
pub const H1: TextStyle = TextStyle::new(17.0, 600.0).lh(24.0);
pub const H2: TextStyle = TextStyle::new(14.0, 590.0).lh(20.0);
pub const LEAD: TextStyle = TextStyle::new(14.0, 420.0).lh(22.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 420.0).lh(20.0);
pub const CHAT: TextStyle = TextStyle::new(14.0, 420.0).lh(23.0);
pub const BODY_MED: TextStyle = TextStyle::new(13.0, 540.0).lh(20.0);
pub const SMALL: TextStyle = TextStyle::new(12.0, 430.0).lh(16.0);
pub const LABEL: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
pub const NAV: TextStyle = TextStyle::new(13.0, 500.0).lh(20.0);
pub const WORDMARK: TextStyle = TextStyle::new(15.0, 640.0).lh(20.0).tracking(-0.2);
pub const MICRO: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0).features("'tnum' 1");
pub const MONO: TextStyle = TextStyle::new(12.0, 430.0)
    .lh(18.0)
    .family(Family::SystemMono);
pub const MONO_MED: TextStyle = TextStyle::new(12.0, 540.0)
    .lh(18.0)
    .family(Family::SystemMono);
pub const NUM: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).features("'tnum' 1");

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

/// The house corner: the Block choice in Personalize. Radii scale down to at most 2 points.
pub const BLOCK_RADIUS: f32 = 0.2;
const BLOCK_MAX: f32 = 2.0;

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
    }
}

pub fn look() -> Look {
    LOOK.lock().map(|look| *look).unwrap_or(Look::DEFAULT)
}

/// Personalize's density, as a scale on the whole interface. Set with the appearance pass.
pub fn set_density(scale: f32) {
    if let Ok(mut look) = LOOK.lock() {
        look.density = if scale.is_finite() { scale.clamp(0.8, 1.2) } else { 1.0 };
    }
}

pub fn density() -> f32 {
    look().density
}

/// A corner radius on the house grid, scaled by the Personalize radius choice. Block, the
/// default, keeps every corner at 2 points or less; Soft (1.0) is exact.
pub fn r(points: f32) -> f32 {
    let k = look().radius;
    if k == 1.0 {
        points
    } else if k == BLOCK_RADIUS {
        (points * k).round().min(BLOCK_MAX)
    } else {
        (points * k).round()
    }
}

/// A floating layer's shadow colour. Graphite separates layers with hard borders, so its
/// shadow is off; every other look keeps the colour it asked for.
pub fn soft(color: Color) -> Color {
    let look = look();
    if look.custom.is_none() && look.theme == Theme::Graphite { Color::TRANSPARENT } else { color }
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
        u.prose = oklch(0.87, chroma.min(0.012), hue);
        u.muted = oklch(0.71, (chroma * 1.5).clamp(0.008, 0.03), hue);
        u.faint = oklch(0.55, (chroma * 1.5).clamp(0.008, 0.03), hue);
        u
    }

    /// The house look: neutral graphite in clear tonal steps (well, sidebar, page, raised),
    /// hard lines instead of shadows, and text that clears AA on every layer.
    fn graphite() -> Ui {
        let mut u = Ui::dark_variant(0.235, 0.205, 0.285, 0.19, 0.0, 0.0);
        u.hair = Color::WHITE.alpha(0.11);
        u.hair_strong = Color::WHITE.alpha(0.19);
        u.ink = oklch(0.97, 0.0, 0.0);
        u.prose = oklch(0.88, 0.0, 0.0);
        u.muted = oklch(0.77, 0.0, 0.0);
        u.faint = oklch(0.70, 0.0, 0.0);
        u.hover = Color::WHITE.alpha(0.06);
        u.selected = Color::WHITE.alpha(0.11);
        u.shadow = Color::TRANSPARENT;
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
            faint: oklch(0.6, 0.01, 85.0),
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
            muted: oklch(0.71, 0.012, 286.0),
            faint: oklch(0.55, 0.012, 286.0),
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

    /// A quiet section label in sentence case. Kept for old call sites; prefer `section`.
    pub fn eyebrow(&self, s: impl Into<String>) -> El {
        text(sentence(&s.into())).font(LABEL).color(self.faint)
    }

    /// A section heading inside a page: one weight step up from body, no rule, no box.
    pub fn section(&self, title: &str) -> El {
        text(title.to_string()).font(H2).color(self.ink)
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

    /// A full-window modal layer: a flat scrim and the dialog centred over the shell.
    pub fn modal_layer(&self, key: &str, inner: El) -> El {
        let scrim = el()
            .key(format!("{key}-scrim"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(self.scrim)
            .a11y_hidden();
        el().key(key)
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(scrim)
            .child(el().h_full().w_full().padding(32.0).center().child(inner))
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
            .h(32.0)
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

    /// A button with a leading icon.
    pub fn icon_action(&self, glyph: &'static Icon, label: &str, kind: ActionKind) -> El {
        let (fill, ink, border) = self.action_colors(kind);
        row()
            .h(32.0)
            .pl(10.0)
            .pr(12.0)
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
            .child(icon(glyph, 16.0).color(ink).a11y_hidden())
            .child(
                text(label.to_string())
                    .font(BODY_MED)
                    .color(ink)
                    .a11y_hidden(),
            )
    }

    /// A square 32 point icon button, labelled for assistive tech.
    pub fn icon_button(&self, glyph: &'static Icon, label: &str) -> El {
        el().size(32.0, 32.0)
            .center()
            .radius(r(8.0))
            .hover(self.hover)
            .press(self.over.alpha(0.08))
            .focusable(true)
            .role("button")
            .label(label)
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

    /// The switch: 36 by 20, ember when on, a white knob either way.
    pub fn toggle(&self, on: bool) -> El {
        let (w, h, k) = (36.0, 20.0, 16.0);
        let inset = (h - k) / 2.0;
        el().key(if on { "toggle-on" } else { "toggle-off" })
            .role("switch")
            .checked(on)
            .focusable(true)
            .size(w, h)
            .radius(h / 2.0)
            .fill(if on {
                self.accent
            } else {
                self.over.alpha(if self.light { 0.16 } else { 0.14 })
            })
            .child(
                el().size(k, k)
                    .absolute(if on { w - k - inset } else { inset }, inset)
                    .radius(k / 2.0)
                    .fill(Color::WHITE)
                    .a11y_hidden(),
            )
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

    /// An empty state: centred title and one line, no box, no icon tile.
    pub fn empty_state(&self, _glyph: &str, title: &str, detail: &str, action: Option<El>) -> El {
        let mut c = col()
            .w_full()
            .py(64.0)
            .gap(6.0)
            .items_center()
            .child(
                text(title.to_string())
                    .font(H1)
                    .color(self.ink)
                    .align(rune::Align::Center),
            )
            .child(
                text(detail.to_string())
                    .font(BODY)
                    .color(self.muted)
                    .max_w(440.0)
                    .wrap()
                    .align(rune::Align::Center),
            );
        if let Some(a) = action {
            c = c.child(el().h(10.0)).child(a);
        }
        c
    }

    /// The page header: title, an optional one line description, actions on the right, and a
    /// hairline under it. The kicker argument is ignored.
    pub fn page_head(&self, _kicker: &str, title: &str, detail: &str, trailing: Option<El>) -> El {
        let mut left = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(4.0)
            .child(text(title.to_string()).font(DISPLAY).color(self.ink));
        if !detail.is_empty() {
            left = left.child(
                text(detail.to_string())
                    .font(BODY)
                    .color(self.muted)
                    .truncate(),
            );
        }
        let mut r = row().w_full().gap(16.0).child(left);
        r = if detail.is_empty() {
            r.items_center()
        } else {
            r.items_end()
        };
        if let Some(t) = trailing {
            r = r.child(t);
        }
        col().w_full().gap(16.0).child(r).child(self.divider())
    }

    pub fn panel_header(&self, title: &str, meta: &str) -> El {
        row()
            .w_full()
            .h(40.0)
            .px(16.0)
            .gap(12.0)
            .items_center()
            .border_sides([0.0, 0.0, 1.0, 0.0], self.hair)
            .child(text(sentence(title)).font(BODY_MED).color(self.ink))
            .child(spacer())
            .child(text(meta.to_string()).font(SMALL).color(self.faint))
    }

    pub fn table_head(&self, cols: &[(&str, f32)]) -> El {
        let mut r = row().w_full().px(16.0).h(32.0).items_center().gap(12.0);
        for (label, w) in cols {
            let e = text(sentence(label)).font(SMALL).color(self.faint);
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

    /// A collapsible-style group heading above a grid: `Enabled (6)`.
    pub fn grid_head(&self, label: &str, meta: &str) -> El {
        let title = if meta.is_empty() {
            sentence(label)
        } else {
            format!(
                "{} ({})",
                sentence(label),
                meta.split_whitespace().next().unwrap_or(meta)
            )
        };
        row()
            .w_full()
            .h(28.0)
            .gap(8.0)
            .center_y()
            .child(text(title).font(LABEL).color(self.muted))
    }

    /// The toolbar search field. The field itself comes from `rune::text_input` so each screen
    /// owns its own `TextEdit`.
    pub fn search_shell(&self, field: El) -> El {
        row()
            .h(32.0)
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

    #[test]
    fn block_corners_stay_at_two_points() {
        set_personal(None, BLOCK_RADIUS);
        for points in [2.0, 4.0, 8.0, 10.0, 14.0, 22.0] {
            assert!(r(points) <= 2.0, "{points}");
        }
        set_personal(None, 1.0);
    }

    /// With Personalize untouched, `Ui::new` paints exactly the theme it always did and every
    /// radius keeps its number.
    #[test]
    fn default_look_is_unchanged() {
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
