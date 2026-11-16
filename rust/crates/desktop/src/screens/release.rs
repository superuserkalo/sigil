//! Release: the latest build, its deploy plan, the publish folder and the review
//! queue. Every value on this screen comes from a read. Install and upload are
//! app actions, so this screen only ever sends prompts to the agent host.

use rune::{El, ScrollSpec, col, el, row, spacer, text};

use crate::app::State;
use crate::data;
use crate::model::{DeployPlanView, ReleaseInfo, ReviewItem, ReviewQueue, Screen};
use crate::ui::{ActionKind, BODY_MED, H1, LABEL, MONO, MONO_MED, RAIL_W, SMALL, Tone, Ui};

const PROMPT_VALIDATE: &str = "Validate and release this project. Read project.json, check every target against the game index, validate the staging payload and prepare builds/<version> with its release manifest. List every warning before packing. Do not write to the game install, edit gameinfo.gi or upload anything.";

const PROMPT_PLAN: &str = "Plan the deploy for this project. Read the latest release manifest and the installed addons, then report the target slot, whether a replacement is required, lower-slot conflicts and the gameinfo SearchPaths state. This plan is read only: do not copy files or edit gameinfo.gi.";

const PROMPT_MIGRATE: &str = "Migrate the moved paths for this project. Use the current game index to list the exact old and new path for each target that moved, explain any ambiguous basename instead of guessing, and update project.json only after I approve the plan. Do not touch the game install.";

