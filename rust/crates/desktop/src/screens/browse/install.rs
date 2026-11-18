//! The install dialog, its plan summary and the VPK picker for archives with several mods.

use super::*;

// --- install ---------------------------------------------------------------

/// Start the next mod in a collection queue, or report the end.
pub(crate) fn start_next_in_queue(st: &mut State) {
    let Some((id, name)) = st.browse_view.queue.first().cloned() else {
        st.install.auto = false;
        if st.browse_view.queue_total > 0 {
            st.status = format!(
                "Installed {} from the collection.",
                plural(st.browse_view.queue_done, "mod", "mods")
            );
        }
        return;
    };
    st.browse_view.queue.remove(0);
    st.install = InstallView::for_mod(id, &name);
    st.install.auto = true;
    st.install.one_click = true;
    crate::data::start_install_plan(st);
}

/// What a finished install means for a queue. `plan_landed` is a validated plan without an
/// apply, `applied` a written slot.
pub fn after_install(st: &mut State, plan_landed: bool, applied: bool, failed: bool) -> bool {
    if applied {
        st.browse_view.queue_done = st.browse_view.queue_done.saturating_add(1);
        if !st.browse_view.queue.is_empty() {
            start_next_in_queue(st);
            return true;
        }
        st.install.auto = false;
        if st.browse_view.queue_total > 1 {
            st.status = format!(
                "Installed {} from the collection.",
                plural(st.browse_view.queue_done, "mod", "mods")
            );
        }
        return true;
    }
    if failed {
        // A multi-VPK archive is a choice, not a failure: the picker follows.
        let choosing = st
            .install
            .log
            .as_deref()
            .is_some_and(|log| log.contains("more than one pakNN_dir.vpk"));
        if !choosing {
            st.install.auto = false;
            st.browse_view.queue.clear();
        }
        return true;
    }
    if plan_landed && st.install.auto && !st.install.busy {
        crate::data::start_install_apply(st);
        return true;
    }
    false
}

/// The stored plan, as the dialog reads it. Every field is filled by `data::plan_summary`.
#[derive(Clone)]
#[allow(dead_code)]
pub struct PlanSummary {
    pub id: String,
    pub source_label: String,
    pub slot: u8,
    pub target: String,
    pub files: usize,
    pub bytes: u64,
    pub conflicts: usize,
    pub replace_required: bool,
    pub replaced: Vec<String>,
    pub gameinfo_repair: bool,
    pub warnings: Vec<String>,
    pub unmapped: Vec<String>,
    pub artifact: String,
}

pub struct InstallOutcome {
    pub plan: Option<PlanSummary>,
    pub applied: Option<String>,
    pub error: Option<String>,
}

pub struct InstallView {
    pub open: bool,
    /// What the install pipeline reads: a GameBanana mod id or a file's download link.
    pub target: String,
    /// The mod the dialog shows.
    pub mod_id: u64,
    pub label: String,
    /// The file chosen in the mod details, when it was not the mod's newest.
    pub file: Option<String>,
    pub slot: TextEdit,
    pub force: bool,
    pub plan: Option<PlanSummary>,
    pub busy: bool,
    pub log: Option<String>,
    pub tone: Tone,
    /// The press that opened this was a single one (a card, a link). Archives that carry a
    /// `.disable_gb1click` marker refuse that.
    pub one_click: bool,
    /// A queued install: plan, then apply, then the next mod.
    pub auto: bool,
    /// One VPK chosen from an archive that holds several.
    pub vpk: Option<String>,
    /// The VPKs the archive holds, read after a plan refused to choose, in install order.
    pub vpks: Vec<VpkChoice>,
    /// Files of `vpks` the player unchecked. Every file starts checked.
    pub unchecked: std::collections::BTreeSet<String>,
    /// The checked files going in (`multi`).
    pub multi: Job<super::multi::Done>,
}

impl Default for InstallView {
    fn default() -> Self {
        InstallView {
            open: false,
            target: String::new(),
            mod_id: 0,
            label: String::new(),
            file: None,
            slot: TextEdit::new(""),
            force: false,
            plan: None,
            busy: false,
            log: None,
            tone: Tone::Quiet,
            one_click: false,
            auto: false,
            vpk: None,
            vpks: Vec::new(),
            unchecked: std::collections::BTreeSet::new(),
            multi: Job::default(),
        }
    }
}

impl InstallView {
    pub fn for_mod(id: u64, name: &str) -> Self {
        InstallView {
            open: true,
            target: id.to_string(),
            mod_id: id,
            label: name.to_string(),
            one_click: true,
            ..InstallView::default()
        }
    }

