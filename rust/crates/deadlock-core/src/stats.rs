//! Read-only Deadlock player statistics from the public Deadlock API.
//!
//! Source: <https://api.deadlock-api.com>. Every endpoint used here is open and
//! needs no account. An API key is optional and only raises the shared rate
//! limits; set `DEADLOCK_API_KEY` to send it as `X-API-KEY`.
//!
//! Match history has no paging or limit parameter upstream. `fetch_matches`
//! downloads the stored history and truncates it locally, so callers should ask
//! for a window and cache it instead of polling.
//!
//! Statistical scope:
//!
//! - Career totals and per-hero rows come from `/v1/players/hero-stats`, which
//!   defaults to normal game mode and ranked plus unranked match modes.
//! - The match window comes from `/v1/players/{account_id}/match-history`.
//! - Win rate is counted only where `player_match_outcome` says win or loss.
//!   Matches marked invalid, penalized or not scored stay in the sample count
//!   but not in the win or loss count.
//! - KDA is `(kills + assists) / max(deaths, 1)`, the convention used by public
//!   MOBA trackers.
//!
//! Heatmaps: `/v1/analytics/kill-death-stats` returns real kill and death
//! positions in map world units. It is an aggregate over the API's default
//! window (the last 30 days) and cannot be filtered to one match.
//! Per-match position timelines exist only through the asynchronous demo query
//! endpoints, which are not used here.

pub mod board;
pub mod cache;
pub mod insight;
pub mod leaderboard;
pub mod social;

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, unix_seconds_to_iso};

use cache::{Age, ResponseCache};

/// Public API root. The OpenAPI document lists this exact server.
pub const STATS_API_BASE: &str = "https://api.deadlock-api.com";
/// Identifies this client without naming a user or an account.
pub const STATS_USER_AGENT: &str = "dah/0.0.0";
/// Optional API key variable. The statistics endpoints work without it.
pub const STATS_API_KEY_ENV: &str = "DEADLOCK_API_KEY";
/// Regions the leaderboard endpoint accepts.
pub const LEADERBOARD_REGIONS: [&str; 5] = ["Europe", "Asia", "NAmerica", "SAmerica", "Oceania"];
/// Recent form counts the latest decisive matches in the requested window.
pub const RECENT_FORM_MATCHES: usize = 10;
/// Default window for a snapshot.
pub const MATCH_WINDOW_DEFAULT: usize = 20;
/// Upper bound for a local match window. The upstream response is not paged.
pub const MATCH_WINDOW_MAX: usize = 1000;
/// Names resolved for the social lists: one profile request, top rows only.
pub const SOCIAL_NAMES: usize = 50;

const REQUEST_TIMEOUT_SECONDS: u64 = 45;
const BODY_SNIPPET_CHARS: usize = 240;

/// One Steam profile row from `/v1/players/steam`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerProfile {
    pub account_id: u64,
    pub persona_name: String,
    pub profile_url: String,
    pub avatar: String,
    pub avatar_medium: String,
    pub avatar_full: String,
    pub country_code: Option<String>,
    pub real_name: Option<String>,
    pub last_updated: String,
    pub matches_played_last_30d: u64,
    pub last_team_avg_badge: Option<u32>,
    /// Number of Steam friends the API returned. The list itself is not kept.
    pub friend_count: usize,
}

/// Career row for one hero, from `/v1/players/hero-stats`.
///
/// `hero_name` is filled from `/v1/assets/heroes` when the caller fetched names.
/// Rates and averages are lifetime values over the API's default filters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeroPerformance {
    pub hero_id: u32,
    pub hero_name: Option<String>,
    pub matches_played: u64,
    pub wins: u64,
    pub losses: u64,
    /// Percent from 0 to 100. `wins / matches_played`.
    pub win_rate_percent: f64,
    pub kills: u64,
    pub deaths: u64,
    pub assists: u64,
    pub kda: f64,
    pub time_played_seconds: u64,
    pub average_match_seconds: f64,
    pub damage_per_minute: f64,
    pub net_worth_per_minute: f64,
    pub objective_damage_per_minute: f64,
    pub damage_taken_per_minute: f64,
    pub denies_per_match: f64,
    /// Total boss damage from the hero-stats endpoint.
    pub boss_damage: u64,
    /// Lower bound of the 95 percent Wilson score interval for `win_rate_percent`.
    pub win_rate_low_percent: f64,
    /// Upper bound of the 95 percent Wilson score interval for `win_rate_percent`.
    pub win_rate_high_percent: f64,
    pub last_played: String,
    pub last_played_unix: i64,
}

/// One row from `/v1/players/{account_id}/match-history`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    pub match_id: u64,
    pub hero_id: u32,
    pub hero_name: Option<String>,
    pub start_time: i64,
    pub started_at: String,
    pub duration_seconds: u32,
    pub game_mode: i32,
    pub match_mode: i32,
    pub player_team: i32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub kda: f64,
    pub net_worth: u64,
    pub last_hits: u32,
    pub denies: u32,
    pub outcome: MatchOutcome,
    /// `Some(true)` for a win, `Some(false)` for a loss, `None` when unscored.
    pub win: Option<bool>,
    pub abandoned_seconds: Option<u32>,
    pub ranked_badge: Option<u32>,
    pub ranked_delta: Option<i32>,
}

/// How the match was scored for the player, mapped from `player_match_outcome`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MatchOutcome {
    Win,
    Loss,
    Penalized,
    PenalizedParty,
    NotScored,
    Invalid,
    Unknown,
}

impl MatchOutcome {
    fn from_api(value: i32) -> Self {
        match value {
            1 => Self::Win,
            2 => Self::Loss,
            3 => Self::Penalized,
            4 => Self::PenalizedParty,
            5 => Self::NotScored,
            0 => Self::Invalid,
            _ => Self::Unknown,
        }
    }

    /// Only win and loss count towards a win rate.
    pub fn win(self) -> Option<bool> {
        match self {
            Self::Win => Some(true),
            Self::Loss => Some(false),
            _ => None,
        }
    }

    /// Stable lower-case label for text output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Win => "win",
            Self::Loss => "loss",
            Self::Penalized => "penalized",
            Self::PenalizedParty => "penalized party",
            Self::NotScored => "not scored",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
        }
    }
}

/// Rank at the end of the latest ranked match, from `/v1/players/{account_id}/rank`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRank {
    /// `tier * 10 + subrank`. `0` means no recent ranked match reported a rank.
    pub badge: u32,
    pub rank: u32,
    pub subrank: u32,
    pub tier_name: Option<String>,
    pub last_match: Option<RankedMatch>,
    /// Population benchmark for the badge, when the caller fetched it.
    pub benchmark: Option<RankBenchmark>,
}

/// Where a badge sits among ranked players, from `/v1/players/rank/distribution`.
///
/// `percentile` is the share of ranked players whose badge is at or below this
/// one. Unranked accounts get no benchmark.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankBenchmark {
    pub badge: u32,
    pub ranked_players: u64,
    pub percentile: f64,
    /// Share of ranked players at this badge or above: the "top X percent" a
    /// player reads. `100 - percentile` would leave out the player's own badge.
    #[serde(default)]
    pub top_percent: f64,
    /// The distribution's time filter. The API defaults to the last 30 days.
    pub window: String,
}

/// One badge row from `/v1/players/rank/distribution`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankDistributionEntry {
    pub badge: u32,
    pub rank: u32,
    pub subrank: u32,
    pub players: u64,
}

