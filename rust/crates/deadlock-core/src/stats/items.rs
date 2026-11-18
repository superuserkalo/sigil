//! What a player buys: item win rates against everyone else, and one match's
//! purchase timeline.
//!
//! Sources:
//!
//! - `GET /v1/analytics/item-stats?account_id=<id>&min_matches=3` for the
//!   player's own rows.
//! - The same endpoint without `account_id` for everyone's rows, the baseline
//!   the win rate delta is measured against. The API caches an analytics answer
//!   for an hour, so the pair costs one body per hour per query on its side.
//! - `GET /v1/matches/{match_id}/metadata?disable_steam=true` for the purchases
//!   in one match. It is the body the scoreboard already reads, kept for a week.
//!
//! The interesting number is not the raw win rate: a strong item wins for
//! everyone. It is the gap to how the item performs in everyone else's hands,
//! and how many seconds later the player buys it.
//!
//! The timeline keeps finished purchases (`upgrade_id == 1`), which is the row
//! the item art and name describe. Component bookkeeping is not a purchase a
//! player recognises. Item names and art come from
//! `/v1/assets/items/by-type/upgrade`, which the scoreboard already caches; this
//! module returns ids and lets the screen join them.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::stats::cache;
use crate::stats::filters::{ANALYTICS_TTL, StatsFilter, ensure_account, filter_query, get_text};

/// The request floor for a row. Fewer matches than this and the row is not
/// worth a query.
pub const MIN_ITEM_MATCHES: u64 = 3;
/// Rows the screen shows. Below five matches one game swings the rate by
/// twenty points.
pub const MIN_ITEM_SAMPLE: u64 = 5;

/// The `upgrade_id` of a finished purchase in a match's item list.
const PURCHASE_UPGRADE_ID: u32 = 1;

/// One item's record for a player, next to everyone's record for the same item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStat {
    pub item_id: u64,
    pub matches: u64,
    pub wins: u64,
    pub losses: u64,
    /// Players in the sample. One for a player's own rows.
    pub players: u64,
    /// Percent from 0 to 100: `wins / matches`.
    pub win_rate_percent: f64,
    /// Everyone's win rate with this item over the same window and mode.
    pub global_win_rate_percent: f64,
    /// `win_rate_percent - global_win_rate_percent`, in percentage points.
    pub win_rate_delta_percent: f64,
    pub avg_buy_time_s: f64,
    pub global_avg_buy_time_s: f64,
    /// Seconds later than everyone else. Negative means earlier.
    pub buy_time_delta_s: f64,
}

/// One finished purchase in a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPurchase {
    pub item_id: u64,
    /// Seconds into the match.
    pub bought_at_s: u32,
    /// Seconds into the match, or `None` while the item was still held.
    pub sold_at_s: Option<u32>,
}

/// A player's item rows, sorted by matches played, with the baseline applied.
///
/// Two requests are made when the player has rows: the player's own and the
/// same query for everyone. When the baseline request fails (a wide window can
/// time out upstream), the rows keep their own rate and a zero delta rather
/// than failing the view. Rows below [`MIN_ITEM_SAMPLE`] matches are dropped,
/// so one lucky game does not lead the list.
pub fn item_stats(account: u32, filter: &StatsFilter) -> Result<Vec<ItemStat>> {
    ensure_account(account)?;
    let mut stats = fetch_stats(Some(account), filter)?;
    if !stats.is_empty()
        && let Ok(baseline) = fetch_stats(None, filter)
    {
        apply_baseline(&mut stats, &baseline);
    }
    stats.retain(|row| row.matches >= MIN_ITEM_SAMPLE);
    Ok(stats)
}

/// Global "best items" rows need at least this many matches, so a rare item with a lucky
/// streak does not top the list.
pub const MIN_GLOBAL_ITEM_SAMPLE: u64 = 500;

/// Everyone's item rows inside `filter` (pass [`StatsFilter::since_last_patch`] for "this
/// patch"), best win rate first. One request, shared with the player view's baseline through
/// the response cache. Rows under [`MIN_GLOBAL_ITEM_SAMPLE`] matches are dropped.
pub fn global_item_stats(filter: &StatsFilter) -> Result<Vec<ItemStat>> {
    Ok(best_items(fetch_stats(None, filter)?, MIN_GLOBAL_ITEM_SAMPLE))
}

