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

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, unix_seconds_to_iso};

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
    pub entries: Vec<LeaderboardEntry>,
}

/// Shared HTTP client for the public Deadlock API.
///
/// The client is read only. It never sends player identifiers anywhere except
/// to `api.deadlock-api.com`, and the optional API key is only used as a header.
pub struct StatsClient {
    agent: ureq::Agent,
    base_url: String,
    api_key: Option<String>,
}

impl StatsClient {
    /// Build a client, reading an optional `DEADLOCK_API_KEY` from the environment.
    pub fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
                .http_status_as_error(false)
                .build()
                .new_agent(),
            base_url: STATS_API_BASE.to_string(),
            api_key: read_api_key(),
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
        let rows: Vec<RawSteamProfile> = self.get(
            "/v1/players/steam",
            &[("account_ids", account_id.to_string())],
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

    /// Rank at the end of the latest ranked match.
    pub fn fetch_rank(&self, account_id: u64) -> Result<PlayerRank> {
        ensure_account(account_id)?;
        let raw: RawRankResponse = self.get(&format!("/v1/players/{account_id}/rank"), &[])?;
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
        let rows: Vec<RawRankDistribution> = self.get("/v1/players/rank/distribution", &[])?;
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

        let mut rows: Vec<RawMatch> =
            self.get(&format!("/v1/players/{account_id}/match-history"), &[])?;
        rows.sort_by(|left, right| right.start_time.cmp(&left.start_time));
        rows.truncate(limit);
        Ok(rows.into_iter().map(RawMatch::into_summary).collect())
    }

    /// Lifetime per-hero rows with the API's default filters.
    pub fn fetch_hero_stats(&self, account_id: u64) -> Result<Vec<HeroPerformance>> {
        ensure_account(account_id)?;
        let rows: Vec<RawHeroStats> = self.get(
            "/v1/players/hero-stats",
            &[("account_ids", account_id.to_string())],
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
        let rows: Vec<RawHeroAsset> = self.get("/v1/assets/heroes", &[])?;
        Ok(rows.into_iter().map(|row| (row.id, row.name)).collect())
    }

    /// Rank tier to display name, from `/v1/assets/ranks`.
    pub fn fetch_rank_names(&self) -> Result<HashMap<u32, String>> {
        let rows: Vec<RawRankAsset> = self.get("/v1/assets/ranks", &[])?;
        Ok(rows.into_iter().map(|row| (row.tier, row.name)).collect())
    }

    /// One region's leaderboard, truncated to `limit` entries.
    pub fn fetch_leaderboard(&self, region: &str, limit: usize) -> Result<Leaderboard> {
        if !LEADERBOARD_REGIONS.contains(&region) {
            return Err(Error::msg(format!(
                "Unknown leaderboard region {region:?}. Use one of {}.",
                LEADERBOARD_REGIONS.join(", ")
            )));
        }
        if limit == 0 {
            return Err(Error::msg("Leaderboard limit must be 1 or greater."));
        }

        let raw: RawLeaderboard = self.get(&format!("/v1/leaderboard/{region}"), &[])?;
        let mut entries: Vec<LeaderboardEntry> = raw
            .entries
            .into_iter()
            .map(LeaderboardEntry::from)
            .collect();
        entries.truncate(limit);

        Ok(Leaderboard {
            region: region.to_string(),
            entries,
        })
    }

    /// Aggregate kill and death positions for the last 30 days.
    ///
    /// This is the only heatmap the public API exposes synchronously. It covers
    /// all matches in the window, not a selected match. See
    /// `docs/research/stats.md` for the demo-query gap.
    pub fn fetch_kill_heatmap(&self, account_id: u64) -> Result<KillHeatmap> {
        ensure_account(account_id)?;
        let rows: Vec<RawKillDeath> = self.get(
            "/v1/analytics/kill-death-stats",
            &[("account_ids", account_id.to_string())],
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
        let profile = self.fetch_player(account_id)?;
        let mut rank = self.fetch_rank(account_id)?;
        rank.benchmark = self.fetch_rank_benchmark(rank.badge).unwrap_or(None);
        let mut heroes = self.fetch_hero_stats(account_id)?;
        let mut matches = self.fetch_matches(account_id, match_limit)?;

        let hero_names = self.fetch_hero_names().unwrap_or_default();
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

        Ok(StatsSnapshot {
            account_id,
            source: STATS_API_BASE.to_string(),
            generated_at: now_iso(),
            profile,
            rank,
            career,
            heroes,
            matches,
            window,
        })
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
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

        serde_json::from_str(&body).map_err(|error| {
            Error::Network(format!(
                "Deadlock API response for {url} did not match the expected shape: {error}. {}",
                snippet(&body)
            ))
        })
    }
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

    Some(RankBenchmark {
        badge,
        ranked_players,
        percentile: percent(at_or_below, ranked_players),
        window: "last 30 days (API default)".to_string(),
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

#[derive(Debug, Deserialize)]
struct RawLeaderboard {
    #[serde(default)]
    entries: Vec<RawLeaderboardEntry>,
}

#[derive(Debug, Deserialize)]
struct RawLeaderboardEntry {
    #[serde(default)]
    rank: Option<u32>,
    #[serde(default)]
    account_name: Option<String>,
    #[serde(default)]
    possible_account_ids: Vec<u64>,
    #[serde(default)]
    top_hero_ids: Vec<u32>,
}

impl From<RawLeaderboardEntry> for LeaderboardEntry {
    fn from(raw: RawLeaderboardEntry) -> Self {
        Self {
            rank: raw.rank,
            account_name: raw.account_name,
            possible_account_ids: raw.possible_account_ids,
            top_hero_ids: raw.top_hero_ids,
        }
    }
}
