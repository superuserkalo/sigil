//! Which lineup a party should queue, from the heroes each player actually
//! picks.
//!
//! Sources:
//!
//! - `GET /v1/players/hero-stats?account_ids=...` for each member's per-hero
//!   matches inside the chosen window and mode. The three most played heroes
//!   are the picks a member offers, the way the game's hero draft works.
//! - `GET /v1/analytics/hero-comb-stats?comb_size=2` and `comb_size=3` for every
//!   pair's and trio's global record in the same window and mode.
//!
//! The API has no party endpoint and no set larger than a trio in one response:
//! sets of four already run past 8 MB. So a pair or trio is rated exactly, and
//! a larger lineup is estimated from the trios inside it, with every trio
//! counted by the games it has. The game does not say how it picks among a
//! player's offered heroes, so every lineup the picks allow counts the same.
//!
//! Lineups below [`LOW_SAMPLE_MATCHES`] are still returned: a rare lineup is
//! one the party can still get, so the screen mutes it rather than hiding it.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::stats::cache;
use crate::stats::filters::{ANALYTICS_TTL, StatsFilter, ensure_account, filter_query, get_text};

/// Deadlock queues parties of up to six, a full team.
pub const MAX_PARTY_SIZE: usize = 6;
/// Each player offers up to three heroes in queue; the game gives them one.
pub const MAX_PICKS: usize = 3;
/// Below this many matches a lineup's win rate is mostly noise. The screen
/// mutes it instead of hiding it.
pub const LOW_SAMPLE_MATCHES: u64 = 200;

/// One set of heroes and its record, from `/v1/analytics/hero-comb-stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeroComb {
    /// Sorted, so two orders of the same set share a key.
    pub hero_ids: Vec<u32>,
    pub wins: u64,
    pub matches: u64,
}

/// One hero row for one account, from `/v1/players/hero-stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeroRow {
    pub account_id: u32,
    pub hero_id: u32,
    pub matches_played: u64,
}

/// One lineup the party's picks allow, rated against everyone's record.
#[derive(Debug, Clone, PartialEq)]
pub struct LineupRating {
    /// The party member behind each hero, in party order.
    pub account_ids: Vec<u32>,
    /// One hero per account, in the same order as `account_ids`.
    pub hero_ids: Vec<u32>,
    /// Percent from 0 to 100, or `None` when no pair or trio record covers the
    /// lineup.
    pub win_rate_percent: Option<f64>,
    /// Matches as exactly this lineup. `None` for a lineup larger than a trio,
    /// whose rate is estimated from the trios inside it.
    pub matches: Option<u64>,
    /// Games behind the rate: the exact count, or the trios' total.
    pub sample_matches: u64,
    /// True when the rate is estimated from smaller sets rather than read from
    /// one record.
    pub estimated: bool,
}

impl LineupRating {
    pub fn is_rated(&self) -> bool {
        self.win_rate_percent.is_some()
    }

    /// Below [`LOW_SAMPLE_MATCHES`] the rate rests on too few games to lead a
    /// verdict. An unrated lineup has no sample, so it reads as low too.
    pub fn is_low_sample(&self) -> bool {
        self.sample_matches < LOW_SAMPLE_MATCHES
    }
}

/// Every lineup the party's picks allow, best first, with unrated lineups last.
///
/// `party` holds SteamID3 account ids. Duplicates are ignored. A party of one
/// player cannot form a lineup, so it returns an empty list. Members with no
/// matches in the chosen window and mode are left out, because their picks are
/// unknown in that scope.
pub fn lineup_ratings(party: &[u32], filter: &StatsFilter) -> Result<Vec<LineupRating>> {
    let accounts = party_accounts(party)?;
    if accounts.len() < 2 {
        return Ok(Vec::new());
    }

    let members = party_picks(&accounts, filter)?;
    rate_members(&members, filter)
}