/// Rows with at least `min_matches`, by win rate (then matches, then id). The baseline fields
/// equal the row's own, since these rows are the baseline.
pub fn best_items(mut rows: Vec<ItemStat>, min_matches: u64) -> Vec<ItemStat> {
    rows.retain(|row| row.matches >= min_matches);
    rows.sort_by(|left, right| {
        right
            .win_rate_percent
            .total_cmp(&left.win_rate_percent)
            .then(right.matches.cmp(&left.matches))
            .then(left.item_id.cmp(&right.item_id))
    });
    rows
}

/// One match's item purchases for one player, oldest first.
///
/// The metadata body is large (about 900 KB) and never changes, so the cache
/// keeps it for a week. A player the match does not name is an error rather
/// than an empty timeline.
pub fn item_timeline(match_id: u64, account: u32) -> Result<Vec<ItemPurchase>> {
    if match_id == 0 {
        return Err(Error::msg(
            "A match id is needed to open its item timeline.",
        ));
    }
    ensure_account(account)?;
    let json = get_text(
        &format!("/v1/matches/{match_id}/metadata"),
        &[("disable_steam", "true".to_string())],
        cache::TTL_MATCH_METADATA,
    )?;
    parse_item_timeline(&json, account)
}

/// Parse one `/v1/analytics/item-stats` response.
///
/// Rows without an item id or without matches are dropped. The baseline fields
/// start equal to the row's own values, so a parsed response is usable before
/// `apply_baseline` runs.
pub fn parse_item_stats(json: &str) -> Result<Vec<ItemStat>> {
    let rows: Vec<RawItemStat> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Item stats did not parse: {error}")))?;
    let mut stats: Vec<ItemStat> = rows
        .into_iter()
        .filter_map(RawItemStat::into_stat)
        .collect();
    stats.sort_by(|left, right| {
        right
            .matches
            .cmp(&left.matches)
            .then(left.item_id.cmp(&right.item_id))
    });
    Ok(stats)
}

/// Parse one match's metadata into the player's purchases.
///
/// The same body carries both teams. `account` picks the player. Rows with
/// `upgrade_id == 1` are finished purchases; a row without the field counts as
/// one, because a response that omits it cannot tell a component apart.
pub fn parse_item_timeline(json: &str, account: u32) -> Result<Vec<ItemPurchase>> {
    let metadata: RawTimelineMetadata = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Match metadata did not parse: {error}")))?;
    let player = metadata
        .match_info
        .players
        .into_iter()
        .find(|player| player.account_id == account)
        .ok_or_else(|| {
            Error::msg(format!(
                "The match metadata has no player with account id {account}."
            ))
        })?;

    let mut purchases: Vec<ItemPurchase> = player
        .items
        .into_iter()
        .filter(|item| item.item_id > 0 && item.upgrade_id == PURCHASE_UPGRADE_ID)
        .map(|item| ItemPurchase {
            item_id: item.item_id,
            bought_at_s: item.game_time_s,
            sold_at_s: (item.sold_time_s > 0).then_some(item.sold_time_s),
        })
        .collect();
    purchases.sort_by(|left, right| {
        left.bought_at_s
            .cmp(&right.bought_at_s)
            .then(left.item_id.cmp(&right.item_id))
    });
    Ok(purchases)
}

fn fetch_stats(account: Option<u32>, filter: &StatsFilter) -> Result<Vec<ItemStat>> {
    let mut query = Vec::new();
    if let Some(account) = account {
        query.push(("account_id", account.to_string()));
    }
    query.push(("min_matches", MIN_ITEM_MATCHES.to_string()));
    query.extend(filter_query(filter));
    let json = get_text("/v1/analytics/item-stats", &query, ANALYTICS_TTL)?;
    parse_item_stats(&json)
}

/// Point every row at the matching baseline row. A row the baseline does not
/// name keeps its own rate and a zero delta.
fn apply_baseline(rows: &mut [ItemStat], baseline: &[ItemStat]) {
    let reference: HashMap<u64, &ItemStat> =
        baseline.iter().map(|row| (row.item_id, row)).collect();
    for row in rows {
        let Some(global) = reference.get(&row.item_id) else {
            continue;
        };
        row.global_win_rate_percent = global.win_rate_percent;
        row.win_rate_delta_percent = row.win_rate_percent - global.win_rate_percent;
        row.global_avg_buy_time_s = global.avg_buy_time_s;
        row.buy_time_delta_s = row.avg_buy_time_s - global.avg_buy_time_s;
    }
}

fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (part as f64 / total as f64) * 100.0
}

