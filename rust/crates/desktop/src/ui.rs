//! Design tokens for the workbench: one palette, one type scale, one set of primitives.
//!
//! Rune's `tok` palette is tuned for neutral app chrome. The workbench keeps its semantic
//! colours and adds a small Deadlock-flavoured layer on top: ember gold for action, teal for
//! verified state, rose for blocked state. Everything is built from `rune::oklch` so the
//! palette stays perceptually even.

use rune::tok::{self, Theme, motion};
use rune::{Color, El, Family, TextStyle, col, el, oklch, row, rule, spacer, text};

use crate::model::Compat;

pub const SIDEBAR_W: f32 = 238.0;
pub const RAIL_W: f32 = 304.0;
pub const HEADER_H: f32 = 58.0;
pub const STATUS_H: f32 = 27.0;

pub const DISPLAY: TextStyle = TextStyle::new(25.0, 640.0).lh(31.0);
pub const H1: TextStyle = TextStyle::new(19.0, 600.0).lh(25.0);
pub const H2: TextStyle = TextStyle::new(14.5, 590.0).lh(20.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 420.0).lh(20.0);
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
    pub panel: Color,
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
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Ui {
        let t: Theme = tok::theme(true);
        Ui {
            bg: oklch(0.168, 0.007, 265.0),
            sidebar: oklch(0.138, 0.006, 265.0),
            panel: t.card,
            well: oklch(0.152, 0.006, 265.0),
            hair: Color::WHITE.alpha(0.065),
            hair_strong: Color::WHITE.alpha(0.11),
            ink: oklch(0.955, 0.004, 80.0),
            prose: oklch(0.86, 0.005, 80.0),
            muted: oklch(0.68, 0.008, 260.0),
            faint: oklch(0.50, 0.008, 260.0),
            accent: oklch(0.80, 0.135, 76.0),
            accent_ink: oklch(0.19, 0.035, 70.0),
            teal: oklch(0.74, 0.09, 190.0),
            rose: oklch(0.68, 0.16, 22.0),
            amber: oklch(0.82, 0.13, 72.0),
            violet: oklch(0.72, 0.10, 300.0),
            shadow: Color::BLACK.alpha(0.32),
        }
    }

    pub fn eyebrow(&self, s: impl Into<String>) -> El {
        text(s).font(LABEL).color(self.faint)
    }

    pub fn panel(&self) -> El {
        col().fill(self.panel).radius(10.0).border(1.0, self.hair)
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
            .h(29.0)
            .px(11.0)
            .gap(6.0)
            .center()
            .radius(7.0)
            .fill(fill)
            .border(1.0, border)
            .hover(Color::WHITE.alpha(0.05))
            .press(Color::WHITE.alpha(0.08))
            .press_scale(0.985)
            .focusable(true)
            .role("button")
            .label(label)
            .motion(motion::SNAPPY)
            .child(text(label).font(LABEL).color(ink).a11y_hidden())
    }

    fn action_colors(&self, kind: ActionKind) -> (Color, Color, Color) {
        match kind {
            ActionKind::Primary => (self.accent, self.accent_ink, Color::TRANSPARENT),
            ActionKind::Secondary => (Color::WHITE.alpha(0.055), self.ink, self.hair_strong),
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
