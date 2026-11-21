//! Agent: the embedded conversation. A narrow column of mods on the left, the thread in the
//! middle, and one composer pinned under it. Tool calls read as a quiet work log; an approval
//! sits on top of the composer until it is answered.

use std::time::{SystemTime, UNIX_EPOCH};

use rune::{Color, El, ScrollSpec, TextEdit, TextStyle, col, el, icon, oklch, row, spacer, text};

use crate::agent::{AgentStatus, Pause};
use crate::app::State;
use crate::icons;
use crate::model::{self, Message, Project, Role, RunState, Screen, ToolRun};
use crate::ui::{ActionKind, BODY, BODY_MED, CHAT, MONO, NAV, SMALL, Ui};

const THREADS_W: f32 = 240.0;
const THREAD_MAX: f32 = 736.0;
const GUTTER: f32 = 20.0;
const HEADLINE: TextStyle = TextStyle::new(24.0, 400.0).lh(32.0).tracking(-0.2);

const SUGGESTIONS: [&str; 4] = [
    "Recolor Infernus in ash and ember",
    "Make my crosshair a small dot",
    "Find every file for Haze's model",
    "Check which of my mods broke in the last patch",
];

pub fn view(s: &State, u: &Ui) -> El {
    row()
        .key("agent-surface")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(threads(s, u))
        .child(conversation(s, u))
}

// The left column: new mod, the mods being made, then the workshop links.

fn threads(s: &State, u: &Ui) -> El {
    let mut list = col().key("agent-mods").w_full().gap(2.0);
    if s.projects.is_empty() {
        list = list.child(
            text("No mods yet")
                .font(BODY)
                .color(u.faint)
                .px(10.0)
                .py(8.0),
        );
    } else {
        for (i, p) in s.projects.iter().enumerate() {
            list = list.child(project_row(s, u, i, p));
        }
    }

    let links = [
        (&icons::SEARCH, "Game index", Screen::Index),
        (&icons::HISTORY, "Patch notes", Screen::Patches),
        (&icons::GIT_COMPARE, "Impact", Screen::Impact),
        (&icons::PACKAGE, "Release", Screen::Release),
        (&icons::WRENCH, "Toolchain", Screen::Toolchain),
        (&icons::USER, "Character", Screen::Character),
    ];
    let mut tools = col().key("agent-links").w_full().gap(0.0);
    for (glyph, label, screen) in links {
        tools = tools.child(link_row(u, glyph, label, screen));
    }

    col()
        .key("agent-threads")
        .w(THREADS_W)
        .h_full()
        .min_h(0.0)
        .fill(oklch(0.158, 0.002, 286.0))
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .px(8.0)
        .pt(8.0)
        .pb(12.0)
        .child(
            row()
                .key("agent-new-mod")
                .w_full()
                .h(36.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .press(Color::WHITE.alpha(0.06))
                .focusable(true)
                .role("button")
                .label("New mod")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.selected_project = st.projects.len();
                    st.open_tool_groups.clear();
                    st.begin_session();
                })
                .child(icon(&icons::SQUARE_PEN, 16.0).color(u.muted).a11y_hidden())
                .child(text("New mod").font(NAV).color(u.ink).a11y_hidden()),
        )
        .child(
            row()
                .w_full()
                .h(28.0)
                .mt(12.0)
                .pl(10.0)
                .pr(4.0)
                .center_y()
                .child(text("Projects").font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    row()
                        .key("agent-all-mods")
                        .h(24.0)
                        .px(6.0)
                        .center_y()
                        .radius(crate::ui::r(6.0))
                        .hover(u.hover)
                        .focusable(true)
                        .role("button")
                        .label("All projects")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Projects))
                        .child(text("All").font(SMALL).color(u.faint).a11y_hidden()),
                ),
        )
        .child(
            col()
                .key("agent-mods-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
        .child(tools)
}

fn project_row(s: &State, u: &Ui, i: usize, p: &Project) -> El {
    let on = i == s.selected_project;
    let mut r = row()
        .key(format!("agent-mod-{i}"))
        .w_full()
        .h(36.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .focusable(true)
        .role("button")
        .label(p.name.clone())
        .checked(on)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| st.selected_project = i)
        .child(
            text(p.name.clone())
                .font(NAV)
                .color(if on { u.ink } else { u.prose })
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        );
    r = if on {
        r.fill(u.selected)
    } else {
        r.hover(u.hover)
    };
    if let Some(ago) = relative(&p.updated) {
        r = r.child(text(ago).font(SMALL).color(u.faint).a11y_hidden());
    }
    r
}

fn link_row(u: &Ui, glyph: &'static rune::Icon, label: &'static str, screen: Screen) -> El {
    row()
        .key(format!("agent-link-{label}"))
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| st.go(screen))
        .child(icon(glyph, 16.0).color(u.faint).a11y_hidden())
        .child(text(label).font(BODY).color(u.muted).a11y_hidden())
}

