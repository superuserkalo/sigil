//! The spotlight over the landing (`design/browse.md` 2.1): one large pick from today's top
//! mods with its cover, title, author, category and likes, Install, the favorite star and a
//! hand-stepped pager, across the full page width. (The quick numbers it once carried repeat
//! the sidebar, status bar and Installed, so they are gone.)
//!
//! Nothing turns by itself: no timer, no rotation, no fade. Every part has a fixed size, so
//! stepping the pager swaps cover and text in place and nothing else moves.

use super::*;
use crate::ui::SMALL_NUM;

/// A top-feed row as a card, so details open with its mature flag, author and link.
pub(crate) fn card_of(t: &TopSub) -> BrowseCard {
    BrowseCard {
        id: t.id,
        name: t.name.clone(),
        author: t.author.clone(),
        category: t.category.clone(),
        updated_at: String::new(),
        likes: t.likes,
        views: 0,
        profile_url: t.profile_url.clone(),
        thumbnail_url: t.image_url.clone(),
        version: None,
        added_ts: 0,
        updated_ts: 0,
        nsfw: t.nsfw,
        model: "Mod".to_string(),
        audio_url: None,
        featured: false,
        comments: 0,
        author_id: 0,
        obsolete: false,
        sub_category_id: 0,
    }
}

/// The spotlight's covers worth fetching, through the same gate as what it shows.
pub(crate) fn cover_urls(s: &State) -> Vec<String> {
    picks(s).into_iter().filter_map(|t| t.image_url.clone()).collect()
}

/// The most picks the pager steps through.
const PICKS: usize = 5;

/// The layout the page width allows.
#[derive(Clone, Copy, PartialEq)]
struct Size {
    cover_w: f32,
    cover_h: f32,
    big_title: bool,
}

fn size(width: f32) -> Size {
    if width >= 960.0 {
        Size { cover_w: 384.0, cover_h: 216.0, big_title: true }
    } else if width >= 720.0 {
        Size { cover_w: 320.0, cover_h: 180.0, big_title: false }
    } else {
        Size { cover_w: 240.0, cover_h: 135.0, big_title: false }
    }
}

/// Today's top mods (this week's when today has none), the mature rule applied.
pub(crate) fn picks(s: &State) -> Vec<&TopSub> {
    let Some(top) = s.browse_view.top.as_ref() else {
        return Vec::new();
    };
    let of = |period: &str| -> Vec<&TopSub> {
        top.iter()
            .filter(|t| t.period == period && t.model.eq_ignore_ascii_case("mod"))
            .filter(|t| content_allowed(s, t.id, t.nsfw, &t.author))
            .take(PICKS)
            .collect()
    };
    let today = of("today");
    if today.is_empty() { of("week") } else { today }
}

/// The facts block's height: a label line over a value line.
const FACTS_H: f32 = 40.0;

/// Author, category and likes as labelled columns, so the text column reads top to bottom:
/// what it is, who made it, then what to do.
fn facts(u: &Ui, p: &TopSub) -> El {
    let cell = |key: &'static str, label: &str, value: String, num: bool| {
        col()
            .key(key)
            .min_w(0.0)
            .gap(4.0)
            .child(text(super::tree::tr(label)).font(SMALL).color(u.muted))
            .child(
                text(value)
                    .font(if num { crate::ui::BODY_NUM } else { BODY })
                    .color(u.prose)
                    .truncate()
                    .min_w(0.0),
            )
    };
    let or_dash = |v: &str| if v.is_empty() { "-".to_string() } else { v.to_string() };
    el().key("browse-spot-facts")
        .w_full()
        .h(FACTS_H)
        .equal_columns(3)
        .gap(16.0)
        .child(cell("browse-spot-author", "Author", or_dash(&p.author), false))
        .child(cell("browse-spot-category", "Category", or_dash(&p.category), false))
        .child(cell("browse-spot-likes", "Likes", group_digits(p.likes), true).label(format!("{} likes", group_digits(p.likes))))
}

fn pager_button(u: &Ui, key: &'static str, forward: bool, enabled: bool, count: usize) -> El {
    let b = el()
        .key(key)
        .size(24.0, 24.0)
        .center()
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair)
        .role("button")
        .label(if forward { "Next pick" } else { "Previous pick" })
        .child(icon(if forward { &icons::CHEVRON_RIGHT } else { &icons::CHEVRON_LEFT }, 14.0).color(u.muted).a11y_hidden());
    if !enabled {
        return b.disabled(true).opacity(0.4);
    }
    b.hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let spot = &mut st.browse_view.shelves.spot;
            *spot = if forward { (*spot + 1).min(count.saturating_sub(1)) } else { spot.saturating_sub(1) };
            st.browse_view.shelves.spot_dir = if forward { 1 } else { -1 };
        })
}

