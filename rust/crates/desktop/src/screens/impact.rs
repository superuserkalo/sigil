//! Impact: compatibility cards, gameinfo and slot warnings, and patch correlation.

use rune::{El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{Compat, ImpactCard, Screen, WarnLevel, Warning};
use crate::ui::{ActionKind, BODY_MED, H1, LABEL, MONO, MONO_MED, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let Some(report) = &s.impact else {
        let trailing = row()
            .gap(8.0)
            .center_y()
            .child(u.status_chip("no report", Tone::Quiet))
            .child(
                u.action("Check project", ActionKind::Primary).on_press(|st: &mut State| {
                    st.status =
                        "Impact needs the catalog and baseline loaded. This build does not load them, so no report was produced."
                            .to_string();
                }),
            );
        return col()
            .key("impact-page")
            .grow(1.0)
            .h_full()
            .min_h(0.0)
            .px(24.0)
            .pt(20.0)
            .pb(18.0)
            .gap(14.0)
            .child(u.page_head(
                Screen::Impact.kicker(),
                "Impact",
                "What a patch does to the selected project: unchanged, changed, missing or moved.",
                Some(trailing),
            ))
            .child(el().grow(1.0).min_h(0.0).child(u.empty_state(
                "⚑",
                "No impact report",
                "A report compares project targets with the current game index. Open a project, then run a check.",
                Some(
                    u.action("Open projects", ActionKind::Secondary)
                        .on_press(|st: &mut State| st.screen = Screen::Projects),
                ),
            )));
    };

    let checked = report.checked_at.clone();
    let detail = format!(
        "Checked {checked} against the cached index. {} targets, {} warnings.",
        report.total(),
        report.warnings.len()
    );
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(u.status_chip(
            if report.missing > 0 || !report.warnings.is_empty() { "needs review" } else { "compatible" },
            if report.missing > 0 || !report.warnings.is_empty() { Tone::Warn } else { Tone::Ok },
        ))
        .child(
            u.action("Re-check", ActionKind::Primary).on_press(|st: &mut State| {
                st.status =
                    "Re-check needs the catalog and baseline loaded. This build does not load them, so no report changed."
                        .to_string();
            }),
        )
        .child(
            u.action("Open project", ActionKind::Secondary)
                .on_press(|st: &mut State| st.screen = Screen::Projects),
        );

    let mut left = col().key("impact-left").w_full().gap(11.0);
    for w in &report.warnings {
        left = left.child(warning(u, w));
    }
    left = left.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(text("PROJECT TARGETS").font(LABEL).color(u.faint))
            .child(spacer())
            .child(
                text(format!("{} cards", report.cards.len()))
                    .font(SMALL)
                    .color(u.faint),
            ),
    );
    for (i, c) in report.cards.iter().enumerate() {
        left = left.child(card(u, i, c));
    }

    let right = col()
        .key("impact-right")
        .w(340.0)
        .gap(11.0)
        .child(project_panel(s, u, report))
        .child(match &report.correlation {
            Some(c) => correlation(u, c),
            None => u
                .panel()
                .padding(14.0)
                .gap(6.0)
                .child(text("PATCH CORRELATION").font(LABEL).color(u.faint))
                .child(
                    text("No patch note correlates with these targets yet.")
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                ),
        })
        .child(legend(u));

    col()
        .key("impact-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(18.0)
        .gap(14.0)
        .child(u.page_head(
            Screen::Impact.kicker(),
            &format!("{} impact", report.project),
            &detail,
            Some(trailing),
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(u.stat(format!("{}", report.unchanged), "UNCHANGED", Tone::Ok))
                .child(u.stat(format!("{}", report.changed), "CHANGED", Tone::Warn))
                .child(u.stat(format!("{}", report.missing), "MISSING", Tone::Bad))
                .child(u.stat(format!("{}", report.moved), "MOVED", Tone::Info)),
        )
        .child(
            row()
                .key("impact-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(
                    col()
                        .key("impact-scroll")
                        .grow(1.0)
                        .min_h(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(left),
                )
                .child(
                    col()
                        .key("impact-rail-scroll")
                        .w(340.0)
                        .h_full()
                        .min_h(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(right),
                ),
        )
}

fn warning(u: &Ui, w: &Warning) -> El {
    let tone = match w.level {
        WarnLevel::Block => Tone::Bad,
        WarnLevel::Warn => Tone::Warn,
    };
    let label = w.action.clone();
    u.panel()
        .key("warning")
        .w_full()
        .padding(13.0)
        .fill(tone.fg(u).alpha(0.045))
        .border(1.0, tone.fg(u).alpha(0.22))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(u.glyph_box("!", tone, 28.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(3.0)
                        .child(text(w.title.clone()).font(BODY_MED).color(u.ink).wrap())
                        .child(text(w.detail.clone()).font(SMALL).color(u.muted).wrap()),
                )
                .child(
                    u.action(&label, ActionKind::Secondary).on_press(move |st: &mut State| {
                        st.status = if label == "Repair gameinfo" {
                            "gameinfo repair is an approval-gated app action. This build does not write to the game install."
                                .to_string()
                        } else {
                            format!("{label} is not wired in this build. Nothing changed.")
                        };
                    }),
                ),
        )
}

fn card(u: &Ui, i: usize, c: &ImpactCard) -> El {
    let tone = c.status.tone();
    let mut paths = col()
        .w_full()
        .gap(4.0)
        .padding(10.0)
        .radius(7.0)
        .fill(u.well)
        .child(
            row()
                .gap(6.0)
                .center_y()
                .child(text("project path").font(LABEL).color(u.faint))
                .child(spacer())
                .child(u.chip(c.status.label(), tone.fg(u), tone.bg(u))),
        )
        .child(
            text(c.old_path.clone())
                .font(MONO)
                .color(u.muted)
                .wrap_anywhere(),
        );
    if let Some(new_path) = &c.new_path {
        paths = paths
            .child(
                row()
                    .gap(6.0)
                    .pt(3.0)
                    .center_y()
                    .child(text("current path").font(LABEL).color(u.teal))
                    .child(spacer())
                    .child(u.chip("moved here", u.teal, u.teal.alpha(0.10))),
            )
            .child(
                text(new_path.clone())
                    .font(MONO)
                    .color(u.teal)
                    .wrap_anywhere(),
            );
    }

    u.panel()
        .key(format!("impact-card-{i}"))
        .w_full()
        .padding(13.0)
        .gap(9.0)
        .border(1.0, tone.fg(u).alpha(0.16))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_center()
                .child(u.status_chip(c.status.headline(), tone))
                .child(
                    text(c.target.clone())
                        .font(MONO_MED)
                        .color(u.ink)
                        .grow(1.0)
                        .truncate(),
                ),
        )
        .child(paths)
        .child(text(c.reason.clone()).font(SMALL).color(u.muted).wrap())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text("ACTION").font(LABEL).color(u.faint).mt(2.0))
                .child(
                    text(c.action.clone())
                        .font(SMALL)
                        .color(u.ink)
                        .grow(1.0)
                        .wrap(),
                ),
        )
}

fn project_panel(s: &State, u: &Ui, report: &crate::model::ImpactReport) -> El {
    let slot = s
        .project()
        .map_or("not assigned".to_string(), |p| p.slot_file());
    u.panel()
        .key("impact-project")
        .padding(14.0)
        .gap(6.0)
        .child(text("PROJECT").font(LABEL).color(u.faint))
        .child(text(report.project.clone()).font(H1).color(u.ink))
        .child(u.divider())
        .child(u.kv("Addon slot", &slot, u.ink))
        .child(u.kv("Targets", &format!("{}", report.total()), u.ink))
        .child(u.kv("Checked", &report.checked_at, u.muted))
        .child(u.note("A check reads citadel, core and maps. It never writes."))
}

fn correlation(u: &Ui, c: &crate::model::Correlation) -> El {
    u.panel()
        .key("correlation")
        .padding(14.0)
        .gap(7.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(text("PATCH CORRELATION").font(LABEL).color(u.faint))
                .child(spacer())
                .child(u.chip("1 note", u.accent, u.accent.alpha(0.11))),
        )
        .child(text(c.patch.clone()).font(BODY_MED).color(u.ink).wrap())
        .child(text(c.date.clone()).font(MONO).color(u.faint))
        .child(text(c.detail.clone()).font(SMALL).color(u.muted).wrap())
        .child(
            u.action("Open patch note", ActionKind::Secondary)
                .on_press(|st: &mut State| {
                    st.screen = Screen::Patches;
                    st.selected_patch = 0;
                    st.status =
                        "Opened the correlated patch note from the cached feed.".to_string();
                }),
        )
}

fn legend(u: &Ui) -> El {
    let items = [
        (Compat::Unchanged, "Path and size match the current index."),
        (
            Compat::Changed,
            "Path exists, the content upstream changed.",
        ),
        (Compat::Missing, "Not in citadel, core or the map VPKs."),
        (Compat::Moved, "Path was renamed. Update the manifest."),
    ];
    let mut body = col().gap(8.0);
    for (c, detail) in items {
        body = body.child(
            row()
                .w_full()
                .gap(9.0)
                .items_start()
                .child(el().w(82.0).child(u.compat_chip(c)))
                .child(text(detail).font(SMALL).color(u.muted).grow(1.0).wrap()),
        );
    }
    u.panel()
        .key("legend")
        .padding(14.0)
        .gap(9.0)
        .child(text("STATUS").font(LABEL).color(u.faint))
        .child(body)
}
