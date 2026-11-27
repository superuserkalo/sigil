//! The mod details view: header, page and the reader-facing state blocks.

use super::*;
pub(crate) use self::content::*;
pub(crate) use self::media::*;

pub mod content;
pub mod media;

// --- mod details (port of Grimoire's ModDetailsModal) ---------------------------------

/// The dialogs this screen mounts at the root: the mod details (unless docked), the image
/// lightbox, a collection or management list, the hero picker, and the install dialog over
/// all of them.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let v = &s.browse_view;
    let mut layers: Vec<El> = Vec::new();
    if let Some(details) = &v.details {
        if !v.docked {
            layers.push(
                el().w_full()
                    .h_full()
                    .center()
                    .child(details_modal(s, u, details)),
            );
        }
        if let Some(index) = details.lightbox {
            layers.push(lightbox(s, u, details, index));
        }
    }
    for (key, extra) in [
        ("browse-collection-overlay", super::collections::collection_dialog(s, u)),
        ("browse-hidden-overlay", super::panels::hidden_dialog(s, u)),
        ("browse-ignored-overlay", super::panels::ignored_dialog(s, u)),
        ("browse-hero-picker-overlay", super::panels::hero_picker_dialog(s, u)),
        ("browse-paste-overlay", super::paste::dialog(s, u)),
        ("browse-source-overlay", super::sources::details_dialog(s, u)),
    ] {
        if let Some(extra) = extra {
            layers.push(
                full_layer(key, Color::BLACK.alpha(0.5))
                    .center()
                    .child(extra),
            );
        }
    }
    // The details' file menu, over the modal it drops from.
    if !v.docked
        && let Some(menu) = super::quick::files_menu(s, u)
    {
        layers.push(menu);
    }
    // Wave 4: the image menu over a preview, the lightbox or a docked carousel.
    if let Some(menu) = super::w4::image_menu(s, u) {
        layers.push(menu);
    }
    if s.install.open {
        let install = install_dialog(s, u);
        layers.push(if layers.is_empty() {
            install
        } else {
            full_layer("install-over-details", Color::BLACK.alpha(0.5)).center().child(install)
        });
    }
    // The lightbox and the install overlay are pinned 32 points past this box, the root
    // modal layer's padding, so they need it as their parent. A lone install dialog is not.
    match layers.len() {
        0 => None,
        1 if s.install.open => layers.pop(),
        _ => Some(el().w_full().h_full().children(layers)),
    }
}

/// A layer over the whole window. The root modal layer pads its child by 32 points; this
/// reaches past that padding.
pub(crate) fn full_layer(key: &str, fill: Color) -> El {
    col()
        .key(key)
        .pin(Some(-32.0), Some(-32.0), Some(-32.0), Some(-32.0))
        .fill(fill)
}

pub(crate) fn details_modal(s: &State, u: &Ui, d: &Details) -> El {
    let close = |st: &mut State| close_details(st);
    u.dialog_shell()
        .key("browse-details")
        .role("dialog")
        .label(d.name.clone())
        .w_full()
        .max_w(DETAILS_MAX_W)
        .h_full()
        .max_h(DETAILS_MAX_H)
        .radius(crate::ui::r(8.0))
        .clip_rounded(crate::ui::r(8.0))
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| match ev.key {
            Key::Escape => {
                close(st);
                true
            }
            Key::Left => {
                step_details(st, false);
                true
            }
            Key::Right => {
                step_details(st, true);
                true
            }
            _ => false,
        })
        .child(details_header(s, u, d, false))
        .child(
            u.scroll_area(col()
                .key("browse-details-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    row()
                        .key("browse-details-body")
                        .w_full()
                        .padding(24.0)
                        .gap(24.0)
                        .items_start()
                        // The reading column: the gallery, then About, videos, changes, comments.
                        .child(
                            col()
                                .key("browse-details-reading")
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(24.0)
                                .child(carousel(s, u, d))
                                .child(details_reading(s, u, d)),
                        )
                        // The action column: who, counts, Install, files, needs, creator.
                        .child(
                            col()
                                .key("browse-details-info")
                                .w(ACTION_W)
                                .min_w(ACTION_W)
                                .child(details_actions(s, u, d)),
                        ),
                )),
        )
}

/// The details' action column width.
const ACTION_W: f32 = 360.0;

