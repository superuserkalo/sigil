//! What Sigil learns about a game session from files the game and Steam write: the phase and
//! hero from `game/citadel/console.log` (written when the game runs with `-condebug`), crash
//! dumps that appeared during a session, the exit code Steam logs, and the `gameinfo.gi`
//! search path Sigil adds and can take out again when the game closes.
//!
//! The console rules follow Deadlock Mod Manager's presence watcher (GPL-3.0) in behaviour
//! only: the log lines it reacts to are the game's, and this parser is written from scratch.
//!
//! GAME WRITE: [`remove_addons_path`] edits `gameinfo.gi` (with a stamped backup first).

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use regex::Regex;

use crate::deploy::{GAMEINFO_ADDONS_LINE, gameinfo_path};
use crate::error::{Error, Result};

/// Where the game writes its console with `-condebug`.
pub fn console_log_path(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("console.log")
}

/// The launch option that makes the game write console.log.
pub const CONDEBUG: &str = "-condebug";

/// Add or remove `-condebug` in a launch options string, keeping the rest as typed.
pub fn with_condebug(options: &str, on: bool) -> String {
    let mut words: Vec<&str> = options
        .split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case(CONDEBUG))
        .collect();
    if on {
        words.insert(0, CONDEBUG);
    }
    words.join(" ")
}

pub fn has_condebug(options: &str) -> bool {
    options.split_whitespace().any(|w| w.eq_ignore_ascii_case(CONDEBUG))
}

// --- phases ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    NotRunning,
    MainMenu,
    Hideout,
    InQueue,
    MatchIntro,
    InMatch,
    PostMatch,
    Spectating,
}

/// What the console says right now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    pub phase: Phase,
    /// Hero codename (`hero_inferno`) once one is picked this match.
    pub hero: Option<String>,
    /// People in the party, the player included. 1 when solo.
    pub party: u8,
    /// Players seen in the match at most, bots included, for the mode.
    pub players: u8,
    pub bots: bool,
    pub street_brawl: bool,
}

impl Session {
    /// The template key for this phase, as Settings names them.
    pub fn template_key(&self) -> &'static str {
        let party = self.party > 1;
        match self.phase {
            Phase::NotRunning | Phase::MainMenu => "menu",
            Phase::Hideout if party => "party_hideout",
            Phase::Hideout => "hideout",
            Phase::InQueue => "queue",
            Phase::MatchIntro | Phase::InMatch if party => "party_match",
            Phase::MatchIntro | Phase::InMatch => "match",
            Phase::PostMatch => "post_match",
            Phase::Spectating => "spectating",
        }
    }

    /// The match mode in words, for `{mode}`.
    pub fn mode(&self) -> &'static str {
        if self.bots {
            "Bot match"
        } else if self.street_brawl {
            "Street Brawl"
        } else if matches!(self.phase, Phase::MatchIntro | Phase::InMatch | Phase::PostMatch) {
            "Standard (6v6)"
        } else {
            ""
        }
    }

    pub fn party_max(&self) -> u8 {
        if self.street_brawl { 4 } else { 6 }
    }

    fn in_match(&self) -> bool {
        matches!(self.phase, Phase::MatchIntro | Phase::InMatch)
    }

    fn new_match(&mut self) {
        self.hero = None;
        self.players = 0;
        self.bots = false;
        self.street_brawl = false;
    }
}

