//! Toolchain: external decompile and compile tools, with real availability.
//!
//! Nothing is bundled or downloaded. Each tool resolves from a `DAH_*` setting
//! and then `PATH`. A plan shows the exact program and argument array before
//! anything starts; running needs the approval step.

use rune::{Color, El, Key, KeyEvent, TextEdit, col, row, spacer, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, H2, LABEL, MONO, SMALL, Tone, Ui};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Decompile,
    Compile,
}

pub struct View {
    pub loaded: bool,
    pub status: Option<deadlock_core::toolchain::ToolchainStatus>,
    pub error: Option<String>,
    pub decompile_input: TextEdit,
    pub decompile_out: TextEdit,
    pub compile_source: TextEdit,
    pub compile_out: TextEdit,
    /// Last plan or result, shown as text.
    pub log: Option<String>,
    pub log_tone: Tone,
    pub pending: Option<Stage>,
    pub busy: bool,
}

impl View {
    /// `gameinfo.gi` for the compile step. `DAH_CSDK_PATH` is the documented
    /// setting; a missing value makes the plan refuse instead of guessing.
    pub fn compile_gameinfo(&self) -> Option<std::path::PathBuf> {
        std::env::var_os("DAH_CSDK_PATH")
            .map(std::path::PathBuf::from)
            .map(|root| {
                if root.is_file() {
                    root
                } else {
                    root.join("gameinfo.gi")
                }
            })
    }
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            status: None,
            error: None,
            decompile_input: TextEdit::new(""),
            decompile_out: TextEdit::new(""),
            compile_source: TextEdit::new(""),
            compile_out: TextEdit::new(""),
            log: None,
            log_tone: Tone::Quiet,
            pending: None,
            busy: false,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.toolchain;
    let status = v.status.as_ref();
    let available = status
        .map(|status| {
            status
                .tools
                .iter()
                .filter(|tool| tool.available)
                .count()
        })
        .unwrap_or(0);

    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.toolchain_job.is_running() || v.busy {
            u.status_chip("probing", Tone::Warn)
        } else if let Some(status) = status {
            u.status_chip(
                &format!("{available} of {} tools", status.tools.len()),
                if available == 0 { Tone::Quiet } else { Tone::Ok },
            )
        } else {
            u.status_chip("not probed", Tone::Quiet)
        })
        .child(
            u.pill("Probe again", "↻", ActionKind::Ghost)
                .key("toolchain-probe")
                .on_press(|st: &mut State| crate::data::start_toolchain_read(st)),
        );

    let detail = match (status, &v.error) {
        (Some(status), _) => format!(
            "{} · decompile {} · compile {}",
            status.platform,
            if status.decompile_available {
                "ready"
            } else {
                "unavailable"
            },
            if status.compile_available {
                "ready"
            } else {
                "unavailable"
            }
        ),
        (None, Some(error)) => error.clone(),
        (None, None) => "Probing the environment for external tools.".to_string(),
    };

    let mut page = col()
        .key("toolchain-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "toolchain-glow",
            [0.1, 0.0],
            (700.0, 400.0),
            u.glow_cool,
        ))
        .child(u.page_head(
            Screen::Toolchain.kicker(),
            "Toolchain",
            &detail,
            Some(trailing),
        ));

    let Some(status) = status else {
        return page.child(if s.toolchain_job.is_running() {
            u.note("Reading DAH_SOURCE2VIEWER_CLI, DAH_CSDK_PATH, DAH_VPKEDITCLI and PATH.")
        } else {
            u.empty_state(
                "⚒",
                "Toolchain not probed",
                "Probe the environment to see which external tools are available. Nothing is downloaded.",
                Some(
                    u.action("Probe", ActionKind::Secondary)
                        .on_press(|st: &mut State| crate::data::start_toolchain_read(st)),
                ),
            )
        });
    };

    page = page.child(tool_grid(u, status));
    page = page.child(
        row()
            .w_full()
            .gap(12.0)
            .items_start()
            .child(decompile_panel(s, u, status))
            .child(compile_panel(s, u, status)),
    );
    page = page.child(
        text("CSDK 12 is used from a local install. It has no license, so it is never bundled, downloaded or redistributed by this app.")
            .font(LABEL)
            .color(u.faint)
            .wrap(),
    );

    if let Some(log) = &v.log {
        page = page.child(log_panel(u, log, v.log_tone));
    }

    page
}

