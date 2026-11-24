//! The page: head, filter bar, body and the numbered pages.

use super::*;

// --- page ----------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let grid = grid_page(s, u);
    match (&s.browse_view.details, s.browse_view.docked) {
        (Some(details), true) => row()
            .key("browse-split")
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(col().grow(1.0).min_w(0.0).h_full().child(grid))
            .child(details_sidebar(s, u, details)),
        _ => grid,
    }
}

pub(crate) fn grid_page(s: &State, u: &Ui) -> El {
    // A custom mod source picked in the header replaces the GameBanana grid.
    if let Some(url) = s.browse_view.sources.url.as_deref() {
        return super::sources::page(s, u, url);
    }
    let shown = Rc::new(shown_indices(s));
    let f = &s.browse_view.filters;
    let current = query_key(s, &current_query(s));
    let stale = s
        .browse
        .page
        .as_ref()
        .is_some_and(|page| page.key != current);
    let collecting = s.browse_view.tab == Tab::Collections;

    let width = super::toolbar::page_width(s);
    let (cols, card_w) = grid_columns(width, s.browse_view.card_size);
    // Card thumbnails are fetched at this width (`data::shrink_card_thumb`).
    crate::data::set_card_points(card_w);

    let body = if collecting {
        collections_page(s, u)
    } else if s.browse.page.is_none() && s.browse.loading {
        loading_grid(u, cols)
    } else if s.browse.page.is_none() {
        u.empty_state(
            "",
            if s.browse.error.as_deref().is_some_and(super::w4::is_rate_limit) {
                crate::screens::settings::i18n::tr("GameBanana is rate-limiting Sigil")
            } else if s.browse.error.is_some() {
                super::tree::tr("GameBanana did not respond")
            } else {
                super::tree::tr("Nothing loaded yet")
            },
            if s.browse.error.as_deref().is_some_and(super::w4::is_rate_limit) {
                crate::screens::settings::i18n::tr("Wait a minute, then try again.")
            } else {
                super::tree::tr("Check your connection, then try again.")
            },
            Some(
                u.icon_action(&icons::REFRESH_CW, super::tree::tr("Try again"), ActionKind::Secondary)
                    .key("browse-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(refresh),
            ),
        )
    } else if shown.is_empty() && stale {
        loading_grid(u, cols)
    } else if shown.is_empty() {
        let mut action = row().gap(8.0);
        let filtered = f.active() > 0 || !s.browse_query.text.trim().is_empty();
        let only_favorites = f.favorites && f.active() == 1 && s.browse_query.text.trim().is_empty();
        if only_favorites {
            action = action.child(
                u.action(super::tree::tr("Show everything"), ActionKind::Secondary)
                    .key("browse-all")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), |f| f.favorites = false)
                    }),
            );
        } else if filtered {
            action = action.child(
                u.action(super::tree::tr("Clear filters"), ActionKind::Secondary)
                    .key("browse-clear")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(clear_filters),
            );
        }
        if more_to_load(s) {
            action = action.child(
                u.action(super::tree::tr("Load more"), ActionKind::Secondary)
                    .key("browse-more-empty")
                    .disabled(s.browse.loading)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(load_more_pressed),
            );
        }
        let (title, detail) = if f.favorites && only_favorites {
            ("No favorites here yet", "Save a mod with the star on its card.")
        } else if f.owned == Owned::Installed && !f.favorites {
            ("No installed GameBanana mods", "Mods you install from Browse show up here.")
        } else if filtered {
            ("Nothing matches", "Try fewer filters or another search.")
        } else {
            ("Nothing to show", "")
        };
        u.empty_state("", super::tree::tr(title), super::tree::tr(detail), Some(action))
    } else {
        let mut c = col().w_full().gap(16.0).pb(16.0);
        if s.browse.error.is_some() && s.browse_view.refreshing {
            c = c.child(page_error(u));
        }
        if let Some(rail) = dashboard(s, u) {
            c = c.child(rail);
        }
        if let Some(rail) = creator_rail(s, u) {
            c = c.child(rail);
        }
        // DMM's author page: the creator's profile and totals over their grid.
        if let Some(page) = super::creator_page::header(s, u) {
            c = c.child(page);
        }
        if s.browse_view.paged
            && let Some(bar) = pagination(s, u)
        {
            c = c.child(bar);
        }
        let list = if s.browse_view.layout == Layout::List {
            list_view(s, u, shown.clone())
        } else {
            virtual_grid(s, shown.clone(), cols, card_w)
        };
        c.child(if stale { list.opacity(0.45) } else { list })
    };

    let detail = if collecting {
        s.browse_view
            .collections
            .as_ref()
            .map(|load| format!("{} collections", group_digits(load.total as u64)))
            .unwrap_or_default()
    } else {
        result_line(s, shown.len(), stale)
    };

    col()
        .key("browse-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_layout_tx(|st: &mut State, r: [f32; 4], tx: &mut Tx| {
            st.browse_view.page_rect = r;
            let width = (r[2] - 2.0 * PAGE_X).max(0.0);
            if (st.browse_view.width - width).abs() > 0.5 {
                st.browse_view.width = width;
            } else {
                tx.live_only();
            }
        })
        .child(
            col()
                .key("browse-page")
                .w_full()
                .px(PAGE_X)
                .pt(PAGE_TOP)
                .gap(16.0)
                .child({
                    let head = col().key("browse-head").w_full().gap(8.0).child(super::toolbar::line(s, u));
                    // The landing has no filter chips, and its count would describe the grid
                    // far below: the row is left out there, unless it carries "Saved copy".
                    if super::grid::landing_shown(s) && !s.browse_view.from_mirror {
                        head
                    } else {
                        head.child(super::toolbar::status_row(s, u, &detail))
                    }
                })
                .child(body),
        )
}

