//! Who a player keeps meeting: the names on the scoreboard that repeat.
//!
//! Source: `GET /v1/players/{account_id}/mate-stats?min_matches_played=3`,
//! the same with `&same_party=true` for the people they queue with, and
//! `GET /v1/players/{account_id}/enemy-stats?min_matches_played=3`, then one
//! `GET /v1/players/steam?account_ids=` call for the names of the rows shown.
//!
//! `wins` is the player's own wins in the shared games, so a mate's row reads
//! "with them" and an opponent's row reads "against them". Both samples count
//! only matches both players were in; the API's `min_matches_played=3` keeps a
//! single game from becoming a verdict. Each response is about 700 KB, so the
//! screen asks for it once and caches it for an hour.
//!
//! Lanes: each mate-stats row also lists the shared match ids. Up to
//! [`LANE_MATCH_IDS`] of the most recent ones go to one bulk
//! `GET /v1/matches/metadata?match_ids=...&include_player_info=true` request,
//! whose players carry `team` and `assigned_lane` (1, 4 or 6). A shared match
//! counts as "same lane" when both players had the same team and lane; it is a
//! win when that team is `winning_team`. Matches the API has not stored are
//! simply missing, so the lane sample can be smaller than the shared one.
//! A rate is shown only from [`MIN_RATE_SAMPLE`] matches up ([`rate`]).

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Players this many shared matches in, at least, before they are shown.
pub const MIN_SHARED_MATCHES: u64 = 3;

/// One player met in a shared match.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRecord {
    pub account_id: u64,
    pub persona_name: Option<String>,
    pub matches: u64,
    pub wins: u64,
    /// The player's win rate in those shared games.
    pub win_rate_percent: f64,
    /// True when at least one of the shared games was queued in the same party.
    pub premade: bool,
    /// The shared matches that were looked up for lanes, and the ones in the same lane.
    /// `None` when no lane data was read for this player.
    pub lane: Option<LaneRecord>,
    /// The shared match ids the API listed for this row (mate-stats and enemy-stats `matches`).
    pub match_ids: Vec<u64>,
    /// The Steam avatar URL (`avatarmedium`) from the name request, when it answered.
    pub avatar: Option<String>,
}

/// Lane facts for one teammate, over the shared matches whose lanes were read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaneRecord {
    /// Shared matches whose lanes the API returned.
    pub checked: u64,
    /// Of those, matches where both had the same lane.
    pub matches: u64,
    /// Of those, the ones the player won.
    pub wins: u64,
}

/// Matches a rate needs before it is shown: below this a rate reads "-", so one win in
/// one game is never "100%".
pub const MIN_RATE_SAMPLE: u64 = 5;

/// A win rate in percent from `wins` of `matches`, or `None` below [`MIN_RATE_SAMPLE`].
pub fn rate(wins: u64, matches: u64) -> Option<f64> {
    (matches >= MIN_RATE_SAMPLE).then(|| 100.0 * wins.min(matches) as f64 / matches as f64)
}

/// Match ids sent in the one bulk lane request (the API takes up to 1000).
pub const LANE_MATCH_IDS: usize = 400;
/// The most recent shared matches looked up per teammate.
pub const LANE_MATCHES_PER_MATE: usize = 60;

impl PlayerRecord {
    fn new(account_id: u64, matches: u64, wins: u64) -> Self {
        PlayerRecord {
            account_id,
            persona_name: None,
            matches,
            wins,
            win_rate_percent: if matches == 0 {
                0.0
            } else {
                100.0 * wins as f64 / matches as f64
            },
            premade: false,
            lane: None,
            match_ids: Vec::new(),
            avatar: None,
        }
    }

    /// The win rate together, or `None` below the sample floor.
    pub fn rate_together(&self) -> Option<f64> {
        rate(self.wins, self.matches)
    }

