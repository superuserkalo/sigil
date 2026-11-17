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
}

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
        }
    }
}

/// The people a player meets, named.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Social {
    /// Teammates, most shared matches first.
    pub mates: Vec<PlayerRecord>,
    /// Opponents, most shared matches first.
    pub enemies: Vec<PlayerRecord>,
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
    Ok(merge(bundle.mates, bundle.party, bundle.enemies, &names))
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
    }
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