// The conversation column.

fn conversation(s: &State, u: &Ui) -> El {
    let empty = !s
        .messages
        .iter()
        .any(|m| m.role == Role::User || m.role == Role::Tool);
    let title = s
        .project()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "New mod".to_string());

    let head = row()
        .key("agent-head")
        .w_full()
        .h(44.0)
        .px(GUTTER)
        .center_y()
        .child(text(title).font(BODY_MED).color(u.ink).truncate());

    let body = if empty {
        centred(empty_state(s, u))
            .key("agent-empty")
            .grow(1.0)
            .min_h(0.0)
            .pb(48.0)
            .justify_center()
    } else {
        col()
            .key("agent-transcript")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec { stick_bottom: true })
            .child(centred(transcript(s, u)))
    };

    let mut dock = col().w_full().gap(8.0);
    if !empty && let Some(line) = notice(s, u) {
        dock = dock.child(line);
    }
    if let Some(pause) = &s.agent.pause {
        dock = dock.child(approval(s, u, pause));
    }
    dock = dock.child(composer(s, u));

    col()
        .key("agent-conversation")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .child(head)
        .child(body)
        .child(centred(dock).key("agent-dock").pb(16.0))
}

/// Centre a column at the thread width with the side gutter.
fn centred(inner: El) -> El {
    col()
        .w_full()
        .px(GUTTER)
        .items_center()
        .child(inner.w_full().max_w(THREAD_MAX))
}

fn host_ready(s: &State) -> bool {
    s.agent.demo || (s.agent.configured && s.agent.status != AgentStatus::Error)
}

fn empty_state(s: &State, u: &Ui) -> El {
    let mut c = col().w_full().gap(0.0).items_center().child(
        text("What should we make?")
            .font(HEADLINE)
            .color(u.ink)
            .align(rune::Align::Center),
    );
    if !s.agent.demo && !s.agent.configured {
        return c
            .child(
                text("Sign in to a model in Settings")
                    .font(CHAT)
                    .color(u.muted)
                    .mt(8.0),
            )
            .child(
                u.action("Open Settings", ActionKind::Secondary)
                    .key("agent-open-settings")
                    .mt(20.0)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            );
    }
    if !host_ready(s) {
        return c
            .child(text("The agent stopped").font(CHAT).color(u.muted).mt(8.0))
            .child(
                u.action("Restart", ActionKind::Secondary)
                    .key("agent-restart")
                    .mt(20.0)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.restart_host()),
            );
    }
    let mut list = col().key("agent-suggestions").w(440.0).mt(28.0).gap(2.0);
    for (n, line) in SUGGESTIONS.iter().enumerate() {
        let fill = line.to_string();
        list = list.child(
            row()
                .key(format!("agent-suggestion-{n}"))
                .w_full()
                .h(40.0)
                .px(12.0)
                .gap(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .press(Color::WHITE.alpha(0.06))
                .focusable(true)
                .role("button")
                .label(*line)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.composer = TextEdit::new(fill.clone()))
                .child(icon(&icons::ARROW_UP, 16.0).color(u.faint).a11y_hidden())
                .child(
                    text(*line)
                        .font(CHAT)
                        .color(u.prose)
                        .truncate()
                        .a11y_hidden(),
                ),
        );
    }
    c = c.child(list);
    c
}

