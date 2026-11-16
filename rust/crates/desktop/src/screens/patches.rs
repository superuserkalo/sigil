//! Patches: the official Steam news list, new badges and the selected note with its impact summary.

use rune::tok::motion;
use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{PatchNote, Screen};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, LABEL, MONO, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let new_count = s.patches.iter().filter(|p| p.is_new).count();
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(u.status_chip(
            &format!("{} new", new_count),
            if new_count > 0 { Tone::Accent } else { Tone::Quiet },
        ))
        .child(u.action("Refresh feed", ActionKind::Secondary).on_press(|st: &mut State| {
            st.status =
                "Patch feed is not connected in this build. Showing the local cache; no network request was made."
                    .to_string();
        }));

    col()
        .key("patches-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(18.0)
        .gap(14.0)
        .child(u.page_head(
            Screen::Patches.kicker(),
            "Patches",
            "Official Deadlock notes from Steam news, paired with the targets each note touches.",
            Some(trailing),
        ))
        .child(
            row()
                .key("patches-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(list(s, u))
                .child(detail(s, u)),
        )
}

fn list(s: &State, u: &Ui) -> El {
    let mut rows = col().key("patch-list-inner").w_full().gap(5.0).padding(8.0);
    if s.patches.is_empty() {
        rows = rows.child(
            col()
                .padding(14.0)
                .gap(4.0)
                .child(text("No patch notes").font(BODY_MED).color(u.muted))
                .child(
                    text("The feed has not been fetched. Cached notes appear here.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        );
    } else {
        for (i, p) in s.patches.iter().enumerate() {
            rows = rows.child(note_row(s, u, i, p));
        }
    }

    u.panel()
        .key("patch-list")
        .w(360.0)
        .h_full()
        .min_h(0.0)
        .child(u.panel_header("STEAM NEWS", &format!("{} notes", s.patches.len())))
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        )
}

fn note_row(s: &State, u: &Ui, i: usize, p: &PatchNote) -> El {
    let on = i == s.selected_patch;
    let title = p.title.clone();
    let press_title = title.clone();
    col()
        .key(format!("patch-{i}"))
        .w_full()
        .padding(11.0)
        .gap(6.0)
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
        .label(title.clone())
        .motion(motion::SNAPPY)
        .on_press(move |st: &mut State| {
            st.selected_patch = i;
            st.status = format!("Opened {press_title}.");
        })
        .child(
            row()
                .w_full()
                .gap(7.0)
                .center_y()
                .child(text(p.date.clone()).font(SMALL).color(u.faint))
                .child(spacer())
                .child(if p.is_new {
                    u.chip("NEW", u.accent_ink, u.accent)
                } else if p.build.is_empty() {
                    el().h(0.0)
                } else {
                    u.chip(&p.build, u.faint, Color::WHITE.alpha(0.035))
                }),
        )
        .child(
            text(title)
                .font(BODY_MED)
                .color(if on { u.ink } else { u.muted })
                .max_lines(2)
                .wrap(),
        )
        .child({
            let mut meta = row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(if p.build.is_empty() {
                    text("cached note").font(MONO).color(u.faint)
                } else {
                    text(p.build.clone()).font(MONO).color(u.faint)
                });
            if !p.changed.is_empty() {
                meta = meta
                    .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                    .child(
                        text(format!(
                            "{} change{}",
                            p.changed.len(),
                            if p.changed.len() == 1 { "" } else { "s" }
                        ))
                        .font(SMALL)
                        .color(u.faint),
                    );
            }
            meta
        })
}

fn detail(s: &State, u: &Ui) -> El {
    let Some(note) = s.selected_note() else {
        return col()
            .key("patch-detail")
            .grow(1.0)
            .h_full()
            .min_h(0.0)
            .child(u.empty_state(
                "❐",
                "No note selected",
                "Patch notes from the official Steam feed appear here with a target correlation.",
                None,
            ));
    };

    let changed = if note.changed.is_empty() {
        u.note(
            "This feed note has no parsed target summary. Read the body, then correlate it with the project.",
        )
    } else {
        let mut block = col()
            .key("what-changed")
            .w_full()
            .gap(7.0)
            .padding(13.0)
            .radius(8.0)
            .fill(u.accent.alpha(0.055))
            .border(1.0, u.accent.alpha(0.16))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(text("WHAT CHANGED").font(LABEL).color(u.accent))
                    .child(spacer())
                    .child(
                        text(format!("{} targets", note.changed.len()))
                            .font(SMALL)
                            .color(u.faint),
                    ),
            );
        for c in &note.changed {
            block = block.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_start()
                    .child(
                        el().w(2.0)
                            .h(13.0)
                            .radius(1.0)
                            .fill(u.accent.alpha(0.7))
                            .mt(2.0)
                            .a11y_hidden(),
                    )
                    .child(text(c.clone()).font(SMALL).color(u.muted).grow(1.0).wrap()),
            );
        }
        block
    };

    let mut body = col().key("patch-copy").w_full().gap(9.0);
    for para in &note.body {
        body = body.child(
            text(para.clone())
                .font(BODY)
                .color(u.prose)
                .wrap()
                .selectable(true),
        );
    }

    let page = col()
        .key("patch-detail-body")
        .w_full()
        .gap(16.0)
        .padding(20.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.status_chip("official", Tone::Quiet))
                .child(text(note.source.clone()).font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    text(if note.build.is_empty() {
                        "cached feed".to_string()
                    } else {
                        format!("build {}", note.build)
                    })
                    .font(MONO)
                    .color(u.faint),
                ),
        )
        .child(text(note.title.clone()).font(H1).color(u.ink).wrap())
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(note.date.clone()).font(MONO).color(u.muted))
                .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                .child(
                    text(if note.is_new {
                        "new since last visit"
                    } else {
                        "seen"
                    })
                    .font(SMALL)
                    .color(if note.is_new { u.accent } else { u.faint }),
                ),
        )
        .child(changed)
        .child(body)
        .child(touched(s, u, note))
        .child(u.divider())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text("Correlation uses the cached index and the last impact check.")
                        .font(SMALL)
                        .color(u.faint)
                        .grow(1.0)
                        .wrap(),
                )
                .child(
                    u.action("Correlate with project", ActionKind::Secondary)
                        .on_press(|st: &mut State| {
                            st.screen = Screen::Impact;
                            st.status =
                                "Correlation opened from the cached note. No game files were read."
                                    .to_string();
                        }),
                ),
        );

    col()
        .key("patch-detail")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .child(
            u.panel().grow(1.0).min_h(0.0).child(
                col()
                    .key("patch-detail-scroll")
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(page),
            ),
        )
}

