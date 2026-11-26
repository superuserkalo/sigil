//! Where the Stats page is: a stack of places, your own page at the bottom.
//!
//! A place is one player's page on a tab, or one match seen as one player. Opening a player
//! from any list pushes their page; opening a match pushes the match page; Back (the button,
//! mouse Back, Alt+Left, and Escape on a match page) pops one place and puts back its tab and
//! scroll. Changing tab replaces the top place's tab. Visiting a player never writes the
//! tracked list: only Track does.
//!
//! The page holds one subject at a time (`View::snapshot`). The last few subjects left behind
//! are kept in memory, so Back is instant; older ones load again from the response cache.

use std::cell::Cell;

use deadlock_core::stats::social::Social;
use deadlock_core::stats::{KillHeatmap, StatsSnapshot};
use rune::{Key, KeyEvent, Tx};

use crate::app::State;
use crate::jobs::Job;

use super::{Fetched, HeatGrid, Tab};

/// Places kept before the oldest visits are dropped.
pub const DEPTH: usize = 24;
/// Players kept in memory for an instant Back.
const KEPT: usize = 4;
/// Matches in a visited player's window, as on your own page.
const WINDOW: usize = 200;
/// The scroll view the page scrolls in.
pub const SCROLL: &str = "stats-scroll";

/// One player or one match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Player { account_id: u64, tab: Tab },
    /// `as_account` is the player whose page opened it: "your team" is their team.
    Match { match_id: u64, as_account: u64 },
    /// One player on one hero.
    Hero { account_id: u64, hero_id: u32 },
}

impl Place {
    /// The player the page is about at this place.
    pub fn subject(self) -> u64 {
        match self {
            Place::Player { account_id, .. } => account_id,
            Place::Match { as_account, .. } => as_account,
            Place::Hero { account_id, .. } => account_id,
        }
    }
}

/// A subject left behind, kept for Back.
struct Kept {
    account_id: u64,
    snapshot: StatsSnapshot,
    heat: Option<HeatGrid>,
    social: Option<Social>,
}

/// A visited player's page, read on a worker.
pub struct Visit {
    pub account_id: u64,
    pub snapshot: Result<StatsSnapshot, String>,
    pub icons: Vec<(String, Vec<u8>)>,
}

#[derive(Default)]
pub struct Nav {
    /// Places with the scroll offset they were left at. The top is shown.
    pub stack: Vec<(Place, f32)>,
    kept: Vec<Kept>,
    visit_job: Job<Visit>,
    /// The player a visit is loading for.
    pub pending: Option<u64>,
    /// Why the last visit failed, shown in place of the page.
    pub error: Option<String>,
    heat_job: Job<(u64, KillHeatmap)>,
}

thread_local! {
    /// The page's scroll offset now, from the scroll view's own report.
    static SCROLL_Y: Cell<f32> = const { Cell::new(0.0) };
}

/// The scroll view reports its offset here (`View::view` wires it).
pub fn note_scroll(y: f32) {
    SCROLL_Y.with(|cell| cell.set(y));
}

fn scroll_now() -> f32 {
    SCROLL_Y.with(Cell::get)
}

impl Nav {
    pub fn top(&self) -> Option<Place> {
        self.stack.last().map(|(place, _)| *place)
    }

    /// A visit is loading.
    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }

    /// The hero page is open: (player, hero).
    pub fn hero_open(&self) -> Option<(u64, u32)> {
        match self.top() {
            Some(Place::Hero { account_id, hero_id }) => Some((account_id, hero_id)),
            _ => None,
        }
    }

    /// The match page is open.
    pub fn match_open(&self) -> Option<(u64, u64)> {
        match self.top() {
            Some(Place::Match { match_id, as_account }) => Some((match_id, as_account)),
            _ => None,
        }
    }
}

/// The page shows someone other than home.
pub fn visiting(st: &State) -> bool {
    match (st.stats.nav.stack.first(), st.stats.nav.top()) {
        (Some((bottom, _)), Some(top)) => top.subject() != bottom.subject(),
        _ => false,
    }
}