fn transcript(s: &State, u: &Ui) -> El {
    let welcome = model::welcome_messages()
        .into_iter()
        .next()
        .map(|m| m.body)
        .unwrap_or_default();
    let mut list = col()
        .key("agent-thread")
        .w_full()
        .pt(8.0)
        .pb(24.0)
        .gap(16.0);
    let msgs = &s.messages;
    let mut i = 0;
    while i < msgs.len() {
        let m = &msgs[i];
        match m.role {
            Role::Tool => {
                let start = i;
                while i < msgs.len() && msgs[i].role == Role::Tool {
                    i += 1;
                }
                list = list.child(work_log(s, u, start, &msgs[start..i]));
                continue;
            }
            Role::User => list = list.child(user_message(u, i, m)),
            Role::Assistant => {
                if !m.body.trim().is_empty() && m.body != welcome {
                    list = list.child(assistant_message(u, i, m));
                }
            }
        }
        i += 1;
    }
    if running(s) {
        list = list.child(working(s, u));
    }
    list
}

fn running(s: &State) -> bool {
    s.agent.pause.is_none()
        && (s.agent.status == AgentStatus::Running
            || s.tools.iter().any(|t| t.state == RunState::Running))
}

fn user_message(u: &Ui, idx: usize, m: &Message) -> El {
    row()
        .key(format!("msg-{idx}"))
        .w_full()
        .child(spacer())
        .child(
            col()
                .max_w(THREAD_MAX * 0.8)
                .px(16.0)
                .py(12.0)
                .radius(crate::ui::r(18.0))
                .fill(Color::WHITE.alpha(0.04))
                .child(
                    text(m.body.clone())
                        .font(CHAT)
                        .color(u.ink)
                        .wrap()
                        .selectable(true),
                ),
        )
}

fn assistant_message(u: &Ui, idx: usize, m: &Message) -> El {
    col().key(format!("msg-{idx}")).w_full().child(
        text(m.body.clone())
            .font(CHAT)
            .color(u.prose)
            .wrap()
            .selectable(true),
    )
}

/// Consecutive tool calls. One call is one line; several fold under "Ran N tools" unless one of
/// them is still running, in which case the lines stay open so the work is visible.
fn work_log(s: &State, u: &Ui, start: usize, msgs: &[Message]) -> El {
    let tools: Vec<&ToolRun> = msgs.iter().filter_map(|m| m.tool.as_ref()).collect();
    let live = tools.iter().any(|t| t.state != RunState::Done);
    let mut log = col().key(format!("msg-{start}")).w_full().gap(0.0);
    if tools.len() < 2 || live {
        for (n, t) in tools.iter().enumerate() {
            log = log.child(tool_line(u, &format!("tool-{start}-{n}"), t));
        }
        return log;
    }
    let open = s.open_tool_groups.contains(&start);
    let failed = tools.iter().filter(|t| failed(t)).count();
    let mut summary = format!("Ran {} tools", tools.len());
    if failed > 0 {
        summary.push_str(&format!(", {failed} failed"));
    }
    log = log.child(
        row()
            .key(format!("tools-{start}"))
            .h(24.0)
            .gap(8.0)
            .pr(8.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .focusable(true)
            .role("button")
            .label(summary.clone())
            .expanded(open)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if let Some(at) = st.open_tool_groups.iter().position(|g| *g == start) {
                    st.open_tool_groups.remove(at);
                } else {
                    st.open_tool_groups.push(start);
                }
            })
            .child(
                icon(
                    if open {
                        &icons::CHEVRON_DOWN
                    } else {
                        &icons::CHEVRON_RIGHT
                    },
                    16.0,
                )
                .color(u.faint)
                .a11y_hidden(),
            )
            .child(text(summary).font(BODY).color(u.muted).a11y_hidden()),
    );
    if open {
        let mut lines = col().w_full().pl(24.0);
        for (n, t) in tools.iter().enumerate() {
            lines = lines.child(tool_line(u, &format!("tool-{start}-{n}"), t));
        }
        log = log.child(lines);
    }
    log
}

