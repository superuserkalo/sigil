//! Stats: one page, one player, one selector.
//!
//! Every number comes from a `StatsSnapshot` read from `api.deadlock-api.com` (or the captured
//! fixture in `fixtures/stats-profile.json` for snapshots). The page always shows one player:
//! your primary tracked account, or anyone you open from a list (`nav.rs` keeps the stack and
//! Back). The tabs: Overview (who, the four numbers, recent matches and heroes, what stands
//! out, progress), Matches (every match by sitting; a row opens the match page), Heroes,
//! Items, Squad (teammates and opponents), Live, Leaderboard and Map. The name search in the
//! head (`lookup.rs`) opens any player; Fun run is the button beside it.
//!
//! No aim, impact or positioning score is shown: the API does not carry one.

use std::time::{Duration, Instant};

use rune::Tx;

use deadlock_core::stats::board::MatchScoreboard;
use deadlock_core::stats::cache::Age;
use deadlock_core::stats::insight::{self, Brief, Form, RankPoint, Session, Streak};
use deadlock_core::stats::leaderboard::RankedSeason;
use deadlock_core::stats::social::Social;
use deadlock_core::stats::{KillHeatmap, Leaderboard, MapObjective, PlayerProfile, StatsSnapshot};
use deadlock_core::steam_users::SteamUser;
use rune::{Color, El, Key, KeyEvent, ScrollSpec, TextEdit, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, LABEL, PAGE_TOP, PAGE_X, SMALL, Ui};

mod board;
pub mod lookup;
pub mod nav;
pub mod pictures;
mod brief;
mod demos;
mod german;
mod heroes;
mod leaderboard;
pub mod live;
mod live_de;
mod map;
mod matches;
pub mod more;
mod notes;
pub mod roll;
mod roll_de;
pub mod w4;
mod shared;
mod social;

use german::tr;
use shared::{avatar, problem, TNUM_SMALL};

/// Heatmap grid resolution, cells per side.
pub const HEAT_CELLS: usize = 48;
const HEROES_SHOWN: usize = 6;
const SITTINGS_SHOWN: usize = 3;
const FORM_BEADS: usize = 20;
const SPARK_MATCHES: usize = 30;
/// Manual refresh waits this long: the endpoint is shared and asks for no bursts.
pub const REFRESH_COOLDOWN: Duration = Duration::from_secs(60);
/// Pages a leaderboard shows before it jumps to the player's own place.
pub const LEADERBOARD_ROWS: usize = 10;

// --- state -----------------------------------------------------------------

pub struct HeatCell {
    pub cx: usize,
    pub cy: usize,
    pub kills: u64,
    pub deaths: u64,
    /// Deaths split by the player's team at the time: `[team 0, team 1]`.
    pub death_team: [u64; 2],
    /// Kills split by the player's team at the time.
    pub kill_team: [u64; 2],
}

pub struct HeatGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<HeatCell>,
    pub kills: u64,
    pub deaths: u64,
    pub window: String,
    pub source_cells: usize,
}

/// One imported demo. Kept whole from the demo list even where the row does not
/// show a field: the review step reads the same value later.
#[derive(Clone)]
#[allow(dead_code)]
pub struct DemoRow {
    pub match_id: String,
    pub source: String,
    pub map: Option<String>,
    pub duration_seconds: Option<u64>,
    pub players: usize,
    pub events: usize,
    pub heat_cells: usize,
    pub has_api: bool,
    pub updated: String,
}

pub struct DemoLoad {
    pub rows: Vec<DemoRow>,
    pub error: Option<String>,
}

pub struct Load {
    pub account_id: u64,
    pub snapshot: Result<StatsSnapshot, String>,
    pub heatmap: Result<KillHeatmap, String>,
    pub objectives: Vec<MapObjective>,
    pub icons: Vec<(String, Vec<u8>)>,
    /// Steam accounts this computer has signed in with, read off the UI thread.
    pub steam: Vec<SteamUser>,
    /// How old the data behind this load is, and whether a stale copy answered.
    pub age: Age,
}

/// An account this machine has loaded before. One of them is the primary
/// account: the page opens on it, and it is the one the header names.
#[derive(Clone, Debug, PartialEq)]
pub struct Remembered {
    pub account_id: u64,
    pub name: String,
    pub primary: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Chart {
    Rank,
    WinRate,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Deaths,
    Kills,
}

/// One background section: nothing yet, a request running, the value, or why it
/// failed.
pub enum Fetched<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

/// Values derived once per snapshot, not per frame.
pub struct Derived {
    pub brief: Brief,
    pub trajectory: Vec<RankPoint>,
    pub rolling: Vec<(i64, f64)>,
    pub sessions: Vec<Session>,
    pub form: Form,
    pub streak: Option<Streak>,
}

/// The Stats tabs. Every section of the page has one home. Fun run is not in the tab row: the
/// button beside the search opens it, as a place Back returns from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Overview,
    Matches,
    Heroes,
    Items,
    Squad,
    Live,
    Leaderboard,
    Map,
    FunRun,
}

impl Tab {
    /// The tab row, in order.
    pub const ROW: [Tab; 8] = [
        Tab::Overview,
        Tab::Matches,
        Tab::Heroes,
        Tab::Items,
        Tab::Squad,
        Tab::Live,
        Tab::Leaderboard,
        Tab::Map,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Live => "Live",
            Tab::Matches => "Matches",
            Tab::Heroes => "Heroes",
            Tab::Items => "Items",
            Tab::Squad => "Squad",
            Tab::Leaderboard => "Leaderboard",
            Tab::Map => "Map",
            Tab::FunRun => "Fun run",
        }
    }

    /// Whether the match filter scopes what this tab shows. On the others the filter line is
    /// hidden and keeps its height.
    pub fn filtered(self) -> bool {
        matches!(self, Tab::Overview | Tab::Matches | Tab::Heroes | Tab::Items | Tab::Squad)
    }
}

