//! The grid: filter bar, card and list layouts, and the dropdowns over them.

use super::*;
pub(crate) use self::cards::*;
pub(crate) use self::menus::*;

pub mod cards;
pub mod menus;

// --- filter bar ------------------------------------------------------------------

/// The tab row: the mixed feed, each section, Collections and Maps.
pub(crate) fn tab_switch(s: &State, u: &Ui) -> El {
    let current = s.browse_view.tab;
    let mut r = row()
        .key("browse-sections")
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .role("radiogroup");
    for tab in Tab::ALL {
        let on = tab == current;
        r = r.child(
            row()
                .key(format!("browse-tab-{}", tab.id()))
                .h_full()
                .px(12.0)
                .center()
                .radius(crate::ui::r(6.0))
                .fill(if on { u.raised } else { Color::TRANSPARENT })
                .border(1.0, if on { u.hair_strong } else { Color::TRANSPARENT })
                .hover(if on { u.raised } else { u.hover })
                .focusable(true)
                .role("radio")
                .label(tab.label())
                .checked(on)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| set_tab(st, Some(tx), tab))
                .child(
                    text(tab.label())
                        .font(LABEL)
                        .color(if on { u.ink } else { u.muted })
                        .a11y_hidden(),
                ),
        );
    }
    r
}

/// A 32 point dropdown trigger. Active ones read in ink and name their value.
pub(crate) fn filter_button(
    u: &Ui,
    menu: FilterMenu,
    open: bool,
    glyph: Option<&'static Icon>,
    label: &str,
    active: bool,
) -> El {
    let ink = if active { u.ink } else { u.muted };
    let mut b = row()
        .key(menu.anchor())
        .h(32.0)
        .pl(if glyph.is_some() { 10.0 } else { 12.0 })
        .pr(8.0)
        .gap(6.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if active || open { u.over.alpha(0.06) } else { u.over.alpha(0.025) })
        .border(1.0, if open { u.over.alpha(0.18) } else { u.hair_strong })
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let v = &mut st.browse_view;
            v.menu = if v.menu == Some(menu) { None } else { Some(menu) };
            v.menu_query = TextEdit::new("");
        });
    if let Some(glyph) = glyph {
        b = b.child(icon(glyph, 14.0).color(ink).a11y_hidden());
    }
    b.child(text(label.to_string()).font(LABEL).color(ink).a11y_hidden())
        .child(icon(&icons::CHEVRON_DOWN, 14.0).color(u.faint).a11y_hidden())
}

/// "Hero" under Skins, "Type" under any other category.
pub(crate) fn sub_noun(category: &str) -> &'static str {
    if category.eq_ignore_ascii_case("skins") {
        "Hero"
    } else {
        "Type"
    }
}