/// A home load landed: the stack starts again from that player, unless the page is already
/// on them alone (a refresh keeps where you are).
pub fn home_loaded(v: &mut super::View, account_id: u64) {
    v.home = Some(account_id);
    let n = &mut v.nav;
    n.pending = None;
    n.error = None;
    n.kept.retain(|kept| kept.account_id != account_id);
    let same = n.stack.len() == 1 && n.top().is_some_and(|top| top.subject() == account_id);
    if !same {
        n.stack = vec![(Place::Player { account_id, tab: v.tab }, 0.0)];
    }
}

/// Open a player's page on Overview. Your own page from anywhere goes back to it.
pub fn open_player(st: &mut State, tx: &mut Tx, account_id: u64) {
    if account_id == 0 {
        return;
    }
    let current = st.stats.nav.top();
    if current == Some(Place::Player { account_id, tab: st.stats.tab }) {
        return;
    }
    push(st, Place::Player { account_id, tab: Tab::Overview });
    tx.scroll_to_y(SCROLL, 0.0);
}

/// Open a match, seen as the page's player.
pub fn open_match(st: &mut State, tx: &mut Tx, match_id: u64) {
    let Some(as_account) = st.stats.snapshot.as_ref().map(|snap| snap.account_id) else { return };
    if st.stats.nav.match_open().is_some() {
        // Previous and next replace the match: Back still leads to the list.
        replace_top(st, Place::Match { match_id, as_account });
    } else {
        push(st, Place::Match { match_id, as_account });
    }
    tx.scroll_to_y(SCROLL, 0.0);
}

/// Open the page player's record on one hero.
pub fn open_hero(st: &mut State, tx: &mut Tx, hero_id: u32) {
    let Some(account_id) = st.stats.snapshot.as_ref().map(|snap| snap.account_id) else { return };
    push(st, Place::Hero { account_id, hero_id });
    tx.scroll_to_y(SCROLL, 0.0);
}

/// Fun run, as a place so Back returns.
pub fn open_fun_run(st: &mut State, tx: &mut Tx) {
    let account_id = st.stats.nav.top().map(Place::subject).or(st.stats.home).unwrap_or(0);
    if st.stats.tab == Tab::FunRun {
        return;
    }
    push(st, Place::Player { account_id, tab: Tab::FunRun });
    tx.scroll_to_y(SCROLL, 0.0);
}

/// Change the tab of the top place. Not a step in the history.
pub fn set_tab(st: &mut State, tab: Tab) {
    st.stats.tab = tab;
    st.stats.menu = None;
    if let Some((Place::Player { tab: t, .. }, _)) = st.stats.nav.stack.last_mut() {
        *t = tab;
    }
    tab_opened(st);
}

/// Start what a tab reads by itself when it opens: Squad's lists, a visited player's map.
pub fn tab_opened(st: &mut State) {
    if st.stats.snapshot.is_none() || rune::is_headless() {
        return;
    }
    match st.stats.tab {
        Tab::Squad if matches!(st.stats.social, Fetched::Idle) => crate::data::start_social_load(st),
        Tab::Map => ensure_heat(st),
        _ => {}
    }
}

fn push(st: &mut State, place: Place) {
    let y = scroll_now();
    let tab = st.stats.tab;
    let n = &mut st.stats.nav;
    if let Some((Place::Player { tab: t, .. }, at)) = n.stack.last_mut() {
        *t = tab;
        *at = y;
    } else if let Some((_, at)) = n.stack.last_mut() {
        *at = y;
    }
    n.stack.push((place, 0.0));
    if n.stack.len() > DEPTH {
        // The bottom is home: drop the oldest visit above it.
        n.stack.remove(1);
    }
    arrive(st, place);
}

fn replace_top(st: &mut State, place: Place) {
    if let Some(top) = st.stats.nav.stack.last_mut() {
        *top = (place, 0.0);
    }
    arrive(st, place);
}

/// Pop one place. True when there was one to pop.
pub fn back(st: &mut State, tx: &mut Tx) -> bool {
    if st.stats.nav.stack.len() < 2 {
        return false;
    }
    st.stats.nav.stack.pop();
    let Some((place, y)) = st.stats.nav.stack.last().copied() else { return false };
    arrive(st, place);
    tx.scroll_to_y(SCROLL, y);
    true
}

