//! The Browse toolbar: one 32 point line (section tabs, Category, Filters, Sort, the page's own
//! search, View and More) and the fixed 24 point status row under it (result count, active
//! filter chips, Clear all). Neither ever wraps.
//!
//! The line has four tiers picked from the page width alone (`design/browse.md` section 1.3), so
//! it changes on a resize or a dock, never on data. Controls that do not apply are drawn
//! disabled in place, so the line never reflows. Every width is a constant: nothing is measured
//! while the view runs.

use super::*;
use crate::ui::SMALL_NUM;

/// The section tabs' width: container padding 2, six segments of `TAB_PX` each side of their
/// label, gap 2. The labels are names (All, Mods, WiPs...) and stay English in every language.
/// `tabs_width_holds_the_labels` checks it against the font.
pub(crate) const TABS_W: f32 = 352.0;
const TAB_PX: f32 = 10.0;
const CATEGORY_W: f32 = 120.0;
const FILTERS_W: f32 = 92.0;
const FILTERS_ICON_W: f32 = 48.0;
const SORT_W: f32 = 128.0;
const SECTION_W: f32 = 112.0;
/// The count slot inside the Filters button, reserved so a count never changes its width.
const COUNT_SLOT: f32 = 12.0;
/// The result count's slot in the status row, so "Loading" and "6,495 mods" swap in place.
const COUNT_W: f32 = 96.0;
const CHIP_MAX: f32 = 160.0;

/// How much of the line fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    /// Everything on the line.
    A,
    /// Category moves into Filters; Filters and Sort become icons.
    B,
    /// The tabs become one dropdown.
    C,
    /// Search becomes an icon that opens over the line; View folds into More.
    D,
}

pub(crate) fn tier_for(width: f32) -> Tier {
    if width >= TABS_W + 608.0 {
        Tier::A
    } else if width >= TABS_W + 348.0 {
        Tier::B
    } else if width >= 412.0 {
        Tier::C
    } else {
        Tier::D
    }
}

/// The page's inner width, as the grid uses it.
pub(crate) fn page_width(s: &State) -> f32 {
    if s.browse_view.width > 0.0 {
        s.browse_view.width
    } else if s.browse_view.docked && s.browse_view.details.is_some() {
        DEFAULT_GRID_W - SIDEBAR_W
    } else {
        DEFAULT_GRID_W
    }
}

pub(crate) fn tier(s: &State) -> Tier {
    tier_for(page_width(s))
}

/// Where a filter menu hangs from: its own button when that button is on the line, the Filters
/// button when the menu is a sub panel of Filters, More when View folded into it.
pub(crate) fn anchor(s: &State, menu: FilterMenu) -> &'static str {
    let t = tier(s);
    match menu {
        FilterMenu::Category if t == Tier::A && !s.browse_view.from_filters => "browse-filter-category",
        FilterMenu::Category | FilterMenu::Sub | FilterMenu::Date => "browse-filter-more",
        FilterMenu::View if t == Tier::D => "browse-filter-tools",
        other => other.anchor(),
    }
}

fn toggle_menu(st: &mut State, menu: FilterMenu) {
    let v = &mut st.browse_view;
    v.menu = if v.menu == Some(menu) { None } else { Some(menu) };
    v.from_filters = false;
    v.menu_query = TextEdit::new("");
}

/// Grey a control that does not apply here, in place.
fn usable(e: El, on: bool) -> El {
    if on { e } else { e.disabled(true).opacity(0.4) }
}