pub(crate) fn filter_bar(s: &State, u: &Ui) -> El {
    let f = &s.browse_view.filters;
    let open = s.browse_view.menu;
    let local = f.favorites || f.owned == Owned::Installed;
    let collecting = s.browse_view.tab == Tab::Collections;
    let mut left = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(tab_switch(s, u));

    if !local && !collecting {
        // Maps is a category of its own; its name stands in the chip, so the picker stays.
        if s.browse_view.tab != Tab::Maps {
            left = left.child(filter_button(
                u,
                FilterMenu::Category,
                open == Some(FilterMenu::Category),
                None,
                f.category.as_ref().map(|p| p.name.as_str()).unwrap_or("Category"),
                f.category.is_some(),
            ));
        }
        let children = f.category.as_ref().and_then(|pick| {
            categories_of(s, f.section)?
                .iter()
                .find(|root| root.id == pick.id)
                .filter(|root| !root.children.is_empty())
        });
        if let (Some(category), Some(_)) = (f.category.as_ref(), children) {
            left = left.child(filter_button(
                u,
                FilterMenu::Sub,
                open == Some(FilterMenu::Sub),
                None,
                f.sub
                    .as_ref()
                    .map(|p| p.name.as_str())
                    .unwrap_or(sub_noun(&category.name)),
                f.sub.is_some(),
            ));
        }
        // Sounds and WiPs have no hero categories, so a name match stands in.
        let skins = f
            .category
            .as_ref()
            .is_some_and(|pick| pick.name.eq_ignore_ascii_case("Skins"));
        if s.browse_view.tab != Tab::Mods || !skins {
            left = left.child(
                row()
                    .key("browse-filter-hero")
                    .h(32.0)
                    .px(12.0)
                    .gap(6.0)
                    .center_y()
                    .radius(crate::ui::r(8.0))
                    .fill(if f.hero.is_some() { u.over.alpha(0.06) } else { u.over.alpha(0.025) })
                    .border(1.0, if f.hero.is_some() { u.over.alpha(0.18) } else { u.hair_strong })
                    .hover(u.hover)
                    .press(u.over.alpha(0.08))
                    .focusable(true)
                    .role("button")
                    .label(f.hero.as_deref().unwrap_or("Hero"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.browse_view.hero_picker = Some(HeroTarget::Filter);
                        st.browse_view.hero_query = TextEdit::new("");
                    })
                    .child(
                        icon(&icons::USER, 14.0)
                            .color(if f.hero.is_some() { u.ink } else { u.muted })
                            .a11y_hidden(),
                    )
                    .child(
                        text(f.hero.clone().unwrap_or_else(|| "Hero".into()))
                            .font(LABEL)
                            .color(if f.hero.is_some() { u.ink } else { u.muted })
                            .a11y_hidden(),
                    )
                    .child(icon(&icons::CHEVRON_DOWN, 14.0).color(u.faint).a11y_hidden()),
            );
        }
        left = left.child(filter_button(
            u,
            FilterMenu::Date,
            open == Some(FilterMenu::Date),
            Some(&CLOCK),
            &date_label(f),
            f.window(),
        ));
    }

    let mut right = row().gap(8.0).center_y();
    if !collecting {
        let saved = favorites(s).len();
        let star = if f.favorites { &STAR_FILLED } else { &STAR };
        right = right.child(
            u.icon_button(star, &if saved > 0 { format!("Favorites ({saved})") } else { "Favorites".to_string() })
                .key("browse-favorites")
                .border(1.0, if f.favorites { u.over.alpha(0.18) } else { u.hair_strong })
                .fill(if f.favorites { u.over.alpha(0.06) } else { u.over.alpha(0.025) })
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                    ensure_favorites(st);
                    set_filters(st, Some(tx), |f| f.favorites = !f.favorites)
                }),
        );

        let more = [
            f.owned != Owned::Any,
            f.mature != MatureView::Setting,
            f.hide_outdated,
            f.exclude,
        ]
        .iter()
        .filter(|on| **on)
        .count();
        let mut filters = u
            .icon_button(&icons::SLIDERS_HORIZONTAL, "Filters")
            .key("browse-filter-more")
            .border(1.0, if more > 0 { u.over.alpha(0.18) } else { u.hair_strong })
            .fill(if more > 0 { u.over.alpha(0.06) } else { u.over.alpha(0.025) })
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                let v = &mut st.browse_view;
                v.menu = if v.menu == Some(FilterMenu::More) {
                    None
                } else {
                    Some(FilterMenu::More)
                };
            });
        if more > 0 {
            filters = filters.child(
                text(more.to_string())
                    .font(SMALL)
                    .color(u.ink)
                    .a11y_hidden(),
            );
        }
        right = right.child(filters);
    }
    row()
        .key("browse-filters")
        .w_full()
        .gap(8.0)
        .items_start()
        .child(left)
        .child(right)
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