/// Mouse Back on the Stats page: one place back, else the app's page history.
pub fn mouse_back(st: &mut State, tx: &mut Tx) -> bool {
    st.screen == crate::model::Screen::Stats && back(st, tx)
}

/// Alt+Left anywhere on the page, and Escape on a match page.
pub fn on_key(st: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if st.screen != crate::model::Screen::Stats {
        return false;
    }
    let alt_left = ev.key == Key::Left && ev.mods.alt && !ev.mods.cmd && !ev.mods.ctrl;
    let page = st.stats.nav.match_open().is_some() || st.stats.nav.hero_open().is_some();
    let escape = ev.key == Key::Escape && page && st.stats.menu.is_none();
    if alt_left || escape { back(st, tx) } else { false }
}

/// Make `place` shown: its subject, its tab, its match.
fn arrive(st: &mut State, place: Place) {
    st.stats.menu = None;
    st.stats.lookup.open = false;
    switch_subject(st, place.subject());
    match place {
        Place::Player { tab, .. } => {
            st.stats.tab = tab;
            tab_opened(st);
        }
        Place::Match { match_id, .. } => super::board::start(st, match_id, false),
        Place::Hero { account_id, hero_id } => super::hero_page::start(st, account_id, hero_id),
    }
}

/// Put `account_id`'s page in the view: already there, kept in memory, or a visit load.
fn switch_subject(st: &mut State, account_id: u64) {
    let current = st.stats.snapshot.as_ref().map(|snap| snap.account_id);
    if current == Some(account_id) {
        return;
    }
    stash(st);
    let n = &mut st.stats.nav;
    n.error = None;
    if let Some(at) = n.kept.iter().position(|kept| kept.account_id == account_id) {
        let kept = n.kept.remove(at);
        n.pending = None;
        install(st, kept.snapshot);
        st.stats.heat = kept.heat;
        st.stats.heat_error = None;
        if let Some(social) = kept.social {
            st.stats.social = Fetched::Ready(social);
        }
        return;
    }
    start_visit(st, account_id);
}

/// Move the page's subject into the kept list.
fn stash(st: &mut State) {
    let Some(snapshot) = st.stats.snapshot.take() else { return };
    st.stats.derived = None;
    let social = match std::mem::replace(&mut st.stats.social, Fetched::Idle) {
        Fetched::Ready(social) => Some(social),
        _ => None,
    };
    let kept = Kept { account_id: snapshot.account_id, snapshot, heat: st.stats.heat.take(), social };
    let n = &mut st.stats.nav;
    n.kept.retain(|k| k.account_id != kept.account_id);
    n.kept.insert(0, kept);
    n.kept.truncate(KEPT);
}

/// A snapshot becomes the page's subject without touching home or the stack.
fn install(st: &mut State, snapshot: StatsSnapshot) {
    super::set_snapshot_only(&mut st.stats, snapshot);
    st.stats.more.items = Fetched::Idle;
    st.stats.more.party = Fetched::Idle;
    st.stats.w4.scoped = Fetched::Idle;
    st.stats.w4.scoped_for = None;
    st.stats.w4.scoped_derived = None;
    super::w4::refilter(st);
}

fn start_visit(st: &mut State, account_id: u64) {
    st.stats.nav.pending = Some(account_id);
    st.stats.heat = None;
    st.stats.heat_error = None;
    if !st.prefs.allow_stats {
        st.stats.nav.pending = None;
        st.stats.nav.error = Some("Player stats are turned off in Settings, Privacy.".to_string());
        return;
    }
    if st.stats.nav.visit_job.is_running() || offline_build() {
        // The running read finishes first; `poll` starts the one asked for now.
        return;
    }
    let wake = st.wake.clone();
    st.stats.nav.visit_job.start(wake, move || Ok(fetch_visit(account_id)));
}

/// Tests and headless scenes never reach the network from here.
pub fn offline_build() -> bool {
    cfg!(test) || rune::is_headless()
}

