//! Patches: official notes from Steam news on the left, the selected note on the right.

use rune::{Color, El, ScrollSpec, col, el, row, text};

use crate::app::State;
use crate::icons;
use crate::model::{PatchNote, Screen};
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let new_count = s.patches.iter().filter(|p| p.is_new).count();
    let actions = row().gap(8.0).center_y().child(
        u.icon_action(&icons::REFRESH_CW, "Refresh", ActionKind::Secondary)
            .key("patches-refresh")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.status = "Refreshing the patch feed is not available yet.".to_string();
            }),
    );
    let detail = if s.patches.is_empty() {
        String::new()
    } else if new_count > 0 {
        format!("{} notes, {} new", s.patches.len(), new_count)
    } else {
        format!("{} notes", s.patches.len())
    };
    let head = subpage::head(u, "Patches", &detail, Some(actions));

    if s.patches.is_empty() {
        return subpage::page(
            "patches",
            head,
            u.empty_state(
                "",
                "No patch notes yet",
                "Official notes from Steam news show up here after a refresh.",
                None,
            ),
        );
    }

    subpage::fixed_page(
        "patches",
        head,
        row()
            .key("patches-body")
            .w_full()
            .min_h(0.0)
            .child(list(s, u))
            .child(detail_pane(s, u)),
    )
}

fn list(s: &State, u: &Ui) -> El {
    let mut rows = col()
        .key("patch-list-inner")
        .w_full()
        .pr(12.0)
        .pb(24.0)
        .gap(2.0);
    for (i, p) in s.patches.iter().enumerate() {
        rows = rows.child(note_row(s, u, i, p));
    }
    col()
        .key("patch-list")
        .w(320.0)
        .h_full()
        .min_h(0.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .child(
            u.scroll_area(col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows)),
        )
}

fn note_row(s: &State, u: &Ui, i: usize, p: &PatchNote) -> El {
    let on = i == s.selected_patch;
    let mut meta = vec![p.date.clone()];
    if !p.build.is_empty() {
        meta.push(p.build.clone());
    }
    if !p.changed.is_empty() {
        meta.push(plural(p.changed.len(), "change"));
    }
    let mut top = row().w_full().gap(8.0).center_y().child(
        text(p.title.clone())
            .font(BODY_MED)
            .color(if on { u.ink } else { u.prose })
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
    );
    if p.is_new {
        top = top.child(text("New").font(SMALL).color(u.muted));
    }
    col()
        .key(format!("patch-{i}"))
        .w_full()
        .h(56.0)
        .px(12.0)
        .gap(2.0)
        .justify_center()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(p.title.clone())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.selected_patch = i;
        })
        .child(top)
        .child(
            text(meta.join("  ·  "))
                .font(SMALL)
                .color(u.faint)
                .truncate(),
        )
}

fn detail_pane(s: &State, u: &Ui) -> El {
    let Some(note) = s.selected_note() else {
        return col()
            .key("patch-detail")
            .grow(1.0)
            .h_full()
            .child(u.empty_state(
                "",
                "Pick a note",
                "Select a patch note on the left to read it.",
                None,
            ));
    };

    let mut meta = vec![note.date.clone()];
    if !note.build.is_empty() {
        meta.push(format!("Build {}", note.build));
    }

    let mut page = col()
        .key("patch-detail-body")
        .max_w(800.0)
        .pl(32.0)
        .pr(8.0)
        .pb(32.0)
        .gap(24.0)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(text(note.title.clone()).font(H1).color(u.ink).wrap())
                .child(
                    text(meta.join("  ·  "))
                        .font(SMALL)
                        .color(u.faint)
                        .truncate(),
                ),
        );

    if !note.changed.is_empty() {
        let mut list = col().w_full().gap(8.0).child(u.section("What changed"));
        for c in &note.changed {
            list = list.child(
                row()
                    .w_full()
                    .gap(10.0)
                    .items_start()
                    .child(
                        el().size(4.0, 4.0)
                            .radius(2.0)
                            .fill(u.faint)
                            .mt(8.0)
                            .a11y_hidden(),
                    )
                    .child(
                        text(c.clone())
                            .font(BODY)
                            .color(u.prose)
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap(),
                    ),
            );
        }
        page = page.child(list);
    }

    if !note.body.is_empty() {
        let mut body = col().key("patch-copy").w_full().gap(8.0);
        for para in &note.body {
            body = body.child(
                text(para.clone())
                    .font(BODY)
                    .color(u.prose)
                    .wrap()
                    .selectable(true),
            );
        }
        page = page.child(body);
    }

    page = page.child(touched(s, u, note));

    u.scroll_area(col()
        .key("patch-detail")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page))
}

fn touched(s: &State, u: &Ui, note: &PatchNote) -> El {
    let project = s.project().map(|p| p.name.clone());
    let mut section = col().w_full().gap(8.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(u.section("Paths touched"))
            .child(
                text(note.targets.len().to_string())
                    .font(BODY)
                    .color(u.faint),
            ),
    );
    if note.targets.is_empty() {
        section = section.child(
            text("No paths were matched to this note.")
                .font(BODY)
                .color(u.muted),
        );
    } else {
        let mut rows = subpage::rows(u);
        for path in &note.targets {
            rows = rows.child(
                row()
                    .w_full()
                    .h(40.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        text(path.clone())
                            .font(MONO)
                            .color(u.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    ),
            );
        }
        section = section.child(rows);
    }
    let label = match &project {
        Some(name) => format!("Check impact on {name}"),
        None => "Check impact".to_string(),
    };
    section.child(
        row().w_full().pt(8.0).child(
            u.action(&label, ActionKind::Secondary)
                .key("patch-check-impact")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.go(Screen::Impact)),
        ),
    )
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}
