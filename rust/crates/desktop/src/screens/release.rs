//! Release: the latest build, the install plan, the GameBanana handoff and the review queue.
//!
//! Every value on this screen comes from a read. Publishing is a browser handoff the user approves
//! in the dialog below; the app never uploads the archive itself.

use std::path::Path;

use rune::{Color, El, Key, KeyEvent, TextEdit, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::data;
use crate::icons;
use crate::model::{DeployPlanView, ReleaseDialog, ReleaseInfo, ReviewItem, Screen, UploadInfo};
use crate::publish::PublishAction;
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, MONO_MED, SMALL, Ui};

const PROMPT_VALIDATE: &str = "Validate and release this project. Read project.json, check every target against the game index, validate the staging payload and prepare builds/<version> with its release manifest. List every warning before packing. Do not write to the game install, edit gameinfo.gi or upload anything.";

const PROMPT_PLAN: &str = "Plan the deploy for this project. Read the latest release manifest and the installed addons, then report the target slot, whether a replacement is required, lower-slot conflicts and the gameinfo SearchPaths state. This plan is read only: do not copy files or edit gameinfo.gi.";

const PROMPT_MIGRATE: &str = "Migrate the moved paths for this project. Use the current game index to list the exact old and new path for each target that moved, explain any ambiguous basename instead of guessing, and update project.json only after I approve the plan. Do not touch the game install.";

const SUBMIT_PAGE: &str = "https://gamebanana.com/mods/submit";

pub fn view(s: &State, u: &Ui) -> El {
    let mut actions = row().gap(8.0).center_y().child(
        u.icon_button(&icons::REFRESH_CW, "Rescan")
            .key("release-rescan")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| rescan(st)),
    );
    if s.release.info.is_some() {
        actions = actions.child(
            u.icon_action(
                &icons::EXTERNAL_LINK,
                "Publish to GameBanana",
                ActionKind::Primary,
            )
            .key("publish-handoff")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| open_approval(st)),
        );
    }

    let mut detail = Vec::new();
    if let Some(project) = s.project() {
        detail.push(project.name.clone());
    }
    if let Some(info) = &s.release.info {
        detail.push(format!("v{}", info.version));
    }
    if s.publish_job.is_running() {
        detail.push("Publishing".to_string());
    }

    let mut body = col().w_full().gap(32.0);
    body = match &s.release.info {
        Some(info) => body.child(build_section(u, info)),
        None => body.child(no_release(s, u)),
    };
    body = body
        .child(plan_section(s, u))
        .child(publish_section(s, u))
        .child(review_section(s, u))
        .child(agent_section(u));

    subpage::page(
        "release",
        subpage::head(u, "Release", &detail.join("  ·  "), Some(actions)),
        body,
    )
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

fn kv_text(u: &Ui, key: &str, value: impl Into<String>, color: Color) -> El {
    subpage::kv(
        u,
        key,
        text(value.into()).font(BODY).color(color).wrap_anywhere(),
    )
}

fn kv_mono(u: &Ui, key: &str, value: impl Into<String>, color: Color) -> El {
    subpage::kv(
        u,
        key,
        text(value.into())
            .font(MONO)
            .color(color)
            .wrap_anywhere()
            .selectable(true),
    )
}

