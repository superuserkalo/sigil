//! The grid: filter bar, card and list layouts, and the dropdowns over them.

use super::*;
pub(crate) use self::cards::*;
pub(crate) use self::menus::*;

pub mod cards;
pub mod menus;

// --- filter bar ------------------------------------------------------------------

/// "Hero" under Skins, "Type" under any other category.
pub(crate) fn sub_noun(category: &str) -> &'static str {
    if category.eq_ignore_ascii_case("skins") {
        "Hero"
    } else {
        "Type"
    }
}

/// The date button names the window it keeps.
pub(crate) fn date_label(f: &Filters) -> String {
    if f.date_from > 0 || f.date_to > 0 {
        return super::tree::tr("Custom range").to_string();
    }
    if f.period != BrowsePeriod::Any {
        return format!("{}: {}", super::tree::tr(f.date_field.label()), super::tree::tr(f.period.label()));
    }
    super::tree::tr("Any time").to_string()
}

/// The landing over the plain catalog (`design/browse.md` 2): the quick stats strip, then the
/// shelves, Steam and Netflix style, then the "All mods" header over the grid. Every shelf is
/// drawn at its final size from the first frame (placeholders until its read lands), so the
/// grid below never moves. No announcements; nothing here needs a server of Sigil's own.
pub(crate) fn dashboard(s: &State, u: &Ui) -> Option<El> {
    use super::shelf;
    let f = &s.browse_view.filters;
    let plain = matches!(s.browse_view.tab, Tab::Mods | Tab::All)
        && f.active() == 0
        && s.browse_view.applied_search.is_empty()
        && s.browse_query.text.trim().is_empty()
        && (current_query(s).uses_subfeed() || s.browse_view.tab == Tab::All);
    if !plain {
        return None;
    }
    let width = super::toolbar::page_width(s);
    let n = shelf::per_page(width);
    let w = shelf::card_width(width, n);
    let feeds = s.browse_view.shelves.feeds.as_ref();
    // A mod shows once on the landing: not in a shelf when the spotlight has it, and in one
    // shelf only. A short shelf keeps empty slots rather than repeat a mod.
    let mut seen: HashSet<u64> = super::spotlight::picks(s).iter().map(|t| t.id).collect();
    let mut tiles = |scope: &str, list: Option<&Vec<BrowseCard>>| -> Vec<El> {
        match list {
            Some(list) => {
                let fresh: Vec<&BrowseCard> =
                    shelf::allowed(s, list).into_iter().filter(|c| !seen.contains(&c.id)).collect();
                fresh
                    .into_iter()
                    .take(shelf::PER_SHELF)
                    .map(|c| {
                        seen.insert(c.id);
                        shelf::card(s, u, scope, c, w)
                    })
                    .collect()
            }
            None => (0..n).map(|_| shelf::placeholder(u, w)).collect(),
        }
    };
    let see = |key: &'static str, period: BrowsePeriod| {
        shelf::see_all(u, key).on_press_tx(move |st: &mut State, tx: &mut Tx| {
            set_filters(st, Some(tx), move |f| {
                f.order = Some(BrowseOrder::MostLiked);
                f.period = period;
            })
        })
    };
    // The spotlight: always on the plain landing.
    let mut rails = col().key("browse-dashboard").w_full().gap(24.0).child(super::spotlight::view(s, u));
    rails = rails
        .child(shelf::shelf(
            s,
            u,
            shelf::TRENDING,
            "browse-shelf-week",
            super::tree::tr("Trending this week"),
            Some(see("browse-shelf-week-all", BrowsePeriod::Week)),
            tiles("week", feeds.map(|f| &f.week)),
            n,
            w,
            shelf::GAP,
        ))
        .child(shelf::shelf(
            s,
            u,
            shelf::MONTH,
            "browse-shelf-month",
            super::tree::tr("Popular this month"),
            Some(see("browse-shelf-month-all", BrowsePeriod::Month)),
            tiles("month", feeds.map(|f| &f.month)),
            n,
            w,
            shelf::GAP,
        ));
    // Wave 4: one featured mod per category this week (DMM), from the saved catalog. Known
    // before the page draws, so it never pops in above the grid.
    let featured: Vec<BrowseCard> = s
        .browse_view
        .w4
        .featured
        .iter()
        .map(|(_, card)| card.clone())
        .collect();
    if !featured.is_empty() && s.browse_view.shelves.featured.unwrap_or(true) {
        rails = rails.child(shelf::shelf(
            s,
            u,
            shelf::FEATURED,
            "browse-shelf-featured",
            crate::screens::settings::i18n::tr("Featured this week"),
            Some(shelf::see_all(u, "browse-shelf-featured-all").on_press_tx(|st: &mut State, tx: &mut Tx| {
                set_filters(st, Some(tx), |f| f.featured = true)
            })),
            tiles("featured", Some(&featured)),
            n,
            w,
            shelf::GAP,
        ));
    }
    Some(
        rails.child(
            u.section(super::tree::tr("All mods")).key("browse-all-head"),
        ),
    )
}

/// Open Mods on Skins with one hero's category picked.
pub(crate) fn open_skins_for(st: &mut State, tx: &mut Tx, hero: &str) {
    let Some(skins) = skins_pick(&*st) else {
        return;
    };
    close_details(st);
    let sub = hero_list(&*st)
        .into_iter()
        .find(|pick| pick.name.eq_ignore_ascii_case(hero));
    let hero = hero.to_string();
    st.browse_view.tab = Tab::Mods;
    set_filters(st, Some(tx), move |f| {
        *f = Filters {
            section: BrowseSection::Mods,
            category: Some(skins),
            sub,
            order: f.order,
            mature: f.mature,
            hide_outdated: f.hide_outdated,
            ..Filters::default()
        };
        if f.sub.is_none() {
            f.hero = Some(hero);
        }
    });
}