/// Rate hand-picked lineups: `member_picks[i]` is the heroes member `i` would play (up to any
/// count; repeats ignored). Every lineup with one pick per member and no hero twice is rated
/// from the same hero combination records as [`lineup_ratings`], best first. The lineups'
/// `account_ids` are member numbers starting at 1, since no account is involved. Fewer than two
/// members with picks returns an empty list.
pub fn rate_lineups(member_picks: &[Vec<u32>], filter: &StatsFilter) -> Result<Vec<LineupRating>> {
    let members = members_from_picks(member_picks);
    rate_members(&members, filter)
}

/// `(member number, picks)` for [`rate_lineups`]: zero hero ids and repeats dropped, members
/// without picks left out.
pub fn members_from_picks(member_picks: &[Vec<u32>]) -> Vec<(u32, Vec<u32>)> {
    member_picks
        .iter()
        .enumerate()
        .filter_map(|(index, picks)| {
            let mut heroes: Vec<u32> = Vec::new();
            for &hero in picks {
                if hero != 0 && !heroes.contains(&hero) {
                    heroes.push(hero);
                }
            }
            (!heroes.is_empty()).then(|| (index as u32 + 1, heroes))
        })
        .collect()
}

fn rate_members(members: &[(u32, Vec<u32>)], filter: &StatsFilter) -> Result<Vec<LineupRating>> {
    if members.len() < 2 {
        return Ok(Vec::new());
    }

    // A pair is the exact record only when the party is two players. Three or
    // more players form trios or larger sets, and larger sets are estimated
    // from trios.
    let pairs = if members.len() == 2 {
        index_combs(&parse_hero_combs(&fetch_combs(2, filter)?)?)
    } else {
        CombIndex::new()
    };
    let trios = if members.len() >= 3 {
        index_combs(&parse_hero_combs(&fetch_combs(3, filter)?)?)
    } else {
        CombIndex::new()
    };

    Ok(sorted(rate_party(members, &pairs, &trios)))
}

/// [`rate_lineups`] without the network: the pair and trio records are given. For tests and
/// for a caller that already holds the combination rows.
pub fn rate_lineups_with(member_picks: &[Vec<u32>], pairs: &[HeroComb], trios: &[HeroComb]) -> Vec<LineupRating> {
    let members = members_from_picks(member_picks);
    if members.len() < 2 {
        return Vec::new();
    }
    sorted(rate_party(&members, &index_combs(pairs), &index_combs(trios)))
}

/// Best first, unrated last.
fn sorted(mut ratings: Vec<LineupRating>) -> Vec<LineupRating> {
    ratings.sort_by(|left, right| {
        right
            .win_rate_percent
            .unwrap_or(-1.0)
            .total_cmp(&left.win_rate_percent.unwrap_or(-1.0))
            .then(right.sample_matches.cmp(&left.sample_matches))
            .then(left.hero_ids.cmp(&right.hero_ids))
    });
    ratings
}

/// The party's expected win rate: the mean over the rated lineups, each
/// counted the same. `None` when no lineup has data.
pub fn expected_win_rate(ratings: &[LineupRating]) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0usize;
    for rate in ratings.iter().filter_map(|rating| rating.win_rate_percent) {
        total += rate;
        count += 1;
    }
    (count > 0).then(|| total / count as f64)
}

/// Parse one `/v1/analytics/hero-comb-stats` response.
///
/// Hero ids are sorted. Rows with fewer than two heroes, a zero hero id, a
/// repeated hero or no matches are dropped. Wins are clamped to matches.
pub fn parse_hero_combs(json: &str) -> Result<Vec<HeroComb>> {
    let rows: Vec<RawHeroComb> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Hero combination stats did not parse: {error}")))?;
    Ok(rows
        .into_iter()
        .filter_map(RawHeroComb::into_comb)
        .collect())
}

/// Parse one `/v1/players/hero-stats` response.
///
/// Rows with a zero account, a zero hero or no matches are dropped. The order
/// is the API's; [`lineup_ratings`] sorts by matches when it picks.
pub fn parse_hero_rows(json: &str) -> Result<Vec<HeroRow>> {
    let rows: Vec<RawHeroRow> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Player hero stats did not parse: {error}")))?;
    Ok(rows.into_iter().filter_map(RawHeroRow::into_row).collect())
}

