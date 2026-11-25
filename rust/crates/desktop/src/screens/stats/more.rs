//! Deadlock Mod Manager's Stats additions: the ranked and date filter, the Items view and the
//! Party view. Grimoire has the raw item endpoint only, no player against everyone join, no
//! filter and no party rating.
//!
//! - The filter (ranked, unranked or both, and a start date) scopes Items and Party on the
//!   server (`stats::filters::filter_query`) and the Matches list here, from each match's mode
//!   and start time.
//! - Items: the player's item win rates against everyone's in the same scope, and how early
//!   they buy each item (`stats::items::item_stats`). An open match also shows its purchase
//!   timeline (`item_timeline`).
//! - Party: every lineup a party can queue from its members' most played heroes, rated from
//!   pair and trio records (`stats::party::lineup_ratings`). Members are the player plus the
//!   tracked players and premades picked on the page.
//!
//! Every read runs on a worker after a press, like the teammates list: each one is several
//! requests to the shared API.

use std::collections::BTreeSet;

use deadlock_core::stats::filters::StatsFilter;
use deadlock_core::stats::items::{ItemPurchase, ItemStat};
use deadlock_core::stats::party::LineupRating;
use deadlock_core::stats::MatchSummary;
use rune::{El, KeyEvent, col, icon, row, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

/// Rows Items shows before "Show all".
const ITEM_ROWS: usize = 12;
/// Lineups Party shows.
const LINEUP_ROWS: usize = 10;

/// The date choices: label and days back. 0 is the API's own window.
pub const SINCE: [(&str, u64); 4] = [("Any time", 0), ("Last 7 days", 7), ("Last 30 days", 30), ("Last 90 days", 90)];

/// Items with their names, joined on the worker.
pub struct ItemsLoad {
    pub rows: Vec<ItemStat>,
    pub names: std::collections::HashMap<u64, String>,
}

/// The filter, the three reads and the party picks, kept in Stats' view.
pub struct More {
    /// `Some(true)` ranked, `Some(false)` unranked, `None` both.
    pub ranked: Option<bool>,
    /// Days back, 0 for the API's window.
    pub days: u64,
    pub items: Fetched<ItemsLoad>,
    pub items_job: Job<ItemsLoad>,
    pub items_all: bool,
    pub timeline: Fetched<(u64, Vec<(ItemPurchase, String)>)>,
    pub timeline_job: Job<(u64, Vec<(ItemPurchase, String)>)>,
    pub party: Fetched<Vec<LineupRating>>,
    pub party_job: Job<Vec<LineupRating>>,
    /// Other players picked for the party, by account id.
    pub members: BTreeSet<u64>,
    /// Street Brawl games in the match list: shown (tagged), hidden or alone.
    pub brawl: Brawl,
    /// A ranked season picked as the window (DMM #1391): its leaderboard id, start and end.
    /// It replaces the days choice; picking days clears it.
    pub season: Option<SeasonWindow>,
    /// The ranked season list, read once on a press (`/v1/assets/ranked-seasons`).
    pub seasons_job: Job<Vec<deadlock_core::stats::leaderboard::RankedSeason>>,
    pub seasons_note: Option<String>,
    /// The Season menu under the filter row is open.
    pub season_menu: bool,
}

/// A season window: matches from `start` up to, not including, `end` (Unix seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeasonWindow {
    pub id: u32,
    pub start: u64,
    pub end: u64,
}

/// Street Brawl in the match list (owner, 2026-11-21: listed with a tag, as Grimoire does).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Brawl {
    #[default]
    Show,
    Hide,
    Only,
}

impl Default for More {
    fn default() -> Self {
        More {
            ranked: None,
            days: 0,
            items: Fetched::Idle,
            items_job: Job::default(),
            items_all: false,
            timeline: Fetched::Idle,
            timeline_job: Job::default(),
            party: Fetched::Idle,
            party_job: Job::default(),
            members: BTreeSet::new(),
            brawl: Brawl::Show,
            season: None,
            seasons_job: Job::default(),
            seasons_note: None,
            season_menu: false,
        }
    }
}