/// The rank metadata Valve reported on the match the badge was read from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankedMatch {
    pub match_id: u64,
    pub start_time: i64,
    pub started_at: String,
    pub initial_display_rank: Option<u32>,
    pub initial_flat_progress: Option<i64>,
    pub final_flat_progress: Option<i64>,
    pub desired_progress_change: Option<i64>,
    pub initial_calibration_games: Option<u32>,
    pub initial_win_streak: Option<u32>,
}

/// Lifetime totals summed across the per-hero rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CareerTotals {
    pub heroes_played: usize,
    pub matches_played: u64,
    pub wins: u64,
    pub losses: u64,
    pub win_rate_percent: f64,
    pub kills: u64,
    pub deaths: u64,
    pub assists: u64,
    pub kda: f64,
    pub time_played_seconds: u64,
    pub average_match_seconds: f64,
    pub player_damage: u64,
    pub boss_damage: u64,
    /// Objective damage estimated from `objective_damage_per_minute` and time
    /// played. The API does not expose a career objective damage total.
    pub objective_damage: u64,
    pub damage_per_minute: f64,
    /// Lower bound of the 95 percent Wilson score interval for `win_rate_percent`.
    pub win_rate_low_percent: f64,
    /// Upper bound of the 95 percent Wilson score interval for `win_rate_percent`.
    pub win_rate_high_percent: f64,
}

/// Derived statistics over the returned match window, ordered newest first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowStats {
    pub matches: usize,
    pub wins: usize,
    pub losses: usize,
    pub unscored: usize,
    pub win_rate_percent: f64,
    pub kills: u64,
    pub deaths: u64,
    pub assists: u64,
    pub kda: f64,
    pub average_duration_seconds: f64,
    pub average_net_worth: f64,
    /// Lower bound of the 95 percent Wilson score interval for `win_rate_percent`.
    pub win_rate_low_percent: f64,
    /// Upper bound of the 95 percent Wilson score interval for `win_rate_percent`.
    pub win_rate_high_percent: f64,
    /// Up to `RECENT_FORM_MATCHES` decisive matches, newest first.
    pub recent_form: Vec<FormEntry>,
    pub recent_win_rate_percent: f64,
}

/// One result in the recent form strip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormEntry {
    pub match_id: u64,
    pub hero_id: u32,
    pub hero_name: Option<String>,
    pub win: bool,
    pub start_time: i64,
    pub started_at: String,
}

/// Everything a player screen needs in one value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    pub account_id: u64,
    pub source: String,
    pub generated_at: String,
    pub profile: PlayerProfile,
    pub rank: PlayerRank,
    pub career: CareerTotals,
    pub heroes: Vec<HeroPerformance>,
    pub matches: Vec<MatchSummary>,
    pub window: WindowStats,
    /// Unix seconds for `generated_at`, the anchor for "today" and "this week".
    #[serde(default)]
    pub generated_unix: i64,
    /// Rank tier to display name, from `/v1/assets/ranks`.
    #[serde(default)]
    pub tier_names: BTreeMap<u32, String>,
    /// Per-hero averages for players in the same rank tier, when fetched.
    #[serde(default)]
    pub peers: Option<PeerBenchmarks>,
}

/// One hero's aggregate for a rank bracket, from `/v1/analytics/hero-stats`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeroBenchmark {
    pub hero_id: u32,
    pub matches: u64,
    pub wins: u64,
    pub losses: u64,
    pub win_rate_percent: f64,
    pub kda: f64,
    pub deaths_per_match: f64,
    pub net_worth_per_match: f64,
}

/// Players in one rank tier over the last 30 days: the honest peer group.
///
/// `min_badge` and `max_badge` are the tier's first and sixth subrank, so the
/// bracket is "everyone in Eternus", not a made-up skill band. The API filters
/// on the average badge of the match, not on each player's own badge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PeerBenchmarks {
    pub min_badge: u32,
    pub max_badge: u32,
    pub tier_name: Option<String>,
    pub window: String,
    pub heroes: Vec<HeroBenchmark>,
    /// Every hero row summed: matches are hero appearances, not games.
    pub overall: HeroBenchmark,
}

impl PeerBenchmarks {
    pub fn hero(&self, hero_id: u32) -> Option<&HeroBenchmark> {
        self.heroes.iter().find(|hero| hero.hero_id == hero_id)
    }
}

/// An objective on the map, in the minimap's relative frame: `left` grows to
/// the east, `top` grows to the south. Keys are the API's own, for example
/// `team0_tier1_3` (team 0, Guardian, lane 3) or `team1_titan` (the Patron).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MapObjective {
    pub key: String,
    pub left: f64,
    pub top: f64,
}

/// Map radius in world units, from `/v1/assets/map`.
pub const MAP_RADIUS: f64 = 10752.0;

/// World units to the minimap frame: `(x + r) / 2r` and `(r - y) / 2r`. The
/// map asset's camp positions confirm this transform.
pub fn world_to_map(x: f64, y: f64) -> (f64, f64) {
    (
        (x + MAP_RADIUS) / (2.0 * MAP_RADIUS),
        (MAP_RADIUS - y) / (2.0 * MAP_RADIUS),
    )
}

/// A captured bundle after parsing: the same values a live load produces.
#[derive(Debug, Clone)]
pub struct CapturedStats {
    pub snapshot: StatsSnapshot,
    pub heatmap: KillHeatmap,
    pub objectives: Vec<MapObjective>,
}

/// What the account field holds after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountInput {
    /// A SteamID3 account id, from any of the numeric forms.
    Account(u64),
    /// A name or vanity URL segment to look up with the profile search.
    Search(String),
    Empty,
}

/// First SteamID64 for an individual account. SteamID64 minus this is SteamID3.
pub const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

/// Parse what a player pastes: a SteamID3 (`1034123675`), a SteamID64, a
/// `[U:1:1034123675]` string, a `steamcommunity.com/profiles/<id64>` link, a
/// `steamcommunity.com/id/<vanity>` link, or a plain name. Vanity links and
/// names become a search, because resolving a vanity URL needs a Steam Web API
/// key this app does not hold.
pub fn parse_account_input(input: &str) -> AccountInput {
    let text = input.trim().trim_end_matches('/');
    if text.is_empty() {
        return AccountInput::Empty;
    }

    let lowered = text.to_ascii_lowercase();
    if let Some(index) = lowered.find("/id/") {
        let vanity = text[index + 4..].split(['/', '?', '#']).next().unwrap_or("");
        return if vanity.is_empty() {
            AccountInput::Empty
        } else {
            AccountInput::Search(vanity.to_string())
        };
    }
    if let Some(index) = lowered.find("/profiles/") {
        let id = text[index + 10..].split(['/', '?', '#']).next().unwrap_or("");
        return numeric_account(id).map_or(AccountInput::Empty, AccountInput::Account);
    }
    if let Some(inner) = lowered
        .strip_prefix("[u:1:")
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return numeric_account(inner).map_or(AccountInput::Empty, AccountInput::Account);
    }
    if let Some(id) = numeric_account(text) {
        return AccountInput::Account(id);
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        return AccountInput::Empty;
    }
    AccountInput::Search(text.to_string())
}

fn numeric_account(text: &str) -> Option<u64> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u64 = text.parse().ok()?;
    let id = if value >= STEAM_ID64_BASE {
        value - STEAM_ID64_BASE
    } else {
        value
    };
    (id > 0 && id < u32::MAX as u64).then_some(id)
}

