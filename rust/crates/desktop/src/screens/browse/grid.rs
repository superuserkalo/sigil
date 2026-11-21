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
        return "Custom range".to_string();
    }
    if f.period != BrowsePeriod::Any {
        return format!("{}: {}", f.date_field.label(), f.period.label());
    }
    "Any time".to_string()
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
    let hide = s.prefs.mature == crate::prefs::Mature::Hide;
    let feeds = s.browse_view.shelves.feeds.as_ref();
    // A mod shows on one shelf only: the month shelf skips this week's, all time skips both.
    let mut seen: HashSet<u64> = HashSet::new();
    let mut tiles = |scope: &str, list: Option<&Vec<BrowseCard>>| -> Vec<El> {
        match list {
            Some(list) => list
                .iter()
                .filter(|c| !(hide && c.nsfw))
                .filter(|c| seen.insert(c.id))
                .take(shelf::PER_SHELF)
                .map(|c| shelf::card(s, u, scope, c, w))
                .collect(),
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
    // DMM's quick stats strip: always on the plain landing, each number a shortcut.
    let mut rails = col().key("browse-dashboard").w_full().gap(24.0).child(super::strip::view(s, u));
    rails = rails
        .child(shelf::shelf(
            s,
            u,
            shelf::TRENDING,
            "browse-shelf-week",
            "Trending this week",
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
            "Popular this month",
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
    if !featured.is_empty() {
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
    if let Some(heroes) = hero_shelf(s, u, width) {
        rails = rails.child(heroes);
    }
    rails = rails.child(shelf::shelf(
        s,
        u,
        shelf::ALL_TIME,
        "browse-shelf-all",
        "All-time favorites",
        Some(see("browse-shelf-all-all", BrowsePeriod::Any)),
        tiles("all", feeds.map(|f| &f.all)),
        n,
        w,
        shelf::GAP,
    ));
    Some(
        rails.child(
            row()
                .key("browse-all-head")
                .w_full()
                .h(24.0)
                .center_y()
                .child(text("All mods").font(H2).color(u.ink)),
        ),
    )
}

/// Hero portraits from the Skins tree as a shelf of 72 point tiles. A press opens that hero's
/// skins. Paged like every shelf; "Random" stands where "See all" is.
pub(crate) fn hero_shelf(s: &State, u: &Ui, width: f32) -> Option<El> {
    use super::shelf;
    let heroes = hero_list(s);
    if heroes.is_empty() {
        return None;
    }
    const TILE: f32 = 72.0;
    let n = shelf::heroes_per_page(width);
    let gap = if n > 1 { ((width - TILE * n as f32) / (n as f32 - 1.0)).floor() } else { 0.0 };
    let tiles: Vec<El> = heroes
        .into_iter()
        .map(|hero| {
            let name = hero.name.clone();
            let portrait = hero_portrait(s, &hero.name);
            col()
                .key(format!("browse-hero-{}", hero.id))
                .w(TILE)
                .min_w(TILE)
                .gap(8.0)
                .items_center()
                .focusable(true)
                .role("button")
                .label(format!("Browse {} skins", hero.name))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_skins_for(st, tx, &name))
                .child(
                    el().size(56.0, 56.0)
                        .radius(crate::ui::r(8.0))
                        .clip_rounded(crate::ui::r(8.0))
                        .fill(u.hover)
                        .hover(u.over.alpha(0.12))
                        .child(match portrait {
                            Some(pixels) => el()
                                .absolute(0.0, 0.0)
                                .w_full()
                                .h_full()
                                .pixels_fit(pixels, SurfaceFit::Fill)
                                .a11y_hidden(),
                            None => el()
                                .absolute(0.0, 0.0)
                                .w_full()
                                .h_full()
                                .center()
                                .child(
                                    text(hero.name.chars().next().unwrap_or('?').to_string())
                                        .font(BODY_MED)
                                        .color(u.muted)
                                        .a11y_hidden(),
                                ),
                        }),
                )
                .child(
                    text(hero.name)
                        .font(SMALL)
                        .color(u.muted)
                        .truncate()
                        .max_w(TILE)
                        .align(rune::Align::Center),
                )
        })
        .collect();
    Some(shelf::shelf(
        s,
        u,
        shelf::HEROES,
        "browse-heroes",
        "Browse by hero",
        // Wave 4: DMM's random hero.
        Some(super::w4::random_button(u)),
        tiles,
        n,
        TILE,
        gap,
    ))
}

pub(crate) fn hero_portrait<'a>(s: &'a State, hero: &str) -> Option<&'a rune::Pixels> {
    let list = categories_of(s, BrowseSection::Mods)?;
    let skins = list
        .iter()
        .find(|root| root.name.eq_ignore_ascii_case("Skins"))?;
    let child = skins
        .children
        .iter()
        .find(|child| child.name.eq_ignore_ascii_case(hero))?;
    child.icon_url.as_ref().and_then(|url| s.thumbnails.get(url))
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