pub fn view(s: &State, u: &Ui) -> El {
    let status = release_status(s, u);
    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(status)
        .child(
            u.action("Rescan release", ActionKind::Secondary)
                .on_press(|st: &mut State| rescan(st)),
        )
        .child(
            u.action("Open projects", ActionKind::Ghost)
                .on_press(|st: &mut State| st.screen = Screen::Projects),
        );

    let left = col()
        .key("release-left")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(12.0)
        .child(release_panel(s, u))
        .child(deploy_panel(s, u))
        .child(review_panel(s, u));

    let right = col()
        .key("release-right")
        .w(RAIL_W)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(12.0)
        .child(publish_panel(s, u))
        .child(action_panel(u))
        .child(guard_panel(u));

    let mut page = col()
        .key("release-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(18.0)
        .gap(14.0)
        .child(u.page_head(
            Screen::Release.kicker(),
            "Release",
            "The latest build, the deploy plan and the queue of projects that need a look after an update. Read only.",
            Some(trailing),
        ));

    if let Some(info) = &s.release.info {
        page = page.child(stats(u, info));
    }

    page.child(
        row()
            .key("release-body")
            .grow(1.0)
            .min_h(0.0)
            .gap(16.0)
            .child(left)
            .child(right),
    )
}

fn release_status(s: &State, u: &Ui) -> El {
    if s.project().is_none() {
        return u.status_chip("no project open", Tone::Quiet);
    }
    if s.release.lookup_error.is_some() {
        return u.status_chip("release read failed", Tone::Bad);
    }
    match &s.release.info {
        Some(info) => u.status_chip(&format!("release v{}", info.version), Tone::Ok),
        None => u.status_chip("no release", Tone::Quiet),
    }
}

fn stats(u: &Ui, info: &ReleaseInfo) -> El {
    row()
        .w_full()
        .gap(9.0)
        .child(u.stat(format!("v{}", info.version), "LATEST RELEASE", Tone::Accent))
        .child(u.stat(format!("{:02}", info.slot), "ADDON SLOT", Tone::Info))
        .child(u.stat(format!("{}", info.entries), "ENTRIES", Tone::Quiet))
        .child(u.stat(info.size_label(), "PACKED SIZE", Tone::Ok))
}

fn release_panel(s: &State, u: &Ui) -> El {
    let Some(info) = &s.release.info else {
        let detail = s.release.lookup_error.clone().unwrap_or_else(|| {
            "A release is a packed staging folder plus builds/<version>/release.json. Capture a baseline, then run `dah mod release` or ask the agent to validate and release."
                .to_string()
        });
        return u
            .panel()
            .key("release-latest")
            .w_full()
            .child(u.panel_header("LATEST RELEASE", "none"))
            .child(
                col()
                    .w_full()
                    .padding(14.0)
                    .gap(10.0)
                    .child(
                        row()
                            .w_full()
                            .gap(11.0)
                            .items_start()
                            .child(u.glyph_box("◈", Tone::Quiet, 30.0))
                            .child(
                                col()
                                    .grow(1.0)
                                    .gap(3.0)
                                    .child(text("No release for this project").font(BODY_MED).color(u.ink))
                                    .child(text(detail).font(SMALL).color(u.muted).wrap()),
                            ),
                    )
                    .child(
                        row()
                            .w_full()
                            .gap(7.0)
                            .child(
                                u.action("Validate and release this project", ActionKind::Primary)
                                    .on_press(|st: &mut State| {
                                        ask(st, PROMPT_VALIDATE);
                                    }),
                            )
                            .child(
                                u.action("Plan the deploy", ActionKind::Secondary)
                                    .on_press(|st: &mut State| {
                                        ask(st, PROMPT_PLAN);
                                    }),
                            ),
                    ),
            );
    };

    let tone = if info.vpk_present { Tone::Ok } else { Tone::Bad };
    u.panel()
        .key("release-latest")
        .w_full()
        .child(u.panel_header(
            "LATEST RELEASE",
            &format!("builds/{}", info.version),
        ))
        .child(
            col()
                .w_full()
                .padding(14.0)
                .gap(2.0)
                .child(
                    row()
                        .w_full()
                        .gap(9.0)
                        .center_y()
                        .child(text(format!("v{}", info.version)).font(H1).color(u.ink))
                        .child(u.chip(
                            &format!("slot {:02}", info.slot),
                            u.accent,
                            u.accent.alpha(0.13),
                        ))
                        .child(u.status_chip(
                            if info.vpk_present { "vpk present" } else { "vpk missing" },
                            tone,
                        ))
                        .child(spacer())
                        .child(text("READ FROM release.json").font(LABEL).color(u.faint)),
                )
                .child(u.divider().mt(7.0).mb(4.0))
                .child(u.kv("VPK file", &info.file_label(), if info.vpk_present { u.ink } else { u.rose }))
                .child(u.kv("Slot file", &info.slot_file(), u.ink))
                .child(u.kv("Chunks", &info.chunks_label(), u.muted))
                .child(u.kv("Entries", &format!("{}", info.entries), u.ink))
                .child(u.kv("Bytes", &info.size_label(), u.ink))
                .child(u.kv("Created", &short_stamp(&info.created_at), u.muted))
                .child(u.kv(
                    "Baseline",
                    &match &info.baseline_captured_at {
                        Some(stamp) => short_stamp(stamp),
                        None => "not recorded".to_string(),
                    },
                    if info.baseline_captured_at.is_some() {
                        u.muted
                    } else {
                        u.amber
                    },
                ))
                .child(u.kv("Folder", &info.dir, u.faint))
                .child(u.divider().mt(6.0).mb(4.0))
                .child(u.field_label("NOTES"))
                .child(
                    text(info.notes.clone().unwrap_or_else(|| "No release note recorded.".to_string()))
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                ),
        )
}

fn deploy_panel(s: &State, u: &Ui) -> El {
    u.panel()
        .key("release-deploy")
        .w_full()
        .child(u.panel_header("DEPLOY PLAN", "read only"))
        .child(match (&s.release.plan, &s.release.plan_error) {
            (Some(plan), _) => plan_body(s, u, plan),
            (None, Some(error)) => col().w_full().padding(14.0).gap(8.0).child(
                row()
                    .w_full()
                    .gap(9.0)
                    .items_start()
                    .child(u.glyph_box("!", Tone::Bad, 28.0))
                    .child(
                        col()
                            .grow(1.0)
                            .gap(3.0)
                            .child(text("The deploy plan could not be read").font(BODY_MED).color(u.ink))
                            .child(text(error.clone()).font(SMALL).color(u.muted).wrap()),
                    ),
            ),
            (None, None) => {
                let detail = if s.release.info.is_none() {
                    "A plan follows the latest release. Pack and release this project first."
                } else if !s.game_detected {
                    "No Deadlock install was found. A plan reads game/citadel/addons and gameinfo.gi, so it needs the install path."
                } else {
                    "No plan has been read yet. Rescan to compare the release with addons and gameinfo.gi."
                };
                col()
                    .w_full()
                    .padding(14.0)
                    .gap(9.0)
                    .child(
                        row()
                            .w_full()
                            .gap(9.0)
                            .items_start()
                            .child(u.glyph_box("◎", Tone::Quiet, 28.0))
                            .child(text(detail).font(SMALL).color(u.muted).grow(1.0).wrap()),
                    )
                    .child(
                        row()
                            .gap(7.0)
                            .child(
                                u.action("Rescan release", ActionKind::Secondary)
                                    .on_press(|st: &mut State| rescan(st)),
                            )
                            .child(if s.release.info.is_some() && !s.game_detected {
                                u.action("Open Settings", ActionKind::Ghost).on_press(
                                    |st: &mut State| st.screen = Screen::Settings,
                                )
                            } else {
                                el().w(0.0)
                            }),
                    )
            }
        })
}

fn plan_body(s: &State, u: &Ui, plan: &DeployPlanView) -> El {
    let mut body = col()
        .w_full()
        .padding(14.0)
        .gap(3.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.status_chip(
                    &format!("target slot {:02}", plan.slot),
                    Tone::Accent,
                ))
                .child(u.status_chip(
                    if plan.replace_required { "replace required" } else { "fresh slot" },
                    if plan.replace_required { Tone::Warn } else { Tone::Ok },
                ))
                .child(u.status_chip(
                    if plan.gameinfo_repair { "gameinfo repair" } else { "gameinfo ok" },
                    if plan.gameinfo_repair { Tone::Bad } else { Tone::Ok },
                ))
                .child(spacer())
                .child(
                    text(format!("{} entries", plan.entry_count))
                        .font(SMALL)
                        .color(u.faint),
                ),
        )
        .child(u.divider().mt(7.0).mb(4.0))
        .child(u.kv("Target", &plan.target, u.ink))
        .child(u.kv(
            "Replacement",
            if plan.replace_required {
                "an addon file already occupies this slot"
            } else {
                "the slot is free"
            },
            if plan.replace_required { u.amber } else { u.muted },
        ))
        .child(u.kv(
            "Gameinfo",
            if plan.gameinfo_repair {
                "SearchPaths needs `Game citadel/addons` above `Game citadel`"
            } else {
                "SearchPaths already lists citadel/addons"
            },
            if plan.gameinfo_repair { u.rose } else { u.muted },
        ));

    if !plan.chunk_targets.is_empty() {
        body = body.child(u.kv("Chunk targets", &plan.chunk_targets.join(", "), u.muted));
    }

    body = body
        .child(
            row()
                .w_full()
                .gap(7.0)
                .center_y()
                .pt(9.0)
                .child(text("LOWER SLOT CONFLICTS").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!("{}", plan.conflicts.len()))
                        .font(SMALL)
                        .color(u.faint),
                ),
        );

    if plan.conflicts.is_empty() {
        body = body.child(u.note(
            "No lower slot ships a path from this release. The full payload reaches the game.",
        ));
    } else {
        for (i, conflict) in plan.conflicts.iter().enumerate() {
            let mut paths = col().w_full().gap(2.0);
            for path in conflict.paths.iter().take(4) {
                paths = paths.child(text(path.clone()).font(MONO).color(u.muted).wrap_anywhere());
            }
            if conflict.paths.len() > 4 {
                paths = paths.child(
                    text(format!("and {} more path(s)", conflict.paths.len() - 4))
                        .font(SMALL)
                        .color(u.faint),
                );
            }
            body = body.child(
                col()
                    .key(format!("deploy-conflict-{i}"))
                    .w_full()
                    .padding(10.0)
                    .gap(4.0)
                    .radius(7.0)
                    .fill(u.well)
                    .border(1.0, u.amber.alpha(0.20))
                    .child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .center_y()
                            .child(u.chip(&format!("slot {}", conflict.slot_label()), u.amber, u.amber.alpha(0.12)))
                            .child(text(conflict.file.clone()).font(MONO_MED).color(u.ink).grow(1.0).truncate())
                            .child(text(conflict.paths_label()).font(SMALL).color(u.faint)),
                    )
                    .child(paths),
            );
        }
    }

    if !plan.warnings.is_empty() {
        body = body.child(
            row()
                .w_full()
                .gap(7.0)
                .center_y()
                .pt(9.0)
                .child(text("WARNINGS").font(LABEL).color(u.faint))
                .child(spacer())
                .child(
                    text(format!("{}", plan.warnings.len()))
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
        for warning in &plan.warnings {
            body = body.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_start()
                    .py(3.0)
                    .child(el().size(5.0, 5.0).radius(3.0).fill(u.amber.alpha(0.8)).mt(6.0))
                    .child(text(warning.clone()).font(SMALL).color(u.muted).grow(1.0).wrap()),
            );
        }
    }

    body = body.child(
        row()
            .w_full()
            .gap(7.0)
            .center_y()
            .pt(9.0)
            .child(text("RELEASE ENTRIES").font(LABEL).color(u.faint))
            .child(spacer())
            .child(text("from the release VPK").font(SMALL).color(u.faint)),
    );

    if plan.entry_preview.is_empty() {
        body = body.child(
            text("The release VPK is missing, so its entry list is unknown.")
                .font(SMALL)
                .color(u.amber)
                .wrap(),
        );
    } else {
        for path in &plan.entry_preview {
            body = body.child(text(path.clone()).font(MONO).color(u.muted).wrap_anywhere());
        }
        if plan.entry_count > plan.entry_preview.len() {
            body = body.child(
                text(format!(
                    "and {} more entry(s)",
                    plan.entry_count - plan.entry_preview.len()
                ))
                .font(SMALL)
                .color(u.faint),
            );
        }
    }

    body = body.child(u.note(if s.game_detected {
        "A plan is a proposal. The copy, the gameinfo repair and the launch are approval-gated app actions."
    } else {
        "The plan was read before the install disappeared. Rescan to confirm the target."
    }));

    body
}