/// The spotlight card and its stats column, at a fixed height from the first frame. Before the
/// top feed lands the cover and text are flat placeholders of the same size.
pub fn view(s: &State, u: &Ui) -> El {
    let width = super::toolbar::page_width(s);
    let sz = size(width);
    let list = picks(s);
    let count = list.len();
    let at = s.browse_view.shelves.spot.min(count.saturating_sub(1));
    let pick = list.get(at).copied();
    let radius = crate::ui::r(8.0);

    // The cover.
    let mut cover = el()
        .key("browse-spot-cover")
        .size(sz.cover_w, sz.cover_h)
        .min_w(sz.cover_w)
        .fill(u.hover)
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .clip_rounded(0.0);
    if let Some(p) = pick {
        let veil = veiled(s, p.id, p.nsfw);
        // The picture pages as a shelf does: the next pick comes in 32 points from the side
        // it pages from while fading in, the last one fades out in place.
        let mut picture = el()
            .key(format!("browse-spot-page-{}", p.id))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .paint_order(1)
            .transition(crate::motion::paged(s.browse_view.shelves.spot_dir))
            .child(image_fill(s, format!("spot-{}", p.id), p.image_url.as_ref(), [0.0; 4], veil));
        if veil {
            picture = picture.child(nsfw_veil(u, "spot", p.id, true));
        }
        cover = cover.child(picture);
        let opened = card_of(p);
        cover = cover
            .focusable(true)
            .role("button")
            .label(format!("Open {}", p.name))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| show_details(st, Details::from_card(&opened)));
    }

    // The text and actions.
    let title_font = if sz.big_title { crate::ui::PAGE_TITLE } else { H1 };
    let title_h = if sz.big_title { 48.0 } else { 44.0 };
    let mut info = col()
        .key("browse-spot-info")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(if sz.big_title { 24.0 } else { 16.0 })
        .gap(4.0);
    match pick {
        Some(p) => {
            let eyebrow = super::tree::tr(if p.period == "today" { "Trending today" } else { "Trending this week" });
            info = info
                .child(text(eyebrow).font(LABEL).color(u.muted))
                .child(
                    el().w_full()
                        .h(title_h)
                        .child(text(p.name.clone()).font(title_font).color(u.ink).wrap().max_lines(2).min_w(0.0)),
                )
                .child(spacer())
                .child(facts(u, p));
        }
        None => {
            info = info
                .child(el().w(96.0).h(12.0).radius(crate::ui::r(4.0)).fill(u.hover))
                .child(el().h(title_h).pt(8.0).child(el().w(220.0).h(16.0).radius(crate::ui::r(4.0)).fill(u.hover)))
                .child(spacer())
                .child(el().w(240.0).h(FACTS_H).pt(4.0).child(el().w(240.0).h(12.0).radius(crate::ui::r(4.0)).fill(u.hover)));
        }
    }
    let mut actions = row().key("browse-spot-actions").w_full().h(32.0).gap(8.0).center_y();
    if let Some(p) = pick {
        let id = p.id;
        let state = super::quick::button(s, id);
        actions = actions.child(
            row()
                .w(120.0)
                .h(32.0)
                .child(super::quick::button_el(u, &state, format!("spot-install-{id}"), id, &p.name, super::quick::Size::Row).w(120.0).min_w(0.0)),
        );
        let saved = is_favorite(s, "Mod", id);
        let favorite = Favorite {
            id,
            model: "Mod".into(),
            name: p.name.clone(),
            author: p.author.clone(),
            category: p.category.clone(),
            profile_url: p.profile_url.clone(),
            thumbnail_url: p.image_url.clone(),
            likes: p.likes,
            nsfw: p.nsfw,
            ..Favorite::default()
        };
        actions = actions.child(
            el().key("browse-spot-star")
                .size(32.0, 32.0)
                .center()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.hair_strong)
                .hover(u.hover)
                .press(u.over.alpha(0.08))
                .focusable(true)
                .role("button")
                .label(if saved { "Remove from favorites" } else { "Save to favorites" })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()))
                .child(icon(if saved { &STAR_FILLED } else { &STAR }, 16.0).color(if saved { u.ink } else { u.muted }).a11y_hidden()),
        );
    }
    actions = actions.child(spacer());
    if count > 1 {
        actions = actions
            .child(pager_button(u, "browse-spot-prev", false, at > 0, count))
            .child(
                text(format!("{}/{}", at + 1, count))
                    .font(SMALL_NUM)
                    .color(u.muted)
                    .w(28.0)
                    .align(rune::Align::Center),
            )
            .child(pager_button(u, "browse-spot-next", true, at + 1 < count, count));
    }
    info = info.child(el().h(12.0)).child(actions);

    let card = row()
        .key("browse-spotlight")
        .w_full()
        .h(sz.cover_h)
        .radius(radius)
        .clip_rounded(radius)
        .fill(u.well)
        .border(1.0, u.hair)
        .child(cover)
        .child(info);
    card
}
