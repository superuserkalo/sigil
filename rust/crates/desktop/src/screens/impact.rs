//! Impact: what the latest game update does to the open project, target by target.

use rune::{Color, El, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::model::{Compat, ImpactCard, ImpactReport, Screen, WarnLevel, Warning};
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, MONO_MED, SMALL, Ui};

const STATUS_W: f32 = 136.0;
const TARGET_W: f32 = 360.0;

pub fn view(s: &State, u: &Ui) -> El {
    let Some(report) = &s.impact else {
        let actions = row().gap(8.0).center_y().child(
            u.action("Check project", ActionKind::Primary)
                .key("impact-check")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.status = "Checking a project is not available yet.".to_string();
                }),
        );
        return subpage::page(
            "impact",
            subpage::head(u, "Impact", "", Some(actions)),
            u.empty_state(
                "",
                "No impact report",
                "Open a project and check it against the current game files.",
                Some(
                    u.action("Open projects", ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Projects)),
                ),
            ),
        );
    };

    let slot = s.project().map(|p| p.slot_file());
    let mut detail = vec![report.project.clone()];
    if let Some(slot) = slot {
        detail.push(slot);
    }
    detail.push(format!("Checked {}", report.checked_at));

    let actions = row()
        .gap(8.0)
        .center_y()
        .child(
            u.action("Open project", ActionKind::Secondary)
                .key("impact-open-project")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.go(Screen::Projects)),
        )
        .child(
            u.icon_action(&icons::REFRESH_CW, "Re-check", ActionKind::Primary)
                .key("impact-recheck")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.status = "Re-checking is not available yet.".to_string();
                }),
        );

    let mut body = col().w_full().gap(24.0);
    if !report.warnings.is_empty() {
        let mut list = subpage::rows(u);
        for (i, w) in report.warnings.iter().enumerate() {
            list = list.child(warning(u, i, w));
        }
        body = body.child(
            col()
                .w_full()
                .gap(8.0)
                .child(section(u, "Problems", &report.warnings.len().to_string()))
                .child(list),
        );
    }
    body = body.child(targets(u, report));
    if let Some(c) = &report.correlation {
        body = body.child(correlation(u, c));
    }

    subpage::page(
        "impact",
        subpage::head(u, "Impact", &detail.join("  ·  "), Some(actions)),
        body,
    )
}

fn warning(u: &Ui, i: usize, w: &Warning) -> El {
    let color = match w.level {
        WarnLevel::Block => u.rose,
        WarnLevel::Warn => u.amber,
    };
    let label = w.action.clone();
    row()
        .key(format!("impact-warning-{i}"))
        .w_full()
        .py(14.0)
        .gap(12.0)
        .items_start()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            el().size(20.0, 20.0).center().child(
                icon(&icons::TRIANGLE_ALERT, 16.0)
                    .color(color)
                    .a11y_hidden(),
            ),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(w.title.clone()).font(BODY_MED).color(u.ink).wrap())
                .child(text(w.detail.clone()).font(BODY).color(u.muted).wrap()),
        )
        .child(
            u.action(&label, ActionKind::Secondary)
                .key(format!("impact-warning-action-{i}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    st.status = format!("{label} is not available yet.");
                }),
        )
}

fn targets(u: &Ui, report: &ImpactReport) -> El {
    let summary = [
        (report.unchanged, "unchanged"),
        (report.changed, "changed"),
        (report.missing, "missing"),
        (report.moved, "moved"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, label)| format!("{n} {label}"))
    .collect::<Vec<_>>()
    .join(", ");

    let head_cell = |label: &str| text(label.to_string()).font(SMALL).color(u.faint);
    let mut table = u.panel().key("impact-targets").w_full().child(
        row()
            .w_full()
            .h(32.0)
            .px(16.0)
            .gap(16.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(head_cell("Target").w(TARGET_W))
            .child(head_cell("Status").w(STATUS_W))
            .child(head_cell("What to do").grow(1.0)),
    );
    let last = report.cards.len().saturating_sub(1);
    for (i, c) in report.cards.iter().enumerate() {
        table = table.child(card_row(u, i, c, i == last));
    }

    col()
        .w_full()
        .gap(8.0)
        .child(section(u, "Targets", &summary))
        .child(table)
}

fn card_row(u: &Ui, i: usize, c: &ImpactCard, last: bool) -> El {
    let status_color = match c.status {
        Compat::Changed => u.amber,
        Compat::Missing => u.rose,
        Compat::Unchanged | Compat::Moved => u.muted,
    };
    let mut target = col()
        .w(TARGET_W)
        .gap(2.0)
        .child(
            text(c.target.clone())
                .font(MONO_MED)
                .color(u.ink)
                .truncate(),
        )
        .child(
            text(c.old_path.clone())
                .font(MONO)
                .color(u.faint)
                .truncate(),
        );
    if let Some(new_path) = &c.new_path {
        target = target.child(
            row()
                .w_full()
                .gap(6.0)
                .items_start()
                .child(text("Now").font(SMALL).color(u.muted).mt(1.0))
                .child(
                    text(new_path.clone())
                        .font(MONO)
                        .color(u.prose)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
        );
    }
    let mut r = row()
        .key(format!("impact-card-{i}"))
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(16.0)
        .items_start()
        .child(target)
        .child(
            text(c.status.headline())
                .font(BODY)
                .color(status_color)
                .w(STATUS_W),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(c.action.clone()).font(BODY).color(u.ink).wrap())
                .child(text(c.reason.clone()).font(SMALL).color(u.muted).wrap()),
        );
    if !last {
        r = r.border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    }
    r.fill(Color::TRANSPARENT)
}

fn correlation(u: &Ui, c: &crate::model::Correlation) -> El {
    col()
        .key("impact-correlation")
        .w_full()
        .gap(8.0)
        .child(section(u, "Related patch", ""))
        .child(
            subpage::rows(u)
                .child(subpage::kv(u, "Note", subpage::val(u, c.patch.clone())))
                .child(subpage::kv(
                    u,
                    "Date",
                    text(c.date.clone()).font(BODY).color(u.muted),
                ))
                .child(subpage::kv(
                    u,
                    "Overlap",
                    text(c.detail.clone()).font(BODY).color(u.prose).wrap(),
                )),
        )
        .child(
            row().w_full().pt(8.0).child(
                u.action("Open patch note", ActionKind::Secondary)
                    .key("impact-open-patch")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.go(Screen::Patches);
                        st.selected_patch = 0;
                    }),
            ),
        )
}

fn section(u: &Ui, title: &str, meta: &str) -> El {
    let mut r = row()
        .w_full()
        .h(32.0)
        .gap(6.0)
        .center_y()
        .child(u.section(title));
    if !meta.is_empty() {
        r = r.child(text(meta.to_string()).font(BODY).color(u.faint));
    }
    r
}
