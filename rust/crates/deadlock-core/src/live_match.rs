//! The match being played right now, read from `game/citadel/console.log` (the game writes it
//! with `-condebug`), and the live board the Sigil live bridge prints into the same log.
//!
//! The console tells Sigil when matchmaking starts, when a lobby for a match id is created and
//! destroyed, and the match's game state. The live bridge (`crate::live_bridge`, an opt-in HUD
//! mod) adds one line per top bar team every time a number on it changes:
//!
//! ```text
//! 10/28 12:10:02 [PanoramaScript] [SIGIL-LIVE] {"v":1,"side":"friendly","clock":"6:25","players":[..]}
//! 10/28 12:09:40 [PanoramaScript] [SIGIL-LIVE-ROSTER] {"v":1,"teams":[[{"name":"..","id":".."}],[..]]}
//! ```
//!
//! Deadlock Mod Manager (GPL-3.0) has the same live tab. The log lines this reacts to are the
//! game's; the wire format, the fold and the tests here are Sigil's own.
//!
//! Nothing here touches the network or the game folder. [`LiveTail`] reads only new bytes on
//! each poll.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone};
use serde::Deserialize;

use crate::game_session::{self, Session};

/// The tag in front of a team report.
pub const BRIDGE_TAG: &str = "[SIGIL-LIVE]";
/// The tag in front of the versus screen's roster.
pub const ROSTER_TAG: &str = "[SIGIL-LIVE-ROSTER]";
/// Bumped with every wire change, here and in `live_bridge.js`. Other versions are skipped.
pub const BRIDGE_VERSION: u32 = 1;
/// Bytes read from the end of a log Sigil starts following late. One match writes a few
/// megabytes at most, bridge lines included, so this reaches back to the lobby line.
pub const REPLAY_BYTES: u64 = 16 * 1024 * 1024;
/// The bridge repeats an unchanged board every 20 seconds; a board older than this reads as
/// "no signal".
pub const STALE_AFTER_MS: i64 = 45_000;
/// Match ids remembered from one log.
const RECENT_KEEP: usize = 20;
/// Soul samples kept for a match: one per 10 seconds of game clock for two hours.
const SAMPLES_KEEP: usize = 720;
/// Game clock seconds between two soul samples.
const SAMPLE_EVERY_S: u32 = 10;
/// SteamID64 of account 0.
const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

// --- the board --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    /// From the lobby until the zipline: hero select, the versus screen, the intro.
    Pregame,
    InProgress,
    /// Any later state: the post-game screen.
    Ended,
}

impl MatchPhase {
    fn from_state(name: &str, number: u32) -> MatchPhase {
        let name = name.to_ascii_lowercase();
        if name == "gameinprogress" || number == 7 {
            MatchPhase::InProgress
        } else if matches!(
            name.as_str(),
            "waitingforplayerstojoin" | "heroselection" | "matchintro" | "waitformaptoload" | "pregamewait"
        ) || number < 7
        {
            MatchPhase::Pregame
        } else {
            MatchPhase::Ended
        }
    }
}

/// The match whose lobby is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentMatch {
    pub match_id: u64,
    pub lobby_id: u64,
    pub phase: MatchPhase,
    /// When the lobby was created, Unix milliseconds.
    pub created_at: Option<i64>,
    /// When the game state turned to in progress, Unix milliseconds.
    pub started_at: Option<i64>,
}

/// The two teams. The top bar's `TeamFriendly` panel is always team 2 and `TeamEnemy` always
/// team 3, whichever one the player is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Team {
    #[default]
    HiddenKing,
    Archmother,
}

/// One player as the top bar shows them. A value the HUD did not resolve stays `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LivePlayer {
    pub name: String,
    /// The 32-bit account id, matched by name from the versus screen's roster.
    pub account_id: Option<u32>,
    pub hero_id: Option<u32>,
    /// The hero's name as the game shows it, in the game's language.
    pub hero: Option<String>,
    pub souls: Option<u32>,
    pub level: Option<i32>,
    pub kills: Option<i32>,
    pub deaths: Option<i32>,
    pub assists: Option<i32>,
    pub objective_damage: Option<i32>,
    /// The player at this computer.
    pub local: bool,
    pub dead: bool,
}