fn tool_line(u: &Ui, key: &str, t: &ToolRun) -> El {
    let bad = failed(t);
    let (done, live) = tool_label(&t.name);
    let (glyph, tint) = if bad {
        (&icons::CIRCLE_X, u.rose)
    } else if t.state == RunState::Running {
        (&icons::LOADER, u.muted)
    } else {
        (tool_icon(&t.name), u.faint)
    };
    let label = match t.state {
        RunState::Running => format!("{live}..."),
        RunState::Queued => live,
        RunState::Done => done,
    };
    let mut r = row()
        .key(key.to_string())
        .w_full()
        .h(24.0)
        .gap(8.0)
        .center_y()
        .child(icon(glyph, 16.0).color(tint).a11y_hidden())
        .child(text(label).font(BODY).color(u.muted));
    match result_hint(t) {
        Some(hint) => {
            r = r.child(
                text(hint)
                    .font(BODY)
                    .color(u.faint)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
        }
        None => r = r.child(spacer()),
    }
    if !t.duration.is_empty() && t.state == RunState::Done {
        r = r.child(text(t.duration.clone()).font(SMALL).color(u.faint));
    }
    r
}

/// The first line of a tool result, unless it is structured output.
fn result_hint(t: &ToolRun) -> Option<String> {
    if t.state != RunState::Done {
        return None;
    }
    let line = t.result.lines().next().unwrap_or("").trim();
    if line.is_empty()
        || line == "Done."
        || line == "Running"
        || line.starts_with('{')
        || line.starts_with('[')
    {
        return None;
    }
    Some(line.trim_end_matches('.').to_string())
}

fn failed(t: &ToolRun) -> bool {
    let lower = t.result.trim_start().to_lowercase();
    t.state == RunState::Done
        && (lower.starts_with("tool failed")
            || lower.starts_with("error")
            || lower.starts_with("denied"))
}

/// Past and present tense labels for the tools the host exposes. Unknown tools fall back to
/// their name in words.
fn tool_label(name: &str) -> (String, String) {
    let pair = match name {
        "read_project" | "watch_projects" => ("Read project", "Reading project"),
        "game_status" | "read_game_status" => ("Checked the game", "Checking the game"),
        "game_index" => ("Built the game index", "Building the game index"),
        "search_paths" => ("Searched paths", "Searching paths"),
        "extract_paths" => ("Extracted files", "Extracting files"),
        "project_check" => ("Checked the mod", "Checking the mod"),
        "project_validate" => ("Validated the mod", "Validating the mod"),
        "validate_payload" => ("Validated payload", "Validating payload"),
        "project_pack" | "pack_vpk" => ("Packed the mod", "Packing the mod"),
        "project_release" => ("Prepared a release", "Preparing a release"),
        "project_migrate" => ("Moved paths", "Moving paths"),
        "deploy_plan" | "install_plan" => ("Planned the install", "Planning the install"),
        "deploy_apply" | "install_addon" => ("Installed into the game", "Install into the game"),
        "install_status" => ("Checked installed mods", "Checking installed mods"),
        "publish_prepare" => ("Prepared the upload", "Preparing the upload"),
        "publish_upload" => ("Published", "Publish to GameBanana"),
        "patch_notes" => ("Read patch notes", "Reading patch notes"),
        "player_stats" => ("Read player stats", "Reading player stats"),
        "player_heatmap" => ("Read the heatmap", "Reading the heatmap"),
        "hero_performance" => ("Read hero stats", "Reading hero stats"),
        "launch_status" => ("Checked launch options", "Checking launch options"),
        "game_launch" => ("Launched the game", "Launch the game"),
        "gameinfo_repair" => ("Repaired gameinfo.gi", "Repair gameinfo.gi"),
        "toolchain_status" => ("Checked the toolchain", "Checking the toolchain"),
        "demo_matches" => ("Listed matches", "Listing matches"),
        "demo_import" => ("Imported a demo", "Importing a demo"),
        "demo_review_prompt" => ("Read the match", "Reading the match"),
        "demo_review_write" => ("Wrote the review", "Writing the review"),
        "profiles_list" => ("Listed profiles", "Listing profiles"),
        "locker_heroes" => ("Read heroes", "Reading heroes"),
        "recolor_brief" => ("Drafted the recolor", "Drafting the recolor"),
        _ => {
            let words = name.replace('_', " ");
            let mut chars = words.chars();
            let words = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Tool".to_string(),
            };
            return (words.clone(), words);
        }
    };
    (pair.0.to_string(), pair.1.to_string())
}