/// One cell of the aggregate kill and death raster.
///
/// `position_x` and `position_y` are map world units; the map asset reports a
/// radius of 10752. `killer_team` is the team of the player who got the kill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KillHeatPoint {
    pub position_x: i32,
    pub position_y: i32,
    pub killer_team: i32,
    pub kills: u64,
    pub deaths: u64,
}

/// Aggregate kill and death positions for one account.
///
/// The analytics endpoint defaults to the last 30 days. Passing an older start
/// time makes the query time out for an active account, so this client uses the
/// documented default and reports the window it actually covers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KillHeatmap {
    pub account_id: u64,
    pub window: String,
    pub cells: usize,
    pub kills: u64,
    pub deaths: u64,
    pub points: Vec<KillHeatPoint>,
}

/// One row from `/v1/leaderboard/{region}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    /// Place on the leaderboard, not a rank badge. Players can share a place.
    pub rank: Option<u32>,
    pub account_name: Option<String>,
    /// Steam names are not unique, so the API can return several candidates.
    pub possible_account_ids: Vec<u64>,
    pub top_hero_ids: Vec<u32>,
}

/// A leaderboard page after local truncation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Leaderboard {
    pub region: String,
    /// The season's leaderboard id, when the board belongs to one.
    #[serde(default)]
    pub season_id: Option<u32>,
    #[serde(default)]
    pub season_name: Option<String>,
    pub entries: Vec<LeaderboardEntry>,
}

impl Leaderboard {
    /// Place of the player whose account id is `account_id`, when the board has
    /// a row that names it. Several ids can sit on one row because Steam names
    /// are not unique.
    pub fn place_of(&self, account_id: u64) -> Option<u32> {
        self.entries
            .iter()
            .find(|entry| entry.possible_account_ids.contains(&account_id))
            .and_then(|entry| entry.rank)
    }

    pub fn entry_of(&self, account_id: u64) -> Option<&LeaderboardEntry> {
        self.entries
            .iter()
            .find(|entry| entry.possible_account_ids.contains(&account_id))
    }
}

/// Shared HTTP client for the public Deadlock API.
///
/// The client is read only. It never sends player identifiers anywhere except
/// to `api.deadlock-api.com`, and the optional API key is only used as a header.
///
/// With a cache directory ([`StatsClient::with_cache`]) each response is stored
/// on disk and reused for its endpoint's lifetime; when the API cannot be
/// reached, an older copy answers and [`StatsClient::age`] says it did.
pub struct StatsClient {
    agent: ureq::Agent,
    base_url: String,
    api_key: Option<String>,
    cache: Option<ResponseCache>,
    age: Mutex<Age>,
}

impl StatsClient {
    /// Build a client, reading an optional `DEADLOCK_API_KEY` from the environment.
    pub fn new() -> Self {
        Self::with_cache_dir(None)
    }

    /// A client that keeps responses in `dir` and falls back to them.
    pub fn with_cache(dir: impl Into<PathBuf>) -> Self {
        Self::with_cache_dir(Some(dir.into()))
    }

