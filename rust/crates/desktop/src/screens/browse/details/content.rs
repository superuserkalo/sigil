//! Files, creator, changelog and comments for the mod details.

use super::*;

pub(crate) fn files_section(s: &State, u: &Ui, d: &Details, profile: &ModProfile) -> El {
    let mut c = col().w_full().gap(8.0).child(section_head(
        u,
        None,
        "Files",
        (profile.files.len() > 1).then_some(profile.files.len() as u64),
    ));
    if profile.files.is_empty() {
        c = c.child(
            text("This mod has no downloadable files.")
                .font(BODY)
                .color(u.muted),
        );
    }
    let only = profile.files.len() == 1;
    for file in &profile.files {
        c = c.child(file_card(s, u, d, file, only));
    }
    if !profile.archived_files.is_empty() {
        let open = d.archived_open;
        c = c.child(
            row()
                .key("details-archived")
                .w_full()
                .h(32.0)
                .px(4.0)
                .gap(8.0)
                .center_y()
                .focusable(true)
                .role("button")
                .label("Archived files")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if let Some(details) = st.browse_view.details.as_mut() {
                        details.archived_open = !details.archived_open;
                    }
                })
                .child(
                    icon(
                        if open {
                            &icons::CHEVRON_DOWN
                        } else {
                            &icons::CHEVRON_RIGHT
                        },
                        16.0,
                    )
                    .color(u.muted)
                    .a11y_hidden(),
                )
                .child(text("Archived files").font(BODY_MED).color(u.muted))
                .child(spacer())
                .child(
                    text(profile.archived_files.len().to_string())
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
        if open {
            for file in &profile.archived_files {
                c = c.child(file_card(s, u, d, file, false));
            }
        }
    }
    c
}

pub(crate) fn file_card(s: &State, u: &Ui, d: &Details, file: &ModFile, only: bool) -> El {
    let mut meta = row()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(
            text(crate::model::format_size(file.size_bytes))
                .font(SMALL)
                .color(u.muted),
        )
        .child(text("-").font(SMALL).color(u.faint))
        .child(
            text(format!(
                "{} download{}",
                group_digits(file.downloads),
                if file.downloads == 1 { "" } else { "s" }
            ))
            .font(SMALL)
            .color(u.muted),
        );
    if file.added_ts > 0 {
        // The separator travels with the date, so a wrapped line never ends on a dash.
        meta = meta.child(
                row()
                    .gap(4.0)
                    .center_y()
                    .child(text("-").font(SMALL).color(u.faint).mr(2.0))
                    .child(icon(&CLOCK, 12.0).color(u.faint).a11y_hidden())
                    .child(
                        text(gamebanana::calendar_date(file.added_ts))
                            .font(SMALL)
                            .color(u.muted),
                    ),
            );
    }
    let mut middle = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(file.name.clone()).font(BODY_MED).color(u.ink).truncate());
    if !file.description.is_empty() {
        middle = middle.child(
            text(file.description.clone())
                .font(SMALL)
                .color(u.muted)
                .truncate(),
        );
    }
    middle = middle.child(meta);

    let id = d.id;
    let section = d.section;
    let name = d.name.clone();
    let file_name = file.name.clone();
    let url = file.download_url.clone();
    let busy = s.install.open && s.install.mod_id == id && s.install.busy;
    let install = row()
        .key(format!("file-install-{}", file.id))
        .h(36.0)
        .min_w(110.0)
        .px(16.0)
        .gap(8.0)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(u.accent.alpha(0.10))
        .border(1.0, u.accent.alpha(0.55))
        .hover(u.accent.alpha(0.18))
        .press(u.accent.alpha(0.24))
        .focusable(true)
        .role("button")
        .label(format!("Install {}", file.name))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let mut view = InstallView::for_mod(id, &name);
            // The mod id installs its newest file; any other file, and every sound or WiP
            // file, goes by its own link.
            if (!only || section != BrowseSection::Mods) && !url.is_empty() {
                view.target = url.clone();
            }
            view.file = Some(file_name.clone());
            // A file the player picked is not a one-click install.
            view.one_click = false;
            view.vpk = None;
            st.install = view;
        })
        .child(icon(&icons::DOWNLOAD, 16.0).color(u.ink).a11y_hidden())
        .child(
            text(if busy { "Installing" } else { "Install" })
                .font(BODY_MED)
                .color(u.ink)
                .a11y_hidden(),
        );

    row()
        .key(format!("file-{}", file.id))
        .w_full()
        .padding(12.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if file.archived {
            Color::WHITE.alpha(0.012)
        } else {
            Color::WHITE.alpha(0.025)
        })
        .border(1.0, u.hair_strong)
        .child(
            el().size(40.0, 40.0)
                .center()
                .radius(crate::ui::r(6.0))
                .fill(Color::WHITE.alpha(0.05))
                .child(
                    icon(&FILE_ARCHIVE, 20.0)
                        .color(if file.archived { u.faint } else { u.muted })
                        .a11y_hidden(),
                ),
        )
        .child(middle)
        .child(install)
}