/// Souls at one point of the game clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub clock_s: u32,
    /// The player's own souls.
    pub mine: Option<u32>,
    pub friendly: u64,
    pub enemy: u64,
}

impl Sample {
    /// The player's team minus the other.
    pub fn lead(&self) -> i64 {
        self.friendly as i64 - self.enemy as i64
    }
}

/// The live board: the player's team first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Board {
    /// The top bar clock, in seconds.
    pub clock_s: Option<u32>,
    pub friendly: Vec<LivePlayer>,
    pub enemy: Vec<LivePlayer>,
    /// Which team `friendly` is.
    pub team: Team,
    /// When the newest report was written, Unix milliseconds.
    pub reported_at: Option<i64>,
    pub samples: Vec<Sample>,
}

impl Board {
    pub fn me(&self) -> Option<&LivePlayer> {
        self.friendly.iter().chain(&self.enemy).find(|player| player.local)
    }

    pub fn souls(players: &[LivePlayer]) -> u64 {
        players.iter().filter_map(|player| player.souls).map(u64::from).sum()
    }

    pub fn kills(players: &[LivePlayer]) -> i64 {
        players.iter().filter_map(|player| player.kills).map(i64::from).sum()
    }

    /// The player's team minus the other, in souls.
    pub fn lead(&self) -> i64 {
        Self::souls(&self.friendly) as i64 - Self::souls(&self.enemy) as i64
    }

    /// The player's souls a minute, once the clock passed a minute.
    pub fn souls_per_minute(&self) -> Option<u32> {
        let clock = self.clock_s.filter(|clock| *clock >= 60)?;
        let souls = self.me()?.souls?;
        Some((u64::from(souls) * 60 / u64::from(clock)) as u32)
    }

    /// True when the bridge has been quiet for longer than its heartbeat allows.
    pub fn stale(&self, now_ms: i64) -> bool {
        self.reported_at.is_none_or(|at| now_ms - at > STALE_AFTER_MS)
    }

    pub fn is_empty(&self) -> bool {
        self.friendly.is_empty() && self.enemy.is_empty()
    }
}

/// A match whose lobby closed after it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedMatch {
    pub match_id: u64,
    /// The last board the bridge printed, when it was loaded.
    pub board: Option<Board>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

impl FinishedMatch {
    /// Match length: the last board's clock, else the time between start and teardown.
    pub fn duration_s(&self) -> Option<u32> {
        if let Some(clock) = self.board.as_ref().and_then(|board| board.clock_s) {
            return Some(clock);
        }
        let (start, end) = (self.started_at?, self.ended_at?);
        (end > start).then(|| ((end - start) / 1000) as u32)
    }
}

// --- the bridge's wire format -----------------------------------------------------------

#[derive(Deserialize)]
struct Versioned<T> {
    v: u32,
    #[serde(flatten)]
    body: T,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Side {
    Friendly,
    Enemy,
}

#[derive(Deserialize)]
struct Report {
    side: Side,
    clock: Option<String>,
    players: Vec<WirePlayer>,
}

#[derive(Deserialize)]
struct WirePlayer {
    name: Option<String>,
    hero: Option<String>,
    hero_id: Option<u32>,
    souls: Option<String>,
    level: Option<i32>,
    k: Option<i32>,
    d: Option<i32>,
    a: Option<i32>,
    obj: Option<i32>,
    me: Option<bool>,
    dead: Option<bool>,
}

#[derive(Deserialize)]
struct Roster {
    teams: Vec<Vec<RosterEntry>>,
}

#[derive(Deserialize)]
struct RosterEntry {
    name: Option<String>,
    id: Option<String>,
}

impl From<WirePlayer> for LivePlayer {
    fn from(wire: WirePlayer) -> LivePlayer {
        LivePlayer {
            name: wire.name.unwrap_or_default(),
            account_id: None,
            hero_id: wire.hero_id.filter(|id| *id > 0),
            hero: wire.hero.filter(|hero| !hero.trim().is_empty()),
            souls: wire.souls.as_deref().and_then(parse_souls),
            level: wire.level,
            kills: wire.k,
            deaths: wire.d,
            assists: wire.a,
            objective_damage: wire.obj,
            local: wire.me.unwrap_or(false),
            dead: wire.dead.unwrap_or(false),
        }
    }
}

fn versioned<T: for<'de> Deserialize<'de>>(json: &str) -> Option<T> {
    let parsed: Versioned<T> = serde_json::from_str(json.trim()).ok()?;
    (parsed.v == BRIDGE_VERSION).then_some(parsed.body)
}

fn after<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let at = line.find(tag)?;
    Some(&line[at + tag.len()..])
}

