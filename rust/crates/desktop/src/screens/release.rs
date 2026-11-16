//! Release: the latest build, the deploy plan, the publish handoff and the
//! review queue. Every value on this screen comes from a read. Install,
//! gameinfo repair and account handling stay user and app actions; uploading is
//! a browser handoff that the user approves, never an API upload.

use std::path::Path;

use rune::{
    Color, El, Key, KeyEvent, ScrollSpec, Spring, TextEdit, Transition, Vec2, col, el, row, spacer,
    text,
};

use crate::app::State;
use crate::data;
use crate::model::{
    DeployPlanView, ReleaseDialog, ReleaseInfo, ReviewItem, Screen, UploadInfo,
};
use crate::publish::PublishAction;
use crate::ui::{
    ActionKind, BODY_MED, H1, HERO, LABEL, LEAD, MONO, MONO_MED, RAIL_W, SMALL, Tone, Ui,
};

const PROMPT_VALIDATE: &str = "Validate and release this project. Read project.json, check every target against the game index, validate the staging payload and prepare builds/<version> with its release manifest. List every warning before packing. Do not write to the game install, edit gameinfo.gi or upload anything.";

const PROMPT_PLAN: &str = "Plan the deploy for this project. Read the latest release manifest and the installed addons, then report the target slot, whether a replacement is required, lower-slot conflicts and the gameinfo SearchPaths state. This plan is read only: do not copy files or edit gameinfo.gi.";

const PROMPT_MIGRATE: &str = "Migrate the moved paths for this project. Use the current game index to list the exact old and new path for each target that moved, explain any ambiguous basename instead of guessing, and update project.json only after I approve the plan. Do not touch the game install.";

const SUBMIT_PAGE: &str = "https://gamebanana.com/mods/submit";