/// A line that names a real problem: an icon in the state colour, then the message.
fn problem(u: &Ui, color: Color, message: impl Into<String>) -> El {
    row()
        .w_full()
        .gap(10.0)
        .items_start()
        .child(
            el().size(20.0, 20.0).center().child(
                icon(&icons::TRIANGLE_ALERT, 14.0)
                    .color(color)
                    .a11y_hidden(),
            ),
        )
        .child(
            text(message.into())
                .font(BODY)
                .color(u.prose)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

fn build_section(u: &Ui, info: &ReleaseInfo) -> El {
    let mut rows =
        subpage::rows(u).child(kv_text(u, "Version", format!("v{}", info.version), u.ink));
    if let Some(notes) = &info.notes {
        rows = rows.child(kv_text(u, "Notes", notes.clone(), u.prose));
    }
    rows = rows
        .child(kv_mono(
            u,
            "VPK",
            info.vpk_file.clone(),
            if info.vpk_present { u.ink } else { u.rose },
        ))
        .child(kv_text(u, "Slot", format!("{:02}", info.slot), u.ink))
        .child(kv_text(
            u,
            "Contents",
            format!("{} entries, {}", info.entries, info.size_label()),
            u.ink,
        ));
    if !info.chunk_files.is_empty() {
        rows = rows.child(kv_mono(u, "Chunks", info.chunks_label(), u.muted));
    }
    rows = rows
        .child(kv_text(
            u,
            "Created",
            short_stamp(&info.created_at),
            u.muted,
        ))
        .child(match &info.baseline_captured_at {
            Some(stamp) => kv_text(u, "Baseline", short_stamp(stamp), u.muted),
            None => kv_text(u, "Baseline", "Not recorded", u.amber),
        })
        .child(kv_mono(u, "Folder", info.dir.clone(), u.muted));

    let mut c = col()
        .key("release-build")
        .w_full()
        .gap(8.0)
        .child(section(u, "Latest build", "", None))
        .child(rows);
    if !info.vpk_present {
        c = c.child(el().h(4.0)).child(problem(
            u,
            u.rose,
            "The VPK is missing from the release folder. Release again before publishing.",
        ));
    }
    c
}

fn no_release(s: &State, u: &Ui) -> El {
    let detail = s
        .release
        .lookup_error
        .clone()
        .unwrap_or_else(|| "Ask the agent to validate and release this project.".to_string());
    col()
        .key("release-build")
        .w_full()
        .gap(8.0)
        .child(section(u, "Latest build", "", None))
        .child(
            col()
                .w_full()
                .py(16.0)
                .gap(12.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    col()
                        .w_full()
                        .gap(2.0)
                        .child(text("No release yet").font(BODY_MED).color(u.ink))
                        .child(text(detail).font(BODY).color(u.muted).wrap()),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            u.action("Validate and release", ActionKind::Primary)
                                .key("release-validate")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| ask(st, PROMPT_VALIDATE)),
                        )
                        .child(
                            u.action("Plan the deploy", ActionKind::Secondary)
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| ask(st, PROMPT_PLAN)),
                        ),
                ),
        )
}

fn plan_section(s: &State, u: &Ui) -> El {
    let meta = s
        .release
        .plan
        .as_ref()
        .map(|p| format!("{} entries", p.entry_count))
        .unwrap_or_default();
    let head = section(u, "Install plan", &meta, None);
    let body = match (&s.release.plan, &s.release.plan_error) {
        (Some(plan), _) => plan_body(u, plan),
        (None, Some(error)) => col()
            .w_full()
            .gap(8.0)
            .child(problem(u, u.rose, "The install plan could not be read."))
            .child(
                text(error.clone())
                    .font(SMALL)
                    .color(u.muted)
                    .pl(30.0)
                    .wrap(),
            ),
        (None, None) => {
            let detail = if s.release.info.is_none() {
                "Release this project first."
            } else if !s.game_detected {
                "Deadlock was not found. Set the game folder in Settings."
            } else {
                "Rescan to compare this release with your installed mods."
            };
            let mut actions = row().gap(8.0).child(
                u.action("Rescan", ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| rescan(st)),
            );
            if s.release.info.is_some() && !s.game_detected {
                actions = actions.child(
                    u.action("Open Settings", ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.go(Screen::Settings)),
                );
            }
            col()
                .w_full()
                .py(16.0)
                .gap(12.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(detail).font(BODY).color(u.muted).wrap())
                .child(actions)
        }
    };
    col()
        .key("release-deploy")
        .w_full()
        .gap(8.0)
        .child(head)
        .child(body)
}

