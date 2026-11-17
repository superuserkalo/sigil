//! Projects: the mods being made. A list on the left, the selected project on the right as
//! plain rows: its facts, the exact paths it replaces, references and history.

use rune::{El, ScrollSpec, col, row, text};

use crate::app::State;
use crate::model::{Project, ProjectState, Screen};
use crate::screens::workbench::relative;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, PAGE_TOP, PAGE_X, SMALL, Ui, sentence};

const LIST_W: f32 = 280.0;

pub fn view(s: &State, u: &Ui) -> El {
    let detail = match s.projects.len() {
        0 => String::new(),
        1 => "1 project".to_string(),
        n => format!("{n} projects"),
    };
    let actions = row().gap(8.0).center_y().child(
        u.action("New mod", ActionKind::Secondary)
            .key("projects-new")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.selected_project = st.projects.len();
                st.open_tool_groups.clear();
                st.begin_session();
                st.go(Screen::Agent);
            }),
    );

    let body = if s.projects.is_empty() {
        u.empty_state(
            "",
            "No projects yet",
            "Ask the agent for a mod and it starts one here.",
            Some(
                u.action("Open Agent", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Agent)),
            ),
        )
    } else {
        row()
            .key("projects-body")
            .w_full()
            .gap(32.0)
            .items_start()
            .child(list(s, u))
            .child(detail_pane(s, u))
    };

    col()
        .key("projects-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("projects-page")
                .w_full()
                .px(PAGE_X)
                .pt(PAGE_TOP)
                .pb(32.0)
                .gap(16.0)
                .child(u.page_head("", "Projects", &detail, Some(actions)))
                .child(body),
        )
}

fn list(s: &State, u: &Ui) -> El {
    let mut rows = col().key("project-list").w(LIST_W).gap(2.0);
    for (i, p) in s.projects.iter().enumerate() {
        rows = rows.child(project_row(s, u, i, p));
    }
    rows
}

fn project_row(s: &State, u: &Ui, i: usize, p: &Project) -> El {
    let on = i == s.selected_project;
    let mut meta = format!("v{}", p.version);
    if p.slot != 0 {
        meta.push_str(&format!("  ·  slot {}", p.slot_label()));
    }
    let mut r = row()
        .key(format!("project-{i}"))
        .w_full()
        .h(52.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .focusable(true)
        .role("button")
        .label(p.name.clone())
        .checked(on)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| st.selected_project = i)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(p.name.clone())
                        .font(BODY_MED)
                        .color(if on { u.ink } else { u.prose })
                        .truncate(),
                )
                .child(text(meta).font(SMALL).color(u.faint).truncate()),
        );
    r = if on {
        r.fill(u.selected)
    } else {
        r.hover(u.hover)
    };
    if p.state == ProjectState::Attention {
        r = r.child(u.dot(crate::ui::Tone::Warn));
    }
    if let Some(ago) = relative(&p.updated) {
        r = r.child(text(ago).font(SMALL).color(u.faint));
    }
    r
}

fn detail_pane(s: &State, u: &Ui) -> El {
    let Some(p) = s.project() else {
        return col()
            .key("project-detail")
            .grow(1.0)
            .min_w(0.0)
            .child(u.empty_state(
                "",
                "Pick a project",
                "Its paths, references and history show up here.",
                None,
            ));
    };

    let head = row()
        .w_full()
        .gap(16.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(text(p.name.clone()).font(H1).color(u.ink).truncate())
                .child(text(p.summary.clone()).font(BODY).color(u.muted).wrap()),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    u.action("Check", ActionKind::Secondary)
                        .key("project-check")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Impact)),
                )
                .child(
                    u.action("Open in Agent", ActionKind::Primary)
                        .key("project-open-agent")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Agent)),
                ),
        );

    let state_color = match p.state {
        ProjectState::Attention => u.amber,
        _ => u.prose,
    };
    let state_label = match p.state {
        ProjectState::Ready => "Ready",
        ProjectState::Attention => "Needs a look",
        ProjectState::Draft => "Draft",
    };
    let facts = col()
        .w_full()
        .child(u.kv("Version", &p.version, u.prose))
        .child(u.kv("Slot", &p.slot_file(), u.prose))
        .child(u.kv("Updated", &p.updated, u.prose))
        .child(u.kv("State", state_label, state_color));

    col()
        .key("project-detail")
        .grow(1.0)
        .min_w(0.0)
        .gap(24.0)
        .child(col().w_full().gap(16.0).child(head).child(facts))
        .child(targets(u, p))
        .child(references(u, p))
        .child(history(u, p))
}

fn section_head(u: &Ui, title: &str, count: usize) -> El {
    row()
        .w_full()
        .h(32.0)
        .gap(6.0)
        .center_y()
        .child(text(title.to_string()).font(BODY_MED).color(u.muted))
        .child(text(count.to_string()).font(BODY).color(u.faint))
}

fn hair_row(u: &Ui, h: f32) -> El {
    row()
        .w_full()
        .h(h)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
}

fn targets(u: &Ui, p: &Project) -> El {
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if p.targets.is_empty() {
        rows =
            rows.child(hair_row(u, 44.0).child(text("No targets yet").font(BODY).color(u.faint)));
    }
    for (i, t) in p.targets.iter().enumerate() {
        let state = t.status.tone().fg(u);
        let state_color = match t.status.tone() {
            crate::ui::Tone::Warn | crate::ui::Tone::Bad => state,
            _ => u.faint,
        };
        rows = rows.child(
            hair_row(u, 44.0)
                .key(format!("target-{i}"))
                .child(
                    text(t.path.clone())
                        .font(MONO)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate_head(),
                )
                .child(text(sentence(&t.kind)).font(SMALL).color(u.faint).w(72.0))
                .child(
                    text(capital(t.status.label()))
                        .font(SMALL)
                        .color(state_color)
                        .w(80.0)
                        .align(rune::Align::End),
                ),
        );
    }
    col()
        .key("targets")
        .w_full()
        .child(section_head(u, "Targets", p.targets.len()))
        .child(rows)
}

fn references(u: &Ui, p: &Project) -> El {
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if p.references.is_empty() {
        rows = rows.child(hair_row(u, 44.0).child(text("None").font(BODY).color(u.faint)));
    }
    for r in &p.references {
        rows = rows.child(
            hair_row(u, 44.0)
                .child(
                    text(r.vpk.clone())
                        .font(MONO)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(text(capital(&r.mode)).font(SMALL).color(u.faint)),
        );
    }
    col()
        .key("references")
        .w_full()
        .child(section_head(u, "References", p.references.len()))
        .child(rows)
}

fn history(u: &Ui, p: &Project) -> El {
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if p.history.is_empty() {
        rows =
            rows.child(hair_row(u, 44.0).child(text("No history yet").font(BODY).color(u.faint)));
    }
    for h in &p.history {
        rows = rows.child(
            row()
                .w_full()
                .py(12.0)
                .gap(16.0)
                .items_start()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(h.version.clone())
                        .font(MONO)
                        .color(u.ink)
                        .w(96.0)
                        .mt(1.0)
                        .truncate(),
                )
                .child(
                    text(h.note.clone())
                        .font(BODY)
                        .color(u.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                )
                .child(text(h.date.clone()).font(SMALL).color(u.faint).mt(2.0)),
        );
    }
    col()
        .key("history")
        .w_full()
        .child(section_head(u, "History", p.history.len()))
        .child(rows)
}

fn capital(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
