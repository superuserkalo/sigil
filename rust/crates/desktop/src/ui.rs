//! Design tokens for the workbench: one palette, one type scale, one set of primitives.
//!
//! Rune's `tok` palette is tuned for neutral app chrome. The workbench keeps its semantic
//! colours and adds a small Deadlock-flavoured layer on top: ember gold for action, teal for
//! verified state, rose for blocked state. Everything is built from `rune::oklch` so the
//! palette stays perceptually even.

use rune::tok::motion;
use rune::{Color, El, Family, Shadow, TextStyle, col, el, oklch, row, rule, spacer, text};

use crate::model::{Compat, ToneKey};

pub const SIDEBAR_W: f32 = 240.0;
pub const RAIL_W: f32 = 304.0;
pub const HEADER_H: f32 = 58.0;
pub const STATUS_H: f32 = 27.0;

pub const HERO: TextStyle = TextStyle::new(31.0, 640.0).lh(37.0);
pub const DISPLAY: TextStyle = TextStyle::new(25.0, 640.0).lh(31.0);
pub const H1: TextStyle = TextStyle::new(19.0, 600.0).lh(25.0);
pub const H2: TextStyle = TextStyle::new(14.5, 590.0).lh(20.0);
pub const LEAD: TextStyle = TextStyle::new(15.0, 440.0).lh(23.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 420.0).lh(20.0);
pub const CHAT: TextStyle = TextStyle::new(14.0, 420.0).lh(23.0);
pub const BODY_MED: TextStyle = TextStyle::new(13.0, 540.0).lh(20.0);
pub const SMALL: TextStyle = TextStyle::new(11.5, 440.0).lh(17.0);
pub const LABEL: TextStyle = TextStyle::new(10.0, 620.0).lh(13.0).tracking(0.55);
pub const MONO: TextStyle = TextStyle::new(11.0, 440.0)
    .lh(16.0)
    .family(Family::SystemMono);
pub const MONO_MED: TextStyle = TextStyle::new(11.5, 540.0)
    .lh(16.0)
    .family(Family::SystemMono);
pub const NUM: TextStyle = TextStyle::new(18.0, 620.0).lh(22.0).features("'tnum' 1");

#[derive(Clone, Copy)]
pub struct Ui {
    pub bg: Color,
    pub sidebar: Color,
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
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Ui {
        Ui {
            bg: oklch(0.112, 0.008, 268.0),
            sidebar: oklch(0.094, 0.007, 268.0),
            well: oklch(0.138, 0.007, 266.0),
            hair: Color::WHITE.alpha(0.062),
            hair_strong: Color::WHITE.alpha(0.11),
            ink: oklch(0.962, 0.004, 80.0),
            prose: oklch(0.875, 0.005, 80.0),
            muted: oklch(0.70, 0.008, 260.0),
            faint: oklch(0.515, 0.008, 260.0),
            accent: oklch(0.805, 0.135, 76.0),
            accent_ink: oklch(0.19, 0.035, 70.0),
            teal: oklch(0.76, 0.095, 190.0),
            rose: oklch(0.70, 0.155, 22.0),
            amber: oklch(0.835, 0.13, 72.0),
            violet: oklch(0.73, 0.10, 300.0),
            shadow: Color::BLACK.alpha(0.42),
            glass_top: Color::WHITE.alpha(0.055),
            glass_bottom: Color::WHITE.alpha(0.018),
            sheen: Color::WHITE.alpha(0.13),
            glow_cool: oklch(0.52, 0.075, 258.0),
            glow_warm: oklch(0.52, 0.09, 78.0),
            field: oklch(0.07, 0.006, 268.0),
        }
    }

    pub fn eyebrow(&self, s: impl Into<String>) -> El {
        text(s).font(LABEL).color(self.faint)
    }

    pub fn panel(&self) -> El {
        col()
            .fill(Color::WHITE.alpha(0.032))
            .radius(14.0)
            .border(1.0, Color::WHITE.alpha(0.055))
            .box_shadow(Shadow {
                y: 1.0,
                blur: 0.0,
                spread: 0.0,
                color: Color::WHITE.alpha(0.045),
                inset: true,
                ..Shadow::default()
            })
    }

