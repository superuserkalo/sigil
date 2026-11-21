//! One pass against the real game: the cfg it writes, the marker that lists every change, the
//! restore that undoes them, the launch, and the follow of `console.log` until the game quits.
//!
//! GAME WRITES, each one an app action behind the player's Run press:
//! `game/citadel/cfg/sigil_benchmark.cfg`, a copied replay in `game/citadel/replays/`, the
//! autoexec fallback block, a removed Launch map block, a vanilla stash, and switched-off slots.
//! [`prepare_pass`] records each change in the marker (`benchmark-run.json` in the app data
//! directory) before it makes it, so [`restore`] can undo a pass the game or Sigil never
//! finished, on the next start.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use super::parse::{self, LogEvent, TimedemoLine};
use super::{Variant, metrics};
use crate::error::{Error, Result};
use crate::util::write_atomic;

pub const MARKER_NAME: &str = "benchmark-run.json";
pub const MARKER_SCHEMA: u32 = 1;
pub const CFG_NAME: &str = "sigil_benchmark.cfg";
pub const CFG_HEADER: &str = "// Sigil benchmark";
pub const DEMO_COPY: &str = "sigil_benchmark.dem";
pub const AUTOEXEC_BEGIN: &str = "// Sigil benchmark BEGIN";
pub const AUTOEXEC_END: &str = "// Sigil benchmark END";
/// Frames a per-frame file must hold to be used over the summary line.
pub const MIN_FRAMES: usize = 100;

/// Every change a pass made, written before each change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub schema: u32,
    pub run_id: String,
    pub pass: u32,
    pub started_at: String,
    pub game_root: PathBuf,
    pub cfg: Option<PathBuf>,
    /// The copied replay and its size.
    pub demo_copy: Option<(PathBuf, u64)>,
    pub autoexec_block: bool,
    /// The Launch map block the pass took out, to put back.
    pub map_block: Option<String>,
    pub stashed: bool,
    pub disabled: Vec<u16>,
    /// video.txt copied aside before a fixed-window pass: (the game's file, the copy).
    #[serde(default)]
    pub video_backup: Option<(PathBuf, PathBuf)>,
}

pub fn marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MARKER_NAME)
}

pub fn read_marker(data_dir: &Path) -> Option<Marker> {
    let text = fs::read_to_string(marker_path(data_dir)).ok()?;
    serde_json::from_str::<Marker>(&text).ok().filter(|m| m.schema == MARKER_SCHEMA)
}

fn save_marker(data_dir: &Path, marker: &Marker) -> Result<()> {
    let mut text = serde_json::to_string_pretty(marker)?;
    text.push('\n');
    write_atomic(&marker_path(data_dir), text.as_bytes())
}

pub fn cfg_path(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("cfg").join(CFG_NAME)
}

pub fn replays_dir(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("replays")
}

/// What one pass runs.
#[derive(Debug, Clone, PartialEq)]
pub struct PassPrep {
    pub run_id: String,
    pub pass: u32,
    /// `replays/<name>` for `timedemoquit`; None is a play session.
    pub demo: Option<String>,
    /// A replay outside the game's folder, copied in as [`DEMO_COPY`].
    pub copy_from: Option<PathBuf>,
    pub window: Option<(u32, u32)>,
    pub variant: Variant,
    /// No Steam executable: start the cfg from an autoexec block instead of `+exec`.
    pub autoexec_fallback: bool,
    /// Copy this video.txt aside to the second path and put it back after the pass: a fixed
    /// window size must not stay in the player's settings.
    pub video_backup: Option<(PathBuf, PathBuf)>,
}

/// The cfg a pass runs. Every command is a release command without a cheat flag
/// (`DumpSource2/commands.txt`, build 6857).
pub fn cfg_text(prep: &PassPrep) -> String {
    let mut lines = vec![
        format!("{CFG_HEADER} run {}, pass {}. Sigil removes this file after the run.", prep.run_id, prep.pass),
        format!("echo \"SIGIL_BENCHMARK {} {}\"", prep.run_id, prep.pass),
        // No value: the game prints the cap into the log.
        "fps_max".to_string(),
        // The frame log, a second frame source. Not archived, so it does not stick.
        "cl_showfps 4".to_string(),
    ];
    if let Some(demo) = &prep.demo {
        let (start, end) = prep.window.map(|(s, e)| (s as i64, e as i64)).unwrap_or((-1, -1));
        lines.push(format!("timedemo_start {start}"));
        lines.push(format!("timedemo_end {end}"));
        lines.push(format!("timedemoquit {demo}"));
    }
    lines.join("\n") + "\n"
}