    fn with_cache_dir(dir: Option<PathBuf>) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .middleware(crate::net::gate)
                .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
                .http_status_as_error(false)
                .build()
                .new_agent(),
            base_url: STATS_API_BASE.to_string(),
            api_key: read_api_key(),
            cache: dir.map(ResponseCache::new),
            age: Mutex::new(Age::default()),
        }
    }

    /// Age of the data the newest calls used, and whether a stale copy answered.
    pub fn age(&self) -> Age {
        self.age.lock().map(|age| *age).unwrap_or_default()
    }

    /// Start a new load: forget the ages of the previous one.
    pub fn reset_age(&self) {
        if let Ok(mut age) = self.age.lock() {
            *age = Age::default();
        }
    }

    fn note(&self, age: Age) {
        if let Ok(mut slot) = self.age.lock() {
            slot.merge(age);
        }
    }

    /// Attach a key explicitly. Empty values are ignored.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into().trim().to_string();
        self.api_key = if key.is_empty() { None } else { Some(key) };
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }

    /// Steam profile for one account. The API also accepts SteamID64, but the
    /// statistics endpoints use SteamID3, so callers should pass SteamID3.
    pub fn fetch_player(&self, account_id: u64) -> Result<PlayerProfile> {
        ensure_account(account_id)?;
        let rows: Vec<RawSteamProfile> = self.get_cached(
            "/v1/players/steam",
            &[("account_ids", account_id.to_string())],
            cache::TTL_STEAM_PROFILE,
        )?;

        let row = rows
            .into_iter()
            .find(|row| row.account_id == account_id)
            .ok_or_else(|| {
                Error::msg(format!(
                    "No Steam profile found for account id {account_id}. Use a SteamID3 from the \
                     leaderboard or Steam profile search."
                ))
            })?;

        Ok(row.into_profile())
    }

    /// Steam profiles for several accounts in one request. Ids the API does not
    /// know are simply missing from the answer.
    pub fn fetch_profiles(&self, account_ids: &[u64]) -> Result<Vec<PlayerProfile>> {
        let mut ids: Vec<u64> = account_ids.iter().copied().filter(|id| *id > 0).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let joined = ids
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let rows: Vec<RawSteamProfile> = self.get_cached(
            "/v1/players/steam",
            &[("account_ids", joined)],
            cache::TTL_STEAM_PROFILE,
        )?;
        Ok(rows.into_iter().map(RawSteamProfile::into_profile).collect())
    }

    /// Rank at the end of the latest ranked match.
    pub fn fetch_rank(&self, account_id: u64) -> Result<PlayerRank> {
        ensure_account(account_id)?;
        let raw: RawRankResponse = self.get_cached(
            &format!("/v1/players/{account_id}/rank"),
            &[],
            cache::TTL_RANK,
        )?;
        let mut rank = raw.into_rank();
        if let Ok(names) = self.fetch_rank_names() {
            rank.tier_name = names.get(&rank.rank).cloned();
        }
        Ok(rank)
    }

    /// Rank distribution across ranked players, from
    /// `/v1/players/rank/distribution`.
    ///
    /// The endpoint is limited to 5 requests per minute on an IP. Use it for a
    /// benchmark row, not for every match refresh.
    pub fn fetch_rank_distribution(&self) -> Result<Vec<RankDistributionEntry>> {
        let rows: Vec<RawRankDistribution> = self.get_cached(
            "/v1/players/rank/distribution",
            &[],
            cache::TTL_RANK_DISTRIBUTION,
        )?;
        Ok(rows
            .into_iter()
            .map(RawRankDistribution::into_entry)
            .collect())
    }

    /// Population percentile for one badge, when the badge is ranked.
    pub fn fetch_rank_benchmark(&self, badge: u32) -> Result<Option<RankBenchmark>> {
        if badge == 0 {
            return Ok(None);
        }
        let distribution = self.fetch_rank_distribution()?;
        Ok(rank_benchmark(badge, &distribution))
    }

    /// Match history, newest first, truncated to `limit`.
    ///
    /// The upstream endpoint returns the full stored history. There is no
    /// cursor or page parameter, so the network cost does not fall with a small
    /// limit. Ask for a window that fits the screen and reuse it.
    pub fn fetch_matches(&self, account_id: u64, limit: usize) -> Result<Vec<MatchSummary>> {
        ensure_account(account_id)?;
        if limit == 0 {
            return Err(Error::msg(
                "Match window must be 1 or greater. The API has no paging parameter, so a window \
                 of zero would still download the full history.",
            ));
        }
        if limit > MATCH_WINDOW_MAX {
            return Err(Error::msg(format!(
                "Match window must be {MATCH_WINDOW_MAX} or fewer. The history is already \
                 downloaded in full, and larger windows are not useful for a player screen."
            )));
        }

        let mut rows: Vec<RawMatch> = self.get_cached(
            &format!("/v1/players/{account_id}/match-history"),
            &[],
            cache::TTL_MATCH_HISTORY,
        )?;
        rows.sort_by_key(|row| std::cmp::Reverse(row.start_time));
        rows.truncate(limit);
        Ok(rows.into_iter().map(RawMatch::into_summary).collect())
    }

    /// Lifetime per-hero rows with the API's default filters.
    pub fn fetch_hero_stats(&self, account_id: u64) -> Result<Vec<HeroPerformance>> {
        ensure_account(account_id)?;
        let rows: Vec<RawHeroStats> = self.get_cached(
            "/v1/players/hero-stats",
            &[("account_ids", account_id.to_string())],
            cache::TTL_HERO_STATS,
        )?;
        Ok(rows
            .into_iter()
            .map(RawHeroStats::into_performance)
            .collect())
    }

    /// Hero id to display name, from `/v1/assets/heroes`.
    ///
    /// The asset response is about 1.9 MB because it includes lore and art
    /// metadata. One fetch per process is enough; do not call it per match.
    pub fn fetch_hero_names(&self) -> Result<HashMap<u32, String>> {
        let rows: Vec<RawHeroAsset> = self.get_cached("/v1/assets/heroes", &[], cache::TTL_ASSETS)?;
        Ok(rows.into_iter().map(|row| (row.id, row.name)).collect())
    }

    /// Rank tier to display name, from `/v1/assets/ranks`.
    pub fn fetch_rank_names(&self) -> Result<HashMap<u32, String>> {
        let rows: Vec<RawRankAsset> = self.get_cached("/v1/assets/ranks", &[], cache::TTL_ASSETS)?;
        Ok(rows.into_iter().map(|row| (row.tier, row.name)).collect())
    }

    /// Item id to name, slot and tier, from `/v1/assets/items/by-type/upgrade`
    /// (about 2 MB). Used for a match's final builds.
    pub fn fetch_item_catalog(&self) -> Result<board::ItemCatalog> {
        let json = self.get_text("/v1/assets/items/by-type/upgrade", &[], cache::TTL_ASSETS)?;
        board::parse_item_catalog(&json)
    }

    /// One region's leaderboard, truncated to `limit` entries.
    pub fn fetch_leaderboard(&self, region: &str, limit: usize) -> Result<Leaderboard> {
        let season = leaderboard::current_season(&self.fetch_seasons()?, now_unix()).cloned();
        match &season {
            Some(season) => {
                let mut board = self.fetch_season_board(region, season)?;
                board.entries.truncate(limit.max(1));
                Ok(board)
            }
            None => Err(Error::msg(
                "The API lists no ranked season with a leaderboard yet.",
            )),
        }
    }

    /// The seasons that name a leaderboard, newest first. Kept for a week: Valve
    /// starts a new one every few months.
    pub fn fetch_seasons(&self) -> Result<Vec<leaderboard::RankedSeason>> {
        let json = self.get_text("/v1/assets/ranked-seasons", &[], cache::TTL_ASSETS)?;
        leaderboard::parse_seasons(&json)
    }

    /// One season's board in one region, every entry, ordered by place.
    pub fn fetch_season_board(
        &self,
        region: &str,
        season: &leaderboard::RankedSeason,
    ) -> Result<Leaderboard> {
        if !LEADERBOARD_REGIONS.contains(&region) {
            return Err(Error::msg(format!(
                "Unknown leaderboard region {region:?}. Use one of {}.",
                LEADERBOARD_REGIONS.join(", ")
            )));
        }
        let body = self.get_text(
            &format!("/v1/leaderboard/{region}"),
            &[("leaderboard_id", season.leaderboard_id.to_string())],
            cache::TTL_LEADERBOARD,
        )?;
        let entries = leaderboard::parse_entries(&body)?;
        Ok(Leaderboard {
            region: region.to_string(),
            season_id: Some(season.leaderboard_id),
            season_name: Some(season.name.clone()),
            entries,
        })
    }

    /// One match's scoreboard: both teams, final builds and the souls curve.
    ///
    /// `account_id` is the player whose page asked, marked on the board. Names of
    /// the other eleven players are best effort: one Steam profile request, and a
    /// row without a name still shows its account id.
    pub fn fetch_match_scoreboard(
        &self,
        match_id: u64,
        account_id: u64,
    ) -> Result<board::MatchScoreboard> {
        if match_id == 0 {
            return Err(Error::msg("A match id is needed to open a scoreboard."));
        }
        let metadata = self.get_text(
            &format!("/v1/matches/{match_id}/metadata"),
            &[("disable_steam", "true".to_string())],
            cache::TTL_MATCH_METADATA,
        )?;
        let catalog = self.fetch_item_catalog().unwrap_or_default();
        let hero_names = self.fetch_hero_names().unwrap_or_default();
        let accounts = board::accounts_in(&metadata);
        let names: HashMap<u64, String> = self
            .fetch_profiles(&accounts)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| (row.account_id, row.persona_name))
                    .collect()
            })
            .unwrap_or_default();
        board::assemble_board(account_id, &metadata, &catalog, &names, &hero_names)
    }

    /// Teammates and opponents, named, from the mate and enemy statistics.
    ///
    /// Three responses of about 700 KB each, then one profile request for the
    /// names of the top fifty. Kept for an hour.
    pub fn fetch_social(&self, account_id: u64) -> Result<social::Social> {
        ensure_account(account_id)?;
        let sample = social::MIN_SHARED_MATCHES.to_string();
        let mates = self.get_text(
            &format!("/v1/players/{account_id}/mate-stats"),
            &[("min_matches_played", sample.clone())],
            cache::TTL_SOCIAL,
        )?;
        let party = self.get_text(
            &format!("/v1/players/{account_id}/mate-stats"),
            &[
                ("min_matches_played", sample.clone()),
                ("same_party", "true".to_string()),
            ],
            cache::TTL_SOCIAL,
        )?;
        let enemies = self.get_text(
            &format!("/v1/players/{account_id}/enemy-stats"),
            &[("min_matches_played", sample)],
            cache::TTL_SOCIAL,
        )?;

        let ids = social::top_accounts(&mates, &party, &enemies, SOCIAL_NAMES);
        let names: HashMap<u64, String> = self
            .fetch_profiles(&ids)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| (row.account_id, row.persona_name))
                    .collect()
            })
            .unwrap_or_default();
        social::assemble_social(&mates, &party, &enemies, &names)
    }

    /// Aggregate kill and death positions for the last 30 days.
    ///
    /// This is the only heatmap the public API exposes synchronously. It covers
    /// all matches in the window, not a selected match. See
    /// `docs/research/stats.md` for the demo-query gap.
    pub fn fetch_kill_heatmap(&self, account_id: u64) -> Result<KillHeatmap> {
        ensure_account(account_id)?;
        let rows: Vec<RawKillDeath> = self.get_cached(
            "/v1/analytics/kill-death-stats",
            &[("account_ids", account_id.to_string())],
            cache::TTL_HERO_STATS,
        )?;

        let kills = rows.iter().map(|row| row.kills).sum();
        let deaths = rows.iter().map(|row| row.deaths).sum();
        Ok(KillHeatmap {
            account_id,
            window: "last 30 days".to_string(),
            cells: rows.len(),
            kills,
            deaths,
            points: rows.into_iter().map(RawKillDeath::into_point).collect(),
        })
    }

    /// Profile, rank, career totals, hero rows and the recent match window.
    ///
    /// Hero names are best effort: if the asset request fails, rows keep their
    /// hero ids and the statistics stay usable.
    pub fn fetch_snapshot(&self, account_id: u64, match_limit: usize) -> Result<StatsSnapshot> {
        ensure_account(account_id)?;
        self.reset_age();
        let profile = self.fetch_player(account_id)?;
        let raw_rank: RawRankResponse = self.get_cached(
            &format!("/v1/players/{account_id}/rank"),
            &[],
            cache::TTL_RANK,
        )?;
        let tier_names = self.fetch_rank_names().unwrap_or_default();
        let mut rank = raw_rank.into_rank();
        rank.benchmark = self.fetch_rank_benchmark(rank.badge).unwrap_or(None);
        let heroes = self.fetch_hero_stats(account_id)?;
        let matches = self.fetch_matches(account_id, match_limit)?;
        let hero_names = self.fetch_hero_names().unwrap_or_default();
        let generated_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);

        let mut snapshot = assemble_snapshot(
            account_id,
            profile,
            rank,
            heroes,
            matches,
            &hero_names,
            tier_names.into_iter().collect(),
            generated_unix,
        );
        snapshot.generated_at = now_iso();
        Ok(snapshot)
    }

    /// Per-hero averages for the rank tier that holds `badge`, over the 30 days
    /// before `until_unix`. `None` for an unranked badge.
    ///
    /// The bracket is the whole tier (subranks 1 to 6). It is one request to
    /// the analytics endpoint, which shares a 200 per minute limit.
    pub fn fetch_peer_benchmarks(
        &self,
        badge: u32,
        until_unix: i64,
        tier_name: Option<String>,
    ) -> Result<Option<PeerBenchmarks>> {
        if badge < 10 {
            return Ok(None);
        }
        let tier = badge / 10;
        let (min_badge, max_badge) = (tier * 10 + 1, tier * 10 + 6);
        let rows: Vec<RawHeroBenchmark> = self.get_cached(
            "/v1/analytics/hero-stats",
            &[
                ("min_average_badge", min_badge.to_string()),
                ("max_average_badge", max_badge.to_string()),
                ("min_unix_timestamp", (until_unix - 30 * 86_400).to_string()),
            ],
            cache::TTL_RANK_DISTRIBUTION,
        )?;
        Ok(Some(peer_benchmarks(min_badge, max_badge, tier_name, rows)))
    }

    /// Steam profiles whose name matches `query`, for the account field when a
    /// player pastes a name or a vanity link. Only profiles with recent Deadlock
    /// matches are returned (the API default is 5 in the last 30 days).
    pub fn search_players(&self, query: &str, limit: usize) -> Result<Vec<PlayerProfile>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<RawSteamProfile> = parse_body(
            "/v1/players/steam-search",
            &self.get_raw(
                "/v1/players/steam-search",
                &[
                    ("search_query", query.to_string()),
                    ("limit", limit.clamp(1, 20).to_string()),
                ],
            )?,
        )?;
        Ok(rows.into_iter().map(RawSteamProfile::into_profile).collect())
    }

    /// Objective positions from `/v1/assets/map` (about 111 KB), so the
    /// heatmap can show where Guardians, Walkers and Patrons stand.
    pub fn fetch_map_objectives(&self) -> Result<Vec<MapObjective>> {
        let raw: RawMapAsset = self.get_cached("/v1/assets/map", &[], cache::TTL_ASSETS)?;
        Ok(map_objectives(raw.objective_positions))
    }

    /// A response body, used for its own lifetime, falling back to an older copy
    /// when the API cannot be reached.
    fn get_cached<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        ttl: i64,
    ) -> Result<T> {
        parse_body(path, &self.get_text(path, query, ttl)?)
    }

    /// The body behind [`StatsClient::get_cached`].
    fn get_text(&self, path: &str, query: &[(&str, String)], ttl: i64) -> Result<String> {
        let Some(cache) = &self.cache else {
            return self.get_raw(path, query);
        };
        let key = cache::key(path, query);
        let now = now_unix();
        if let Some(stored) = cache.read_fresh(&key, ttl, now) {
            self.note(Age {
                seconds: (now - stored.fetched_at).max(0),
                stale: false,
            });
            return Ok(stored.body);
        }
        match self.get_raw(path, query) {
            Ok(body) => {
                let _ = cache.write(&key, &body, now);
                Ok(body)
            }
            Err(error) => match cache.read(&key) {
                Some(stored) => {
                    self.note(Age {
                        seconds: (now - stored.fetched_at).max(0),
                        stale: true,
                    });
                    Ok(stored.body)
                }
                None => Err(error),
            },
        }
    }

    fn get_raw(&self, path: &str, query: &[(&str, String)]) -> Result<String> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut request = self
            .agent
            .get(&url)
            .header("User-Agent", STATS_USER_AGENT)
            .header("Accept", "application/json");

        if let Some(key) = &self.api_key {
            request = request.header("X-API-KEY", key);
        }

        for (name, value) in query {
            request = request.query(*name, value);
        }

        let mut response = request.call().map_err(|error| {
            Error::Network(format!(
                "Deadlock API request to {url} failed: {error}. Check the network connection and \
                 retry."
            ))
        })?;

        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let body = response.body_mut().read_to_string().map_err(|error| {
            Error::Network(format!(
                "Deadlock API response for {url} could not be read: {error}"
            ))
        })?;

        if !(200..300).contains(&status) {
            let hint = status_hint(status, retry_after.as_deref());
            return Err(Error::Network(format!(
                "Deadlock API returned HTTP {status} for {url}.{hint} {}",
                snippet(&body)
            )));
        }

        Ok(body)
    }
}