/// Profile, rank, match history and hero stats (names, ranks and the distribution are cached),
/// then the rank tier's benchmarks, which the tier shares with every player in it.
pub fn fetch_visit(account_id: u64) -> Visit {
    let client = crate::data::stats_client();
    let snapshot = client
        .fetch_snapshot(account_id, WINDOW)
        .map(|mut snapshot| {
            let tier_name = snapshot.tier_names.get(&(snapshot.rank.badge / 10)).cloned();
            snapshot.peers =
                client.fetch_peer_benchmarks(snapshot.rank.badge, snapshot.generated_unix, tier_name).unwrap_or(None);
            snapshot
        })
        .map_err(|error| error.to_string());
    let icons = snapshot.as_ref().map(crate::data::stats_hero_icons).unwrap_or_default();
    Visit { account_id, snapshot, icons }
}

/// Load the kill map for a visited player when the Map tab opens.
fn ensure_heat(st: &mut State) {
    let Some(account_id) = st.stats.snapshot.as_ref().map(|snap| snap.account_id) else { return };
    if st.stats.heat.is_some() || st.stats.nav.heat_job.is_running() || !st.prefs.allow_stats {
        return;
    }
    let wake = st.wake.clone();
    st.stats.nav.heat_job.start(wake, move || {
        crate::data::stats_client()
            .fetch_kill_heatmap(account_id)
            .map(|heat| (account_id, heat))
            .map_err(|error| error.to_string())
    });
}

/// Reload the visited player's page (the refresh button on a visit).
pub fn reload(st: &mut State) {
    let Some(account_id) = st.stats.nav.top().map(Place::subject) else { return };
    if st.stats.nav.visit_job.is_running() {
        return;
    }
    st.stats.last_refresh = Some(std::time::Instant::now());
    st.stats.nav.pending = Some(account_id);
    let wake = st.wake.clone();
    st.stats.nav.visit_job.start(wake, move || Ok(fetch_visit(account_id)));
}

/// Take finished visits and maps. Called from the Stats poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.stats.nav.heat_job.try_take() {
        let subject = st.stats.snapshot.as_ref().map(|snap| snap.account_id);
        match result {
            Ok((account_id, heat)) if Some(account_id) == subject => {
                st.stats.heat = Some(super::bucket(&heat, super::HEAT_CELLS, super::HEAT_CELLS));
                st.stats.heat_error = None;
            }
            Ok(_) => {}
            Err(error) => st.stats.heat_error = Some(error),
        }
        changed = true;
    }
    if let Some(Ok(visit)) = st.stats.nav.visit_job.try_take() {
        changed = true;
        let wanted = st.stats.nav.pending;
        st.pending_thumbs.extend(visit.icons);
        if wanted == Some(visit.account_id) {
            st.stats.nav.pending = None;
            match visit.snapshot {
                Ok(snapshot) => {
                    // A refresh of the page already shown keeps its other sections.
                    st.stats.snapshot = None;
                    install(st, snapshot);
                    tab_opened(st);
                }
                Err(error) => st.stats.nav.error = Some(error),
            }
        } else if let Some(account_id) = wanted {
            let wake = st.wake.clone();
            st.stats.nav.visit_job.start(wake, move || Ok(fetch_visit(account_id)));
        }
    }
    changed
}

/// Track the page's player: add them to the tracked list, as the account button would.
pub fn track(st: &mut State) {
    let Some(snap) = st.stats.snapshot.as_ref() else { return };
    let (account_id, name) = (snap.account_id, snap.profile.persona_name.clone());
    crate::data::remember_stats_account(st, account_id, &name);
}

/// Scenes: put a second player on the stack over home, as a visit would.
pub fn scene_visit(st: &mut State, snapshot: StatsSnapshot) {
    let place = Place::Player { account_id: snapshot.account_id, tab: Tab::Overview };
    stash(st);
    st.stats.nav.stack.push((place, 0.0));
    install(st, snapshot);
    st.stats.tab = Tab::Overview;
}