impl More {
    /// The filter the API reads, from the choices and the clock.
    pub fn filter(&self, now: u64) -> StatsFilter {
        StatsFilter {
            ranked: self.ranked,
            since: match self.season {
                Some(season) => Some(season.start),
                None => (self.days > 0).then(|| now.saturating_sub(self.days * 86_400)),
            },
            // A season still running has no end yet: the window stays open to now.
            until: self.season.map(|season| season.end).filter(|end| *end > 0 && *end <= now),
        }
    }
}

/// Does a match pass the filter? Mode 4 is ranked, 1 unranked; other modes pass "both" only.
pub fn keeps(more: &More, item: &MatchSummary, now: i64) -> bool {
    let brawl = item.is_street_brawl();
    let brawl_ok = match more.brawl {
        Brawl::Show => true,
        Brawl::Hide => !brawl,
        Brawl::Only => brawl,
    };
    // Street Brawl is never ranked: it shows under "both" and "unranked".
    let mode = if brawl { 1 } else { item.match_mode };
    brawl_ok && keeps_mode_at(more, mode, item.start_time, now)
}

fn keeps_mode_at(more: &More, match_mode: i32, start_time: i64, now: i64) -> bool {
    let mode_ok = match more.ranked {
        None => true,
        Some(true) => match_mode == 4,
        Some(false) => match_mode == 1,
    };
    let window_ok = match more.season {
        Some(season) => start_time >= season.start as i64 && (season.end == 0 || start_time < season.end as i64),
        None => more.days == 0 || start_time >= now - (more.days as i64) * 86_400,
    };
    mode_ok && window_ok
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- loads -------------------------------------------------------------------------------

fn allowed(st: &State) -> Result<u32, String> {
    if !st.prefs.allow_stats {
        return Err(tr("Player stats are turned off in Settings, Privacy.").to_string());
    }
    st.stats
        .snapshot
        .as_ref()
        .and_then(|snap| u32::try_from(snap.account_id).ok())
        .ok_or_else(|| tr("Load a player first.").to_string())
}

/// Read the player's item rows for the current filter.
pub fn start_items(st: &mut State) {
    if st.stats.more.items_job.is_running() {
        return;
    }
    let account = match allowed(st) {
        Ok(account) => account,
        Err(error) => {
            st.stats.more.items = Fetched::Failed(error);
            return;
        }
    };
    let filter = st.stats.more.filter(now());
    st.stats.more.items = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.more.items_job.start(wake, move || {
        let rows = deadlock_core::stats::items::item_stats(account, &filter).map_err(|e| e.to_string())?;
        let catalog = crate::data::stats_client().fetch_item_catalog().unwrap_or_default();
        let names = rows
            .iter()
            .filter_map(|row| catalog.get(row.item_id).map(|(name, _, _)| (row.item_id, name.clone())))
            .collect();
        Ok(ItemsLoad { rows, names })
    });
}

/// Read one match's purchases for the player.
pub fn start_timeline(st: &mut State, match_id: u64) {
    if st.stats.more.timeline_job.is_running() {
        return;
    }
    let account = match allowed(st) {
        Ok(account) => account,
        Err(error) => {
            st.stats.more.timeline = Fetched::Failed(error);
            return;
        }
    };
    st.stats.more.timeline = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.more.timeline_job.start(wake, move || {
        let bought = deadlock_core::stats::items::item_timeline(match_id, account).map_err(|e| e.to_string())?;
        let catalog = crate::data::stats_client().fetch_item_catalog().unwrap_or_default();
        let rows = bought
            .into_iter()
            .map(|p| {
                let name = catalog
                    .get(p.item_id)
                    .map(|(name, _, _)| name.clone())
                    .unwrap_or_else(|| format!("Item {}", p.item_id));
                (p, name)
            })
            .collect();
        Ok((match_id, rows))
    });
}

/// Rate the party's lineups for the current filter.
pub fn start_party(st: &mut State) {
    if st.stats.more.party_job.is_running() {
        return;
    }
    let account = match allowed(st) {
        Ok(account) => account,
        Err(error) => {
            st.stats.more.party = Fetched::Failed(error);
            return;
        }
    };
    let mut party = vec![account];
    party.extend(st.stats.more.members.iter().filter_map(|id| u32::try_from(*id).ok()).filter(|id| *id != account));
    party.truncate(deadlock_core::stats::party::MAX_PARTY_SIZE);
    if party.len() < 2 {
        st.stats.more.party = Fetched::Failed(tr("Pick at least one other player for the party.").to_string());
        return;
    }
    let filter = st.stats.more.filter(now());
    st.stats.more.party = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.more.party_job.start(wake, move || {
        deadlock_core::stats::party::lineup_ratings(&party, &filter).map_err(|e| e.to_string())
    });
}

/// Take finished reads. Called from Stats' poll.
pub fn poll(st: &mut State) -> bool {
    let m = &mut st.stats.more;
    let mut changed = false;
    if let Some(result) = m.items_job.try_take() {
        m.items = match result {
            Ok(load) => Fetched::Ready(load),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = m.timeline_job.try_take() {
        m.timeline = match result {
            Ok(rows) => Fetched::Ready(rows),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = m.party_job.try_take() {
        m.party = match result {
            Ok(rows) => Fetched::Ready(rows),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = st.stats.more.seasons_job.try_take() {
        match result {
            Ok(seasons) if seasons.is_empty() => {
                st.stats.more.seasons_note = Some(tr("The API lists no ranked season yet.").to_string())
            }
            Ok(seasons) => {
                st.stats.more.seasons_note = None;
                st.stats.seasons = seasons;
            }
            Err(error) => st.stats.more.seasons_note = Some(error),
        }
        changed = true;
    }
    changed
}

/// Read the ranked season list for the season choice. Leaderboard reads fill the same list.
pub fn start_seasons(st: &mut State) {
    if st.stats.more.seasons_job.is_running() {
        return;
    }
    if !st.prefs.allow_stats {
        st.stats.more.seasons_note = Some(tr("Player stats are turned off in Settings, Privacy.").to_string());
        return;
    }
    st.stats.more.seasons_note = None;
    let wake = st.wake.clone();
    st.stats.more.seasons_job.start(wake, move || {
        crate::data::stats_client().fetch_seasons().map_err(|error| error.to_string())
    });
}

/// The newest season picked, for snapshots.
pub fn stage_season(st: &mut State) {
    let first = season_choices(&st.stats.seasons, i64::MAX).into_iter().next();
    st.stats.more.season = first;
}

/// Pick a season as the window, or `None` to go back to the days choice.
fn pick_season(st: &mut State, season: Option<SeasonWindow>) {
    st.stats.more.season = season;
    if season.is_some() {
        st.stats.more.days = 0;
    }
    refilter(st);
}

/// Seasons the choice lists: the ones that have started, newest first, at most four.
fn season_choices(seasons: &[deadlock_core::stats::leaderboard::RankedSeason], now: i64) -> Vec<SeasonWindow> {
    seasons
        .iter()
        .filter(|season| season.has_started(now))
        .take(4)
        .map(|season| SeasonWindow {
            id: season.leaderboard_id,
            start: season.starts_at.max(0) as u64,
            end: season.ends_at.max(0) as u64,
        })
        .collect()
}

/// A new filter: the server reads are stale, so they go back to their Load button.
fn refilter(st: &mut State) {
    let m = &mut st.stats.more;
    if !m.items_job.is_running() {
        m.items = Fetched::Idle;
    }
    if !m.party_job.is_running() {
        m.party = Fetched::Idle;
    }
    // Wave 4: the overview and Heroes follow the filter too.
    super::w4::refilter(st);
}

// --- views -------------------------------------------------------------------------------

/// One choice inside a filter group: a short word on screen, the full name for assistive tech.
fn pill(key: String, label: &str, full: &str, on: bool) -> crate::ui::SegPart {
    crate::ui::SegPart::new(key, tr(label)).name(tr(full)).on(on)
}

/// A filter group: the shared segmented selector at 28 points.
fn strip(key: &str, name: &str, parts: Vec<crate::ui::SegPart>, u: &Ui) -> El {
    crate::ui::Segmented::new(key, tr(name)).size(crate::ui::SegSize::M).parts(parts).build(u)
}

fn group(u: &Ui) -> El {
    row().h(28.0).gap(2.0).padding(2.0).center_y().radius(crate::ui::r(4.0)).border(1.0, u.hair)
}

/// The filter row over the tabs it scopes: one line of three groups.
pub(super) fn filter_bar(s: &State, u: &Ui) -> El {
    let m = &s.stats.more;
    let mut modes = Vec::new();
    for (label, full, value) in [
        ("All", "Ranked and unranked", None),
        ("Ranked", "Ranked", Some(true)),
        ("Unranked", "Unranked", Some(false)),
    ] {
        modes.push(pill(format!("stats-filter-mode-{full}"), label, full, m.ranked == value).with(move |e| {
            e.on_press(move |st: &mut State| {
                st.stats.more.ranked = value;
                refilter(st);
            })
        }));
    }
    let modes = strip("stats-filter-mode", "Mode", modes, u);
    let mut since = Vec::new();
    for ((full, days), label) in SINCE.into_iter().zip(["All time", "7 d", "30 d", "90 d"]) {
        since.push(pill(format!("stats-filter-days-{days}"), label, full, m.season.is_none() && m.days == days).with(move |e| {
            e.on_press(move |st: &mut State| {
                st.stats.more.days = days;
                st.stats.more.season = None;
                refilter(st);
            })
        }));
    }
    let since = strip("stats-filter-days", "Window", since, u);
    // One Season segment opens the list of ranked seasons (`season_menu`).
    let season_name = m.season.and_then(|picked| {
        s.stats
            .seasons
            .iter()
            .find(|known| known.leaderboard_id == picked.id)
            .map(|known| known.name.clone())
    });
    let season_on = m.season.is_some();
    let season = group(u).child(
        row()
            .key("stats-filter-season")
            .w(96.0)
            .h(24.0)
            .px(8.0)
            .gap(4.0)
            .center_y()
            .radius(crate::ui::r(2.0))
            .fill(if season_on { u.selected } else { rune::Color::TRANSPARENT })
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .expanded(m.season_menu)
            .label(tr("Ranked seasons").to_string())
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.stats.more.season_menu = !st.stats.more.season_menu;
                if st.stats.more.season_menu && st.stats.seasons.is_empty() && !st.stats.more.seasons_job.is_running() {
                    start_seasons(st);
                }
            })
            .child(
                text(season_name.unwrap_or_else(|| tr("Season").to_string()))
                    .font(SMALL)
                    .color(if season_on { u.ink } else { u.muted })
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0)
                    .a11y_hidden(),
            )
            .child(icon(&crate::icons::CHEVRON_DOWN, 12.0).color(u.muted).a11y_hidden()),
    );
    let mut brawl = Vec::new();
    for (label, full, value) in [
        ("With", "With Street Brawl", Brawl::Show),
        ("Without", "Without Street Brawl", Brawl::Hide),
        ("Only", "Street Brawl only", Brawl::Only),
    ] {
        brawl.push(
            pill(format!("stats-filter-brawl-{full}"), label, full, m.brawl == value)
                .with(move |e| e.on_press(move |st: &mut State| st.stats.more.brawl = value)),
        );
    }
    let brawl = strip("stats-filter-brawl", "Street Brawl", brawl, u);
    row()
        .key("stats-filter")
        .w_full()
        .h(28.0)
        .gap(12.0)
        .center_y()
        .child(modes)
        .child(since)
        .child(season)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(tr("Street Brawl")).font(SMALL).color(u.muted))
                .child(brawl),
        )
}

/// The ranked seasons, under the Season segment. Mounted beside the page so it is not clipped.
pub(super) fn season_menu(s: &State, u: &Ui) -> Option<El> {
    let m = &s.stats.more;
    if !m.season_menu {
        return None;
    }
    let mut list = col()
        .key("stats-season-menu")
        .anchored("stats-filter-season", rune::Side::Bottom, rune::Align::Start, 4.0)
        .transition(crate::motion::pop(false))
        .w(200.0)
        .padding(4.0)
        .gap(0.0)
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair_strong)
        .shadow(4.0, 16.0, rune::Color::BLACK.alpha(0.35))
        .role("menu")
        .label(tr("Ranked seasons").to_string())
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == rune::Key::Escape {
                st.stats.more.season_menu = false;
                true
            } else {
                false
            }
        });
    let item = |key: String, label: String, on: bool| {
        row()
            .key(key)
            .w_full()
            .h(28.0)
            .px(8.0)
            .center_y()
            .radius(crate::ui::r(2.0))
            .fill(if on { u.selected } else { rune::Color::TRANSPARENT })
            .hover(u.hover)
            .focusable(true)
            .role("menuitemradio")
            .checked(on)
            .label(label.clone())
            .cursor(rune::Cursor::Pointer)
            .child(text(label).font(SMALL).color(if on { u.ink } else { u.prose }).truncate().a11y_hidden())
    };
    let choices = season_choices(&s.stats.seasons, now() as i64);
    if choices.is_empty() {
        let line = if m.seasons_job.is_running() {
            tr("Loading seasons").to_string()
        } else {
            m.seasons_note.clone().unwrap_or_else(|| tr("No seasons yet").to_string())
        };
        list = list.child(row().h(28.0).px(8.0).center_y().child(text(line).font(SMALL).color(u.muted).truncate()));
        return Some(list);
    }
    list = list.child(
        item("stats-filter-season-none".into(), tr("Any season").to_string(), m.season.is_none()).on_press(|st: &mut State| {
            st.stats.more.season_menu = false;
            pick_season(st, None);
        }),
    );
    for choice in choices {
        let name = s
            .stats
            .seasons
            .iter()
            .find(|known| known.leaderboard_id == choice.id)
            .map(|known| known.name.clone())
            .unwrap_or_default();
        let on = m.season.map(|picked| picked.id) == Some(choice.id);
        list = list.child(item(format!("stats-filter-season-{}", choice.id), name, on).on_press(move |st: &mut State| {
            st.stats.more.season_menu = false;
            pick_season(st, Some(choice));
        }));
    }
    Some(list)
}

