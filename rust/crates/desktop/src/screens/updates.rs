//! Updates: the review queue and the patch feed in one place.
//!
//! The queue is the same read `dah watch` does: every project's baseline is
//! compared against the current catalog. It loads the catalog, so it runs on a
//! worker, never from a view. The patch list is the cached Steam news feed that
//! the Patches screen already reads.

use rune::{Color, El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{PatchNote, ReviewItem, ReviewQueue, Screen};
use crate::ui::{ActionKind, BODY_MED, H1, LABEL, MONO, RAIL_W, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(match &s.release.review {
            Some(queue) => u.status_chip(&queue.summary(), review_tone(queue)),
            None => u.status_chip("queue not run", Tone::Quiet),
        })
        .child(if s.review_job.is_running() {
            u.status_chip("scanning", Tone::Warn)
        } else {
            u.pill("Run review queue", "↻", ActionKind::Secondary)
                .key("updates-scan")
                .on_press(|st: &mut State| scan(st))
        });

    let detail = match &s.release.review {
        Some(queue) if queue.error.is_some() => {
            "The queue ran, but a required input is missing. The exact reason is on the card."
                .to_string()
        }
        Some(queue) => format!(
            "{} · {} project{} in the workspace",
            queue.catalog_label,
            queue.items.len(),
            if queue.items.len() == 1 { "" } else { "s" }
        ),
        None => "Compare every project baseline with the current patch feed and game catalog."
            .to_string(),
    };

    col()
        .key("updates-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "updates-glow",
            [0.16, 0.0],
            (720.0, 460.0),
            u.glow_warm,
        ))
        .child(u.page_head(
            Screen::Updates.kicker(),
            "Updates",
            &detail,
            Some(trailing),
        ))
        .child(watch_strip(s, u))
        .child(
            row()
                .key("updates-body")
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(queue_panel(s, u))
                .child(rail(s, u)),
        )
}

fn scan(st: &mut State) {
    if st.demo {
        st.status =
            "Demo fixture: the review queue stays on the fixture. Nothing was read.".to_string();
        return;
    }
    crate::data::start_review_scan(st);
}

fn review_tone(queue: &ReviewQueue) -> Tone {
    if queue.error.is_some() {
        Tone::Bad
    } else if queue.review_count() > 0 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}

fn queue_panel(s: &State, u: &Ui) -> El {
    let meta = match &s.release.review {
        Some(queue) => queue.summary(),
        None => "not scanned".to_string(),
    };
    let mut body = col().w_full().padding(16.0).gap(9.0);

    match &s.release.review {
        Some(queue) => {
            if let Some(error) = &queue.error {
                body = body.child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .items_start()
                        .padding(11.0)
                        .radius(10.0)
                        .fill(u.rose.alpha(0.05))
                        .border(1.0, u.rose.alpha(0.18))
                        .child(u.glyph_box("!", Tone::Bad, 26.0))
                        .child(
                            text(error.clone())
                                .font(SMALL)
                                .color(u.muted)
                                .grow(1.0)
                                .wrap(),
                        ),
                );
            }
            if queue.items.is_empty() {
                body = body
                    .child(
                        text("No mod projects in this workspace")
                            .font(BODY_MED)
                            .color(u.muted),
                    )
                    .child(
                        text("The queue reads <workspace>/mods/*/project.json and <workspace>/project.json. Create a project, then scan again.")
                            .font(SMALL)
                            .color(u.faint)
                            .wrap(),
                    )
                    .child(
                        row().w_full().child(
                            u.action("Open Projects", ActionKind::Secondary)
                                .on_press(|st: &mut State| st.go(Screen::Projects)),
                        ),
                    );
            } else {
                for (index, item) in queue.items.iter().enumerate() {
                    body = body.child(item_card(u, index, item));
                }
            }
            body = body.child(
                text(format!("Workspace · {}", queue.workspace))
                    .font(MONO)
                    .color(u.faint)
                    .truncate(),
            );
        }
        None => {
            body = body
                .child(
                    col()
                        .w_full()
                        .padding(14.0)
                        .gap(5.0)
                        .child(text("The queue has not run yet").font(BODY_MED).color(u.muted))
                        .child(
                            text("It loads the game catalog and diffs every project baseline. Run it explicitly; it never runs per frame and never writes a project file.")
                                .font(SMALL)
                                .color(u.faint)
                                .wrap(),
                        ),
                )
                .child(
                    u.pill("Run review queue", "↻", ActionKind::Primary)
                        .on_press(|st: &mut State| scan(st)),
                );
        }
    }

    col()
        .key("updates-queue")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(
            row()
                .w_full()
                .px(16.0)
                .pt(13.0)
                .pb(11.0)
                .gap(9.0)
                .center_y()
                .child(text("REVIEW QUEUE").font(LABEL).color(u.faint))
                .child(spacer())
                .child(text(meta).font(SMALL).color(u.faint))
                .child(u.divider().absolute(0.0, 38.0)),
        )
        .child(
            col()
                .key("updates-queue-scroll")
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
}