    /// The win rate in the same lane, or `None` without lane data or below the floor.
    pub fn rate_in_lane(&self) -> Option<f64> {
        self.lane.and_then(|lane| rate(lane.wins, lane.matches))
    }
}

/// A teammate against the rest of the subject's matches, over the loaded match history.
///
/// "With" counts the scored history matches whose id is in the row's shared list, "without"
/// every other scored history match. The history is a window (the newest few hundred), so the
/// split is only trusted when at least half of the row's shared matches fall inside it
/// ([`WithWithout::covered`]); older shared games often have no valid outcome.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WithWithout {
    pub with_matches: u64,
    pub with_wins: u64,
    pub without_matches: u64,
    pub without_wins: u64,
    /// Newest `start_time` of a shared match found in the history.
    pub last_together: Option<i64>,
    /// At least half the row's shared matches are in the history window.
    pub covered: bool,
}

impl WithWithout {
    /// Win rate in the shared matches inside the window, from [`MIN_RATE_SAMPLE`] up.
    pub fn with_rate(&self) -> Option<f64> {
        rate(self.with_wins, self.with_matches)
    }

    /// Win rate in the other matches of the window, from [`MIN_RATE_SAMPLE`] up, and only
    /// when the window covers the row ([`WithWithout::covered`]).
    pub fn without_rate(&self) -> Option<f64> {
        if !self.covered {
            return None;
        }
        rate(self.without_wins, self.without_matches)
    }

    /// With minus without, in points, when both sides have [`MIN_RATE_SAMPLE`] matches and
    /// the window covers the row.
    pub fn lift(&self) -> Option<f64> {
        Some(self.with_rate()? - self.without_rate()?)
    }
}

/// Join one row's shared match ids with the subject's match history.
pub fn with_without(record: &PlayerRecord, history: &[super::MatchSummary]) -> WithWithout {
    let shared: std::collections::HashSet<u64> = record.match_ids.iter().copied().collect();
    let mut out = WithWithout::default();
    let mut found = 0u64;
    for item in history {
        let together = shared.contains(&item.match_id);
        if together {
            found += 1;
            out.last_together = Some(out.last_together.map_or(item.start_time, |at| at.max(item.start_time)));
        }
        let Some(win) = item.win else { continue };
        if together {
            out.with_matches += 1;
            out.with_wins += u64::from(win);
        } else {
            out.without_matches += 1;
            out.without_wins += u64::from(win);
        }
    }
    out.covered = !shared.is_empty() && found * 2 >= shared.len() as u64;
    out
}

/// The people a player meets, named.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Social {
    /// Teammates, most shared matches first.
    pub mates: Vec<PlayerRecord>,
    /// Opponents, most shared matches first.
    pub enemies: Vec<PlayerRecord>,
    /// Lane data was read for the teammates (the bulk match request answered).
    pub lanes_known: bool,
}

/// The overview's duo line: the teammate with the best lane win rate when lane data exists,
/// else the best win rate together. Both need [`MIN_RATE_SAMPLE`] matches.
#[derive(Debug, Clone, PartialEq)]
pub struct BestPartner {
    pub account_id: u64,
    pub persona_name: Option<String>,
    /// True: the rate and sample are for the same lane. False: for every match together.
    pub in_lane: bool,
    pub rate_percent: f64,
    pub wins: u64,
    pub matches: u64,
}

impl Social {
    /// Mate rows queued with the player at least once.
    pub fn premades(&self) -> impl Iterator<Item = &PlayerRecord> {
        self.mates.iter().filter(|row| row.premade)
    }

    /// The best teammate by win rate, needing [`BEST_MATE_MATCHES`] games.
    pub fn best_mate(&self) -> Option<&PlayerRecord> {
        self.mates
            .iter()
            .filter(|row| row.matches >= BEST_MATE_MATCHES)
            .max_by(|left, right| {
                left.win_rate_percent
                    .total_cmp(&right.win_rate_percent)
                    .then(left.matches.cmp(&right.matches))
            })
    }