/// Souls as the top bar prints them: `980`, `9,870`, `12.3k`, or with the suffix in a span.
pub fn parse_souls(text: &str) -> Option<u32> {
    let mut plain = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            ',' | ' ' => {}
            c if !inside => plain.push(c),
            _ => {}
        }
    }
    if let Some(thousands) = plain.strip_suffix(['k', 'K']) {
        let value: f64 = thousands.parse().ok()?;
        return (value.is_finite() && value >= 0.0).then(|| (value * 1000.0).round() as u32);
    }
    plain.parse().ok()
}

/// `12:34` or `1:02:03` in seconds. The countdown before the horn (`-0:10`) and anything else
/// is `None`.
pub fn parse_clock(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut total: u32 = 0;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) || (index > 0 && part.len() != 2) {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(part.parse().ok()?)?;
    }
    Some(total)
}

/// A 32-bit account id from a SteamID64 or a bare account id.
pub fn account_id(raw: &str) -> Option<u32> {
    let value: u64 = raw.trim().parse().ok()?;
    let account = if value > STEAM_ID64_BASE { value - STEAM_ID64_BASE } else { value };
    u32::try_from(account).ok().filter(|id| *id > 0)
}

/// When a console line was written, from its `MM/DD HH:MM:SS` stamp in local time, in Unix
/// milliseconds. The stamp has no year: one more than a day ahead of `now` is from last year.
pub fn line_time(line: &str, now: DateTime<Local>) -> Option<i64> {
    let stamp = line.get(..14)?;
    let at = |year: i32| {
        let naive = NaiveDateTime::parse_from_str(&format!("{year}/{stamp}"), "%Y/%m/%d %H:%M:%S").ok()?;
        Local.from_local_datetime(&naive).earliest().map(|at| at.timestamp_millis())
    };
    let this_year = at(now.year())?;
    if this_year - now.timestamp_millis() > 24 * 60 * 60 * 1000 {
        return at(now.year() - 1);
    }
    Some(this_year)
}

// --- the fold ---------------------------------------------------------------------------

/// What the bridge reported since the lobby was created.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Bridge {
    /// The `TeamFriendly` and `TeamEnemy` panels, as the game names them (fixed teams).
    slots: [Vec<LivePlayer>; 2],
    clock_s: Option<u32>,
    reported_at: Option<i64>,
    /// Account ids by player name, from the versus screen.
    accounts: HashMap<String, u32>,
    samples: Vec<Sample>,
}

impl Bridge {
    fn local_slot(&self) -> usize {
        usize::from(self.slots[1].iter().any(|player| player.local))
    }

    fn board(&self) -> Board {
        let local = self.local_slot();
        let named = |slot: usize| -> Vec<LivePlayer> {
            self.slots[slot]
                .iter()
                .map(|player| LivePlayer {
                    account_id: self.accounts.get(&player.name).copied(),
                    ..player.clone()
                })
                .collect()
        };
        Board {
            clock_s: self.clock_s,
            friendly: named(local),
            enemy: named(1 - local),
            team: if local == 0 { Team::HiddenKing } else { Team::Archmother },
            reported_at: self.reported_at,
            samples: self.samples.clone(),
        }
    }

    fn sample(&mut self) {
        let Some(clock_s) = self.clock_s else { return };
        if self.slots.iter().any(Vec::is_empty) {
            return;
        }
        let local = self.local_slot();
        let sample = Sample {
            clock_s,
            mine: self.slots.iter().flatten().find(|player| player.local).and_then(|player| player.souls),
            friendly: Board::souls(&self.slots[local]),
            enemy: Board::souls(&self.slots[1 - local]),
        };
        // Kept points sit about SAMPLE_EVERY_S apart; the newest one always follows the board.
        let n = self.samples.len();
        let last = self.samples.last().map(|s| s.clock_s);
        let before = n.checked_sub(2).map(|i| self.samples[i].clock_s);
        match (last, before) {
            // The clock went back: a new board in the same lobby (a rejoin). Start over.
            (Some(last), _) if clock_s < last => {
                self.samples.clear();
                self.samples.push(sample);
            }
            (Some(last), _) if clock_s == last => self.samples[n - 1] = sample,
            (Some(_), Some(before)) if clock_s - before < SAMPLE_EVERY_S => self.samples[n - 1] = sample,
            _ => self.samples.push(sample),
        }
        if self.samples.len() > SAMPLES_KEEP {
            let extra = self.samples.len() - SAMPLES_KEEP;
            self.samples.drain(..extra);
        }
    }
}