fn item_card(u: &Ui, index: usize, item: &ReviewItem) -> El {
    let tone = item_tone(item);
    let mut card = col()
        .key(format!("update-item-{index}"))
        .w_full()
        .padding(12.0)
        .gap(6.0)
        .radius(11.0)
        .fill(Color::WHITE.alpha(0.02))
        .border(1.0, tone.fg(u).alpha(0.14))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.dot(tone))
                .child(
                    text(item.label())
                        .font(BODY_MED)
                        .color(u.ink)
                        .grow(1.0)
                        .truncate(),
                )
                .child(u.chip(
                    &format!("slot {}", item.slot_label()),
                    u.accent,
                    u.accent.alpha(0.10),
                ))
                .child(u.status_chip(
                    if item.needs_review() {
                        "needs review"
                    } else {
                        "clean"
                    },
                    tone,
                )),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(format!("v{}", item.version)).font(MONO).color(u.faint))
                .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                .child(
                    text(item.path.clone())
                        .font(MONO)
                        .color(u.faint)
                        .grow(1.0)
                        .truncate(),
                ),
        );

    let mut counts = row().w_full().gap(6.0).center_y();
    if item.changed > 0 {
        counts = counts.child(u.status_chip(&format!("changed {}", item.changed), Tone::Warn));
    }
    if item.missing > 0 {
        counts = counts.child(u.status_chip(&format!("missing {}", item.missing), Tone::Bad));
    }
    if item.moved > 0 {
        counts = counts.child(u.status_chip(&format!("moved {}", item.moved), Tone::Info));
    }
    if item.unbaselined > 0 {
        counts = counts.child(u.status_chip(
            &format!("stale baseline {}", item.unbaselined),
            Tone::Warn,
        ));
    }
    if !item.migrations.is_empty() {
        counts = counts.child(u.status_chip(
            &format!("{} path move{}", item.migrations.len(), if item.migrations.len() == 1 { "" } else { "s" }),
            Tone::Info,
        ));
    }
    if item.unchanged > 0 {
        counts = counts.child(
            text(format!("{} unchanged", item.unchanged))
                .font(SMALL)
                .color(u.faint),
        );
    }
    if !item.needs_review() {
        counts = counts.child(u.status_chip("targets match the baseline", Tone::Ok));
    }
    card = card.child(counts);

    for warning in item.warnings.iter().take(2) {
        card = card.child(
            row()
                .w_full()
                .gap(8.0)
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
    if let Some(error) = &item.error {
        card = card.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(u.chip("check failed", u.rose, u.rose.alpha(0.12)))
                .child(
                    text(error.clone())
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .wrap(),
                ),
        );
    }
    card
}

fn item_tone(item: &ReviewItem) -> Tone {
    if item.error.is_some() || item.missing > 0 {
        Tone::Bad
    } else if item.changed > 0
        || item.unbaselined > 0
        || !item.migrations.is_empty()
        || !item.warnings.is_empty()
    {
        Tone::Warn
    } else if item.moved > 0 {
        Tone::Info
    } else {
        Tone::Ok
    }
}

fn rail(s: &State, u: &Ui) -> El {
    col()
        .key("updates-rail")
        .w(RAIL_W)
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(12.0)
        .child(project_card(s, u))
        .child(patch_card(s, u))
        .child(
            col()
                .w_full()
                .padding(14.0)
                .gap(6.0)
                .radius(12.0)
                .fill(Color::WHITE.alpha(0.02))
                .border(1.0, u.hair)
                .child(text("WHAT A STALE BASELINE MEANS").font(LABEL).color(u.faint))
                .child(
                    text("A project baseline records the game files it was packed against. When the feed moves an asset path, the baseline no longer matches and the project needs a re-check before the next release.")
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                ),
        )
}