struct Rules {
    state: Regex,
    map: Regex,
    lobby: Regex,
    connected: Regex,
    disconnect: Regex,
    hero: Regex,
    hero_model: Regex,
    players: Regex,
    party_size: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        state: Regex::new(r"(?i)(?:ChangeGameState|OnGameStateChanged):\s+(\w+)\s+\((\d+)\)").unwrap(),
        map: Regex::new(r#"(?i)\[Client\] (?:Map:\s+"([^"]+)"|Created physics for\s+(\S+))"#).unwrap(),
        lobby: Regex::new(r"(?i)Lobby \d+ for Match \d+ (created|destroyed)").unwrap(),
        connected: Regex::new(r"(?i)\[Client\] CL: Connected to '([^']+)'").unwrap(),
        disconnect: Regex::new(r"(?i)\[Client\] Disconnecting from server:\s*(\S+)").unwrap(),
        hero: Regex::new(r"(?i)\[Server\] Loaded hero \d+/(hero_\w+)").unwrap(),
        hero_model: Regex::new(r"(?i)models/heroes(?:_wip|_staging)?/(\w+)/").unwrap(),
        players: Regex::new(r"(?i)\[Client\] Players:\s+(\d+)\s+\((\d+) bots\)").unwrap(),
        party_size: Regex::new(r"(?i)party.{0,40}?(?:members|size)[:=\s]+(\d+)").unwrap(),
    })
}

/// Feed one console line. Returns true when something a presence would show changed.
pub fn read_line(session: &mut Session, line: &str) -> bool {
    let before = session.clone();
    let r = rules();
    if line.contains("Source2Shutdown") || line.contains("EventAppShutdown_t") || line.contains("Server shutting down: EXITING") {
        *session = Session::default();
        return before != *session;
    }
    if session.phase == Phase::NotRunning {
        session.phase = Phase::MainMenu;
    }
    if let Some(c) = r.map.captures(line) {
        let map = c.get(1).or_else(|| c.get(2)).map(|m| m.as_str().to_ascii_lowercase()).unwrap_or_default();
        let map = map.trim_end_matches(".vpk").rsplit(['/', '\\']).next().unwrap_or("").to_string();
        if map.starts_with("dl_hideout") {
            session.phase = Phase::Hideout;
            session.new_match();
        } else if map.starts_with("street_test") {
            session.street_brawl = true;
            session.phase = Phase::MatchIntro;
        } else if !map.is_empty() && map != "<empty>" && session.phase != Phase::Spectating {
            if !session.in_match() {
                session.new_match();
            }
            session.phase = Phase::MatchIntro;
        }
    } else if line.contains("k_EMsgClientToGCStartMatchmaking") {
        if matches!(session.phase, Phase::Hideout | Phase::MainMenu) {
            session.phase = Phase::InQueue;
        }
    } else if line.contains("k_EMsgClientToGCStopMatchmaking") {
        if session.phase == Phase::InQueue {
            session.phase = Phase::Hideout;
        }
    } else if let Some(c) = r.state.captures(line) {
        let name = c[1].to_ascii_lowercase();
        let number: u32 = c[2].parse().unwrap_or(0);
        if !matches!(session.phase, Phase::Hideout | Phase::Spectating) {
            if name == "matchintro" || number == 4 {
                session.phase = Phase::MatchIntro;
            } else if name == "gameinprogress" || number == 7 {
                session.phase = Phase::InMatch;
            } else if name.starts_with("postgame") || (8..=11).contains(&number) {
                session.phase = Phase::PostMatch;
            }
        }
    } else if let Some(c) = r.lobby.captures(line) {
        if c[1].eq_ignore_ascii_case("created") {
            session.new_match();
            session.phase = Phase::MatchIntro;
        } else if session.in_match() {
            session.phase = Phase::PostMatch;
        }
    } else if let Some(c) = r.connected.captures(line) {
        let addr = &c[1];
        if !addr.contains("loopback") && !addr.starts_with("127.0.0.1") && !session.in_match() {
            session.new_match();
            session.phase = Phase::MatchIntro;
        }
    } else if let Some(c) = r.disconnect.captures(line) {
        let reason = c[1].to_ascii_uppercase();
        if reason.contains("EXITING") {
            *session = Session::default();
        } else if !reason.contains("LOOPDEACTIVATE") && session.in_match() {
            session.phase = Phase::PostMatch;
        }
    } else if line.contains("LoopMode: menu") || line.contains("LoopMode:menu") {
        if session.in_match() {
            session.phase = Phase::PostMatch;
        }
    } else if line.contains("Playing Broadcast") {
        session.phase = Phase::Spectating;
    } else if let Some(c) = r.hero.captures(line) {
        if session.hero.is_none() && !matches!(session.phase, Phase::MainMenu | Phase::PostMatch) {
            session.hero = Some(c[1].to_ascii_lowercase());
        }
    } else if line.contains("VMDL Camera Pose Success") {
        if let Some(c) = r.hero_model.captures(line)
            && session.hero.is_none()
            && !matches!(session.phase, Phase::MainMenu | Phase::PostMatch)
        {
            session.hero = Some(c[1].to_ascii_lowercase());
        }
    } else if line.contains("k_ECitadelBotDifficulty_") {
        session.bots = true;
    } else if let Some(c) = r.players.captures(line) {
        let players: u8 = c[1].parse().unwrap_or(0);
        let bots: u8 = c[2].parse().unwrap_or(0);
        if session.phase == Phase::Hideout {
            session.party = players.clamp(1, 6);
        } else {
            session.players = session.players.max(players);
            if bots > 1 {
                session.bots = true;
            } else if (5..=8).contains(&session.players) {
                session.street_brawl = true;
            } else if session.players > 8 {
                session.street_brawl = false;
            }
        }
    } else if let Some(c) = r.party_size.captures(line) {
        session.party = c[1].parse::<u8>().unwrap_or(1).clamp(1, 6);
    }
    if session.party == 0 {
        session.party = 1;
    }
    before != *session
}

