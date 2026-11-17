//! The public leaderboard: one region, one season, and the player's place in it.
//!
//! Source: `GET /v1/leaderboard/{region}?leaderboard_id=<id>` for the board and
//! `GET /v1/assets/ranked-seasons` for the season list. A season is listed only
//! once it names a `leaderboard_id`; the newest one that has started is the board
//! played now.
//!
//! `rank` on a leaderboard row is a place, not a rank badge, and Steam names are
//! not unique, so a row carries every account id that could be behind the name.
//! The player's row is the one whose ids contain their account id.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::stats::{Leaderboard, LeaderboardEntry};

/// One ranked season, as the leaderboard knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedSeason {
    pub leaderboard_id: u32,
    pub name: String,
    pub starts_at: i64,
    pub ends_at: i64,
}

impl RankedSeason {
    pub fn has_started(&self, now: i64) -> bool {
        self.starts_at <= now
    }

    pub fn is_running(&self, now: i64) -> bool {
        self.starts_at <= now && now < self.ends_at
    }
}

/// Parse `/v1/assets/ranked-seasons` into the seasons that have a board, newest
/// first. A season can hold several intervals; the newest one that names a
/// leaderboard id is the board the season is played on.
pub fn parse_seasons(json: &str) -> Result<Vec<RankedSeason>> {
    let rows: Vec<RawSeason> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Ranked seasons did not parse: {error}")))?;
    Ok(seasons_from_rows(rows))
}

fn seasons_from_rows(rows: Vec<RawSeason>) -> Vec<RankedSeason> {
    let mut out: Vec<RankedSeason> = rows
        .into_iter()
        .filter_map(|row| {
            let (id, start, end) = row
                .intervals
                .into_iter()
                .filter_map(|interval| {
                    let id = interval.leaderboard_id.filter(|id| *id > 0)?;
                    Some((id, interval.start_timestamp, interval.end_timestamp))
                })
                .max_by_key(|(_, start, _)| *start)?;
            Some(RankedSeason {
                leaderboard_id: id,
                name: row.name.or(Some(row.class_name)).filter(|name| !name.trim().is_empty()).unwrap_or_else(|| format!("Board {id}")),
                starts_at: start,
                ends_at: end,
            })
        })
        .collect();
    out.sort_by_key(|season| std::cmp::Reverse(season.starts_at));
    out
}

/// The board to show: the newest season Valve has started.
pub fn current_season(seasons: &[RankedSeason], now: i64) -> Option<&RankedSeason> {
    seasons.iter().find(|season| season.has_started(now))
}

/// The season before `season`, for the fallback when the current board is empty.
pub fn previous_season<'a>(
    seasons: &'a [RankedSeason],
    season: &RankedSeason,
) -> Option<&'a RankedSeason> {
    seasons
        .iter()
        .find(|candidate| {
            candidate.leaderboard_id != season.leaderboard_id && candidate.starts_at < season.starts_at
        })
}

/// The leaderboard region a country code belongs to, for the board's default.
/// The API keys regions by name, not by country, so this is the coarse mapping
/// a player would pick by hand; anything unknown gets no default.
pub fn region_for_country(country: Option<&str>) -> Option<&'static str> {
    let country = country?.trim().to_ascii_uppercase();
    let europe = [
        "AL", "AD", "AT", "BY", "BE", "BA", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE",
        "GR", "HU", "IS", "IE", "IT", "XK", "LV", "LI", "LT", "LU", "MT", "MD", "MC", "ME", "NL",
        "MK", "NO", "PL", "PT", "RO", "RU", "SM", "RS", "SK", "SI", "ES", "SE", "CH", "TR", "UA",
        "GB", "VA",
    ];
    let na = ["US", "CA", "MX", "PR", "DO", "GT", "CR", "PA", "JM", "TT", "SV", "HN", "NI", "BZ"];
    let sa = ["BR", "AR", "CL", "CO", "PE", "VE", "EC", "BO", "PY", "UY", "GY", "SR"];
    let asia = [
        "CN", "JP", "KR", "TW", "HK", "SG", "MY", "TH", "VN", "PH", "ID", "IN", "PK", "BD", "LK",
        "NP", "MN", "KZ", "UZ", "SA", "AE", "IL", "QA", "KW", "BH", "OM", "JO", "LB",
    ];
    let oceania = ["AU", "NZ", "FJ", "PG"];
    let code = country.as_str();
    if europe.contains(&code) {
        Some("Europe")
    } else if na.contains(&code) {
        Some("NAmerica")
    } else if sa.contains(&code) {
        Some("SAmerica")
    } else if asia.contains(&code) {
        Some("Asia")
    } else if oceania.contains(&code) {
        Some("Oceania")
    } else {
        None
    }
}

/// Parse one leaderboard response body into rows ordered by place.
pub(crate) fn parse_entries(json: &str) -> Result<Vec<LeaderboardEntry>> {
    let rows: Vec<RawEntry> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The leaderboard did not parse: {error}")))?;
    Ok(entries_from_rows(rows))
}

fn entries_from_rows(rows: Vec<RawEntry>) -> Vec<LeaderboardEntry> {
    let mut entries: Vec<LeaderboardEntry> = rows
        .into_iter()
        .map(|row| LeaderboardEntry {
            rank: row.rank,
            account_name: row.account_name.filter(|name| !name.trim().is_empty()),
            possible_account_ids: row.possible_account_ids,
            top_hero_ids: row.top_hero_ids,
        })
        .collect();
    entries.sort_by_key(|entry| entry.rank.unwrap_or(u32::MAX));
    entries
}

