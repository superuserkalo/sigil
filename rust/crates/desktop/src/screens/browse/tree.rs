//! The category tree beyond two levels (DMM 2.4.0 `cd622cb`): a hero's styles under Skins >
//! Haze, the NPC kinds under Model Replacement. The first two levels load with the section;
//! the third is read after, on a worker, so the filter bar shows at once. Also the breadcrumb
//! a third-level pick shows in the chip row, the subtree a pick holds, and the hero pick that
//! follows the player from section to section.

use deadlock_core::gamebanana::{self, BrowseCategory, BrowseSection};
use rune::{El, Tx, row, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{LABEL, Ui};

use super::{Filters, Pick, Tab, categories_of, set_filters};

const DE: &[(&str, &str)] = &[
    ("Every {} mod", "Jeder Mod unter {}"),
    ("Scroll the heroes", "Helden durchblättern"),
    ("Trending this week", "Diese Woche im Trend"),
    ("Popular this month", "Beliebt diesen Monat"),
    ("All-time favorites", "Beliebteste aller Zeiten"),
    ("Browse by hero", "Nach Held stöbern"),
    ("See all", "Alle ansehen"),
    ("For you", "Für dich"),
    ("Rising", "Im Aufwind"),
    ("Rising fast", "Steigt schnell"),
    ("Something different", "Mal etwas anderes"),
    ("Not interested: show fewer like this", "Kein Interesse: weniger davon zeigen"),
    ("Not interested", "Kein Interesse"),
    ("Because you play {}", "Weil du {} spielst"),
    ("More from {}", "Mehr von {}"),
    ("More {}", "Mehr {}"),
    ("Picked on this PC from your heroes, mods and favorites", "Auf diesem PC ausgewählt: nach deinen Helden, Mods und Favoriten"),
    ("All mods", "Alle Mods"),
    ("Trending today", "Heute im Trend"),
    ("Author", "Autor"),
    ("Top 10 this week", "Top 10 dieser Woche"),
    ("Top 10 of the last 14 days", "Top 10 der letzten 14 Tage"),
    ("Top 10 this month", "Top 10 dieses Monats"),
    ("Top 10 of all time", "Top 10 aller Zeiten"),
    ("New this week", "Neu diese Woche"),
    ("New mods", "Neue Mods"),
    ("Trending in {}", "Im Trend: {}"),
    ("This week in {}", "Diese Woche in {}"),
    ("Trending", "Im Trend"),
    ("Details", "Details"),
    ("NSFW", "NSFW"),
    ("Category", "Kategorie"),
    ("Filters", "Filter"),
    ("More", "Mehr"),
    ("View options", "Ansicht"),
    ("Favorites only", "Nur Favoriten"),
    ("Featured only", "Nur Empfohlene"),
    ("Hide outdated", "Veraltete ausblenden"),
    ("Layout", "Layout"),
    ("Cards", "Karten"),
    ("Numbered pages", "Seitenzahlen"),
    ("Paste mod links", "Mod-Links einfügen"),
    ("Hidden items", "Ausgeblendete Einträge"),
    ("Ignored updates", "Ignorierte Updates"),
    ("Date", "Datum"),
    ("Any", "Alle"),
    ("Yes", "Ja"),
    ("No", "Nein"),
    ("Include", "Einschließen"),
    ("Exclude", "Ausschließen"),
    ("Setting", "Einstellung"),
    ("Grid", "Raster"),
    ("List", "Liste"),
    ("All categories", "Alle Kategorien"),
    ("About", "Info"),
    ("Before Jan 22", "Vor dem 22. Jan."),
    ("Next page", "Nächste Seite"),
    ("Previous page", "Vorherige Seite"),
    ("Close search", "Suche schließen"),
    ("Saved copy", "Gespeicherte Kopie"),
    ("Sort: {}", "Sortierung: {}"),
    ("{} likes", "{} Likes"),
    ("Small", "Klein"),
    ("All", "Alle"),
    ("Collections", "Sammlungen"),
    ("Maps", "Karten"),
    ("Nothing new here yet", "Hier gibt es noch nichts Neues"),
    ("Popular now", "Gerade beliebt"),
    ("Newest", "Neueste"),
    ("Recently updated", "Zuletzt aktualisiert"),
    ("Most liked", "Meiste Likes"),
    ("Most downloaded", "Meiste Downloads"),
    ("Most viewed", "Meiste Aufrufe"),
    ("Most discussed", "Meist diskutiert"),
    ("Latest comment", "Neuester Kommentar"),
    ("Name, A to Z", "Name, A bis Z"),
    ("Past day", "Letzter Tag"),
    ("Past week", "Letzte Woche"),
    ("Past month", "Letzter Monat"),
    ("Past 3 months", "Letzte 3 Monate"),
    ("Added", "Hinzugefügt"),
    ("Updated", "Aktualisiert"),
    ("Not installed", "Nicht installiert"),
    ("As in Settings", "Wie in den Einstellungen"),
    ("Hide NSFW", "Nicht jugendfreie ausblenden"),
    ("NSFW only", "Nur nicht jugendfreie"),
    ("Custom range", "Eigener Zeitraum"),
    ("GameBanana did not respond", "GameBanana hat nicht geantwortet"),
    ("Nothing loaded yet", "Noch nichts geladen"),
    ("Check your connection, then try again.", "Prüfe deine Verbindung und versuche es dann erneut."),
    ("Try again", "Erneut versuchen"),
    ("Show everything", "Alles zeigen"),
    ("Clear filters", "Filter löschen"),
    ("Load more", "Mehr laden"),
    ("No favorites here yet", "Noch keine Favoriten"),
    ("Save a mod with the star on its card.", "Speichere einen Mod mit dem Stern auf seiner Karte."),
    ("No installed GameBanana mods", "Keine installierten GameBanana-Mods"),
    ("Mods you install from Browse show up here.", "Mods, die du über Entdecken installierst, erscheinen hier."),
    ("Nothing matches", "Keine Treffer"),
    ("Try fewer filters or another search.", "Versuche weniger Filter oder eine andere Suche."),
    ("Nothing to show", "Nichts anzuzeigen"),
    ("Loading", "Lädt"),
    ("Page {} of {}", "Seite {} von {}"),
    ("mod", "Mod"),
    ("mods", "Mods"),
    ("item", "Eintrag"),
    ("items", "Einträge"),
    ("sound", "Sound"),
    ("sounds", "Sounds"),
    ("WiP", "WiP"),
    ("WiPs", "WiPs"),
    ("favorite", "Favorit"),
    ("favorites", "Favoriten"),
    ("Creators", "Ersteller"),
    ("Loading more mods", "Weitere Mods werden geladen"),
    ("The next page did not load.", "Die nächste Seite wurde nicht geladen."),
    ("That is every match.", "Das sind alle Treffer."),
    ("That is the whole catalog.", "Das ist der ganze Katalog."),
    ("Unknown author", "Unbekannter Autor"),
    ("by {}", "von {}"),
    ("Added {}", "Hinzugefügt {}"),
    ("Remove from favorites", "Aus Favoriten entfernen"),
    ("Save to favorites", "Zu Favoriten hinzufügen"),
    ("Copy link", "Link kopieren"),
    ("Hide this mod", "Diesen Mod ausblenden"),
    ("Pop out", "Als Fenster öffnen"),
    ("Dock to the side", "Seitlich andocken"),
    ("Close", "Schließen"),
    ("Link copied.", "Link kopiert."),
    ("Stop the sample", "Hörprobe stoppen"),
    ("Play the sample", "Hörprobe abspielen"),
    ("This mod has no downloadable files.", "Dieser Mod hat keine herunterladbaren Dateien."),
    ("Archived files", "Archivierte Dateien"),
    ("{} download", "{} Download"),
    ("{} downloads", "{} Downloads"),
    ("Requirements", "Voraussetzungen"),
    ("Changelog", "Änderungen"),
    ("Comments", "Kommentare"),
    ("Videos", "Videos"),
    ("Search mods", "Mods suchen"),
    ("Medium", "Mittel"),
    ("Large", "Groß"),
];

/// This file's German for `text`, for width checks in tests.
#[cfg(test)]
pub fn german(text: &str) -> &str {
    DE.iter().find(|(en, _)| *en == text).map(|(_, de)| *de).unwrap_or(text)
}

/// `tr` with this file's German strings first.
pub fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some((_, de)) = DE.iter().find(|(en, _)| *en == text)
    {
        return de;
    }
    i18n::tr(text)
}

