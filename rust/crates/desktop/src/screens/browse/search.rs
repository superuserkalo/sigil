//! Browse search over the local catalog index (ADR 0018).
//!
//! When an index is on disk, the text typed in the Browse search field is matched on the PC as
//! it is typed: no request per keystroke, and the text goes nowhere. The index holds ids and
//! titles only, so the cards (picture, likes, author) come from the saved catalog, from rows
//! fetched earlier, or, once typing pauses, from one `Mod/Multi` request by id for the top
//! matches that are still missing (ids only, never the text). Results show in match order;
//! every grid filter still applies.
//!
//! GameBanana's own title search stays the answer when there is no index yet, outside the
//! Mods section (the index covers mods only), with the Featured filter (a row read by id does
//! not say whether it was featured), and when offline or after a failed card read with no
//! card known for the matches.
//!
//! The index is loaded on a worker at start and refreshed from Sigil's Worker at most every
//! three hours (`deadlock_core::catalog_index`), never while offline. That download carries
//! nothing about the player and is not behind a consent switch (owner decision, 2026-11-25).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use deadlock_core::catalog_index::{self as ci, CachedCard, CatalogIndex, Matcher, Refresh};

use super::*;

/// Matches the matcher returns for one query.
const MAX_HITS: usize = 1_000;
/// Wait before asking the Worker again after a network failure.
const RETRY_AFTER: Duration = Duration::from_secs(15 * 60);

/// A loaded index, ready to match.
pub struct Ready {
    pub index: Arc<CatalogIndex>,
    pub matcher: Arc<Matcher>,
}

impl Ready {
    pub fn new(index: CatalogIndex) -> Ready {
        let matcher = Arc::new(Matcher::new(&index));
        Ready {
            index: Arc::new(index),
            matcher,
        }
    }
}

/// What the start-up read found on disk.
pub struct Loaded {
    ready: Option<Ready>,
    cards: HashMap<u64, CachedCard>,
    checked_ms: u64,
}

/// Rows a `Mod/Multi` read returned, and the ids it asked for.
pub struct CardsLoad {
    asked: Vec<u64>,
    rows: Vec<deadlock_core::gamebanana::BrowseMod>,
}

