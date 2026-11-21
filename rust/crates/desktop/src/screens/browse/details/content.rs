//! Files, creator, changelog and comments for the mod details.

use super::*;

pub(crate) fn files_section(s: &State, u: &Ui, d: &Details, profile: &ModProfile) -> El {
    // Wave 4: the slots this mod is installed in, each with Delete (Grimoire).
    let copies = super::super::w4::installed_copies(s, u, d);
    let mut c = col().w_full().gap(8.0);
    let alone = profile.files.len() == 1 && copies.is_none() && profile.archived_files.is_empty();
    if !alone {
        c = c.child(section_head(
            u,
            None,
            "Files",
            (profile.files.len() > 1).then_some(profile.files.len() as u64),
        ));
    }
    if profile.files.is_empty() {
        c = c.child(
            text(super::super::tree::tr("This mod has no downloadable files."))
                .font(BODY)
                .color(u.muted),
        );
    }
    if let Some(copies) = copies {
        c = c.child(copies);
    }
    // With one file the Install block above already says it all.
    let only = profile.files.len() == 1;
    if !only {
        let rows = profile.files.iter().map(|file| file_card(s, u, d, file, only)).collect();
        c = c.child(file_list(rows, u));
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
                .child(text(super::super::tree::tr("Archived files")).font(BODY_MED).color(u.muted))
                .child(spacer())
                .child(
                    text(profile.archived_files.len().to_string())
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
        if open {
            let rows = profile.archived_files.iter().map(|file| file_card(s, u, d, file, false)).collect();
            c = c.child(file_list(rows, u));
        }
    }
    c
}

/// One file row, 52 high: what it is (its description, or its name), then name, size,
/// downloads and date in one quiet line, and its own Install. Rows share one bordered list.
pub(crate) fn file_card(s: &State, u: &Ui, d: &Details, file: &ModFile, _only: bool) -> El {
    let lead = if file.description.trim().is_empty() { file.name.clone() } else { file.description.clone() };
    // Size, downloads and date first, so a long file name is what truncates.
    let mut meta = vec![crate::model::format_size(file.size_bytes)];
    meta.push(crate::screens::settings::i18n::fill(super::super::tree::tr(if file.downloads == 1 { "{} download" } else { "{} downloads" }), &[&group_digits(file.downloads)]));
    if file.added_ts > 0 {
        meta.push(gamebanana::calendar_date(file.added_ts));
    }
    if lead != file.name {
        meta.push(file.name.clone());
    }
    // This file's own state while it installs; the press installs it at once.
    let install = match d.profile.as_ref() {
        Some(profile) => super::super::replace::file_actions(s, u, d, profile, file),
        None => el(),
    };
    row()
        .key(format!("file-{}", file.id))
        .w_full()
        .h(52.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(lead).font(BODY_MED).color(if file.archived { u.muted } else { u.ink }).truncate())
                .child(text(meta.join(" · ")).font(crate::ui::SMALL_NUM).color(u.muted).truncate()),
        )
        .child(install)
}

/// Rows in one bordered list with hairlines between them.
fn file_list(rows: Vec<El>, u: &Ui) -> El {
    let mut list = col()
        .key("details-file-list")
        .w_full()
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .border(1.0, u.hair);
    for (i, row) in rows.into_iter().enumerate() {
        list = list.child(if i == 0 { row } else { row.border_sides([1.0, 0.0, 0.0, 0.0], u.hair) });
    }
    list
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
                .child(donation_row(u, member))
                // Wave 4: the creator's own links (both apps read `_aContactInfo`).
                .children(super::super::w4::author_links(s, u, member.id)),
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
                .radius(crate::ui::r(13.0))
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
                .radius(crate::ui::r(9.0))
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
                            .radius(crate::ui::r(4.0))
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