/// The autoexec text with the benchmark block set (`on`) or removed. None when nothing changes.
pub fn set_autoexec_block(text: &str, on: bool) -> Option<String> {
    let without = match text.find(AUTOEXEC_BEGIN) {
        Some(begin) => match text[begin..].find(AUTOEXEC_END) {
            Some(rel) => {
                let end = begin + rel + AUTOEXEC_END.len();
                let end = text[end..].find('\n').map(|o| end + o + 1).unwrap_or(end);
                format!("{}{}", &text[..begin], &text[end..])
            }
            None => text.to_string(),
        },
        None => text.to_string(),
    };
    let trimmed = without.trim_end();
    let next = if on {
        let gap = if trimmed.is_empty() { "" } else { "\n" };
        format!("{trimmed}{gap}{AUTOEXEC_BEGIN}\nexec sigil_benchmark\n{AUTOEXEC_END}\n")
    } else if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    };
    (next.trim_end() != text.trim_end()).then_some(next)
}

/// Why a run cannot start now. Empty means it can.
pub fn blockers(game_root: &Path, data_dir: &Path, game_running: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if game_running {
        out.push("Deadlock is running. Close it first.");
    }
    if read_marker(data_dir).is_some() {
        out.push("A benchmark pass is still open. Sigil puts it back first.");
    }
    if crate::launch::stash_status(data_dir).is_some() {
        out.push("Mods from a vanilla launch are still set aside. Put them back first.");
    }
    if !game_root.join("game").join("citadel").is_dir() {
        out.push("Deadlock was not found.");
    }
    out
}

/// GAME WRITE. Make every change a pass needs, recording each in the marker first.
pub fn prepare_pass(game_root: &Path, data_dir: &Path, prep: &PassPrep) -> Result<Marker> {
    if read_marker(data_dir).is_some() {
        return Err(Error::msg("A benchmark pass is still open. Restore it before starting another."));
    }
    let mut marker = Marker {
        schema: MARKER_SCHEMA,
        run_id: prep.run_id.clone(),
        pass: prep.pass,
        started_at: crate::util::now_iso(),
        game_root: game_root.to_path_buf(),
        ..Marker::default()
    };
    save_marker(data_dir, &marker)?;
    let step = |marker: &mut Marker, change: &dyn Fn(&mut Marker)| -> Result<()> {
        change(marker);
        save_marker(data_dir, marker)
    };

    if let Some(from) = &prep.copy_from {
        let dir = replays_dir(game_root);
        fs::create_dir_all(&dir)?;
        let to = dir.join(DEMO_COPY);
        let bytes = fs::metadata(from)?.len();
        if to.is_file() && fs::metadata(&to)?.len() != bytes {
            return Err(Error::msg(format!("{} is in the way. Move it, then run again.", to.display())));
        }
        step(&mut marker, &|m| m.demo_copy = Some((to.clone(), bytes)))?;
        fs::copy(from, &to)?;
    }

    let cfg = cfg_path(game_root);
    if let Some(parent) = cfg.parent() {
        fs::create_dir_all(parent)?;
    }
    if cfg.is_file() && !is_ours(&cfg) {
        return Err(Error::msg(format!("{} is not Sigil's. Rename it, then run again.", cfg.display())));
    }
    step(&mut marker, &|m| m.cfg = Some(cfg.clone()))?;
    write_atomic(&cfg, cfg_text(prep).as_bytes())?;

    // A Launch map block would start the map instead of the replay.
    if let Some(map) = crate::servers::read_map_launch(game_root) {
        step(&mut marker, &|m| m.map_block = Some(map.clone()))?;
        crate::servers::write_map_launch(game_root, None, true)?;
    }

    if prep.autoexec_fallback {
        step(&mut marker, &|m| m.autoexec_block = true)?;
        let current = crate::autoexec::read_autoexec(game_root)?.unwrap_or_default();
        if let Some(next) = set_autoexec_block(&current, true) {
            crate::autoexec::write_autoexec(game_root, &next, true)?;
        }
    }

    if let Some((video, copy)) = &prep.video_backup
        && video.is_file()
    {
        if let Some(parent) = copy.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(video, copy)?;
        step(&mut marker, &|m| m.video_backup = Some((video.clone(), copy.clone())))?;
    }

    match &prep.variant {
        Variant::AsIs => {}
        Variant::Vanilla => {
            step(&mut marker, &|m| m.stashed = true)?;
            crate::deploy::ensure_launch_search_paths(game_root).ok();
            crate::launch::stash_vanilla(game_root, data_dir, true)?;
        }
        Variant::Without(slots) => {
            let installed = crate::addons::list_installed(game_root)?;
            for slot in slots {
                if installed.iter().any(|a| a.slot == *slot && a.enabled) {
                    let slot = *slot;
                    step(&mut marker, &|m| m.disabled.push(slot))?;
                    crate::addons::set_enabled(game_root, slot, false, true)?;
                }
            }
        }
    }
    Ok(marker)
}