    /// The best lane partner, or the best duo when no teammate has enough lane matches.
    pub fn best_partner(&self) -> Option<BestPartner> {
        let pick = |in_lane: bool| {
            self.mates
                .iter()
                .filter_map(|row| {
                    let (wins, matches) = if in_lane {
                        let lane = row.lane?;
                        (lane.wins, lane.matches)
                    } else {
                        (row.wins, row.matches)
                    };
                    let rate_percent = rate(wins, matches)?;
                    Some(BestPartner {
                        account_id: row.account_id,
                        persona_name: row.persona_name.clone(),
                        in_lane,
                        rate_percent,
                        wins,
                        matches,
                    })
                })
                .max_by(|left, right| {
                    left.rate_percent
                        .total_cmp(&right.rate_percent)
                        .then(left.matches.cmp(&right.matches))
                        .then(right.account_id.cmp(&left.account_id))
                })
        };
        if self.lanes_known
            && let Some(best) = pick(true)
        {
            return Some(best);
        }
        pick(false)
    }

    /// The opponent met most often.
    pub fn most_faced(&self) -> Option<&PlayerRecord> {
        self.enemies.iter().max_by_key(|row| row.matches)
    }
}

/// Shared games a "best teammate" claim needs.
pub const BEST_MATE_MATCHES: u64 = 5;

/// Build the lists from the three responses and the name map.
pub(crate) fn assemble_social(
    mates_json: &str,
    party_json: &str,
    enemies_json: &str,
    names: &HashMap<u64, String>,
) -> Result<Social> {
    let mates = parse_rows(mates_json, "mate-stats")?;
    let party = parse_rows(party_json, "mate-stats same party")?;
    let enemies = parse_rows(enemies_json, "enemy-stats")?;
    Ok(merge(mates, party, enemies, names))
}

/// A bundle captured from the API in one file, for an offline fixture.
#[derive(Debug, Deserialize)]
struct SocialBundle {
    #[serde(default)]
    mates: Vec<RawCount>,
    #[serde(default, alias = "mates_party")]
    party: Vec<RawCount>,
    #[serde(default)]
    enemies: Vec<RawCount>,
    #[serde(default)]
    steam: Vec<RawNameRow>,
    /// The player the bundle is for, and the bulk match lanes response, when captured.
    #[serde(default)]
    account_id: u64,
    #[serde(default)]
    lanes: Option<serde_json::Value>,
}

/// A social bundle through the same assembly as a live fetch.
pub fn social_from_bundle(json: &str) -> Result<Social> {
    let bundle: SocialBundle = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Social bundle did not parse: {error}")))?;
    let names: HashMap<u64, String> = bundle
        .steam
        .into_iter()
        .map(|row| (row.account_id, row.personaname))
        .collect();
    let lanes = bundle.lanes.as_ref().map(|value| parse_match_lanes(&value.to_string())).transpose()?;
    let rows = bundle.mates;
    let mut social = merge(clone_rows(&rows), bundle.party, bundle.enemies, &names);
    if let Some(lanes) = lanes {
        apply_lane_rows(&mut social, bundle.account_id, &rows, &lanes);
    }
    Ok(social)
}

fn clone_rows(rows: &[RawCount]) -> Vec<RawCount> {
    rows.iter()
        .map(|row| RawCount {
            mate_id: row.mate_id,
            enemy_id: row.enemy_id,
            wins: row.wins,
            matches_played: row.matches_played,
            matches: row.matches.clone(),
        })
        .collect()
}