/// Everything the live view knows from one console log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveState {
    /// The presence parser's view: phase, hero codename, party, mode.
    pub session: Session,
    /// When matchmaking started, while still searching. Unix milliseconds.
    pub queued_since: Option<i64>,
    pub current: Option<CurrentMatch>,
    /// Match ids this log saw, newest last.
    pub recent: Vec<u64>,
    /// The last match that started, until the player queues again.
    pub last: Option<FinishedMatch>,
    /// The bridge printed at least one line in this log, so the HUD mod is loaded.
    pub bridge_heard: bool,
    bridge: Option<Bridge>,
}

impl LiveState {
    /// The live board of the open lobby, once the bridge reported.
    pub fn board(&self) -> Option<Board> {
        self.bridge.as_ref().filter(|bridge| bridge.slots.iter().any(|slot| !slot.is_empty())).map(Bridge::board)
    }

    /// Account ids the versus screen tied to names in this lobby.
    pub fn roster_accounts(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.bridge.iter().flat_map(|bridge| bridge.accounts.values().copied()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Fold one console line. Returns true when something the live view shows changed.
    pub fn read_line(&mut self, line: &str, now: DateTime<Local>) -> bool {
        let before = self.clone();
        self.fold(line, now);
        before != *self
    }

    fn fold(&mut self, line: &str, now: DateTime<Local>) {
        // A bridge line carries player names: read it before anything that matches on text.
        if let Some(json) = after(line, ROSTER_TAG) {
            self.bridge_heard = true;
            if let Some(roster) = versioned::<Roster>(json) {
                let accounts = roster
                    .teams
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| Some((entry.name?, account_id(&entry.id?)?)))
                    .collect();
                self.bridge.get_or_insert_with(Bridge::default).accounts = accounts;
            }
            return;
        }
        if let Some(json) = after(line, BRIDGE_TAG) {
            self.bridge_heard = true;
            if let Some(report) = versioned::<Report>(json) {
                let bridge = self.bridge.get_or_insert_with(Bridge::default);
                let slot = match report.side {
                    Side::Friendly => 0,
                    Side::Enemy => 1,
                };
                bridge.slots[slot] = report.players.into_iter().map(LivePlayer::from).collect();
                if let Some(clock) = report.clock.as_deref().and_then(parse_clock) {
                    bridge.clock_s = Some(clock);
                }
                if let Some(at) = line_time(line, now) {
                    bridge.reported_at = Some(at);
                }
                bridge.sample();
            }
            return;
        }
        game_session::read_line(&mut self.session, line);
        let rules = rules();
        if let Some(c) = rules.state.captures(line) {
            let number = c[2].parse().unwrap_or(0);
            if let Some(current) = self.current.as_mut() {
                let phase = MatchPhase::from_state(&c[1], number);
                if phase == MatchPhase::InProgress && current.phase != MatchPhase::InProgress {
                    current.started_at = line_time(line, now).or(current.started_at);
                }
                current.phase = phase;
            }
        } else if line.contains("k_EMsgClientToGCStartMatchmaking") {
            self.queued_since = Some(line_time(line, now).unwrap_or_else(|| now.timestamp_millis()));
            self.last = None;
        } else if line.contains("k_EMsgClientToGCStopMatchmaking") {
            self.queued_since = None;
        } else if let Some(c) = rules.lobby.captures(line) {
            let (Ok(lobby_id), Ok(match_id)) = (c[1].parse::<u64>(), c[2].parse::<u64>()) else {
                return;
            };
            if c[3].eq_ignore_ascii_case("created") {
                if !self.recent.contains(&match_id) {
                    self.recent.push(match_id);
                    if self.recent.len() > RECENT_KEEP {
                        self.recent.remove(0);
                    }
                }
                self.queued_since = None;
                self.last = None;
                self.bridge = None;
                self.current = Some(CurrentMatch {
                    match_id,
                    lobby_id,
                    phase: MatchPhase::Pregame,
                    created_at: line_time(line, now),
                    started_at: None,
                });
            } else if let Some(current) = self.current.take_if(|current| current.lobby_id == lobby_id) {
                // A lobby left in hero select never became a match.
                if current.phase != MatchPhase::Pregame {
                    self.last = Some(FinishedMatch {
                        match_id: current.match_id,
                        board: self.board(),
                        started_at: current.started_at,
                        ended_at: line_time(line, now),
                    });
                }
            }
        } else if line.contains("Source2Shutdown") || line.contains("EventAppShutdown_t") {
            // The game closed with a lobby open: no teardown line follows.
            if let Some(current) = self.current.take()
                && current.phase != MatchPhase::Pregame
            {
                self.last = Some(FinishedMatch {
                    match_id: current.match_id,
                    board: self.board(),
                    started_at: current.started_at,
                    ended_at: line_time(line, now),
                });
            }
            self.queued_since = None;
        }
    }
}

struct Rules {
    state: regex::Regex,
    lobby: regex::Regex,
}

fn rules() -> &'static Rules {
    static RULES: std::sync::OnceLock<Rules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| Rules {
        state: regex::Regex::new(r"(?:ChangeGameState|OnGameStateChanged):\s+(\w+)\s+\((\d+)\)").unwrap(),
        lobby: regex::Regex::new(r"Lobby (\d+) for Match (\d+) (created|destroyed)").unwrap(),
    })
}