fn is_ours(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|t| t.starts_with(CFG_HEADER))
}

/// What a restore did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Restored {
    /// Plain lines: "Put 2 mods back on", "Removed the benchmark cfg".
    pub done: Vec<String>,
    pub problems: Vec<String>,
}

/// GAME WRITE. Undo every change the marker lists, newest first, then remove the marker. A
/// change that fails stays in the marker for the next try. Refuses while the game runs.
pub fn restore(data_dir: &Path, game_running: bool) -> Result<Option<Restored>> {
    let Some(mut marker) = read_marker(data_dir) else {
        return Ok(None);
    };
    if game_running {
        return Err(Error::msg("Deadlock is running. Sigil puts the benchmark changes back when it closes."));
    }
    let root = marker.game_root.clone();
    let mut out = Restored::default();

    let mut left = Vec::new();
    let mut back = 0;
    for slot in marker.disabled.iter().rev() {
        let off = crate::addons::list_installed(&root)
            .map(|list| list.iter().any(|a| a.slot == *slot && !a.enabled))
            .unwrap_or(false);
        if !off {
            continue;
        }
        match crate::addons::set_enabled(&root, *slot, true, true) {
            Ok(_) => back += 1,
            Err(error) => {
                left.push(*slot);
                out.problems.push(error.to_string());
            }
        }
    }
    if back > 0 {
        out.done.push(format!("Switched {back} mod(s) back on"));
    }
    marker.disabled = left;

    if marker.stashed {
        if crate::launch::stash_status(data_dir).is_some() {
            match crate::launch::restore_vanilla(data_dir, true) {
                Ok(r) => {
                    out.done.push(format!("Put {} mod file(s) back from the vanilla stash", r.restored.len()));
                    marker.stashed = false;
                }
                Err(error) => out.problems.push(error.to_string()),
            }
        } else {
            marker.stashed = false;
        }
    }

    if marker.autoexec_block {
        let current = crate::autoexec::read_autoexec(&root).ok().flatten().unwrap_or_default();
        match set_autoexec_block(&current, false) {
            Some(next) => match crate::autoexec::write_autoexec(&root, &next, true) {
                Ok(_) => {
                    out.done.push("Took the benchmark line out of autoexec.cfg".into());
                    marker.autoexec_block = false;
                }
                Err(error) => out.problems.push(error.to_string()),
            },
            None => marker.autoexec_block = false,
        }
    }

    if let Some(map) = marker.map_block.clone() {
        if crate::servers::read_map_launch(&root).is_none() {
            match crate::servers::write_map_launch(&root, Some(&map), true) {
                Ok(_) => {
                    out.done.push(format!("Set {map} as the start map again"));
                    marker.map_block = None;
                }
                Err(error) => out.problems.push(error.to_string()),
            }
        } else {
            marker.map_block = None;
        }
    }

    if let Some((video, copy)) = marker.video_backup.clone() {
        let same = fs::read(&video).ok() == fs::read(&copy).ok();
        let result = if same || !copy.is_file() { Ok(0) } else { fs::copy(&copy, &video) };
        match result {
            Ok(_) => {
                if !same && copy.is_file() {
                    out.done.push("Put your video settings back".into());
                }
                marker.video_backup = None;
            }
            Err(error) => out.problems.push(error.to_string()),
        }
    }

    if let Some(cfg) = marker.cfg.clone() {
        if cfg.is_file() && is_ours(&cfg) {
            match fs::remove_file(&cfg) {
                Ok(()) => {
                    out.done.push("Removed the benchmark cfg".into());
                    marker.cfg = None;
                }
                Err(error) => out.problems.push(error.to_string()),
            }
        } else {
            marker.cfg = None;
        }
    }

    if let Some((copy, bytes)) = marker.demo_copy.clone() {
        let same = fs::metadata(&copy).is_ok_and(|m| m.len() == bytes);
        if same {
            match fs::remove_file(&copy) {
                Ok(()) => {
                    out.done.push("Removed the copied replay".into());
                    marker.demo_copy = None;
                }
                Err(error) => out.problems.push(error.to_string()),
            }
        } else {
            marker.demo_copy = None;
        }
    }

    let open = !marker.disabled.is_empty()
        || marker.stashed
        || marker.autoexec_block
        || marker.map_block.is_some()
        || marker.cfg.is_some()
        || marker.demo_copy.is_some()
        || marker.video_backup.is_some();
    if open {
        save_marker(data_dir, &marker)?;
    } else {
        match fs::remove_file(marker_path(data_dir)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(out))
}

// --- launch -------------------------------------------------------------------------------

/// What Steam is handed: the game's id and this launch's own arguments. `-condebug` makes the
/// engine write `console.log` for this launch only; the saved launch options stay as they are.
/// A fixed display adds its window arguments before the `+exec`.
pub fn launch_args(display: super::Display) -> Vec<String> {
    let mut args: Vec<String> = ["-applaunch", crate::discovery::DEADLOCK_APP_ID, "-condebug", "-novid"].map(String::from).to_vec();
    args.extend(display.args());
    args.extend(["+exec".to_string(), "sigil_benchmark".to_string()]);
    args
}

/// The Steam executable under one of the Steam roots, when there is one.
pub fn steam_exe(roots: &[PathBuf]) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["steam.exe"] } else if cfg!(target_os = "linux") { &["steam.sh"] } else { &[] };
    roots.iter().flat_map(|root| names.iter().map(move |n| root.join(n))).find(|p| p.is_file())
}