fn touched(s: &State, u: &Ui, note: &PatchNote) -> El {
    if note.targets.is_empty() {
        return el().h(0.0);
    }
    let mut rows = col().key("touched-list").w_full();
    for (i, path) in note.targets.iter().enumerate() {
        if i > 0 {
            rows = rows.child(u.divider());
        }
        rows = rows.child(
            row()
                .w_full()
                .h(35.0)
                .px(14.0)
                .gap(8.0)
                .center_y()
                .child(u.dot(Tone::Warn))
                .child(
                    text(path.clone())
                        .font(MONO)
                        .color(u.ink)
                        .grow(1.0)
                        .truncate(),
                )
                .child(u.chip("review", u.amber, u.amber.alpha(0.11))),
        );
    }

    let project = s
        .project()
        .map_or("No project open".to_string(), |p| p.name.clone());
    let target_panel = u
        .panel()
        .key("touched")
        .grow(1.0)
        .min_w(0.0)
        .child(u.panel_header("TARGETS TOUCHED", &format!("{} paths", note.targets.len())))
        .child(rows);

    let project_panel = u
        .panel()
        .key("patch-project")
        .w(252.0)
        .padding(13.0)
        .gap(6.0)
        .child(text("PROJECT CONTEXT").font(LABEL).color(u.faint))
        .child(text(project).font(BODY_MED).color(u.ink).truncate())
        .child(
            text("Review the impacted paths before the next pack.")
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        )
        .child(
            u.action("Review impact", ActionKind::Ghost)
                .on_press(|st: &mut State| {
                    st.screen = Screen::Impact;
                    st.status =
                        "Opened impact from the patch note. No game files were read.".to_string();
                }),
        );

    row()
        .w_full()
        .gap(10.0)
        .items_start()
        .child(target_panel)
        .child(project_panel)
}