/// Follows console.log by offset, so each poll reads only the new lines. A file that got
/// shorter (the game started again) is read from the start.
#[derive(Debug, Clone, Default)]
pub struct Tail {
    pub offset: u64,
    pub session: Session,
}

/// Bytes read from the end of an old log when following starts.
pub const REPLAY_BYTES: u64 = 4 * 1024 * 1024;

impl Tail {
    /// Read what is new. Returns true when the session changed.
    pub fn poll(&mut self, path: &Path) -> bool {
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            self.offset = 0;
            self.session = Session::default();
        }
        if self.offset == 0 && len > REPLAY_BYTES {
            self.offset = len - REPLAY_BYTES;
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut bytes = Vec::new();
        let _ = file.take(len.saturating_sub(self.offset)).read_to_end(&mut bytes);
        // Only whole lines; a half-written last line waits for the next poll.
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
            return false;
        };
        self.offset += end as u64 + 1;
        let text = String::from_utf8_lossy(&bytes[..end]);
        let mut changed = false;
        for line in text.lines() {
            changed |= read_line(&mut self.session, line);
        }
        changed
    }
}

// --- crashes --------------------------------------------------------------------------------

/// Folders the game and Steam write crash dumps to.
pub fn dump_folders(game_root: Option<&Path>, steam_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(game) = game_root {
        out.push(game.join("game").join("bin").join("win64"));
    }
    for root in steam_roots {
        out.push(root.join("dumps"));
    }
    out
}

/// One crash dump on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dump {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    /// `game` or `steam`.
    pub source: &'static str,
}

/// Every `.dmp` / `.mdmp` in the folders, newest first.
pub fn list_dumps(folders: &[PathBuf]) -> Vec<Dump> {
    let mut out = Vec::new();
    for folder in folders {
        let source = if folder.ends_with("dumps") { "steam" } else { "game" };
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dump = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mdmp") || e.eq_ignore_ascii_case("dmp"));
            if !is_dump {
                continue;
            }
            let meta = entry.metadata().ok();
            out.push(Dump {
                bytes: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()),
                source,
                path,
            });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    out
}

/// Dumps written at or after `since` (with a few seconds of slack), newest first.
pub fn dumps_since(dumps: &[Dump], since: SystemTime) -> Vec<Dump> {
    let since = since.checked_sub(std::time::Duration::from_secs(5)).unwrap_or(since);
    dumps
        .iter()
        .filter(|d| d.modified.is_some_and(|m| m >= since))
        .cloned()
        .collect()
}

