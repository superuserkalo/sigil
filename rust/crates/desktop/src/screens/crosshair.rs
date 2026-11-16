//! Crosshair: the reticle the game reads, with a live geometry preview.
//!
//! The settings and the command text come from `deadlock_core::crosshair`, which
//! also owns the HUD geometry. Applying writes a managed block into
//! `autoexec.cfg`, so it sits behind a confirm step and the same backup path as
//! the Autoexec screen.

use rune::{Color, El, Key, KeyEvent, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY_MED, H2, LABEL, MONO, SMALL, Tone, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Gap,
    Height,
    Width,
    Opacity,
    OutlineBorder,
    OutlineGap,
    OutlineOpacity,
    DotSize,
    DotOpacity,
    DotBorder,
    DotGap,
    DotOutlineOpacity,
}

pub struct View {
    pub settings: deadlock_core::crosshair::Crosshair,
    pub source: String,
    pub log: Option<String>,
    pub tone: Tone,
    pub pending: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            settings: deadlock_core::crosshair::Crosshair::default(),
            source: "editor defaults".to_string(),
            log: None,
            tone: Tone::Quiet,
            pending: false,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.crosshair;
    let crosshair = &v.settings;
    let dirty = v.settings != deadlock_core::crosshair::Crosshair::default();

    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if crosshair.pip_width == 0 || crosshair.pip_height == 0 {
            u.status_chip("pips off", Tone::Warn)
        } else {
            u.status_chip(
                &format!(
                    "{}x{} gap {}",
                    crosshair.pip_width, crosshair.pip_height, crosshair.pip_gap
                ),
                Tone::Accent,
            )
        })
        .child(if dirty {
            u.status_chip("edited", Tone::Warn)
        } else {
            u.status_chip("defaults", Tone::Quiet)
        })
        .child(
            u.action("Read from game", ActionKind::Secondary)
                .key("crosshair-import")
                .on_press(|st: &mut State| crate::data::import_crosshair(st)),
        )
        .child(
            u.action("Apply to autoexec", ActionKind::Primary)
                .key("crosshair-apply")
                .on_press(|st: &mut State| st.crosshair.pending = true),
        );

    let detail = format!(
        "{} · {} · the block is one managed section in game/citadel/cfg/autoexec.cfg",
        v.source,
        if s.game_detected {
            "game found"
        } else {
            "no game install"
        }
    );

    let mut page = col()
        .key("crosshair-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "crosshair-glow",
            [0.2, 0.0],
            (700.0, 420.0),
            u.glow_cool,
        ))
        .child(u.page_head(
            Screen::Crosshair.kicker(),
            "Crosshair",
            &detail,
            Some(trailing),
        ));

    if let Some(log) = &v.log {
        page = page.child(
            row()
                .w_full()
                .padding(10.0)
                .gap(10.0)
                .items_start()
                .radius(9.0)
                .fill(v.tone.bg(u))
                .border(1.0, v.tone.fg(u).alpha(0.22))
                .child(u.dot(v.tone))
                .child(text(log.clone()).font(SMALL).color(u.muted).wrap()),
        );
    }

    page.child(
        row()
            .w_full()
            .gap(12.0)
            .items_start()
            .child(preview(s, u))
            .child(controls(s, u)),
    )
    .child(commands(s, u))
}