pub(crate) fn artist_card(s: &State, u: &Ui, member: &ModMember, fallback: &str) -> El {
    let name = if member.name.is_empty() {
        fallback.to_string()
    } else {
        member.name.clone()
    };
    let url = member.profile_url.clone();
    let author = fallback.to_string();
    let creator = Pick {
        id: member.id,
        name: name.clone(),
    };
    let mut who = row()
        .key("details-artist-link")
        .min_w(0.0)
        .gap(6.0)
        .center_y()
        .child(
            text(name.clone())
                .font(TextStyle::new(15.0, 620.0).lh(20.0))
                .color(u.ink)
                .truncate()
                .min_w(0.0)
                .shrink(),
        );
    if !url.is_empty() {
        who = who
            .focusable(true)
            .role("link")
            .label(format!("{name} on GameBanana"))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url))
            .child(icon(&icons::CHEVRON_RIGHT, 16.0).color(u.faint).a11y_hidden());
    }
    row()
        .key("details-artist")
        .w_full()
        .padding(12.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(12.0))
        .fill(Color::WHITE.alpha(0.02))
        .border(1.0, u.hair_strong)
        .child(avatar(s, u, Some(member), &name, 44.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text("Creator").font(SMALL).color(u.faint))
                .child(who)
                .child(donation_row(u, member)),
        )
        .child(
            row()
                .key("details-more-artist")
                .h(32.0)
                .px(10.0)
                .gap(6.0)
                .center()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .press(Color::WHITE.alpha(0.08))
                .focusable(true)
                .role("button")
                .label(format!("More from {}", creator.name))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    let creator = creator.clone();
                    close_details(st);
                    set_filters(st, Some(tx), move |f| {
                        f.creator = Some(creator);
                        f.favorites = false;
                        if f.owned == Owned::Installed {
                            f.owned = Owned::Any;
                        }
                    });
                })
                .child(icon(&icons::USER, 14.0).color(u.muted).a11y_hidden())
                .child(text("More").font(SMALL).color(u.muted).a11y_hidden()),
        )
        .child(
            row()
                .key("details-hide-artist")
                .h(32.0)
                .px(10.0)
                .gap(6.0)
                .center()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .press(Color::WHITE.alpha(0.08))
                .focusable(true)
                .role("button")
                .label("Hide creator")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| hide_author(st, author.clone()))
                .child(icon(&EYE_OFF, 14.0).color(u.muted).a11y_hidden())
                .child(text("Hide creator").font(SMALL).color(u.muted).a11y_hidden()),
        )
}

/// A creator's donation links, as small buttons under their name.
pub(crate) fn donation_row(u: &Ui, member: &ModMember) -> El {
    let mut r = row().w_full().gap(6.0).mt(4.0).flex_wrap();
    for link in &member.donations {
        let url = link.url.clone();
        r = r.child(
            row()
                .key(format!("details-donation-{}", link.title))
                .h(26.0)
                .px(10.0)
                .center_y()
                .radius(13.0)
                .fill(u.over.alpha(0.05))
                .border(1.0, u.hair_strong)
                .hover(u.hover)
                .focusable(true)
                .role("link")
                .label(format!("Support the creator on {}", link.title))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url))
                .child(
                    text(link.title.clone())
                        .font(SMALL)
                        .color(u.prose),
                ),
        );
    }
    r
}