#[derive(Default)]
pub struct Tree {
    /// The third level being read for a section.
    deepen: Job<(BrowseSection, Vec<BrowseCategory>)>,
    /// Sections whose third level was asked for this run, read or failed.
    deepened: Vec<BrowseSection>,
}

/// Read the third level of `section`'s tree once its first two are in.
pub fn start_deepen(st: &mut State, section: BrowseSection) {
    if st.demo || rune::is_headless() || st.prefs.offline {
        return;
    }
    if st.browse_view.tree.deepened.contains(&section) || st.browse_view.tree.deepen.is_running() {
        return;
    }
    let Some(list) = categories_of(st, section).map(<[BrowseCategory]>::to_vec) else {
        return;
    };
    st.browse_view.tree.deepened.push(section);
    let wake = st.wake.clone();
    st.browse_view
        .tree
        .deepen
        .start(wake, move || Ok((section, gamebanana::deepen_categories(section, list))));
}

/// Swap in a deepened tree. A failed read keeps the two levels already shown.
pub fn poll(st: &mut State) -> bool {
    let Some(Ok((section, list))) = st.browse_view.tree.deepen.try_take() else {
        return false;
    };
    if let Some((_, slot)) = st.browse_view.categories.iter_mut().find(|(owner, _)| *owner == section) {
        *slot = list;
    }
    // Another section may be waiting for its turn.
    let current = st.browse_view.filters.section;
    start_deepen(st, current);
    true
}