fn party_accounts(party: &[u32]) -> Result<Vec<u32>> {
    if party.is_empty() {
        return Err(Error::msg(
            "A party needs at least one account id. Leaderboard entries and the Steam profile \
             search show the account id.",
        ));
    }
    let mut accounts = Vec::with_capacity(party.len());
    for &account in party {
        ensure_account(account)?;
        if !accounts.contains(&account) {
            accounts.push(account);
        }
    }
    if accounts.len() > MAX_PARTY_SIZE {
        return Err(Error::msg(format!(
            "A Deadlock party queues at most {MAX_PARTY_SIZE} players; {} were given.",
            accounts.len()
        )));
    }
    Ok(accounts)
}

/// Each member's most played heroes in the chosen scope, in party order.
fn party_picks(accounts: &[u32], filter: &StatsFilter) -> Result<Vec<(u32, Vec<u32>)>> {
    let ids = accounts
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut query = vec![("account_ids", ids)];
    query.extend(filter_query(filter));
    let json = get_text("/v1/players/hero-stats", &query, cache::TTL_HERO_STATS)?;
    Ok(top_picks(accounts, &parse_hero_rows(&json)?, MAX_PICKS))
}

/// The top `limit` heroes by matches for each account, dropping accounts with
/// no rows and repeated hero ids. Party order is preserved.
fn top_picks(accounts: &[u32], rows: &[HeroRow], limit: usize) -> Vec<(u32, Vec<u32>)> {
    let mut by_account: HashMap<u32, Vec<&HeroRow>> = HashMap::new();
    for row in rows {
        if !accounts.contains(&row.account_id) {
            continue;
        }
        by_account.entry(row.account_id).or_default().push(row);
    }

    let mut picks = Vec::new();
    for &account in accounts {
        let Some(mut rows) = by_account.remove(&account) else {
            continue;
        };
        rows.sort_by(|left, right| {
            right
                .matches_played
                .cmp(&left.matches_played)
                .then(left.hero_id.cmp(&right.hero_id))
        });
        let mut heroes: Vec<u32> = Vec::with_capacity(limit);
        for row in rows {
            if heroes.len() == limit {
                break;
            }
            if !heroes.contains(&row.hero_id) {
                heroes.push(row.hero_id);
            }
        }
        if !heroes.is_empty() {
            picks.push((account, heroes));
        }
    }
    picks
}

/// Every lineup the game can hand the party: one pick per player, never the
/// same hero twice. Account ids and hero ids stay in step.
fn party_lineups(members: &[(u32, Vec<u32>)]) -> Vec<(Vec<u32>, Vec<u32>)> {
    let mut lineups: Vec<(Vec<u32>, Vec<u32>)> = vec![(Vec::new(), Vec::new())];
    for (account, picks) in members {
        let mut next = Vec::new();
        for (accounts, heroes) in &lineups {
            for &hero in picks {
                if heroes.contains(&hero) {
                    continue;
                }
                let mut accounts = accounts.clone();
                accounts.push(*account);
                let mut heroes = heroes.clone();
                heroes.push(hero);
                next.push((accounts, heroes));
            }
        }
        lineups = next;
    }
    lineups
}

fn rate_party(
    members: &[(u32, Vec<u32>)],
    pairs: &CombIndex,
    trios: &CombIndex,
) -> Vec<LineupRating> {
    party_lineups(members)
        .into_iter()
        .map(
            |(account_ids, hero_ids)| match lineup_stat(&hero_ids, pairs, trios) {
                Some(stat) => LineupRating {
                    account_ids,
                    hero_ids,
                    win_rate_percent: Some(stat.win_rate_percent),
                    matches: stat.matches,
                    sample_matches: stat.sample,
                    estimated: stat.estimated,
                },
                None => LineupRating {
                    account_ids,
                    hero_ids,
                    win_rate_percent: None,
                    matches: None,
                    sample_matches: 0,
                    estimated: false,
                },
            },
        )
        .collect()
}