fn parse_body<T: DeserializeOwned>(path: &str, body: &str) -> Result<T> {
    serde_json::from_str(body).map_err(|error| {
        Error::Network(format!(
            "Deadlock API response for {path} did not match the expected shape: {error}. {}",
            snippet(body)
        ))
    })
}

/// Seconds since the Unix epoch, the clock the API's timestamps use.
pub fn now_seconds() -> i64 {
    now_unix()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

impl Default for StatsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for StatsClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StatsClient")
            .field("base_url", &self.base_url)
            .field("api_key_set", &self.api_key.is_some())
            .finish()
    }
}

/// Fetch a profile without constructing a client by hand.
pub fn fetch_player(account_id: u64) -> Result<PlayerProfile> {
    StatsClient::new().fetch_player(account_id)
}

/// Fetch a match window without constructing a client by hand.
pub fn fetch_matches(account_id: u64, limit: usize) -> Result<Vec<MatchSummary>> {
    StatsClient::new().fetch_matches(account_id, limit)
}

/// Fetch per-hero career rows without constructing a client by hand.
pub fn fetch_hero_stats(account_id: u64) -> Result<Vec<HeroPerformance>> {
    StatsClient::new().fetch_hero_stats(account_id)
}

/// Build a full snapshot without constructing a client by hand.
pub fn fetch_snapshot(account_id: u64, match_limit: usize) -> Result<StatsSnapshot> {
    StatsClient::new().fetch_snapshot(account_id, match_limit)
}