/// PROCESS. Hand the pass to Steam: `steam -applaunch` with the arguments, or the plain Steam
/// URL when there is no executable (the cfg then starts from the autoexec block, and a fixed
/// display cannot be passed).
pub fn launch(steam: Option<&Path>, display: super::Display) -> Result<String> {
    match steam {
        Some(exe) => {
            let args = launch_args(display);
            crate::util::hidden_command(exe)
                .args(&args)
                .spawn()
                .map_err(|e| Error::msg(format!("Steam could not be started: {e}")))?;
            Ok(format!("{} {}", exe.display(), args.join(" ")))
        }
        None => crate::launch::launch_modded(true).map(|r| r.url),
    }
}

// --- following a pass -----------------------------------------------------------------------

/// Reads whole new lines of a growing file. A file that got shorter is read from the start.
#[derive(Debug, Clone, Default)]
pub struct LineTail {
    pub offset: u64,
}

impl LineTail {
    pub fn from_end(path: &Path) -> Self {
        LineTail { offset: fs::metadata(path).map(|m| m.len()).unwrap_or(0) }
    }

    pub fn lines(&mut self, path: &Path) -> Vec<String> {
        let Ok(mut file) = fs::File::open(path) else { return Vec::new() };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            self.offset = 0;
        }
        if len == self.offset || file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        let _ = file.take(len - self.offset).read_to_end(&mut bytes);
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else { return Vec::new() };
        self.offset += end as u64 + 1;
        String::from_utf8_lossy(&bytes[..end]).lines().map(|l| l.trim_end_matches('\r').to_string()).collect()
    }
}

/// Where a pass is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    WaitingForGame,
    Loading,
    Measuring,
    Finishing,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::WaitingForGame => "Waiting for Deadlock to start",
            Stage::Loading => "Loading the replay",
            Stage::Measuring => "Measuring",
            Stage::Finishing => "Deadlock is closing",
        }
    }
}

/// Live state of a pass, for the running screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub stage: Stage,
    pub last_line: Option<String>,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The game quit (after the result, or not).
    Exited,
    Stopped,
    TimedOut,
    NeverStarted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Followed {
    pub events: Vec<LogEvent>,
    pub end: End,
    pub last_line: Option<String>,
    /// Every console line of the pass, for the run's log folder (at most [`LOG_LINES`]).
    pub lines: Vec<String>,
}

/// Console lines a pass keeps for its log.
pub const LOG_LINES: usize = 50_000;

impl Followed {
    pub fn timedemo(&self) -> Option<&TimedemoLine> {
        self.events.iter().rev().find_map(|e| match e {
            LogEvent::Timedemo(t) => Some(t),
            _ => None,
        })
    }

    pub fn fps_cap(&self) -> Option<f64> {
        self.events.iter().find_map(|e| match e {
            LogEvent::FpsCap(c) => Some(*c),
            _ => None,
        })
    }