/// Fold a block of console text in order. Returns true when anything changed.
pub fn fold_text(state: &mut LiveState, text: &str, now: DateTime<Local>) -> bool {
    let mut changed = false;
    for line in text.lines() {
        changed |= state.read_line(line, now);
    }
    changed
}

/// [`fold_text`] at the current time.
pub fn fold_text_now(state: &mut LiveState, text: &str) -> bool {
    fold_text(state, text, Local::now())
}

/// Unix milliseconds now, on the same clock the stamps are read with.
pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

/// The console stamp the game would write `ago` seconds before now (scenes and tests).
pub fn stamp_ago(ago: i64) -> String {
    (Local::now() - chrono::Duration::seconds(ago)).format("%m/%d %H:%M:%S").to_string()
}

// --- following the log ------------------------------------------------------------------

/// Follows console.log by offset: the first poll reads up to [`REPLAY_BYTES`] from the end
/// (so opening Sigil in the middle of a match still finds its lobby), later polls only what was
/// appended. A log that shrank or was created again (the game restarted) starts over.
#[derive(Debug, Clone, Default)]
pub struct LiveTail {
    pub state: LiveState,
    /// console.log exists.
    pub found: bool,
    /// When the log last changed.
    pub modified: Option<SystemTime>,
    path: Option<PathBuf>,
    created: Option<SystemTime>,
    offset: u64,
}

impl LiveTail {
    /// A tail over a log that exists, already folded to `state` (scenes and tests).
    pub fn with_state(state: LiveState) -> LiveTail {
        LiveTail { state, found: true, ..LiveTail::default() }
    }

    /// Read what is new. Returns true when anything the live view shows changed.
    pub fn poll(&mut self, path: &Path) -> bool {
        self.poll_at(path, Local::now())
    }