pub fn view(s: &State, u: &Ui) -> El {
    let trailing = row().gap(9.0).center_y().child(release_status(s, u)).child(
        u.pill("Rescan", "↻", ActionKind::Ghost)
            .key("release-rescan")
            .on_press(|st: &mut State| rescan(st)),
    );

    let mut page = col()
        .key("release-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(26.0)
        .pt(20.0)
        .pb(18.0)
        .gap(16.0)
        .child(u.glow(
            "release-glow-warm",
            [0.16, 0.02],
            (760.0, 520.0),
            u.glow_warm,
        ))
        .child(u.glow(
            "release-glow-cool",
            [0.92, 0.06],
            (620.0, 460.0),
            u.glow_cool,
        ))
        .child(head(u, Some(trailing)));

    if let Some(info) = &s.release.info {
        page = page.child(hero(s, u, info));
    } else {
        page = page.child(no_release(s, u));
    }

    let left = col()
        .key("release-left")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(14.0)
        .child(deploy_panel(s, u))
        .child(review_panel(s, u));

    let right = col()
        .key("release-right")
        .w(RAIL_W)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .gap(14.0)
        .child(upload_panel(s, u))
        .child(action_panel(u))
        .child(guard_panel(u));

    page.child(
        row()
            .key("release-body")
            .grow(1.0)
            .min_h(0.0)
            .gap(18.0)
            .child(left)
            .child(right),
    )
}

fn head(u: &Ui, trailing: Option<El>) -> El {
    let mut left = col().grow(1.0).gap(7.0).max_w(760.0);
    left = left
        .child(u.eyebrow(Screen::Release.kicker()))
        .child(text("Release").font(HERO).color(u.ink))
        .child(
            text("One latest build, one deliberate handoff. Read the deploy plan, then publish when the archive is right.")
                .font(LEAD)
                .color(u.muted)
                .wrap(),
        );
    let mut r = row().w_full().gap(20.0).items_end().child(left);
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

fn release_status(s: &State, u: &Ui) -> El {
    if s.project().is_none() {
        return u.status_chip("no project open", Tone::Quiet);
    }
    if s.release.lookup_error.is_some() {
        return u.status_chip("release read failed", Tone::Bad);
    }
    if s.publish_job.is_running() {
        return u.status_chip("publish running", Tone::Warn);
    }
    match &s.release.info {
        Some(info) => u.status_chip(&format!("release v{}", info.version), Tone::Ok),
        None => u.status_chip("no release", Tone::Quiet),
    }
}

fn upload_tone(upload: Option<&UploadInfo>) -> Tone {
    upload
        .map(|upload| upload.state_tone().tone())
        .unwrap_or(Tone::Quiet)
}

fn upload_label(upload: Option<&UploadInfo>) -> String {
    upload
        .map(UploadInfo::state_label)
        .unwrap_or_else(|| "none".to_string())
}

fn hero(s: &State, u: &Ui, info: &ReleaseInfo) -> El {
    let upload = s.release.upload.as_ref();
    let tone = upload_tone(upload);
    let state = upload_label(upload);

    let mut metrics = row().gap(8.0).center_y();
    metrics = metrics
        .child(u.chip(
            &format!("slot {:02}", info.slot),
            u.accent,
            u.accent.alpha(0.12),
        ))
        .child(u.chip(
            &format!("{} entries", info.entries),
            u.violet,
            u.violet.alpha(0.10),
        ))
        .child(u.chip(&info.size_label(), u.teal, u.teal.alpha(0.10)));
    if !info.vpk_present {
        metrics = metrics.child(u.status_chip("vpk missing", Tone::Bad));
    }

    let publish = u
        .pill("Publish to GameBanana", "↗", ActionKind::Primary)
        .key("publish-handoff")
        .on_press(|st: &mut State| open_approval(st));

    let notes = info
        .notes
        .clone()
        .unwrap_or_else(|| "No release note recorded.".to_string());

    let mut body = col()
        .w_full()
        .padding(24.0)
        .gap(18.0)
        .child(
            row()
                .w_full()
                .gap(22.0)
                .items_end()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(9.0)
                        .child(u.eyebrow("LATEST RELEASE"))
                        .child(
                            row()
                                .gap(12.0)
                                .center_y()
                                .child(text(format!("v{}", info.version)).font(HERO).color(u.ink))
                                .child(metrics),
                        )
                        .child(text(notes).font(LEAD).color(u.muted).max_w(620.0).wrap()),
                )
                .child(
                    col()
                        .gap(9.0)
                        .items_end()
                        .child(publish)
                        .child(u.status_chip(&format!("publish {state}"), tone)),
                ),
        )
        .child(u.divider())
        .child(
            row()
                .w_full()
                .gap(18.0)
                .items_start()
                .child(fact_col(
                    u,
                    "VPK",
                    &info.file_label(),
                    if info.vpk_present { u.ink } else { u.rose },
                ))
                .child(fact_col(u, "CHUNKS", &info.chunks_label(), u.muted))
                .child(fact_col(
                    u,
                    "CREATED",
                    &short_stamp(&info.created_at),
                    u.muted,
                ))
                .child(fact_col(
                    u,
                    "BASELINE",
                    &match &info.baseline_captured_at {
                        Some(stamp) => short_stamp(stamp),
                        None => "not recorded".to_string(),
                    },
                    if info.baseline_captured_at.is_some() {
                        u.muted
                    } else {
                        u.amber
                    },
                )),
        )
        .child(
            text(info.dir.clone())
                .font(MONO)
                .color(u.faint)
                .wrap_anywhere(),
        );

    if !info.vpk_present {
        body = body.child(
            u.note("The release manifest lists a VPK that is not in its folder. Re-run the release before publishing."),
        );
    }

    u.glass_shell()
        .key("release-lead")
        .w_full()
        .padding(1.0)
        .child(u.glass_core().w_full().child(body))
}

fn fact_col(u: &Ui, label: &str, value: &str, tone: Color) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(text(label).font(LABEL).color(u.faint))
        .child(
            text(value)
                .font(SMALL)
                .color(tone)
                .wrap_anywhere()
                .selectable(true),
        )
}