/// A menu open over the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Menu {
    /// Track, Statlocker and Copy id for a player, under the row keyed by the string.
    Person(u64, String),
    /// The Matches tab's hero filter.
    Hero,
    /// The Matches tab's result filter.
    Result,
    /// The Street Brawl choice on the filter line.
    Brawl,
}

pub struct View {
    /// The open tab, kept for the session.
    pub tab: Tab,
    pub loaded: bool,
    pub account: TextEdit,
    pub error: Option<String>,
    pub snapshot: Option<StatsSnapshot>,
    pub derived: Option<Derived>,
    pub heat: Option<HeatGrid>,
    pub heat_error: Option<String>,
    pub objectives: Vec<MapObjective>,
    pub chart: Chart,
    pub layer: Layer,
    pub all_heroes: bool,
    pub sittings_shown: usize,
    pub account_open: bool,
    pub remembered: Vec<Remembered>,
    /// Steam accounts found on this computer, offered as a one-press add.
    pub steam: Vec<SteamUser>,
    pub search: Vec<PlayerProfile>,
    pub search_job: Job<Vec<PlayerProfile>>,
    pub search_note: Option<String>,
    /// When the page last asked the API, for the 60 second refresh cooldown.
    pub last_refresh: Option<Instant>,
    pub age: Age,
    /// The match page's board, for the match on top of the stack.
    pub board: Fetched<MatchScoreboard>,
    pub board_job: Job<MatchScoreboard>,
    /// When "Fetch from Steam" was last pressed: deadlock-api allows 3 an hour per IP.
    pub board_steam_at: Option<Instant>,
    /// Where the page is, and Back (`nav.rs`).
    pub nav: nav::Nav,
    /// The account the page loads by itself: the primary tracked one.
    pub home: Option<u64>,
    /// Find a player (`lookup.rs`).
    pub lookup: lookup::Lookup,
    /// Badge art and other pictures (`pictures.rs`).
    pub pictures: pictures::Pictures,
    /// The open menu, if any.
    pub menu: Option<Menu>,
    /// The Matches tab's own filters and paging.
    pub list: matches::ListView,
    /// Items shows everyone's item meta instead of yours.
    pub items_everyone: bool,
    /// Steam accounts on this PC, read once for the start page.
    pub steam_job: Job<(Vec<SteamUser>, Vec<(String, Vec<u8>)>)>,
    pub steam_asked: bool,
    pub social: Fetched<Social>,
    pub social_job: Job<Social>,
    pub social_all: bool,
    /// The Squad tab's teammates sort.
    pub people_sort: social::PeopleSort,
    pub leaderboard: Fetched<Leaderboard>,
    pub leaderboard_job: Job<Leaderboard>,
    pub leaderboard_all: bool,
    /// The leaderboard region the player picked; `None` until one is derived
    /// from the profile's country.
    pub region: Option<String>,
    pub seasons: Vec<RankedSeason>,
    pub demo_path: TextEdit,
    pub demos: Vec<DemoRow>,
    pub demo_error: Option<String>,
    pub demo_busy: bool,
    pub review: Option<ReviewPanel>,
    /// The ranked and date filter, Items, the purchase timeline and Party (`more.rs`).
    pub more: more::More,
    /// Wave 4: the filtered overview, Party by hand and Item meta (`w4.rs`).
    pub w4: w4::W4,
    /// The match being played right now (`live.rs`).
    pub live: live::Live,
    /// Fun run: a random hero and build from a share code (`roll.rs`).
    pub roll: roll::RollView,
}

pub struct ReviewPanel {
    pub match_id: String,
    pub facts: deadlock_core::review::ReviewFacts,
    pub body: TextEdit,
    pub log: Option<String>,
    pub tone: crate::ui::Tone,
}

impl Default for View {
    fn default() -> Self {
        View {
            tab: Tab::Overview,
            loaded: false,
            account: TextEdit::new(""),
            error: None,
            snapshot: None,
            derived: None,
            heat: None,
            heat_error: None,
            objectives: Vec::new(),
            chart: Chart::Rank,
            layer: Layer::Deaths,
            all_heroes: false,
            sittings_shown: SITTINGS_SHOWN,
            account_open: false,
            remembered: if rune::is_headless() {
                Vec::new()
            } else {
                crate::data::stats_remembered()
            },
            steam: Vec::new(),
            search: Vec::new(),
            search_job: Job::default(),
            search_note: None,
            last_refresh: None,
            age: Age::default(),
            board: Fetched::Idle,
            board_job: Job::default(),
            board_steam_at: None,
            nav: nav::Nav::default(),
            home: None,
            lookup: lookup::Lookup::default(),
            pictures: pictures::Pictures::default(),
            menu: None,
            list: matches::ListView::default(),
            items_everyone: false,
            steam_job: Job::default(),
            steam_asked: false,
            social: Fetched::Idle,
            social_job: Job::default(),
            social_all: false,
            people_sort: social::PeopleSort::default(),
            leaderboard: Fetched::Idle,
            leaderboard_job: Job::default(),
            leaderboard_all: false,
            region: None,
            seasons: Vec::new(),
            demo_path: TextEdit::new(""),
            demos: Vec::new(),
            demo_error: None,
            demo_busy: false,
            review: None,
            more: more::More::default(),
            w4: w4::W4::default(),
            live: live::Live::default(),
            roll: roll::RollView::default(),
        }
    }
}

impl View {
    /// True when the refresh button may ask again. The API asks clients not to
    /// send bursts, and a manual refresh costs about nine requests.
    pub fn can_refresh(&self) -> bool {
        self.last_refresh
            .is_none_or(|at| at.elapsed() >= REFRESH_COOLDOWN)
    }

    pub fn refresh_wait(&self) -> u64 {
        self.last_refresh
            .map(|at| REFRESH_COOLDOWN.saturating_sub(at.elapsed()).as_secs() + 1)
            .unwrap_or(0)
    }

    /// The leaderboard region: the player's choice, else the profile's.
    pub fn region(&self) -> &str {
        self.region.as_deref().unwrap_or("Europe")
    }