/// The docked form: one column beside the grid, a carousel instead of the image stack.
pub(crate) fn details_sidebar(s: &State, u: &Ui, d: &Details) -> El {
    col()
        .key("browse-details-dock")
        .w(super::w4::dock_width(s, SIDEBAR_W))
        .h_full()
        .fill(u.sidebar)
        .border_sides([0.0, 0.0, 0.0, 1.0], u.hair)
        .on_key(|st: &mut State, ev: &KeyEvent, _| match ev.key {
            Key::Escape => {
                close_details(st);
                true
            }
            Key::Left => {
                step_details(st, false);
                true
            }
            Key::Right => {
                step_details(st, true);
                true
            }
            _ => false,
        })
        .child(details_header(s, u, d, true))
        // Wave 4: drag the left edge to resize (Grimoire), kept across launches.
        .child(super::w4::dock_handle(s, u, SIDEBAR_W))
        .child(
            u.scroll_area(col()
                .key("browse-details-dock-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .w_full()
                        .padding(16.0)
                        .gap(20.0)
                        .child(carousel(s, u, d))
                        .child(details_actions(s, u, d))
                        .child(details_reading(s, u, d)),
                )),
        )
}

/// One 28 point ghost icon button of the details header.
fn head_button(u: &Ui, key: &'static str, glyph: &'static Icon, label: &str) -> El {
    el().key(key)
        .size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .help(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(icon(glyph, 16.0).color(u.muted).a11y_hidden())
}

/// The 48 point header: the title alone, then five actions in one style, then Close. Dates and
/// counts live in the action column.
pub(crate) fn details_header(s: &State, u: &Ui, d: &Details, docked: bool) -> El {
    let id = d.id;
    let saved = is_favorite(s, d.section.model(), id);
    let favorite = d.favorite();
    let link = d.profile_url.clone();
    let url = d.profile_url.clone();
    row()
        .key("details-head")
        .w_full()
        .h(48.0)
        .px(if docked { 16.0 } else { 24.0 })
        .gap(4.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            text(d.name.clone())
                .font(H1)
                .color(u.ink)
                .truncate()
                .min_w(0.0)
                .shrink()
                .grow(1.0)
                .mr(8.0),
        )
        .child(
            head_button(
                u,
                "details-favorite",
                if saved { &STAR_FILLED } else { &STAR },
                super::tree::tr(if saved { "Remove from favorites" } else { "Save to favorites" }),
            )
            .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone())),
        )
        .child(head_button(u, "details-copy", &icons::COPY, super::tree::tr("Copy link")).on_press_tx(move |st: &mut State, tx: &mut Tx| {
            tx.copy(link.clone());
            st.status = super::tree::tr("Link copied.").to_string();
        }))
        .child(
            head_button(u, "details-open", &icons::EXTERNAL_LINK, super::tree::tr("Open on GameBanana"))
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url)),
        )
        .child(head_button(u, "details-hide", &EYE_OFF, super::tree::tr("Hide this mod")).on_press(move |st: &mut State| hide_mod(st, id)))
        .child(
            head_button(
                u,
                "details-dock",
                if docked { &MAXIMIZE_2 } else { &PANEL_RIGHT },
                super::tree::tr(if docked { "Pop out" } else { "Dock to the side" }),
            )
            .on_press(|st: &mut State| st.browse_view.docked = !st.browse_view.docked),
        )
        .child(el().w(8.0))
        .child(head_button(u, "details-close", &icons::X, super::tree::tr("Close")).on_press(close_details))
}

/// `25901` as `25,901`.
pub(crate) fn section_head(u: &Ui, glyph: Option<&'static Icon>, label: &str, count: Option<u64>) -> El {
    let _ = glyph;
    u.section_head(super::tree::tr(label), count.filter(|count| *count > 0).map(|count| count as usize), None)
}

pub(crate) fn tag(u: &Ui, label: &str) -> El {
    row()
        .h(22.0)
        .px(8.0)
        .center()
        .radius(crate::ui::r(6.0))
        .fill(Color::WHITE.alpha(0.06))
        .border(1.0, u.hair_strong)
        .child(text(label.to_string()).font(SMALL).color(u.prose))
}