/// Local search state (`View::search`).
#[derive(Default)]
pub struct LocalSearch {
    pub ready: Option<Ready>,
    /// The start-up read of the index and the saved card rows has finished.
    pub loaded: bool,
    pub load_job: Job<Loaded>,
    pub refresh_job: Job<Option<Ready>>,
    /// When the Worker may be asked next.
    pub next_refresh: Option<Instant>,
    /// Card rows read by id, kept on disk.
    pub cards: HashMap<u64, CachedCard>,
    pub cards_job: Job<CardsLoad>,
    pub cards_save_job: Job<()>,
    pub cards_dirty: bool,
    /// Ids GameBanana did not return (gone, withheld, private) this session.
    pub gone: HashSet<u64>,
    /// When a card read last failed: no new read for a while, and a match with no known card
    /// goes back to GameBanana's title search.
    pub cards_failed: Option<Instant>,
    /// When the typed text last changed while matched locally: the card read and the product
    /// data record wait for the same pause GameBanana's search waits for.
    pub settle: Option<Instant>,
    /// The shown page has matches whose cards are not read yet.
    pub missing: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn offline(s: &State) -> bool {
    s.prefs.offline || deadlock_core::net::is_offline()
}

fn can_fetch(s: &State) -> bool {
    !s.demo && !rune::is_headless() && !offline(s)
}

/// The filters local search can answer: the Mods catalog, not a saved or installed list, not
/// a custom source, not Featured.
fn eligible(s: &State) -> bool {
    let v = &s.browse_view;
    let f = &v.filters;
    v.search.ready.is_some()
        && v.sources.url.is_none()
        && matches!(v.tab, Tab::Mods | Tab::Maps)
        && f.section == BrowseSection::Mods
        && !f.favorites
        && !f.for_you
        && !f.rising
        && !f.featured
        && f.owned != Owned::Installed
}

/// Text local search answers: words to match, and not a pasted id or link.
fn usable(text: &str) -> bool {
    !ci::normalize(text).is_empty() && pasted_submission(text).is_none()
}

/// True when the typed text should be matched locally at once, without the typing pause.
pub(crate) fn instant(s: &State) -> bool {
    eligible(s) && usable(s.browse_query.text.trim())
}

/// True while the grid shows local matches for the applied search.
pub(crate) fn active(s: &State) -> bool {
    eligible(s) && usable(s.browse_view.applied_search.trim())
}

/// A card for an id: a row read by id first (fresher), then the saved catalog.
fn known_cards(s: &State, ids: &[u64]) -> HashMap<u64, BrowseCard> {
    let mut out: HashMap<u64, BrowseCard> = HashMap::new();
    let wanted: HashSet<u64> = ids.iter().copied().collect();
    if let Some(mirror) = s.browse_view.mirror.as_ref() {
        for entry in &mirror.entries {
            if entry.model == "Mod" && wanted.contains(&entry.id) {
                out.insert(entry.id, mirror_card(entry));
            }
        }
    }
    for id in ids {
        if let Some(cached) = s.browse_view.search.cards.get(id) {
            out.insert(*id, card_from_browse(&cached.row));
        }
    }
    out
}

/// The matches for the applied search, best first, with the category pick applied from the
/// index itself so the count is right before the cards arrive.
fn hits(s: &State, text: &str) -> Vec<u64> {
    let Some(ready) = s.browse_view.search.ready.as_ref() else {
        return Vec::new();
    };
    // Likes break ties when the card is known.
    let likes: HashMap<u64, u64> = s
        .browse_view
        .search
        .cards
        .iter()
        .map(|(id, card)| (*id, card.row.likes))
        .collect();
    let found = ready.matcher.search(text, MAX_HITS, |id| likes.get(&id).copied());
    let f = &s.browse_view.filters;
    let category = f.category.as_ref().map(|pick| ci::normalize(&pick.name));
    let by_id: HashMap<u64, &ci::IndexMod> = if category.is_some() {
        ready.index.mods.iter().map(|m| (m.id, m)).collect()
    } else {
        HashMap::new()
    };
    found
        .into_iter()
        .map(|hit| hit.id)
        .filter(|id| !s.browse_view.search.gone.contains(id))
        .filter(|id| {
            let Some(category) = category.as_deref() else {
                return true;
            };
            let inside = by_id
                .get(id)
                .is_some_and(|m| ready.index.strings_of(m, ci::matcher::CLS_CATEGORY).any(|c| c == category));
            inside != f.exclude
        })
        .collect()
}

/// The grid page for the applied search, matched locally, or None when GameBanana's title
/// search should answer. Starts the card read for missing matches unless typing is still
/// going on.
pub fn local_page(st: &mut State, page: usize) -> Option<BrowsePageView> {
    if !active(st) {
        return None;
    }
    let text = st.browse_view.applied_search.trim().to_string();
    let hits = hits(st, &text);
    let per = PAGE_SIZE;
    let mut page = page.max(1);
    let paged = st.browse_view.paged;
    let known = known_cards(st, &hits);
    let missing_in = |range: std::ops::Range<usize>| -> Vec<u64> {
        hits[range.start.min(hits.len())..range.end.min(hits.len())]
            .iter()
            .copied()
            .filter(|id| !known.contains_key(id))
            .collect()
    };
    // A later page waits until the cards before it are known, so paging never runs ahead of
    // the reads.
    if !paged && page > 1 && !missing_in(0..(page - 1) * per).is_empty() {
        page -= 1;
    }
    let window = if paged { (page - 1) * per..page * per } else { 0..page * per };
    let missing = missing_in(window.clone());
    st.browse_view.search.missing = !missing.is_empty();
    let any_known = hits.iter().any(|id| known.contains_key(id));
    let failed_recently = st.browse_view.search.cards_failed.is_some_and(|at| at.elapsed() < RETRY_AFTER);
    // Offline, or after a failed read, with no card for any match: GameBanana's title search
    // (or the saved catalog when offline) answers as before.
    if !any_known && !hits.is_empty() && (offline(st) || failed_recently) {
        return None;
    }
    if !missing.is_empty() && st.browse_view.search.settle.is_none() && !failed_recently {
        start_cards(st, &missing);
    }
    let f = &st.browse_view.filters;
    let mods: Vec<BrowseCard> = hits[window.start.min(hits.len())..window.end.min(hits.len())]
        .iter()
        .filter_map(|id| known.get(id))
        .filter(|card| {
            // The server applies these to a title search; here they apply to the cards.
            let category_ok = f.exclude || f.category.as_ref().is_none_or(|pick| in_category(card, &pick.name));
            let creator_ok = f.creator.as_ref().is_none_or(|pick| card.author_id == pick.id);
            category_ok && creator_ok
        })
        .map(clone_card)
        .collect();
    let key = query_key(st, &current_query(st));
    Some(BrowsePageView {
        key,
        page,
        per_page: per,
        total: hits.len(),
        complete: page * per >= hits.len(),
        page_count: hits.len().div_ceil(per).max(1),
        mods,
    })
}

/// True while the grid waits for the cards of local matches: the grid shows its loading
/// state instead of "Nothing matches".
pub(crate) fn waiting(s: &State) -> bool {
    let v = &s.browse_view.search;
    active(s) && v.missing && (v.cards_job.is_running() || v.settle.is_some())
}

/// Read the cards for up to 50 ids in one `Mod/Multi` request: ids only, never the text.
fn start_cards(st: &mut State, ids: &[u64]) {
    if !can_fetch(st) || st.browse_view.search.cards_job.is_running() {
        return;
    }
    let ids: Vec<u64> = ids.iter().copied().take(deadlock_core::gamebanana::MULTI_MAX_IDS).collect();
    if ids.is_empty() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.search.cards_job.start(wake, move || {
        let rows = deadlock_core::gamebanana::fetch_mods_by_id(&ids).map_err(|e| e.to_string())?;
        Ok(CardsLoad { asked: ids, rows })
    });
}

/// Show the local page for the applied search in the grid. False when local search does not
/// answer, so the caller reads GameBanana.
pub fn show(st: &mut State, page: usize) -> bool {
    let Some(view) = local_page(st, page) else {
        return false;
    };
    st.browse.loading = false;
    st.browse.error = None;
    st.browse_view.from_mirror = false;
    st.browse_view.in_flight = None;
    st.browse.page = Some(view);
    true
}

/// The typed text changed and local search answers it: apply it now, without the pause.
pub(crate) fn apply_typed(st: &mut State, tx: &mut Tx) {
    let typed = st.browse_query.text.trim().to_string();
    st.browse_view.search_edited = None;
    st.browse_view.applied_search = typed;
    st.browse_view.search.settle = Some(Instant::now());
    tx.wake_after(SEARCH_PAUSE);
    refresh_creator_hits(st);
    tx.scroll_to_instant("browse-scroll", 0.0, 0.0);
    st.browse_view.visible = 0..0;
    st.browse_view.auto_streak = 0;
    if !show(st, 1) {
        // Nothing local after all (no card known while offline): the usual read.
        st.browse_view.search.settle = None;
        st.browse_view.refreshing = true;
        crate::data::start_browse_fetch(st, 1);
    }
}

/// Load the index at start, refresh it when due, take finished reads, and act once typing
/// pauses. Called from the Browse poll on every frame.
pub(crate) fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    let quiet = st.demo || rune::is_headless();
    if !quiet && !st.browse_view.search.loaded && !st.browse_view.search.load_job.is_running() {
        let wake = st.wake.clone();
        st.browse_view.search.load_job.start(wake, || {
            let dir = crate::data::data_dir();
            Ok(Loaded {
                ready: ci::load(&dir).map(Ready::new),
                cards: ci::read_cards(&dir),
                checked_ms: ci::read_meta(&dir).checked_ms,
            })
        });
    }
    if let Some(result) = st.browse_view.search.load_job.try_take() {
        st.browse_view.search.loaded = true;
        if let Ok(loaded) = result {
            let have = loaded.ready.is_some();
            st.browse_view.search.ready = loaded.ready;
            st.browse_view.search.cards = loaded.cards;
            let age = now_ms().saturating_sub(loaded.checked_ms);
            let wait = if have && loaded.checked_ms > 0 { ci::REFRESH_EVERY_MS.saturating_sub(age) } else { 0 };
            st.browse_view.search.next_refresh = Some(Instant::now() + Duration::from_millis(wait));
            changed |= index_arrived(st);
        }
    }
    let due = st.browse_view.search.next_refresh.is_some_and(|at| Instant::now() >= at);
    if !quiet && due && !offline(st) && !st.browse_view.search.refresh_job.is_running() {
        st.browse_view.search.next_refresh = None;
        let wake = st.wake.clone();
        st.browse_view.search.refresh_job.start(wake, || {
            let dir = crate::data::data_dir();
            match ci::refresh(&dir, crate::updater::VERSION, now_ms()) {
                Ok(Refresh::Updated(index)) => Ok(Some(Ready::new(index))),
                Ok(Refresh::Unchanged) => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        });
    }
    if let Some(result) = st.browse_view.search.refresh_job.try_take() {
        let wait = match result {
            Ok(Some(ready)) => {
                st.browse_view.search.ready = Some(ready);
                changed |= index_arrived(st);
                Duration::from_millis(ci::REFRESH_EVERY_MS)
            }
            Ok(None) => Duration::from_millis(ci::REFRESH_EVERY_MS),
            Err(error) => {
                crate::screens::settings::log::info(format!("The search index was not refreshed: {error}"));
                RETRY_AFTER
            }
        };
        st.browse_view.search.next_refresh = Some(Instant::now() + wait);
    }
    if let Some(result) = st.browse_view.search.cards_job.try_take() {
        match result {
            Ok(load) => {
                let fetched = gamebanana::now_unix();
                let returned: HashSet<u64> = load.rows.iter().map(|row| row.id).collect();
                for id in load.asked.iter().filter(|id| !returned.contains(id)) {
                    st.browse_view.search.gone.insert(*id);
                }
                for row in load.rows {
                    st.browse_view.search.cards.insert(row.id, CachedCard { fetched, row });
                }
                st.browse_view.search.cards_failed = None;
                st.browse_view.search.cards_dirty = true;
            }
            Err(error) => {
                crate::screens::settings::log::error(format!("Browse search could not read mod cards: {error}"));
                st.browse_view.search.cards_failed = Some(Instant::now());
            }
        }
        if active(st) {
            let page = st.browse.page.as_ref().map(|page| page.page).unwrap_or(1);
            if !show(st, page) {
                st.browse_view.refreshing = true;
                crate::data::start_browse_fetch(st, 1);
            }
        }
        changed = true;
    }
    if !quiet && st.browse_view.search.cards_dirty && !st.browse_view.search.cards_save_job.is_running() {
        st.browse_view.search.cards_dirty = false;
        let cards = st.browse_view.search.cards.clone();
        let wake = st.wake.clone();
        st.browse_view
            .search
            .cards_save_job
            .start(wake, move || ci::write_cards(&crate::data::data_dir(), &cards).map_err(|e| e.to_string()));
    }
    let _ = st.browse_view.search.cards_save_job.try_take();
    if let Some(at) = st.browse_view.search.settle {
        let waited = at.elapsed();
        if waited < SEARCH_PAUSE {
            tx.wake_after(SEARCH_PAUSE - waited);
        } else {
            st.browse_view.search.settle = None;
            changed |= settled(st);
        }
    }
    changed
}

/// Typing paused on a local search: read the missing cards, and record the query for product
/// data exactly where a GameBanana search records it (its own consent gate decides).
fn settled(st: &mut State) -> bool {
    if !active(st) {
        return false;
    }
    let page = st.browse.page.as_ref().map(|page| page.page).unwrap_or(1);
    show(st, page);
    let total = st.browse.page.as_ref().map(|page| page.total).unwrap_or(0);
    crate::screens::settings::product::browse_query(st, total);
    true
}

/// A new index landed: a search on screen is matched against it.
fn index_arrived(st: &mut State) -> bool {
    if !active(st) {
        return false;
    }
    st.browse_view.refreshing = true;
    show(st, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::catalog_index::{IndexMod, PACK, SUPPORTED_VERSION};
    use deadlock_core::gamebanana::BrowseMod;

    fn row(id: u64, name: &str, likes: u64) -> BrowseMod {
        BrowseMod {
            id,
            name: name.to_string(),
            profile_url: format!("https://gamebanana.com/mods/{id}"),
            model: "Mod".into(),
            author: "someone".into(),
            category: "Skins".into(),
            subcategory: "Kelvin".into(),
            sub_category_id: 0,
            version: None,
            updated_at: String::new(),
            likes,
            views: 0,
            has_files: true,
            thumbnail_url: None,
            added_ts: 1_790_000_000,
            updated_ts: 1_790_000_000,
            nsfw: false,
            audio_url: None,
            featured: false,
            comments: 0,
            author_id: 7,
            obsolete: false,
        }
    }

    /// Three mods; only "Mei as Kelvin" knows it is from Overwatch, through a verified term.
    fn index() -> CatalogIndex {
        let strings = vec!["overwatch".to_string(), "skins".to_string(), "ice".to_string()];
        let fields = |extra: &[u32]| {
            let mut list = vec![PACK + 1];
            list.extend_from_slice(extra);
            list
        };
        CatalogIndex {
            version: SUPPORTED_VERSION,
            built: 1,
            strings,
            mods: vec![
                IndexMod { id: 1, title: "Mei as Kelvin".into(), updated: 1, fields: fields(&[4, 2 * PACK]) },
                IndexMod { id: 2, title: "Kelvin winter coat".into(), updated: 1, fields: fields(&[2 * PACK]) },
                IndexMod { id: 3, title: "Haze recolor".into(), updated: 1, fields: fields(&[]) },
            ],
        }
    }

    fn state(with_index: bool) -> State {
        let mut st = State::default();
        // No worker reads in a unit test.
        st.demo = true;
        st.browse_view.tab = Tab::Mods;
        if with_index {
            st.browse_view.search.ready = Some(Ready::new(index()));
        }
        for (id, name, likes) in [(1, "Mei as Kelvin", 5), (2, "Kelvin winter coat", 50), (3, "Haze recolor", 1)] {
            st.browse_view.search.cards.insert(id, CachedCard { fetched: 1, row: row(id, name, likes) });
        }
        st
    }

    fn search(st: &mut State, text: &str) -> Option<Vec<u64>> {
        st.browse_query = TextEdit::new(text);
        st.browse_view.applied_search = text.to_string();
        local_page(st, 1).map(|page| page.mods.iter().map(|card| card.id).collect())
    }

    #[test]
    fn a_word_only_in_a_generated_term_finds_the_mod_locally() {
        let mut st = state(true);
        assert!(instant(&{
            let mut s = state(true);
            s.browse_query = TextEdit::new("overwatch");
            s
        }));
        assert_eq!(search(&mut st, "overwatch"), Some(vec![1]));
        // Title words first, in match order; the tie between equal titles breaks by likes.
        assert_eq!(search(&mut st, "kelvin"), Some(vec![2, 1]));
        assert_eq!(search(&mut st, "ice"), Some(vec![2, 1]));
        // Nothing matches: an empty local page, so the grid says so.
        assert_eq!(search(&mut st, "unicorn"), Some(vec![]));
        let page = local_page(&mut st, 1).unwrap();
        assert_eq!((page.total, page.complete), (0, true));
    }

    #[test]
    fn the_grid_filters_still_apply_to_local_results() {
        let mut st = state(true);
        st.browse_view.filters.creator = Some(Pick { id: 99, name: "other".into() });
        assert_eq!(search(&mut st, "kelvin"), Some(vec![]));
        st.browse_view.filters.creator = None;
        st.browse_view.filters.category = Some(Pick { id: 33295, name: "Skins".into() });
        assert_eq!(search(&mut st, "kelvin"), Some(vec![2, 1]));
        st.browse_view.filters.category = Some(Pick { id: 1, name: "HUD".into() });
        assert_eq!(search(&mut st, "kelvin"), Some(vec![]));
        st.browse_view.filters.category = None;
        // Hidden mods leave the shown cards through the grid's own filter.
        st.browse_view.hidden_mods.insert(2);
        assert_eq!(search(&mut st, "kelvin"), Some(vec![2, 1]));
        st.browse.page = local_page(&mut st, 1);
        let shown: Vec<u64> = shown_indices(&st)
            .into_iter()
            .map(|i| st.browse.page.as_ref().unwrap().mods[i].id)
            .collect();
        assert_eq!(shown, vec![1]);
    }

    #[test]
    fn without_an_index_gamebanana_answers_as_before() {
        let mut st = state(false);
        assert_eq!(search(&mut st, "overwatch"), None);
        st.browse_query = TextEdit::new("overwatch");
        assert!(!instant(&st));
        // GameBanana's title search is what the query asks for.
        assert_eq!(current_query(&st).search, "overwatch");
    }

    #[test]
    fn gamebanana_answers_where_the_index_cannot() {
        let mut st = state(true);
        // A pasted id opens the mod instead.
        assert_eq!(search(&mut st, "741478"), None);
        st.browse_view.filters.featured = true;
        assert_eq!(search(&mut st, "kelvin"), None);
        st.browse_view.filters.featured = false;
        st.browse_view.tab = Tab::Sounds;
        st.browse_view.filters.section = BrowseSection::Sounds;
        assert_eq!(search(&mut st, "kelvin"), None);
    }

    #[test]
    fn offline_with_no_card_for_any_match_falls_back() {
        let mut st = state(true);
        st.browse_view.search.cards.clear();
        st.prefs.offline = true;
        assert_eq!(search(&mut st, "overwatch"), None);
        // With a card known, the local result shows offline too.
        st.browse_view.search.cards.insert(1, CachedCard { fetched: 1, row: row(1, "Mei as Kelvin", 5) });
        assert_eq!(search(&mut st, "overwatch"), Some(vec![1]));
    }

    #[test]
    fn missing_cards_count_but_do_not_show_until_read() {
        let mut st = state(true);
        st.browse_view.search.cards.remove(&1);
        let page = {
            st.browse_query = TextEdit::new("kelvin");
            st.browse_view.applied_search = "kelvin".into();
            local_page(&mut st, 1).unwrap()
        };
        assert_eq!(page.total, 2);
        assert_eq!(page.mods.iter().map(|c| c.id).collect::<Vec<_>>(), vec![2]);
    }
}