fn no_release(s: &State, u: &Ui) -> El {
    let detail = s.release.lookup_error.clone().unwrap_or_else(|| {
        "A release is a packed staging folder plus builds/<version>/release.json. Capture a baseline, then run `dah mod release` or ask the agent to validate and release."
            .to_string()
    });
    u.glass_shell()
        .key("release-lead")
        .w_full()
        .padding(1.0)
        .child(
            u.glass_core()
                .w_full()
                .padding(24.0)
                .gap(14.0)
                .child(
                    row()
                        .w_full()
                        .gap(14.0)
                        .items_start()
                        .child(u.glyph_box("◈", Tone::Quiet, 38.0))
                        .child(
                            col()
                                .grow(1.0)
                                .gap(6.0)
                                .child(text("No release for this project").font(H1).color(u.ink))
                                .child(text(detail).font(LEAD).color(u.muted).wrap()),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .gap(9.0)
                        .child(
                            u.pill("Validate and release", "↗", ActionKind::Primary)
                                .key("release-validate")
                                .on_press(|st: &mut State| ask(st, PROMPT_VALIDATE)),
                        )
                        .child(
                            u.pill("Plan the deploy", "◎", ActionKind::Secondary)
                                .on_press(|st: &mut State| ask(st, PROMPT_PLAN)),
                        ),
                ),
        )
}

fn deploy_panel(s: &State, u: &Ui) -> El {
    u.glass_shell()
        .key("release-deploy")
        .w_full()
        .padding(1.0)
        .child(
            u.glass_core()
                .w_full()
                .child(section_head(u, "DEPLOY PLAN", "read only"))
                .child(match (&s.release.plan, &s.release.plan_error) {
                    (Some(plan), _) => plan_body(s, u, plan),
                    (None, Some(error)) => col().w_full().padding(16.0).gap(10.0).child(
                        row()
                            .w_full()
                            .gap(10.0)
                            .items_start()
                            .child(u.glyph_box("!", Tone::Bad, 30.0))
                            .child(
                                col()
                                    .grow(1.0)
                                    .gap(4.0)
                                    .child(
                                        text("The deploy plan could not be read")
                                            .font(BODY_MED)
                                            .color(u.ink),
                                    )
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
                            .padding(16.0)
                            .gap(10.0)
                            .child(
                                row()
                                    .w_full()
                                    .gap(11.0)
                                    .items_start()
                                    .child(u.glyph_box("◎", Tone::Quiet, 30.0))
                                    .child(
                                        text(detail)
                                            .font(SMALL)
                                            .color(u.muted)
                                            .grow(1.0)
                                            .wrap(),
                                    ),
                            )
                            .child(
                                row()
                                    .gap(8.0)
                                    .child(
                                        u.pill("Rescan release", "↻", ActionKind::Secondary)
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
                }),
        )
}

fn section_head(u: &Ui, title: &str, meta: &str) -> El {
    row()
        .w_full()
        .px(18.0)
        .pt(15.0)
        .pb(12.0)
        .gap(10.0)
        .items_center()
        .child(text(title).font(LABEL).color(u.faint))
        .child(spacer())
        .child(text(meta).font(SMALL).color(u.faint))
        .child(u.divider().absolute(0.0, 40.0))
}

fn plan_body(s: &State, u: &Ui, plan: &DeployPlanView) -> El {
    let mut body = col()
        .w_full()
        .padding(18.0)
        .gap(3.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(u.status_chip(&format!("target slot {:02}", plan.slot), Tone::Accent))
                .child(u.status_chip(
                    if plan.replace_required {
                        "replace required"
                    } else {
                        "fresh slot"
                    },
                    if plan.replace_required {
                        Tone::Warn
                    } else {
                        Tone::Ok
                    },
                ))
                .child(u.status_chip(
                    if plan.gameinfo_repair {
                        "gameinfo repair"
                    } else {
                        "gameinfo ok"
                    },
                    if plan.gameinfo_repair {
                        Tone::Bad
                    } else {
                        Tone::Ok
                    },
                ))
                .child(spacer())
                .child(
                    text(format!("{} entries", plan.entry_count))
                        .font(SMALL)
                        .color(u.faint),
                ),
        )
        .child(u.divider().mt(8.0).mb(6.0))
        .child(u.kv("Target", &plan.target, u.ink))
        .child(u.kv(
            "Replacement",
            if plan.replace_required {
                "an addon file already occupies this slot"
            } else {
                "the slot is free"
            },
            if plan.replace_required {
                u.amber
            } else {
                u.muted
            },
        ))
        .child(u.kv(
            "Gameinfo",
            if plan.gameinfo_repair {
                "SearchPaths needs `Game citadel/addons` above `Game citadel`"
            } else {
                "SearchPaths already lists citadel/addons"
            },
            if plan.gameinfo_repair {
                u.rose
            } else {
                u.muted
            },
        ));

    if !plan.chunk_targets.is_empty() {
        body = body.child(u.kv("Chunk targets", &plan.chunk_targets.join(", "), u.muted));
    }

    body = body.child(
        row()
            .w_full()
            .gap(7.0)
            .center_y()
            .pt(12.0)
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
                    .padding(12.0)
                    .gap(5.0)
                    .radius(12.0)
                    .fill(u.amber.alpha(0.055))
                    .border(1.0, u.amber.alpha(0.18))
                    .child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .center_y()
                            .child(u.chip(
                                &format!("slot {}", conflict.slot_label()),
                                u.amber,
                                u.amber.alpha(0.12),
                            ))
                            .child(
                                text(conflict.file.clone())
                                    .font(MONO_MED)
                                    .color(u.ink)
                                    .grow(1.0)
                                    .truncate(),
                            )
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
                .pt(12.0)
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
                    .gap(9.0)
                    .items_start()
                    .py(4.0)
                    .child(
                        el().size(5.0, 5.0)
                            .radius(3.0)
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
    }

    body = body.child(
        row()
            .w_full()
            .gap(7.0)
            .center_y()
            .pt(12.0)
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

/// The publish status capsule: one state badge, the archive hash and the
/// durable record, with no dense table.
fn upload_panel(s: &State, u: &Ui) -> El {
    let upload = s.release.upload.as_ref();
    let tone = upload_tone(upload);
    let state = upload_label(upload);

    let mut body = col().w_full().padding(16.0).gap(11.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(u.dot(tone))
            .child(text("PUBLISH STATUS").font(LABEL).color(u.faint))
            .child(spacer())
            .child(u.status_chip(&state, tone)),
    );

    if let Some(error) = &s.release.upload_error {
        body = body.child(
            col()
                .w_full()
                .padding(12.0)
                .gap(4.0)
                .radius(12.0)
                .fill(u.rose.alpha(0.06))
                .border(1.0, u.rose.alpha(0.20))
                .child(
                    text("upload.json could not be read")
                        .font(BODY_MED)
                        .color(u.ink),
                )
                .child(text(error.clone()).font(SMALL).color(u.muted).wrap()),
        );
    }

    match upload {
        Some(upload) => {
            body = body
                .child(
                    row()
                        .w_full()
                        .gap(11.0)
                        .items_start()
                        .child(u.glyph_box("▣", Tone::Accent, 32.0))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(3.0)
                                .child(
                                    text(upload.archive_file.clone())
                                        .font(MONO_MED)
                                        .color(u.ink)
                                        .wrap_anywhere(),
                                )
                                .child(
                                    text(upload.archive_size_label()).font(SMALL).color(u.faint),
                                ),
                        ),
                )
                .child(u.field_label("SHA-256"))
                .child(
                    u.well_field().w_full().padding(10.0).child(
                        text(upload.archive_sha256.clone())
                            .font(MONO)
                            .color(u.prose)
                            .wrap_anywhere()
                            .selectable(true),
                    ),
                );

            if let Some(url) = &upload.submission_url {
                body = body.child(u.field_label("SUBMISSION URL")).child(
                    text(url.clone())
                        .font(MONO)
                        .color(u.teal)
                        .wrap_anywhere()
                        .selectable(true),
                );
            } else if upload.pending() {
                body = body.child(text(
                    "The handoff is recorded. Upload the archive, then confirm the submission URL here.",
                )
                .font(SMALL)
                .color(u.faint)
                .wrap());
            }
        }
        None => {
            let archive = s
                .release
                .publish
                .as_ref()
                .and_then(|publish| publish.archive_file.clone());
            body = body
                .child(text("No handoff has been approved yet.").font(BODY_MED).color(u.muted))
                .child(
                    text(match archive {
                        Some(file) => format!(
                            "{file} is prepared. Publish opens the approval dialog, records the handoff and hands the page to your browser."
                        ),
                        None => "Publish prepares the browser handoff. Run `dah publish prepare` first if the archive is missing.".to_string(),
                    })
                    .font(SMALL)
                    .color(u.faint)
                    .wrap(),
                );
        }
    }

    let confirm = u
        .action("Confirm submission URL", ActionKind::Secondary)
        .w_full()
        .on_press(|st: &mut State| open_confirm(st));
    let refresh = u
        .action("Refresh upload status", ActionKind::Ghost)
        .w_full()
        .on_press(|st: &mut State| refresh_upload(st));

    body = body.child(u.divider()).child(confirm).child(refresh).child(
        text(if s.publish_job.is_running() {
            "A publish command is running in the background. Its output lands in the status line."
        } else {
            "Confirm re-checks the archive hash and verifies the page names this project."
        })
        .font(SMALL)
        .color(u.faint)
        .wrap(),
    );

    if let Some(upload) = upload {
        body = body
            .child(u.divider())
            .child(
                row()
                    .w_full()
                    .gap(14.0)
                    .items_start()
                    .child(fact_col(
                        u,
                        "CREATED",
                        &upload
                            .created_at
                            .as_deref()
                            .map(short_stamp)
                            .unwrap_or_else(|| "not recorded".to_string()),
                        u.muted,
                    ))
                    .child(fact_col(
                        u,
                        "CONFIRMED",
                        &upload
                            .confirmed_at
                            .as_deref()
                            .map(short_stamp)
                            .unwrap_or_else(|| "not confirmed".to_string()),
                        if upload.verified { u.teal } else { u.amber },
                    )),
            )
            .child(
                text(upload.record_path.clone())
                    .font(MONO)
                    .color(u.faint)
                    .wrap_anywhere(),
            );
    }

    u.glass_shell()
        .key("release-upload")
        .w_full()
        .padding(1.0)
        .child(u.glass_core().w_full().child(body))
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
            u.pill("Run review queue", "↻", ActionKind::Secondary)
                .on_press(|st: &mut State| scan_queue(st)),
        )
        .child(spacer());

    let mut body = col().w_full().padding(18.0).gap(11.0).child(head);

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
                    .gap(10.0)
                    .items_start()
                    .padding(12.0)
                    .radius(12.0)
                    .fill(u.rose.alpha(0.05))
                    .border(1.0, u.rose.alpha(0.20))
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
            body = body.child(
                col()
                    .w_full()
                    .padding(14.0)
                    .gap(5.0)
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
                    .padding(14.0)
                    .gap(5.0)
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
                    .gap(8.0)
                    .child(
                        u.pill("Run review queue", "↻", ActionKind::Primary)
                            .on_press(|st: &mut State| scan_queue(st)),
                    )
                    .child(
                        u.action("Open Settings", ActionKind::Ghost)
                            .on_press(|st: &mut State| st.screen = Screen::Settings),
                    ),
            );
    }

    u.glass_shell()
        .key("release-review")
        .w_full()
        .padding(1.0)
        .child(
            u.glass_core()
                .w_full()
                .child(section_head(u, "REVIEW QUEUE", &meta))
                .child(body),
        )
}

fn review_card(u: &Ui, i: usize, item: &ReviewItem) -> El {
    let tone = review_tone(item);
    let mut card = col()
        .key(format!("review-{i}"))
        .w_full()
        .padding(13.0)
        .gap(6.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, tone.fg(u).alpha(0.14))
        .child(
            row()
                .w_full()
                .gap(9.0)
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
                    u.accent.alpha(0.11),
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
        counts =
            counts.child(u.status_chip(&format!("unbaselined {}", item.unbaselined), Tone::Quiet));
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
                .gap(3.0)
                .padding(10.0)
                .radius(10.0)
                .fill(u.violet.alpha(0.055))
                .child(
                    row()
                        .gap(7.0)
                        .center_y()
                        .child(u.chip("move", u.violet, u.violet.alpha(0.12)))
                        .child(
                            text("the manifest still points at the old path")
                                .font(SMALL)
                                .color(u.muted),
                        ),
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
                .gap(9.0)
                .items_start()
                .child(
                    el().size(5.0, 5.0)
                        .radius(3.0)
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
                .gap(9.0)
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

fn action_panel(u: &Ui) -> El {
    u.glass_shell()
        .key("release-actions")
        .w_full()
        .padding(1.0)
        .child(
            u.glass_core()
                .w_full()
                .padding(18.0)
                .gap(9.0)
                .child(text("AGENT ACTIONS").font(LABEL).color(u.faint))
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
                    "Each action opens an agent prompt with the open project in context. It does not run a game write.",
                )),
        )
}

fn guard_panel(u: &Ui) -> El {
    let rows = [
        ("Install copy", "app action, approval required"),
        ("gameinfo.gi repair", "app action, approval required"),
        ("GameBanana account", "your browser and your account"),
        ("Archive upload", "your handoff, not an API upload"),
    ];
    let mut body = col()
        .w_full()
        .padding(18.0)
        .gap(8.0)
        .child(text("WRITE BOUNDARY").font(LABEL).color(u.faint));
    for (name, detail) in rows {
        body = body.child(
            row()
                .w_full()
                .gap(10.0)
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
    body = body.child(u.note(
        "Publishing records a browser handoff in upload.json and opens the GameBanana page. The app never receives your password or uploads the archive itself.",
    ));
    u.glass_shell()
        .key("release-guard")
        .w_full()
        .padding(1.0)
        .child(u.glass_core().w_full().child(body))
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
        st.status = "Demo fixture: the queue stays on the fixture. Nothing was read or written."
            .to_string();
        return;
    }
    crate::data::start_review_scan(st);
}

fn review_tone(item: &ReviewItem) -> Tone {
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

fn short_stamp(value: &str) -> String {
    value.chars().take(16).collect::<String>().replace('T', " ")
}

fn open_approval(st: &mut State) {
    st.release_dialog = ReleaseDialog::ApproveUpload;
}

fn open_confirm(st: &mut State) {
    let existing = st
        .release
        .upload
        .as_ref()
        .and_then(|upload| upload.submission_url.clone())
        .unwrap_or_default();
    st.confirm_url = TextEdit::new(existing);
    st.release_dialog = ReleaseDialog::ConfirmUrl;
}

fn start_upload(st: &mut State) {
    st.release_dialog = ReleaseDialog::None;
    if st.demo {
        st.status =
            "Demo fixture: no publish command was run and no browser page was opened.".to_string();
        return;
    }
    let workspace = st.paths.workspace.clone();
    let wake = st.wake.clone();
    match st
        .publish_job
        .start(PublishAction::Upload, Path::new(&workspace), None, wake)
    {
        Ok(()) => {
            st.status =
                "Approved the handoff. Running `dah publish upload --approve --open` in the background."
                    .to_string()
        }
        Err(error) => st.status = error,
    }
}

fn start_confirm(st: &mut State) {
    let url = st.confirm_url.text.trim().to_string();
    if url.is_empty() {
        st.status = "Enter the GameBanana submission URL before confirming.".to_string();
        return;
    }
    st.release_dialog = ReleaseDialog::None;
    if st.demo {
        st.status = "Demo fixture: no confirm command was run and no page was fetched.".to_string();
        return;
    }
    let workspace = st.paths.workspace.clone();
    let wake = st.wake.clone();
    match st.publish_job.start(
        PublishAction::Confirm,
        Path::new(&workspace),
        Some(&url),
        wake,
    ) {
        Ok(()) => {
            st.status = if crate::publish::is_local_http(&url) {
                "Recording the local test submission. Running `dah publish confirm --allow-http` in the background."
                    .to_string()
            } else {
                "Recording the submission. Running `dah publish confirm` in the background."
                    .to_string()
            }
        }
        Err(error) => st.status = error,
    }
}

fn refresh_upload(st: &mut State) {
    if st.demo {
        st.status =
            "Demo fixture: the upload state stays on the fixture. No file was read.".to_string();
        return;
    }
    data::refresh_upload(st);
    st.status = match &st.release.upload {
        Some(upload) => format!(
            "Upload status refreshed from upload.json: {}{}.",
            upload.state_label(),
            upload
                .submission_url
                .as_ref()
                .map(|url| format!(" · {url}"))
                .unwrap_or_default()
        ),
        None => "No upload.json in the publish folder, so the state is none.".to_string(),
    };
}

// The modal layer is mounted at the window root so the scrim dims the whole
// canvas and the dialog never clips inside a scroll region.

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let which = s.release_dialog;
    if which == ReleaseDialog::None {
        return None;
    }
    let content = match which {
        ReleaseDialog::ApproveUpload => approval_dialog(s, u),
        ReleaseDialog::ConfirmUrl => confirm_dialog(s, u),
        ReleaseDialog::None => return None,
    };
    let scrim = el()
        .key("release-modal-scrim")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.68))
        .radial_gradient(
            [0.5, 0.28],
            [760.0, 520.0],
            [(0.0, u.accent.alpha(0.055)), (1.0, Color::TRANSPARENT)],
        )
        .transition(Transition::opacity())
        .a11y_hidden();
    Some(
        el().key("release-modal-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(scrim)
            .child(
                el().key("release-modal-center")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .padding(34.0)
                    .center()
                    .child(content),
            ),
    )
}

fn modal_frame(u: &Ui, key: &str, title: &str, inner: El) -> El {
    u.dialog_shell()
        .key(key)
        .role("dialog")
        .label(title)
        .w(600.0)
        .max_w(600.0)
        .padding(1.0)
        .transition(Transition::scale_anchor(0.97, Vec2::new(0.5, 0.5)))
        .token_motion(Spring::snappy(0.34, 0.12))
        .modal(true, |st: &mut State| {
            st.release_dialog = ReleaseDialog::None
        })
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.release_dialog = ReleaseDialog::None;
                true
            } else {
                false
            }
        })
        .child(
            u.dialog_core()
                .w_full()
                .padding(28.0)
                .gap(20.0)
                .child(inner),
        )
}

fn approval_dialog(s: &State, u: &Ui) -> El {
    let upload = s.release.upload.as_ref();
    let publish = s.release.publish.as_ref();
    let info = s.release.info.as_ref();
    let project = s
        .project()
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "the open project".to_string());
    let version = info
        .map(|info| format!("v{}", info.version))
        .unwrap_or_else(|| "no release".to_string());
    let archive = upload
        .map(|upload| upload.archive_file.clone())
        .or_else(|| publish.and_then(|publish| publish.archive_file.clone()))
        .unwrap_or_else(|| "archive not prepared".to_string());
    let bytes = upload
        .map(|upload| upload.archive_bytes)
        .or_else(|| publish.and_then(|publish| publish.archive_bytes))
        .unwrap_or(0);
    let sha = upload
        .map(|upload| upload.archive_sha256.clone())
        .or_else(|| publish.and_then(|publish| publish.archive_sha256.clone()))
        .unwrap_or_else(|| "not recorded".to_string());
    let submit = upload
        .map(UploadInfo::submit_label)
        .unwrap_or_else(|| SUBMIT_PAGE.to_string());

    let body = col()
        .w_full()
        .gap(20.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(7.0)
                        .child(u.eyebrow(&format!("{project} · {version}")))
                        .child(text("Publish to GameBanana").font(H1).color(u.ink))
                        .child(
                            text("One deliberate step. The app records the handoff and opens the submission page. You choose the archive and sign in yourself.")
                                .font(LEAD)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("◈", Tone::Accent, 42.0)),
        )
        .child(
            col()
                .w_full()
                .gap(2.0)
                .child(u.kv("Project", &project, u.ink))
                .child(u.kv("Version", &version, u.ink))
                .child(u.kv("Archive", &archive, u.ink))
                .child(u.kv(
                    "Size",
                    &if bytes > 0 {
                        crate::model::format_size(bytes)
                    } else {
                        "not recorded".to_string()
                    },
                    u.muted,
                )),
        )
        .child(
            col().w_full().gap(6.0).child(u.field_label("SHA-256")).child(
                u.well_field().w_full().padding(11.0).child(
                    text(sha)
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ),
            ),
        )
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(u.field_label("SUBMIT URL"))
                .child(
                    text(submit)
                        .font(MONO)
                        .color(u.teal)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(11.0)
                .items_start()
                .padding(13.0)
                .radius(14.0)
                .fill(u.amber.alpha(0.055))
                .border(1.0, u.amber.alpha(0.17))
                .child(u.glyph_box("!", Tone::Warn, 28.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(3.0)
                        .child(
                            text("This records the handoff and opens your browser")
                                .font(BODY_MED)
                                .color(u.ink),
                        )
                        .child(
                            text("upload.json gains the archive hash, submit URL and timestamp. Nothing is uploaded by the app, and no GameBanana password or session is read.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    u.pill("Approve handoff", "↗", ActionKind::Primary)
                        .key("approve-handoff")
                        .on_press(|st: &mut State| start_upload(st)),
                )
                .child(
                    u.pill("Cancel", "esc", ActionKind::Ghost)
                        .key("cancel-handoff")
                        .autofocus()
                        .on_press(|st: &mut State| st.release_dialog = ReleaseDialog::None),
                )
                .child(spacer())
                .child(text("Esc cancels").font(SMALL).color(u.faint)),
        );

    modal_frame(u, "release-approval-dialog", "Publish to GameBanana", body)
}

fn confirm_dialog(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.confirm_url, MONO)
        .key("confirm-url")
        .label("Submission URL")
        .single_line()
        .w_full()
        .padding(11.0)
        .color(u.ink)
        .placeholder("https://gamebanana.com/mods/123456", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.confirm_url)
        .on_submit(|st: &mut State, _| start_confirm(st))
        .autofocus();

    let body = col()
        .w_full()
        .gap(20.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(7.0)
                        .child(u.eyebrow(&format!(
                            "{} · {}",
                            s.project()
                                .map(|project| project.name.clone())
                                .unwrap_or_else(|| "the open project".to_string()),
                            s.release
                                .info
                                .as_ref()
                                .map(|info| format!("v{}", info.version))
                                .unwrap_or_else(|| "no release".to_string())
                        )))
                        .child(text("Confirm the submission URL").font(H1).color(u.ink))
                        .child(
                            text("Paste the GameBanana page that now hosts this build. The CLI re-checks the archive hash and verifies the page names your project.")
                                .font(LEAD)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("✓", Tone::Ok, 42.0)),
        )
        .child(
            col()
                .w_full()
                .gap(7.0)
                .child(u.field_label("SUBMISSION URL"))
                .child(u.well_field().w_full().child(field)),
        )
        .child(u.note(
            "https://gamebanana.com/mods/<id> is the public form. A local test server at http://127.0.0.1: or http://localhost: adds --allow-http; every other URL fails the CLI check.",
        ))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    u.pill("Record and verify", "✓", ActionKind::Primary)
                        .key("confirm-submission")
                        .on_press(|st: &mut State| start_confirm(st)),
                )
                .child(
                    u.pill("Cancel", "esc", ActionKind::Ghost)
                        .key("cancel-confirm")
                        .on_press(|st: &mut State| st.release_dialog = ReleaseDialog::None),
                )
                .child(spacer())
                .child(text("Esc cancels").font(SMALL).color(u.faint)),
        );

    modal_frame(
        u,
        "release-confirm-dialog",
        "Confirm the submission URL",
        body,
    )
}