    pub fn primary(&self) -> Option<&Remembered> {
        self.remembered
            .iter()
            .find(|row| row.primary)
            .or_else(|| self.remembered.first())
    }
}

/// Keep the primary account first, then the most recently loaded.
pub fn sort_remembered(rows: &mut [Remembered]) {
    rows.sort_by_key(|row| !row.primary);
}

pub fn icon_key(hero_id: u32) -> String {
    format!("stats-hero:{hero_id}")
}

/// Everything the view draws from a snapshot. Street Brawl games stay in the sittings (the
/// match list, tagged); the brief, rank line, win rate, form and streak read normal games only,
/// as their souls and K/D do not compare (Grimoire tags them for the same reason).
pub fn derive(snapshot: &StatsSnapshot) -> Derived {
    let normal = StatsSnapshot {
        matches: deadlock_core::stats::filters::normal_only(&snapshot.matches),
        ..snapshot.clone()
    };
    let matches = &normal.matches;
    Derived {
        brief: insight::brief(&normal),
        trajectory: insight::rank_trajectory(matches),
        rolling: insight::rolling_win_rate(matches, insight::RECENT_MATCHES),
        sessions: insight::sessions(&snapshot.matches, insight::SESSION_GAP_SECONDS),
        form: insight::form(matches),
        streak: insight::current_streak(matches),
    }
}

/// A home load: the snapshot becomes the page, and the stack starts from it.
pub fn set_snapshot(view: &mut View, snapshot: StatsSnapshot) {
    let account_id = snapshot.account_id;
    set_snapshot_only(view, snapshot);
    nav::home_loaded(view, account_id);
}

/// Store a snapshot and derive everything the view draws from it.
pub fn set_snapshot_only(view: &mut View, snapshot: StatsSnapshot) {
    let derived = derive(&snapshot);
    view.chart = if derived.trajectory.len() >= 2 {
        Chart::Rank
    } else {
        Chart::WinRate
    };
    view.derived = Some(derived);
    view.sittings_shown = SITTINGS_SHOWN;
    view.list = matches::ListView::default();
    view.all_heroes = false;
    view.social = Fetched::Idle;
    view.social_all = false;
    view.leaderboard = Fetched::Idle;
    if view.region.is_none() {
        view.region = deadlock_core::stats::leaderboard::region_for_country(
            snapshot.profile.country_code.as_deref(),
        )
        .map(str::to_string);
    }
    view.snapshot = Some(snapshot);
}

/// Search Steam names on a worker, for a pasted name or vanity link.
pub fn start_search(state: &mut State, query: String) {
    if state.stats.search_job.is_running() {
        return;
    }
    if !state.prefs.allow_stats {
        state.stats.search_note = Some(tr("Player stats are turned off in Settings, Privacy.").to_string());
        return;
    }
    state.stats.search.clear();
    state.stats.search_note = Some(crate::screens::settings::i18n::fill(
        tr("Looking for “{}”"),
        &[&query],
    ));
    let wake = state.wake.clone();
    state.stats.search_job.start(wake, move || {
        deadlock_core::stats::StatsClient::new()
            .search_players(&query, 6)
            .map_err(|error| error.to_string())
    });
}