    pub fn slot_value(&self) -> Option<u8> {
        self.slot.text.trim().parse::<u8>().ok()
    }
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A small section label inside the dialog.
pub(crate) fn label(u: &Ui, s: &str) -> El {
    text(s.to_string()).font(SMALL).color(u.faint)
}

/// The install dialog: the image on the left; title, install, artist and details on the right.
pub(crate) fn install_dialog(s: &State, u: &Ui) -> El {
    let v = &s.install;
    let card = card_by_id(s, v.mod_id);
    let installed = v.plan.is_some() && v.tone == Tone::Ok && !v.busy;

    // Header: title and the file or category, open on GameBanana, close.
    let mut title = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(v.label.clone()).font(H1).color(u.ink).truncate());
    let subtitle = v
        .file
        .clone()
        .or_else(|| card.map(|card| card.category.clone()))
        .filter(|line| !line.is_empty());
    if let Some(line) = subtitle {
        title = title.child(text(line).font(SMALL).color(u.muted).truncate());
    }
    let mut head = row().w_full().gap(4.0).items_start().child(title);
    if let Some(card) = card {
        let url = card.profile_url.clone();
        head = head.child(
            u.icon_button(&icons::EXTERNAL_LINK, "Open on GameBanana")
                .key("install-open-page")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    st.status = match tx.open_url(&url) {
                        Ok(()) => format!("Opened {url}."),
                        Err(error) => format!("The page could not be opened: {error}"),
                    };
                }),
        );
    }
    head = head.child(
        u.icon_button(&icons::X, "Close")
            .key("install-close")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.install.open = false),
    );

    // Left: the preview image, veiled like the card when the mod is mature.
    let image = {
        let veil = card.is_some_and(|card| veiled(s, card.id, card.nsfw));
        let mut frame = el()
            .w_full()
            .aspect_ratio(16.0 / 9.0)
            .radius(crate::ui::r(8.0))
            .clip_rounded(crate::ui::r(8.0))
            .child(image_fill(
                s,
                "install-image".to_string(),
                card.and_then(|card| card.thumbnail_url.as_ref()),
                [8.0; 4],
                veil,
            ));
        if veil && let Some(card) = card {
            frame = frame.child(nsfw_veil(u, "install", card.id, false));
        }
        frame
    };
    let left = col().w(400.0).gap(12.0).child(image);

    // Right: install, artist, details.
    let mut install = col().w_full().child(label(u, "Install").mb(4.0));
    if let Some(plan) = &v.plan {
        install = install
            .child(u.kv("Slot", &format!("{:02}", plan.slot), u.ink))
            .child(u.kv(
                "Files",
                &format!(
                    "{}, {}",
                    plural(plan.files, "file", "files"),
                    crate::model::format_size(plan.bytes)
                ),
                u.ink,
            ))
            .child(u.kv(
                "Overlap",
                &if plan.conflicts == 0 {
                    "None".to_string()
                } else {
                    format!(
                        "{} shared with a lower slot, which wins",
                        plural(plan.conflicts, "file", "files")
                    )
                },
                if plan.conflicts == 0 {
                    u.muted
                } else {
                    u.amber
                },
            ));
        if plan.replace_required {
            install = install.child(u.kv(
                "Replaces",
                &format!(
                    "{} in this slot, backed up first",
                    plural(plan.replaced.len(), "file", "files")
                ),
                u.amber,
            ));
        }
        for warning in &plan.warnings {
            install = install.child(
                text(warning.clone())
                    .font(SMALL)
                    .color(u.amber)
                    .wrap()
                    .mt(6.0),
            );
        }
        if !plan.unmapped.is_empty() {
            install = install.child(
                text(format!(
                    "{} could not be matched to a game path and will be skipped.",
                    plural(plan.unmapped.len(), "file", "files")
                ))
                .font(SMALL)
                .color(u.muted)
                .wrap()
                .mt(6.0),
            );
        }
        if plan.gameinfo_repair {
            install = install.child(
                text("Deadlock is not set up to load mods. Installing also fixes gameinfo.gi and keeps a backup.")
                    .font(SMALL)
                    .color(u.amber)
                    .wrap()
                    .mt(6.0),
            );
        }
    } else {
        install = install
            .child(
                row()
                    .w_full()
                    .h(40.0)
                    .gap(16.0)
                    .center_y()
                    .child(text("Slot").font(BODY).color(u.muted).w(120.0))
                    .child(
                        row()
                            .w(88.0)
                            .h(32.0)
                            .px(10.0)
                            .center_y()
                            .radius(crate::ui::r(8.0))
                            .fill(u.field)
                            .border(1.0, u.hair)
                            .child(
                                rune::text_input(&v.slot, BODY)
                                    .key("install-slot")
                                    .label("Addon slot")
                                    .single_line()
                                    .color(u.ink)
                                    .placeholder("Auto", u.faint)
                                    .caret(u.accent, u.accent.alpha(0.20))
                                    .on_edit(|st: &mut State| &mut st.install.slot)
                                    .grow(1.0)
                                    .min_w(0.0),
                            ),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .h(40.0)
                    .gap(16.0)
                    .center_y()
                    .child(text("Replace").font(BODY).color(u.muted).w(120.0))
                    .child(
                        u.toggle(v.force)
                            .key("install-force")
                            .label("Replace a used slot")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| st.install.force = !st.install.force),
                    )
                    .child(
                        text(if v.force {
                            "A used slot is backed up, then replaced"
                        } else {
                            "Only a free slot is used"
                        })
                        .font(SMALL)
                        .color(u.faint)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                    ),
            );
    }

    // Settings > Mods > Show what a mod needs: the requirements GameBanana lists.
    if let Ok(mod_id) = v.target.trim().parse::<u64>()
        && let Some(needs) = crate::screens::settings::extra::requirements_panel(s, u, mod_id)
    {
        install = install.child(needs);
    }
    // Several files in one archive: every one is checked, the player picks and orders them.
    if let Some(picker) = super::multi::picker(s, u) {
        install = install.child(picker);
    }
    let mut right = col().grow(1.0).min_w(0.0).gap(24.0).child(install);

    if let Some(card) = card {
        let mut details = col()
            .w_full()
            .child(label(u, "Details").mb(4.0))
            .child(u.kv(
                "Artist",
                if card.author.is_empty() {
                    "Unknown"
                } else {
                    &card.author
                },
                u.ink,
            ));
        if let Some(version) = card.version_label() {
            details = details.child(u.kv("Version", &version, u.muted));
        }
        if card.updated_ts > 0 {
            details = details.child(u.kv(
                "Updated",
                &gamebanana::relative_age(card.updated_ts, gamebanana::now_unix()),
                u.muted,
            ));
        }
        details = details
            .child(u.kv("Likes", &format_count(card.likes), u.muted))
            .child(u.kv("Views", &format_count(card.views), u.muted));
        right = right.child(details);
    }

    // Outcome line: an error, or the installed confirmation.
    let mut outcome: Option<El> = None;
    if let Some(log) = &v.log
        && !v.busy
    {
        if v.tone == Tone::Bad {
            outcome = Some(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_start()
                    .child(
                        icon(&icons::TRIANGLE_ALERT, 14.0)
                            .color(u.rose)
                            .mt(3.0)
                            .a11y_hidden(),
                    )
                    .child(
                        text(log.clone())
                            .font(SMALL)
                            .color(u.prose)
                            .wrap_anywhere()
                            .selectable(true)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        } else if installed {
            let slot = v.plan.as_ref().map(|plan| plan.slot).unwrap_or(0);
            outcome = Some(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(icon(&icons::CIRCLE_CHECK, 14.0).color(u.muted).a11y_hidden())
                    .child(
                        text(format!(
                            "Installed in slot {slot:02}. It loads the next time you start Deadlock."
                        ))
                        .font(SMALL)
                        .color(u.prose)
                        .wrap(),
                    ),
            );
        }
    }
    if !s.game_detected && v.plan.is_some() && !installed {
        outcome = Some(
            text("Set the game folder in Settings to install.")
                .font(SMALL)
                .color(u.amber),
        );
    }
    if let Some(line) = outcome {
        right = right.child(line);
    }

    // Footer: Close on the left of the one primary action.
    let plan_ready = !v.busy && !v.target.is_empty();
    let apply_ready = v.plan.is_some() && !v.busy && s.game_detected;
    let mut actions = row().w_full().gap(8.0).center_y().child(spacer());
    if installed {
        actions = actions.child(
            u.action("Done", ActionKind::Secondary)
                .key("install-done")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.install.open = false),
        );
    } else {
        actions = actions.child(
            u.action("Cancel", ActionKind::Ghost)
                .key("install-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.install.open = false),
        );
        if let Some(plan) = &v.plan {
            actions = actions
                .child(
                    u.action("Download again", ActionKind::Secondary)
                        .key("install-replan")
                        .disabled(!plan_ready)
                        .opacity(if plan_ready { 1.0 } else { 0.45 })
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            st.install.plan = None;
                            crate::data::start_install_plan(st);
                        }),
                )
                .child(
                    u.icon_action(
                        &icons::DOWNLOAD,
                        &if v.busy {
                            "Installing".to_string()
                        } else {
                            format!("Install in slot {:02}", plan.slot)
                        },
                        ActionKind::Primary,
                    )
                    .key("install-apply")
                    .disabled(!apply_ready)
                    .opacity(if apply_ready { 1.0 } else { 0.45 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| crate::data::start_install_apply(st)),
                );
        } else if v.vpks.len() < 2 {
            // With several files the picker's own Install is the one action.
            actions = actions.child(
                u.icon_action(
                    &icons::DOWNLOAD,
                    if v.busy { "Downloading" } else { "Download" },
                    ActionKind::Primary,
                )
                .key("install-plan")
                .disabled(!plan_ready)
                .opacity(if plan_ready { 1.0 } else { 0.45 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| crate::data::start_install_plan(st)),
            );
        }
    }

    let close = |st: &mut State| st.install.open = false;
    u.dialog_shell()
        .key("install-dialog")
        .role("dialog")
        .label(format!("Install {}", v.label))
        .w(880.0)
        .max_w(880.0)
        .padding(24.0)
        .gap(20.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(head)
        .child(
            row()
                .w_full()
                .gap(24.0)
                .items_start()
                .child(left)
                .child(right),
        )
        .child(actions)
}

