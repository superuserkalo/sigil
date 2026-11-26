//! One match, every player: the scoreboard behind a match row.
//!
//! Source: `GET /v1/matches/{match_id}/metadata?disable_steam=true` (about 900 KB
//! for one match), `/v1/assets/items/by-type/upgrade` for item names and
//! `/v1/players/steam` for the twelve names. The metadata response is used
//! without the demo-only parts (movement paths, damage matrix, accolades), which
//! this module never reads.
//!
//! What the response carries per player: the final score line, the level, the
//! items still held at the end, the rank badge the match was played at, the MVP
//! vote and a net worth sample every three minutes. That gives the scoreboard,
//! both teams' souls over time and the final build. It does not give positions,
//! damage per player at a chosen moment, or aim; those need the demo lane.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::util::unix_seconds_to_iso;

/// One player on the scoreboard.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardPlayer {
    pub account_id: u64,
    pub persona_name: Option<String>,
    pub hero_id: u32,
    pub hero_name: Option<String>,
    pub team: i32,
    pub player_slot: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub net_worth: u64,
    pub last_hits: u32,
    pub denies: u32,
    pub level: u32,
    /// Damage dealt to players, from the last sample of the match.
    pub player_damage: u64,
    /// Healing given, from the last sample of the match.
    pub player_healing: u64,
    pub damage_taken: u64,
    pub badge: Option<u32>,
    pub mvp_rank: Option<u32>,
    /// Items still held when the match ended, in shop order.
    pub build: Vec<BuildItem>,
    /// Seconds into the match when the game recorded this player as abandoning
    /// (`abandon_match_time_s`). `None` for a player who stayed.
    pub abandoned_seconds: Option<u32>,
    /// How the match was scored for this player (`player_match_outcome`), when the
    /// metadata says.
    pub outcome: Option<super::MatchOutcome>,
    /// The lane the game assigned (`assigned_lane`), when the metadata says.
    pub lane: Option<u32>,
    /// The Steam avatar URL (`avatarmedium`) from the name request, when it answered.
    pub avatar: Option<String>,
}

impl BoardPlayer {
    pub fn kda(&self) -> f64 {
        (self.kills + self.assists) as f64 / self.deaths.max(1) as f64
    }

    /// Souls a minute over the whole match: net worth over minutes played.
    pub fn souls_per_minute(&self, duration_seconds: u32) -> u64 {
        souls_per_minute(self.net_worth, duration_seconds)
    }
}

/// Souls a minute: `net_worth / (duration / 60)`, rounded; 0 for a match with no length.
pub fn souls_per_minute(net_worth: u64, duration_seconds: u32) -> u64 {
    if duration_seconds == 0 {
        return 0;
    }
    (net_worth as f64 / (duration_seconds as f64 / 60.0)).round() as u64
}

/// The number columns of the match page, for the lobby best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    Kills,
    Assists,
    Souls,
    SoulsPerMinute,
    Damage,
    Healing,
    LastHits,
}

impl Column {
    pub const ALL: [Column; 7] = [
        Column::Kills,
        Column::Assists,
        Column::Souls,
        Column::SoulsPerMinute,
        Column::Damage,
        Column::Healing,
        Column::LastHits,
    ];

    fn value(self, player: &BoardPlayer, duration_seconds: u32) -> u64 {
        match self {
            Column::Kills => player.kills.into(),
            Column::Assists => player.assists.into(),
            Column::Souls => player.net_worth,
            Column::SoulsPerMinute => player.souls_per_minute(duration_seconds),
            Column::Damage => player.player_damage,
            Column::Healing => player.player_healing,
            Column::LastHits => player.last_hits.into(),
        }
    }
}

/// One item in a final build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildItem {
    pub item_id: u64,
    pub name: String,
    /// `weapon`, `vitality`, `spirit`, or `other` when the catalog did not know it.
    pub slot: String,
    pub tier: u32,
    /// Seconds into the match of the latest purchase.
    pub bought_at: u32,
    /// The item's icon on `assets-bucket.deadlock-api.com`, when the catalog names one.
    pub icon: Option<String>,
}

/// Souls of both teams at one sample of the match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoulPoint {
    pub time_s: u32,
    pub team0: u64,
    pub team1: u64,
}

