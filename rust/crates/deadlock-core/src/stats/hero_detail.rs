//! One hero, one player: the matchups and the items behind a hero page.
//!
//! Sources:
//!
//! - `GET /v1/analytics/hero-counter-stats?account_id=<id>&min_matches=2`: the player's record
//!   on each hero against each enemy hero (wins and matches from the player's side). One
//!   request covers every hero the player has; the page keeps the rows of the hero it shows.
//!   The analytics budget is charged by data read, so the answer is kept 12 hours.
//! - `GET /v1/analytics/item-stats?account_id=<id>&hero_id=<hero>`: the items the player buys
//!   on that hero, with win rate and buy time, kept 12 hours.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::stats::filters::{StatsFilter, ensure_account, filter_query, get_text};
use crate::stats::items::{ItemStat, parse_item_stats};

/// How long a hero page's analytics answers are kept.
pub const HERO_TTL: i64 = 12 * 3_600;
/// Matchup rows need this many matches to be listed.
pub const MIN_MATCHUP_MATCHES: u64 = 2;

/// The player on one hero against one enemy hero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matchup {
    pub enemy_hero_id: u32,
    pub matches: u64,
    pub wins: u64,
    /// The player's souls minus the enemy hero's, a match on average.
    pub soul_gap: f64,
}

impl Matchup {
    /// Win rate from [`crate::stats::social::MIN_RATE_SAMPLE`] matches up.
    pub fn rate(&self) -> Option<f64> {
        crate::stats::social::rate(self.wins, self.matches)
    }
}

#[derive(Debug, Deserialize)]
struct RawCounter {
    hero_id: u32,
    enemy_hero_id: u32,
    #[serde(default)]
    wins: u64,
    #[serde(default)]
    matches_played: u64,
    #[serde(default)]
    networth: u64,
    #[serde(default)]
    enemy_networth: u64,
}

/// The matchups of `hero_id` from a counter-stats answer, most played first.
pub fn parse_matchups(json: &str, hero_id: u32) -> Result<Vec<Matchup>> {
    let rows: Vec<RawCounter> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The hero matchup response did not parse: {error}")))?;
    let mut out: Vec<Matchup> = rows
        .into_iter()
        .filter(|row| row.hero_id == hero_id && row.matches_played >= MIN_MATCHUP_MATCHES)
        .map(|row| Matchup {
            enemy_hero_id: row.enemy_hero_id,
            matches: row.matches_played,
            wins: row.wins.min(row.matches_played),
            soul_gap: (row.networth as f64 - row.enemy_networth as f64) / row.matches_played.max(1) as f64,
        })
        .collect();
    out.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.enemy_hero_id.cmp(&b.enemy_hero_id)));
    Ok(out)
}

/// The player's matchups on one hero, inside the page filter's mode and window.
pub fn matchups(account: u32, hero_id: u32, filter: &StatsFilter) -> Result<Vec<Matchup>> {
    ensure_account(account)?;
    let mut query = vec![("account_id", account.to_string()), ("min_matches", MIN_MATCHUP_MATCHES.to_string())];
    query.extend(filter_query(filter));
    parse_matchups(&get_text("/v1/analytics/hero-counter-stats", &query, HERO_TTL)?, hero_id)
}

/// The items the player buys on one hero, most bought first.
pub fn hero_items(account: u32, hero_id: u32, filter: &StatsFilter) -> Result<Vec<ItemStat>> {
    ensure_account(account)?;
    let mut query = vec![
        ("account_id", account.to_string()),
        ("hero_id", hero_id.to_string()),
        ("min_matches", crate::stats::items::MIN_ITEM_MATCHES.to_string()),
    ];
    query.extend(filter_query(filter));
    let mut rows = parse_item_stats(&get_text("/v1/analytics/item-stats", &query, HERO_TTL)?)?;
    rows.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.item_id.cmp(&b.item_id)));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTERS: &str = r#"[
        {"hero_id":4,"enemy_hero_id":1,"wins":2,"matches_played":4,"networth":164100,"enemy_networth":155735},
        {"hero_id":4,"enemy_hero_id":7,"wins":9,"matches_played":12,"networth":400000,"enemy_networth":412000},
        {"hero_id":4,"enemy_hero_id":9,"wins":1,"matches_played":1,"networth":1,"enemy_networth":1},
        {"hero_id":63,"enemy_hero_id":1,"wins":2,"matches_played":3}
    ]"#;

    #[test]
    fn matchups_keep_one_hero_most_played_first() {
        let rows = parse_matchups(COUNTERS, 4).unwrap();
        assert_eq!(rows.iter().map(|m| m.enemy_hero_id).collect::<Vec<_>>(), vec![7, 1], "one match is under the floor");
        assert_eq!(rows[0].rate(), Some(75.0));
        assert_eq!(rows[1].rate(), None, "four matches is below the rate floor");
        assert!((rows[0].soul_gap - (-1000.0)).abs() < 1e-9);
        assert!(parse_matchups(COUNTERS, 99).unwrap().is_empty());
        assert!(parse_matchups("{", 4).is_err());
    }
}