fn preview(s: &State, u: &Ui) -> El {
    let crosshair = &s.crosshair.settings;
    let scale = 2.2;
    let layout = crosshair.layout(scale);
    let size = 320.0;
    let centre = size / 2.0;
    let fill = Color {
        r: crosshair.color_r as f32 / 255.0,
        g: crosshair.color_g as f32 / 255.0,
        b: crosshair.color_b as f32 / 255.0,
        a: crosshair.pip_opacity as f32,
    };
    let outline = Color {
        r: crosshair.outline_color_r as f32 / 255.0,
        g: crosshair.outline_color_g as f32 / 255.0,
        b: crosshair.outline_color_b as f32 / 255.0,
        a: crosshair.pip_outline_opacity as f32,
    };
    let dot = Color {
        a: crosshair.dot_opacity as f32,
        ..fill
    };

    let mut canvas = el()
        .key("crosshair-canvas")
        .size(size, size)
        .radius(14.0)
        .fill(Color::BLACK.alpha(0.45))
        .border(1.0, u.hair)
        .clip_rounded(14.0);

    // A soft backdrop and a centre grid, so the preview reads as a screen.
    canvas = canvas
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .radial_gradient(
                    [0.5, 0.46],
                    [220.0, 220.0],
                    [(0.0, u.glow_cool.alpha(0.16)), (1.0, Color::TRANSPARENT)],
                )
                .a11y_hidden(),
        )
        .child(
            el().absolute(centre - 0.5, 24.0)
                .w(1.0)
                .h(size - 48.0)
                .fill(Color::WHITE.alpha(0.035))
                .a11y_hidden(),
        )
        .child(
            el().absolute(24.0, centre - 0.5)
                .w(size - 48.0)
                .h(1.0)
                .fill(Color::WHITE.alpha(0.035))
                .a11y_hidden(),
        );

    for (index, rect) in layout.pip_outlines.iter().enumerate() {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        canvas = canvas.child(
            el().key(format!("outline-{index}"))
                .absolute(centre + rect.x, centre + rect.y)
                .size(rect.w, rect.h)
                .fill(outline)
                .a11y_hidden(),
        );
    }
    for (index, rect) in layout.pips.iter().enumerate() {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        canvas = canvas.child(
            el().key(format!("pip-{index}"))
                .absolute(centre + rect.x, centre + rect.y)
                .size(rect.w, rect.h)
                .fill(fill)
                .a11y_hidden(),
        );
    }
    if layout.dot_outline.w > 0.0 && crosshair.dot_opacity > 0.0 {
        canvas = canvas.child(
            el().key("dot-outline")
                .absolute(
                    centre + layout.dot_outline.x,
                    centre + layout.dot_outline.y,
                )
                .size(layout.dot_outline.w, layout.dot_outline.h)
                .radius(layout.dot_outline.w / 2.0)
                .fill(outline)
                .a11y_hidden(),
        );
    }
    if layout.dot.w > 0.0 && crosshair.dot_opacity > 0.0 {
        canvas = canvas.child(
            el().key("dot")
                .absolute(centre + layout.dot.x, centre + layout.dot.y)
                .size(layout.dot.w, layout.dot.h)
                .radius(layout.dot.w / 2.0)
                .fill(dot)
                .a11y_hidden(),
        );
    }

    col()
        .key("crosshair-preview")
        .gap(9.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .child(text("PREVIEW").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!("{:.2}x scale", scale))
                        .font(LABEL)
                        .color(u.faint),
                ),
        )
        .child(canvas)
        .child(
            text("Geometry mirrors the HUD: sizes snap once to device pixels, the dot is a circle, and outlines paint inward.")
                .font(LABEL)
                .color(u.faint)
                .max_w(size)
                .wrap(),
        )
}

fn controls(s: &State, u: &Ui) -> El {
    let crosshair = &s.crosshair.settings;
    let mut left = col().w_full().gap(6.0);
    left = left
        .child(u.grid_head("PIPS", "size and gap"))
        .child(step_row(
            u,
            "Gap",
            crosshair.pip_gap,
            Field::Gap,
            1,
        ))
        .child(step_row(
            u,
            "Height",
            crosshair.pip_height,
            Field::Height,
            1,
        ))
        .child(step_row(u, "Width", crosshair.pip_width, Field::Width, 1))
        .child(step_row_float(
            u,
            "Opacity",
            crosshair.pip_opacity,
            Field::Opacity,
            0.05,
        ))
        .child(u.grid_head("PIP OUTLINE", "border, gap, opacity"))
        .child(step_row(
            u,
            "Border",
            crosshair.pip_outline_border,
            Field::OutlineBorder,
            1,
        ))
        .child(step_row(
            u,
            "Gap",
            crosshair.pip_outline_gap,
            Field::OutlineGap,
            1,
        ))
        .child(step_row_float(
            u,
            "Opacity",
            crosshair.pip_outline_opacity,
            Field::OutlineOpacity,
            0.05,
        ));

    let mut right = col().w_full().gap(6.0);
    right = right
        .child(u.grid_head("DOT", "size and outline"))
        .child(step_row(u, "Size", crosshair.dot_size, Field::DotSize, 1))
        .child(step_row_float(
            u,
            "Opacity",
            crosshair.dot_opacity,
            Field::DotOpacity,
            0.05,
        ))
        .child(step_row(
            u,
            "Border",
            crosshair.dot_outline_border,
            Field::DotBorder,
            1,
        ))
        .child(step_row(
            u,
            "Gap",
            crosshair.dot_outline_gap,
            Field::DotGap,
            1,
        ))
        .child(step_row_float(
            u,
            "Opacity",
            crosshair.dot_outline_opacity,
            Field::DotOutlineOpacity,
            0.05,
        ))
        .child(u.grid_head("COLOUR", "click a swatch"))
        .child(swatches(u, crosshair))
        .child(u.grid_head("BEHAVIOUR", ""))
        .child(switch_row(
            u,
            "Static gap",
            "the gap does not grow when you move",
            crosshair.pip_gap_static,
            |st: &mut State| {
                st.crosshair.settings.pip_gap_static =
                    !st.crosshair.settings.pip_gap_static
            },
        ))
        .child(switch_row(
            u,
            "Ignore hero crosshairs",
            "one reticle for every hero",
            crosshair.disable_hero_specific_crosshairs,
            |st: &mut State| {
                st.crosshair.settings.disable_hero_specific_crosshairs =
                    !st.crosshair.settings.disable_hero_specific_crosshairs
            },
        ))
        .child(u.divider())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(
                    u.action("Game defaults", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| {
                            st.crosshair.settings =
                                deadlock_core::crosshair::Crosshair::game_defaults();
                            st.crosshair.source = "game defaults".to_string();
                            st.crosshair.log =
                                Some("Loaded the values a fresh install registers.".to_string());
                            st.crosshair.tone = Tone::Info;
                        }),
                )
                .child(
                    u.action("Reset", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| {
                            st.crosshair.settings =
                                deadlock_core::crosshair::Crosshair::default();
                            st.crosshair.source = "editor defaults".to_string();
                            st.crosshair.log = Some("Reset to the editor defaults.".to_string());
                            st.crosshair.tone = Tone::Quiet;
                        }),
                ),
        );

    col()
        .key("crosshair-controls")
        .grow(1.0)
        .min_w(0.0)
        .gap(12.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .child(text("SETTINGS").font(BODY_MED).color(u.ink))
                .child(spacer())
                .child(
                    text("the full convar set is written")
                        .font(LABEL)
                        .color(u.faint),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(18.0)
                .items_start()
                .child(left.grow(1.0))
                .child(right.grow(1.0)),
        )
}