/// Called from the app's poll loop.
pub fn poll(state: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = state.stats.search_job.try_take() {
        match result {
            Ok(rows) if rows.is_empty() => {
                state.stats.search_note = Some(
                    tr("No active player by that name. Paste the profile link or the account id instead.")
                        .to_string(),
                );
            }
            Ok(rows) => {
                state.stats.search_note = None;
                state.stats.search = rows;
            }
            Err(error) => state.stats.search_note = Some(error),
        }
        changed = true;
    }
    changed |= board::poll(state);
    changed |= nav::poll(state);
    changed |= lookup::poll(state);
    changed |= pictures::poll(state);
    changed |= poll_steam(state);
    if let Some(result) = state.stats.social_job.try_take() {
        state.stats.social = match result {
            Ok(social) => Fetched::Ready(social),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = state.stats.leaderboard_job.try_take() {
        state.stats.leaderboard = match result {
            Ok(board) => Fetched::Ready(board),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    changed |= more::poll(state);
    changed |= w4::poll(state);
    changed |= live::poll(state);
    changed |= roll::poll(state);
    changed
}

/// Read the Steam accounts on this PC (and Steam's own copy of their avatars) once, for the
/// start page. Local files only; nothing is sent.
fn poll_steam(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok((users, avatars))) = st.stats.steam_job.try_take() {
        if st.stats.steam.is_empty() {
            st.stats.steam = users;
        }
        st.pending_thumbs.extend(avatars);
        changed = true;
    }
    if st.stats.steam_asked || st.stats.snapshot.is_some() || st.screen != crate::model::Screen::Stats || nav::offline_build() || st.demo {
        return changed;
    }
    st.stats.steam_asked = true;
    let wake = st.wake.clone();
    st.stats.steam_job.start(wake, || {
        let roots = deadlock_core::discovery::default_steam_roots();
        let users = deadlock_core::steam_users::detect(&roots);
        let avatars = users
            .iter()
            .filter_map(|user| {
                let path = deadlock_core::steam_users::cached_avatar(&roots, user.steam_id64)?;
                let bytes = std::fs::read(path).ok()?;
                Some((pictures::avatar_key(user.account_id as u64), pictures::scale(&bytes, 96)?))
            })
            .collect();
        Ok((users, avatars))
    });
    changed
}

/// Bucket raw kill and death positions into a fixed grid in the minimap frame.
pub fn bucket(heatmap: &KillHeatmap, cols: usize, rows: usize) -> HeatGrid {
    #[derive(Default, Clone, Copy)]
    struct Slot {
        kills: u64,
        deaths: u64,
        death_team: [u64; 2],
        kill_team: [u64; 2],
    }
    let mut slots = vec![Slot::default(); cols * rows];
    for point in &heatmap.points {
        let (nx, ny) =
            deadlock_core::stats::world_to_map(point.position_x as f64, point.position_y as f64);
        if !(0.0..=1.0).contains(&nx) || !(0.0..=1.0).contains(&ny) {
            continue;
        }
        let cx = ((nx * cols as f64) as usize).min(cols - 1);
        let cy = ((ny * rows as f64) as usize).min(rows - 1);
        let slot = &mut slots[cy * cols + cx];
        slot.kills += point.kills;
        slot.deaths += point.deaths;
        let killer = (point.killer_team.clamp(0, 1)) as usize;
        slot.kill_team[killer] += point.kills;
        slot.death_team[1 - killer] += point.deaths;
    }
    let mut cells = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        if slot.kills == 0 && slot.deaths == 0 {
            continue;
        }
        cells.push(HeatCell {
            cx: index % cols,
            cy: index / cols,
            kills: slot.kills,
            deaths: slot.deaths,
            death_team: slot.death_team,
            kill_team: slot.kill_team,
        });
    }
    HeatGrid {
        cols,
        rows,
        cells,
        kills: heatmap.kills,
        deaths: heatmap.deaths,
        window: heatmap.window.clone(),
        source_cells: heatmap.cells,
    }
}

// --- scenes ----------------------------------------------------------------

const FIXTURE: &str = include_str!("../../../fixtures/stats-profile.json");
const EXTRA: &str = include_str!("../../../fixtures/stats-extra.json");

/// Load the captured profile, as a live load would, without the network.
pub fn load_fixture(s: &mut State) {
    s.screen = crate::model::Screen::Stats;
    let captured = match deadlock_core::stats::snapshot_from_bundle(FIXTURE) {
        Ok(captured) => captured,
        Err(error) => {
            s.stats.error = Some(error.to_string());
            return;
        }
    };
    s.stats.loaded = true;
    s.pending_thumbs
        .extend(crate::data::stats_hero_icons(&captured.snapshot));
    s.stats.remembered = vec![
        Remembered {
            account_id: captured.snapshot.account_id,
            name: captured.snapshot.profile.persona_name.clone(),
            primary: true,
        },
        Remembered {
            account_id: 81_220_464,
            name: "Duo partner".to_string(),
            primary: false,
        },
    ];
    s.stats.steam = vec![
        SteamUser {
            steam_id64: deadlock_core::steam_users::account_to_id64(captured.snapshot.account_id as u32),
            account_id: captured.snapshot.account_id as u32,
            persona: captured.snapshot.profile.persona_name.clone(),
            most_recent: true,
        },
        SteamUser {
            steam_id64: deadlock_core::steam_users::account_to_id64(104_293_871),
            account_id: 104_293_871,
            persona: "Alt".to_string(),
            most_recent: false,
        },
    ];
    s.stats.age = Age::default();
    s.stats.heat = Some(bucket(&captured.heatmap, HEAT_CELLS, HEAT_CELLS));
    s.stats.objectives = captured.objectives;
    set_snapshot(&mut s.stats, captured.snapshot);
}

/// The full profile from the fixture.
pub fn scene_profile(s: &mut State) {
    load_fixture(s);
}

/// The Matches tab with every hero shown and the kill layer on.
pub fn scene_drill(s: &mut State) {
    load_fixture(s);
    s.stats.tab = Tab::Matches;
    s.stats.chart = Chart::WinRate;
    s.stats.layer = Layer::Kills;
    s.stats.all_heroes = true;
}

/// The newest match's page: both teams, builds and the soul lead.
pub fn scene_board(s: &mut State) {
    load_fixture(s);
    let Ok(board) = deadlock_core::stats::board::scoreboard_from_bundle(EXTRA) else {
        return;
    };
    nav::scene_match(s, board);
}

/// Another player's page, opened from a list: the context bar with Back and Track.
pub fn scene_visit(s: &mut State) {
    load_fixture(s);
    let Ok(mut captured) = deadlock_core::stats::snapshot_from_bundle(FIXTURE) else { return };
    let snap = &mut captured.snapshot;
    snap.account_id = 81_220_464;
    snap.profile.account_id = 81_220_464;
    snap.profile.persona_name = "Duo partner".to_string();
    snap.matches.truncate(37);
    nav::scene_visit(s, captured.snapshot);
}

/// The name search open with results.
pub fn scene_lookup(s: &mut State) {
    use deadlock_core::stats::RankBadge;
    load_fixture(s);
    s.stats.lookup.edit = rune::TextEdit::new("mika");
    s.stats.lookup.open = true;
    let rows = [
        (385_814_004, "MikaelS", Some("FI"), 116),
        (220_118_009, "mikael", Some("SE"), 72),
        (97_550_431, "Mika the Third", None, 0),
        (1_204_377_610, "mikaaa", Some("DE"), 94),
    ];
    s.stats.lookup.results = rows
        .into_iter()
        .map(|(account_id, name, country, badge)| lookup::Found {
            account_id,
            name: name.to_string(),
            country: country.map(str::to_string),
            avatar: None,
            rank: Some(RankBadge { account_id, badge, rank: badge / 10, subrank: badge % 10 }),
        })
        .collect();
    s.stats.remembered.push(Remembered { account_id: 55_000_123, name: "Mika (duo)".to_string(), primary: false });
}

/// Squad: the lists, the cards and the planner, from the fixture.
pub fn scene_squad(s: &mut State) {
    scene_people(s);
    s.stats.tab = Tab::Squad;
}

/// Teammates, opponents and the leaderboard, from the fixture.
pub fn scene_people(s: &mut State) {
    load_fixture(s);
    s.stats.tab = Tab::Squad;
    if let Ok(mut social) = deadlock_core::stats::social::social_from_bundle(EXTRA) {
        scene_lanes(&mut social);
        s.stats.social = Fetched::Ready(social);
    }
    if let Ok(captured) = deadlock_core::stats::leaderboard::board_from_bundle(EXTRA) {
        s.stats.region = Some(captured.region);
        s.stats.seasons = captured.seasons;
        s.stats.leaderboard = Fetched::Ready(captured.board);
    }
}

/// Lane counts for the scene's teammates: the captured bundle has only three shared match ids
/// per mate, so the scene fills a fixed spread (some below the five-match floor).
fn scene_lanes(social: &mut Social) {
    use deadlock_core::stats::social::LaneRecord;
    for (index, mate) in social.mates.iter_mut().enumerate() {
        let checked = mate.matches.min(60);
        let matches = match index % 4 {
            0 => checked * 2 / 3,
            1 => checked / 3,
            2 => 3,
            _ => checked / 2,
        };
        let wins = (matches * (40 + (index as u64 * 7) % 35)) / 100;
        mate.lane = Some(LaneRecord { checked, matches, wins });
    }
    social.lanes_known = true;
}

/// Overview with the Squad data loaded.
pub fn scene_partner(s: &mut State) {
    scene_people(s);
    s.stats.tab = Tab::Overview;
}

/// Items rows for a scene (`wave3_scenes`).
pub fn more_items(rows: Vec<deadlock_core::stats::items::ItemStat>, names: std::collections::HashMap<u64, String>) -> more::ItemsLoad {
    more::ItemsLoad { rows, names, icons: Default::default(), slots: Default::default() }
}

/// The account panel open, with tracked players and Steam accounts to add.
pub fn scene_tracked(s: &mut State) {
    load_fixture(s);
    s.stats.account_open = true;
}

/// First run: nothing tracked, two Steam accounts found on this PC, one press each.
pub fn scene_entry(s: &mut State) {
    s.screen = crate::model::Screen::Stats;
    s.stats.loaded = true;
    s.stats.remembered = Vec::new();
    s.stats.steam = vec![
        SteamUser {
            steam_id64: deadlock_core::steam_users::account_to_id64(40_312_977),
            account_id: 40_312_977,
            persona: "Wren".to_string(),
            most_recent: true,
        },
        SteamUser {
            steam_id64: deadlock_core::steam_users::account_to_id64(104_293_871),
            account_id: 104_293_871,
            persona: "Wren (smurf)".to_string(),
            most_recent: false,
        },
    ];
}

/// First run on a PC with no Steam account found: the typed fallback.
pub fn scene_entry_manual(s: &mut State) {
    s.screen = crate::model::Screen::Stats;
    s.stats.loaded = true;
    s.stats.remembered = Vec::new();
    s.stats.steam_asked = true;
}

// --- page ------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let loading = s.stats_job.is_running() || v.nav.loading();
    let ready = v.can_refresh();
    let has_player = v.snapshot.is_some();
    let visiting = nav::visiting(s);
    // With no player every tab shows the start page under Overview, except Fun run.
    let tab = if has_player || loading || v.tab == Tab::FunRun { v.tab } else { Tab::Overview };

    // The status slot keeps its width whatever it says, so the buttons beside it never move.
    let error = v.nav.error.as_ref().filter(|_| visiting).or(v.error.as_ref());
    let (status, warn) = match (&v.snapshot, error) {
        (Some(_), Some(error)) => (error.clone(), true),
        (Some(_), None) if loading => (tr("Refreshing").to_string(), false),
        (Some(_), None) if v.age.stale => (tr("Older copy").to_string(), false),
        _ => (String::new(), false),
    };
    let mut slot = row().w(160.0).h(28.0).gap(6.0).center_y().justify_end();
    if warn {
        slot = slot
            .label(status.clone())
            .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden());
    }
    slot = slot.child(text(status).font(SMALL).color(u.muted).truncate().min_w(0.0).shrink());

    let mut actions = row().gap(8.0).center_y().child(slot).child(lookup::field(s, u));
    if let Some(primary) = v.primary() {
        actions = actions.child(account_button(s, u, primary.account_id, &primary.name, v.account_open));
    }
    actions = actions.child(
        u.icon_button(&icons::SPARKLES, tr("Fun run"))
            .key("stats-fun-run")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(|st: &mut State, tx: &mut Tx| nav::open_fun_run(st, tx)),
    );
    if has_player {
        actions = actions.child(
            u.icon_button(&icons::REFRESH_CW, tr("Refresh"))
                .key("stats-refresh")
                .opacity(if loading || !ready { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if nav::visiting(st) {
                        if st.stats.can_refresh() {
                            nav::reload(st);
                        }
                    } else {
                        crate::data::start_stats_load(st);
                    }
                }),
        );
    }
    // The shared page head, so Stats picks up the app's one title style.
    let head = u.page_head("", tr("Stats"), "", Some(actions));

    let mut page = col().key("stats-page").w_full().px(PAGE_X).pt(PAGE_TOP).pb(48.0).child(head);
    if v.nav.match_open().is_some() {
        page = page.child(board::page(s, u).mt(12.0));
    } else {
        if let Some(bar) = shared::context_bar(s, u) {
            page = page.child(bar.mt(8.0));
        }
        // The filter line is hidden, not greyed, on the tabs it does not scope, and keeps its
        // height there so the body starts at the same y on every tab.
        let filter = if has_player && tab.filtered() {
            more::filter_bar(s, u)
        } else {
            row().key("stats-filter-room").w_full().h(28.0)
        };
        let body = match (&v.snapshot, &v.derived) {
            (Some(snap), Some(derived)) => tab_body(s, u, tab, snap, derived),
            _ if tab == Tab::FunRun => roll::section(s, u),
            _ if loading => skeleton(u),
            _ if visiting => match &v.nav.error {
                Some(error) => problem(u, error),
                None => skeleton(u),
            },
            _ => entry(s, u),
        };
        page = page
            .child(shared::tabs(s, u, tab, has_player || loading).mt(12.0))
            .child(filter.mt(12.0))
            .child(body.mt(16.0));
    }

    let mut out = col()
        .key("stats-root")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            u.scroll_area(col()
                .key(nav::SCROLL)
                .w_full()
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .on_scroll_offset(|at| nav::note_scroll(at.offset[1]))
                .child(page)),
        );
    // Tracked players, the search results and the menus open over the page.
    if v.account_open {
        out = out.child(account_popover(s, u));
    }
    if let Some(popover) = lookup::popover(s, u) {
        out = out.child(popover);
    }
    if let Some(menu) = more::season_menu(s, u) {
        out = out.child(menu);
    }
    if let Some(menu) = menu_layer(s, u) {
        out = out.child(menu);
    }
    out
}

