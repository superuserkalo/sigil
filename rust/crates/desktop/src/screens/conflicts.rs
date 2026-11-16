//! Conflicts: game paths that more than one enabled addon replaces.
//!
//! `read_conflict_report` does the comparison. Lower slot wins, so the winner
//! chip is the slot the game actually loads for that path. Nothing here changes
//! a slot; resolving a conflict is a user decision made by editing the addon
//! folder.

use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{ConflictRow, Screen};
use crate::ui::{ActionKind, LABEL, MONO_MED, SMALL, Tone, Ui};

const SHOWN_LIMIT: usize = 80;

pub fn view(s: &State, u: &Ui) -> El {
    let query = s.conflicts_query.text.trim().to_lowercase();
    let conflicts: Vec<&ConflictRow> = s
        .addons
        .conflicts
        .iter()
        .filter(|row| matches_query(row, &query))
        .collect();

    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.addon_job.is_running() {
            u.status_chip("scanning", Tone::Warn)
        } else if s.addons.scanned {
            u.status_chip(
                &format!("{} enabled slots", s.addons.enabled_count()),
                Tone::Info,
            )
        } else {
            u.status_chip("not scanned", Tone::Quiet)
        })
        .child(
            u.pill("Rescan", "↻", ActionKind::Ghost)
                .key("conflicts-rescan")
                .on_press(|st: &mut State| {
                    if st.demo {
                        st.status =
                            "Demo fixture: the conflict scan stays on the fixture. Nothing was read."
                                .to_string();
                    } else {
                        crate::data::start_addon_scan(st);
                    }
                }),
        );

    let detail = if !s.game_detected {
        "No Deadlock install was found, so no addon paths could be compared.".to_string()
    } else if !s.addons.scanned {
        "Comparing enabled slots in the background.".to_string()
    } else if s.addons.conflicts.is_empty() {
        "Every enabled slot ships distinct paths.".to_string()
    } else {
        format!(
            "{} shared path{} across {} enabled slot{} · lower slot wins",
            s.addons.conflicts.len(),
            if s.addons.conflicts.len() == 1 { "" } else { "s" },
            s.addons.enabled_count(),
            if s.addons.enabled_count() == 1 { "" } else { "s" }
        )
    };

    let mut page = col()
        .key("conflicts-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "conflicts-glow",
            [0.92, 0.0],
            (620.0, 460.0),
            u.glow_warm,
        ))
        .child(u.page_head(
            Screen::Conflicts.kicker(),
            "Conflicts",
            &detail,
            Some(trailing),
        ))
        .child(summary_strip(s, u));

    if !s.addons.warnings.is_empty() {
        page = page.child(warning_panel(s, u));
    }

    let body = if !s.game_detected {
        u.empty_state(
            "⚑",
            "No Deadlock install",
            "A conflict report needs the installed addon VPKs. Point Settings at a Deadlock folder, then rescan.",
            Some(
                u.action("Open Settings", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else if !s.addons.scanned {
        u.empty_state(
            "⚑",
            "Not read yet",
            "The addon scan compares the directory VPKs of every enabled slot. Rescan to run it.",
            Some(
                u.action("Scan addons", ActionKind::Primary)
                    .on_press(|st: &mut State| crate::data::start_addon_scan(st)),
            ),
        )
    } else if s.addons.conflicts.is_empty() {
        u.empty_state(
            "✓",
            "No conflicts",
            if s.addons.addons.is_empty() {
                "No addon slots are installed, so there are no shared paths."
            } else {
                "Every enabled addon ships a different set of game paths. Nothing is being overridden."
            },
            None,
        )
    } else if conflicts.is_empty() {
        u.empty_state(
            "⌕",
            "No path matches",
            "The search covers the game path and the slot numbers.",
            Some(
                u.action("Clear search", ActionKind::Ghost).on_press(|st: &mut State| {
                    st.conflicts_query = rune::TextEdit::new("");
                }),
            ),
        )
    } else {
        let mut list = col().w_full().gap(7.0);
        for (index, conflict) in conflicts.iter().take(SHOWN_LIMIT).enumerate() {
            list = list.child(conflict_row(u, index, conflict));
        }
        if conflicts.len() > SHOWN_LIMIT {
            list = list.child(
                text(format!(
                    "Showing the first {SHOWN_LIMIT} of {} conflicting paths.",
                    conflicts.len()
                ))
                .font(SMALL)
                .color(u.faint),
            );
        }
        col()
            .key("conflicts-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list)
    };

    page.child(body)
}

fn matches_query(row: &ConflictRow, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    row.path.to_lowercase().contains(query)
        || row
            .slots
            .iter()
            .any(|slot| format!("{slot:02}").contains(query))
}

fn summary_strip(s: &State, u: &Ui) -> El {
    let mut chips = row().gap(6.0).center_y();
    if s.addons.scanned {
        chips = chips
            .child(u.chip(
                &format!("{} addons", s.addons.addons.len()),
                u.muted,
                Color::WHITE.alpha(0.04),
            ))
            .child(u.chip(
                &format!("{} enabled", s.addons.enabled_count()),
                u.teal,
                u.teal.alpha(0.10),
            ))
            .child(u.chip(
                &format!("{} warnings", s.addons.warnings.len()),
                if s.addons.warnings.is_empty() {
                    u.faint
                } else {
                    u.amber
                },
                if s.addons.warnings.is_empty() {
                    Color::WHITE.alpha(0.035)
                } else {
                    u.amber.alpha(0.10)
                },
            ));
    }
    row()
        .key("conflicts-summary")
        .w_full()
        .padding(11.0)
        .gap(10.0)
        .center_y()
        .radius(10.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(text("RULE").font(LABEL).color(u.faint))
        .child(
            text("A mod replaces files by exact path. When two enabled slots ship the same path, the lower slot number loads first.")
                .font(SMALL)
                .color(u.muted)
                .grow(1.0)
                .truncate(),
        )
        .child(chips)
}

fn conflict_row(u: &Ui, index: usize, conflict: &ConflictRow) -> El {
    let mut slots = row().gap(5.0).center_y();
    for label in conflict.slot_labels() {
        let winner = label == format!("{:02}", conflict.winner);
        slots = slots.child(u.chip(
            &format!("slot {label}"),
            if winner { u.amber } else { u.muted },
            if winner {
                u.amber.alpha(0.12)
            } else {
                Color::WHITE.alpha(0.04)
            },
        ));
    }

    col()
        .key(format!("conflict-{index}"))
        .w_full()
        .padding(12.0)
        .gap(6.0)
        .radius(11.0)
        .fill(if index % 2 == 0 {
            Color::WHITE.alpha(0.022)
        } else {
            Color::WHITE.alpha(0.014)
        })
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    text(conflict.path.clone())
                        .font(MONO_MED)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate()
                        .selectable(true),
                )
                .child(slots)
                .child(u.status_chip(&conflict.winner_label(), Tone::Warn)),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(conflict.detail())
                        .font(SMALL)
                        .color(u.faint)
                        .grow(1.0)
                        .truncate(),
                )
                .child(
                    text(format!(
                        "{} slots",
                        conflict.slots.len()
                    ))
                        .font(LABEL)
                        .color(u.faint),
                ),
        )
}

fn warning_panel(s: &State, u: &Ui) -> El {
    let mut body = col().w_full().padding(12.0).gap(6.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(u.glyph_box("!", Tone::Warn, 24.0))
            .child(
                text("THE REPORT COULD NOT CHECK EVERYTHING")
                    .font(LABEL)
                    .color(u.amber),
            )
            .child(spacer())
            .child(
                text(format!("{}", s.addons.warnings.len()))
                    .font(SMALL)
                    .color(u.faint),
            ),
    );
    for warning in &s.addons.warnings {
        body = body.child(
            row()
                .w_full()
                .gap(9.0)
                .items_start()
                .child(
                    el().size(4.0, 4.0)
                        .radius(2.0)
                        .fill(u.amber.alpha(0.8))
                        .mt(6.0),
                )
                .child(
                    text(warning.clone())
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .wrap(),
                ),
        );
    }
    body = body.child(
        text("A VPK that cannot be read is reported instead of silently skipped, so the conflict list is never presented as complete when it is not.")
            .font(SMALL)
            .color(u.faint)
            .wrap(),
    );
    col()
        .key("conflict-warnings")
        .w_full()
        .radius(11.0)
        .fill(u.amber.alpha(0.045))
        .border(1.0, u.amber.alpha(0.16))
        .child(body)
}