impl SoulPoint {
    /// Team 0 minus team 1, in souls.
    pub fn lead(&self) -> i64 {
        self.team0 as i64 - self.team1 as i64
    }
}

/// Everything one match's scoreboard shows.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchScoreboard {
    pub match_id: u64,
    pub start_time: i64,
    pub started_at: String,
    pub duration_seconds: u32,
    pub game_mode: i32,
    pub match_mode: i32,
    pub winning_team: Option<i32>,
    /// The account whose page opened this board, so the view can mark the row.
    pub account_id: u64,
    /// Team 0 first, each ordered by net worth.
    pub teams: [Vec<BoardPlayer>; 2],
    pub souls: Vec<SoulPoint>,
    /// Each team's average rank badge (`average_badge_team0/1`), when the API has one.
    pub average_badge: [Option<u32>; 2],
    /// Objectives lost, as (seconds into the match, team that lost it), earliest first: walkers,
    /// guardians and base buildings from `objectives`, and each Mid Boss kill as a loss for the
    /// team that did not claim it.
    pub objectives_lost: Vec<(u32, i32)>,
}

impl MatchScoreboard {
    /// The player whose page opened the board, when they are on it.
    pub fn me(&self) -> Option<&BoardPlayer> {
        self.teams.iter().flatten().find(|player| player.account_id == self.account_id && self.account_id > 0)
    }

    /// Teammates of the page's player who the game recorded as abandoning, earliest first.
    /// Empty when the player is not on the board.
    pub fn teammates_who_left(&self) -> Vec<&BoardPlayer> {
        let Some(me) = self.me() else {
            return Vec::new();
        };
        let mut left: Vec<&BoardPlayer> = self
            .team(me.team.clamp(0, 1) as usize)
            .iter()
            .filter(|player| player.account_id != me.account_id && player.abandoned_seconds.is_some())
            .collect();
        left.sort_by_key(|player| player.abandoned_seconds);
        left
    }

    /// The two teams, team 0 first.
    pub fn team(&self, index: usize) -> &[BoardPlayer] {
        self.teams.get(index).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The team of the player the page is about, when they are on the board.
    pub fn my_team(&self) -> Option<usize> {
        self.me().map(|me| me.team.clamp(0, 1) as usize)
    }

    /// Whether `account_id` holds the lobby's best value in `column` (ties all count). A column
    /// where every value is 0 has no best.
    pub fn is_best(&self, column: Column, account_id: u64) -> bool {
        let values: Vec<(u64, u64)> = self
            .teams
            .iter()
            .flatten()
            .map(|player| (player.account_id, column.value(player, self.duration_seconds)))
            .collect();
        let top = values.iter().map(|(_, value)| *value).max().unwrap_or(0);
        top > 0 && values.iter().any(|(id, value)| *id == account_id && *value == top)
    }
}

/// Item id to name, slot and tier, from `/v1/assets/items/by-type/upgrade`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemCatalog {
    pub items: HashMap<u64, (String, String, u32)>,
    /// Item id to icon URL (`shop_image_webp`, else `shop_image`).
    pub icons: HashMap<u64, String>,
}

impl ItemCatalog {
    pub fn icon(&self, item_id: u64) -> Option<&str> {
        self.icons.get(&item_id).map(String::as_str)
    }