    pub fn poll_at(&mut self, path: &Path, now: DateTime<Local>) -> bool {
        let Ok(meta) = fs::metadata(path) else {
            let was = self.found;
            *self = LiveTail::default();
            return was;
        };
        let len = meta.len();
        let created = meta.created().ok();
        let modified = meta.modified().ok();
        let mut changed = !self.found || self.modified != modified;
        self.found = true;
        self.modified = modified;
        if self.path.as_deref() != Some(path) || self.created != created || len < self.offset {
            changed |= self.state != LiveState::default();
            self.state = LiveState::default();
            self.path = Some(path.to_path_buf());
            self.created = created;
            self.offset = 0;
        }
        if len == self.offset {
            return changed;
        }
        let Ok(mut file) = fs::File::open(path) else {
            return changed;
        };
        let start = if self.offset == 0 { len.saturating_sub(REPLAY_BYTES) } else { self.offset };
        if file.seek(SeekFrom::Start(start)).is_err() {
            return changed;
        }
        let mut bytes = Vec::new();
        if file.take(len - start).read_to_end(&mut bytes).is_err() {
            return changed;
        }
        // Whole lines only; a half-written last line waits for the next poll.
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
            return changed;
        };
        // A replay that starts mid-file starts at the next whole line.
        let from = if self.offset == 0 && start > 0 {
            bytes.iter().position(|b| *b == b'\n').map_or(end + 1, |at| at + 1)
        } else {
            0
        };
        self.offset = start + end as u64 + 1;
        if from > end {
            return changed;
        }
        let text = String::from_utf8_lossy(&bytes[from..end]);
        changed | fold_text(&mut self.state, &text, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATED: &str = "11/20 20:03:37 Lobby 174072581626958477 for Match 96996563 created";
    const DESTROYED: &str = "11/20 20:35:54 Lobby 174072581626958477 for Match 96996563 destroyed";
    const QUEUED: &str = "11/20 20:01:21 [GCClient] Send msg 9010 (k_EMsgClientToGCStartMatchmaking), 263 bytes";
    const STARTED: &str = "11/20 20:05:00 OnGameStateChanged: GameInProgress (7)";

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 20, 20, 40, 0).unwrap()
    }

    fn fold(text: &str) -> LiveState {
        let mut state = LiveState::default();
        fold_text(&mut state, text, now());
        state
    }

    fn report(side: &str, clock: &str, players: &str) -> String {
        format!(
            "11/20 20:20:02 [PanoramaScript] {BRIDGE_TAG} {{\"v\":1,\"side\":\"{side}\",\"clock\":\"{clock}\",\"players\":[{players}]}}"
        )
    }

    const ME: &str = r#"{"name":"Wren","hero":"Haze","hero_id":13,"souls":"12.3<span class=\"demote\">k</span>","level":14,"k":5,"d":1,"a":7,"obj":4210,"me":true,"dead":false}"#;
    const MATE: &str = r#"{"name":"Ivo","hero":"Abrams","hero_id":6,"souls":"9,870","level":12,"k":2,"d":3,"a":4,"obj":null,"me":false,"dead":true}"#;
    const RIVAL: &str = r#"{"name":"Rival","hero":"Seven","souls":"11k","level":13,"k":4,"d":2,"a":1,"obj":800,"me":false,"dead":false}"#;

    #[test]
    fn finds_the_open_lobby_and_drops_it_at_teardown() {
        let state = fold(&format!("noise\n{CREATED}\nmore noise\n"));
        let current = state.current.clone().unwrap();
        assert_eq!(current.match_id, 96_996_563);
        assert_eq!(current.lobby_id, 174_072_581_626_958_477);
        assert_eq!(current.phase, MatchPhase::Pregame);
        assert!(fold(&format!("{CREATED}\n{DESTROYED}\n")).current.is_none());
    }

    #[test]
    fn a_teardown_of_another_lobby_keeps_the_match() {
        let state = fold(&format!("{CREATED}\n11/20 20:10:00 Lobby 9 for Match 222 destroyed\n"));
        assert_eq!(state.current.unwrap().match_id, 96_996_563);
    }

    #[test]
    fn state_carries_across_polls_so_a_long_match_is_not_lost() {
        let mut state = fold(CREATED);
        fold_text(&mut state, &"engine noise\n".repeat(20_000), now());
        assert!(state.current.is_some());
        fold_text(&mut state, STARTED, now());
        assert_eq!(state.current.as_ref().unwrap().phase, MatchPhase::InProgress);
        assert!(state.current.as_ref().unwrap().started_at.is_some());
        fold_text(&mut state, DESTROYED, now());
        assert!(state.current.is_none());
        let last = state.last.unwrap();
        assert_eq!(last.match_id, 96_996_563);
        assert_eq!(last.duration_s(), Some(30 * 60 + 54));
    }

    #[test]
    fn queue_starts_stops_and_ends_at_the_lobby() {
        let queued = fold(QUEUED);
        assert_eq!(queued.queued_since, line_time(QUEUED, now()));
        assert!(fold(&format!("{QUEUED}\n[GCClient] Send msg 9012 (k_EMsgClientToGCStopMatchmaking), 8 bytes\n")).queued_since.is_none());
        assert!(fold(&format!("{QUEUED}\n{CREATED}\n")).queued_since.is_none());
    }

    #[test]
    fn the_hideout_states_do_not_make_a_match() {
        let state = fold("04:25:53 ChangeGameState: HeroSelection (3)\n04:25:53 ChangeGameState: GameInProgress (7)\n");
        assert!(state.current.is_none());
        let state = fold(&format!("{CREATED}\n11/20 20:04:07 OnGameStateChanged: MatchIntro (4)\n"));
        assert_eq!(state.current.unwrap().phase, MatchPhase::Pregame);
    }

    #[test]
    fn reads_both_teams_with_the_player_first() {
        let state = fold(&format!(
            "{CREATED}\n{STARTED}\n{}\n{}\n",
            report("friendly", "12:34", &format!("{ME},{MATE}")),
            report("enemy", "12:36", RIVAL)
        ));
        let board = state.board().unwrap();
        assert_eq!(board.clock_s, Some(12 * 60 + 36));
        assert_eq!(board.team, Team::HiddenKing);
        let me = board.me().unwrap();
        assert_eq!(me.name, "Wren");
        assert_eq!(me.souls, Some(12_300));
        assert_eq!(me.hero_id, Some(13));
        assert_eq!(me.objective_damage, Some(4210));
        assert_eq!(board.friendly[1].souls, Some(9_870));
        assert!(board.friendly[1].dead);
        assert_eq!(board.friendly[1].objective_damage, None);
        assert_eq!(board.enemy[0].souls, Some(11_000));
        assert_eq!(board.lead(), 12_300 + 9_870 - 11_000);
        assert_eq!(Board::kills(&board.friendly), 7);
        assert_eq!(board.souls_per_minute(), Some(12_300 * 60 / 756));
        assert!(state.bridge_heard);
        assert!(!board.stale(line_time(&report("enemy", "0:00", ""), now()).unwrap() + 1_000));
    }

    #[test]
    fn the_player_in_the_enemy_panel_is_still_friendly() {
        let state = fold(&format!(
            "{}\n{}\n",
            report("friendly", "3:00", MATE),
            report("enemy", "3:00", ME)
        ));
        let board = state.board().unwrap();
        assert_eq!(board.friendly[0].name, "Wren");
        assert_eq!(board.enemy[0].name, "Ivo");
        assert_eq!(board.team, Team::Archmother);
    }

    #[test]
    fn the_roster_names_accounts_even_before_the_top_bar() {
        let roster = format!(
            "11/20 20:04:01 [PanoramaScript] {ROSTER_TAG} {{\"v\":1,\"teams\":[[{{\"name\":\"Wren\",\"id\":\"76561198276744320\"}},{{\"name\":\"Bot\",\"id\":null}}],[{{\"name\":\"Rival\",\"id\":\"1234\"}}]]}}"
        );
        let state = fold(&format!(
            "{CREATED}\n{roster}\n{}\n{}\n",
            report("friendly", "1:00", ME),
            report("enemy", "1:00", RIVAL)
        ));
        let board = state.board().unwrap();
        assert_eq!(board.friendly[0].account_id, Some(316_478_592));
        assert_eq!(board.enemy[0].account_id, Some(1234));
        assert_eq!(state.roster_accounts(), vec![1234, 316_478_592]);
    }

    #[test]
    fn unreadable_and_other_version_lines_are_skipped() {
        let state = fold(&format!(
            "{}\n{BRIDGE_TAG} not json\n{BRIDGE_TAG} {{\"v\":9,\"side\":\"friendly\",\"players\":[]}}\n{BRIDGE_TAG} {{\"v\":1,\"side\":\"renamed\",\"players\":[]}}\n",
            report("friendly", "1:00", ME)
        ));
        assert_eq!(state.board().unwrap().friendly.len(), 1);
    }

    #[test]
    fn a_new_lobby_starts_an_empty_board_and_the_last_match_stays_until_the_queue() {
        let played = fold(&format!(
            "{CREATED}\n{STARTED}\n{}\n{}\n{DESTROYED}\n",
            report("friendly", "30:54", ME),
            report("enemy", "30:54", RIVAL)
        ));
        let last = played.last.clone().unwrap();
        assert_eq!(last.board.as_ref().unwrap().me().unwrap().hero.as_deref(), Some("Haze"));
        assert_eq!(last.duration_s(), Some(30 * 60 + 54));
        let mut next = played.clone();
        fold_text(&mut next, QUEUED, now());
        assert!(next.last.is_none());
        let mut lobby = played;
        fold_text(&mut lobby, "11/20 21:00:00 Lobby 2 for Match 333 created", now());
        assert!(lobby.board().is_none());
        assert_eq!(lobby.recent, vec![96_996_563, 333]);
    }

    #[test]
    fn a_lobby_left_in_hero_select_is_not_kept() {
        let dodged = fold(&format!("{CREATED}\n{}\n{DESTROYED}\n", report("friendly", "0:00", ME)));
        assert!(dodged.last.is_none());
    }

    #[test]
    fn souls_are_sampled_along_the_clock() {
        let mut text = format!("{CREATED}\n{STARTED}\n");
        for second in (0..=120).step_by(2) {
            let clock = format!("{}:{:02}", second / 60, second % 60);
            let me = ME.replace("12.3<span class=\\\"demote\\\">k</span>", &format!("{}", 500 + second * 10));
            text.push_str(&report("friendly", &clock, &me));
            text.push('\n');
            text.push_str(&report("enemy", &clock, RIVAL));
            text.push('\n');
        }
        let board = fold(&text).board().unwrap();
        assert!((12..=17).contains(&board.samples.len()), "{}", board.samples.len());
        assert!(board.samples.windows(2).all(|pair| pair[0].clock_s < pair[1].clock_s));
        let last = board.samples.last().unwrap();
        assert_eq!(last.clock_s, 120);
        assert_eq!(last.mine, Some(1_700));
        assert_eq!(last.lead(), 1_700 - 11_000);
    }

    #[test]
    fn parses_the_top_bar_numbers() {
        assert_eq!(parse_souls("980"), Some(980));
        assert_eq!(parse_souls("9,870"), Some(9_870));
        assert_eq!(parse_souls("12.3k"), Some(12_300));
        assert_eq!(parse_souls("?"), None);
        assert_eq!(parse_clock("12:34"), Some(754));
        assert_eq!(parse_clock("1:02:03"), Some(3723));
        assert_eq!(parse_clock("-0:10"), None);
        assert_eq!(parse_clock("12:3"), None);
        assert_eq!(account_id("76561198276744320"), Some(316_478_592));
        assert_eq!(account_id("0"), None);
    }

    #[test]
    fn a_stamp_from_late_december_read_in_january_is_last_year() {
        let jan = Local.with_ymd_and_hms(2027, 1, 1, 0, 0, 5).unwrap();
        let eve = Local.with_ymd_and_hms(2026, 12, 31, 23, 59, 58).unwrap();
        assert_eq!(line_time("12/31 23:59:58 x", jan), Some(eve.timestamp_millis()));
        assert_eq!(line_time("04:25:53 Change", jan), None);
    }

    #[test]
    fn the_tail_reads_new_lines_and_starts_over_on_a_new_log() {
        let dir = std::env::temp_dir().join(format!("sigil-live-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("console.log");
        fs::write(&path, format!("{CREATED}\n{STARTED}\nhalf a li")).unwrap();
        let mut tail = LiveTail::default();
        assert!(tail.poll_at(&path, now()));
        assert!(tail.found);
        assert_eq!(tail.state.current.as_ref().unwrap().phase, MatchPhase::InProgress);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, format!("ne\n{DESTROYED}\n").as_bytes()).unwrap();
        drop(file);
        tail.poll_at(&path, now());
        assert!(tail.state.current.is_none());
        assert_eq!(tail.state.last.as_ref().unwrap().match_id, 96_996_563);
        fs::write(&path, "short\n").unwrap();
        tail.poll_at(&path, now());
        assert!(tail.state.last.is_none());
        fs::remove_file(&path).unwrap();
        assert!(tail.poll_at(&path, now()));
        assert!(!tail.found);
        let _ = fs::remove_dir_all(&dir);
    }
}