/// Creators whose name matches the search, from the local copy. Pressing one scopes the grid
/// to them, which the server cannot do by name.
pub(crate) fn creator_rail(s: &State, u: &Ui) -> Option<El> {
    let hits = &s.browse_view.creator_hits;
    if hits.is_empty() || s.browse_view.applied_search.trim().is_empty() {
        return None;
    }
    let mut strip = row()
        .key("browse-creators")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap();
    for hit in hits.iter().take(8) {
        let pick = Pick {
            id: hit.id,
            name: hit.name.clone(),
        };
        strip = strip.child(
            row()
                .key(format!("browse-creator-{}", hit.id))
                .h(36.0)
                .pl(4.0)
                .pr(12.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(18.0))
                .fill(u.over.alpha(0.04))
                .border(1.0, u.hair_strong)
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("Only mods by {}", hit.name))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    let pick = pick.clone();
                    set_filters(st, Some(tx), move |f| {
                        f.creator = Some(pick);
                        f.favorites = false;
                    })
                })
                .child(
                    el().size(28.0, 28.0)
                        .radius(14.0)
                        .center()
                        .fill(u.accent.alpha(0.15))
                        .border(1.0, u.accent.alpha(0.30))
                        .child(
                            text(
                                hit.name
                                    .chars()
                                    .next()
                                    .map(|c| c.to_uppercase().to_string())
                                    .unwrap_or_else(|| "?".into()),
                            )
                            .font(LABEL)
                            .color(u.accent),
                        ),
                )
                .child(text(hit.name.clone()).font(LABEL).color(u.ink))
                .child(
                    text(format_count(hit.count as u64))
                        .font(SMALL)
                        .color(u.faint),
                ),
        );
    }
    Some(
        col()
            .key("browse-creator-rail")
            .w_full()
            .gap(8.0)
            .child(u.section(super::tree::tr("Creators")))
            .child(strip),
    )
}

/// Numbered pages: previous, a window of numbers, next.
pub(crate) fn pagination(s: &State, u: &Ui) -> Option<El> {
    let page = s.browse.page.as_ref()?;
    let count = page.page_count.max(1);
    if count <= 1 {
        return None;
    }
    let current = page.page.clamp(1, count);
    let first = current.saturating_sub(2).max(1);
    let last = (first + 4).min(count);
    let mut bar = row()
        .key("browse-pages")
        .w_full()
        .gap(4.0)
        .center()
        .pt(8.0)
        .child(
            u.icon_button(&icons::CHEVRON_LEFT, "Previous page")
                .key("browse-page-prev")
                .disabled(current == 1)
                .opacity(if current == 1 { 0.4 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    crate::data::start_browse_fetch(st, current.saturating_sub(1).max(1));
                }),
        );
    for number in first..=last {
        let on = number == current;
        bar = bar.child(
            row()
                .key(format!("browse-page-{number}"))
                .h(32.0)
                .w(36.0)
                .center()
                .radius(crate::ui::r(8.0))
                .fill(if on { u.over.alpha(0.08) } else { Color::TRANSPARENT })
                .border(1.0, if on { u.hair_strong } else { Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("Page {number}"))
                .checked(on)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    crate::data::start_browse_fetch(st, number);
                })
                .child(
                    text(number.to_string())
                        .font(LABEL)
                        .color(if on { u.ink } else { u.muted }),
                ),
        );
    }
    bar = bar.child(
        u.icon_button(&icons::CHEVRON_RIGHT, "Next page")
            .key("browse-page-next")
            .disabled(current >= count)
            .opacity(if current >= count { 0.4 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                crate::data::start_browse_fetch(st, (current + 1).min(count));
            }),
    );
    Some(bar.child(
        text(crate::screens::settings::i18n::fill(super::tree::tr("Page {} of {}"), &[&current.to_string(), &count.to_string()]))
            .font(SMALL)
            .color(u.faint)
            .ml(8.0),
    ))
}


/// How many results the filters give, as the toolbar shows it.
pub(crate) fn result_line(s: &State, shown: usize, stale: bool) -> String {
    if stale || (s.browse.loading && s.browse.page.is_none()) {
        return super::tree::tr("Loading").to_string();
    }
    let Some(page) = s.browse.page.as_ref() else {
        return String::new();
    };
    let f = &s.browse_view.filters;
    let noun = if s.browse_view.tab == Tab::All {
        ("item", "items")
    } else {
        match f.section {
            BrowseSection::Mods => ("mod", "mods"),
            BrowseSection::Sounds => ("sound", "sounds"),
            BrowseSection::Wips => ("WiP", "WiPs"),
        }
    };
    if f.favorites {
        return if shown == 0 {
            String::new()
        } else {
            format!("{} {}", shown, super::tree::tr(if shown == 1 { "favorite" } else { "favorites" }))
        };
    }
    // A complete or locally filtered list counts what shows; an open feed counts the server's
    // total, which the next pages fill in.
    let client_side = f.owned != Owned::Any
        || f.mature != MatureView::Setting
        || f.hide_outdated
        || f.exclude;
    let total = if page.complete || client_side || f.owned == Owned::Installed {
        shown
    } else {
        page.total.max(shown)
    };
    format!(
        "{} {}",
        group_digits(total as u64),
        super::tree::tr(if total == 1 { noun.0 } else { noun.1 })
    )
}