/// Sum lifetime hero rows into career totals.
pub fn career_totals(heroes: &[HeroPerformance]) -> CareerTotals {
    let mut matches_played = 0u64;
    let mut wins = 0u64;
    let mut kills = 0u64;
    let mut deaths = 0u64;
    let mut assists = 0u64;
    let mut time_played = 0u64;
    let mut player_damage = 0u64;
    let mut boss_damage = 0u64;
    let mut objective_damage = 0u64;

    for hero in heroes {
        matches_played = matches_played.saturating_add(hero.matches_played);
        wins = wins.saturating_add(hero.wins);
        kills = kills.saturating_add(hero.kills);
        deaths = deaths.saturating_add(hero.deaths);
        assists = assists.saturating_add(hero.assists);
        time_played = time_played.saturating_add(hero.time_played_seconds);
        player_damage = player_damage.saturating_add(
            (hero.damage_per_minute * minutes(hero.time_played_seconds)).round() as u64,
        );
        boss_damage = boss_damage.saturating_add(hero.boss_damage);
        objective_damage = objective_damage.saturating_add(
            (hero.objective_damage_per_minute * minutes(hero.time_played_seconds)).round() as u64,
        );
    }

    let losses = matches_played.saturating_sub(wins);
    let time_minutes = minutes(time_played);
    let (win_rate_low_percent, win_rate_high_percent) = wilson_interval(wins, matches_played);

    CareerTotals {
        heroes_played: heroes.len(),
        matches_played,
        wins,
        losses,
        win_rate_percent: percent(wins, matches_played),
        kills,
        deaths,
        assists,
        kda: kda(kills, deaths, assists),
        time_played_seconds: time_played,
        average_match_seconds: average(time_played as f64, matches_played),
        player_damage,
        boss_damage,
        objective_damage,
        damage_per_minute: if time_minutes > 0.0 {
            player_damage as f64 / time_minutes
        } else {
            0.0
        },
        win_rate_low_percent,
        win_rate_high_percent,
    }
}

/// Derive the window statistics from matches already ordered newest first.
pub fn window_stats(matches: &[MatchSummary]) -> WindowStats {
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut kills = 0u64;
    let mut deaths = 0u64;
    let mut assists = 0u64;
    let mut duration_total = 0u64;
    let mut duration_count = 0usize;
    let mut net_worth_total = 0u64;

    for summary in matches {
        match summary.win {
            Some(true) => wins += 1,
            Some(false) => losses += 1,
            None => {}
        }
        kills = kills.saturating_add(summary.kills as u64);
        deaths = deaths.saturating_add(summary.deaths as u64);
        assists = assists.saturating_add(summary.assists as u64);
        if summary.duration_seconds > 0 {
            duration_total = duration_total.saturating_add(summary.duration_seconds as u64);
            duration_count += 1;
        }
        net_worth_total = net_worth_total.saturating_add(summary.net_worth);
    }

    let mut recent_form = Vec::new();
    for summary in matches {
        let Some(win) = summary.win else {
            continue;
        };
        recent_form.push(FormEntry {
            match_id: summary.match_id,
            hero_id: summary.hero_id,
            hero_name: summary.hero_name.clone(),
            win,
            start_time: summary.start_time,
            started_at: summary.started_at.clone(),
        });
        if recent_form.len() == RECENT_FORM_MATCHES {
            break;
        }
    }

    let form_wins = recent_form.iter().filter(|entry| entry.win).count();
    let decided = (wins + losses) as u64;
    let (win_rate_low_percent, win_rate_high_percent) = wilson_interval(wins as u64, decided);

    WindowStats {
        matches: matches.len(),
        wins,
        losses,
        unscored: matches.len().saturating_sub(wins + losses),
        win_rate_percent: percent(wins as u64, decided),
        kills,
        deaths,
        assists,
        kda: kda(kills, deaths, assists),
        average_duration_seconds: average(duration_total as f64, duration_count as u64),
        average_net_worth: average(net_worth_total as f64, matches.len() as u64),
        win_rate_low_percent,
        win_rate_high_percent,
        recent_win_rate_percent: percent(form_wins as u64, recent_form.len() as u64),
        recent_form,
    }
}

/// Percentile of a badge among ranked players, counting badges at or below it.
pub fn rank_benchmark(badge: u32, distribution: &[RankDistributionEntry]) -> Option<RankBenchmark> {
    if badge == 0 {
        return None;
    }

    let ranked_players: u64 = distribution
        .iter()
        .filter(|entry| entry.badge > 0)
        .map(|entry| entry.players)
        .sum();
    if ranked_players == 0 {
        return None;
    }

    let at_or_below: u64 = distribution
        .iter()
        .filter(|entry| entry.badge > 0 && entry.badge <= badge)
        .map(|entry| entry.players)
        .sum();

    let at_or_above: u64 = distribution
        .iter()
        .filter(|entry| entry.badge >= badge)
        .map(|entry| entry.players)
        .sum();

    Some(RankBenchmark {
        badge,
        ranked_players,
        percentile: percent(at_or_below, ranked_players),
        top_percent: percent(at_or_above, ranked_players),
        window: "last 30 days (API default)".to_string(),
    })
}

/// Name the rows, sort heroes by sample size and derive the totals. Shared by
/// the live fetch and by a captured bundle so both produce the same snapshot.
#[allow(clippy::too_many_arguments)]
fn assemble_snapshot(
    account_id: u64,
    profile: PlayerProfile,
    mut rank: PlayerRank,
    mut heroes: Vec<HeroPerformance>,
    mut matches: Vec<MatchSummary>,
    hero_names: &HashMap<u32, String>,
    tier_names: BTreeMap<u32, String>,
    generated_unix: i64,
) -> StatsSnapshot {
    rank.tier_name = tier_names.get(&rank.rank).cloned();
    for hero in &mut heroes {
        hero.hero_name = hero_names.get(&hero.hero_id).cloned();
    }
    for summary in &mut matches {
        summary.hero_name = hero_names.get(&summary.hero_id).cloned();
    }
    heroes.sort_by(|left, right| {
        right
            .matches_played
            .cmp(&left.matches_played)
            .then(left.hero_id.cmp(&right.hero_id))
    });
    let career = career_totals(&heroes);
    let window = window_stats(&matches);
    StatsSnapshot {
        account_id,
        source: STATS_API_BASE.to_string(),
        generated_at: unix_seconds_to_iso(generated_unix),
        profile,
        rank,
        career,
        heroes,
        matches,
        window,
        generated_unix,
        tier_names,
        peers: None,
    }
}

fn peer_benchmarks(
    min_badge: u32,
    max_badge: u32,
    tier_name: Option<String>,
    rows: Vec<RawHeroBenchmark>,
) -> PeerBenchmarks {
    let mut total = RawHeroBenchmark::default();
    let mut heroes = Vec::with_capacity(rows.len());
    for row in rows {
        total.wins += row.wins;
        total.losses += row.losses;
        total.matches += row.matches;
        total.total_kills += row.total_kills;
        total.total_deaths += row.total_deaths;
        total.total_assists += row.total_assists;
        total.total_net_worth += row.total_net_worth;
        heroes.push(row.into_benchmark());
    }
    PeerBenchmarks {
        min_badge,
        max_badge,
        tier_name,
        window: "last 30 days".to_string(),
        heroes,
        overall: total.into_benchmark(),
    }
}

