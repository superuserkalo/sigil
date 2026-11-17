//! Stats: how you are playing now, and what to work on next.
//!
//! Every number comes from a `StatsSnapshot` read from `api.deadlock-api.com`
//! (or the captured fixture in `fixtures/stats-profile.json` for snapshots).
//! The page reads top to bottom as one argument:
//!
//! 1. The brief: what changed (rank, streak, form) and the one thing to work on
//!    next, from `deadlock_core::stats::insight`.
//! 2. Four numbers for the newest 20 matches against the matches before, with
//!    the rank tier's average where the API has one.
//! 3. Progress: the rank ladder or the rolling win rate, and the last 30 results.
//! 4. Leaderboard: the region and season's board, with the player's place marked.
//! 5. Heroes against the player's own average and against their rank tier.
//! 6. Matches grouped by sitting; a row opens to the match, a scoreboard for all
//!    twelve players, and a reviewed demo opens to the moment.
//! 7. Players you meet: teammates and opponents with the shared sample.
//! 8. Where kills and deaths happen on the map, over the API's 30 day window.
//!
//! No aim, impact or positioning score is shown: the API does not carry one.

use std::time::{Duration, Instant};

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
mod brief;
mod demos;
mod german;
mod heroes;
mod leaderboard;
mod map;
mod matches;
mod shared;
mod social;

use german::tr;
use shared::{avatar, problem, TNUM_SMALL};

/// Heatmap grid resolution, cells per side.
pub const HEAT_CELLS: usize = 48;
const HEROES_SHOWN: usize = 6;
const SITTINGS_SHOWN: usize = 3;
const FORM_BEADS: usize = 30;
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