fn project_card(s: &State, u: &Ui) -> El {
    let mut body = col().w_full().padding(16.0).gap(7.0);
    match s.project() {
        Some(project) => {
            body = body
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(u.dot(match project.state {
                            crate::model::ProjectState::Ready => Tone::Ok,
                            crate::model::ProjectState::Attention => Tone::Warn,
                            crate::model::ProjectState::Draft => Tone::Quiet,
                        }))
                        .child(
                            text(project.name.clone())
                                .font(H1)
                                .color(u.ink)
                                .grow(1.0)
                                .truncate(),
                        )
                        .child(u.chip(
                            &format!("slot {}", project.slot_label()),
                            u.accent,
                            u.accent.alpha(0.11),
                        )),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text(format!("v{}", project.version)).font(MONO).color(u.faint))
                        .child(text("·").font(SMALL).color(u.faint).a11y_hidden())
                        .child(text(project.state.label()).font(SMALL).color(u.muted)),
                )
                .child(
                    text(format!(
                        "{} target{}, {} reference{}",
                        project.targets.len(),
                        if project.targets.len() == 1 { "" } else { "s" },
                        project.references.len(),
                        if project.references.len() == 1 { "" } else { "s" }
                    ))
                    .font(SMALL)
                    .color(u.faint),
                )
                .child(
                    row()
                        .w_full()
                        .pt(4.0)
                        .gap(7.0)
                        .child(
                            u.action("Open project", ActionKind::Secondary)
                                .grow(1.0)
                                .on_press(|st: &mut State| st.go(Screen::Projects)),
                        )
                        .child(
                            u.action("Impact", ActionKind::Ghost)
                                .grow(1.0)
                                .on_press(|st: &mut State| st.go(Screen::Impact)),
                        ),
                );
        }
        None => {
            body = body
                .child(text("No project open").font(BODY_MED).color(u.muted))
                .child(
                    text("A project baseline is what the queue diffs. Open a workspace project to see its status here.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                )
                .child(
                    row().w_full().child(
                        u.action("Open Projects", ActionKind::Secondary)
                            .on_press(|st: &mut State| st.go(Screen::Projects)),
                    ),
                );
        }
    }
    col()
        .key("updates-project")
        .w_full()
        .radius(12.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(body)
}

fn patch_card(s: &State, u: &Ui) -> El {
    let mut body = col()
        .w_full()
        .padding(16.0)
        .gap(9.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text("PATCH FEED").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!("{}", s.patches.len()))
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
    if s.patches.is_empty() {
        body = body
            .child(text("No cached patch notes").font(BODY_MED).color(u.muted))
            .child(
                text("The feed is read from the cached Steam news file. Run `dah watch` to fill it.")
                    .font(SMALL)
                    .color(u.faint)
                    .wrap(),
            );
    } else {
        for note in s.patches.iter().take(4) {
            body = body.child(patch_row(u, note));
        }
        body = body.child(
            u.action("Open Patches", ActionKind::Ghost)
                .w_full()
                .on_press(|st: &mut State| st.go(Screen::Patches)),
        );
    }
    col()
        .key("updates-patches")
        .w_full()
        .radius(12.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, u.hair)
        .child(body)
}

fn patch_row(u: &Ui, note: &PatchNote) -> El {
    col()
        .w_full()
        .padding(9.0)
        .gap(3.0)
        .radius(8.0)
        .fill(Color::WHITE.alpha(0.018))
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(text(note.date.clone()).font(MONO).color(u.faint))
                .child(spacer())
                .child(if note.changed.is_empty() {
                    u.chip("no paths", u.faint, Color::WHITE.alpha(0.035))
                } else {
                    u.chip(
                        &format!(
                            "{} change{}",
                            note.changed.len(),
                            if note.changed.len() == 1 { "" } else { "s" }
                        ),
                        u.amber,
                        u.amber.alpha(0.10),
                    )
                }),
        )
        .child(
            text(note.title.clone())
                .font(SMALL)
                .color(u.ink)
                .max_lines(2)
                .wrap(),
        )
}

/// The durable state the opt-in `dah watch --watch` loop writes. A missing file
/// means no loop has run; this is never inferred from the queue.
fn watch_strip(s: &State, u: &Ui) -> El {
    match &s.watch {
        Some(watch) => row()
            .key("updates-watch")
            .w_full()
            .padding(11.0)
            .gap(10.0)
            .center_y()
            .radius(10.0)
            .fill(u.violet.alpha(0.05))
            .border(1.0, u.violet.alpha(0.16))
            .child(u.dot(Tone::Info))
            .child(
                text(format!(
                    "Watch loop last ran {} at {}s interval over {} project(s)",
                    watch.updated_at, watch.interval_seconds, watch.total_projects
                ))
                .font(SMALL)
                .color(u.muted)
                .grow(1.0)
                .truncate(),
            )
            .child(u.chip(
                &format!("{} need review", watch.needs_review),
                if watch.needs_review == 0 { u.faint } else { u.amber },
                Color::WHITE.alpha(0.04),
            ))
            .child(
                text(
                    watch
                        .newest_patch
                        .clone()
                        .unwrap_or_else(|| "no patch recorded".to_string()),
                )
                .font(LABEL)
                .color(u.faint)
                .max_w(320.0)
                .truncate(),
            ),
        None => row()
            .key("updates-watch-none")
            .w_full()
            .padding(10.0)
            .gap(9.0)
            .center_y()
            .radius(10.0)
            .fill(Color::WHITE.alpha(0.02))
            .border(1.0, u.hair)
            .child(u.dot(Tone::Quiet))
            .child(
                text("No watch loop state yet. `dah watch --watch` writes .dah/watch-state.json; this queue is the same read, run on demand.")
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0)
                    .truncate(),
            ),
    }
}