pub(crate) fn note_box(u: &Ui, message: &str) -> El {    row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .radius(crate::ui::r(8.0))
        .fill(Color::WHITE.alpha(0.025))
        .border(1.0, u.hair_strong)
        .child(text(message.to_string()).font(BODY).color(u.muted))
}

pub(crate) fn update_title(update: &ModUpdate) -> String {
    match (&update.version, update.title.is_empty()) {
        (Some(version), false) => format!("{} - {}", version_label(version), update.title),
        (Some(version), true) => version_label(version),
        (None, _) => update.title.clone(),
    }
}

pub(crate) fn version_label(version: &str) -> String {
    if version.starts_with(['v', 'V']) {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

pub(crate) fn changelog_section(u: &Ui, d: &Details, profile: &ModProfile) -> El {
    let open = d.changelog_open;
    let total = match &d.updates {
        Some(Ok(updates)) => updates.total as u64,
        _ => profile.updates_count,
    };
    let mut head = row()
        .key("details-changelog")
        .w_full()
        .gap(8.0)
        .center_y()
        .focusable(true)
        .role("button")
        .label("Changelog")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            if let Some(details) = st.browse_view.details.as_mut() {
                details.changelog_open = !details.changelog_open;
            }
        })
        .child(
            icon(
                if open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                14.0,
            )
            .color(u.muted)
            .a11y_hidden(),
        );
    head = head.child(section_head(u, Some(&icons::REFRESH_CW), "Changelog", Some(total)));
    let mut c = col().w_full().gap(8.0).child(head);
    if !open {
        return c;
    }
    match &d.updates {
        None => {
            c = c.child(el().w_full().h(44.0).radius(crate::ui::r(8.0)).fill(u.hover));
        }
        Some(Err(_)) => c = c.child(note_box(u, "The changelog could not be loaded.")),
        Some(Ok(updates)) if updates.items.is_empty() => {
            c = c.child(text("No changelog yet.").font(BODY).color(u.muted));
        }
        Some(Ok(updates)) => {
            for update in &updates.items {
                c = c.child(update_card(u, d, update));
            }
            if updates.items.len() < updates.total {
                c = c.child(
                    row().w_full().h(40.0).center().child(
                        u.action(
                            &format!(
                                "Show more changes ({})",
                                updates.total.saturating_sub(updates.items.len())
                            ),
                            ActionKind::Ghost,
                        )
                        .key("details-more-updates")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(load_more_updates),
                    ),
                );
            }
        }
    }
    c
}