/// The picks from the section's root to `id`, root first. Empty when the tree lacks it.
pub fn path(s: &State, section: BrowseSection, id: u64) -> Vec<Pick> {
    categories_of(s, section)
        .map(|roots| {
            gamebanana::category_path(roots, id)
                .into_iter()
                .map(|node| Pick { id: node.id, name: node.name.clone() })
                .collect()
        })
        .unwrap_or_default()
}

/// `id` and every category under it. Empty when the tree lacks it.
pub fn subtree(s: &State, section: BrowseSection, id: u64) -> Vec<u64> {
    categories_of(s, section)
        .and_then(|roots| roots.iter().find_map(|root| root.find(id)))
        .map(BrowseCategory::subtree_ids)
        .unwrap_or_default()
}

/// False while a category under `id` has children that have not been read: the third level is
/// still loading or its read failed, so a row's leaf may belong to the pick without being listed.
pub fn subtree_complete(s: &State, section: BrowseSection, id: u64) -> bool {
    fn whole(node: &BrowseCategory) -> bool {
        (node.child_count == 0 || !node.children.is_empty()) && node.children.iter().all(whole)
    }
    categories_of(s, section)
        .and_then(|roots| roots.iter().find_map(|root| root.find(id)))
        .is_none_or(whole)
}

/// Every category of the section named for `hero`, with what sits under it: Skins > Haze and
/// its styles on Mods, a hero's folder on Sounds when GameBanana files one.
pub fn hero_ids(s: &State, section: BrowseSection, hero: &str) -> Vec<u64> {
    fn walk(node: &BrowseCategory, hero: &str, ids: &mut Vec<u64>) {
        if deadlock_core::catalog_mirror::fold(&node.name) == hero {
            ids.extend(node.subtree_ids());
            return;
        }
        for child in &node.children {
            walk(child, hero, ids);
        }
    }
    let hero = deadlock_core::catalog_mirror::fold(hero);
    let mut ids = Vec::new();
    if hero.is_empty() {
        return ids;
    }
    for root in categories_of(s, section).unwrap_or_default() {
        walk(root, &hero, &mut ids);
    }
    ids
}