fn plan_body(u: &Ui, plan: &DeployPlanView) -> El {
    let mut rows = subpage::rows(u)
        .child(kv_mono(u, "Target", plan.target.clone(), u.ink))
        .child(if plan.replace_required {
            kv_text(u, "Replaces", "A file already uses this slot", u.amber)
        } else {
            kv_text(u, "Replaces", "Nothing, the slot is free", u.muted)
        })
        .child(if plan.gameinfo_repair {
            kv_text(u, "gameinfo.gi", "Needs the citadel/addons line", u.rose)
        } else {
            kv_text(u, "gameinfo.gi", "Ready", u.muted)
        });
    if !plan.chunk_targets.is_empty() {
        rows = rows.child(kv_mono(u, "Chunks", plan.chunk_targets.join(", "), u.muted));
    }

    let mut c = col().w_full().gap(8.0).child(rows);

    if !plan.conflicts.is_empty() {
        let mut list = subpage::rows(u);
        for (i, conflict) in plan.conflicts.iter().enumerate() {
            let mut paths = col().w_full().gap(2.0);
            for path in conflict.paths.iter().take(4) {
                paths = paths.child(text(path.clone()).font(MONO).color(u.faint).truncate());
            }
            if conflict.paths.len() > 4 {
                paths = paths.child(
                    text(format!("and {} more", conflict.paths.len() - 4))
                        .font(SMALL)
                        .color(u.faint),
                );
            }
            list = list.child(
                col()
                    .key(format!("deploy-conflict-{i}"))
                    .w_full()
                    .py(12.0)
                    .gap(6.0)
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        row()
                            .w_full()
                            .gap(12.0)
                            .center_y()
                            .child(
                                text(conflict.slot_label())
                                    .font(MONO)
                                    .color(u.amber)
                                    .w(20.0),
                            )
                            .child(
                                text(conflict.file.clone())
                                    .font(MONO_MED)
                                    .color(u.ink)
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .truncate(),
                            )
                            .child(text(conflict.paths_label()).font(SMALL).color(u.muted)),
                    )
                    .child(paths.pl(32.0)),
            );
        }
        c = c
            .child(el().h(8.0))
            .child(subpage::group_head(
                u,
                "Lower slots that win",
                &plan.conflicts.len().to_string(),
            ))
            .child(list);
    }

    if !plan.warnings.is_empty() {
        let mut list = col().w_full().gap(8.0);
        for warning in &plan.warnings {
            list = list.child(problem(u, u.amber, warning.clone()));
        }
        c = c
            .child(el().h(8.0))
            .child(subpage::group_head(
                u,
                "Warnings",
                &plan.warnings.len().to_string(),
            ))
            .child(list);
    }

    let mut files = subpage::rows(u);
    if plan.entry_preview.is_empty() {
        files = files.child(
            row().w_full().h(36.0).center_y().child(
                text("The release VPK is missing.")
                    .font(BODY)
                    .color(u.amber),
            ),
        );
    } else {
        for path in &plan.entry_preview {
            files = files.child(
                row()
                    .w_full()
                    .h(36.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(text(path.clone()).font(MONO).color(u.prose).truncate()),
            );
        }
        if plan.entry_count > plan.entry_preview.len() {
            files = files.child(
                row().w_full().h(36.0).center_y().child(
                    text(format!(
                        "and {} more",
                        plan.entry_count - plan.entry_preview.len()
                    ))
                    .font(SMALL)
                    .color(u.faint),
                ),
            );
        }
    }
    c.child(el().h(8.0))
        .child(subpage::group_head(
            u,
            "Files",
            &plan.entry_count.to_string(),
        ))
        .child(files)
}

fn state_text(upload: &UploadInfo) -> String {
    match upload.state.as_str() {
        "awaiting_user" => "Waiting for your upload".to_string(),
        "recorded" => "Recorded".to_string(),
        "verified" => "Verified".to_string(),
        _ => upload.state_label(),
    }
}