pub(crate) fn update_card(u: &Ui, d: &Details, update: &ModUpdate) -> El {
    let has_body = !update.changes.is_empty() || !update.text.is_empty();
    let open = has_body && d.open_updates.contains(&update.id);
    let id = update.id;
    let mut head = row()
        .key(format!("update-{id}"))
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .child(if has_body {
            icon(
                if open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                16.0,
            )
            .color(u.muted)
            .a11y_hidden()
        } else {
            el().size(16.0, 16.0)
        })
        .child(
            text(update_title(update))
                .font(BODY_MED)
                .color(u.ink)
                .truncate()
                .min_w(0.0)
                .shrink(),
        );
    if !update.changes.is_empty() {
        head = head.child(
            row()
                .h(18.0)
                .px(6.0)
                .center()
                .radius(9.0)
                .fill(Color::WHITE.alpha(0.06))
                .child(
                    text(update.changes.len().to_string())
                        .font(SMALL)
                        .color(u.muted),
                ),
        );
    }
    head = head.child(spacer());
    if update.added_ts > 0 {
        head = head.child(
            row()
                .gap(4.0)
                .center_y()
                .child(icon(&CLOCK, 12.0).color(u.faint).a11y_hidden())
                .child(
                    text(gamebanana::calendar_date(update.added_ts))
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
    }
    if has_body {
        head = head
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(update_title(update))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if let Some(details) = st.browse_view.details.as_mut()
                    && !details.open_updates.remove(&id)
                {
                    details.open_updates.insert(id);
                }
            });
    }
    let mut card = col()
        .w_full()
        .radius(crate::ui::r(8.0))
        .fill(Color::WHITE.alpha(0.025))
        .border(1.0, u.hair_strong)
        .clip_rounded(crate::ui::r(8.0))
        .child(head);
    if open {
        let mut body = col()
            .w_full()
            .px(12.0)
            .py(10.0)
            .gap(6.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        if update.changes.is_empty() {
            body = body.child(text(update.text.clone()).font(BODY).color(u.prose).wrap());
        } else {
            for (label, line) in &update.changes {
                let mut r = row().w_full().gap(8.0).items_start();
                if !label.is_empty() {
                    r = r.child(
                        row()
                            .h(18.0)
                            .px(6.0)
                            .mt(1.0)
                            .center()
                            .radius(4.0)
                            .border(1.0, u.hair_strong)
                            .child(text(label.to_uppercase()).font(MICRO_CAPS).color(u.muted)),
                    );
                }
                body = body.child(r.child(
                    text(line.clone())
                        .font(BODY)
                        .color(u.prose)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ));
            }
        }
        card = card.child(body);
    }
    card
}

pub(crate) fn comments_section(s: &State, u: &Ui, d: &Details, profile: &ModProfile) -> El {
    let (total, shown) = match &d.comments {
        Some(Ok(comments)) => (comments.total as u64, comments.items.len() as u64),
        _ => (profile.post_count, 0),
    };
    let mut head = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(section_head(u, Some(&icons::MESSAGE_SQUARE), "Comments", Some(total)))
        .child(spacer());
    if shown > 0 && total > shown {
        head = head.child(
            text(format!("Showing {shown} of {total}"))
                .font(SMALL)
                .color(u.faint),
        );
    }
    let mut c = col().w_full().gap(8.0).child(head);
    match &d.comments {
        None => {
            c = c.child(el().w_full().h(64.0).radius(crate::ui::r(8.0)).fill(u.hover));
        }
        Some(Err(_)) => c = c.child(note_box(u, "Comments could not be loaded.")),
        Some(Ok(comments)) if comments.items.is_empty() => {
            c = c.child(
                row()
                    .w_full()
                    .px(12.0)
                    .py(16.0)
                    .gap(12.0)
                    .center_y()
                    .radius(crate::ui::r(8.0))
                    .fill(Color::WHITE.alpha(0.015))
                    .border(1.0, u.hair_strong)
                    .child(icon(&icons::MESSAGE_SQUARE, 16.0).color(u.faint).a11y_hidden())
                    .child(text("No comments yet").font(BODY).color(u.muted)),
            );
        }
        Some(Ok(comments)) => {
            let mut list = col()
                .w_full()
                .radius(crate::ui::r(8.0))
                .fill(Color::WHITE.alpha(0.015))
                .border(1.0, u.hair_strong);
            let last = comments.items.len().saturating_sub(1);
            for (i, comment) in comments.items.iter().enumerate() {
                let mut item = row()
                    .key(format!("comment-{}", comment.id))
                    .w_full()
                    .px(12.0)
                    .py(12.0)
                    .gap(12.0)
                    .items_start()
                    .child(avatar(s, u, Some(&comment.poster), &comment.poster.name, 32.0))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(
                                row()
                                    .w_full()
                                    .gap(8.0)
                                    .center_y()
                                    .child(
                                        text(comment.poster.name.clone())
                                            .font(BODY_MED)
                                            .color(u.ink)
                                            .truncate()
                                            .min_w(0.0)
                                            .shrink(),
                                    )
                                    .child(icon(&CLOCK, 12.0).color(u.faint).a11y_hidden())
                                    .child(
                                        text(gamebanana::calendar_date(comment.added_ts))
                                            .font(SMALL)
                                            .color(u.faint),
                                    ),
                            )
                            .child(
                                text(comment.text.clone())
                                    .font(BODY)
                                    .color(u.prose)
                                    .wrap()
                                    .selectable(true),
                            ),
                    );
                if i < last {
                    item = item.border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
                }
                list = list.child(item);
            }
            c = c.child(list);
            if comments.items.len() < comments.total {
                c = c.child(
                    row().w_full().h(40.0).center().child(
                        u.action(
                            &format!(
                                "Show more comments ({})",
                                comments.total.saturating_sub(comments.items.len())
                            ),
                            ActionKind::Ghost,
                        )
                        .key("details-more-comments")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(load_more_comments),
                    ),
                );
            }
        }
    }
    c
}