pub(crate) fn avatar(s: &State, u: &Ui, member: Option<&ModMember>, name: &str, size: f32) -> El {
    let base = el().size(size, size).radius(size / 2.0);
    if let Some(pixels) = member
        .and_then(avatar_url)
        .and_then(|url| s.thumbnails.get(&url))
    {
        return base
            .pixels_fit(pixels, SurfaceFit::Fill)
            .border(1.0, u.hair_strong)
            .a11y_hidden();
    }
    let initial = name
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string());
    base.center()
        .fill(u.accent.alpha(0.15))
        .border(1.0, u.accent.alpha(0.30))
        .child(
            text(initial)
                .font(if size >= 40.0 { H2 } else { LABEL })
                .color(u.accent)
                .a11y_hidden(),
        )
}

/// The right column: category, files, artist, about, changelog, comments, and the link out.
/// The action column: identity, counts, the last update, notices, Install and its state, the
/// hero, the sample, the files, requirements, the map tools and the creator. Before the profile
/// lands, a skeleton holds the same places.
pub(crate) fn details_actions(s: &State, u: &Ui, d: &Details) -> El {
    let mut c = col().key("details-actions").w_full().gap(16.0);
    let Some(profile) = d.profile.as_ref() else {
        if let Some(error) = &d.error {
            return c.child(details_error(u, error));
        }
        return c.child(info_skeleton(u));
    };

    // Who and what: the hero portrait when GameBanana has one, the category, the 18+ tag.
    let mut identity = row().key("details-identity").w_full().h(24.0).gap(8.0).center_y();
    if let Some(pixels) = profile.category_icon.as_ref().and_then(|url| s.thumbnails.get(url)) {
        identity = identity.child(
            el().size(24.0, 24.0)
                .radius(crate::ui::r(4.0))
                .clip_rounded(crate::ui::r(4.0))
                .pixels_fit(pixels, SurfaceFit::Fill)
                .label(profile.category.clone()),
        );
    }
    if !profile.category.is_empty() {
        identity = identity.child(text(profile.category.clone()).font(BODY).color(u.prose).truncate().min_w(0.0).shrink());
    }
    if !profile.super_category.is_empty() {
        identity = identity.child(tag(u, &profile.super_category));
    }
    if d.nsfw {
        identity = identity.child(nsfw_tag(u));
    }
    c = c.child(
        col()
            .w_full()
            .gap(6.0)
            .child(identity)
            // The same counts, icons and order as a card.
            .child(
                row()
                    .key("details-counts")
                    .h(16.0)
                    .gap(16.0)
                    .center_y()
                    .child(details_count(u, &icons::THUMBS_UP, "Likes", profile.likes))
                    .child(details_count(u, &icons::EYE, "Views", profile.views))
                    .child(details_count(u, &icons::DOWNLOAD, "Downloads", profile.downloads)),
            )
            .child({
                let updated = if profile.updated_ts > 0 { profile.updated_ts } else { d.updated_ts };
                text(if updated > 0 {
                    crate::screens::settings::i18n::fill(super::tree::tr("Updated {}"), &[&crate::prefs::show_local_date(updated)])
                } else {
                    String::new()
                })
                .font(SMALL)
                .color(u.muted)
                .h(16.0)
            }),
    );

    for notice in notices(u, d, profile) {
        c = c.child(notice);
    }

    // Install at once: the newest file. The Files list below picks any other.
    if let Some(install) = super::quick::details_install(s, u, d, profile) {
        c = c.child(install);
    }

    if let Some(status) = status_row(s, u, d) {
        c = c.child(status);
    }

    if d.hero.is_some() || d.hero_override.is_some() {
        c = c.child(hero_row(s, u, d));
    }

    if let Some(url) = profile.audio_url.clone() {
        let playing = s.browse_view.audio.playing == Some(d.id);
        let loading = s.browse_view.audio.loading == Some(d.id);
        let id = d.id;
        c = c.child(
            u.icon_action(
                if playing { &icons::SQUARE_STOP } else if loading { &icons::LOADER } else { &icons::PLAY },
                super::tree::tr(if playing || loading { "Stop the sample" } else { "Play the sample" }),
                ActionKind::Secondary,
            )
            .key("details-sample")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.browse_view.audio.toggle(id, url.clone());
            }),
        );
    }

    c = c.child(files_section(s, u, d, profile));
    c = c.child(requirements_section(s, u, d));
    // Custom maps (experimental): a map's name, Launch map and Invite friends (DMM).
    if let Some(play) = crate::screens::maps::details_section(
        s,
        u,
        d.id,
        &profile.super_category,
        &profile.category,
        &d.profile_url,
        &profile.text,
    ) {
        c = c.child(play);
    }
    if let Some(member) = &profile.submitter {
        c = c.child(artist_card(s, u, member, &d.author));
    }
    c
}