/// The account ids worth naming: the most played rows of both lists, capped at
/// `limit` so one profile request covers them.
pub(crate) fn top_accounts(
    mates_json: &str,
    party_json: &str,
    enemies_json: &str,
    limit: usize,
) -> Vec<u64> {
    let mut ids: Vec<(u64, u64)> = Vec::new();
    for (json, what) in [
        (mates_json, "mate-stats"),
        (party_json, "mate-stats same party"),
        (enemies_json, "enemy-stats"),
    ] {
        let Ok(rows) = parse_rows(json, what) else {
            continue;
        };
        ids.extend(
            rows.into_iter()
                .filter(|row| row.matches_played >= MIN_SHARED_MATCHES)
                .map(|row| (row.id(), row.matches_played)),
        );
    }
    ids.sort_by_key(|row| std::cmp::Reverse(row.1));
    let mut seen = std::collections::HashSet::new();
    ids.into_iter()
        .filter(|(id, _)| *id > 0 && seen.insert(*id))
        .map(|(id, _)| id)
        .take(limit)
        .collect()
}

fn parse_rows(json: &str, what: &str) -> Result<Vec<RawCount>> {
    serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The {what} response did not parse: {error}")))
}

fn merge(
    mates: Vec<RawCount>,
    party: Vec<RawCount>,
    enemies: Vec<RawCount>,
    names: &HashMap<u64, String>,
) -> Social {
    let premade: std::collections::HashSet<u64> = party
        .iter()
        .filter(|row| row.matches_played >= MIN_SHARED_MATCHES)
        .map(|row| row.id())
        .collect();

    let named = |rows: Vec<RawCount>| -> Vec<PlayerRecord> {
        let mut out: Vec<PlayerRecord> = rows
            .into_iter()
            .filter(|row| row.matches_played >= MIN_SHARED_MATCHES)
            .map(|row| {
                let account_id = row.id();
                let mut record = PlayerRecord::new(account_id, row.matches_played, row.wins.min(row.matches_played));
                record.persona_name = names.get(&account_id).cloned();
                record.premade = premade.contains(&account_id);
                record.match_ids = row.matches;
                record
            })
            .collect();
        out.sort_by(|left, right| {
            right
                .matches
                .cmp(&left.matches)
                .then(right.wins.cmp(&left.wins))
                .then(left.account_id.cmp(&right.account_id))
        });
        out
    };

    Social {
        mates: named(mates),
        enemies: named(enemies),
        lanes_known: false,
    }
}

/// The shared match ids worth a lane lookup: the most played teammates first, each with its
/// [`LANE_MATCHES_PER_MATE`] most recent shared matches (higher ids are newer), until
/// [`LANE_MATCH_IDS`] ids are picked. Sorted and unique.
pub(crate) fn lane_match_ids(mates_json: &str) -> Vec<u64> {
    let Ok(mut rows) = parse_rows(mates_json, "mate-stats") else {
        return Vec::new();
    };
    rows.retain(|row| row.matches_played >= MIN_SHARED_MATCHES && row.id() > 0);
    rows.sort_by(|left, right| right.matches_played.cmp(&left.matches_played).then(left.id().cmp(&right.id())));
    let mut picked = std::collections::BTreeSet::new();
    for row in rows {
        let mut ids: Vec<u64> = row.matches.iter().copied().filter(|id| *id > 0).collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.truncate(LANE_MATCHES_PER_MATE);
        if picked.len() + ids.len() > LANE_MATCH_IDS {
            break;
        }
        picked.extend(ids);
    }
    picked.into_iter().collect()
}

/// One match's players as the bulk metadata request returns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchLanes {
    /// 0 or 1.
    pub winning_team: u8,
    /// `(account_id, team, assigned_lane)`.
    pub players: Vec<(u64, u8, u32)>,
}

/// A team as `0`, `1`, `"Team0"` or `"Team1"`; anything else is not a team.
fn team_of(value: Option<&serde_json::Value>) -> Option<u8> {
    match value? {
        serde_json::Value::Number(number) => number.as_u64().filter(|team| *team <= 1).map(|team| team as u8),
        serde_json::Value::String(text) => match text.as_str() {
            "Team0" | "0" => Some(0),
            "Team1" | "1" => Some(1),
            _ => None,
        },
        _ => None,
    }
}