fn publish_panel(s: &State, u: &Ui) -> El {
    let Some(info) = &s.release.info else {
        return u
            .panel()
            .key("release-publish")
            .w_full()
            .child(u.panel_header("PUBLISH FOLDER", "no release"))
            .child(
                col()
                    .w_full()
                    .padding(14.0)
                    .gap(5.0)
                    .child(
                        text("Nothing to publish yet.")
                            .font(BODY_MED)
                            .color(u.muted),
                    )
                    .child(
                        text("The folder is publish/<version>/ under the project, written by `dah publish prepare`.")
                            .font(SMALL)
                            .color(u.faint)
                            .wrap(),
                    ),
            );
    };

    let Some(publish) = &s.release.publish else {
        return u
            .panel()
            .key("release-publish")
            .w_full()
            .child(u.panel_header("PUBLISH FOLDER", "unknown"))
            .child(
                col()
                    .w_full()
                    .padding(14.0)
                    .child(text("The publish folder was not read.").font(SMALL).color(u.muted).wrap()),
            );
    };

    let tone = match (publish.dir_present, publish.metadata_present) {
        (true, true) => Tone::Ok,
        (true, false) => Tone::Warn,
        _ => Tone::Quiet,
    };
    let label = match (publish.dir_present, publish.metadata_present) {
        (true, true) => "metadata present",
        (true, false) => "folder without metadata",
        _ => "not prepared",
    };

    let mut body = col()
        .w_full()
        .padding(14.0)
        .gap(2.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(u.status_chip(label, tone))
                .child(spacer())
                .child(
                    text(format!("v{}", info.version))
                        .font(MONO)
                        .color(u.faint),
                ),
        )
        .child(u.divider().mt(7.0).mb(4.0))
        .child(u.kv(
            "metadata.json",
            if publish.metadata_present {
                publish.metadata_path.as_str()
            } else {
                "missing"
            },
            if publish.metadata_present { u.muted } else { u.amber },
        ))
        .child(u.kv("Folder", &publish.dir, u.faint));

    if let Some(created) = &publish.created_at {
        body = body.child(u.kv("Prepared", &short_stamp(created), u.muted));
    }

    if !publish.files.is_empty() {
        body = body.child(u.kv("Files", &publish.files.join(", "), u.muted));
    }

    body = body.child(u.note(match (publish.dir_present, publish.metadata_present) {
        (true, true) => {
            "Prepared for upload. Uploading to GameBanana is a user action, this screen never posts a file."
        }
        (true, false) => {
            "The folder exists without metadata.json. Run `dah publish prepare` again so the upload has a manifest."
        }
        _ => {
            "Run `dah publish prepare` to stage the release, metadata.json and the install note in publish/<version>."
        }
    }));

    u.panel()
        .key("release-publish")
        .w_full()
        .child(u.panel_header(
            "PUBLISH FOLDER",
            if publish.metadata_present { "prepared" } else { "not prepared" },
        ))
        .child(body)
}