    pub fn get(&self, item_id: u64) -> Option<&(String, String, u32)> {
        self.items.get(&item_id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Every account id on a metadata response, for the one name request.
pub(crate) fn accounts_in(metadata_json: &str) -> Vec<u64> {
    let Ok(raw) = serde_json::from_str::<RawMetadata>(metadata_json) else {
        return Vec::new();
    };
    raw.match_info
        .players
        .into_iter()
        .map(|player| player.account_id)
        .filter(|account_id| *account_id > 0)
        .collect()
}

/// A bundle captured from the API in one file, for an offline fixture. Shapes are
/// the API's own (`metadata` is the metadata response), trimmed to the fields the
/// board reads.
#[derive(Debug, Deserialize)]
struct BoardBundle {
    account_id: u64,
    scoreboard: RawMetadata,
    #[serde(default)]
    items: Vec<RawItemAsset>,
    #[serde(default)]
    steam: Vec<RawNameRow>,
    #[serde(default)]
    hero_names: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawNameRow {
    account_id: u64,
    #[serde(default)]
    personaname: String,
}

#[derive(Debug, Deserialize)]
struct RawItemAsset {
    id: u64,
    name: String,
    #[serde(default)]
    item_slot_type: String,
    #[serde(default)]
    item_tier: u32,
    #[serde(default)]
    shop_image_webp: Option<String>,
    #[serde(default)]
    shop_image: Option<String>,
}

impl RawItemAsset {
    fn icon(&self) -> Option<String> {
        self.shop_image_webp.clone().or_else(|| self.shop_image.clone()).filter(|url| !url.is_empty())
    }

    fn into_catalog_row(self) -> (u64, (String, String, u32)) {
        (self.id, (self.name, self.item_slot_type, self.item_tier))
    }
}

fn catalog_of(rows: Vec<RawItemAsset>) -> ItemCatalog {
    let mut catalog = ItemCatalog { items: HashMap::with_capacity(rows.len()), icons: HashMap::new() };
    for row in rows {
        if let Some(icon) = row.icon() {
            catalog.icons.insert(row.id, icon);
        }
        let (id, value) = row.into_catalog_row();
        catalog.items.insert(id, value);
    }
    catalog
}

/// Build a catalog from the item asset response.
pub(crate) fn parse_item_catalog(json: &str) -> Result<ItemCatalog> {
    let rows: Vec<RawItemAsset> = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Item asset did not parse: {error}")))?;
    Ok(catalog_of(rows))
}

/// Parse one metadata response and assemble the board.
pub(crate) fn assemble_board(
    account_id: u64,
    metadata_json: &str,
    catalog: &ItemCatalog,
    names: &HashMap<u64, String>,
    hero_names: &HashMap<u32, String>,
) -> Result<MatchScoreboard> {
    let raw: RawMetadata = serde_json::from_str(metadata_json)
        .map_err(|error| Error::msg(format!("Match metadata did not parse: {error}")))?;
    if raw.match_info.players.is_empty() {
        return Err(Error::msg("This match has no players in its metadata."));
    }
    Ok(build_board(account_id, raw, catalog, names, hero_names))
}

/// A scoreboard from a captured bundle, through the same assembly as a live fetch.
pub fn scoreboard_from_bundle(json: &str) -> Result<MatchScoreboard> {
    let bundle: BoardBundle = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("Scoreboard bundle did not parse: {error}")))?;
    let catalog: ItemCatalog = catalog_of(bundle.items);
    let names: HashMap<u64, String> = bundle
        .steam
        .into_iter()
        .map(|row| (row.account_id, row.personaname))
        .collect();
    let hero_names: HashMap<u32, String> = bundle
        .hero_names
        .into_iter()
        .filter_map(|(key, value)| key.parse::<u32>().ok().map(|key| (key, value)))
        .collect();
    let raw: RawMetadata = bundle.scoreboard;
    if raw.match_info.players.is_empty() {
        return Err(Error::msg("Scoreboard bundle has no players."));
    }
    Ok(build_board(
        bundle.account_id,
        raw,
        &catalog,
        &names,
        &hero_names,
    ))
}

fn build_board(
    account_id: u64,
    raw: RawMetadata,
    catalog: &ItemCatalog,
    names: &HashMap<u64, String>,
    hero_names: &HashMap<u32, String>,
) -> MatchScoreboard {
    let info = raw.match_info;
    let mut souls: BTreeMap<u32, [u64; 2]> = BTreeMap::new();
    let mut teams: [Vec<BoardPlayer>; 2] = [Vec::new(), Vec::new()];

    for player in info.players {
        let team = player.team.clamp(0, 1);
        let last = player
            .stats
            .iter()
            .max_by_key(|sample| sample.time_stamp_s);
        for sample in &player.stats {
            let slot = souls.entry(sample.time_stamp_s).or_insert([0, 0]);
            slot[team as usize] = slot[team as usize].saturating_add(sample.net_worth);
        }
        teams[team as usize].push(BoardPlayer {
            account_id: player.account_id,
            persona_name: names.get(&player.account_id).cloned(),
            hero_id: player.hero_id,
            hero_name: hero_names.get(&player.hero_id).cloned(),
            team,
            player_slot: player.player_slot,
            kills: player.kills,
            deaths: player.deaths,
            assists: player.assists,
            net_worth: player.net_worth,
            last_hits: player.last_hits,
            denies: player.denies,
            level: player.level,
            player_damage: last.map(|sample| sample.player_damage).unwrap_or(0),
            player_healing: last.map(|sample| sample.player_healing).unwrap_or(0),
            damage_taken: last.map(|sample| sample.player_damage_taken).unwrap_or(0),
            badge: player
                .player_rank_data
                .and_then(|rank| rank.initial_display_rank)
                .filter(|badge| *badge > 0),
            mvp_rank: player.mvp_rank.filter(|rank| *rank > 0),
            build: final_build(&player.items, catalog),
            abandoned_seconds: player.abandon_match_time_s.filter(|seconds| *seconds > 0),
            outcome: player.player_match_outcome.map(super::MatchOutcome::from_api),
            lane: player.assigned_lane.filter(|lane| *lane > 0),
            avatar: None,
        });
    }

    for team in &mut teams {
        team.sort_by(|left, right| {
            right
                .net_worth
                .cmp(&left.net_worth)
                .then(left.player_slot.cmp(&right.player_slot))
        });
    }

    let mut objectives_lost: Vec<(u32, i32)> = info
        .objectives
        .iter()
        .filter(|objective| objective.destroyed_time_s > 0)
        .map(|objective| (objective.destroyed_time_s, objective.team))
        .chain(
            info.mid_boss
                .iter()
                .filter(|boss| boss.destroyed_time_s > 0 && (0..=1).contains(&boss.team_claimed))
                .map(|boss| (boss.destroyed_time_s, 1 - boss.team_claimed)),
        )
        .collect();
    objectives_lost.sort_unstable();
    let badge = |value: Option<u32>| value.filter(|badge| *badge > 0);
    MatchScoreboard {
        average_badge: [badge(info.average_badge_team0), badge(info.average_badge_team1)],
        objectives_lost,
        match_id: info.match_id,
        start_time: info.start_time,
        started_at: unix_seconds_to_iso(info.start_time),
        duration_seconds: info.duration_s,
        game_mode: info.game_mode,
        match_mode: info.match_mode,
        winning_team: info.winning_team.filter(|team| *team >= 0),
        account_id,
        teams,
        souls: souls
            .into_iter()
            .map(|(time_s, [team0, team1])| SoulPoint {
                time_s,
                team0,
                team1,
            })
            .collect(),
    }
}

/// The items a player still held at the end: every purchase that was not sold,
/// one row per item (the latest), in shop order weapon, vitality, spirit.
fn final_build(items: &[RawItem], catalog: &ItemCatalog) -> Vec<BuildItem> {
    let mut latest: BTreeMap<u64, u32> = BTreeMap::new();
    let icon = |item_id: u64| catalog.icon(item_id).map(str::to_string);
    for item in items.iter().filter(|item| item.sold_time_s == 0) {
        let at = latest.entry(item.item_id).or_insert(0);
        *at = (*at).max(item.game_time_s);
    }
    let mut build: Vec<(u8, BuildItem)> = latest
        .into_iter()
        .map(|(item_id, bought_at)| match catalog.get(item_id) {
            Some((name, slot, tier)) => (
                slot_order(slot),
                BuildItem {
                    item_id,
                    name: name.clone(),
                    slot: slot.clone(),
                    tier: *tier,
                    bought_at,
                    icon: icon(item_id),
                },
            ),
            None => (
                slot_order(""),
                BuildItem {
                    item_id,
                    name: format!("Item {item_id}"),
                    slot: "other".to_string(),
                    tier: 0,
                    bought_at,
                    icon: None,
                },
            ),
        })
        .collect();
    build.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.tier.cmp(&right.1.tier))
            .then(left.1.name.cmp(&right.1.name))
    });
    build.into_iter().map(|(_, item)| item).collect()
}