#[derive(Debug, Deserialize)]
struct RawItemStat {
    #[serde(default)]
    item_id: u64,
    #[serde(default)]
    wins: u64,
    #[serde(default)]
    losses: u64,
    #[serde(default)]
    matches: u64,
    #[serde(default)]
    players: u64,
    #[serde(default)]
    avg_buy_time_s: f64,
}

impl RawItemStat {
    fn into_stat(self) -> Option<ItemStat> {
        if self.item_id == 0 || self.matches == 0 {
            return None;
        }
        let wins = self.wins.min(self.matches);
        let losses = self.losses.min(self.matches);
        let win_rate_percent = percent(wins, self.matches);
        Some(ItemStat {
            item_id: self.item_id,
            matches: self.matches,
            wins,
            losses,
            players: self.players,
            win_rate_percent,
            global_win_rate_percent: win_rate_percent,
            win_rate_delta_percent: 0.0,
            avg_buy_time_s: self.avg_buy_time_s,
            global_avg_buy_time_s: self.avg_buy_time_s,
            buy_time_delta_s: 0.0,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawTimelineMetadata {
    #[serde(default)]
    match_info: RawTimelineMatchInfo,
}

#[derive(Debug, Default, Deserialize)]
struct RawTimelineMatchInfo {
    #[serde(default)]
    players: Vec<RawTimelinePlayer>,
}

#[derive(Debug, Deserialize)]
struct RawTimelinePlayer {
    #[serde(default)]
    account_id: u32,
    #[serde(default)]
    items: Vec<RawTimelineItem>,
}

#[derive(Debug, Deserialize)]
struct RawTimelineItem {
    #[serde(default)]
    item_id: u64,
    #[serde(default)]
    game_time_s: u32,
    #[serde(default)]
    sold_time_s: u32,
    #[serde(default = "purchase_upgrade_id")]
    upgrade_id: u32,
}

fn purchase_upgrade_id() -> u32 {
    PURCHASE_UPGRADE_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM_STATS: &str = r#"[
        {"item_id": 26002154, "bucket": 0, "wins": 1, "losses": 2, "matches": 3,
         "players": 1, "avg_buy_time_s": 588.0, "avg_sell_time_s": 1610.5,
         "avg_buy_time_relative": 34.99, "avg_sell_time_relative": 76.68, "sold": 2},
        {"item_id": 7409189, "bucket": 0, "wins": 184, "losses": 141, "matches": 325,
         "players": 1, "avg_buy_time_s": 564.5661538461538,
         "avg_sell_time_s": 1810.878048780488, "avg_buy_time_relative": 29.69,
         "avg_sell_time_relative": 83.33, "sold": 82},
        {"item_id": 84321454, "bucket": 0, "wins": 13, "losses": 10, "matches": 23,
         "players": 1, "avg_buy_time_s": 381.60869565217394,
         "avg_sell_time_s": 1200.0, "avg_buy_time_relative": 21.1,
         "avg_sell_time_relative": 66.0, "sold": 9}
    ]"#;

    const BASELINE: &str = r#"[
        {"item_id": 26002154, "bucket": 0, "wins": 3747359, "losses": 3611234,
         "matches": 7358593, "players": 5000, "avg_buy_time_s": 784.1487868672721},
        {"item_id": 7409189, "bucket": 0, "wins": 19678480, "losses": 19394264,
         "matches": 39072744, "players": 20000, "avg_buy_time_s": 793.4701668764293}
    ]"#;

    const METADATA: &str = r#"{
        "match_info": {
            "match_id": 42,
            "players": [
                {"account_id": 1, "items": [
                    {"game_time_s": 166, "item_id": 2922054143, "upgrade_id": 1, "sold_time_s": 1611},
                    {"game_time_s": 11, "item_id": 548943648, "upgrade_id": 0, "sold_time_s": 0},
                    {"game_time_s": 59, "item_id": 1998374645, "upgrade_id": 1, "sold_time_s": 1546},
                    {"game_time_s": 244, "item_id": 3077079169, "upgrade_id": 1, "sold_time_s": 244},
                    {"game_time_s": 55, "item_id": 539192269, "upgrade_id": 1758432787, "sold_time_s": 0}
                ]},
                {"account_id": 2, "items": [
                    {"game_time_s": 90, "item_id": 111, "upgrade_id": 1, "sold_time_s": 0}
                ]}
            ]
        }
    }"#;

    #[test]
    fn item_rows_read_rates_and_lead_with_the_most_played() {
        let stats = parse_item_stats(ITEM_STATS).expect("stats");
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].item_id, 7409189);
        assert_eq!(stats[0].matches, 325);
        assert!((stats[0].win_rate_percent - 184.0 / 325.0 * 100.0).abs() < 1e-9);
        assert_eq!(stats[0].win_rate_delta_percent, 0.0);
        assert_eq!(stats[1].item_id, 84321454);
        assert_eq!(stats[2].item_id, 26002154);
    }

    #[test]
    fn a_row_without_matches_or_an_item_id_is_dropped() {
        let json = r#"[
            {"item_id": 0, "matches": 10, "wins": 5},
            {"item_id": 7, "matches": 0, "wins": 0},
            {"item_id": 8, "matches": 4, "wins": 9, "losses": 99}
        ]"#;
        let stats = parse_item_stats(json).expect("stats");
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].wins, 4);
        assert_eq!(stats[0].losses, 4);
        assert!((stats[0].win_rate_percent - 100.0).abs() < 1e-9);
    }

    #[test]
    fn the_baseline_sets_the_delta_and_the_buy_time_gap() {
        let mut stats = parse_item_stats(ITEM_STATS).expect("stats");
        let baseline = parse_item_stats(BASELINE).expect("baseline");
        apply_baseline(&mut stats, &baseline);

        let first = &stats[0];
        assert!((first.global_win_rate_percent - 19678480.0 / 39072744.0 * 100.0).abs() < 1e-9);
        assert!(
            (first.win_rate_delta_percent
                - (first.win_rate_percent - first.global_win_rate_percent))
                .abs()
                < 1e-9
        );
        assert!((first.buy_time_delta_s - (564.5661538461538 - 793.4701668764293)).abs() < 1e-9);

        let without = stats
            .iter()
            .find(|row| row.item_id == 84321454)
            .expect("row");
        assert_eq!(without.win_rate_delta_percent, 0.0);
        assert_eq!(without.buy_time_delta_s, 0.0);
    }

    #[test]
    fn a_broken_item_response_is_an_error() {
        assert!(parse_item_stats("not json").is_err());
        assert!(parse_item_stats("{}").is_err());
    }

    #[test]
    fn the_timeline_keeps_finished_purchases_in_time_order() {
        let purchases = parse_item_timeline(METADATA, 1).expect("timeline");
        assert_eq!(purchases.len(), 3);
        assert_eq!(purchases[0].item_id, 1998374645);
        assert_eq!(purchases[0].bought_at_s, 59);
        assert_eq!(purchases[0].sold_at_s, Some(1546));
        assert_eq!(purchases[1].item_id, 2922054143);
        assert_eq!(purchases[1].sold_at_s, Some(1611));
        assert_eq!(purchases[2].item_id, 3077079169);
        assert_eq!(purchases[2].sold_at_s, Some(244));
    }

    #[test]
    fn a_held_item_has_no_sale_time() {
        let json = r#"{"match_info": {"players": [
            {"account_id": 3, "items": [
                {"game_time_s": 10, "item_id": 5, "upgrade_id": 1, "sold_time_s": 0},
                {"game_time_s": 20, "item_id": 6, "upgrade_id": 1}
            ]}
        ]}}"#;
        let purchases = parse_item_timeline(json, 3).expect("timeline");
        assert_eq!(purchases.len(), 2);
        assert_eq!(purchases[0].sold_at_s, None);
        assert_eq!(purchases[1].sold_at_s, None);
    }

    #[test]
    fn a_player_the_match_does_not_name_is_an_error() {
        assert!(parse_item_timeline(METADATA, 99).is_err());
        assert!(parse_item_timeline("not json", 1).is_err());
    }

    #[test]
    fn best_items_need_a_sample_and_lead_with_the_win_rate() {
        let row = |id: u64, matches: u64, rate: f64| ItemStat {
            item_id: id,
            matches,
            wins: 0,
            losses: 0,
            players: 0,
            win_rate_percent: rate,
            global_win_rate_percent: rate,
            win_rate_delta_percent: 0.0,
            avg_buy_time_s: 0.0,
            global_avg_buy_time_s: 0.0,
            buy_time_delta_s: 0.0,
        };
        let best = best_items(vec![row(1, 600, 51.0), row(2, 10, 90.0), row(3, 900, 55.0)], 500);
        assert_eq!(best.iter().map(|r| r.item_id).collect::<Vec<_>>(), [3, 1]);
    }
}