/// The hero the filters hold: a hero name match, or the hero branch of a pick under Skins (a
/// style under Haze still means Haze).
pub fn picked_hero(s: &State, f: &Filters) -> Option<String> {
    if let Some(hero) = &f.hero {
        return Some(hero.clone());
    }
    let skins = f.category.as_ref().is_some_and(|pick| pick.name.eq_ignore_ascii_case("Skins"));
    let sub = f.sub.as_ref().filter(|_| skins)?;
    let path = path(s, f.section, sub.id);
    Some(path.get(1).map_or_else(|| sub.name.clone(), |hero| hero.name.clone()))
}

/// Carry a hero pick into a new tab's filters: Skins > hero on Mods, a name match on Sounds,
/// WiPs and All. Maps and Collections have no hero.
pub fn follow_hero(s: &State, hero: Option<String>, tab: Tab, f: &mut Filters) {
    let Some(hero) = hero else { return };
    match tab {
        Tab::Mods => {
            let sub = super::hero_list(s).into_iter().find(|pick| pick.name.eq_ignore_ascii_case(&hero));
            match (super::skins_pick(s), sub) {
                (Some(skins), Some(sub)) => {
                    f.category = Some(skins);
                    f.sub = Some(sub);
                }
                _ => f.hero = Some(hero),
            }
        }
        Tab::Sounds | Tab::Wips | Tab::All => f.hero = Some(hero),
        Tab::Maps | Tab::Collections => {}
    }
}

/// The categories between the root and the picked sub category, as steps back up the
/// breadcrumb: Skins > *Haze* > Faithful/Vanilla+. Empty when the pick is one level down.
pub fn middle(s: &State) -> Vec<Pick> {
    let f = &s.browse_view.filters;
    let Some(sub) = &f.sub else { return Vec::new() };
    let path = path(s, f.section, sub.id);
    if path.len() < 3 {
        return Vec::new();
    }
    path[1..path.len() - 1].to_vec()
}

/// A breadcrumb step: pressing it widens the pick to that category.
pub fn crumb(u: &Ui, pick: Pick) -> El {
    let label = pick.name.clone();
    row()
        .key(format!("browse-crumb-{}", pick.id))
        .h(28.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(14.0))
        .border(1.0, u.hair_strong)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(crate::screens::settings::i18n::fill(tr("Every {} mod"), &[&label]))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
            let pick = pick.clone();
            set_filters(st, Some(tx), move |f| f.sub = Some(pick));
        })
        .child(text(label).font(LABEL).color(u.prose).a11y_hidden())
}

/// Give Haze two styles in the stored tree, as a deepened read would.
fn stage_third_level(s: &mut State) {
    if let Some((_, roots)) = s.browse_view.categories.iter_mut().find(|(owner, _)| *owner == BrowseSection::Mods)
        && let Some(haze) = roots.iter_mut().find_map(|root| {
            root.children.iter_mut().find(|child| child.name.eq_ignore_ascii_case("Haze"))
        })
    {
        let style = |id: u64, name: &str, count: u64| BrowseCategory { id, name: name.into(), count, ..Default::default() };
        haze.children = vec![style(50617, "Faithful/Vanilla+", 41), style(50618, "Alternate", 17)];
        haze.child_count = 2;
    }
}

/// `browse-tree-menu`: Skins > Haze > Faithful/Vanilla+ picked, the breadcrumb in the chip row
/// and the hero menu open on Haze's styles.
pub fn scene_tree_menu(s: &mut State) {
    super::scenes::scene_with(s, |_| {});
    stage_third_level(s);
    let f = &mut s.browse_view.filters;
    f.category = Some(Pick { id: 33295, name: "Skins".into() });
    f.sub = Some(Pick { id: 50617, name: "Faithful/Vanilla+".into() });
    s.browse_view.menu = Some(super::FilterMenu::Sub);
    // The fixture page stands for these filters.
    let key = super::query_key(s, &super::current_query(s));
    if let Some(page) = s.browse.page.as_mut() {
        page.key = key;
    }
}