fn publish_section(s: &State, u: &Ui) -> El {
    let upload = s.release.upload.as_ref();
    let mut c =
        col()
            .key("release-upload")
            .w_full()
            .gap(8.0)
            .child(section(u, "Publishing", "", None));

    if let Some(error) = &s.release.upload_error {
        c = c.child(problem(
            u,
            u.rose,
            format!("upload.json could not be read: {error}"),
        ));
    }

    match upload {
        Some(upload) => {
            let mut rows = subpage::rows(u)
                .child(kv_text(u, "Status", state_text(upload), u.ink))
                .child(kv_mono(u, "Archive", upload.archive_file.clone(), u.ink))
                .child(kv_text(u, "Size", upload.archive_size_label(), u.muted))
                .child(kv_mono(
                    u,
                    "SHA-256",
                    upload.archive_sha256.clone(),
                    u.prose,
                ));
            rows = rows.child(match &upload.submission_url {
                Some(url) => kv_mono(u, "Page", url.clone(), u.ink),
                None => kv_text(u, "Page", "Not confirmed yet", u.muted),
            });
            rows = rows
                .child(kv_text(
                    u,
                    "Handed off",
                    upload
                        .created_at
                        .as_deref()
                        .map(short_stamp)
                        .unwrap_or_else(|| "Not recorded".to_string()),
                    u.muted,
                ))
                .child(kv_text(
                    u,
                    "Confirmed",
                    upload
                        .confirmed_at
                        .as_deref()
                        .map(short_stamp)
                        .unwrap_or_else(|| "Not yet".to_string()),
                    u.muted,
                ));
            c = c.child(rows);
        }
        None => {
            let archive = s
                .release
                .publish
                .as_ref()
                .and_then(|publish| publish.archive_file.clone());
            c = c.child(
                subpage::rows(u)
                    .child(kv_text(u, "Status", "Not published", u.muted))
                    .child(match archive {
                        Some(file) => kv_mono(u, "Archive", file, u.ink),
                        None => kv_text(u, "Archive", "Not prepared", u.muted),
                    }),
            );
        }
    }

    c.child(
        row()
            .w_full()
            .pt(8.0)
            .gap(8.0)
            .child(
                u.action("Confirm submission URL", ActionKind::Secondary)
                    .key("release-open-confirm")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| open_confirm(st)),
            )
            .child(
                u.action("Refresh", ActionKind::Ghost)
                    .key("release-refresh-upload")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| refresh_upload(st)),
            ),
    )
}

fn review_section(s: &State, u: &Ui) -> El {
    let queue = s.release.review.as_ref();
    let meta = queue.map(|q| q.summary()).unwrap_or_default();
    let run = u
        .icon_action(&icons::REFRESH_CW, "Run", ActionKind::Ghost)
        .key("release-run-queue")
        .mr(-12.0)
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| scan_queue(st));
    let mut c = col().key("release-review").w_full().gap(8.0).child(section(
        u,
        "Review queue",
        &meta,
        Some(run),
    ));

    let Some(queue) = queue else {
        return c.child(
            col()
                .w_full()
                .py(16.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text("Run the queue to check every project against the current game files.")
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                ),
        );
    };

    if let Some(error) = &queue.error {
        c = c.child(problem(u, u.rose, error.clone()));
    }
    if queue.items.is_empty() {
        return c.child(
            col()
                .w_full()
                .py(16.0)
                .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text("No projects in this workspace.")
                        .font(BODY)
                        .color(u.muted),
                ),
        );
    }
    let mut list = subpage::rows(u);
    for (i, item) in queue.items.iter().enumerate() {
        list = list.child(review_row(u, i, item));
    }
    c.child(list).child(
        text(queue.workspace.clone())
            .font(SMALL)
            .color(u.faint)
            .pt(4.0)
            .truncate(),
    )
}

/// One project in the review queue. The Updates screen shows the same rows.
pub fn review_row(u: &Ui, i: usize, item: &ReviewItem) -> El {
    let (state, color) = review_state(u, item);
    let mut meta = vec![format!("v{}", item.version)];
    if item.slot.is_some() {
        meta.push(format!("slot {}", item.slot_label()));
    }
    col()
        .key(format!("review-{i}"))
        .w_full()
        .py(12.0)
        .gap(6.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(item.label())
                        .font(BODY_MED)
                        .color(u.ink)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(text(meta.join("  ·  ")).font(SMALL).color(u.faint))
                .child(spacer())
                .child(text(state).font(BODY).color(color)),
        )
        .child(review_lines(u, item))
}

/// The counts, moves, warnings and error under a review row.
pub fn review_lines(u: &Ui, item: &ReviewItem) -> El {
    let mut c = col().w_full().gap(4.0);
    let counts = counts_label(item);
    if !counts.is_empty() {
        c = c.child(text(counts).font(SMALL).color(u.muted));
    }
    for migration in &item.migrations {
        c = c.child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(
                    text(migration.from.clone())
                        .font(MONO)
                        .color(u.faint)
                        .truncate(),
                )
                .child(
                    icon(&icons::CHEVRON_RIGHT, 12.0)
                        .color(u.faint)
                        .a11y_hidden(),
                )
                .child(
                    text(migration.to.clone())
                        .font(MONO)
                        .color(u.prose)
                        .truncate(),
                ),
        );
    }
    for warning in &item.warnings {
        c = c.child(text(warning.clone()).font(SMALL).color(u.amber).wrap());
    }
    if let Some(error) = &item.error {
        c = c.child(
            text(format!("Check failed: {error}"))
                .font(SMALL)
                .color(u.rose)
                .wrap(),
        );
    }
    c
}