    /// A radial glow field painted behind content. Mount it first inside a
    /// full-height column so later children paint above it.
    pub fn glow(&self, key: &str, center: [f32; 2], radius: (f32, f32), color: Color) -> El {
        el().key(key)
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .radial_gradient(
                center,
                radius,
                [
                    (0.0, color.alpha(0.26)),
                    (0.42, color.alpha(0.10)),
                    (1.0, Color::TRANSPARENT),
                ],
            )
            .a11y_hidden()
    }

    /// The outer shell of a double bezel: a hairline lit from above, a deep
    /// translucent fill and one soft shadow.
    pub fn glass_shell(&self) -> El {
        col()
            .fill(Color::WHITE.alpha(0.038))
            .backdrop_blur(16.0)
            .radius(22.0)
            .border_gradient(1.0, self.sheen, Color::WHITE.alpha(0.025))
            .shadow(26.0, 64.0, Color::BLACK.alpha(0.46))
    }

    /// The inner core of a double bezel, one radius step in.
    pub fn glass_core(&self) -> El {
        col()
            .fill(Color::WHITE.alpha(0.021))
            .gradient(self.glass_top, self.glass_bottom)
            .radius(20.0)
            .border(1.0, Color::WHITE.alpha(0.045))
            .box_shadow(Shadow {
                y: 1.0,
                blur: 10.0,
                spread: 0.0,
                color: Color::BLACK.alpha(0.18),
                inset: true,
                ..Shadow::default()
            })
    }

    /// A modal shell: the same double bezel, but opaque enough that the page
    /// behind it never competes with the dialog copy.
    pub fn dialog_shell(&self) -> El {
        col()
            .fill(self.bg)
            .radius(22.0)
            .border_gradient(1.0, self.sheen, Color::WHITE.alpha(0.03))
            .shadow(34.0, 90.0, Color::BLACK.alpha(0.62))
    }

    pub fn dialog_core(&self) -> El {
        col()
            .fill(Color::WHITE.alpha(0.022))
            .gradient(self.glass_top, self.glass_bottom)
            .radius(20.0)
            .border(1.0, Color::WHITE.alpha(0.05))
            .box_shadow(Shadow {
                y: 1.0,
                blur: 12.0,
                spread: 0.0,
                color: Color::BLACK.alpha(0.22),
                inset: true,
                ..Shadow::default()
            })
    }