/// `browse-tree-crumbs`: the same pick with the menu closed.
pub fn scene_tree_crumbs(s: &mut State) {
    scene_tree_menu(s);
    s.browse_view.menu = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Vec<BrowseCategory> {
        let leaf = |id: u64, name: &str| BrowseCategory { id, name: name.into(), ..Default::default() };
        let haze = BrowseCategory { children: vec![leaf(50617, "Faithful/Vanilla+")], ..leaf(33311, "Haze") };
        let skins = BrowseCategory { children: vec![haze, leaf(33306, "Abrams")], ..leaf(33295, "Skins") };
        let hud = BrowseCategory { children: vec![leaf(44745, "Haze HUD")], ..leaf(1, "HUD") };
        vec![skins, hud]
    }

    #[test]
    fn every_string_has_german() {
        let body = include_str!("tree.rs").split("#[cfg(test)]").next().unwrap();
        for (at, _) in body.match_indices("tr(\"") {
            let rest = &body[at + 4..];
            let literal = &rest[..rest.find('"').unwrap()];
            assert!(DE.iter().any(|(en, _)| *en == literal), "untranslated {literal:?}");
        }
    }

    #[test]
    fn a_third_level_pick_has_a_breadcrumb_and_a_subtree() {
        let mut s = State::default();
        s.browse_view.categories = vec![(BrowseSection::Mods, tree())];
        let names: Vec<String> = path(&s, BrowseSection::Mods, 50617).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Skins", "Haze", "Faithful/Vanilla+"]);
        assert_eq!(subtree(&s, BrowseSection::Mods, 33311), vec![33311, 50617]);
        assert!(subtree(&s, BrowseSection::Sounds, 33311).is_empty());
        s.browse_view.filters.sub = Some(Pick { id: 50617, name: "Faithful/Vanilla+".into() });
        assert_eq!(middle(&s).into_iter().map(|p| p.id).collect::<Vec<_>>(), vec![33311]);
        s.browse_view.filters.sub = Some(Pick { id: 33311, name: "Haze".into() });
        assert!(middle(&s).is_empty());
        assert!(subtree_complete(&s, BrowseSection::Mods, 33311));
        s.browse_view.categories[0].1[0].children[0].children.clear();
        s.browse_view.categories[0].1[0].children[0].child_count = 1;
        assert!(!subtree_complete(&s, BrowseSection::Mods, 33311), "styles not read yet");
        s.browse_view.categories = vec![(BrowseSection::Mods, tree())];
        assert_eq!(hero_ids(&s, BrowseSection::Mods, "haze"), vec![33311, 50617]);
    }

    #[test]
    fn a_hero_follows_the_player_between_sections() {
        let mut s = State::default();
        s.browse_view.categories = vec![(BrowseSection::Mods, tree())];
        let mods = Filters {
            category: Some(Pick { id: 33295, name: "Skins".into() }),
            sub: Some(Pick { id: 33311, name: "Haze".into() }),
            ..Filters::default()
        };
        let hero = picked_hero(&s, &mods);
        assert_eq!(hero.as_deref(), Some("Haze"));
        let style = Filters { sub: Some(Pick { id: 50617, name: "Faithful/Vanilla+".into() }), ..mods.clone() };
        assert_eq!(picked_hero(&s, &style).as_deref(), Some("Haze"), "a style follows as its hero");
        let mut sounds = Filters { section: BrowseSection::Sounds, ..Filters::default() };
        follow_hero(&s, hero.clone(), Tab::Sounds, &mut sounds);
        assert_eq!(sounds.hero.as_deref(), Some("Haze"));
        let mut back = Filters::default();
        follow_hero(&s, picked_hero(&s, &sounds), Tab::Mods, &mut back);
        assert_eq!(back.sub.map(|p| p.id), Some(33311));
        assert_eq!(back.hero, None);
        let mut maps = Filters::default();
        follow_hero(&s, hero, Tab::Maps, &mut maps);
        assert_eq!(maps, Filters::default());
    }
}