pub fn counts_label(item: &ReviewItem) -> String {
    let mut parts = Vec::new();
    if item.changed > 0 {
        parts.push(format!("{} changed", item.changed));
    }
    if item.missing > 0 {
        parts.push(format!("{} missing", item.missing));
    }
    if item.moved > 0 {
        parts.push(format!("{} moved", item.moved));
    }
    if item.unbaselined > 0 {
        parts.push(format!("{} without a baseline", item.unbaselined));
    }
    if item.unchanged > 0 {
        parts.push(format!("{} unchanged", item.unchanged));
    }
    parts.join(", ")
}

pub fn review_state(u: &Ui, item: &ReviewItem) -> (&'static str, Color) {
    if item.error.is_some() {
        ("Check failed", u.rose)
    } else if item.missing > 0 {
        ("Needs review", u.rose)
    } else if item.needs_review() {
        ("Needs review", u.amber)
    } else {
        ("Up to date", u.muted)
    }
}

fn agent_section(u: &Ui) -> El {
    col()
        .key("release-actions")
        .w_full()
        .gap(8.0)
        .child(section(u, "Ask the agent", "", None))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .flex_wrap()
                .child(
                    u.action("Validate and release", ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| ask(st, PROMPT_VALIDATE)),
                )
                .child(
                    u.action("Plan the deploy", ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| ask(st, PROMPT_PLAN)),
                )
                .child(
                    u.action("Migrate moved paths", ActionKind::Secondary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| ask(st, PROMPT_MIGRATE)),
                ),
        )
}

fn ask(st: &mut State, prompt: &str) {
    let label = match st.project() {
        Some(project) => project.name.clone(),
        None => "the open project".to_string(),
    };
    st.ask_agent(format!("{prompt}\n\nProject: {label}."));
}

/// Re-read the release and the plan. A demo scene keeps its data so the snapshot stays hermetic.
fn rescan(st: &mut State) {
    if st.demo {
        st.status = "Demo data does not rescan.".to_string();
        return;
    }
    data::refresh_release(st);
    st.status = "Release rescanned.".to_string();
}

/// Load the review queue. It reads the game catalog, so it runs on a press, not every frame.
fn scan_queue(st: &mut State) {
    if st.demo {
        st.status = "Demo data does not rescan.".to_string();
        return;
    }
    crate::data::start_review_scan(st);
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
        st.status = "Demo data: nothing was published.".to_string();
        return;
    }
    let workspace = st.paths.workspace.clone();
    let wake = st.wake.clone();
    match st
        .publish_job
        .start(PublishAction::Upload, Path::new(&workspace), None, wake)
    {
        Ok(()) => st.status = "Opening GameBanana in your browser.".to_string(),
        Err(error) => st.status = error,
    }
}

fn start_confirm(st: &mut State) {
    let url = st.confirm_url.text.trim().to_string();
    if url.is_empty() {
        st.status = "Enter the GameBanana page URL first.".to_string();
        return;
    }
    st.release_dialog = ReleaseDialog::None;
    if st.demo {
        st.status = "Demo data: nothing was confirmed.".to_string();
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
        Ok(()) => st.status = "Checking the submission page.".to_string(),
        Err(error) => st.status = error,
    }
}

fn refresh_upload(st: &mut State) {
    if st.demo {
        st.status = "Demo data does not rescan.".to_string();
        return;
    }
    data::refresh_upload(st);
    st.status = match &st.release.upload {
        Some(upload) => format!("Publishing status: {}.", state_text(upload)),
        None => "Not published yet.".to_string(),
    };
}

// The modal layer is mounted at the window root so the scrim dims the whole canvas and the
// dialog never clips inside a scroll region.

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let content = match s.release_dialog {
        ReleaseDialog::ApproveUpload => approval_dialog(s, u),
        ReleaseDialog::ConfirmUrl => confirm_dialog(s, u),
        ReleaseDialog::None => return None,
    };
    Some(u.modal_layer("release-modal-layer", content))
}

fn modal_frame(u: &Ui, key: &str, title: &str, inner: El) -> El {
    u.dialog_shell()
        .key(key)
        .role("dialog")
        .label(title)
        .w(480.0)
        .padding(24.0)
        .gap(16.0)
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
        .child(inner)
}

