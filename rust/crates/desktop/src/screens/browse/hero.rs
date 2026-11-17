//! Opening Browse on one hero's skins, as the Locker asks.

use super::*;

const FIXTURE_CATEGORIES: &str = include_str!("../../../fixtures/browse/categories.json");
/// Haze under Skins on GameBanana.
#[cfg(test)]
const HAZE: u64 = 33311;

// --- open from another screen ----------------------------------------------------

/// Open Browse on Skins with `hero` picked as the sub-category, as the Locker's hero page
/// asks. The hero's GameBanana category comes from the loaded category tree, or from the
/// stored copy when the tree has not loaded yet; a hero neither knows is searched by name
/// under Skins instead. The sort, mature and outdated choices are kept; other filters clear.
pub fn open_for_hero(st: &mut State, hero: &str) {
    st.browse_view.tab = Tab::Mods;
    let loaded = categories_of(st, BrowseSection::Mods).and_then(|list| hero_skins(list, hero));
    let (skins, sub) = match loaded {
        Some((skins, Some(sub))) => (skins, Some(sub)),
        other => serde_json::from_str::<Vec<BrowseCategory>>(FIXTURE_CATEGORIES)
            .ok()
            .and_then(|list| hero_skins(&list, hero))
            .filter(|(_, sub)| sub.is_some())
            .or(other)
            .unwrap_or((Pick { id: SKINS_CATEGORY, name: "Skins".into() }, None)),
    };
    let search = if sub.is_some() { String::new() } else { hero.trim().to_string() };
    let search_changed = st.browse_view.applied_search != search;
    st.browse_query = TextEdit::new(&search);
    st.browse_view.applied_search = search;
    st.browse_view.search_edited = None;
    let before = st.browse_view.filters.clone();
    set_filters(st, None, move |f| {
        *f = Filters {
            section: BrowseSection::Mods,
            category: Some(skins),
            sub,
            order: f.order,
            mature: f.mature,
            hide_outdated: f.hide_outdated,
            ..Filters::default()
        }
    });
    if st.browse_view.filters == before && search_changed && !st.demo {
        crate::data::start_browse_fetch(st, 1);
    }
    st.go(Screen::Browse);
}

/// GameBanana's Skins root under Mods.
const SKINS_CATEGORY: u64 = 33295;

/// The Skins root and, when it has one, the child category named for `hero`. Names compare
/// on letters and digits only, with `&` read as "and", so "Mo & Krill" finds "Mo and Krill".
pub(crate) fn hero_skins(list: &[BrowseCategory], hero: &str) -> Option<(Pick, Option<Pick>)> {
    let fold = |name: &str| -> String {
        name.replace('&', "and")
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let wanted = fold(hero);
    let root = list
        .iter()
        .find(|root| root.id == SKINS_CATEGORY)
        .or_else(|| list.iter().find(|root| root.name.eq_ignore_ascii_case("Skins")))?;
    let sub = root
        .children
        .iter()
        .find(|child| !wanted.is_empty() && fold(&child.name) == wanted)
        .map(|child| Pick { id: child.id, name: child.name.clone() });
    Some((Pick { id: root.id, name: root.name.clone() }, sub))
}

#[cfg(test)]
mod open_for_hero_tests {
    use super::*;

const FIXTURE_CATEGORIES: &str = include_str!("../../../fixtures/browse/categories.json");
/// Haze under Skins on GameBanana.
#[cfg(test)]
const HAZE: u64 = 33311;

    #[test]
    fn the_stored_tree_finds_heroes_by_folded_name() {
        let list: Vec<BrowseCategory> = serde_json::from_str(FIXTURE_CATEGORIES).unwrap();
        let (skins, haze) = hero_skins(&list, "Haze").unwrap();
        assert_eq!(skins.id, SKINS_CATEGORY);
        assert_eq!(haze.unwrap().id, HAZE);
        let (_, krill) = hero_skins(&list, "Mo and Krill").unwrap();
        assert_eq!(krill.unwrap().name, "Mo & Krill");
        let (_, talon) = hero_skins(&list, "grey talon").unwrap();
        assert!(talon.is_some());
        let (_, nobody) = hero_skins(&list, "Nobody").unwrap();
        assert!(nobody.is_none());
        assert!(hero_skins(&[], "Haze").is_none());
    }
}

