//! Projects: the project list and the selected project manifest.

use rune::tok::motion;
use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{Project, ProjectState, Screen};
use crate::ui::{ActionKind, BODY_MED, H1, LABEL, MONO, MONO_MED, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let count = if s.projects.is_empty() {
        "no projects".to_string()
    } else {
        format!(
            "{} project{}",
            s.projects.len(),
            if s.projects.len() == 1 { "" } else { "s" }
        )
    };
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(u.status_chip(
            &count,
            if s.projects.is_empty() { Tone::Quiet } else { Tone::Info },
        ))
        .child(u.action("New project", ActionKind::Secondary).on_press(|st: &mut State| {
            st.status = "Project creation is an app action: it writes a manifest into the workspace. This build does not write."
                .to_string();
        }));

    col()
        .key("projects-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(18.0)
        .gap(14.0)
        .child(u.page_head(
            Screen::Projects.kicker(),
            "Projects",
            "Each project is one workspace: a manifest, a slot and the exact paths it replaces.",
            Some(trailing),
        ))
        .child(
            row()
                .key("projects-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(list(s, u))
                .child(detail(s, u)),
        )
}

fn list(s: &State, u: &Ui) -> El {
    let meta = if s.projects.is_empty() {
        "empty".to_string()
    } else {
        format!("{} total", s.projects.len())
    };
    let mut rows = col()
        .key("project-list-inner")
        .w_full()
        .gap(5.0)
        .padding(8.0);
    if s.projects.is_empty() {
        rows = rows.child(
            col()
                .padding(14.0)
                .gap(4.0)
                .child(text("No projects yet").font(BODY_MED).color(u.muted))
                .child(
                    text("A manifest appears here once the workspace has one.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        );
    } else {
        for (i, p) in s.projects.iter().enumerate() {
            rows = rows.child(project_row(s, u, i, p));
        }
    }

    u.panel()
        .key("project-list")
        .w(300.0)
        .h_full()
        .min_h(0.0)
        .child(u.panel_header("WORKSPACE", &meta))
        .child(
            col()
                .key("project-list-scroll")
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        )
}

fn project_row(s: &State, u: &Ui, i: usize, p: &Project) -> El {
    let on = i == s.selected_project;
    let name = p.name.clone();
    let press_name = name.clone();
    let version = p.version.clone();
    let updated = p.updated.clone();
    let summary = p.summary.clone();
    let slot = p.slot_label();
    col()
        .key(format!("project-{i}"))
        .w_full()
        .padding(11.0)
        .gap(5.0)
        .radius(8.0)
        .fill(if on {
            u.accent.alpha(0.10)
        } else {
            Color::TRANSPARENT
        })
        .border(
            1.0,
            if on {
                u.accent.alpha(0.22)
            } else {
                Color::TRANSPARENT
            },
        )
        .hover(Color::WHITE.alpha(0.035))
        .press(Color::WHITE.alpha(0.05))
        .focusable(true)
        .role("button")
        .label(name.clone())
        .motion(motion::SNAPPY)
        .on_press(move |st: &mut State| {
            st.selected_project = i;
            st.status = format!("Selected {press_name}.");
        })
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.dot(project_tone(p.state)))
                .child(
                    text(name)
                        .font(BODY_MED)
                        .color(if on { u.ink } else { u.muted })
                        .grow(1.0)
                        .truncate(),
                )
                .child(u.chip(&slot, u.accent, u.accent.alpha(0.12))),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(version).font(MONO).color(u.faint))
                .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                .child(text(updated).font(SMALL).color(u.faint).truncate())
                .child(spacer()),
        )
        .child(text(summary).font(SMALL).color(u.faint).max_lines(2).wrap())
}

fn detail(s: &State, u: &Ui) -> El {
    let Some(p) = s.project() else {
        return col()
            .key("project-detail")
            .grow(1.0)
            .h_full()
            .min_h(0.0)
            .child(u.empty_state(
                "▤",
                "No project open",
                "The workbench reads manifests from the workspace. Set a workspace path in Settings, then open a project.",
                Some(
                    u.action("Go to Settings", ActionKind::Secondary)
                        .on_press(|st: &mut State| st.screen = Screen::Settings),
                ),
            ));
    };

    let mut scroll = col()
        .key("project-detail")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(14.0)
        .child(hero(s, u, p))
        .child(stats(u, p));

    scroll = scroll.child(manifest(u, p));
    scroll = scroll.child(
        row()
            .w_full()
            .gap(14.0)
            .items_start()
            .child(references(u, p))
            .child(history(u, p)),
    );
    scroll = scroll.child(u.note(
        "Payload rule: compiled _c files only, at exact game paths. Source assets, scripts/*.vdata_c and .vnmskel_c are rejected.",
    ));
    scroll
}

fn hero(s: &State, u: &Ui, p: &Project) -> El {
    let name = p.name.clone();
    u.panel()
        .key("project-hero")
        .padding(16.0)
        .child(
            row()
                .w_full()
                .gap(14.0)
                .items_center()
                .child(u.glyph_box("◆", Tone::Accent, 42.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(5.0)
                        .child(
                            row()
                                .gap(9.0)
                                .center_y()
                                .child(text(name.clone()).font(H1).color(u.ink))
                                .child(u.chip(
                                    &format!("slot {}", p.slot_label()),
                                    u.accent,
                                    u.accent.alpha(0.13),
                                ))
                                .child(u.status_chip(p.state.label(), project_tone(p.state))),
                        )
                        .child(text(p.summary.clone()).font(SMALL).color(u.muted).wrap()),
                )
                .child(
                    row()
                        .gap(7.0)
                        .center_y()
                        .child(
                            u.action("Check", ActionKind::Primary).on_press(|st: &mut State| {
                                let name = st.project().map_or("the project".to_string(), |p| p.name.clone());
                                st.screen = Screen::Impact;
                                st.status = format!(
                                    "Compatibility check ran for {name} against the cached game index. No game files were touched."
                                );
                            }),
                        )
                        .child(
                            u.action("Open session", ActionKind::Secondary)
                                .on_press(|st: &mut State| st.go(Screen::Agent)),
                        )
                        .child(
                            u.action("Reveal", ActionKind::Ghost).on_press(|st: &mut State| {
                                st.status =
                                    "Reveal is an app action. This build does not open a file manager."
                                        .to_string();
                            }),
                        ),
                ),
        )
        .child(
            row()
                .w_full()
                .pt(10.0)
                .gap(8.0)
                .center_y()
                .child(text(format!("v{}", p.version)).font(MONO).color(u.faint))
                .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                .child(text(format!("updated {}", p.updated)).font(SMALL).color(u.faint))
                .child(spacer())
                .child(text(if s.demo { "DEMO PROJECT" } else { "" }).font(LABEL).color(u.accent)),
        )
}

fn stats(u: &Ui, p: &Project) -> El {
    row()
        .w_full()
        .gap(9.0)
        .child(u.stat(p.slot_label(), "ADDON SLOT", Tone::Accent))
        .child(u.stat(format!("{}", p.targets.len()), "TARGETS", Tone::Info))
        .child(u.stat(format!("{}", p.references.len()), "REFERENCES", Tone::Quiet))
        .child(u.stat(
            p.history.first().map_or("none", |h| h.date.as_str()),
            "LAST CHANGE",
            Tone::Ok,
        ))
}

fn manifest(u: &Ui, p: &Project) -> El {
    let mut body = col().key("targets").w_full();
    body = body.child(u.table_head(&[("PATH", 0.0), ("KIND", 74.0), ("STATE", 88.0)]));
    for (i, t) in p.targets.iter().enumerate() {
        let tone = t.status.tone();
        if i > 0 {
            body = body.child(u.divider());
        }
        body = body.child(
            row()
                .w_full()
                .h(37.0)
                .px(14.0)
                .gap(12.0)
                .center_y()
                .child(
                    text(t.path.clone())
                        .font(MONO)
                        .color(u.ink)
                        .grow(1.0)
                        .truncate(),
                )
                .child(text(t.kind.clone()).font(SMALL).color(u.faint).w(74.0))
                .child(el().w(88.0).child(u.status_chip(t.status.label(), tone))),
        );
    }

    u.panel()
        .key("manifest")
        .w_full()
        .child(u.panel_header("MANIFEST TARGETS", &format!("{} paths", p.targets.len())))
        .child(body)
}

fn references(u: &Ui, p: &Project) -> El {
    let mut body = col().w_full().gap(0.0).padding(12.0);
    if p.references.is_empty() {
        body = body.child(text("No references declared.").font(SMALL).color(u.faint));
    } else {
        for r in &p.references {
            body = body.child(
                row()
                    .w_full()
                    .h(30.0)
                    .gap(8.0)
                    .center_y()
                    .child(text("▸").font(LABEL).color(u.faint).a11y_hidden())
                    .child(
                        text(r.vpk.clone())
                            .font(MONO)
                            .color(u.ink)
                            .grow(1.0)
                            .truncate(),
                    )
                    .child(u.chip(&r.mode, u.faint, Color::WHITE.alpha(0.035))),
            );
        }
    }
    u.panel()
        .key("references")
        .grow(0.46)
        .child(u.panel_header("REFERENCES", &format!("{} vpk", p.references.len())))
        .child(body)
}

fn history(u: &Ui, p: &Project) -> El {
    let mut body = col().w_full().gap(0.0).padding(12.0);
    if p.history.is_empty() {
        body = body.child(text("No history yet.").font(SMALL).color(u.faint));
    } else {
        for (i, h) in p.history.iter().enumerate() {
            body = body.child(
                row()
                    .w_full()
                    .gap(9.0)
                    .py(7.0)
                    .items_start()
                    .child(
                        el().size(5.0, 5.0)
                            .radius(3.0)
                            .fill(if i == 0 { u.accent } else { u.faint })
                            .mt(6.0),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .gap(1.0)
                            .child(
                                row()
                                    .gap(8.0)
                                    .center_y()
                                    .child(text(h.version.clone()).font(MONO_MED).color(u.ink))
                                    .child(text(h.date.clone()).font(SMALL).color(u.faint)),
                            )
                            .child(text(h.note.clone()).font(SMALL).color(u.muted).wrap()),
                    ),
            );
        }
    }
    u.panel()
        .key("history")
        .grow(0.54)
        .child(u.panel_header("HISTORY", &format!("{} entries", p.history.len())))
        .child(body)
}

fn project_tone(state: ProjectState) -> Tone {
    match state {
        ProjectState::Ready => Tone::Ok,
        ProjectState::Attention => Tone::Warn,
        ProjectState::Draft => Tone::Quiet,
    }
}
