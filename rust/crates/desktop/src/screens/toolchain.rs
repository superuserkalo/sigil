//! Toolchain: the external decompile and compile tools, whether each is installed, and a plan and
//! run for each stage. Running a tool goes through the confirm dialog below.

use rune::{El, Key, KeyEvent, TextEdit, col, el, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, H1, MONO, Tone, Ui};

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
    let probing = s.toolchain_job.is_running() || v.busy;

    let actions = row().gap(8.0).center_y().child(
        u.icon_button(&icons::REFRESH_CW, "Check again")
            .key("toolchain-probe")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| crate::data::start_toolchain_read(st)),
    );

    let detail = if probing {
        "Checking".to_string()
    } else {
        match (status, &v.error) {
            (Some(status), _) => {
                let available = status.tools.iter().filter(|tool| tool.available).count();
                format!("{available} of {} tools available", status.tools.len())
            }
            (None, Some(error)) => error.clone(),
            (None, None) => String::new(),
        }
    };
    let head = subpage::head(u, "Toolchain", &detail, Some(actions));

    let Some(status) = status else {
        let body = if s.toolchain_job.is_running() {
            el()
        } else {
            u.empty_state(
                "",
                "Tools not checked yet",
                "Check which compile and decompile tools are on this computer.",
                Some(
                    u.action("Check", ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::start_toolchain_read(st)),
                ),
            )
        };
        return subpage::page("toolchain", head, body);
    };

    let mut body = col()
        .w_full()
        .gap(32.0)
        .child(tools(u, status))
        .child(stage_section(s, u, status, Stage::Decompile))
        .child(stage_section(s, u, status, Stage::Compile));
    if let Some(log) = &v.log {
        body = body.child(log_section(u, log, v.log_tone));
    }
    subpage::page("toolchain", head, body)
}

/// Mounted at the root so the process approval overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    s.toolchain.pending.map(|stage| approve_layer(s, u, stage))
}

fn tools(u: &Ui, status: &deadlock_core::toolchain::ToolchainStatus) -> El {
    let mut list = subpage::rows(u);
    for tool in &status.tools {
        let detail = if tool.available {
            tool.version
                .clone()
                .or_else(|| tool.path.as_ref().map(|p| p.display().to_string()))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let action = if tool.available {
            None
        } else {
            let reason = tool
                .reason
                .clone()
                .unwrap_or_else(|| format!("{} was not found.", tool.label));
            Some(
                u.action("How to set up", ActionKind::Ghost)
                    .key(format!("tool-help-{:?}", tool.kind))
                    .mr(-12.0)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.status = reason.clone()),
            )
        };
        list = list.child(
            subpage::tool_line(u, &tool.label, tool.available, &detail, action)
                .key(format!("tool-{:?}", tool.kind)),
        );
    }
    col()
        .w_full()
        .max_w(760.0)
        .gap(8.0)
        .child(row().w_full().h(32.0).center_y().child(u.section("Tools")))
        .child(list)
}