/// Mounted at the root so the process approval overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    s.toolchain
        .pending
        .map(|stage| approve_layer(s, u, stage))
}

fn tool_grid(u: &Ui, status: &deadlock_core::toolchain::ToolchainStatus) -> El {
    let mut r = row().w_full().gap(10.0).items_start();
    for tool in &status.tools {
        let tone = if tool.available { Tone::Ok } else { Tone::Quiet };
        let mut card = col()
            .key(format!("tool-{:?}", tool.kind))
            .grow(1.0)
            .min_w(0.0)
            .gap(6.0)
            .padding(13.0)
            .radius(12.0)
            .fill(if tool.available {
                u.teal.alpha(0.045)
            } else {
                Color::WHITE.alpha(0.022)
            })
            .border(
                1.0,
                if tool.available {
                    u.teal.alpha(0.18)
                } else {
                    u.hair
                },
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(u.dot(tone))
                    .child(
                        text(tool.label.clone())
                            .font(BODY_MED)
                            .color(u.ink)
                            .grow(1.0)
                            .truncate(),
                    )
                    .child(u.chip(
                        if tool.available { "ready" } else { "absent" },
                        tone.fg(u),
                        tone.bg(u),
                    )),
            )
            .child(
                text(match &tool.version {
                    Some(version) => version.clone(),
                    None => if tool.available {
                        "version not reported".to_string()
                    } else {
                        "no binary".to_string()
                    },
                })
                .font(MONO)
                .color(u.faint)
                .truncate(),
            );
        card = card.child(
            text(
                tool.path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .or_else(|| tool.reason.clone())
                    .unwrap_or_else(|| "not found".to_string()),
            )
            .font(SMALL)
            .color(if tool.available { u.muted } else { u.faint })
            .wrap(),
        );
        r = r.child(card);
    }
    r
}

fn decompile_panel(
    s: &State,
    u: &Ui,
    status: &deadlock_core::toolchain::ToolchainStatus,
) -> El {
    let v = &s.toolchain;
    let ready = status.decompile_available && !v.busy;
    col()
        .key("decompile-panel")
        .grow(1.0)
        .min_w(0.0)
        .gap(9.0)
        .padding(15.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(u.eyebrow("COMPILED TO SOURCE"))
                        .child(text("Decompile").font(H2).color(u.ink))
                        .child(
                            text("Source2Viewer-CLI turns a compiled _c asset into editable source inside the project.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("↓", if ready { Tone::Ok } else { Tone::Quiet }, 30.0)),
        )
        .child(
            text(
                status
                    .decompile_reason
                    .clone()
                    .unwrap_or_else(|| "Source2Viewer-CLI is available.".to_string()),
            )
            .font(LABEL)
            .color(if ready { u.teal } else { u.amber })
            .wrap(),
        )
        .child(field(u, "COMPILED FILE", &v.decompile_input, "decompile-input", |st| {
            &mut st.toolchain.decompile_input
        }))
        .child(field(u, "OUTPUT FOLDER", &v.decompile_out, "decompile-out", |st| {
            &mut st.toolchain.decompile_out
        }))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(
                    u.action("Plan", ActionKind::Secondary)
                        .grow(1.0)
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.5 })
                        .on_press(|st: &mut State| crate::data::plan_tool(st, Stage::Decompile)),
                )
                .child(
                    u.action("Run with approval", ActionKind::Primary)
                        .grow(1.0)
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.5 })
                        .on_press(|st: &mut State| st.toolchain.pending = Some(Stage::Decompile)),
                ),
        )
}

