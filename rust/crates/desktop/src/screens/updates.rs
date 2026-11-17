//! Updates: every project checked against the current game files, and the latest patch notes.
//!
//! The project check loads the game catalog, so it runs on a press or a scene load, never from a
//! view. The patch list is the same cached Steam news the Patches screen reads.

use rune::{Color, El, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::{PatchNote, Screen};
use crate::screens::{release, subpage};
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let running = s.review_job.is_running();
    let actions = row().gap(8.0).center_y().child(
        u.icon_action(
            &icons::REFRESH_CW,
            if running { "Checking" } else { "Check again" },
            ActionKind::Secondary,
        )
        .key("updates-scan")
        .disabled(running)
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| scan(st)),
    );

    let detail = match &s.release.review {
        Some(queue) if queue.items.is_empty() => String::new(),
        Some(queue) => queue.summary(),
        None => String::new(),
    };

    let mut body = col().w_full().gap(32.0).child(projects(s, u));
    body = body.child(patches(s, u));

    let mut head =
        col()
            .w_full()
            .gap(12.0)
            .child(subpage::head(u, "Updates", &detail, Some(actions)));
    if let Some(watch) = &s.watch {
        head = head.child(
            text(format!(
                "Last automatic check {}, {} of {} need review.",
                watch.updated_at, watch.needs_review, watch.total_projects
            ))
            .key("updates-watch")
            .font(SMALL)
            .color(u.faint),
        );
    }

    subpage::page("updates", head, body)
}

fn scan(st: &mut State) {
    if st.demo {
        st.status = "Demo data does not rescan.".to_string();
        return;
    }
    crate::data::start_review_scan(st);
}

fn section(u: &Ui, title: &str, meta: &str, trailing: Option<El>) -> El {
    let mut r = row()
        .w_full()
        .h(32.0)
        .gap(6.0)
        .center_y()
        .child(u.section(title));
    if !meta.is_empty() {
        r = r.child(text(meta.to_string()).font(BODY).color(u.faint));
    }
    if let Some(t) = trailing {
        r = r.child(spacer()).child(t);
    }
    r
}

fn projects(s: &State, u: &Ui) -> El {
    let trailing = s.project().map(|_| {
        u.action("Check impact", ActionKind::Ghost)
            .key("updates-impact")
            .mr(-12.0)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.go(Screen::Impact))
    });
    let count = s
        .release
        .review
        .as_ref()
        .filter(|q| !q.items.is_empty())
        .map(|q| q.items.len().to_string())
        .unwrap_or_default();
    let mut c = col()
        .key("updates-queue")
        .w_full()
        .gap(8.0)
        .child(section(u, "Projects", &count, trailing));

    let Some(queue) = &s.release.review else {
        return c.child(
            col()
                .w_full()
                .py(16.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text("Check your projects against the current game files.")
                        .font(BODY)
                        .color(u.muted),
                ),
        );
    };

    if queue.items.is_empty() {
        let (message, color, action) = match &queue.error {
            Some(error) => (
                error.clone(),
                u.amber,
                u.action("Open Settings", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
            None => (
                "No projects yet.".to_string(),
                u.muted,
                u.action("Open projects", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Projects)),
            ),
        };
        return c.child(
            col()
                .w_full()
                .py(16.0)
                .gap(12.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(message).font(BODY).color(color).wrap())
                .child(row().child(action)),
        );
    }
    if let Some(error) = &queue.error {
        c = c.child(text(error.clone()).font(BODY).color(u.amber).wrap());
    }
    let mut list = subpage::rows(u);
    for (i, item) in queue.items.iter().enumerate() {
        list = list.child(release::review_row(u, i, item).key(format!("update-item-{i}")));
    }
    c.child(list).child(
        text(queue.workspace.clone())
            .font(SMALL)
            .color(u.faint)
            .pt(4.0)
            .truncate(),
    )
}

fn patches(s: &State, u: &Ui) -> El {
    let trailing = (!s.patches.is_empty()).then(|| {
        u.action("All patches", ActionKind::Ghost)
            .key("updates-open-patches")
            .mr(-12.0)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.go(Screen::Patches))
    });
    let c = col()
        .key("updates-patches")
        .w_full()
        .gap(8.0)
        .child(section(u, "Recent patches", "", trailing));
    if s.patches.is_empty() {
        return c.child(
            col()
                .w_full()
                .py(16.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(text("No patch notes yet.").font(BODY).color(u.muted)),
        );
    }
    let mut list = subpage::rows(u);
    for (i, note) in s.patches.iter().take(4).enumerate() {
        list = list.child(patch_row(u, i, note));
    }
    c.child(list)
}

fn patch_row(u: &Ui, i: usize, note: &PatchNote) -> El {
    let changes = match note.changed.len() {
        0 => String::new(),
        1 => "1 change".to_string(),
        n => format!("{n} changes"),
    };
    row()
        .key(format!("updates-patch-{i}"))
        .w_full()
        .h(44.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .fill(Color::TRANSPARENT)
        .focusable(true)
        .role("button")
        .label(note.title.clone())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.go(Screen::Patches);
            st.selected_patch = i;
        })
        .child(text(note.date.clone()).font(SMALL).color(u.faint).w(80.0))
        .child(
            text(note.title.clone())
                .font(BODY_MED)
                .color(u.ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(text(changes).font(SMALL).color(u.muted))
}
