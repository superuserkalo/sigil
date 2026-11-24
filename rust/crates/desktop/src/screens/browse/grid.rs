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

/// Whether the landing top shows: Mods or All with no filter and no search.
pub(crate) fn landing_shown(s: &State) -> bool {
    let f = &s.browse_view.filters;
    matches!(s.browse_view.tab, Tab::Mods | Tab::All)
        && f.active() == 0
        && s.browse_view.applied_search.is_empty()
        && s.browse_query.text.trim().is_empty()
        && (current_query(s).uses_subfeed() || s.browse_view.tab == Tab::All)
}

/// The landing over the plain catalog (`design/browse-top.md`): the billboard and its strip,
/// the rows, then the grid's header naming its sort. Every box is drawn at its final size from
/// the first frame, so the grid below never moves.
pub(crate) fn dashboard(s: &State, u: &Ui) -> Option<El> {
    if !landing_shown(s) {
        return None;
    }
    Some(
        col()
            .key("browse-landing")
            .w_full()
            .child(super::top::view(s, u))
            .child(u.section(super::tree::tr(current_query(s).shown_order().label())).key("browse-all-head")),
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