    pub fn demo_error(&self) -> Option<&str> {
        self.events.iter().find_map(|e| match e {
            LogEvent::DemoError(line) => Some(line.as_str()),
            _ => None,
        })
    }

    pub fn showfps_log(&self) -> Option<&str> {
        self.events.iter().rev().find_map(|e| match e {
            LogEvent::ShowFpsLog(p) => Some(p.as_str()),
            _ => None,
        })
    }

    pub fn cfg_ran(&self) -> bool {
        self.events.iter().any(|e| matches!(e, LogEvent::Marker(..)))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// How long Steam has to start the game.
    pub start: Duration,
    /// The longest a pass may take once the game is up.
    pub run: Duration,
    /// How long the game may stay open after the result before Sigil closes it.
    pub linger: Duration,
    pub tick: Duration,
    /// How often the process table is read.
    pub check: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            start: Duration::from_secs(180),
            run: Duration::from_secs(15 * 60),
            linger: Duration::from_secs(30),
            tick: Duration::from_millis(500),
            check: Duration::from_secs(2),
        }
    }
}

/// Follow `console.log` and the game process until the pass ends. `running` answers whether the
/// game process is up; `live` hears every change. A play session (no replay) ends when the
/// game quits. Never closes the game itself: the caller does, on Stopped and TimedOut.
pub fn follow(
    log: &Path,
    replay: bool,
    limits: Limits,
    stop: &AtomicBool,
    mut running: impl FnMut() -> bool,
    mut live: impl FnMut(&Live),
) -> Followed {
    let mut tail = LineTail::from_end(log);
    let started = Instant::now();
    let mut up_since: Option<Instant> = None;
    let mut result_at: Option<Instant> = None;
    let mut events = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut last_line: Option<String> = None;
    let mut stage = Stage::WaitingForGame;
    let mut last_check: Option<Instant> = None;
    let mut is_up = false;
    loop {
        for line in tail.lines(log) {
            if lines.len() < LOG_LINES {
                lines.push(line.clone());
            }
            if !line.trim().is_empty() {
                last_line = Some(line.trim().chars().take(160).collect());
            }
            if let Some(event) = parse::scan_line(&line) {
                match &event {
                    LogEvent::TimedemoRequested(_) | LogEvent::Marker(..) if stage < Stage::Measuring => {
                        stage = if matches!(event, LogEvent::TimedemoRequested(_)) || !replay { Stage::Measuring } else { Stage::Loading };
                    }
                    LogEvent::Timedemo(_) | LogEvent::PlaybackFinished { .. } => {
                        stage = Stage::Finishing;
                        result_at.get_or_insert_with(Instant::now);
                    }
                    _ => {}
                }
                events.push(event);
            }
        }
        if last_check.is_none_or(|t| t.elapsed() >= limits.check) {
            last_check = Some(Instant::now());
            is_up = running();
            if is_up && up_since.is_none() {
                up_since = Some(Instant::now());
                if stage == Stage::WaitingForGame {
                    stage = if replay { Stage::Loading } else { Stage::Measuring };
                }
            }
        }
        live(&Live { stage, last_line: last_line.clone(), elapsed: started.elapsed() });
        let end = if stop.load(Ordering::Relaxed) {
            Some(End::Stopped)
        } else if up_since.is_some() && !is_up {
            Some(End::Exited)
        } else if up_since.is_none() && started.elapsed() > limits.start {
            Some(End::NeverStarted)
        } else if up_since.is_some_and(|t| t.elapsed() > limits.run)
            || result_at.is_some_and(|t| t.elapsed() > limits.linger)
        {
            Some(End::TimedOut)
        } else {
            None
        };
        if let Some(end) = end {
            // The last lines the game wrote before it quit.
            for line in tail.lines(log) {
                if let Some(event) = parse::scan_line(&line) {
                    events.push(event);
                }
                if lines.len() < LOG_LINES {
                    lines.push(line);
                }
            }
            let end = if end == End::TimedOut && result_at.is_some() { End::Exited } else { end };
            return Followed { events, end, last_line, lines };
        }
        std::thread::sleep(limits.tick);
    }
}

// --- frames ---------------------------------------------------------------------------------

/// Where a pass's frame times came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FrameSource {
    /// The engine's `timedemo_profile.csv`.
    TimedemoProfile,
    /// The `cl_showfps 4` log.
    ShowFps,
    /// Only the result line: no per-frame data.
    SummaryOnly,
    /// Nothing: the pass failed.
    None,
}