/// Parse the bulk `/v1/matches/metadata` response: an array of matches, each with
/// `match_id`, `winning_team` and `players[]` (`account_id`, `team`, `assigned_lane`).
/// A match or player missing a field is skipped, not an error.
pub fn parse_match_lanes(json: &str) -> Result<HashMap<u64, MatchLanes>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The match lanes response did not parse: {error}")))?;
    let matches = value
        .as_array()
        .ok_or_else(|| Error::msg("The match lanes response is not a list."))?;
    let mut out = HashMap::new();
    for entry in matches {
        let Some(match_id) = entry.get("match_id").and_then(serde_json::Value::as_u64) else { continue };
        let Some(winning_team) = team_of(entry.get("winning_team")) else { continue };
        let players = entry
            .get("players")
            .and_then(serde_json::Value::as_array)
            .map(|players| {
                players
                    .iter()
                    .filter_map(|player| {
                        let account = player.get("account_id")?.as_u64().filter(|id| *id > 0)?;
                        let team = team_of(player.get("team"))?;
                        let lane = player.get("assigned_lane")?.as_u64().filter(|lane| *lane > 0 && *lane < 64)? as u32;
                        Some((account, team, lane))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.insert(match_id, MatchLanes { winning_team, players });
    }
    Ok(out)
}

/// Count each teammate's same-lane matches and wins from the shared match ids in
/// `mates_json` and the lanes read for them. Teammates with no looked-up match keep
/// `lane: None`. Sets [`Social::lanes_known`].
pub(crate) fn apply_lanes(social: &mut Social, account_id: u64, mates_json: &str, lanes: &HashMap<u64, MatchLanes>) {
    let Ok(rows) = parse_rows(mates_json, "mate-stats") else {
        return;
    };
    apply_lane_rows(social, account_id, &rows, lanes);
}

fn apply_lane_rows(social: &mut Social, account_id: u64, rows: &[RawCount], lanes: &HashMap<u64, MatchLanes>) {
    let shared: HashMap<u64, &Vec<u64>> = rows.iter().map(|row| (row.id(), &row.matches)).collect();
    for mate in &mut social.mates {
        let Some(ids) = shared.get(&mate.account_id) else { continue };
        let mut record = LaneRecord::default();
        for id in ids.iter() {
            let Some(game) = lanes.get(id) else { continue };
            let me = game.players.iter().find(|player| player.0 == account_id);
            let them = game.players.iter().find(|player| player.0 == mate.account_id);
            let (Some(me), Some(them)) = (me, them) else { continue };
            if me.1 != them.1 {
                continue;
            }
            record.checked += 1;
            if me.2 == them.2 {
                record.matches += 1;
                if me.1 == game.winning_team {
                    record.wins += 1;
                }
            }
        }
        if record.checked > 0 {
            mate.lane = Some(record);
        }
    }
    social.lanes_known = social.mates.iter().any(|mate| mate.lane.is_some());
}

#[derive(Debug, Deserialize)]
struct RawNameRow {
    account_id: u64,
    #[serde(default)]
    personaname: String,
}

#[derive(Debug, Deserialize)]
struct RawCount {
    #[serde(default)]
    mate_id: u64,
    #[serde(default)]
    enemy_id: u64,
    #[serde(default)]
    wins: u64,
    #[serde(default)]
    matches_played: u64,
    /// The shared match ids (mate-stats `matches`).
    #[serde(default)]
    matches: Vec<u64>,
}

impl RawCount {
    fn id(&self) -> u64 {
        if self.mate_id != 0 {
            self.mate_id
        } else {
            self.enemy_id
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(match_id: u64, start_time: i64, win: Option<bool>) -> crate::stats::MatchSummary {
        let json = format!(
            r#"{{"match_id": {match_id}, "hero_id": 1, "start_time": {start_time}, "game_mode": 1, "match_mode": 4,
                "player_team": 0, "player_kills": 0, "player_deaths": 0, "player_assists": 0,
                "net_worth": 0, "last_hits": 0, "denies": 0{}}}"#,
            match win {
                Some(true) => r#", "match_result": 0"#,
                Some(false) => r#", "match_result": 1"#,
                None => r#", "player_match_outcome": 5"#,
            }
        );
        serde_json::from_str::<crate::stats::RawMatch>(&json).unwrap().into_summary()
    }

    #[test]
    fn with_and_without_split_the_history_on_shared_ids() {
        let mut record = PlayerRecord::new(9, 6, 5);
        record.match_ids = vec![1, 2, 3, 4, 5, 6];
        // Shared 1 to 6 (5 wins, 1 loss); others 10 to 19 (4 wins, 6 losses); one unscored.
        let mut history: Vec<_> = (1..=6).map(|id| summary(id, 100 + id as i64, Some(id != 6))).collect();
        history.extend((10..20).map(|id| summary(id, 1000 + id as i64, Some(id < 14))));
        history.push(summary(30, 5000, None));
        let split = with_without(&record, &history);
        assert_eq!((split.with_matches, split.with_wins), (6, 5));
        assert_eq!((split.without_matches, split.without_wins), (10, 4));
        assert!(split.covered);
        assert_eq!(split.last_together, Some(106));
        let lift = split.lift().expect("lift");
        assert!((lift - (500.0 / 6.0 - 40.0)).abs() < 1e-9, "{lift}");
    }

    #[test]
    fn a_thin_window_shows_no_without_or_lift() {
        let mut record = PlayerRecord::new(9, 12, 8);
        record.match_ids = (1..=12).collect();
        // Only 5 of 12 shared matches are in the window: less than half.
        let mut history: Vec<_> = (1..=5).map(|id| summary(id, id as i64, Some(true))).collect();
        history.extend((20..40).map(|id| summary(id, id as i64, Some(false))));
        let split = with_without(&record, &history);
        assert!(!split.covered);
        assert_eq!(split.without_rate(), None);
        assert_eq!(split.lift(), None);
        assert_eq!(split.with_rate(), Some(100.0));
        // Too few on one side is "-" too.
        record.match_ids = vec![1, 2];
        let split = with_without(&record, &history);
        assert!(split.covered);
        assert_eq!(split.lift(), None, "two shared matches are below the floor");
        assert_eq!(with_without(&PlayerRecord::new(1, 0, 0), &history).covered, false);
    }

    const MATES: &str = r#"[
        {"mate_id": 10, "wins": 7, "matches_played": 10},
        {"mate_id": 11, "wins": 1, "matches_played": 2}
    ]"#;
    const PARTY: &str = r#"[
        {"mate_id": 10, "wins": 7, "matches_played": 10}
    ]"#;
    const ENEMIES: &str = r#"[
        {"enemy_id": 20, "wins": 2, "matches_played": 6},
        {"enemy_id": 21, "wins": 5, "matches_played": 5}
    ]"#;

    fn names() -> HashMap<u64, String> {
        let mut names = HashMap::new();
        names.insert(10, "Ana".to_string());
        names.insert(20, "Bo".to_string());
        names
    }

    #[test]
    fn rows_below_the_sample_floor_are_dropped() {
        let social = assemble_social(MATES, PARTY, ENEMIES, &names()).expect("social");
        assert_eq!(social.mates.len(), 1);
        assert_eq!(social.enemies.len(), 2);
        assert_eq!(social.mates[0].persona_name.as_deref(), Some("Ana"));
        assert_eq!(social.enemies[0].persona_name.as_deref(), Some("Bo"));
        assert_eq!(social.enemies[1].persona_name, None);
    }

    #[test]
    fn wins_over_matches_read_as_a_rate() {
        let social = assemble_social(MATES, PARTY, ENEMIES, &names()).expect("social");
        assert!((social.mates[0].win_rate_percent - 70.0).abs() < 1e-9);
        assert!((social.enemies[0].win_rate_percent - 100.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_premade_row_is_marked_and_leads_the_list() {
        let social = assemble_social(MATES, PARTY, ENEMIES, &names()).expect("social");
        assert!(social.mates[0].premade);
        assert_eq!(social.premades().count(), 1);
        assert_eq!(social.most_faced().map(|row| row.account_id), Some(20));
        assert_eq!(social.best_mate().map(|row| row.account_id), Some(10));
    }

    #[test]
    fn the_list_is_ordered_by_shared_matches() {
        let social = assemble_social("[]", "[]", ENEMIES, &HashMap::new()).expect("social");
        assert_eq!(social.enemies[0].account_id, 20);
        assert_eq!(social.enemies[1].account_id, 21);
    }

    #[test]
    fn a_broken_response_is_an_error() {
        assert!(assemble_social("not json", PARTY, ENEMIES, &names()).is_err());
    }

    #[test]
    fn names_cover_the_most_played_rows_once_each() {
        let ids = top_accounts(MATES, PARTY, ENEMIES, 10);
        assert_eq!(ids, vec![10, 20, 21]);
        assert_eq!(top_accounts(MATES, PARTY, ENEMIES, 2), vec![10, 20]);
        assert!(top_accounts("x", PARTY, ENEMIES, 10).contains(&20));
    }

    const LANE_MATES: &str = r#"[
        {"mate_id": 10, "wins": 4, "matches_played": 6, "matches": [101, 102, 103, 104, 105, 106]},
        {"mate_id": 11, "wins": 1, "matches_played": 3, "matches": [101, 102, 107]}
    ]"#;

    /// Six matches: player 1 laned with 10 in five (four won), with 11 never.
    const LANES: &str = r#"[
        {"match_id": 101, "winning_team": "Team0", "players": [
            {"account_id": 1, "team": "Team0", "assigned_lane": 1},
            {"account_id": 10, "team": "Team0", "assigned_lane": 1},
            {"account_id": 11, "team": "Team0", "assigned_lane": 4}]},
        {"match_id": 102, "winning_team": "Team0", "players": [
            {"account_id": 1, "team": "Team0", "assigned_lane": 6},
            {"account_id": 10, "team": "Team0", "assigned_lane": 6},
            {"account_id": 11, "team": "Team0", "assigned_lane": 1}]},
        {"match_id": 103, "winning_team": 1, "players": [
            {"account_id": 1, "team": 1, "assigned_lane": 4},
            {"account_id": 10, "team": 1, "assigned_lane": 4}]},
        {"match_id": 104, "winning_team": "Team1", "players": [
            {"account_id": 1, "team": "Team0", "assigned_lane": 4},
            {"account_id": 10, "team": "Team0", "assigned_lane": 4}]},
        {"match_id": 105, "winning_team": "Team0", "players": [
            {"account_id": 1, "team": "Team0", "assigned_lane": 1},
            {"account_id": 10, "team": "Team0", "assigned_lane": 1}]},
        {"match_id": 106, "winning_team": "Team0", "players": [
            {"account_id": 1, "team": "Team0", "assigned_lane": 1},
            {"account_id": 10, "team": "Team0", "assigned_lane": 6}]},
        {"match_id": 999, "players": []},
        {"match_id": 107, "winning_team": "Team9", "players": []}
    ]"#;

    #[test]
    fn the_floor_hides_small_samples() {
        assert_eq!(rate(1, 1), None);
        assert_eq!(rate(4, 4), None);
        assert_eq!(rate(5, 5), Some(100.0));
        assert_eq!(rate(3, 6), Some(50.0));
        // Wins never exceed matches.
        assert_eq!(rate(9, 6), Some(100.0));
    }

    #[test]
    fn lane_matches_and_wins_count_per_teammate() {
        let mut social = assemble_social(LANE_MATES, "[]", "[]", &HashMap::new()).expect("social");
        let lanes = parse_match_lanes(LANES).expect("lanes");
        assert_eq!(lanes.len(), 6, "matches without a winner are skipped");
        apply_lanes(&mut social, 1, LANE_MATES, &lanes);
        assert!(social.lanes_known);
        let ten = social.mates.iter().find(|m| m.account_id == 10).unwrap();
        assert_eq!(ten.lane, Some(LaneRecord { checked: 6, matches: 5, wins: 4 }));
        assert_eq!(ten.rate_in_lane(), Some(80.0));
        let eleven = social.mates.iter().find(|m| m.account_id == 11).unwrap();
        assert_eq!(eleven.lane, Some(LaneRecord { checked: 2, matches: 0, wins: 0 }));
        assert_eq!(eleven.rate_in_lane(), None);
        let best = social.best_partner().expect("partner");
        assert_eq!((best.account_id, best.in_lane, best.matches, best.wins), (10, true, 5, 4));
    }

    #[test]
    fn without_lanes_the_best_duo_uses_matches_together() {
        let social = assemble_social(LANE_MATES, "[]", "[]", &HashMap::new()).expect("social");
        assert!(!social.lanes_known);
        let best = social.best_partner().expect("duo");
        assert_eq!((best.account_id, best.in_lane, best.matches), (10, false, 6));
        // Player 11 has 3 matches: below the floor, no rate.
        assert_eq!(social.mates.iter().find(|m| m.account_id == 11).unwrap().rate_together(), None);
        let few = assemble_social(r#"[{"mate_id": 5, "wins": 4, "matches_played": 4}]"#, "[]", "[]", &HashMap::new()).unwrap();
        assert_eq!(few.best_partner(), None, "4 of 4 is never a best duo");
    }

    #[test]
    fn lane_ids_take_recent_matches_of_the_most_played_mates() {
        assert_eq!(lane_match_ids(LANE_MATES), vec![101, 102, 103, 104, 105, 106, 107]);
        let many: Vec<u64> = (1..=500).collect();
        let json = format!(r#"[{{"mate_id": 3, "wins": 1, "matches_played": 500, "matches": {many:?}}}]"#);
        let ids = lane_match_ids(&json);
        assert_eq!(ids.len(), LANE_MATCHES_PER_MATE);
        assert_eq!(ids.last(), Some(&500));
        assert!(lane_match_ids("junk").is_empty());
    }

    #[test]
    fn hostile_lane_responses_do_not_count() {
        assert!(parse_match_lanes("{}").is_err());
        assert!(parse_match_lanes("nope").is_err());
        let odd = r#"[{"match_id": 1, "winning_team": "Team0", "players": [
            {"account_id": "1", "team": "Team0", "assigned_lane": 1},
            {"account_id": 2, "team": "Team7", "assigned_lane": 1},
            {"account_id": 3, "team": "Team0", "assigned_lane": -1},
            {"account_id": 4, "team": "Team0", "assigned_lane": 4}]}]"#;
        let lanes = parse_match_lanes(odd).expect("lanes");
        assert_eq!(lanes[&1].players, vec![(4, 0, 4)]);
    }

    #[test]
    fn a_bundle_goes_through_the_same_assembly() {
        let json = format!(
            r#"{{"mates": {MATES}, "party": {PARTY}, "enemies": {ENEMIES},
                "steam": [{{"account_id": 10, "personaname": "Ana"}}]}}"#
        );
        let social = social_from_bundle(&json).expect("social");
        assert_eq!(social.mates.len(), 1);
        assert_eq!(social.mates[0].persona_name.as_deref(), Some("Ana"));
    }
}