fn review_panel(s: &State, u: &Ui) -> El {
    let queue = s.release.review.as_ref();
    let meta = match queue {
        Some(queue) => queue.summary(),
        None => "not scanned".to_string(),
    };

    let head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            u.action("Run review queue", ActionKind::Secondary)
                .on_press(|st: &mut State| scan_queue(st)),
        )
        .child(spacer());

    let mut body = col().w_full().padding(14.0).gap(9.0).child(head);

    if let Some(queue) = queue {
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text("WORKSPACE").font(LABEL).color(u.faint).w(80.0).mt(2.0))
                .child(
                    text(queue.workspace.clone())
                        .font(MONO)
                        .color(u.muted)
                        .grow(1.0)
                        .wrap_anywhere(),
                ),
        );
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text("CATALOG").font(LABEL).color(u.faint).w(80.0).mt(2.0))
                .child(
                    text(queue.catalog_label.clone())
                        .font(SMALL)
                        .color(u.muted)
                        .grow(1.0)
                        .wrap(),
                ),
        );

        if let Some(error) = &queue.error {
            body = body.child(
                row()
                    .w_full()
                    .gap(9.0)
                    .items_start()
                    .padding(10.0)
                    .radius(7.0)
                    .fill(u.rose.alpha(0.05))
                    .border(1.0, u.rose.alpha(0.22))
                    .child(u.glyph_box("!", Tone::Bad, 24.0))
                    .child(text(error.clone()).font(SMALL).color(u.muted).grow(1.0).wrap()),
            );
        }

        if queue.items.is_empty() {
            body = body.child(
                col()
                    .w_full()
                    .padding(12.0)
                    .gap(4.0)
                    .child(text("No mod projects in this workspace").font(BODY_MED).color(u.muted))
                    .child(
                        text("The queue reads <workspace>/mods/*/project.json and <workspace>/project.json. Create a project, then scan again.")
                            .font(SMALL)
                            .color(u.faint)
                            .wrap(),
                    ),
            );
        } else {
            for (i, item) in queue.items.iter().enumerate() {
                body = body.child(review_card(u, i, item));
            }
        }

        body = body.child(u.note(
            "The queue mirrors `dah watch`: one read-only pass, baseline diffs against the catalog, no project file is written.",
        ));
    } else {
        body = body
            .child(
                col()
                    .w_full()
                    .padding(12.0)
                    .gap(4.0)
                    .child(text("The queue has not run yet").font(BODY_MED).color(u.muted))
                    .child(
                        text("It loads the game catalog, so it runs on an explicit scan rather than every frame. Local scenes run it once when they load.")
                            .font(SMALL)
                            .color(u.faint)
                            .wrap(),
                    ),
            )
            .child(
                row()
                    .gap(7.0)
                    .child(
                        u.action("Run review queue", ActionKind::Primary).on_press(
                            |st: &mut State| scan_queue(st),
                        ),
                    )
                    .child(
                        u.action("Open Settings", ActionKind::Ghost)
                            .on_press(|st: &mut State| st.screen = Screen::Settings),
                    ),
            );
    }

    u.panel()
        .key("release-review")
        .w_full()
        .child(u.panel_header("REVIEW QUEUE", &meta))
        .child(body)
}