/// One count in the details: icon and tabular number, named for assistive tech.
fn details_count(u: &Ui, glyph: &'static Icon, name: &str, value: u64) -> El {
    row()
        .gap(4.0)
        .center_y()
        .label(format!("{name}: {}", group_digits(value)))
        .child(icon(glyph, 12.0).color(u.faint).a11y_hidden())
        .child(text(group_digits(value)).font(crate::ui::SMALL_NUM).color(u.muted).a11y_hidden())
}

/// The reading column: About (with the date it was added), videos, changes and comments.
pub(crate) fn details_reading(s: &State, u: &Ui, d: &Details) -> El {
    let mut c = col().key("details-reading").w_full().gap(24.0);
    let Some(profile) = d.profile.as_ref() else {
        return c;
    };
    let added = if profile.added_ts > 0 { profile.added_ts } else { d.added_ts };
    if !profile.text.is_empty() || added > 0 {
        let mut about = col().w_full().gap(8.0).child(section_head(u, None, "About", None));
        if !profile.text.is_empty() {
            about = about.child(
                text(profile.text.clone())
                    .font(BODY)
                    .color(u.prose)
                    .wrap()
                    .max_w(READING_MAX_W)
                    .selectable(true),
            );
        }
        if added > 0 {
            about = about.child(
                text(crate::screens::settings::i18n::fill(super::tree::tr("Added {}"), &[&crate::prefs::show_local_date(added)]))
                    .font(SMALL)
                    .color(u.muted),
            );
        }
        c = c.child(about);
    }
    c.child(videos_section(u, d, profile))
        .child(changelog_section(u, d, profile))
        .child(comments_section(s, u, d, profile))
}

/// About's line length: about 72 characters of 13 point text.
const READING_MAX_W: f32 = 560.0;

/// GameBanana's own warnings about a submission, before its files.
pub(crate) fn notices(u: &Ui, _d: &Details, profile: &ModProfile) -> Vec<El> {
    use crate::screens::settings::i18n::{fill, tr};
    let mut list = Vec::new();
    if profile.withheld {
        // DMM `cba597f2`: the staff's own reason, the rules and who withheld it when.
        let notice = profile.withhold.clone().unwrap_or_default();
        let mut lines = Vec::new();
        for (code, name) in &notice.rules {
            lines.push(match (code.is_empty(), name.is_empty()) {
                (false, false) => format!("{code}: {name}"),
                (false, true) => code.clone(),
                _ => name.clone(),
            });
        }
        if !notice.notes.is_empty() {
            lines.push(notice.notes.clone());
        }
        let by = match (&notice.by, notice.at) {
            (Some(by), Some(at)) => Some(fill(tr("Withheld by {} on {}."), &[by, &crate::prefs::show_unix_date(at)])),
            (Some(by), None) => Some(fill(tr("Withheld by {}."), &[by])),
            (None, Some(at)) => Some(fill(tr("Withheld on {}."), &[&crate::prefs::show_unix_date(at)])),
            (None, None) => None,
        };
        lines.extend(by);
        list.push(notice_box(
            u,
            "details-withheld",
            tr("GameBanana staff withheld this mod. Its files may not download until the creator fixes it."),
            &lines,
        ));
    }
    if profile.private {
        list.push(note_box(u, tr("Only the creator can see this mod right now.")));
    }
    if profile.obsolete {
        // DMM `7ede98c1`: the author's note, when they wrote one.
        let lines: Vec<String> = profile.obsolete_note.iter().cloned().collect();
        list.push(notice_box(
            u,
            "details-obsolete",
            tr("The creator marked this mod outdated. A newer mod may replace it."),
            &lines,
        ));
    }
    list
}