/// The open menu over the page, with a scrim that closes it.
fn menu_layer(s: &State, u: &Ui) -> Option<El> {
    let menu = match s.stats.menu.as_ref()? {
        Menu::Person(account_id, anchor) => shared::person_menu(s, u, *account_id, anchor),
        Menu::Hero => matches::hero_menu(s, u),
        Menu::Result => matches::result_menu(s, u),
        Menu::Brawl => more::brawl_menu(s, u),
    };
    let scrim = el().on_press(|st: &mut State| st.stats.menu = None);
    Some(u.popover_layer("stats-menu-layer", scrim, menu.on_key(|st: &mut State, ev: &KeyEvent, _| {
        if ev.key == Key::Escape {
            st.stats.menu = None;
            true
        } else {
            false
        }
    })))
}

/// One tab's sections. Every section of the old single page lives in exactly one tab.
fn tab_body(s: &State, u: &Ui, tab: Tab, snap: &StatsSnapshot, derived: &Derived) -> El {
    let (osnap, oderived) = w4::overview(s).unwrap_or((snap, derived));
    // A tab body fades in whole on a tab press and the old one is gone that frame
    // (`design/motion.md` 4.12), so its rows never fade in one by one on their own default.
    let section = |key: &str| col().key(key.to_string()).w_full().gap(24.0).transition(crate::motion::page_enter());
    match tab {
        Tab::Overview => brief::overview(s, u, osnap, oderived).transition(crate::motion::page_enter()),
        Tab::Live => section("stats-tab-live").child(live::tab(s, u)),
        Tab::Matches => section("stats-tab-matches")
            .children(w4::scope_note(s, u))
            .child(matches::matches(s, u, snap, derived))
            .child(demos::demo_section(s, u)),
        Tab::Heroes => section("stats-tab-heroes")
            .children(w4::scope_note(s, u))
            .child(heroes::heroes(s, u, osnap)),
        Tab::Items => section("stats-tab-items").child(items_tab(s, u)),
        Tab::Squad => section("stats-tab-squad")
            .child(social::social_section(s, u))
            .child(more::party_section(s, u)),
        Tab::Leaderboard => section("stats-tab-board").child(leaderboard::leaderboard_section(s, u, snap)),
        Tab::Map => section("stats-tab-map").child(map::map_section(s, u)),
        Tab::FunRun => roll::section(s, u),
    }
}