/// One lineup's rate: exact for a pair or trio, summed from the trios inside a
/// larger lineup, `None` when no record covers it.
fn lineup_stat(hero_ids: &[u32], pairs: &CombIndex, trios: &CombIndex) -> Option<LineupStat> {
    if hero_ids.len() < 2 {
        return None;
    }
    if hero_ids.len() <= 3 {
        let index = if hero_ids.len() == 2 { pairs } else { trios };
        let record = index.get(&comb_key(hero_ids))?;
        return (record.matches > 0).then(|| LineupStat {
            win_rate_percent: percent(record.wins, record.matches),
            matches: Some(record.matches),
            sample: record.matches,
            estimated: false,
        });
    }

    let mut wins = 0u64;
    let mut matches = 0u64;
    for trio in subsets_of_three(hero_ids) {
        if let Some(record) = trios.get(&trio) {
            wins = wins.saturating_add(record.wins);
            matches = matches.saturating_add(record.matches);
        }
    }
    (matches > 0).then(|| LineupStat {
        win_rate_percent: percent(wins, matches),
        matches: None,
        sample: matches,
        estimated: true,
    })
}

fn subsets_of_three(hero_ids: &[u32]) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for first in 0..hero_ids.len() {
        for second in (first + 1)..hero_ids.len() {
            for third in (second + 1)..hero_ids.len() {
                out.push(comb_key(&[
                    hero_ids[first],
                    hero_ids[second],
                    hero_ids[third],
                ]));
            }
        }
    }
    out
}

type CombIndex = HashMap<Vec<u32>, CombRecord>;

#[derive(Debug, Clone, Copy, Default)]
struct CombRecord {
    wins: u64,
    matches: u64,
}

fn index_combs(combs: &[HeroComb]) -> CombIndex {
    let mut index = CombIndex::new();
    for comb in combs {
        let entry = index.entry(comb_key(&comb.hero_ids)).or_default();
        entry.wins = entry.wins.saturating_add(comb.wins);
        entry.matches = entry.matches.saturating_add(comb.matches);
    }
    index
}

/// The sorted, de-duplicated hero ids that key a combination.
fn comb_key(hero_ids: &[u32]) -> Vec<u32> {
    let mut key = hero_ids.to_vec();
    key.sort_unstable();
    key.dedup();
    key
}

fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (part as f64 / total as f64) * 100.0
}

fn fetch_combs(size: u32, filter: &StatsFilter) -> Result<String> {
    // `min_matches` stays at the API default of 20: a rarer pair or trio is not
    // in the response, and its lineup reads as unrated instead of quoting one
    // game.
    let mut query = vec![("comb_size", size.to_string())];
    query.extend(filter_query(filter));
    get_text("/v1/analytics/hero-comb-stats", &query, ANALYTICS_TTL)
}

#[derive(Debug, Clone, Copy)]
struct LineupStat {
    win_rate_percent: f64,
    matches: Option<u64>,
    sample: u64,
    estimated: bool,
}

#[derive(Debug, Deserialize)]
struct RawHeroComb {
    #[serde(default)]
    hero_ids: Vec<u32>,
    #[serde(default)]
    wins: u64,
    #[serde(default)]
    matches: u64,
}