/// A minidump's exception code and the readable comment the game leaves in it, when there is
/// one. Reads the stream directory only, never the memory it saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DumpInfo {
    pub exception: Option<u32>,
    pub comment: Option<String>,
}

pub fn read_dump_info(path: &Path) -> DumpInfo {
    let Ok(bytes) = fs::read(path) else {
        return DumpInfo::default();
    };
    parse_minidump(&bytes)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The header is `MDMP`, version, stream count, directory offset; each directory entry is
/// type, size, offset. Stream 6 holds the exception record, 10 and 11 comments.
pub fn parse_minidump(b: &[u8]) -> DumpInfo {
    let mut info = DumpInfo::default();
    if b.get(0..4) != Some(b"MDMP") {
        return info;
    }
    let (Some(count), Some(dir)) = (u32_at(b, 8), u32_at(b, 12)) else {
        return info;
    };
    for i in 0..count.min(256) as usize {
        let at = dir as usize + i * 12;
        let (Some(kind), Some(size), Some(offset)) = (u32_at(b, at), u32_at(b, at + 4), u32_at(b, at + 8)) else {
            break;
        };
        let (size, offset) = (size as usize, offset as usize);
        match kind {
            // MINIDUMP_EXCEPTION_STREAM: thread id, alignment, then ExceptionCode.
            6 => info.exception = u32_at(b, offset + 8),
            10 => {
                if let Some(s) = b.get(offset..offset + size) {
                    let text = String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string();
                    if !text.is_empty() {
                        info.comment = Some(text.chars().take(4000).collect());
                    }
                }
            }
            11 => {
                if let Some(s) = b.get(offset..offset + size) {
                    let units: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    let text = String::from_utf16_lossy(&units).trim_end_matches('\0').trim().to_string();
                    if !text.is_empty() && info.comment.is_none() {
                        info.comment = Some(text.chars().take(4000).collect());
                    }
                }
            }
            _ => {}
        }
    }
    info
}

/// A short name for an exception code.
pub fn exception_label(code: u32) -> String {
    match code {
        0xC000_0005 => "Access violation".into(),
        0xC000_00FD => "Stack overflow".into(),
        0xC000_0409 => "Stack buffer overrun".into(),
        0x8000_0003 => "Breakpoint".into(),
        other => format!("Exception 0x{other:08X}"),
    }
}

/// The last exit code Steam logged for Deadlock in `logs/gameprocess_log.txt`.
pub fn steam_exit_code(log: &str) -> Option<i64> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"AppID 1422450 no longer tracking PID \d+, exit code (-?\d+)").unwrap()
    });
    re.captures_iter(log).last().and_then(|c| c[1].parse().ok())
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Crash,
    Clean,
    Unknown,
}

/// A new dump, or an exit code in the NTSTATUS error range, is a crash. Exit 0, or the
/// console's shutdown lines, is clean.
pub fn classify(new_dumps: usize, exit_code: Option<i64>, console_clean: bool) -> Ending {
    let error_code = exit_code.is_some_and(|c| (c as u32) & 0xF000_0000 == 0xC000_0000);
    if new_dumps > 0 || error_code {
        Ending::Crash
    } else if exit_code == Some(0) || console_clean {
        Ending::Clean
    } else {
        Ending::Unknown
    }
}

/// Whether the end of console.log shows the game shut down on purpose.
pub fn console_ended_cleanly(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else { return false };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(8192)));
    let mut tail = String::new();
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    tail.push_str(&String::from_utf8_lossy(&bytes));
    tail.contains("ShutdownSource2Logging") || tail.contains("Source2Shutdown")
}

// --- gameinfo.gi ----------------------------------------------------------------------------

/// `gameinfo.gi` text without Sigil's addons search path. None when there is no such line.
pub fn without_addons_line(text: &str) -> Option<String> {
    let mut removed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let bare = line.split("//").next().unwrap_or("").trim();
        let words: Vec<&str> = bare.split_whitespace().collect();
        let target: Vec<&str> = GAMEINFO_ADDONS_LINE.split_whitespace().collect();
        if words.len() == 2 && words[0].eq_ignore_ascii_case(target[0]) && words[1].eq_ignore_ascii_case(target[1]) {
            removed = true;
            continue;
        }
        out.push_str(line);
    }
    removed.then_some(out)
}