pub struct View {
    pub loaded: bool,
    pub account: TextEdit,
    pub error: Option<String>,
    pub snapshot: Option<StatsSnapshot>,
    pub derived: Option<Derived>,
    pub heat: Option<HeatGrid>,
    pub heat_error: Option<String>,
    pub objectives: Vec<MapObjective>,
    pub expanded: Option<u64>,
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
    pub board: Fetched<MatchScoreboard>,
    /// The match whose scoreboard is open, if any.
    pub board_open: Option<u64>,
    pub board_job: Job<MatchScoreboard>,
    pub social: Fetched<Social>,
    pub social_job: Job<Social>,
    pub social_all: bool,
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
            loaded: false,
            account: TextEdit::new(""),
            error: None,
            snapshot: None,
            derived: None,
            heat: None,
            heat_error: None,
            objectives: Vec::new(),
            expanded: None,
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
            board_open: None,
            board_job: Job::default(),
            social: Fetched::Idle,
            social_job: Job::default(),
            social_all: false,
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

/// Store a snapshot and derive everything the view draws from it.
pub fn set_snapshot(view: &mut View, snapshot: StatsSnapshot) {
    let matches = &snapshot.matches;
    let trajectory = insight::rank_trajectory(matches);
    view.chart = if trajectory.len() >= 2 {
        Chart::Rank
    } else {
        Chart::WinRate
    };
    view.derived = Some(Derived {
        brief: insight::brief(&snapshot),
        trajectory,
        rolling: insight::rolling_win_rate(matches, insight::RECENT_MATCHES),
        sessions: insight::sessions(matches, insight::SESSION_GAP_SECONDS),
        form: insight::form(matches),
        streak: insight::current_streak(matches),
    });
    view.expanded = None;
    view.sittings_shown = SITTINGS_SHOWN;
    view.all_heroes = false;
    view.board = Fetched::Idle;
    view.board_open = None;
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
    if let Some(result) = state.stats.board_job.try_take() {
        state.stats.board = match result {
            Ok(board) => Fetched::Ready(board),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
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

/// The profile with the newest match opened and the kill layer on.
pub fn scene_drill(s: &mut State) {
    load_fixture(s);
    s.stats.expanded = s
        .stats
        .snapshot
        .as_ref()
        .and_then(|snap| snap.matches.first().map(|item| item.match_id));
    s.stats.chart = Chart::WinRate;
    s.stats.layer = Layer::Kills;
    s.stats.account_open = true;
    s.stats.all_heroes = true;
}

/// The newest match's scoreboard over the page.
pub fn scene_board(s: &mut State) {
    load_fixture(s);
    let Ok(board) = deadlock_core::stats::board::scoreboard_from_bundle(EXTRA) else {
        return;
    };
    s.stats.expanded = Some(board.match_id);
    s.stats.board_open = Some(board.match_id);
    s.stats.board = Fetched::Ready(board);
}

/// Teammates, opponents and the leaderboard, from the fixture.
pub fn scene_people(s: &mut State) {
    load_fixture(s);
    if let Ok(social) = deadlock_core::stats::social::social_from_bundle(EXTRA) {
        s.stats.social = Fetched::Ready(social);
    }
    if let Ok(captured) = deadlock_core::stats::leaderboard::board_from_bundle(EXTRA) {
        s.stats.region = Some(captured.region);
        s.stats.seasons = captured.seasons;
        s.stats.leaderboard = Fetched::Ready(captured.board);
    }
}

/// The account panel open, with tracked players and Steam accounts to add.
pub fn scene_tracked(s: &mut State) {
    load_fixture(s);
    s.stats.account_open = true;
}

/// First run: no account yet.
pub fn scene_entry(s: &mut State) {
    s.screen = crate::model::Screen::Stats;
    s.stats.loaded = true;
    s.stats.remembered = vec![Remembered {
        account_id: 40_312_977,
        name: "Wren".to_string(),
        primary: true,
    }];
    s.stats.steam = vec![SteamUser {
        steam_id64: deadlock_core::steam_users::account_to_id64(40_312_977),
        account_id: 40_312_977,
        persona: "Wren".to_string(),
        most_recent: true,
    }];
}

// --- page ------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let loading = s.stats_job.is_running();
    let ready = v.can_refresh();

    let mut actions = row().gap(8.0).center_y();
    if let Some(snap) = &v.snapshot {
        actions = actions.child(account_button(u, &snap.profile, v.account_open));
        actions = actions.child(
            u.icon_button(&icons::REFRESH_CW, tr("Refresh"))
                .key("stats-refresh")
                .opacity(if loading || !ready { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| crate::data::start_stats_load(st)),
        );
    }

    let mut page = col()
        .key("stats-page")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(48.0)
        .gap(24.0)
        .child(u.page_head(
            "",
            tr("Stats"),
            match (&v.snapshot, loading) {
                (Some(_), true) => tr("Refreshing"),
                (Some(_), false) if v.age.stale => tr("Showing an older copy"),
                (Some(_), false) => "",
                _ => "",
            },
            Some(actions),
        ));

    if v.account_open && v.snapshot.is_some() {
        page = page.child(account_panel(s, u));
    }
    if let (Some(error), Some(_)) = (&v.error, &v.snapshot) {
        page = page.child(problem(u, error));
    }

    page = match (&v.snapshot, &v.derived) {
        (Some(snap), Some(_)) if snap.matches.is_empty() => page.child(u.empty_state(
            "",
            &crate::screens::settings::i18n::fill(
                tr("No matches for {} yet"),
                &[&snap.profile.persona_name],
            ),
            tr("The API only knows matches that finished. New ones show up a little while after they end."),
            None,
        )),
        (Some(snap), Some(derived)) => page
            .child(brief::brief_block(s, u, snap, derived))
            .child(brief::numbers(u, snap, derived))
            .child(brief::progress(s, u, snap, derived))
            .child(leaderboard::leaderboard_section(s, u, snap))
            .child(heroes::heroes(s, u, snap))
            .child(matches::matches(s, u, snap, derived))
            .child(social::social_section(s, u))
            .child(map::map_section(s, u))
            .child(demos::demo_section(s, u)),
        _ if loading => page.child(skeleton(u)),
        _ => page.child(entry(s, u)),
    };

    col()
        .key("stats-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn account_button(u: &Ui, profile: &PlayerProfile, open: bool) -> El {
    row()
        .key("stats-account-button")
        .h(32.0)
        .pl(4.0)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
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
        .child(avatar(u, &profile.persona_name, 24.0))
        .child(
            text(profile.persona_name.clone())
                .font(BODY_MED)
                .color(u.ink)
                .max_w(160.0)
                .truncate(),
        )
        .child(icon(&icons::CHEVRON_DOWN, 14.0).color(u.faint).a11y_hidden())
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
        .child(icon(&icons::USER, 14.0).color(u.faint).a11y_hidden())
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
                let mut meta = vec![row.account_id.to_string()];
                if row.primary {
                    meta.push(tr("Primary").to_string());
                }
                if Some(row.account_id) == current {
                    meta.push(tr("Showing").to_string());
                }
                (row.account_id, row.name.clone(), meta.join("  ·  "), row.primary)
            })
            .collect()
    };
    let searching = !v.search.is_empty();
    for (account_id, name, meta, primary) in rows {
        let mut r = row()
            .key(format!("stats-account-{account_id}"))
            .w_full()
            .h(44.0)
            .px(8.0)
            .gap(12.0)
            .center_y()
            .radius(crate::ui::r(8.0))
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
                    .min_w(0.0),
            )
            .child(spacer())
            .child(text(meta).font(TNUM_SMALL).color(u.faint));
        if !searching {
            if !primary {
                r = r.child(
                    u.icon_button(&icons::PIN, tr("Make this the primary account"))
                        .key(format!("stats-primary-{account_id}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::data::set_primary_account(st, account_id)
                        }),
                );
            }
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
    let mut list = col().w_full().pt(8.0).gap(2.0).child(
        text(tr("Found on this computer"))
            .font(LABEL)
            .color(u.muted)
            .pb(4.0),
    );
    for user in found {
        let account_id = user.account_id as u64;
        let name = user.persona.clone();
        list = list.child(
            row()
                .key(format!("stats-steam-{account_id}"))
                .w_full()
                .h(44.0)
                .px(8.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
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
                    .color(u.faint),
                )
                .child(spacer())
                .child(
                    u.action(tr("Track"), ActionKind::Secondary)
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
fn account_panel(s: &State, u: &Ui) -> El {
    let mut panel = col()
        .key("stats-account-panel")
        .w_full()
        .gap(8.0)
        .pb(16.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(account_field(s, u, 360.0))
                .child(
                    u.action(tr("Track and show"), ActionKind::Secondary)
                        .key("stats-load")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::track_stats_account(st)),
                ),
        )
        .child(
            col()
                .w_full()
                .max_w(620.0)
                .child(account_list(s, u)),
        );
    if let Some(steam) = steam_list(s, u) {
        panel = panel.child(col().w_full().max_w(620.0).child(steam));
    }
    panel
}

/// First run and no account: one field, one action, the tracked list.
fn entry(s: &State, u: &Ui) -> El {
    let v = &s.stats;
    let mut form = col()
        .w(480.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(account_field(s, u, 0.0))
                .child(
                    u.action(tr("Show stats"), ActionKind::Primary)
                        .key("stats-load")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| crate::data::track_stats_account(st)),
                ),
        )
        .child(
            text(tr("Profile links work in both forms: /profiles/ with the number, or /id/ with a name."))
                .font(SMALL)
                .color(u.faint),
        );
    if let Some(error) = &v.error {
        form = form.child(problem(u, error).pt(8.0));
    }
    if let Some(steam) = steam_list(s, u) {
        form = form.child(
            col()
                .w_full()
                .pt(16.0)
                .child(steam),
        );
    }
    if !v.remembered.is_empty() || !v.search.is_empty() || v.search_note.is_some() {
        form = form.child(
            col()
                .w_full()
                .gap(4.0)
                .pt(24.0)
                .child(
                    text(if v.search.is_empty() {
                        tr("Recent")
                    } else {
                        tr("Players with that name")
                    })
                    .font(LABEL)
                    .color(u.muted)
                    .pb(4.0),
                )
                .child(account_list(s, u).mx(-8.0)),
        );
    }
    col()
        .key("stats-entry")
        .w_full()
        .pt(64.0)
        .gap(24.0)
        .items_center()
        .child(
            col()
                .gap(6.0)
                .items_center()
                .child(
                    text(tr("See how you are playing"))
                        .font(H1)
                        .color(u.ink)
                        .align(rune::Align::Center),
                )
                .child(
                    text(tr("Paste your Steam profile link, SteamID or Deadlock account id."))
                        .font(BODY)
                        .color(u.muted)
                        .align(rune::Align::Center),
                ),
        )
        .child(form)
}

/// Placeholder shapes in the page's own layout while the first load runs.
fn skeleton(u: &Ui) -> El {
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(4.0).fill(u.hover);
    let mut c = col()
        .key("stats-skeleton")
        .w_full()
        .gap(24.0)
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(el().size(40.0, 40.0).radius(20.0).fill(u.hover))
                .child(col().gap(8.0).child(bar(140.0, 14.0)).child(bar(280.0, 10.0))),
        )
        .child(col().gap(8.0).child(bar(360.0, 20.0)).child(bar(260.0, 10.0)))
        .child(row().w_full().gap(32.0).children((0..4).map(|_| {
            col()
                .grow(1.0)
                .gap(8.0)
                .child(bar(64.0, 10.0))
                .child(bar(88.0, 18.0))
                .child(bar(120.0, 8.0))
        })))
        .child(
            el()
                .w_full()
                .h(brief::CHART_H)
                .radius(crate::ui::r(10.0))
                .fill(Color::WHITE.alpha(0.025)),
        );
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for n in 0..4 {
        rows = rows.child(
            row()
                .w_full()
                .h(48.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(el().size(28.0, 28.0).radius(crate::ui::r(6.0)).fill(u.hover))
                .child(bar(120.0 + (n % 3) as f32 * 40.0, 10.0)),
        );
    }
    c = c.child(rows);
    c
}

/// Mounted at the root: the scoreboard, or the review panel, over the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(match_id) = s.stats.board_open {
        return Some(board::board_dialog(s, u, match_id));
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