/// One removable chip per active filter, and Clear all. Nothing when no filter is on.
pub(crate) fn active_chips(s: &State, u: &Ui) -> Option<El> {
    let f = &s.browse_view.filters;
    let search = s.browse_view.applied_search.trim();
    if f.active() == 0 && search.is_empty() {
        return None;
    }
    let chip = |key: &str, label: String| {
        row()
            .key(format!("browse-chip-{key}"))
            .h(28.0)
            .pl(10.0)
            .pr(6.0)
            .gap(4.0)
            .center_y()
            .radius(crate::ui::r(14.0))
            .fill(u.over.alpha(0.06))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(format!("Remove {label}"))
            .cursor(rune::Cursor::Pointer)
            .child(text(label).font(LABEL).color(u.prose).a11y_hidden())
            .child(icon(&icons::X, 12.0).color(u.faint).a11y_hidden())
    };
    let clearing = |key: &str, label: String, clear: fn(&mut Filters)| {
        chip(key, label)
            .on_press_tx(move |st: &mut State, tx: &mut Tx| set_filters(st, Some(tx), clear))
    };
    let mut r = row().key("browse-chips").w_full().gap(8.0).center_y().flex_wrap();
    if !search.is_empty() {
        r = r.child(
            chip("search", format!("\u{201c}{search}\u{201d}")).on_press(|st: &mut State| {
                st.browse_query = TextEdit::new("");
            }),
        );
    }
    let not = if f.exclude { "Not " } else { "" };
    if let Some(pick) = &f.category {
        r = r.child(clearing("category", format!("{not}{}", pick.name), |f| {
            f.category = None;
            f.sub = None;
        }));
    }
    if let Some(pick) = &f.sub {
        r = r.child(clearing("sub", format!("{not}{}", pick.name), |f| f.sub = None));
    }
    if let Some(hero) = &f.hero {
        r = r.child(clearing("hero", format!("Hero: {hero}"), |f| f.hero = None));
    }
    if f.window() {
        r = r.child(clearing("period", date_label(f), |f| {
            f.period = BrowsePeriod::Any;
            f.date_from = 0;
            f.date_to = 0;
        }));
    }
    if f.exclude {
        r = r.child(clearing("exclude", "Excluding".into(), |f| f.exclude = false));
    }
    if f.featured {
        r = r.child(clearing("featured", "Featured".into(), |f| f.featured = false));
    }
    if let Some(pick) = &f.creator {
        r = r.child(clearing("creator", format!("By {}", pick.name), |f| f.creator = None));
    }
    if f.owned != Owned::Any {
        r = r.child(clearing("owned", f.owned.label().to_string(), |f| f.owned = Owned::Any));
    }
    if f.mature != MatureView::Setting {
        r = r.child(clearing("mature", f.mature.label().to_string(), |f| {
            f.mature = MatureView::Setting
        }));
    }
    if f.hide_outdated {
        r = r.child(clearing("outdated", "Hide outdated".into(), |f| f.hide_outdated = false));
    }
    if f.favorites {
        r = r.child(clearing("favorites", "Favorites".into(), |f| f.favorites = false));
    }
    Some(
        r.child(
            row()
                .key("browse-clear-all")
                .h(28.0)
                .px(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label("Clear all filters")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(clear_filters)
                .child(text("Clear all").font(LABEL).color(u.muted).a11y_hidden()),
        ),
    )
}

/// The landing dashboard over the plain catalog: trending rails and browse by hero. No
/// announcements; nothing here needs a server of Sigil's own.
pub(crate) fn dashboard(s: &State, u: &Ui) -> Option<El> {
    let f = &s.browse_view.filters;
    let plain = matches!(s.browse_view.tab, Tab::Mods | Tab::All)
        && f.active() == 0
        && s.browse_view.applied_search.is_empty()
        && s.browse_query.text.trim().is_empty()
        && (current_query(s).uses_subfeed() || s.browse_view.tab == Tab::All);
    if !plain {
        return None;
    }
    let mut rails = col().key("browse-dashboard").w_full().gap(20.0);
    let mut any = false;
    if let Some(rail) = trending_rail(s, u, "week", "Trending this week") {
        rails = rails.child(rail);
        any = true;
    }
    if let Some(rail) = trending_rail(s, u, "month", "Popular this month") {
        rails = rails.child(rail);
        any = true;
    }
    if let Some(rail) = hero_rail(s, u) {
        rails = rails.child(rail);
        any = true;
    }
    any.then(|| rails.child(el().h(4.0)))
}

pub(crate) fn trending_rail(s: &State, u: &Ui, period: &str, title: &str) -> Option<El> {
    let top = s.browse_view.top.as_ref()?;
    let items: Vec<&TopSub> = top
        .iter()
        .filter(|t| t.period == period && t.model.eq_ignore_ascii_case("mod"))
        .filter(|t| !(t.nsfw && s.prefs.mature == crate::prefs::Mature::Hide))
        .take(3)
        .collect();
    if items.is_empty() {
        return None;
    }
    let mut tiles = el().w_full().equal_columns(3).gap(GRID_GAP);
    for item in items {
        let id = item.id;
        let name = item.name.clone();
        let image = item.image_url.clone();
        let veil = item.nsfw
            && s.prefs.mature == crate::prefs::Mature::Blur
            && !s.browse_view.revealed.contains(&id)
            && !crate::screens::settings::nsfw::remembered(id);
        tiles = tiles.child(
            row()
                .key(format!("browse-top-{period}-{id}"))
                .min_w(0.0)
                .h(64.0)
                .padding(6.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(10.0))
                .border(1.0, u.hair)
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("{} by {}", item.name, item.author))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    show_details(st, Details::from_id(BrowseSection::Mods, id, &name, image.clone()))
                })
                .child(
                    el().w(92.0)
                        .h_full()
                        .clip_rounded(crate::ui::r(6.0))
                        .child(image_fill(
                            s,
                            format!("top-thumb-{id}"),
                            item.image_url.as_ref(),
                            [6.0; 4],
                            veil,
                        )),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(item.name.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(
                                    text(if item.author.is_empty() {
                                        item.category.clone()
                                    } else {
                                        format!("by {}", item.author)
                                    })
                                    .font(SMALL)
                                    .color(u.muted)
                                    .truncate()
                                    .min_w(0.0)
                                    .shrink(),
                                )
                                .child(stat(u, &icons::THUMBS_UP, item.likes)),
                        ),
                ),
        );
    }
    Some(
        col()
            .key(format!("browse-rail-{period}"))
            .w_full()
            .gap(12.0)
            .child(text(title).font(BODY_MED).color(u.muted))
            .child(tiles),
    )
}

/// Hero portraits from the Skins tree, as a filmstrip. A press opens that hero's skins.
pub(crate) fn hero_rail(s: &State, u: &Ui) -> Option<El> {
    let heroes = hero_list(s);
    if heroes.is_empty() {
        return None;
    }
    let mut strip = row().key("browse-heroes").w_full().gap(12.0).scroll_x();
    for hero in heroes.into_iter().take(24) {
        let name = hero.name.clone();
        let portrait = hero_portrait(s, &hero.name);
        strip = strip.child(
            col()
                .key(format!("browse-hero-{}", hero.id))
                .w(88.0)
                .gap(6.0)
                .items_center()
                .focusable(true)
                .role("button")
                .label(format!("Browse {} skins", hero.name))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_skins_for(st, tx, &name))
                .child(
                    el().size(56.0, 56.0)
                        .radius(28.0)
                        .clip_rounded(28.0)
                        .fill(u.hover)
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
                        .max_w(88.0),
                ),
        );
    }
    Some(
        col()
            .key("browse-hero-rail")
            .w_full()
            .gap(12.0)
            .child(text("Browse by hero").font(BODY_MED).color(u.muted))
            .child(strip),
    )
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