/// GAME WRITE. Take the `Game citadel/addons` line out of gameinfo.gi, so the game starts
/// without mods until it is put back (Repair, or the next modded launch). A stamped backup is
/// written first and the edit is atomic. Returns false when the line was not there.
pub fn remove_addons_path(game_root: &Path, approve: bool) -> Result<bool> {
    if !approve {
        return Err(Error::msg("Not approved: edit gameinfo.gi. Nothing was changed."));
    }
    let path = gameinfo_path(game_root);
    let text = fs::read_to_string(&path)?;
    let Some(updated) = without_addons_line(&text) else {
        return Ok(false);
    };
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S-%3fZ");
    let backup = path.with_file_name(format!("gameinfo.gi{}{stamp}", crate::backups::STAMPED_MARKER));
    fs::write(&backup, &text)?;
    crate::util::write_atomic(&path, updated.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(lines: &[&str]) -> Session {
        let mut s = Session::default();
        for l in lines {
            read_line(&mut s, l);
        }
        s
    }

    #[test]
    fn condebug_is_added_once_and_removed() {
        assert_eq!(with_condebug("-novid +fps_max 240", true), "-condebug -novid +fps_max 240");
        assert_eq!(with_condebug("-condebug -novid", true), "-condebug -novid");
        assert_eq!(with_condebug("-novid -CONDEBUG", false), "-novid");
        assert!(has_condebug("-high -condebug"));
    }

    #[test]
    fn a_match_goes_hideout_queue_intro_match_post() {
        let s = feed(&[r#"[Client] Map: "dl_hideout""#]);
        assert_eq!(s.phase, Phase::Hideout);
        assert_eq!(s.template_key(), "hideout");
        let s = feed(&[
            r#"[Client] Map: "dl_hideout""#,
            "[GCClient] Send msg 9010 (k_EMsgClientToGCStartMatchmaking)",
        ]);
        assert_eq!(s.phase, Phase::InQueue);
        let s = feed(&[
            r#"[Client] Map: "dl_hideout""#,
            "[GCClient] Send msg 9010 (k_EMsgClientToGCStartMatchmaking)",
            r#"[Client] Map: "dl_midtown""#,
            "[Server] Loaded hero 3/hero_inferno",
            "ChangeGameState: GameInProgress (7)",
        ]);
        assert_eq!(s.phase, Phase::InMatch);
        assert_eq!(s.hero.as_deref(), Some("hero_inferno"));
        assert_eq!(s.mode(), "Standard (6v6)");
        let mut s = s;
        read_line(&mut s, "ChangeGameState: PostGame (9)");
        assert_eq!(s.phase, Phase::PostMatch);
        read_line(&mut s, r#"[Client] Map: "dl_hideout""#);
        assert_eq!(s.hero, None, "a new match forgets the hero");
        read_line(&mut s, "Source2Shutdown");
        assert_eq!(s.phase, Phase::NotRunning);
    }

    #[test]
    fn party_and_modes() {
        let s = feed(&[r#"[Client] Map: "dl_hideout""#, "[Client] Players: 3 (0 bots) / 3 humans"]);
        assert_eq!(s.party, 3);
        assert_eq!(s.template_key(), "party_hideout");
        let s = feed(&[
            r#"[Client] Map: "dl_midtown""#,
            "Initializing bot for player slot 4: k_ECitadelBotDifficulty_Easy",
        ]);
        assert_eq!(s.mode(), "Bot match");
        let s = feed(&[r#"[Client] Map: "street_test""#]);
        assert_eq!(s.mode(), "Street Brawl");
        assert_eq!(s.party_max(), 4);
        let s = feed(&["Playing Broadcast for match 123"]);
        assert_eq!(s.template_key(), "spectating");
        let s = feed(&[r#"[Client] Map: "dl_hideout""#, "VMDL Camera Pose Success! models/heroes_staging/haze/haze.vmdl"]);
        assert_eq!(s.hero.as_deref(), Some("haze"));
    }

    #[test]
    fn tail_reads_only_new_whole_lines() {
        let dir = std::env::temp_dir().join(format!("sigil-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("console.log");
        fs::write(&path, "[Client] Map: \"dl_hideout\"\n[GCClient] k_EMsgClientToGCStartMatch").unwrap();
        let mut tail = Tail::default();
        assert!(tail.poll(&path));
        assert_eq!(tail.session.phase, Phase::Hideout);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, b"making\n").unwrap();
        assert!(tail.poll(&path));
        assert_eq!(tail.session.phase, Phase::InQueue);
        fs::write(&path, "short\n").unwrap();
        tail.poll(&path);
        assert_eq!(tail.session.phase, Phase::MainMenu, "a shorter file starts over");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn minidump_header_gives_code_and_comment() {
        let mut b = vec![0u8; 200];
        b[0..4].copy_from_slice(b"MDMP");
        b[8..12].copy_from_slice(&2u32.to_le_bytes());
        b[12..16].copy_from_slice(&32u32.to_le_bytes());
        // exception stream at 80
        b[32..36].copy_from_slice(&6u32.to_le_bytes());
        b[36..40].copy_from_slice(&168u32.to_le_bytes());
        b[40..44].copy_from_slice(&80u32.to_le_bytes());
        b[88..92].copy_from_slice(&0xC000_0005u32.to_le_bytes());
        // comment stream at 120
        b[44..48].copy_from_slice(&10u32.to_le_bytes());
        b[48..52].copy_from_slice(&11u32.to_le_bytes());
        b[52..56].copy_from_slice(&120u32.to_le_bytes());
        b[120..131].copy_from_slice(b"FATAL ERROR");
        let info = parse_minidump(&b);
        assert_eq!(info.exception, Some(0xC000_0005));
        assert_eq!(info.comment.as_deref(), Some("FATAL ERROR"));
        assert_eq!(exception_label(0xC000_0005), "Access violation");
        assert_eq!(parse_minidump(b"nope"), DumpInfo::default());
    }

    #[test]
    fn endings_are_classified() {
        let log = "[2026-11-17 10:00:00] AppID 1422450 adding PID 42 as a tracked process\n[2026-11-17 10:20:00] AppID 1422450 no longer tracking PID 42, exit code 3221225477\n";
        assert_eq!(steam_exit_code(log), Some(3_221_225_477));
        assert_eq!(classify(0, steam_exit_code(log), false), Ending::Crash);
        assert_eq!(classify(1, Some(0), true), Ending::Crash);
        assert_eq!(classify(0, Some(0), false), Ending::Clean);
        assert_eq!(classify(0, None, true), Ending::Clean);
        assert_eq!(classify(0, Some(1), false), Ending::Unknown);
    }

    #[test]
    fn the_addons_line_comes_out_with_a_backup() {
        let gi = "GameInfo\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel/addons\n\t\t\tGame citadel\n\t\t}\n\t}\n}\n";
        let out = without_addons_line(gi).unwrap();
        assert!(!out.contains("citadel/addons"));
        assert!(out.contains("\t\t\tGame citadel\n"));
        assert!(without_addons_line(&out).is_none());

        let root = std::env::temp_dir().join(format!("sigil-gi-{}", std::process::id()));
        let citadel = root.join("game").join("citadel");
        fs::create_dir_all(&citadel).unwrap();
        fs::write(citadel.join("gameinfo.gi"), gi).unwrap();
        assert!(remove_addons_path(&root, false).is_err());
        assert!(remove_addons_path(&root, true).unwrap());
        assert!(!remove_addons_path(&root, true).unwrap());
        let backups = fs::read_dir(&citadel)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(crate::backups::STAMPED_MARKER))
            .count();
        assert_eq!(backups, 1);
        let _ = fs::remove_dir_all(&root);
    }
}