/// Responses captured from the API in one file, for an offline fixture.
///
/// Field names and shapes are the API's own (`match_history` is the raw
/// match-history array). The kill and death raster is stored as compact rows
/// `[position_x, position_y, killer_team, kills, deaths]` to keep it small.
#[derive(Debug, Deserialize)]
struct ApiBundle {
    account_id: u64,
    generated_at: i64,
    #[serde(default)]
    peer_badges: Option<(u32, u32)>,
    steam: Vec<RawSteamProfile>,
    rank: RawRankResponse,
    #[serde(default)]
    rank_distribution: Vec<RawRankDistribution>,
    #[serde(default)]
    rank_names: BTreeMap<String, String>,
    #[serde(default)]
    hero_names: BTreeMap<String, String>,
    hero_stats: Vec<RawHeroStats>,
    match_history: Vec<RawMatch>,
    #[serde(default)]
    kill_death: Vec<(i32, i32, i32, u64, u64)>,
    #[serde(default)]
    hero_benchmarks: Vec<RawHeroBenchmark>,
    #[serde(default)]
    objective_positions: BTreeMap<String, RawRelativePosition>,
}

#[derive(Debug, Deserialize)]
struct RawRelativePosition {
    left_relative: f64,
    top_relative: f64,
}

#[derive(Debug, Deserialize)]
struct RawMapAsset {
    #[serde(default)]
    objective_positions: BTreeMap<String, RawRelativePosition>,
}

fn map_objectives(raw: BTreeMap<String, RawRelativePosition>) -> Vec<MapObjective> {
    raw.into_iter()
        .map(|(key, position)| MapObjective {
            key,
            left: position.left_relative,
            top: position.top_relative,
        })
        .collect()
}

/// Build a snapshot and a heatmap from a captured [`ApiBundle`] JSON file,
/// through the same parsing and derivation as a live fetch.
pub fn snapshot_from_bundle(json: &str) -> Result<CapturedStats> {
    let bundle: ApiBundle = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Stats bundle did not parse: {error}")))?;
    let profile = bundle
        .steam
        .into_iter()
        .find(|row| row.account_id == bundle.account_id)
        .map(RawSteamProfile::into_profile)
        .ok_or_else(|| Error::msg("Stats bundle has no Steam profile for its account id."))?;
    let parse_keys = |map: BTreeMap<String, String>| -> BTreeMap<u32, String> {
        map.into_iter()
            .filter_map(|(key, value)| key.parse::<u32>().ok().map(|key| (key, value)))
            .collect()
    };
    let tier_names = parse_keys(bundle.rank_names);
    let hero_names: HashMap<u32, String> = parse_keys(bundle.hero_names).into_iter().collect();
    let distribution: Vec<RankDistributionEntry> = bundle
        .rank_distribution
        .into_iter()
        .map(RawRankDistribution::into_entry)
        .collect();
    let mut rank = bundle.rank.into_rank();
    rank.benchmark = rank_benchmark(rank.badge, &distribution);
    let heroes = bundle
        .hero_stats
        .into_iter()
        .map(RawHeroStats::into_performance)
        .collect();
    let mut rows = bundle.match_history;
    rows.sort_by_key(|row| std::cmp::Reverse(row.start_time));
    let matches = rows.into_iter().map(RawMatch::into_summary).collect();
    let mut snapshot = assemble_snapshot(
        bundle.account_id,
        profile,
        rank,
        heroes,
        matches,
        &hero_names,
        tier_names,
        bundle.generated_at,
    );
    if let Some((min_badge, max_badge)) = bundle.peer_badges
        && !bundle.hero_benchmarks.is_empty()
    {
        let tier_name = snapshot.tier_names.get(&(min_badge / 10)).cloned();
        snapshot.peers = Some(peer_benchmarks(
            min_badge,
            max_badge,
            tier_name,
            bundle.hero_benchmarks,
        ));
    }
    let points: Vec<KillHeatPoint> = bundle
        .kill_death
        .into_iter()
        .map(|(position_x, position_y, killer_team, kills, deaths)| KillHeatPoint {
            position_x,
            position_y,
            killer_team,
            kills,
            deaths,
        })
        .collect();
    let heatmap = KillHeatmap {
        account_id: bundle.account_id,
        window: "last 30 days".to_string(),
        cells: points.len(),
        kills: points.iter().map(|point| point.kills).sum(),
        deaths: points.iter().map(|point| point.deaths).sum(),
        points,
    };
    Ok(CapturedStats {
        snapshot,
        heatmap,
        objectives: map_objectives(bundle.objective_positions),
    })
}

fn ensure_account(account_id: u64) -> Result<()> {
    if account_id == 0 {
        return Err(Error::msg(
            "Account id must be a SteamID3 greater than zero. Leaderboard entries and the Steam \
             profile search show the account id.",
        ));
    }
    Ok(())
}

fn read_api_key() -> Option<String> {
    std::env::var(STATS_API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn status_hint(status: u16, retry_after: Option<&str>) -> String {
    let retry = retry_after
        .map(|seconds| format!(" Retry after {seconds} second(s)."))
        .unwrap_or_default();

    match status {
        429 => format!(
            " Rate limited.{retry} The API asks clients not to send uncached requests in parallel."
        ),
        503 => {
            format!(" The database queue is busy.{retry} Narrow the query or wait before retrying.")
        }
        504 => format!(
            " The query hit the time limit.{retry} Narrow the query instead of retrying it unchanged."
        ),
        401 | 403 => format!(
            " The endpoint refused the request. If it needs a key, set {STATS_API_KEY_ENV}."
        ),
        404 => " The API has no record for this identifier.".to_string(),
        _ => String::new(),
    }
}

fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (part as f64 / total as f64) * 100.0
}

/// 95 percent Wilson score interval for a binomial win rate.
///
/// Wilson is used instead of the normal approximation because hero samples can
/// be small and a raw percentage hides that uncertainty. The formula is the
/// standard one with z = 1.959963984540054:
///
/// center = (p + z^2 / 2n) / (1 + z^2 / n)
/// margin = z * sqrt(p(1 - p) / n + z^2 / 4n^2) / (1 + z^2 / n)
fn wilson_interval(wins: u64, total: u64) -> (f64, f64) {
    if total == 0 {
        return (0.0, 100.0);
    }

    const Z: f64 = 1.959_963_984_540_054;
    let n = total as f64;
    let p = wins as f64 / n;
    let denominator = 1.0 + (Z * Z) / n;
    let center = (p + (Z * Z) / (2.0 * n)) / denominator;
    let margin = Z * (p * (1.0 - p) / n + (Z * Z) / (4.0 * n * n)).sqrt() / denominator;

    (
        ((center - margin).max(0.0) * 100.0),
        ((center + margin).min(1.0) * 100.0),
    )
}

fn kda(kills: u64, deaths: u64, assists: u64) -> f64 {
    (kills + assists) as f64 / deaths.max(1) as f64
}

fn average(total: f64, count: u64) -> f64 {
    if count == 0 {
        return 0.0;
    }
    total / count as f64
}

fn minutes(seconds: u64) -> f64 {
    seconds as f64 / 60.0
}

fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "The response body was empty.".to_string();
    }

    let mut text: String = collapsed.chars().take(BODY_SNIPPET_CHARS).collect();
    if collapsed.chars().count() > BODY_SNIPPET_CHARS {
        text.push_str("...");
    }
    format!("Response starts with: {text}")
}

#[derive(Debug, Deserialize)]
struct RawSteamProfile {
    account_id: u64,
    personaname: String,
    profileurl: String,
    avatar: String,
    avatarmedium: String,
    avatarfull: String,
    #[serde(default)]
    countrycode: Option<String>,
    #[serde(default)]
    realname: Option<String>,
    #[serde(default)]
    last_updated: String,
    #[serde(default)]
    matches_played_last_30d: u64,
    #[serde(default)]
    last_team_avg_badge: Option<u32>,
    #[serde(default)]
    friends: Vec<serde_json::Value>,
}