/// The pieces of a captured leaderboard, for a fixture scene.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedLeaderboard {
    pub region: String,
    pub seasons: Vec<RankedSeason>,
    pub board: Leaderboard,
    pub account_id: u64,
}

/// A bundle captured from the API in one file, for an offline fixture.
#[derive(Debug, Deserialize)]
struct BoardBundle {
    #[serde(default)]
    seasons: Vec<RawSeason>,
    /// Raw board rows for the season named by `season_id`.
    #[serde(default)]
    leaderboard: RawBoard,
    #[serde(default)]
    season_id: u32,
    #[serde(default)]
    account_id: u64,
}

#[derive(Debug, Deserialize, Default)]
struct RawBoard {
    #[serde(default)]
    region: String,
    #[serde(default)]
    entries: Vec<RawEntry>,
}

/// Read a captured leaderboard bundle through the same assembly as a live fetch.
pub fn board_from_bundle(json: &str) -> Result<CapturedLeaderboard> {
    let bundle: BoardBundle = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Leaderboard bundle did not parse: {error}")))?;
    let seasons = seasons_from_rows(bundle.seasons);
    let season = seasons
        .iter()
        .find(|season| season.leaderboard_id == bundle.season_id)
        .or_else(|| current_season(&seasons, i64::MAX))
        .cloned()
        .ok_or_else(|| Error::msg("Leaderboard bundle has no season with a board."))?;
    let region = if bundle.leaderboard.region.is_empty() {
        "Europe".to_string()
    } else {
        bundle.leaderboard.region
    };
    Ok(CapturedLeaderboard {
        board: Leaderboard {
            region: region.clone(),
            season_id: Some(season.leaderboard_id),
            season_name: Some(season.name.clone()),
            entries: entries_from_rows(bundle.leaderboard.entries),
        },
        region,
        seasons,
        account_id: bundle.account_id,
    })
}

#[derive(Debug, Deserialize, Clone)]
struct RawSeason {
    #[serde(default)]
    class_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    intervals: Vec<RawInterval>,
}

#[derive(Debug, Deserialize, Clone)]
struct RawInterval {
    #[serde(default)]
    leaderboard_id: Option<u32>,
    #[serde(default)]
    start_timestamp: i64,
    #[serde(default)]
    end_timestamp: i64,
}

#[derive(Debug, Deserialize, Clone)]
struct RawEntry {
    #[serde(default)]
    rank: Option<u32>,
    #[serde(default)]
    account_name: Option<String>,
    #[serde(default)]
    possible_account_ids: Vec<u64>,
    #[serde(default)]
    top_hero_ids: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEASONS: &str = r#"[
        {"class_name": "season_9", "name": "No board", "intervals": []},
        {"class_name": "season_1", "name": "Beta Season 1", "intervals": [
            {"interval": 1, "start_timestamp": 1785430800, "end_timestamp": 1793908800, "leaderboard_id": 1001}]},
        {"class_name": "season_2", "name": "Beta Season 2", "intervals": [
            {"interval": 2, "start_timestamp": 1793908800, "end_timestamp": 1804802400, "leaderboard_id": 1002}]},
        {"class_name": "season_3", "name": "Not started", "intervals": [
            {"interval": 3, "start_timestamp": 1900000000, "end_timestamp": 1905000000, "leaderboard_id": 1003}]}
    ]"#;

    #[test]
    fn seasons_need_a_board_and_are_newest_first() {
        let seasons = parse_seasons(SEASONS).expect("seasons");
        assert_eq!(seasons.len(), 3);
        assert_eq!(seasons[0].leaderboard_id, 1003);
        assert_eq!(seasons[2].leaderboard_id, 1001);
        assert_eq!(seasons[1].name, "Beta Season 2");
    }

    #[test]
    fn the_current_season_is_the_newest_that_started() {
        let seasons = parse_seasons(SEASONS).expect("seasons");
        let now = 1794841957;
        let current = current_season(&seasons, now).expect("current");
        assert_eq!(current.leaderboard_id, 1002);
        assert!(current.is_running(now));
        let previous = previous_season(&seasons, current).expect("previous");
        assert_eq!(previous.leaderboard_id, 1001);
    }

    #[test]
    fn a_board_before_the_first_season_is_none() {
        let seasons = parse_seasons(SEASONS).expect("seasons");
        assert!(current_season(&seasons, 1_600_000_000).is_none());
        assert_eq!(
            current_season(&seasons, i64::MAX).map(|season| season.leaderboard_id),
            Some(1003)
        );
        assert!(current_season(&[], 1).is_none());
    }

    #[test]
    fn a_country_maps_to_the_region_a_player_would_pick() {
        assert_eq!(region_for_country(Some("de")), Some("Europe"));
        assert_eq!(region_for_country(Some("US")), Some("NAmerica"));
        assert_eq!(region_for_country(Some("BR")), Some("SAmerica"));
        assert_eq!(region_for_country(Some("JP")), Some("Asia"));
        assert_eq!(region_for_country(Some("AU")), Some("Oceania"));
        assert_eq!(region_for_country(Some("AQ")), None);
        assert_eq!(region_for_country(None), None);
    }

    #[test]
    fn entries_are_ordered_by_place_and_kept_whole() {
        let rows = r#"[
            {"rank": 7, "account_name": "Me", "possible_account_ids": [138006262], "top_hero_ids": [63]},
            {"rank": 1, "account_name": "Top", "possible_account_ids": [1, 2]}
        ]"#;
        let entries = parse_entries(rows).expect("entries");
        assert_eq!(entries[0].rank, Some(1));
        assert_eq!(entries[1].possible_account_ids, vec![138006262]);
        assert!(parse_entries("{}").is_err());
    }
}