fn review_card(u: &Ui, i: usize, item: &ReviewItem) -> El {
    let tone = review_tone(item);
    let mut card = col()
        .key(format!("review-{i}"))
        .w_full()
        .padding(11.0)
        .gap(5.0)
        .radius(8.0)
        .fill(u.well)
        .border(1.0, tone.fg(u).alpha(0.18))
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
                .child(u.chip(&format!("slot {}", item.slot_label()), u.accent, u.accent.alpha(0.11)))
                .child(u.status_chip(
                    if item.needs_review() { "needs review" } else { "clean" },
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

    let mut counts = row().w_full().gap(6.0).center_y().items_center();
    counts = counts.child(text("TARGETS").font(LABEL).color(u.faint));
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
            &format!("unbaselined {}", item.unbaselined),
            Tone::Quiet,
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
        counts = counts.child(u.status_chip("all targets match the baseline", Tone::Ok));
    }
    card = card.child(counts);

    for migration in &item.migrations {
        card = card.child(
            col()
                .w_full()
                .gap(2.0)
                .padding(8.0)
                .radius(6.0)
                .fill(u.violet.alpha(0.06))
                .child(
                    row()
                        .gap(7.0)
                        .center_y()
                        .child(u.chip("move", u.violet, u.violet.alpha(0.12)))
                        .child(text("the manifest still points at the old path").font(SMALL).color(u.muted)),
                )
                .child(
                    text(migration.from.clone())
                        .font(MONO)
                        .color(u.faint)
                        .wrap_anywhere(),
                )
                .child(
                    text(format!("→ {}", migration.to))
                        .font(MONO)
                        .color(u.teal)
                        .wrap_anywhere(),
                ),
        );
    }

    for warning in &item.warnings {
        card = card.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(el().size(5.0, 5.0).radius(3.0).fill(u.amber.alpha(0.8)).mt(6.0))
                .child(text(warning.clone()).font(SMALL).color(u.muted).grow(1.0).wrap()),
        );
    }

    if let Some(error) = &item.error {
        card = card.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(u.chip("check failed", u.rose, u.rose.alpha(0.12)))
                .child(text(error.clone()).font(SMALL).color(u.muted).grow(1.0).wrap()),
        );
    }

    card
}