fn dialog_kv(u: &Ui, key: &str, value: El) -> El {
    row()
        .w_full()
        .py(6.0)
        .gap(16.0)
        .items_start()
        .child(text(key.to_string()).font(BODY).color(u.muted).w(88.0))
        .child(value.grow(1.0).min_w(0.0))
}

fn approval_dialog(s: &State, u: &Ui) -> El {
    let upload = s.release.upload.as_ref();
    let publish = s.release.publish.as_ref();
    let info = s.release.info.as_ref();
    let project = s
        .project()
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "The open project".to_string());
    let version = info
        .map(|info| format!("v{}", info.version))
        .unwrap_or_else(|| "No release".to_string());
    let archive = upload
        .map(|upload| upload.archive_file.clone())
        .or_else(|| publish.and_then(|publish| publish.archive_file.clone()))
        .unwrap_or_else(|| "Not prepared".to_string());
    let bytes = upload
        .map(|upload| upload.archive_bytes)
        .or_else(|| publish.and_then(|publish| publish.archive_bytes))
        .unwrap_or(0);
    let sha = upload
        .map(|upload| upload.archive_sha256.clone())
        .or_else(|| publish.and_then(|publish| publish.archive_sha256.clone()))
        .unwrap_or_else(|| "Not recorded".to_string());
    let submit = upload
        .map(UploadInfo::submit_label)
        .unwrap_or_else(|| SUBMIT_PAGE.to_string());
    let size = if bytes > 0 {
        crate::model::format_size(bytes)
    } else {
        "Not recorded".to_string()
    };

    let body = col()
        .w_full()
        .gap(16.0)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(text("Publish to GameBanana").font(H1).color(u.ink))
                .child(
                    text("Sigil saves this handoff and opens the submission page. You upload the archive and sign in yourself.")
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(
            col()
                .w_full()
                .child(dialog_kv(u, "Project", text(project).font(BODY).color(u.ink)))
                .child(dialog_kv(u, "Version", text(version).font(BODY).color(u.ink)))
                .child(dialog_kv(
                    u,
                    "Archive",
                    text(archive).font(BODY).color(u.ink).wrap_anywhere(),
                ))
                .child(dialog_kv(u, "Size", text(size).font(BODY).color(u.muted)))
                .child(dialog_kv(
                    u,
                    "SHA-256",
                    text(sha)
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ))
                .child(dialog_kv(
                    u,
                    "Page",
                    text(submit)
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                )),
        )
        .child(
            row()
                .w_full()
                .pt(8.0)
                .gap(8.0)
                .center_y()
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .key("cancel-handoff")
                        .autofocus()
                        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.release_dialog = ReleaseDialog::None),
                )
                .child(
                    u.action("Approve and open", ActionKind::Primary)
                        .key("approve-handoff")
                        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| start_upload(st)),
                ),
        );

    modal_frame(u, "release-approval-dialog", "Publish to GameBanana", body)
}

fn confirm_dialog(s: &State, u: &Ui) -> El {
    let field = rune::text_input(&s.confirm_url, MONO)
        .key("confirm-url")
        .label("Submission URL")
        .single_line()
        .w_full()
        .color(u.ink)
        .placeholder("https://gamebanana.com/mods/123456", u.faint)
        .caret(u.accent, u.accent.alpha(0.22))
        .on_edit(|st: &mut State| &mut st.confirm_url)
        .on_submit(|st: &mut State, _| start_confirm(st))
        .autofocus();

    let body = col()
        .w_full()
        .gap(16.0)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(text("Confirm the submission").font(H1).color(u.ink))
                .child(
                    text("Paste the GameBanana page for this build. Sigil checks the archive and that the page names your project.")
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair_strong)
                .child(field.grow(1.0).min_w(0.0)),
        )
        .child(
            row()
                .w_full()
                .pt(8.0)
                .gap(8.0)
                .center_y()
                .child(spacer())
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .key("cancel-confirm")
                        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.release_dialog = ReleaseDialog::None),
                )
                .child(
                    u.action("Confirm", ActionKind::Primary)
                        .key("confirm-submission")
                        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| start_confirm(st)),
                ),
        );

    modal_frame(u, "release-confirm-dialog", "Confirm the submission", body)
}