impl FrameSource {
    pub fn label(self) -> &'static str {
        match self {
            FrameSource::TimedemoProfile => "The game's timedemo frame file",
            FrameSource::ShowFps => "The game's frame log (cl_showfps 4)",
            FrameSource::SummaryOnly => "The game's summary line only",
            FrameSource::None => "No data",
        }
    }
}

fn newest_since(dirs: &[PathBuf], since: SystemTime, matches: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !matches(&name) {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
            if modified >= since && best.as_ref().is_none_or(|(t, _)| modified > *t) {
                best = Some((modified, entry.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Folders the game may write its frame files to.
pub fn frame_dirs(game_root: &Path) -> Vec<PathBuf> {
    let game = game_root.join("game");
    vec![
        game.join("citadel"),
        game.clone(),
        game.join("bin").join("win64"),
        game.join("citadel").join("cfg"),
        game_root.to_path_buf(),
    ]
}

/// The `cl_showfps 4` frames that belong to the timedemo: `frames` consecutive frames whose
/// time is closest to the timedemo's seconds, the latest such run on a tie. The log also holds
/// the menu and the loading screen, which are not the measurement.
pub fn align(all: &[f64], frames: usize, seconds: f64) -> Vec<f64> {
    if frames == 0 || frames >= all.len() {
        return all.to_vec();
    }
    let target = seconds * 1000.0;
    let mut sum: f64 = all[..frames].iter().sum();
    let (mut best, mut best_end) = ((sum - target).abs(), frames);
    for end in frames + 1..=all.len() {
        sum += all[end - 1] - all[end - 1 - frames];
        let gap = (sum - target).abs();
        if gap <= best {
            best = gap;
            best_end = end;
        }
    }
    all[best_end - frames..best_end].to_vec()
}

/// The frame times of a pass, best source first, and the file they came from.
pub fn collect_frames(game_root: &Path, since: SystemTime, followed: &Followed) -> (FrameSource, Vec<f64>, Option<PathBuf>) {
    let dirs = frame_dirs(game_root);
    if let Some(path) = newest_since(&dirs, since, |n| n == "timedemo_profile.csv")
        && let Ok(text) = fs::read_to_string(&path)
    {
        let frames = parse::timedemo_profile(&text);
        if frames.len() >= MIN_FRAMES {
            return (FrameSource::TimedemoProfile, frames, Some(path));
        }
    }
    let named = followed.showfps_log().map(|name| {
        let name = Path::new(name);
        if name.is_absolute() { vec![name.to_path_buf()] } else { dirs.iter().map(|d| d.join(name)).collect() }
    });
    let showfps = named
        .and_then(|candidates| candidates.into_iter().find(|p| p.is_file()))
        .or_else(|| newest_since(&dirs, since, |n| n.starts_with("prof_") && n.ends_with(".csv")));
    if let Some(path) = showfps
        && let Ok(text) = fs::read_to_string(&path)
    {
        let all = parse::showfps_log(&text);
        let frames = match followed.timedemo() {
            Some(t) => align(&all, t.frames as usize, t.seconds),
            None => all,
        };
        if frames.len() >= MIN_FRAMES {
            return (FrameSource::ShowFps, frames, Some(path));
        }
    }
    if followed.timedemo().is_some() {
        (FrameSource::SummaryOnly, Vec::new(), None)
    } else {
        (FrameSource::None, Vec::new(), None)
    }
}

/// The replay a pass is about to play is the one the run started with: same SHA-256 as
/// `expected`. A replay the game downloaded again, or a copy that went wrong, stops the run.
pub fn verify_demo(path: &Path, expected: &str) -> Result<()> {
    let found = crate::servers::file_sha256(path)?;
    if found.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "{} changed since the run started (SHA-256 {} is now {}). The passes would not be comparable.",
            path.display(),
            &expected[..expected.len().min(12)],
            &found[..found.len().min(12)]
        )))
    }
}

/// The pass summary: from the frames when there are enough, else from the result line.
pub fn summary_for(source: FrameSource, frames: &[f64], followed: &Followed) -> Option<metrics::Summary> {
    if matches!(source, FrameSource::TimedemoProfile | FrameSource::ShowFps) {
        return Some(metrics::summarize(frames));
    }
    let t = followed.timedemo()?;
    Some(metrics::Summary {
        frames: t.frames,
        seconds: t.seconds,
        avg_fps: t.fps,
        p50_ms: t.p50_ms,
        p95_ms: t.p95_ms,
        ..metrics::Summary::default()
    })
}

#[cfg(test)]
mod tests;