/// The segmented tabs, for tier A and B.
fn tabs(s: &State, u: &Ui) -> El {
    let current = s.browse_view.tab;
    let mut r = row()
        .key("browse-sections")
        .w(TABS_W)
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
                .px(TAB_PX)
                .center()
                .radius(crate::ui::r(4.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .border(1.0, if on { u.hair_strong } else { Color::TRANSPARENT })
                .hover(if on { u.selected } else { u.hover })
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

/// A fixed-width 32 point dropdown trigger: an optional icon, the label truncating, a chevron.
fn dropdown(u: &Ui, key: &'static str, w: f32, glyph: Option<&'static Icon>, label: &str, active: bool, open: bool) -> El {
    let ink = if active { u.ink } else { u.muted };
    let mut b = row()
        .key(key)
        .w(w)
        .h(32.0)
        .px(8.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if active || open { u.selected } else { u.over.alpha(0.025) })
        .border(1.0, if active || open { u.hair_strong } else { u.hair })
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer);
    if let Some(glyph) = glyph {
        b = b.child(icon(glyph, 14.0).color(ink).a11y_hidden());
    }
    b.child(text(label.to_string()).font(LABEL).color(ink).truncate().min_w(0.0).grow(1.0).a11y_hidden())
        .child(icon(&icons::CHEVRON_DOWN, 14.0).color(u.faint).a11y_hidden())
}

/// A 32 point square icon button with the toolbar's border.
fn square(u: &Ui, key: &'static str, glyph: &'static Icon, label: &str, active: bool) -> El {
    el().key(key)
        .size(32.0, 32.0)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(if active { u.selected } else { u.over.alpha(0.025) })
        .border(1.0, if active { u.hair_strong } else { u.hair })
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .help(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(icon(glyph, 16.0).color(if active { u.ink } else { u.muted }).a11y_hidden())
}

fn filters_button(s: &State, u: &Ui, t: Tier) -> El {
    let count = s.browse_view.filters.active();
    let open = s.browse_view.menu == Some(FilterMenu::More) || s.browse_view.from_filters;
    let on = count > 0 || open;
    let ink = if on { u.ink } else { u.muted };
    let mut b = row()
        .key("browse-filter-more")
        .w(if t == Tier::A { FILTERS_W } else { FILTERS_ICON_W })
        .h(32.0)
        .px(8.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { u.over.alpha(0.025) })
        .border(1.0, if on { u.hair_strong } else { u.hair })
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(if count > 0 { format!("Filters, {count} on") } else { "Filters".to_string() })
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| toggle_menu(st, FilterMenu::More))
        .child(icon(&icons::SLIDERS_HORIZONTAL, 14.0).color(ink).a11y_hidden());
    if t == Tier::A {
        b = b.child(text("Filters").font(LABEL).color(ink).a11y_hidden());
    }
    b.child(
        text(if count > 0 { count.to_string() } else { String::new() })
            .font(SMALL_NUM)
            .color(u.ink)
            .w(COUNT_SLOT)
            .a11y_hidden(),
    )
}

/// The Browse search field, the shell's own `s.browse_query` field moved onto this line. Taking
/// it here takes it out of the top bar on the same frame (`app::page_search`). It takes the
/// slack up to 240 and gives way down to `min`.
fn search_field(s: &State, u: &Ui, w: f32) -> El {
    crate::app::page_search(s, u).key("browse-search-box").w(w)
}

/// The search field's width at a tier: the room the fixed controls leave, between the tier's
/// minimum and 240. Every other width on the line is a constant, so this is exact.
pub(crate) fn search_width(width: f32, t: Tier) -> f32 {
    const TRAILING: f32 = 32.0 + 4.0 + 32.0;
    let (fixed, min) = match t {
        Tier::A => (TABS_W + CATEGORY_W + FILTERS_W + SORT_W + TRAILING + 6.0 * 8.0, 160.0),
        Tier::B => (TABS_W + FILTERS_ICON_W + 32.0 + TRAILING + 5.0 * 8.0, 160.0),
        Tier::C => (SECTION_W + FILTERS_ICON_W + 32.0 + TRAILING + 5.0 * 8.0, 120.0),
        Tier::D => return 0.0,
    };
    (width - fixed).clamp(min, 240.0)
}

/// The one toolbar line.
pub(crate) fn line(s: &State, u: &Ui) -> El {
    let t = tier(s);
    let f = &s.browse_view.filters;
    let open = s.browse_view.menu;
    let collecting = s.browse_view.tab == Tab::Collections;
    let local = f.favorites || f.owned == Owned::Installed;

    // Tier D with the search open: the field takes the whole line.
    if t == Tier::D && s.browse_view.search_open {
        return row()
            .key("browse-toolbar")
            .w_full()
            .h(32.0)
            .gap(8.0)
            .center_y()
            .child(search_field(s, u, (page_width(s) - 40.0).max(0.0)))
            .child(
                square(u, "browse-search-close", &icons::X, "Close search", false)
                    .on_press(|st: &mut State| st.browse_view.search_open = false),
            );
    }

    let mut r = row().key("browse-toolbar").w_full().h(32.0).gap(8.0).center_y();
    r = r.child(if t <= Tier::B {
        tabs(s, u)
    } else {
        dropdown(
            u,
            "browse-filter-section",
            SECTION_W,
            None,
            s.browse_view.tab.label(),
            false,
            open == Some(FilterMenu::Section),
        )
        .on_press(|st: &mut State| toggle_menu(st, FilterMenu::Section))
    });
    if t == Tier::A {
        let maps = s.browse_view.tab == Tab::Maps;
        r = r.child(usable(
            dropdown(
                u,
                "browse-filter-category",
                CATEGORY_W,
                None,
                f.category.as_ref().map(|p| p.name.as_str()).unwrap_or("Category"),
                f.category.is_some(),
                open == Some(FilterMenu::Category) && !s.browse_view.from_filters,
            )
            .on_press(|st: &mut State| toggle_menu(st, FilterMenu::Category)),
            !collecting && !local && !maps,
        ));
    }
    r = r.child(usable(filters_button(s, u, t), !collecting));
    let order = current_query(s).shown_order();
    let sort = if t == Tier::A {
        dropdown(u, "browse-filter-sort", SORT_W, Some(&SORT), order.label(), false, open == Some(FilterMenu::Sort))
    } else {
        square(u, "browse-filter-sort", &SORT, &format!("Sort: {}", order.label()), open == Some(FilterMenu::Sort))
    };
    r = r.child(usable(
        sort.on_press(|st: &mut State| toggle_menu(st, FilterMenu::Sort)),
        !collecting && !local,
    ));
    r = r.child(spacer());
    r = r.child(if t == Tier::D {
        square(u, "browse-search-open", &icons::SEARCH, "Search mods", !s.browse_query.text.is_empty())
            .on_press(|st: &mut State| st.browse_view.search_open = true)
    } else {
        search_field(s, u, search_width(page_width(s), t))
    });
    let trailing = row().gap(4.0).center_y();
    let trailing = if t == Tier::D {
        trailing
    } else {
        trailing.child(usable(
            square(
                u,
                "browse-filter-view",
                if s.browse_view.layout == Layout::List { &icons::LIST } else { &icons::LAYOUT_GRID },
                "View options",
                open == Some(FilterMenu::View),
            )
            .on_press(|st: &mut State| toggle_menu(st, FilterMenu::View)),
            !collecting,
        ))
    };
    r.child(
        trailing.child(
            square(u, "browse-filter-tools", &icons::ELLIPSIS, "More", open == Some(FilterMenu::Tools))
                .on_press(|st: &mut State| toggle_menu(st, FilterMenu::Tools)),
        ),
    )
}

/// One removable chip in the status row.
fn chip(u: &Ui, key: &str, label: String) -> El {
    row()
        .key(format!("browse-chip-{key}"))
        .h(24.0)
        .max_w(CHIP_MAX)
        .pl(8.0)
        .pr(6.0)
        .gap(4.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .fill(u.over.alpha(0.06))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("Remove {label}"))
        .help(label.clone())
        .cursor(rune::Cursor::Pointer)
        .child(text(label).font(LABEL).color(u.prose).truncate().min_w(0.0).shrink().a11y_hidden())
        .child(icon(&icons::X, 12.0).color(u.faint).a11y_hidden())
}

/// The active filters as chips, in reading order. The search text comes first.
fn chips(s: &State, u: &Ui) -> Vec<El> {
    let f = &s.browse_view.filters;
    let mut out = Vec::new();
    let clearing = |key: &str, label: String, clear: fn(&mut Filters)| {
        chip(u, key, label).on_press_tx(move |st: &mut State, tx: &mut Tx| set_filters(st, Some(tx), clear))
    };
    let search = s.browse_view.applied_search.trim();
    if !search.is_empty() {
        out.push(chip(u, "search", format!("\u{201c}{search}\u{201d}")).on_press(|st: &mut State| {
            st.browse_query = TextEdit::new("");
        }));
    }
    let not = if f.exclude { "Not " } else { "" };
    if let Some(pick) = &f.category {
        out.push(clearing("category", format!("{not}{}", pick.name), |f| {
            f.category = None;
            f.sub = None;
        }));
    }
    if let Some(pick) = &f.sub {
        for step in super::tree::middle(s) {
            out.push(super::tree::crumb(u, step));
        }
        out.push(clearing("sub", format!("{not}{}", pick.name), |f| f.sub = None));
    }
    if let Some(hero) = &f.hero {
        out.push(clearing("hero", format!("Hero: {hero}"), |f| f.hero = None));
    }
    if f.window() {
        out.push(clearing("period", date_label(f), |f| {
            f.period = BrowsePeriod::Any;
            f.date_from = 0;
            f.date_to = 0;
        }));
    }
    if f.exclude {
        out.push(clearing("exclude", "Excluding".into(), |f| f.exclude = false));
    }
    if f.featured {
        out.push(clearing("featured", "Featured".into(), |f| f.featured = false));
    }
    if let Some(pick) = &f.creator {
        out.push(clearing("creator", format!("By {}", pick.name), |f| f.creator = None));
    }
    if f.owned != Owned::Any {
        out.push(clearing("owned", f.owned.label().to_string(), |f| f.owned = Owned::Any));
    }
    if f.mature != MatureView::Setting {
        out.push(clearing("mature", f.mature.label().to_string(), |f| f.mature = MatureView::Setting));
    }
    if f.hide_outdated {
        out.push(clearing("outdated", "Hide outdated".into(), |f| f.hide_outdated = false));
    }
    if f.favorites {
        out.push(clearing("favorites", "Favorites".into(), |f| f.favorites = false));
    }
    out
}

/// The status row: always 24 points high, one line. With no filters the chip area is empty and
/// the row keeps its height, so the grid under it never moves.
pub(crate) fn status_row(s: &State, u: &Ui, detail: &str) -> El {
    let mut r = row()
        .key("browse-status")
        .w_full()
        .h(24.0)
        .gap(8.0)
        .center_y()
        .child(
            text(detail.to_string())
                .font(SMALL_NUM)
                .color(u.muted)
                .w(COUNT_W)
                .truncate(),
        );
    if s.browse_view.from_mirror {
        r = r.child(
            row()
                .h(20.0)
                .px(6.0)
                .center()
                .radius(crate::ui::r(4.0))
                .border(1.0, u.hair_strong)
                .child(text("Saved copy").font(crate::ui::MICRO).color(u.muted)),
        );
    }
    let all = chips(s, u);
    let room = match tier(s) {
        Tier::A => 4,
        Tier::B => 3,
        Tier::C => 1,
        Tier::D => 0,
    };
    let total = all.len();
    let mut chip_row = row().key("browse-chips").gap(8.0).center_y().min_w(0.0).shrink();
    for c in all.into_iter().take(room) {
        chip_row = chip_row.child(c);
    }
    if total > room {
        chip_row = chip_row.child(
            row()
                .key("browse-chip-rest")
                .h(24.0)
                .px(8.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .fill(u.over.alpha(0.06))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(format!("{} more filters", total - room))
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.browse_view.from_filters = false;
                    st.browse_view.menu = Some(FilterMenu::More);
                })
                .child(text(format!("+{}", total - room)).font(LABEL).color(u.prose).a11y_hidden()),
        );
    }
    if total > 0 {
        chip_row = chip_row.child(
            row()
                .key("browse-clear-all")
                .h(24.0)
                .px(8.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label("Clear all filters")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(clear_filters)
                .child(text("Clear all").font(LABEL).color(u.muted).a11y_hidden()),
        );
    }
    r = r.child(chip_row).child(spacer());
    if s.browse_view.paged
        && let Some(page) = s.browse.page.as_ref()
        && page.page_count > 1
    {
        r = r.child(
            text(format!("Page {} of {}", page.page.clamp(1, page.page_count), page.page_count))
                .font(SMALL_NUM)
                .color(u.muted),
        );
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_follow_the_spec() {
        assert_eq!(tier_for(984.0), Tier::A, "1280 window");
        assert_eq!(tier_for(744.0), Tier::B, "1040 window");
        assert_eq!(tier_for(544.0), Tier::C, "1280 docked");
        assert_eq!(tier_for(304.0), Tier::D, "1040 docked");
    }

    #[test]
    fn the_search_field_takes_the_slack_up_to_240() {
        assert_eq!(search_width(984.0, Tier::A), 176.0);
        assert_eq!(search_width(744.0, Tier::B), 204.0);
        assert_eq!(search_width(1400.0, Tier::A), 240.0);
        assert_eq!(search_width(544.0, Tier::C), 240.0);
    }

    #[test]
    fn tabs_width_holds_the_labels() {
        let labels: f32 = Tab::ALL
            .iter()
            .map(|tab| rune::text::measure(tab.label(), &[], LABEL, None).0 + 2.0 * TAB_PX)
            .sum();
        // Container padding 2 each side, five gaps of 2, a 2 point border.
        let need = labels + 4.0 + 10.0 + 2.0;
        assert!(need <= TABS_W, "tabs need {need}");
        assert!(TABS_W - need < 24.0, "TABS_W wastes {} points", TABS_W - need);
    }
}