fn tool_icon(name: &str) -> &'static rune::Icon {
    if name.contains("search") || name.contains("index") {
        &icons::SEARCH
    } else if name.contains("validate") || name.contains("check") {
        &icons::CIRCLE_CHECK
    } else if name.contains("pack") || name.contains("release") {
        &icons::PACKAGE
    } else if name.contains("extract") {
        &icons::DOWNLOAD
    } else if name.contains("deploy") || name.contains("install") {
        &icons::FOLDER_OPEN
    } else if name.contains("launch") {
        &icons::PLAY
    } else if name.contains("stats") || name.contains("heatmap") || name.contains("performance") {
        &icons::CHART_LINE
    } else if name.contains("publish") {
        &icons::GLOBE
    } else if name.contains("recolor") {
        &icons::SPARKLES
    } else if name.contains("read")
        || name.contains("status")
        || name.contains("notes")
        || name.contains("list")
        || name.contains("project")
    {
        &icons::FILE
    } else {
        &icons::WRENCH
    }
}

fn working(s: &State, u: &Ui) -> El {
    let secs = s
        .run_started
        .map(|t| t.elapsed().as_secs())
        .unwrap_or_else(|| parse_elapsed(&s.elapsed));
    // The text breathes on Rune's loop engine, which wakes at 30 fps and never runs the view;
    // a pulse kept in state rebuilt the whole window on every display frame.
    const BREATHE: rune::Cycle = rune::Cycle {
        period: 1.333,
        phase: 0.0,
        stops: &[(0.0, 0.55), (0.5, 1.0), (1.0, 0.55)],
        still: 1.0,
        fps: 30.0,
    };
    row().key("agent-working").h(24.0).center_y().child(
        text(format!("Working for {}:{:02}", secs / 60, secs % 60))
            .font(CHAT)
            .color(u.muted)
            .cycle(BREATHE),
    )
}

/// `2m 14s` or `9s` back to seconds.
fn parse_elapsed(label: &str) -> u64 {
    let mut total = 0;
    for part in label.split_whitespace() {
        if let Some(m) = part.strip_suffix('m') {
            total += m.parse::<u64>().unwrap_or(0) * 60;
        } else if let Some(sec) = part.strip_suffix('s') {
            total += sec.parse::<u64>().unwrap_or(0);
        }
    }
    total
}

/// One calm line above the composer when the conversation cannot continue.
fn notice(s: &State, u: &Ui) -> Option<El> {
    if s.agent.demo {
        return None;
    }
    let (line, action) = if !s.agent.configured {
        (
            "Sign in to a model in Settings",
            u.action("Open Settings", ActionKind::Ghost)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.go(Screen::Settings)),
        )
    } else if s.agent.status == AgentStatus::Error || s.agent.status == AgentStatus::Stopped {
        (
            "The agent stopped",
            u.action("Restart", ActionKind::Ghost)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.restart_host()),
        )
    } else {
        return None;
    };
    Some(
        row()
            .key("agent-notice")
            .w_full()
            .h(32.0)
            .pl(16.0)
            .gap(8.0)
            .center_y()
            .child(text(line).font(BODY).color(u.muted))
            .child(spacer())
            .child(action),
    )
}

fn approval(_s: &State, u: &Ui, pause: &Pause) -> El {
    let (_, ask) = tool_label(&pause.tool_name);
    let payload = payload_summary(&pause.payload);
    col()
        .key("agent-approval")
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .radius(crate::ui::r(12.0))
        .fill(u.amber.alpha(0.08))
        .border(1.0, u.amber.alpha(0.28))
        .role("alert")
        .label(format!("Approval needed: {ask}"))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    icon(&icons::TRIANGLE_ALERT, 16.0)
                        .color(u.amber)
                        .a11y_hidden(),
                )
                .child(text("Approval needed").font(BODY_MED).color(u.ink))
                .child(text(ask).font(BODY).color(u.muted).truncate()),
        )
        .child(
            text(payload)
                .font(MONO)
                .color(u.muted)
                .wrap_anywhere()
                .max_lines(3)
                .selectable(true),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(4.0)
                .center_y()
                .child(spacer())
                .child(
                    u.action("Deny", ActionKind::Secondary)
                        .key("deny")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.deny_pause()),
                )
                .child(
                    u.action("Approve", ActionKind::Primary)
                        .key("approve")
                        .autofocus()
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.approve_pause()),
                ),
        )
}