fn minutes(seconds: f64) -> String {
    let s = seconds.round().max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn signed_points(delta: f64) -> String {
    if delta >= 0.05 {
        format!("+{delta:.1}")
    } else if delta <= -0.05 {
        format!("{delta:.1}")
    } else {
        "0.0".to_string()
    }
}

/// Items: the player's item rows against everyone's.
pub(super) fn items_section(s: &State, u: &Ui) -> El {
    let m = &s.stats.more;
    let mut section = col()
        .key("stats-items")
        .w_full()
        .gap(12.0)
        .child(section_head(u, "Items", "Your win rate with each item against everyone's, and when you buy it"));
    section = match &m.items {
        Fetched::Idle => section.child(
            row().w_full().gap(12.0).center_y().child(
                u.action(tr("Show items"), ActionKind::Secondary)
                    .key("stats-items-load")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(start_items),
            ),
        ),
        Fetched::Loading => section.child(text(tr("Loading items")).font(BODY).color(u.muted)),
        Fetched::Failed(error) => section.child(problem_retry(u, error, "stats-items-retry", start_items)),
        Fetched::Ready(load) if load.rows.is_empty() => {
            section.child(text(tr("No item has five matches in this filter yet.")).font(BODY).color(u.muted))
        }
        Fetched::Ready(load) => {
            let mut rows: Vec<&ItemStat> = load.rows.iter().collect();
            rows.sort_by_key(|row| std::cmp::Reverse(row.matches));
            let shown = if m.items_all { rows.len() } else { rows.len().min(ITEM_ROWS) };
            let mut table = col().w_full().child(head_row(
                u,
                &[
                    ("Item", 0.0, false),
                    ("Matches", 72.0, true),
                    ("Win rate", 80.0, true),
                    ("Against everyone", 120.0, true),
                    ("Bought at", 80.0, true),
                    ("Than everyone", 100.0, true),
                ],
            ));
            for item in rows.iter().take(shown) {
                let name = load.names.get(&item.item_id).cloned().unwrap_or_else(|| format!("Item {}", item.item_id));
                let better = item.win_rate_delta_percent >= 0.0;
                let earlier = item.buy_time_delta_s <= 0.0;
                table = table.child(
                    data_row(format!("stats-item-{}", item.item_id), u, 40.0)
                        .child(cell(text(name).font(BODY_MED).color(u.ink).truncate(), 0.0))
                        .child(num(item.matches.to_string(), 72.0, u.prose))
                        .child(num(format!("{:.0}%", item.win_rate_percent), 80.0, u.prose))
                        .child(num(
                            signed_points(item.win_rate_delta_percent),
                            120.0,
                            if better { u.teal } else { u.rose },
                        ))
                        .child(num(minutes(item.avg_buy_time_s), 80.0, u.prose))
                        .child(num(
                            format!("{}{}", if earlier { "-" } else { "+" }, minutes(item.buy_time_delta_s.abs())),
                            100.0,
                            if earlier { u.teal } else { u.muted },
                        )),
                );
            }
            let mut out = section.child(table);
            if rows.len() > ITEM_ROWS {
                out = out.child(
                    u.action(if m.items_all { tr("Show fewer") } else { tr("Show all") }, ActionKind::Ghost)
                        .key("stats-items-all")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.stats.more.items_all = !st.stats.more.items_all),
                );
            }
            out
        }
    };
    section
}