/// Items: yours against everyone, or everyone's meta, one at a time.
fn items_tab(s: &State, u: &Ui) -> El {
    let everyone = s.stats.items_everyone;
    let pick = crate::ui::Segmented::new("stats-items-scope", tr("Items"))
        .size(crate::ui::SegSize::M)
        .part(crate::ui::SegPart::new("stats-items-mine", tr("Mine")).on(!everyone).with(|e| {
            e.on_press(|st: &mut State| st.stats.items_everyone = false)
        }))
        .part(crate::ui::SegPart::new("stats-items-everyone", tr("Everyone")).on(everyone).with(|e| {
            e.on_press(|st: &mut State| st.stats.items_everyone = true)
        }))
        .build(u)
        .self_start();
    col()
        .w_full()
        .gap(12.0)
        .child(pick)
        .child(if everyone { w4::meta_section(s, u) } else { more::items_section(s, u) })
}

fn account_button(s: &State, u: &Ui, account_id: u64, name: &str, open: bool) -> El {
    row()
        .key("stats-account-button")
        .h(32.0)
        .pl(4.0)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair_strong)
        .fill(if open { u.selected } else { Color::WHITE.alpha(0.035) })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(tr("Switch player"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            st.stats.account_open = !st.stats.account_open;
            st.stats.search.clear();
            st.stats.search_note = None;
        })
        .child(shared::player_avatar(s, u, account_id, name, 24.0))
        .child(
            text(name.to_string())
                .font(BODY_MED)
                .color(u.ink)
                .max_w(120.0)
                .truncate(),
        )
        .child(icon(&icons::CHEVRON_DOWN, 14.0).color(u.muted).a11y_hidden())
}