impl RawSteamProfile {
    fn into_profile(self) -> PlayerProfile {
        PlayerProfile {
            account_id: self.account_id,
            persona_name: self.personaname,
            profile_url: self.profileurl,
            avatar: self.avatar,
            avatar_medium: self.avatarmedium,
            avatar_full: self.avatarfull,
            country_code: self.countrycode,
            real_name: self.realname,
            last_updated: self.last_updated,
            matches_played_last_30d: self.matches_played_last_30d,
            last_team_avg_badge: self.last_team_avg_badge,
            friend_count: self.friends.len(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawRankResponse {
    badge: u32,
    rank: u32,
    subrank: u32,
    #[serde(default)]
    last_match: Option<RawRankMatch>,
}

impl RawRankResponse {
    fn into_rank(self) -> PlayerRank {
        PlayerRank {
            badge: self.badge,
            rank: self.rank,
            subrank: self.subrank,
            tier_name: None,
            last_match: self.last_match.map(RawRankMatch::into_match),
            benchmark: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawRankMatch {
    match_id: u64,
    start_time: i64,
    #[serde(default)]
    player_rank_initial_display_rank: Option<u32>,
    #[serde(default)]
    player_rank_initial_flat_progress: Option<i64>,
    #[serde(default)]
    player_rank_final_flat_progress: Option<i64>,
    #[serde(default)]
    player_rank_desired_progress_change: Option<i64>,
    #[serde(default)]
    player_rank_initial_calibration_games: Option<u32>,
    #[serde(default)]
    player_rank_initial_win_streak: Option<u32>,
}

impl RawRankMatch {
    fn into_match(self) -> RankedMatch {
        RankedMatch {
            match_id: self.match_id,
            start_time: self.start_time,
            started_at: unix_seconds_to_iso(self.start_time),
            initial_display_rank: self.player_rank_initial_display_rank,
            initial_flat_progress: self.player_rank_initial_flat_progress,
            final_flat_progress: self.player_rank_final_flat_progress,
            desired_progress_change: self.player_rank_desired_progress_change,
            initial_calibration_games: self.player_rank_initial_calibration_games,
            initial_win_streak: self.player_rank_initial_win_streak,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawHeroStats {
    hero_id: u32,
    matches_played: u64,
    last_played: i64,
    time_played: u64,
    wins: u64,
    kills: u64,
    deaths: u64,
    assists: u64,
    #[serde(default)]
    damage_per_min: f64,
    #[serde(default)]
    networth_per_min: f64,
    #[serde(default)]
    obj_damage_per_min: f64,
    #[serde(default)]
    damage_taken_per_min: f64,
    #[serde(default)]
    denies_per_match: f64,
    #[serde(default)]
    total_boss_damage: u64,
}

impl RawHeroStats {
    fn into_performance(self) -> HeroPerformance {
        let losses = self.matches_played.saturating_sub(self.wins);
        let (win_rate_low_percent, win_rate_high_percent) =
            wilson_interval(self.wins, self.matches_played);
        HeroPerformance {
            hero_id: self.hero_id,
            hero_name: None,
            matches_played: self.matches_played,
            wins: self.wins,
            losses,
            win_rate_percent: percent(self.wins, self.matches_played),
            kills: self.kills,
            deaths: self.deaths,
            assists: self.assists,
            kda: kda(self.kills, self.deaths, self.assists),
            time_played_seconds: self.time_played,
            average_match_seconds: average(self.time_played as f64, self.matches_played),
            damage_per_minute: self.damage_per_min,
            net_worth_per_minute: self.networth_per_min,
            objective_damage_per_minute: self.obj_damage_per_min,
            damage_taken_per_minute: self.damage_taken_per_min,
            denies_per_match: self.denies_per_match,
            boss_damage: self.total_boss_damage,
            win_rate_low_percent,
            win_rate_high_percent,
            last_played: unix_seconds_to_iso(self.last_played),
            last_played_unix: self.last_played,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawMatch {
    match_id: u64,
    hero_id: u32,
    start_time: i64,
    #[serde(default)]
    match_duration_s: i32,
    game_mode: i32,
    match_mode: i32,
    player_team: i32,
    player_kills: u32,
    player_deaths: u32,
    player_assists: u32,
    net_worth: u64,
    last_hits: u32,
    denies: u32,
    player_match_outcome: i32,
    #[serde(default)]
    abandoned_time_s: Option<u32>,
    #[serde(default)]
    ranked_display_badge: Option<u32>,
    #[serde(default)]
    ranked_delta: Option<i32>,
}

impl RawMatch {
    fn into_summary(self) -> MatchSummary {
        let outcome = MatchOutcome::from_api(self.player_match_outcome);
        MatchSummary {
            match_id: self.match_id,
            hero_id: self.hero_id,
            hero_name: None,
            start_time: self.start_time,
            started_at: unix_seconds_to_iso(self.start_time),
            duration_seconds: self.match_duration_s.max(0) as u32,
            game_mode: self.game_mode,
            match_mode: self.match_mode,
            player_team: self.player_team,
            kills: self.player_kills,
            deaths: self.player_deaths,
            assists: self.player_assists,
            kda: kda(
                self.player_kills as u64,
                self.player_deaths as u64,
                self.player_assists as u64,
            ),
            net_worth: self.net_worth,
            last_hits: self.last_hits,
            denies: self.denies,
            outcome,
            win: outcome.win(),
            abandoned_seconds: self.abandoned_time_s,
            ranked_badge: self.ranked_display_badge,
            ranked_delta: self.ranked_delta,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawHeroBenchmark {
    #[serde(default)]
    hero_id: u32,
    #[serde(default)]
    wins: u64,
    #[serde(default)]
    losses: u64,
    #[serde(default)]
    matches: u64,
    #[serde(default)]
    total_kills: u64,
    #[serde(default)]
    total_deaths: u64,
    #[serde(default)]
    total_assists: u64,
    #[serde(default)]
    total_net_worth: u64,
}

impl RawHeroBenchmark {
    fn into_benchmark(self) -> HeroBenchmark {
        HeroBenchmark {
            hero_id: self.hero_id,
            matches: self.matches,
            wins: self.wins,
            losses: self.losses,
            win_rate_percent: percent(self.wins, self.wins + self.losses),
            kda: kda(self.total_kills, self.total_deaths, self.total_assists),
            deaths_per_match: average(self.total_deaths as f64, self.matches),
            net_worth_per_match: average(self.total_net_worth as f64, self.matches),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawHeroAsset {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawRankAsset {
    tier: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawRankDistribution {
    badge: u32,
    rank: u32,
    subrank: u32,
    players: u64,
}

impl RawRankDistribution {
    fn into_entry(self) -> RankDistributionEntry {
        RankDistributionEntry {
            badge: self.badge,
            rank: self.rank,
            subrank: self.subrank,
            players: self.players,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawKillDeath {
    position_x: i32,
    position_y: i32,
    killer_team: i32,
    kills: u64,
    deaths: u64,
}

impl RawKillDeath {
    fn into_point(self) -> KillHeatPoint {
        KillHeatPoint {
            position_x: self.position_x,
            position_y: self.position_y,
            killer_team: self.killer_team,
            kills: self.kills,
            deaths: self.deaths,
        }
    }
}