    /// A full-window modal layer: scrim, a soft accent bloom and the dialog
    /// centred over the shell. Mount it last on the root so it overlays.
    pub fn modal_layer(&self, key: &str, inner: El) -> El {
        let scrim = el()
            .key(format!("{key}-scrim"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(Color::BLACK.alpha(0.68))
            .radial_gradient(
                [0.5, 0.28],
                [760.0, 520.0],
                [(0.0, self.accent.alpha(0.055)), (1.0, Color::TRANSPARENT)],
            )
            .a11y_hidden();
        el().key(key)
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(scrim)
            .child(el().h_full().w_full().padding(34.0).center().child(inner))
    }

    /// A quiet inset field for a path, hash or prompt.
    pub fn well_field(&self) -> El {
        col()
            .fill(self.field)
            .radius(12.0)
            .border(1.0, Color::WHITE.alpha(0.045))
            .box_shadow(Shadow {
                y: 1.0,
                blur: 0.0,
                spread: 0.0,
                color: Color::WHITE.alpha(0.035),
                inset: true,
                ..Shadow::default()
            })
    }

    /// A rounded action pill with a nested trailing glyph circle. Primary is
    /// the one strongest action on a screen; cancel stays quiet.
    pub fn pill(&self, label: &str, glyph: &str, kind: ActionKind) -> El {
        let (fill, ink, border, badge) = match kind {
            ActionKind::Primary => (
                self.accent,
                self.accent_ink,
                Color::TRANSPARENT,
                self.accent_ink.alpha(0.16),
            ),
            ActionKind::Secondary => (
                Color::WHITE.alpha(0.055),
                self.ink,
                Color::WHITE.alpha(0.09),
                Color::WHITE.alpha(0.07),
            ),
            ActionKind::Ghost => (
                Color::TRANSPARENT,
                self.muted,
                Color::TRANSPARENT,
                Color::WHITE.alpha(0.045),
            ),
        };
        row()
            .h(38.0)
            .pl(16.0)
            .pr(6.0)
            .gap(10.0)
            .center_y()
            .radius(19.0)
            .fill(fill)
            .border(1.0, border)
            .hover(Color::WHITE.alpha(if kind == ActionKind::Primary {
                0.10
            } else {
                0.05
            }))
            .press(Color::WHITE.alpha(0.10))
            .press_scale(0.972)
            .focusable(true)
            .role("button")
            .label(label)
            .motion(motion::SNAPPY)
            .child(text(label).font(BODY_MED).color(ink).a11y_hidden())
            .child(
                el().size(26.0, 26.0)
                    .radius(13.0)
                    .center()
                    .fill(badge)
                    .child(text(glyph).font(SMALL).color(ink).a11y_hidden())
                    .a11y_hidden(),
            )
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

    pub fn glyph_box(&self, glyph: &str, tone: Tone, size: f32) -> El {
        let glyph_size = (size * 0.56).clamp(12.0, 20.0);
        el().size(size, size)
            .radius(size * 0.28)
            .center()
            .fill(tone.bg(self))
            .border(1.0, tone.fg(self).alpha(0.22))
            .child(
                text(glyph)
                    .font(TextStyle::new(glyph_size, 620.0))
                    .color(tone.fg(self))
                    .a11y_hidden(),
            )
            .a11y_hidden()
    }

    pub fn stat(&self, value: impl Into<String>, label: &str, tone: Tone) -> El {
        col()
            .grow(1.0)
            .gap(4.0)
            .padding(12.0)
            .radius(8.0)
            .fill(self.well)
            .border(1.0, self.hair)
            .child(text(value).font(NUM).color(tone.fg(self)))
            .child(text(label).font(LABEL).color(self.faint))
    }

    pub fn chip(&self, label: &str, fg: Color, bg: Color) -> El {
        text(label)
            .font(LABEL)
            .color(fg)
            .padding(7.0)
            .pt(3.0)
            .pb(3.0)
            .radius(4.0)
            .fill(bg)
            .a11y_hidden()
    }

    pub fn status_chip(&self, label: &str, tone: Tone) -> El {
        self.chip(label, tone.fg(self), tone.bg(self))
    }

    pub fn compat_chip(&self, c: Compat) -> El {
        self.status_chip(c.label(), c.tone())
    }

    pub fn action(&self, label: &str, kind: ActionKind) -> El {
        let (fill, ink, border) = self.action_colors(kind);
        row()
            .h(32.0)
            .px(13.0)
            .gap(6.0)
            .center()
            .radius(16.0)
            .fill(fill)
            .border(1.0, border)
            .hover(Color::WHITE.alpha(0.05))
            .press(Color::WHITE.alpha(0.08))
            .press_scale(0.98)
            .focusable(true)
            .role("button")
            .label(label)
            .motion(motion::SNAPPY)
            .child(text(label).font(LABEL).color(ink).a11y_hidden())
    }

    fn action_colors(&self, kind: ActionKind) -> (Color, Color, Color) {
        match kind {
            ActionKind::Primary => (self.accent, self.accent_ink, Color::TRANSPARENT),
            ActionKind::Secondary => (
                Color::WHITE.alpha(0.045),
                self.ink,
                Color::WHITE.alpha(0.075),
            ),
            ActionKind::Ghost => (Color::TRANSPARENT, self.muted, Color::TRANSPARENT),
        }
    }

    pub fn toggle(&self, on: bool) -> El {
        let (w, h, k) = (34.0, 20.0, 16.0);
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
                Color::WHITE.alpha(0.12)
            })
            .motion(motion::SNAPPY)
            .child(
                el().size(k, k)
                    .absolute(if on { w - k - inset } else { inset }, inset)
                    .radius(k / 2.0)
                    .fill(if on { self.accent_ink } else { self.muted })
                    .shadow(1.0, 3.0, self.shadow)
                    .a11y_hidden(),
            )
    }

    pub fn kv(&self, key: &str, value: &str, value_color: Color) -> El {
        row()
            .w_full()
            .gap(12.0)
            .py(6.0)
            .items_start()
            .child(text(key).font(SMALL).color(self.faint).w(92.0))
            .child(
                text(value)
                    .font(MONO)
                    .color(value_color)
                    .grow(1.0)
                    .wrap_anywhere(),
            )
    }

    pub fn empty_state(&self, glyph: &str, title: &str, detail: &str, action: Option<El>) -> El {
        let mut card = self.panel().padding(28.0).gap(10.0).items_center().center();
        card = card
            .child(self.glyph_box(glyph, Tone::Quiet, 38.0))
            .child(
                text(title)
                    .font(H2)
                    .color(self.ink)
                    .align(rune::Align::Center),
            )
            .child(
                text(detail)
                    .font(SMALL)
                    .color(self.muted)
                    .max_w(420.0)
                    .wrap()
                    .align(rune::Align::Center),
            );
        if let Some(a) = action {
            card = card.child(el().h(4.0)).child(a);
        }
        card
    }

    pub fn page_head(&self, kicker: &str, title: &str, detail: &str, trailing: Option<El>) -> El {
        let mut left = col().grow(1.0).gap(5.0).max_w(760.0);
        left = left
            .child(self.eyebrow(kicker))
            .child(text(title).font(DISPLAY).color(self.ink))
            .child(text(detail).font(SMALL).color(self.muted).wrap());
        let mut r = row().w_full().gap(18.0).items_end().child(left);
        if let Some(t) = trailing {
            r = r.child(t);
        }
        r
    }

    pub fn panel_header(&self, title: &str, meta: &str) -> El {
        row()
            .w_full()
            .px(14.0)
            .pt(12.0)
            .pb(10.0)
            .gap(10.0)
            .items_center()
            .child(text(title).font(LABEL).color(self.muted))
            .child(spacer())
            .child(text(meta).font(SMALL).color(self.faint))
            .child(self.divider().absolute(0.0, 34.0))
    }

    pub fn table_head(&self, cols: &[(&str, f32)]) -> El {
        let mut r = row().w_full().px(14.0).h(30.0).items_center().gap(12.0);
        for (label, w) in cols {
            let e = text(*label).font(LABEL).color(self.faint);
            r = r.child(if *w > 0.0 { e.w(*w) } else { e.grow(1.0) });
        }
        r.border_sides([0.0, 0.0, 1.0, 0.0], self.hair)
    }

    pub fn note(&self, s: impl Into<String>) -> El {
        row()
            .w_full()
            .gap(8.0)
            .padding(10.0)
            .radius(7.0)
            .fill(Color::WHITE.alpha(0.025))
            .child(text("·").font(BODY).color(self.faint).a11y_hidden())
            .child(text(s).font(SMALL).color(self.muted).grow(1.0).wrap())
    }

    pub fn field_label(&self, s: &str) -> El {
        text(s).font(LABEL).color(self.faint)
    }

    /// A section row above a card grid: `ENABLED` and its count.
    pub fn grid_head(&self, label: &str, meta: &str) -> El {
        row()
            .w_full()
            .pt(2.0)
            .pb(2.0)
            .gap(9.0)
            .center_y()
            .child(text(label).font(LABEL).color(self.faint))
            .child(text(meta).font(LABEL).color(self.faint).opacity(0.72))
            .child(spacer())
            .child(self.divider().opacity(0.5))
    }

    /// The top bar search shell. The field itself comes from `rune::text_input`
    /// so each screen owns its own `TextEdit`.
    pub fn search_shell(&self, field: El) -> El {
        row()
            .h(34.0)
            .w(268.0)
            .px(12.0)
            .center_y()
            .radius(9.0)
            .fill(Color::WHITE.alpha(0.035))
            .border(1.0, self.hair)
            .child(field.grow(1.0).min_w(0.0))
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