fn compile_panel(
    s: &State,
    u: &Ui,
    status: &deadlock_core::toolchain::ToolchainStatus,
) -> El {
    let v = &s.toolchain;
    let ready = status.compile_available && !v.busy;
    col()
        .key("compile-panel")
        .grow(1.0)
        .min_w(0.0)
        .gap(9.0)
        .padding(15.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(u.eyebrow("SOURCE TO COMPILED"))
                        .child(text("Compile").font(H2).color(u.ink))
                        .child(
                            text("resourcecompiler.exe turns edited source into a compiled _c asset. It is Windows only.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("↑", if ready { Tone::Ok } else { Tone::Quiet }, 30.0)),
        )
        .child(
            text(
                status
                    .compile_reason
                    .clone()
                    .unwrap_or_else(|| "resourcecompiler.exe is available.".to_string()),
            )
            .font(LABEL)
            .color(if ready { u.teal } else { u.amber })
            .wrap(),
        )
        .child(field(u, "SOURCE FILE", &v.compile_source, "compile-source", |st| {
            &mut st.toolchain.compile_source
        }))
        .child(field(u, "OUTPUT FOLDER", &v.compile_out, "compile-out", |st| {
            &mut st.toolchain.compile_out
        }))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(
                    u.action("Plan", ActionKind::Secondary)
                        .grow(1.0)
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.5 })
                        .on_press(|st: &mut State| crate::data::plan_tool(st, Stage::Compile)),
                )
                .child(
                    u.action("Run with approval", ActionKind::Primary)
                        .grow(1.0)
                        .disabled(!ready)
                        .opacity(if ready { 1.0 } else { 0.5 })
                        .on_press(|st: &mut State| st.toolchain.pending = Some(Stage::Compile)),
                ),
        )
}

fn field<'a>(
    u: &Ui,
    label: &str,
    value: &'a TextEdit,
    key: &str,
    edit: fn(&mut State) -> &mut TextEdit,
) -> El {
    col()
        .w_full()
        .gap(4.0)
        .child(text(label).font(LABEL).color(u.faint))
        .child(
            rune::text_input(value, BODY)
                .key(key)
                .label(label)
                .single_line()
                .color(u.ink)
                .placeholder("absolute path", u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(edit),
        )
}

fn log_panel(u: &Ui, log: &str, tone: Tone) -> El {
    col()
        .key("toolchain-log")
        .w_full()
        .gap(6.0)
        .padding(12.0)
        .radius(10.0)
        .fill(u.field)
        .border(1.0, tone.fg(u).alpha(0.22))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.dot(tone))
                .child(text("PLAN / RESULT").font(LABEL).color(u.faint))
                .child(spacer()),
        )
        .child(
            text(log.to_string())
                .font(MONO)
                .color(u.prose)
                .wrap_anywhere()
                .selectable(true),
        )
}

fn approve_layer(s: &State, u: &Ui, stage: Stage) -> El {
    let (title, program, target) = match stage {
        Stage::Decompile => (
            "Run Source2Viewer-CLI",
            "Source2Viewer-CLI -i <compiled file> -o <output> -d",
            s.toolchain.decompile_input.text.clone(),
        ),
        Stage::Compile => (
            "Run resourcecompiler.exe",
            "resourcecompiler.exe -i <source> -game <gameinfo>",
            s.toolchain.compile_source.text.clone(),
        ),
    };
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
                        .child(u.eyebrow("PROCESS LAUNCH · NO SHELL"))
                        .child(text(title).font(H2).color(u.ink))
                        .child(
                            text("The program and an argument array start directly. Output is captured, capped by a timeout, and the process never goes through a shell.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("⚒", Tone::Warn, 36.0)),
        )
        .child(u.divider())
        .child(u.kv("Command", program, u.ink))
        .child(u.kv(
            "Input",
            if target.trim().is_empty() {
                "an empty path, which the plan will refuse"
            } else {
                &target
            },
            u.muted,
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.toolchain.pending = None),
                )
                .child(
                    u.action("Run", ActionKind::Primary)
                        .grow(1.0)
                        .on_press(move |st: &mut State| crate::data::run_tool(st, stage)),
                ),
        );
    let close = |st: &mut State| st.toolchain.pending = None;
    u.dialog_shell()
        .key("toolchain-approve")
        .role("dialog")
        .label(title)
        .w(520.0)
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
        .child(u.dialog_core().padding(20.0).child(body))
}