fn action_panel(u: &Ui) -> El {
    u.panel()
        .key("release-actions")
        .w_full()
        .child(u.panel_header("AGENT ACTIONS", "sends a prompt"))
        .child(
            col()
                .w_full()
                .padding(14.0)
                .gap(7.0)
                .child(
                    u.action("Validate and release this project", ActionKind::Primary)
                        .w_full()
                        .on_press(|st: &mut State| ask(st, PROMPT_VALIDATE)),
                )
                .child(
                    u.action("Plan the deploy", ActionKind::Secondary)
                        .w_full()
                        .on_press(|st: &mut State| ask(st, PROMPT_PLAN)),
                )
                .child(
                    u.action("Migrate moved paths", ActionKind::Secondary)
                        .w_full()
                        .on_press(|st: &mut State| ask(st, PROMPT_MIGRATE)),
                )
                .child(u.note(
                    "Each action opens a workbench prompt with the open project in context. It does not run a game write.",
                )),
        )
}

fn guard_panel(u: &Ui) -> El {
    let rows = [
        ("Install copy", "app action, approval required"),
        ("gameinfo.gi repair", "app action, approval required"),
        ("Upload", "your action on GameBanana"),
    ];
    let mut body = col().w_full().padding(14.0).gap(8.0);
    body = body.child(
        text("Install and upload need the app and you. The agent prepares a plan and a validated artifact, and this screen only reads.")
            .font(SMALL)
            .color(u.muted)
            .wrap(),
    );
    for (name, detail) in rows {
        body = body.child(
            row()
                .w_full()
                .gap(9.0)
                .items_start()
                .child(el().size(5.0, 5.0).radius(3.0).fill(u.faint).mt(6.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(1.0)
                        .child(text(name).font(SMALL).color(u.ink))
                        .child(text(detail).font(SMALL).color(u.faint)),
                ),
        );
    }
    u.panel()
        .key("release-guard")
        .w_full()
        .child(u.panel_header("WRITE BOUNDARY", "no writes"))
        .child(body)
}

fn ask(st: &mut State, prompt: &str) {
    let label = match st.project() {
        Some(project) => project.name.clone(),
        None => "the open project".to_string(),
    };
    st.ask_agent(format!("{prompt}\n\nProject: {label}."));
}

/// Re-read the release and the plan. A demo scene keeps its fixture so the
/// snapshot stays hermetic; a local scene reads the workspace.
fn rescan(st: &mut State) {
    if st.demo {
        st.status =
            "Demo fixture: the release read stays on the fixture. Nothing was written.".to_string();
        return;
    }
    data::refresh_release(st);
    st.status =
        "Release and deploy reads refreshed from the workspace and the game install. Nothing was written."
            .to_string();
}

/// Load the review queue. This is the only read on this screen that can touch
/// the game catalog, so it stays behind a press or a local scene load.
fn scan_queue(st: &mut State) {
    if st.demo {
        st.status =
            "Demo fixture: the queue stays on the fixture. Nothing was read or written.".to_string();
        return;
    }
    data::load_review_queue(st);
    let summary = st
        .release
        .review
        .as_ref()
        .map(ReviewQueue::summary)
        .unwrap_or_else(|| "nothing was scanned".to_string());
    st.status = format!("Review queue: {summary}. Nothing was written.");
}

fn review_tone(item: &ReviewItem) -> Tone {
    if item.error.is_some() || item.missing > 0 {
        Tone::Bad
    } else if item.changed > 0 || item.unbaselined > 0 || !item.migrations.is_empty() || !item.warnings.is_empty() {
        Tone::Warn
    } else if item.moved > 0 {
        Tone::Info
    } else {
        Tone::Ok
    }
}

fn short_stamp(value: &str) -> String {
    value.chars().take(16).collect::<String>().replace('T', " ")
}