/// A notice with a heading line and the words GameBanana gave for it.
fn notice_box(u: &Ui, key: &str, heading: &str, lines: &[String]) -> El {
    let mut c = col()
        .key(key.to_string())
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(4.0)
        .radius(crate::ui::r(8.0))
        .fill(Color::WHITE.alpha(0.025))
        .border(1.0, u.hair_strong)
        .child(text(heading.to_string()).font(BODY_MED).color(u.ink).wrap().w_full());
    for line in lines {
        c = c.child(text(line.clone()).font(BODY).color(u.muted).wrap().w_full());
    }
    c
}

/// Installed, update and outdated state as tags, with the action each one needs.
pub(crate) fn status_row(s: &State, u: &Ui, d: &Details) -> Option<El> {
    let slot = installed_slot(s, d.id);
    let update = available_update(s, d.id);
    let ignored = update_ignored(s, d.id);
    let stale = d
        .profile
        .as_ref()
        .map(|profile| profile.updated_ts.max(d.updated_ts))
        .unwrap_or(d.updated_ts);
    let old = d.profile.as_ref().map(|p| p.obsolete).unwrap_or(false)
        || (stale > 0 && stale < OUTDATED_BEFORE);
    if slot.is_none() && update.is_none() && !ignored && !old {
        return None;
    }
    let mut r = row().w_full().gap(8.0).center_y().flex_wrap();
    if let Some(slot) = slot {
        r = r.child(tag(u, &format!("Installed in slot {slot:02}")));
        // The hero it changes, in 3D, as the player has it (on or off).
        if !crate::viewer::dialog::heroes_of(s, slot).is_empty() {
            let name = d.name.clone();
            r = r.child(
                u.icon_action(&icons::BOX, crate::screens::settings::i18n::tr(crate::viewer::dialog::LABEL), ActionKind::Secondary)
                    .key("details-preview-3d")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| crate::viewer::dialog::open(st, slot, &name)),
            );
        }
    }
    if let Some(file) = &update {
        let id = d.id;
        let name = d.name.clone();
        r = r.child(tag(u, &format!("Update: {}", file.file_name)));
        // The Install button above turns into Update; this tag says which file is new.
        let _ = (id, name);
        if ignored {
            r = r.child(tag(u, "Updates ignored"));
        } else {
            let id = d.id;
            r = r.child(
                u.action("Ignore update", ActionKind::Ghost)
                    .key("details-ignore-update")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| set_update_ignored(st, id, true)),
            );
        }
    } else if ignored {
        r = r.child(tag(u, "Updates ignored"));
        let id = d.id;
        r = r.child(
            u.action("Check again", ActionKind::Ghost)
                .key("details-check-update")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| set_update_ignored(st, id, false)),
        );
    }
    if old {
        r = r.child(tag(u, "Outdated"));
    }
    Some(r)
}

/// The hero the page mentions, and a way to change it.
pub(crate) fn hero_row(_s: &State, u: &Ui, d: &Details) -> El {
    let hero = d
        .hero_override
        .clone()
        .or_else(|| d.hero.clone())
        .unwrap_or_default();
    let mut r = row().key("details-hero").w_full().gap(8.0).center_y();
    r = r.child(
        row()
            .gap(6.0)
            .center_y()
            .child(icon(&icons::USER, 14.0).color(u.faint).a11y_hidden())
            .child(text("Hero").font(SMALL).color(u.faint)),
    );
    r = r.child(tag(u, &hero));
    r = r.child(spacer());
    let hero_for_browse = hero.clone();
    let section = d.section;
    r = r.child(
        u.action("Browse this hero", ActionKind::Ghost)
            .key("details-hero-browse")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                if section == BrowseSection::Mods {
                    super::grid::open_skins_for(st, tx, &hero_for_browse);
                } else {
                    browse_hero_sounds(st, tx, hero_for_browse.clone());
                }
            }),
    );
    r.child(
        u.action("Change", ActionKind::Ghost)
            .key("details-hero-change")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.browse_view.hero_picker = Some(HeroTarget::Details);
                st.browse_view.hero_query = TextEdit::new("");
            }),
    )
}