/// Scenes: push a match page over the page.
pub fn scene_match(st: &mut State, board: deadlock_core::stats::board::MatchScoreboard) {
    let place = Place::Match { match_id: board.match_id, as_account: board.account_id };
    st.stats.nav.stack.push((place, 0.0));
    st.stats.board = Fetched::Ready(board);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut s = State::default();
        super::super::load_fixture(&mut s);
        s
    }

    #[test]
    fn home_is_the_bottom_and_visits_stack_on_it() {
        let mut s = state();
        let home = s.stats.snapshot.as_ref().unwrap().account_id;
        assert_eq!(s.stats.nav.stack.len(), 1);
        assert_eq!(s.stats.nav.top().map(Place::subject), Some(home));
        assert!(!visiting(&s));
        let mut tx = Tx::default();
        set_tab(&mut s, Tab::Heroes);
        open_player(&mut s, &mut tx, 81_220_464);
        assert_eq!(s.stats.nav.stack.len(), 2);
        assert!(visiting(&s));
        assert_eq!(s.stats.tab, Tab::Overview);
        assert_eq!(s.stats.nav.pending, Some(81_220_464), "a visit loads");
        assert!(s.stats.snapshot.is_none(), "home is kept aside");
        // Back: home again at once, on the tab it was left on.
        assert!(back(&mut s, &mut tx));
        assert_eq!(s.stats.snapshot.as_ref().map(|snap| snap.account_id), Some(home));
        assert_eq!(s.stats.tab, Tab::Heroes);
        assert!(!back(&mut s, &mut tx), "home is the bottom");
    }

    #[test]
    fn visiting_never_tracks_and_tabs_do_not_push() {
        let mut s = state();
        let tracked = s.stats.remembered.clone();
        let mut tx = Tx::default();
        open_player(&mut s, &mut tx, 55);
        set_tab(&mut s, Tab::Matches);
        set_tab(&mut s, Tab::Heroes);
        assert_eq!(s.stats.nav.stack.len(), 2);
        assert_eq!(s.stats.nav.top(), Some(Place::Player { account_id: 55, tab: Tab::Heroes }));
        assert_eq!(s.stats.remembered, tracked);
    }

    #[test]
    fn a_match_is_seen_as_the_page_player_and_prev_next_replace_it() {
        let mut s = state();
        let home = s.stats.snapshot.as_ref().unwrap().account_id;
        let mut tx = Tx::default();
        open_match(&mut s, &mut tx, 7);
        assert_eq!(s.stats.nav.top(), Some(Place::Match { match_id: 7, as_account: home }));
        open_match(&mut s, &mut tx, 8);
        assert_eq!(s.stats.nav.stack.len(), 2, "previous and next replace the match");
        let mut ev = KeyEvent { key: Key::Escape, mods: Default::default() };
        assert!(on_key(&mut s, &ev, &mut tx), "Escape leaves a match page");
        assert_eq!(s.stats.nav.stack.len(), 1);
        ev.key = Key::Escape;
        assert!(!on_key(&mut s, &ev, &mut tx), "and does nothing on a player page");
    }

    #[test]
    fn a_hero_page_is_a_place_back_and_escape_leave() {
        let mut s = state();
        let mut tx = Tx::default();
        set_tab(&mut s, Tab::Heroes);
        open_hero(&mut s, &mut tx, 7);
        let home = s.stats.snapshot.as_ref().unwrap().account_id;
        assert_eq!(s.stats.nav.hero_open(), Some((home, 7)));
        let ev = KeyEvent { key: Key::Escape, mods: Default::default() };
        assert!(on_key(&mut s, &ev, &mut tx));
        assert_eq!(s.stats.nav.hero_open(), None);
        assert_eq!(s.stats.tab, Tab::Heroes);
    }

    #[test]
    fn the_stack_is_capped_and_keeps_home() {
        let mut s = state();
        let home = s.stats.snapshot.as_ref().unwrap().account_id;
        let mut tx = Tx::default();
        for id in 1..=(DEPTH as u64 + 5) {
            open_player(&mut s, &mut tx, 1000 + id);
        }
        assert_eq!(s.stats.nav.stack.len(), DEPTH);
        assert_eq!(s.stats.nav.stack[0].0.subject(), home);
    }
}