/// One match's purchases, under its open row.
pub(super) fn timeline(s: &State, u: &Ui, match_id: u64) -> El {
    let m = &s.stats.more;
    match &m.timeline {
        Fetched::Ready((id, rows)) if *id == match_id => {
            if rows.is_empty() {
                return text(tr("No purchases were recorded for you in this match.")).font(SMALL).color(u.muted);
            }
            let mut line = row().w_full().gap(6.0).flex_wrap();
            for (purchase, name) in rows {
                let sold = purchase.sold_at_s.map(|at| format!(", {} {}", tr("sold"), minutes(at as f64))).unwrap_or_default();
                line = line.child(
                    row()
                        .h(24.0)
                        .px(8.0)
                        .gap(6.0)
                        .center_y()
                        .radius(crate::ui::r(4.0))
                        .border(1.0, u.hair)
                        .child(text(minutes(purchase.bought_at_s as f64)).font(TNUM_SMALL).color(u.muted))
                        .child(text(format!("{name}{sold}")).font(SMALL).color(u.prose)),
                );
            }
            col().w_full().gap(6.0).child(text(tr("Items in the order you bought them")).font(SMALL).color(u.muted)).child(line)
        }
        Fetched::Loading => text(tr("Loading the item timeline")).font(SMALL).color(u.muted),
        Fetched::Failed(error) => problem_retry(u, error, &format!("stats-timeline-retry-{match_id}"), move |st: &mut State| {
            start_timeline(st, match_id)
        }),
        _ => row().child(
            u.action(tr("Item timeline"), ActionKind::Secondary)
                .key(format!("stats-timeline-{match_id}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| start_timeline(st, match_id)),
        ),
    }
}

fn hero_label(s: &State, hero_id: u32) -> String {
    s.stats
        .snapshot
        .as_ref()
        .and_then(|snap| snap.heroes.iter().find(|h| h.hero_id == hero_id))
        .map(hero_name)
        .or_else(|| {
            s.stats.w4.names.as_ref().and_then(|names| {
                names.iter().find(|(id, _)| *id == hero_id).map(|(_, name)| name.clone())
            })
        })
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(tr("Hero {}"), &[&hero_id.to_string()]))
}

/// Party: pick the members, then every lineup they can queue, rated.
pub(super) fn party_section(s: &State, u: &Ui) -> El {
    let m = &s.stats.more;
    let me = s.stats.snapshot.as_ref().map(|snap| snap.account_id);
    let mut section = col()
        .key("stats-party")
        .w_full()
        .gap(12.0)
        .child(section_head(u, "Party", "Every lineup your party can queue, rated from pair and trio records"))
        // Wave 4: DMM's planner, heroes picked by hand.
        .child(super::w4::party_mode(s, u));
    if s.stats.w4.hand {
        return section.child(super::w4::hand_party(s, u));
    }
    // Members: tracked players, then premades from the teammates list.
    let mut people: Vec<(u64, String)> = s
        .stats
        .remembered
        .iter()
        .filter(|r| Some(r.account_id) != me)
        .map(|r| (r.account_id, r.name.clone()))
        .collect();
    if let Fetched::Ready(social) = &s.stats.social {
        for mate in social.mates.iter().filter(|p| p.premade) {
            if Some(mate.account_id) != me && !people.iter().any(|(id, _)| *id == mate.account_id) {
                people.push((mate.account_id, mate.persona_name.clone().unwrap_or_else(|| mate.account_id.to_string())));
            }
        }
    }
    if people.is_empty() {
        return section.child(
            text(tr("Track a friend's account or show the players you meet; premades and tracked players can join the party here."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }
    let mut chips = row().w_full().gap(6.0).flex_wrap();
    for (id, name) in people {
        let on = m.members.contains(&id);
        chips = chips.child(
            row()
                .key(format!("stats-party-member-{id}"))
                .h(28.0)
                .pl(4.0)
                .pr(10.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(14.0))
                .border(1.0, if on { u.accent } else { u.hair_strong })
                .fill(if on { u.accent.alpha(0.10) } else { rune::Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("checkbox")
                .checked(on)
                .label(name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let members = &mut st.stats.more.members;
                    if !members.remove(&id) && members.len() + 1 < deadlock_core::stats::party::MAX_PARTY_SIZE {
                        members.insert(id);
                    }
                })
                .child(avatar(u, &name, 20.0))
                .child(text(name).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden()),
        );
    }
    section = section.child(chips).child(
        row().w_full().gap(12.0).center_y().child(
            u.action(tr("Rate lineups"), ActionKind::Secondary)
                .key("stats-party-rate")
                .disabled(m.members.is_empty())
                .cursor(rune::Cursor::Pointer)
                .on_press(start_party),
        ),
    );
    section.child(lineup_table(s, u, &m.party, false))
}

/// The rated lineups: the top ten with hero chips, estimates marked, thin samples greyed, and
/// the party's expected win rate. `hand` lineups come from picks made by hand.
pub(super) fn lineup_table(s: &State, u: &Ui, party: &Fetched<Vec<LineupRating>>, hand: bool) -> El {
    let section = col().w_full().gap(12.0);
    let prefix = if hand { "stats-lineup-hand" } else { "stats-lineup" };
    match party {
        Fetched::Idle => section,
        Fetched::Loading => section.child(text(tr("Rating lineups")).font(BODY).color(u.muted)),
        Fetched::Failed(error) => section.child(problem(u, error)),
        Fetched::Ready(ratings) if ratings.is_empty() => {
            section.child(text(tr("No lineup to rate: the party has no heroes in this filter.")).font(BODY).color(u.muted))
        }
        Fetched::Ready(ratings) => {
            let expected = deadlock_core::stats::party::expected_win_rate(ratings);
            let mut table = col().w_full().child(head_row(u, &[("Lineup", 0.0, false), ("Matches", 96.0, true), ("Win rate", 96.0, true)]));
            for (index, rating) in ratings.iter().take(LINEUP_ROWS).enumerate() {
                let mut heroes = row().gap(6.0).center_y();
                for hero in &rating.hero_ids {
                    let name = hero_label(s, *hero);
                    heroes = heroes.child(hero_chip(s, u, *hero, &name, 24.0)).child(text(name).font(SMALL).color(u.prose));
                }
                let muted = rating.is_low_sample();
                let matches = match rating.matches {
                    Some(n) => thousands(n),
                    None => crate::screens::settings::i18n::fill(tr("about {}"), &[&thousands(rating.sample_matches)]),
                };
                let rate = rating.win_rate_percent.map(|r| format!("{r:.1}%")).unwrap_or_else(|| "-".to_string());
                table = table.child(
                    data_row(format!("{prefix}-{index}"), u, 40.0)
                        .child(cell(heroes, 0.0))
                        .child(num(matches, 96.0, if muted { u.muted } else { u.prose }))
                        .child(num(rate, 96.0, if muted { u.muted } else { u.ink })),
                );
            }
            let mut out = section.child(table);
            if let Some(rate) = expected {
                out = out.child(
                    text(crate::screens::settings::i18n::fill(
                        tr("Across these lineups the party wins about {}% of its games. Grey rows have few matches behind them."),
                        &[&format!("{rate:.1}")],
                    ))
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
                );
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_keeps_the_mode_and_window() {
        let now = 1_000_000;
        let mut more = More::default();
        assert!(keeps_mode_at(&more, 1, 0, now));
        more.ranked = Some(true);
        assert!(keeps_mode_at(&more, 4, now, now) && !keeps_mode_at(&more, 1, now, now));
        more.ranked = Some(false);
        assert!(keeps_mode_at(&more, 1, now, now) && !keeps_mode_at(&more, 4, now, now));
        more.ranked = None;
        more.days = 7;
        assert!(keeps_mode_at(&more, 1, now - 86_400, now));
        assert!(!keeps_mode_at(&more, 1, now - 8 * 86_400, now));
        assert_eq!(more.filter(10 * 86_400).since, Some(3 * 86_400));
        more.days = 0;
        assert_eq!(more.filter(10 * 86_400).since, None);
    }

    #[test]
    fn street_brawl_can_be_shown_hidden_or_alone() {
        let item = |game_mode: i32, match_mode: i32| MatchSummary {
            match_id: 1,
            hero_id: 1,
            hero_name: None,
            start_time: 0,
            started_at: String::new(),
            duration_seconds: 900,
            game_mode,
            match_mode,
            player_team: 0,
            kills: 0,
            deaths: 0,
            assists: 0,
            kda: 0.0,
            net_worth: 0,
            last_hits: 0,
            denies: 0,
            outcome: deadlock_core::stats::MatchOutcome::Win,
            win: Some(true),
            abandoned_seconds: None,
            ranked_badge: None,
            ranked_delta: None,
        };
        let brawl = item(deadlock_core::stats::filters::GAME_MODE_STREET_BRAWL, 1);
        let normal = item(1, 4);
        let mut more = More::default();
        assert!(keeps(&more, &brawl, 0) && keeps(&more, &normal, 0));
        more.brawl = Brawl::Hide;
        assert!(!keeps(&more, &brawl, 0) && keeps(&more, &normal, 0));
        more.brawl = Brawl::Only;
        assert!(keeps(&more, &brawl, 0) && !keeps(&more, &normal, 0));
        more.brawl = Brawl::Show;
        more.ranked = Some(true);
        assert!(!keeps(&more, &brawl, 0), "Street Brawl is never ranked");
    }
}