impl RawHeroComb {
    fn into_comb(self) -> Option<HeroComb> {
        if self.hero_ids.len() < 2 || self.matches == 0 {
            return None;
        }
        if self.hero_ids.contains(&0) {
            return None;
        }
        let hero_ids = comb_key(&self.hero_ids);
        // A set names each hero once. A repeated id is a malformed row, not a
        // smaller set.
        if hero_ids.len() != self.hero_ids.len() {
            return None;
        }
        Some(HeroComb {
            hero_ids,
            wins: self.wins.min(self.matches),
            matches: self.matches,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawHeroRow {
    #[serde(default)]
    account_id: u32,
    #[serde(default)]
    hero_id: u32,
    #[serde(default)]
    matches_played: u64,
}

impl RawHeroRow {
    fn into_row(self) -> Option<HeroRow> {
        if self.account_id == 0 || self.hero_id == 0 || self.matches_played == 0 {
            return None;
        }
        Some(HeroRow {
            account_id: self.account_id,
            hero_id: self.hero_id,
            matches_played: self.matches_played,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERO_ROWS: &str = r#"[
        {"account_id": 132240836, "hero_id": 1, "matches_played": 169, "wins": 109},
        {"account_id": 132240836, "hero_id": 2, "matches_played": 492, "wins": 262},
        {"account_id": 132240836, "hero_id": 4, "matches_played": 9, "wins": 2},
        {"account_id": 177502895, "hero_id": 2, "matches_played": 30, "wins": 12},
        {"account_id": 0, "hero_id": 5, "matches_played": 10, "wins": 5},
        {"account_id": 177502895, "hero_id": 0, "matches_played": 10, "wins": 5}
    ]"#;

    const COMBS: &str = r#"[
        {"hero_ids": [66, 76, 84], "wins": 64544, "losses": 28582, "matches": 93126},
        {"hero_ids": [9, 7], "wins": 10, "losses": 0, "matches": 0},
        {"hero_ids": [5, 5, 6], "wins": 1, "losses": 1, "matches": 2},
        {"hero_ids": [3, 0], "wins": 1, "losses": 1, "matches": 2},
        {"hero_ids": [2, 1], "wins": 3, "losses": 1, "matches": 4}
    ]"#;

    #[test]
    fn hero_rows_need_a_real_account_a_real_hero_and_matches() {
        let rows = parse_hero_rows(HERO_ROWS).expect("rows");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].account_id, 132240836);
        assert_eq!(rows[0].hero_id, 1);
        assert_eq!(rows[0].matches_played, 169);
        assert!(parse_hero_rows("not json").is_err());
    }

    #[test]
    fn picks_are_the_three_most_played_heroes_per_member_in_party_order() {
        let rows = parse_hero_rows(HERO_ROWS).expect("rows");
        let picks = top_picks(&[177502895, 132240836], &rows, MAX_PICKS);
        assert_eq!(
            picks,
            vec![(177502895, vec![2]), (132240836, vec![2, 1, 4]),]
        );
    }

    #[test]
    fn a_member_without_rows_is_left_out() {
        let rows = parse_hero_rows(HERO_ROWS).expect("rows");
        let picks = top_picks(&[132240836, 999], &rows, MAX_PICKS);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].0, 132240836);
    }

    #[test]
    fn lineups_take_one_pick_per_member_and_never_repeat_a_hero() {
        let members = vec![(10, vec![5, 2]), (11, vec![3, 5])];
        let lineups = party_lineups(&members);
        assert_eq!(
            lineups,
            vec![
                (vec![10, 11], vec![5, 3]),
                (vec![10, 11], vec![2, 3]),
                (vec![10, 11], vec![2, 5]),
            ]
        );
    }

    #[test]
    fn a_single_hero_is_not_a_lineup() {
        assert!(lineup_stat(&[1], &CombIndex::new(), &CombIndex::new()).is_none());
    }

    #[test]
    fn a_pair_and_a_trio_are_rated_exactly() {
        let pairs = index_combs(&[HeroComb {
            hero_ids: vec![4, 3],
            wins: 7,
            matches: 10,
        }]);
        let pair = lineup_stat(&[3, 4], &pairs, &CombIndex::new()).expect("pair");
        assert!((pair.win_rate_percent - 70.0).abs() < 1e-9);
        assert_eq!(pair.matches, Some(10));
        assert_eq!(pair.sample, 10);
        assert!(!pair.estimated);

        let trios = index_combs(&[HeroComb {
            hero_ids: vec![1, 2, 3],
            wins: 60,
            matches: 100,
        }]);
        let trio = lineup_stat(&[3, 1, 2], &CombIndex::new(), &trios).expect("trio");
        assert!((trio.win_rate_percent - 60.0).abs() < 1e-9);
        assert_eq!(trio.matches, Some(100));
    }

    #[test]
    fn a_larger_lineup_is_estimated_from_its_trios() {
        let trios = index_combs(&[
            HeroComb {
                hero_ids: vec![1, 2, 3],
                wins: 60,
                matches: 100,
            },
            HeroComb {
                hero_ids: vec![1, 2, 4],
                wins: 40,
                matches: 100,
            },
            HeroComb {
                hero_ids: vec![1, 3, 4],
                wins: 50,
                matches: 100,
            },
            HeroComb {
                hero_ids: vec![2, 3, 4],
                wins: 30,
                matches: 100,
            },
        ]);
        let stat = lineup_stat(&[1, 2, 3, 4], &CombIndex::new(), &trios).expect("estimate");
        assert!((stat.win_rate_percent - 45.0).abs() < 1e-9);
        assert_eq!(stat.matches, None);
        assert_eq!(stat.sample, 400);
        assert!(stat.estimated);
    }

    #[test]
    fn a_lineup_without_a_record_is_unrated() {
        assert!(lineup_stat(&[1, 2], &CombIndex::new(), &CombIndex::new()).is_none());
    }

    #[test]
    fn combination_rows_are_sorted_and_bad_rows_dropped() {
        let combs = parse_hero_combs(COMBS).expect("combs");
        assert_eq!(combs.len(), 2);
        assert_eq!(combs[0].hero_ids, vec![66, 76, 84]);
        assert_eq!(combs[0].wins, 64544);
        assert_eq!(combs[1].hero_ids, vec![1, 2]);
        assert!(parse_hero_combs("[]").expect("empty").is_empty());
        assert!(parse_hero_combs("{}").is_err());
    }

    #[test]
    fn the_expected_rate_averages_the_rated_lineups_only() {
        let ratings = vec![
            LineupRating {
                account_ids: vec![1, 2],
                hero_ids: vec![1, 2],
                win_rate_percent: Some(60.0),
                matches: Some(100),
                sample_matches: 100,
                estimated: false,
            },
            LineupRating {
                account_ids: vec![1, 2],
                hero_ids: vec![3, 4],
                win_rate_percent: None,
                matches: None,
                sample_matches: 0,
                estimated: false,
            },
            LineupRating {
                account_ids: vec![1, 2],
                hero_ids: vec![5, 6],
                win_rate_percent: Some(40.0),
                matches: Some(300),
                sample_matches: 300,
                estimated: false,
            },
        ];
        assert_eq!(expected_win_rate(&ratings), Some(50.0));
        assert_eq!(expected_win_rate(&ratings[1..2]), None);
        assert!(!ratings[1].is_rated());
        assert!(ratings[1].is_low_sample());
        assert!(!ratings[2].is_low_sample());
    }

    #[test]
    fn a_party_is_validated_before_any_request() {
        assert!(lineup_ratings(&[], &StatsFilter::default()).is_err());
        assert!(lineup_ratings(&[0], &StatsFilter::default()).is_err());
        assert!(
            lineup_ratings(&[1], &StatsFilter::default())
                .expect("solo")
                .is_empty()
        );
        assert!(
            lineup_ratings(&[1, 1], &StatsFilter::default())
                .expect("dup")
                .is_empty()
        );
        assert!(lineup_ratings(&[1, 2, 3, 4, 5, 6, 7], &StatsFilter::default()).is_err());
    }

    #[test]
    fn hand_picked_lineups_are_rated_without_accounts() {
        let pairs = [HeroComb { hero_ids: vec![1, 2], wins: 6, matches: 10 }, HeroComb { hero_ids: vec![1, 3], wins: 3, matches: 10 }];
        let ratings = rate_lineups_with(&[vec![1, 1, 0], vec![2, 3, 1]], &pairs, &[]);
        // Member 1 plays 1; member 2 plays 2 or 3 (1 is taken).
        assert_eq!(ratings.len(), 2);
        assert_eq!(ratings[0].hero_ids, vec![1, 2]);
        assert_eq!(ratings[0].account_ids, vec![1, 2]);
        assert!((ratings[0].win_rate_percent.unwrap() - 60.0).abs() < 1e-9);
        assert_eq!(members_from_picks(&[vec![], vec![5]]), vec![(2, vec![5])]);
        assert!(rate_lineups_with(&[vec![1]], &pairs, &[]).is_empty());
    }
}