fn step_row(u: &Ui, label: &str, value: i32, field: Field, delta: i32) -> El {
    row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(text(label).font(SMALL).color(u.faint).grow(1.0))
        .child(stepper(u, field, -delta))
        .child(
            text(format!("{value}"))
                .font(MONO)
                .color(u.ink)
                .w(34.0)
                .align(rune::Align::Center),
        )
        .child(stepper(u, field, delta))
}

fn step_row_float(u: &Ui, label: &str, value: f64, field: Field, delta: f64) -> El {
    let scaled = (delta * 100.0) as i32;
    row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(text(label).font(SMALL).color(u.faint).grow(1.0))
        .child(stepper_scaled(u, field, -scaled))
        .child(
            text(format!("{value:.2}"))
                .font(MONO)
                .color(u.ink)
                .w(38.0)
                .align(rune::Align::Center),
        )
        .child(stepper_scaled(u, field, scaled))
}

fn stepper(u: &Ui, field: Field, delta: i32) -> El {
    step_button(u, field, delta, if delta > 0 { "+" } else { "−" })
}

fn stepper_scaled(u: &Ui, field: Field, delta: i32) -> El {
    step_button(u, field, delta, if delta > 0 { "+" } else { "−" })
}

fn step_button(u: &Ui, field: Field, delta: i32, glyph: &str) -> El {
    let label = format!("{glyph} {field:?}");
    row()
        .size(22.0, 22.0)
        .center()
        .radius(6.0)
        .fill(Color::WHITE.alpha(0.05))
        .border(1.0, u.hair)
        .hover(Color::WHITE.alpha(0.09))
        .press(Color::WHITE.alpha(0.12))
        .focusable(true)
        .role("button")
        .label(label)
        .child(text(glyph).font(MONO).color(u.muted).a11y_hidden())
        .on_press(move |st: &mut State| nudge(st, field, delta))
}

pub fn nudge(st: &mut State, field: Field, delta: i32) {
    let crosshair = &mut st.crosshair.settings;
    match field {
        Field::Gap => crosshair.pip_gap = clamp_int(crosshair.pip_gap + delta, 0, 64),
        Field::Height => crosshair.pip_height = clamp_int(crosshair.pip_height + delta, 0, 64),
        Field::Width => crosshair.pip_width = clamp_int(crosshair.pip_width + delta, 0, 64),
        Field::OutlineBorder => {
            crosshair.pip_outline_border = clamp_int(crosshair.pip_outline_border + delta, 0, 16)
        }
        Field::OutlineGap => {
            crosshair.pip_outline_gap = clamp_int(crosshair.pip_outline_gap + delta, 0, 16)
        }
        Field::DotSize => crosshair.dot_size = clamp_int(crosshair.dot_size + delta, 0, 64),
        Field::DotBorder => {
            crosshair.dot_outline_border = clamp_int(crosshair.dot_outline_border + delta, 0, 16)
        }
        Field::DotGap => {
            crosshair.dot_outline_gap = clamp_int(crosshair.dot_outline_gap + delta, 0, 16)
        }
        Field::Opacity => {
            crosshair.pip_opacity = clamp_float(crosshair.pip_opacity + delta as f64 / 100.0)
        }
        Field::OutlineOpacity => {
            crosshair.pip_outline_opacity =
                clamp_float(crosshair.pip_outline_opacity + delta as f64 / 100.0)
        }
        Field::DotOpacity => {
            crosshair.dot_opacity = clamp_float(crosshair.dot_opacity + delta as f64 / 100.0)
        }
        Field::DotOutlineOpacity => {
            crosshair.dot_outline_opacity =
                clamp_float(crosshair.dot_outline_opacity + delta as f64 / 100.0)
        }
    }
    st.crosshair.source = "edited".to_string();
}