fn slot_order(slot: &str) -> u8 {
    match slot {
        "weapon" => 0,
        "vitality" => 1,
        "spirit" => 2,
        _ => 3,
    }
}

#[derive(Debug, Deserialize)]
struct RawMetadata {
    match_info: RawMatchInfo,
}

#[derive(Debug, Deserialize)]
struct RawMatchInfo {
    #[serde(default)]
    match_id: u64,
    #[serde(default)]
    start_time: i64,
    #[serde(default)]
    duration_s: u32,
    #[serde(default)]
    game_mode: i32,
    #[serde(default)]
    match_mode: i32,
    #[serde(default)]
    winning_team: Option<i32>,
    #[serde(default)]
    players: Vec<RawBoardPlayer>,
    #[serde(default)]
    average_badge_team0: Option<u32>,
    #[serde(default)]
    average_badge_team1: Option<u32>,
    #[serde(default)]
    objectives: Vec<RawObjective>,
    #[serde(default)]
    mid_boss: Vec<RawMidBoss>,
}

#[derive(Debug, Deserialize)]
struct RawObjective {
    #[serde(default)]
    destroyed_time_s: u32,
    #[serde(default)]
    team: i32,
}

#[derive(Debug, Deserialize)]
struct RawMidBoss {
    #[serde(default)]
    destroyed_time_s: u32,
    #[serde(default)]
    team_claimed: i32,
}