/// Required and recommended mods, when the page lists any.
pub(crate) fn requirements_section(s: &State, u: &Ui, d: &Details) -> El {
    let mut c = col().w_full().gap(8.0);
    let Some(result) = &d.requirements else {
        return c;
    };
    let list = match result {
        Ok(list) if list.is_empty() => return c,
        Ok(list) => list,
        Err(_) => {
            return c.child(note_box(u, "The requirements could not be loaded."));
        }
    };
    c = c.child(section_head(u, Some(&icons::BOX), "Requirements", None));
    for need in list {
        let id = need.mod_id;
        let url = need.url.clone();
        let owned = id.is_some_and(|id| is_installed(s, id));        let mut row = row()
            .key(format!("requirement-{}", need.name))
            .w_full()
            .min_h(44.0)
            .px(12.0)
            .py(8.0)
            .gap(12.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .border(1.0, u.hair_strong);
        row = row.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(need.name.clone()).font(BODY).color(u.ink).truncate())
                .child(
                    text(if need.url.is_empty() {
                        "Required".to_string()
                    } else {
                        need.url.clone()
                    })
                    .font(SMALL)
                    .color(u.muted)
                    .truncate(),
                ),
        );
        if owned {
            row = row.child(tag(u, "Installed"));
        } else if let Some(id) = id {
            let install_name = need.name.clone();
            row = row.child(
                u.icon_action(&icons::DOWNLOAD, "Install", ActionKind::Secondary)
                    .key(format!("requirement-install-{id}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        super::quick::press(st, id, &install_name);
                    }),
            );
        }
        if !url.is_empty() {
            row = row.child(
                u.icon_button(&icons::EXTERNAL_LINK, "Open the requirement")
                    .key(format!("requirement-open-{}", need.name))
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url)),
            );
        }
        c = c.child(row);
    }
    c
}

/// Videos the page embeds. Rune has no video surface, so each opens in the browser.
pub(crate) fn videos_section(u: &Ui, _d: &Details, profile: &ModProfile) -> El {
    let mut c = col().w_full().gap(8.0);
    if profile.videos.is_empty() {
        return c;
    }
    c = c.child(section_head(
        u,
        Some(&icons::PLAY),
        "Videos",
        Some(profile.videos.len() as u64),
    ));
    for (index, video) in profile.videos.iter().enumerate() {
        let url = video.clone();
        let site = gamebanana::video_site(video);
        c = c.child(
            row()
                .key(format!("video-{index}"))
                .w_full()
                .h(48.0)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.hair_strong)
                .hover(u.hover)
                .focusable(true)
                .role("link")
                .label(format!("Watch the {site} video"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url))
                .child(
                    el().size(32.0, 32.0)
                        .center()
                        .radius(16.0)
                        .fill(u.over.alpha(0.06))
                        .child(icon(&icons::PLAY, 14.0).color(u.ink).a11y_hidden()),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(format!("{site} video")).font(BODY).color(u.ink))
                        .child(text(video.clone()).font(SMALL).color(u.muted).truncate()),
                )
                .child(icon(&icons::EXTERNAL_LINK, 14.0).color(u.faint).a11y_hidden()),
        );
    }
    c
}

pub(crate) fn details_error(u: &Ui, error: &str) -> El {    col()
        .w_full()
        .gap(12.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                .child(text("This mod page did not load.").font(BODY_MED).color(u.ink)),
        )
        .child(
            text(error.to_string())
                .font(SMALL)
                .color(u.muted)
                .wrap_anywhere()
                .selectable(true),
        )
        .child(
            u.icon_action(&icons::REFRESH_CW, "Try again", ActionKind::Secondary)
                .key("details-retry")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if let Some(details) = st.browse_view.details.as_mut() {
                        details.error = None;
                        details.loading = true;
                    }
                    start_details_fetch(st);
                }),
        )
}

pub(crate) fn info_skeleton(u: &Ui) -> El {
    let block = |w: f32| skeleton(u, w, 10.0);
    col()
        .w_full()
        .gap(20.0)
        .child(row().gap(8.0).child(skeleton(u, 22.0, 22.0)).child(skeleton(u, 56.0, 22.0)))
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(block(48.0))
                .child(el().w_full().h(66.0).radius(crate::ui::r(8.0)).fill(u.hover)),
        )
        .child(el().w_full().h(70.0).radius(crate::ui::r(12.0)).fill(u.hover))
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(block(56.0))
                .child(block(360.0))
                .child(block(320.0))
                .child(block(240.0)),
        )
}