fn clamp_int(value: i32, low: i32, high: i32) -> i32 {
    value.clamp(low, high)
}

fn clamp_float(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn swatches(u: &Ui, crosshair: &deadlock_core::crosshair::Crosshair) -> El {
    let colours = [
        ("green", 0, 255, 0),
        ("cyan", 0, 255, 255),
        ("white", 255, 255, 255),
        ("red", 255, 60, 60),
        ("amber", 255, 190, 0),
        ("magenta", 255, 60, 220),
    ];
    let mut row_el = row().w_full().gap(6.0);
    for (name, r, g, b) in colours {
        let selected =
            crosshair.color_r == r && crosshair.color_g == g && crosshair.color_b == b;
        row_el = row_el.child(
            el()
                .key(format!("swatch-{name}"))
                .size(24.0, 24.0)
                .radius(6.0)
                .fill(Color {
                    r: r as f32 / 255.0,
                    g: g as f32 / 255.0,
                    b: b as f32 / 255.0,
                    a: 1.0,
                })
                .border(
                    if selected { 2.0 } else { 1.0 },
                    if selected { u.ink } else { u.hair },
                )
                .focusable(true)
                .role("button")
                .label(format!("{name} crosshair colour"))
                .on_press(move |st: &mut State| {
                    st.crosshair.settings.color_r = r;
                    st.crosshair.settings.color_g = g;
                    st.crosshair.settings.color_b = b;
                    st.crosshair.source = "edited".to_string();
                }),
        );
    }
    row_el
}

fn switch_row(
    u: &Ui,
    label: &str,
    detail: &str,
    on: bool,
    press: fn(&mut State),
) -> El {
    row()
        .w_full()
        .gap(9.0)
        .center_y()
        .py(3.0)
        .child(u.toggle(on))
        .child(
            col()
                .grow(1.0)
                .gap(1.0)
                .child(text(label).font(SMALL).color(u.ink))
                .child(text(detail).font(LABEL).color(u.faint)),
        )
        .on_press(press)
}

fn commands(s: &State, u: &Ui) -> El {
    let crosshair = &s.crosshair.settings;
    col()
        .key("crosshair-commands")
        .w_full()
        .gap(8.0)
        .padding(14.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .child(text("CONVARS").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!(
                        "{} commands · full set, never a partial write",
                        crosshair.commands().len()
                    ))
                    .font(LABEL)
                    .color(u.faint),
                ),
        )
        .child(
            col()
                .w_full()
                .h(150.0)
                .padding(10.0)
                .radius(9.0)
                .fill(u.field)
                .border(1.0, u.hair)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(crosshair.command_text())
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        )
        .child(
            text("Apply merges a managed block into autoexec.cfg and keeps one backup per write. Clear removes only that block.")
                .font(LABEL)
                .color(u.faint)
                .wrap(),
        )
}

/// Mounted at the root so the apply approval overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if !s.crosshair.pending {
        return None;
    }
    let crosshair = &s.crosshair.settings;
    let body = col()
        .w_full()
        .gap(15.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(6.0)
                        .child(u.eyebrow("APP ACTION · AUTOEXEC"))
                        .child(text("Apply crosshair").font(H2).color(u.ink))
                        .child(
                            text("Writes the managed crosshair block into game/citadel/cfg/autoexec.cfg. The previous file is copied beside itself first.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("✛", Tone::Accent, 36.0)),
        )
        .child(u.divider())
        .child(u.kv(
            "Block",
            &format!(
                "{} .. {}",
                deadlock_core::crosshair::AUTOEXEC_BEGIN,
                deadlock_core::crosshair::AUTOEXEC_END
            ),
            u.faint,
        ))
        .child(u.kv("Values", &crosshair.to_string(), u.ink))
        .child(u.kv(
            "Refuses",
            "an autoexec with a begin marker and no end marker, before writing anything",
            u.faint,
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.crosshair.pending = false),
                )
                .child(
                    u.action("Write block", ActionKind::Primary)
                        .grow(1.0)
                        .on_press(|st: &mut State| crate::data::apply_crosshair(st)),
                ),
        );
    let close = |st: &mut State| st.crosshair.pending = false;
    Some(
        u.dialog_shell()
            .key("crosshair-apply-dialog")
            .role("dialog")
            .label("Apply crosshair")
            .w(500.0)
            .padding(1.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(u.dialog_core().padding(20.0).child(body)),
    )
}