#[derive(Debug, Deserialize)]
struct RawBoardPlayer {
    account_id: u64,
    #[serde(default)]
    player_slot: u32,
    #[serde(default)]
    team: i32,
    #[serde(default)]
    kills: u32,
    #[serde(default)]
    deaths: u32,
    #[serde(default)]
    assists: u32,
    #[serde(default)]
    net_worth: u64,
    #[serde(default)]
    hero_id: u32,
    #[serde(default)]
    last_hits: u32,
    #[serde(default)]
    denies: u32,
    #[serde(default)]
    level: u32,
    #[serde(default)]
    mvp_rank: Option<u32>,
    #[serde(default)]
    items: Vec<RawItem>,
    #[serde(default)]
    stats: Vec<RawSample>,
    #[serde(default)]
    player_rank_data: Option<RawRankData>,
    #[serde(default)]
    abandon_match_time_s: Option<u32>,
    #[serde(default)]
    player_match_outcome: Option<i32>,
    #[serde(default)]
    assigned_lane: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawRankData {
    #[serde(default)]
    initial_display_rank: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawItem {
    #[serde(default)]
    item_id: u64,
    #[serde(default)]
    game_time_s: u32,
    #[serde(default)]
    sold_time_s: u32,
}

#[derive(Debug, Deserialize)]
struct RawSample {
    #[serde(default)]
    time_stamp_s: u32,
    #[serde(default)]
    net_worth: u64,
    #[serde(default)]
    player_damage: u64,
    #[serde(default)]
    player_healing: u64,
    #[serde(default)]
    player_damage_taken: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const METADATA: &str = r#"{
        "match_info": {
            "match_id": 42,
            "start_time": 1794841957,
            "duration_s": 1800,
            "game_mode": 1,
            "match_mode": 4,
            "winning_team": 1,
            "players": [
                {"account_id": 1, "player_slot": 0, "team": 0, "kills": 5, "deaths": 4, "assists": 7,
                 "net_worth": 20000, "hero_id": 7, "last_hits": 100, "denies": 3, "level": 25,
                 "mvp_rank": 2,
                 "player_rank_data": {"initial_display_rank": 0},
                 "items": [{"item_id": 11, "game_time_s": 100, "sold_time_s": 0},
                           {"item_id": 11, "game_time_s": 900, "sold_time_s": 0},
                           {"item_id": 12, "game_time_s": 700, "sold_time_s": 0},
                           {"item_id": 13, "game_time_s": 800, "sold_time_s": 300},
                           {"item_id": 99, "game_time_s": 50, "sold_time_s": 0}],
                 "stats": [{"time_stamp_s": 180, "net_worth": 5000, "player_damage": 900, "player_healing": 0},
                           {"time_stamp_s": 1800, "net_worth": 20000, "player_damage": 15000, "player_healing": 400}]},
                {"account_id": 2, "player_slot": 6, "team": 1, "kills": 9, "deaths": 2, "assists": 4,
                 "net_worth": 26000, "hero_id": 8,
                 "items": [], "stats": [{"time_stamp_s": 180, "net_worth": 6000},
                                        {"time_stamp_s": 1800, "net_worth": 26000}]}
            ]
        }
    }"#;

    const ITEMS: &str = r#"[
        {"id": 11, "name": "Rapid Recharge", "item_slot_type": "spirit", "item_tier": 2},
        {"id": 12, "name": "Bullet Armor", "item_slot_type": "weapon", "item_tier": 3}
    ]"#;