fn stage_section(
    s: &State,
    u: &Ui,
    status: &deadlock_core::toolchain::ToolchainStatus,
    stage: Stage,
) -> El {
    let v = &s.toolchain;
    let (
        key,
        title,
        line,
        available,
        input_label,
        input,
        input_key,
        out,
        out_key,
        input_edit,
        out_edit,
    ): (
        &str,
        &str,
        &str,
        bool,
        &str,
        &TextEdit,
        &str,
        &TextEdit,
        &str,
        fn(&mut State) -> &mut TextEdit,
        fn(&mut State) -> &mut TextEdit,
    ) = match stage {
        Stage::Decompile => (
            "decompile-panel",
            "Decompile",
            "Turn a compiled _c file into editable source in your project.",
            status.decompile_available,
            "Compiled file",
            &v.decompile_input,
            "decompile-input",
            &v.decompile_out,
            "decompile-out",
            |st| &mut st.toolchain.decompile_input,
            |st| &mut st.toolchain.decompile_out,
        ),
        Stage::Compile => (
            "compile-panel",
            "Compile",
            "Turn edited source back into a compiled _c file. Windows only.",
            status.compile_available,
            "Source file",
            &v.compile_source,
            "compile-source",
            &v.compile_out,
            "compile-out",
            |st| &mut st.toolchain.compile_source,
            |st| &mut st.toolchain.compile_out,
        ),
    };
    let ready = available && !v.busy;
    let fade = if ready { 1.0 } else { 0.5 };

    let mut about = row()
        .w_full()
        .gap(4.0)
        .flex_wrap()
        .child(text(line).font(BODY).color(u.muted));
    if !available {
        let tool = match stage {
            Stage::Decompile => "Source2Viewer-CLI",
            Stage::Compile => "resourcecompiler.exe",
        };
        about = about.child(text(format!("Needs {tool}.")).font(BODY).color(u.amber));
    }
    let c = col()
        .key(key)
        .w_full()
        .max_w(760.0)
        .gap(8.0)
        .child(col().w_full().gap(2.0).child(u.section(title)).child(about));
    c.child(el().h(4.0))
        .child(
            subpage::rows(u)
                .child(field_row(u, input_label, input, input_key, input_edit))
                .child(field_row(u, "Output folder", out, out_key, out_edit)),
        )
        .child(
            row()
                .w_full()
                .pt(8.0)
                .gap(8.0)
                .child(spacer())
                .child(
                    u.action("Plan", ActionKind::Ghost)
                        .key(format!("{key}-plan"))
                        .disabled(!ready)
                        .opacity(fade)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| crate::data::plan_tool(st, stage)),
                )
                .child(
                    u.action("Run", ActionKind::Secondary)
                        .key(format!("{key}-run"))
                        .disabled(!ready)
                        .opacity(fade)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| st.toolchain.pending = Some(stage)),
                ),
        )
}

fn field_row(
    u: &Ui,
    label: &str,
    value: &TextEdit,
    key: &str,
    edit: fn(&mut State) -> &mut TextEdit,
) -> El {
    row()
        .w_full()
        .h(48.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(label.to_string()).font(BODY).color(u.muted).w(140.0))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(value, BODY)
                        .key(key)
                        .label(label.to_string())
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .color(u.ink)
                        .placeholder("Full path", u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(edit),
                ),
        )
}

fn log_section(u: &Ui, log: &str, tone: Tone) -> El {
    let color = match tone {
        Tone::Warn | Tone::Bad => tone.fg(u),
        _ => u.prose,
    };
    col()
        .key("toolchain-log")
        .w_full()
        .max_w(760.0)
        .gap(8.0)
        .child(subpage::group_head(u, "Last result", ""))
        .child(
            u.well_field().w_full().padding(12.0).child(
                text(log.to_string())
                    .font(MONO)
                    .color(color)
                    .wrap_anywhere()
                    .selectable(true),
            ),
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
    let kv = |key: &str, value: El| {
        row()
            .w_full()
            .py(6.0)
            .gap(16.0)
            .items_start()
            .child(text(key.to_string()).font(BODY).color(u.muted).w(88.0))
            .child(value.grow(1.0).min_w(0.0))
    };
    let close = |st: &mut State| st.toolchain.pending = None;
    u.dialog_shell()
        .key("toolchain-approve")
        .role("dialog")
        .label(title)
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(text(title).font(H1).color(u.ink))
                .child(
                    text("Sigil starts the program directly and shows its output here.")
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(
            col()
                .w_full()
                .child(kv(
                    "Command",
                    text(program).font(MONO).color(u.ink).wrap_anywhere(),
                ))
                .child(kv(
                    "Input",
                    if target.trim().is_empty() {
                        text("No file chosen").font(BODY).color(u.amber)
                    } else {
                        text(target).font(MONO).color(u.prose).wrap_anywhere()
                    },
                )),
        )
        .child(
            row()
                .w_full()
                .pt(8.0)
                .gap(8.0)
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .key("toolchain-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.toolchain.pending = None),
                )
                .child(
                    u.action("Run", ActionKind::Primary)
                        .key("toolchain-run")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| crate::data::run_tool(st, stage)),
                ),
        )
}