/// `slot 12  to game/citadel/addons/pak12_dir.vpk` rather than raw JSON: top level keys and
/// plain values, nested values compacted.
fn payload_summary(value: &serde_json::Value) -> String {
    use serde_json::Value;
    let plain = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{}:\u{a0}{}", k.replace('_', " "), plain(v)))
            .collect::<Vec<_>>()
            .join("   "),
        other => plain(other),
    }
}

fn composer(s: &State, u: &Ui) -> El {
    let paused = s.agent.pause.is_some();
    let field = if paused {
        rune::text_input(&TextEdit::new(""), CHAT)
            .key("composer-paused")
            .label("Prompt")
            .disabled(true)
            .placeholder("Resolve this approval to continue", u.faint)
    } else {
        rune::text_input(&s.composer, CHAT)
            .key("composer")
            .label("Prompt")
            .color(u.ink)
            .placeholder("Describe the mod you want", u.faint)
            .caret(u.accent, u.accent.alpha(0.22))
            .on_edit(|st: &mut State| &mut st.composer)
            .on_submit(|st: &mut State, _| st.send_prompt())
    }
    .w_full()
    .min_h(48.0)
    .max_h(192.0);

    let busy = running(s);
    let can_send = !paused && !s.composer.text.trim().is_empty() && host_ready(s);
    let action = if busy {
        round_button(
            u,
            &icons::SQUARE_STOP,
            "Stop",
            Color::WHITE.alpha(0.10),
            u.ink,
        )
        .key("abort")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.abort_run())
    } else if can_send {
        round_button(u, &icons::ARROW_UP, "Send", u.accent, u.accent_ink)
            .key("send")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.send_prompt())
    } else {
        round_button(
            u,
            &icons::ARROW_UP,
            "Send",
            Color::WHITE.alpha(0.06),
            u.faint,
        )
        .key("send")
        .disabled(true)
    };

    let model = s
        .agent
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "Choose a model".to_string());
    let picker = row()
        .key("agent-model")
        .h(28.0)
        .pl(8.0)
        .pr(6.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("Model: {model}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.go(Screen::Settings))
        .child(text(model).font(BODY).color(u.muted).a11y_hidden())
        .child(
            icon(&icons::CHEVRON_DOWN, 14.0)
                .color(u.faint)
                .a11y_hidden(),
        );

    col()
        .key("composer-box")
        .w_full()
        .radius(crate::ui::r(22.0))
        .fill(u.raised)
        .border(1.0, u.hair)
        .px(16.0)
        .pt(14.0)
        .pb(12.0)
        .gap(8.0)
        .child(field)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(picker.ml(-8.0))
                .child(spacer())
                .child(action),
        )
}

fn round_button(_u: &Ui, glyph: &'static rune::Icon, label: &str, fill: Color, ink: Color) -> El {
    el().size(32.0, 32.0)
        .radius(16.0)
        .center()
        .fill(fill)
        .hover(Color::WHITE.alpha(0.10))
        .press(Color::WHITE.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .child(icon(glyph, 16.0).color(ink).a11y_hidden())
}

/// `2026-11-15 18:04` (UTC) as `now`, `5m`, `3h`, `2d`, `3w`, or nothing when it does not parse.
pub(crate) fn relative(stamp: &str) -> Option<String> {
    let stamp = stamp.trim();
    let date = stamp.get(0..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let (mut hh, mut mm) = (0i64, 0i64);
    if let Some(time) = stamp.get(11..16) {
        let mut t = time.split(':');
        hh = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        mm = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    let then = days_from_civil(y, m, d) * 86_400 + hh * 3_600 + mm * 60;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let diff = (now - then).max(0);
    Some(if diff < 60 {
        "now".to_string()
    } else if diff < 3_600 {
        format!("{}m", diff / 60)
    } else if diff < 86_400 {
        format!("{}h", diff / 3_600)
    } else if diff < 7 * 86_400 {
        format!("{}d", diff / 86_400)
    } else if diff < 60 * 86_400 {
        format!("{}w", diff / (7 * 86_400))
    } else {
        format!("{}mo", diff / (30 * 86_400))
    })
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