/// The account input; a width of 0 grows to fill the row.
fn account_field(s: &State, u: &Ui, width: f32) -> El {
    let field = row();
    let field = if width > 0.0 {
        field.w(width)
    } else {
        field.grow(1.0).min_w(0.0)
    };
    field
        .h(32.0)
        .pl(10.0)
        .pr(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(icon(&icons::USER, 14.0).color(u.muted).a11y_hidden())
        .child(
            rune::text_input(&s.stats.account, BODY)
                .key("stats-account")
                .label(tr("Steam profile link, account id or name"))
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .color(u.ink)
                .placeholder(tr("Steam profile link, account id or name"), u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(|st: &mut State| &mut st.stats.account)
                .on_key(|st: &mut State, ev: &KeyEvent, _| {
                    if ev.key == Key::Enter {
                        crate::data::track_stats_account(st);
                        true
                    } else {
                        false
                    }
                }),
        )
}

/// Tracked players, or the results of a name search.
fn account_list(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let mut list = col().w_full();
    if let Some(note) = &v.search_note {
        list = list.child(
            text(note.clone())
                .font(SMALL)
                .color(u.muted)
                .wrap()
                .pb(8.0),
        );
    }
    let current = v.snapshot.as_ref().map(|snap| snap.account_id);
    // (id, name, tag, primary, id shown)
    let rows: Vec<(u64, String, String, bool)> = if !v.search.is_empty() {
        v.search
            .iter()
            .map(|profile| {
                (
                    profile.account_id,
                    profile.persona_name.clone(),
                    crate::screens::settings::i18n::fill(
                        tr("{} matches in the last 30 days"),
                        &[&profile.matches_played_last_30d.to_string()],
                    ),
                    false,
                )
            })
            .collect()
    } else {
        v.remembered
            .iter()
            .map(|row| {
                let mut meta = Vec::new();
                if row.primary {
                    meta.push(tr("Primary").to_string());
                }
                if Some(row.account_id) == current {
                    meta.push(tr("Showing").to_string());
                }
                (row.account_id, row.name.clone(), meta.join(" · "), row.primary)
            })
            .collect()
    };
    let searching = !v.search.is_empty();
    for (account_id, name, meta, primary) in rows {
        let mut r = row()
            .key(format!("stats-account-{account_id}"))
            .w_full()
            .h(40.0)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(crate::screens::settings::i18n::fill(tr("Load {}"), &[&name]))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| crate::data::load_stats_account(st, account_id))
            .child(avatar(u, &name, 24.0))
            .child(
                text(name)
                    .font(BODY_MED)
                    .color(u.ink)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        // Fixed cells: the id, the tags, then three button slots reserved when empty, so the
        // icons line up down the list.
        if searching {
            r = r.child(text(meta).font(TNUM_SMALL).color(u.muted).truncate());
        } else {
            r = r
                .child(text(account_id.to_string()).font(TNUM_SMALL).color(u.muted).w(72.0).align(rune::Align::End))
                .child(row().w(104.0).child(text(meta).font(SMALL).color(u.muted).truncate().min_w(0.0).shrink()));
        }
        r = r.child(shared::statlocker_link(
            u,
            format!("stats-statlocker-{account_id}"),
            shared::statlocker_profile_url(account_id),
            tr("Open on Statlocker"),
        ));
        if !searching {
            // The primary row keeps the pin's room with an invisible copy of the button.
            r = r.child(if primary {
                u.icon_button(&icons::PIN, "").opacity(0.0).focusable(false).a11y_hidden()
            } else {
                u.icon_button(&icons::PIN, tr("Make this the primary account"))
                    .key(format!("stats-primary-{account_id}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| crate::data::set_primary_account(st, account_id))
            });
            r = r.child(
                u.icon_button(&icons::X, tr("Stop tracking this player"))
                    .key(format!("stats-forget-{account_id}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        crate::data::forget_stats_account(st, account_id)
                    }),
            );
        }
        list = list.child(r);
    }
    list
}

/// Steam accounts this computer has signed in with, when any are not tracked yet.
fn steam_list(s: &State, u: &Ui) -> Option<El> {
    let v = &s.stats;
    let found: Vec<&SteamUser> = v
        .steam
        .iter()
        .filter(|user| {
            !v.remembered
                .iter()
                .any(|row| row.account_id == user.account_id as u64)
        })
        .collect();
    if found.is_empty() {
        return None;
    }
    let mut list = col().w_full().pt(8.0).child(
        row().h(32.0).px(8.0).center_y().child(text(tr("Found on this computer")).font(LABEL).color(u.muted)),
    );
    for user in found {
        let account_id = user.account_id as u64;
        let name = user.persona.clone();
        list = list.child(
            row()
                .key(format!("stats-steam-{account_id}"))
                .w_full()
                .h(40.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .child(avatar(u, &name, 24.0))
                .child(
                    text(name.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .truncate()
                        .min_w(0.0),
                )
                .child(
                    text(if user.most_recent {
                        tr("Last signed in")
                    } else {
                        tr("Signed in before")
                    })
                    .font(TNUM_SMALL)
                    .color(u.muted),
                )
                .child(spacer())
                .child(
                    u.action(tr("Track"), ActionKind::Secondary)
                        .w(72.0)
                        .key(format!("stats-steam-add-{account_id}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::data::load_stats_account(st, account_id)
                        }),
                ),
        );
    }
    Some(list)
}

/// The inline switcher under the page head.
/// Tracked players, in a popover under the account button: opening it never moves the page.
fn account_popover(s: &State, u: &Ui) -> El {
    let mut list = col()
        .w_full()
        .gap(8.0)
        .child(account_list(s, u));
    if let Some(steam) = steam_list(s, u) {
        list = list.child(steam);
    }
    col()
        .key("stats-account-panel")
        .anchored("stats-account-button", rune::Side::Bottom, rune::Align::End, 4.0)
        .transition(crate::motion::pop(false))
        .w(460.0)
        .max_h(480.0)
        .padding(12.0)
        .gap(12.0)
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair_strong)
        .shadow(4.0, 16.0, Color::BLACK.alpha(0.35))
        .role("dialog")
        .label(tr("Tracked players"))
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.stats.account_open = false;
                true
            } else {
                false
            }
        })
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(account_field(s, u, 0.0).grow(1.0).min_w(0.0))
                .child(
                    u.action(tr("Track and show"), ActionKind::Secondary)
                        .key("stats-load")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::track_stats_account(st)),
                ),
        )
        .child(
            u.scroll_area(col()
                .w_full()
                .min_h(0.0)
                .shrink()
                .scroll(ScrollSpec { stick_bottom: false })
                .child(list)),
        )
}