    #[test]
    fn leavers_are_teammates_with_an_abandon_time() {
        let metadata = r#"{"match_info": {"match_id": 7, "winning_team": 0, "players": [
            {"account_id": 1, "team": 0, "hero_id": 1, "player_match_outcome": 5},
            {"account_id": 2, "team": 0, "hero_id": 2, "abandon_match_time_s": 610},
            {"account_id": 3, "team": 0, "hero_id": 3, "abandon_match_time_s": 240},
            {"account_id": 4, "team": 0, "hero_id": 4, "abandon_match_time_s": 0},
            {"account_id": 5, "team": 1, "hero_id": 5, "abandon_match_time_s": 100}
        ]}}"#;
        let board = assemble_board(1, metadata, &ItemCatalog::default(), &HashMap::new(), &HashMap::new()).unwrap();
        assert_eq!(board.me().and_then(|me| me.outcome), Some(crate::stats::MatchOutcome::NotScored));
        let left: Vec<u64> = board.teammates_who_left().iter().map(|player| player.account_id).collect();
        assert_eq!(left, vec![3, 2], "own team only, earliest first, a zero time is no abandon");
        let other = assemble_board(9, metadata, &ItemCatalog::default(), &HashMap::new(), &HashMap::new()).unwrap();
        assert!(other.teammates_who_left().is_empty(), "no player, no teammates");
    }

    #[test]
    fn history_rows_without_an_outcome_fall_back_to_the_winning_team() {
        let row = |extra: &str| {
            let json = format!(
                r#"{{"match_id": 1, "hero_id": 1, "start_time": 0, "game_mode": 1, "match_mode": 4,
                    "player_team": 1, "player_kills": 0, "player_deaths": 0, "player_assists": 0,
                    "net_worth": 0, "last_hits": 0, "denies": 0{extra}}}"#
            );
            serde_json::from_str::<crate::stats::RawMatch>(&json).unwrap().into_summary()
        };
        assert_eq!(row(r#", "player_match_outcome": 5, "match_result": 1"#).win, None, "not scored is no loss");
        assert_eq!(row(r#", "match_result": 1"#).win, Some(true));
        assert_eq!(row(r#", "match_result": 0"#).win, Some(false));
        assert_eq!(row("").outcome, crate::stats::MatchOutcome::Unknown);
        assert_eq!(row(r#", "abandoned_time_s": 0"#).abandoned_seconds, None);
        assert_eq!(row(r#", "abandoned_time_s": 420"#).abandoned_seconds, Some(420));
    }

    #[test]
    fn catalog_reads_names_and_slots() {
        let catalog = parse_item_catalog(ITEMS).expect("catalog");
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.get(12).map(|row| row.0.as_str()), Some("Bullet Armor"));
        assert_eq!(catalog.get(12).map(|row| row.1.as_str()), Some("weapon"));
    }

    #[test]
    fn board_sums_souls_and_orders_teams_by_net_worth() {
        let catalog = parse_item_catalog(ITEMS).expect("catalog");
        let mut names = HashMap::new();
        names.insert(2, "Old Mate".to_string());
        let mut heroes = HashMap::new();
        heroes.insert(7, "Haze".to_string());
        let board = assemble_board(1, METADATA, &catalog, &names, &heroes).expect("board");
        assert_eq!(board.match_id, 42);
        assert_eq!(board.winning_team, Some(1));
        assert_eq!(board.team(0).len(), 1);
        assert_eq!(board.team(1).len(), 1);
        assert_eq!(board.team(1)[0].persona_name.as_deref(), Some("Old Mate"));
        assert_eq!(board.team(0)[0].hero_name.as_deref(), Some("Haze"));
        assert_eq!(
            board.souls,
            vec![
                SoulPoint {
                    time_s: 180,
                    team0: 5000,
                    team1: 6000
                },
                SoulPoint {
                    time_s: 1800,
                    team0: 20000,
                    team1: 26000
                }
            ]
        );
        assert_eq!(board.souls[1].lead(), -6000);
    }

    #[test]
    fn final_build_drops_sold_items_and_orders_by_slot() {
        let catalog = parse_item_catalog(ITEMS).expect("catalog");
        let board = assemble_board(1, METADATA, &catalog, &HashMap::new(), &HashMap::new())
            .expect("board");
        let ours = &board.team(0)[0];
        assert_eq!(ours.build.len(), 3);
        assert_eq!(ours.build[0].name, "Bullet Armor");
        assert_eq!(ours.build[1].name, "Rapid Recharge");
        assert_eq!(ours.build[2].name, "Item 99");
        assert_eq!(ours.build[2].slot, "other");
        assert_eq!(ours.player_damage, 15000);
        assert_eq!(ours.player_healing, 400);
        assert_eq!(ours.badge, None);
        assert_eq!(ours.mvp_rank, Some(2));
        assert!((ours.kda() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn lobby_best_and_souls_a_minute() {
        let catalog = parse_item_catalog(ITEMS).expect("catalog");
        let board = assemble_board(1, METADATA, &catalog, &HashMap::new(), &HashMap::new()).expect("board");
        // 20,000 souls over 30 minutes.
        assert_eq!(board.team(0)[0].souls_per_minute(board.duration_seconds), 667);
        assert_eq!(souls_per_minute(1000, 0), 0);
        assert!(board.is_best(Column::Kills, 2));
        assert!(!board.is_best(Column::Kills, 1));
        assert!(board.is_best(Column::Assists, 1));
        assert!(board.is_best(Column::Damage, 1), "player 2 has no damage sample");
        assert!(!board.is_best(Column::Healing, 2), "player 2 healed nothing");
        assert!(!board.is_best(Column::LastHits, 2));
        assert_eq!(board.my_team(), Some(0));
    }

    #[test]
    fn build_items_carry_their_buy_time_and_icon() {
        let items = r#"[{"id": 11, "name": "Rapid Recharge", "item_slot_type": "spirit", "item_tier": 2,
            "shop_image": "https://assets-bucket.deadlock-api.com/a.png",
            "shop_image_webp": "https://assets-bucket.deadlock-api.com/a.webp"}]"#;
        let catalog = parse_item_catalog(items).expect("catalog");
        let board = assemble_board(1, METADATA, &catalog, &HashMap::new(), &HashMap::new()).expect("board");
        let recharge = board.team(0)[0].build.iter().find(|item| item.item_id == 11).expect("item 11");
        assert_eq!(recharge.bought_at, 900, "the latest purchase");
        assert_eq!(recharge.icon.as_deref(), Some("https://assets-bucket.deadlock-api.com/a.webp"));
    }

    #[test]
    fn objectives_badges_and_lanes_read_from_the_metadata() {
        let metadata = r#"{"match_info": {"match_id": 7, "duration_s": 1800, "winning_team": 0,
            "average_badge_team0": 113, "average_badge_team1": 0,
            "objectives": [{"destroyed_time_s": 900, "team": 1}, {"destroyed_time_s": 0, "team": 0},
                           {"destroyed_time_s": 400, "team": 0}],
            "mid_boss": [{"destroyed_time_s": 1200, "team_claimed": 0}],
            "players": [{"account_id": 1, "team": 0, "hero_id": 1, "assigned_lane": 4},
                        {"account_id": 2, "team": 1, "hero_id": 2}]}}"#;
        let board = assemble_board(1, metadata, &ItemCatalog::default(), &HashMap::new(), &HashMap::new()).unwrap();
        assert_eq!(board.average_badge, [Some(113), None]);
        assert_eq!(board.objectives_lost, vec![(400, 0), (900, 1), (1200, 1)]);
        assert_eq!(board.team(0)[0].lane, Some(4));
        assert_eq!(board.team(1)[0].lane, None);
    }

    #[test]
    fn an_empty_match_is_an_error() {
        let catalog = ItemCatalog::default();
        let empty = r#"{"match_info": {"players": []}}"#;
        assert!(
            assemble_board(1, empty, &catalog, &HashMap::new(), &HashMap::new()).is_err()
        );
        assert!(scoreboard_from_bundle("{}").is_err());
    }
}