/// One Steam account found on this PC, as a large row: picture, name, Track. One press loads
/// the page; nothing is read from the network before it.
fn steam_row(s: &State, u: &Ui, user: &SteamUser) -> El {
    let account_id = user.account_id as u64;
    row()
        .key(format!("stats-steam-{account_id}"))
        .w_full()
        .h(64.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair)
        .child(shared::player_avatar(s, u, account_id, &user.persona, 40.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(user.persona.clone()).font(crate::ui::H2).color(u.ink).truncate())
                .child(
                    text(if user.most_recent { tr("Signed in on this PC, most recently") } else { tr("Signed in on this PC before") })
                        .font(SMALL)
                        .color(u.muted)
                        .truncate(),
                ),
        )
        .child(
            u.action(tr("Track"), ActionKind::Primary)
                .w(96.0)
                .key(format!("stats-steam-add-{account_id}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| crate::data::load_stats_account(st, account_id)),
        )
}

/// The start page: the Steam accounts on this PC, one press each. Typing an id or a link is
/// the fallback when none is found.
fn entry(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let untracked: Vec<&SteamUser> = v
        .steam
        .iter()
        .filter(|user| !v.remembered.iter().any(|row| row.account_id == user.account_id as u64))
        .collect();
    let found = !untracked.is_empty();
    let mut page = col()
        .key("stats-entry")
        .w(560.0)
        .pt(8.0)
        .gap(12.0)
        .child(text(tr("See how you are playing")).font(H1).color(u.ink));
    if found {
        page = page.child(
            text(if untracked.len() == 1 {
                tr("This Steam account is signed in on this PC. Is it you?")
            } else {
                tr("These Steam accounts are signed in on this PC. Pick yours.")
            })
            .font(BODY)
            .color(u.prose),
        );
        for user in &untracked {
            page = page.child(steam_row(s, u, user));
        }
    }
    // The error line keeps its room, so the lists under it never jump.
    page = page.child(match &v.error {
        Some(error) => problem(u, error),
        None => row().h(4.0),
    });
    let fallback = if found {
        tr("Not you? Paste a Steam profile link, SteamID or Deadlock account id.")
    } else {
        tr("Paste your Steam profile link, SteamID or Deadlock account id.")
    };
    page = page.child(
        col()
            .w_full()
            .gap(8.0)
            .pt(if found { 12.0 } else { 0.0 })
            .child(text(fallback).font(if found { SMALL } else { BODY }).color(if found { u.muted } else { u.prose }))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(account_field(s, u, 0.0))
                    .child(
                        u.action(tr("Show stats"), if found { ActionKind::Secondary } else { ActionKind::Primary })
                            .key("stats-load")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| crate::data::track_stats_account(st)),
                    ),
            ),
    );
    if !v.remembered.is_empty() || !v.search.is_empty() || v.search_note.is_some() {
        page = page.child(
            col()
                .w_full()
                .gap(4.0)
                .pt(16.0)
                .child(
                    text(if v.search.is_empty() { tr("Recent") } else { tr("Players with that name") })
                        .font(LABEL)
                        .color(u.muted)
                        .pb(4.0),
                )
                .child(account_list(s, u).mx(-8.0)),
        );
    }
    page
}

/// The Overview's own boxes, empty, while the first load runs: band 72, tiles 96, the
/// insight head and rows, the chart, then the two lists. Nothing here animates.
fn skeleton(u: &Ui) -> El {
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(crate::ui::r(4.0)).fill(u.hover);
    let panel = |h: f32| {
        row()
            .h(h)
            .padding(12.0)
            .gap(12.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .fill(u.raised)
            .border(1.0, u.hair)
    };
    let rows = |n: usize, h: f32| {
        let mut c = col().w_full();
        for i in 0..n {
            c = c.child(
                row()
                    .w_full()
                    .h(h)
                    .gap(12.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(bar(24.0, 24.0))
                    .child(bar(120.0 + (i % 3) as f32 * 40.0, 12.0)),
            );
        }
        c
    };
    let list = || col().w(0.0).grow(1.0).min_w(440.0).child(row().h(32.0).center_y().child(bar(120.0, 14.0))).child(rows(5, 40.0));
    col()
        .key("stats-skeleton")
        .w_full()
        .gap(8.0)
        .child(panel(72.0).w_full().child(bar(48.0, 48.0)).child(col().gap(6.0).child(bar(120.0, 16.0)).child(bar(160.0, 12.0))))
        .child(row().w_full().gap(8.0).children((0..4).map(|_| {
            panel(96.0)
                .w(0.0)
                .grow(1.0)
                .min_w(0.0)
                .child(col().gap(8.0).child(bar(72.0, 12.0)).child(bar(64.0, 24.0)).child(bar(96.0, 12.0)))
        })))
        .child(col().w_full().mt(16.0).child(row().h(32.0).center_y().child(bar(220.0, 14.0))).child(rows(4, 44.0)))
        .child(el().w_full().mt(16.0).h(32.0 + 12.0 + brief::CHART_H).radius(crate::ui::r(4.0)).fill(u.hover.alpha(0.5)))
        .child(row().w_full().mt(16.0).gap(24.0).flex_wrap().child(list()).child(list()))
}

/// Mounted at the root: the live bridge dialogs, or the review panel, over the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(dialog) = live::dialog(s, u) {
        return Some(dialog);
    }
    s.stats
        .review
        .as_ref()
        .map(|review| demos::review_dialog(s, u, review))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_fixture_parses() {
        let captured = deadlock_core::stats::snapshot_from_bundle(FIXTURE).expect("snapshot");
        assert!(!captured.snapshot.matches.is_empty());
        assert!(!captured.snapshot.heroes.is_empty());
        assert!(captured.snapshot.rank.badge > 0);
    }

    #[test]
    fn the_extra_fixture_parses_through_all_three_readers() {
        let board = deadlock_core::stats::board::scoreboard_from_bundle(EXTRA).expect("board");
        assert_eq!(board.team(0).len(), 6);
        assert_eq!(board.team(1).len(), 6);
        assert!(!board.souls.is_empty());
        assert!(board.souls.windows(2).all(|pair| pair[0].time_s < pair[1].time_s));
        assert!(board.team(0).iter().any(|player| player.mvp_rank.is_some()));
        assert!(
            board
                .teams
                .iter()
                .flatten()
                .any(|player| !player.build.is_empty())
        );
        assert!(
            board
                .teams
                .iter()
                .flatten()
                .all(|player| player.persona_name.is_some() || player.account_id == 0)
        );

        let social = deadlock_core::stats::social::social_from_bundle(EXTRA).expect("social");
        assert!(!social.enemies.is_empty());
        assert!(social.mates.iter().all(|row| row.matches >= 3));
        assert!(social.enemies.iter().any(|row| row.persona_name.is_some()));

        let board = deadlock_core::stats::leaderboard::board_from_bundle(EXTRA).expect("board");
        assert!(board.board.entries.len() > LEADERBOARD_ROWS);
        assert!(board.board.place_of(board.account_id).is_some());
        assert!(board.board.season_id.is_some());
    }
}

