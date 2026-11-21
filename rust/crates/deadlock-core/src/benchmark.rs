//! Sigil's in-game benchmark: play the same replay section with `timedemo`, read the frames the
//! game logs, and compare variants of the game (mods on, vanilla, a mod off) the same way every
//! time. Design and the facts it rests on: `docs/design/benchmark.md`.
//!
//! - [`metrics`]: frame time statistics (average FPS, lows, percentiles, stutters, the graph).
//! - [`parse`]: `console.log` lines and the per-frame CSV files the game writes.
//! - [`compare`]: two results, the noise between them and a verdict.
//! - [`session`]: the cfg a pass writes, the run marker, restore, the launch and the follow.
//! - [`system`]: the machine and the settings a run ran on.
//! - [`store`]: results on disk and the exports.
//!
//! This file holds the run model: what a run tests, the passes it plans, and the mod cost
//! search. It is pure; [`session`] does the game writes, each one an app action the player's
//! Run press approves.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub mod compare;
pub mod metrics;
pub mod parse;
pub mod session;
pub mod store;
pub mod system;

pub use metrics::Summary;

/// Schema of `run.json`.
pub const RUN_SCHEMA: u32 = 1;
/// Demo ticks per second.
pub const TICK_RATE: u32 = 64;
/// Passes a side has by default.
pub const DEFAULT_PASSES: u32 = 3;
pub const MAX_PASSES: u32 = 5;
/// The smallest gap a single-pass cost search step counts, as a share of the base.
pub const COST_FLOOR_SHARE: f64 = 0.02;

/// Where a replay came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DemoSource {
    /// `game/citadel/replays/`, downloaded by the game.
    Game,
    /// A match imported into Sigil's Stats.
    Sigil,
    /// A file the player picked.
    Picked,
}

/// One replay a run can play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Demo {
    pub source: DemoSource,
    pub path: PathBuf,
    /// The match id or the file stem.
    pub label: String,
    pub map: Option<String>,
    /// The build the replay was recorded on, from its header.
    pub build: Option<i32>,
    pub ticks: Option<u32>,
    pub seconds: Option<f32>,
    pub bytes: u64,
    /// SHA-256 of the file, taken when a run starts and checked before every pass: a replay the
    /// game downloaded again mid-run is not the same workload.
    #[serde(default)]
    pub sha256: Option<String>,
}

impl Demo {
    /// The name `timedemo` takes, relative to `game/citadel`, without `.dem`. A replay outside
    /// the game's replay folder is copied in as [`session::DEMO_COPY`].
    pub fn game_name(&self) -> String {
        match self.source {
            DemoSource::Game => format!(
                "replays/{}",
                self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            ),
            DemoSource::Sigil | DemoSource::Picked => {
                format!("replays/{}", session::DEMO_COPY.trim_end_matches(".dem"))
            }
        }
    }

    /// The replay was recorded on another build than `current`: the engine will likely refuse it.
    pub fn other_build(&self, current: Option<&str>) -> bool {
        match (self.build, current.and_then(|c| c.trim().parse::<i32>().ok())) {
            (Some(demo), Some(game)) => demo != game,
            _ => false,
        }
    }
}

/// A replay file as the setup lists it. The header read streams the whole file (the info
/// block sits at the end), so callers run it on a worker. A file that does not parse as a
/// Source 2 demo is still listed, with nothing known about it.
pub fn read_demo(path: &std::path::Path, source: DemoSource) -> Demo {
    let parsed = crate::demo::parse_demo(path).ok();
    let header = parsed.as_ref().and_then(|d| d.header.as_ref());
    let info = parsed.as_ref().and_then(|d| d.file_info.as_ref());
    let label = match source {
        DemoSource::Sigil => path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    Demo {
        source,
        path: path.to_path_buf(),
        label,
        map: header.and_then(|h| h.map_name.clone()).filter(|m| !m.is_empty()),
        build: header.and_then(|h| h.build_num),
        ticks: info.and_then(|i| i.playback_ticks),
        seconds: info.and_then(|i| i.playback_time_seconds),
        bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        sha256: None,
    }
}

/// Every replay a run can play: the game's own replay folder first (newest first), then the
/// matches imported into Sigil's Stats. Sigil's own copy is never listed.
pub fn find_demos(game_root: Option<&std::path::Path>, workspace: &std::path::Path) -> Vec<Demo> {
    fn dems(dir: &std::path::Path) -> Vec<(std::time::SystemTime, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dem")))
            .filter(|p| p.file_name().is_some_and(|n| n != session::DEMO_COPY))
            .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
            .collect()
    }
    let mut out = Vec::new();
    if let Some(root) = game_root {
        let mut game = dems(&session::replays_dir(root));
        game.sort_by_key(|a| std::cmp::Reverse(a.0));
        out.extend(game.into_iter().map(|(_, p)| read_demo(&p, DemoSource::Game)));
    }
    let mut sigil: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(crate::demo::matches_dir(workspace))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path().join(crate::demo::MATCH_DEMO_FILE_NAME))
                .filter(|p| p.is_file())
                .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
                .collect()
        })
        .unwrap_or_default();
    sigil.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.extend(sigil.into_iter().map(|(_, p)| read_demo(&p, DemoSource::Sigil)));
    out
}

/// How long the measured section is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Length {
    Short,
    #[default]
    Standard,
    Long,
    Whole,
}

impl Length {
    pub const ALL: [Length; 4] = [Length::Short, Length::Standard, Length::Long, Length::Whole];

    pub fn seconds(self) -> Option<u32> {
        match self {
            Length::Short => Some(45),
            Length::Standard => Some(90),
            Length::Long => Some(180),
            Length::Whole => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Length::Short => "45 seconds",
            Length::Standard => "90 seconds",
            Length::Long => "3 minutes",
            Length::Whole => "Whole replay",
        }
    }
}

/// The ticks to measure: a window of `length` centred on the middle of the replay, clamped to
/// it. None measures the whole replay.
pub fn window(ticks: Option<u32>, length: Length) -> Option<(u32, u32)> {
    let want = length.seconds()? * TICK_RATE;
    let total = ticks.filter(|t| *t > TICK_RATE * 10)?;
    if want >= total {
        return None;
    }
    let start = (total / 2).saturating_sub(want / 2);
    Some((start, start + want))
}

/// The window the game draws in during a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Display {
    /// The player's own video settings.
    #[default]
    Game,
    /// A fixed windowed size for this run's launches (`-windowed -w -h`), so runs on different
    /// days and monitors draw the same number of pixels. video.txt is put back after each pass.
    Fixed { width: u32, height: u32 },
}

impl Display {
    pub const ALL: [Display; 4] = [
        Display::Game,
        Display::Fixed { width: 1280, height: 720 },
        Display::Fixed { width: 1920, height: 1080 },
        Display::Fixed { width: 2560, height: 1440 },
    ];

    pub fn label(self) -> String {
        match self {
            Display::Game => "Game settings".to_string(),
            Display::Fixed { height, .. } => format!("{height}p window"),
        }
    }

    /// The launch arguments for this display.
    pub fn args(self) -> Vec<String> {
        match self {
            Display::Game => Vec::new(),
            Display::Fixed { width, height } => {
                vec!["-windowed".into(), "-noborder".into(), "-w".into(), width.to_string(), "-h".into(), height.to_string()]
            }
        }
    }
}

/// The longest a whole run may take before Sigil stops it: every launch with its section, eight
/// minutes of slack each (start, load, quit), and ten minutes more.
pub fn run_limit(spec: &RunSpec) -> std::time::Duration {
    let section = spec.length.seconds().unwrap_or(45 * 60) as u64;
    let launches = launches(spec).max(1) as u64;
    std::time::Duration::from_secs(launches * (section + 8 * 60) + 10 * 60)
}

/// What a run measures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Test {
    /// The game as it is.
    Measure,
    /// All mods against none.
    ModsVsVanilla,
    /// One mod on against off.
    OneMod { slot: u16, name: String },
    /// The halving search for the costliest mod.
    CostSearch { pool: Vec<u16> },
    /// The player's own play with the frame log on. Never compared against a replay run.
    PlaySession,
}

impl Test {
    pub fn label(&self) -> &'static str {
        match self {
            Test::Measure => "Just measure",
            Test::ModsVsVanilla => "Mods vs vanilla",
            Test::OneMod { .. } => "One mod",
            Test::CostSearch { .. } => "Find the costliest mod",
            Test::PlaySession => "Play session",
        }
    }

    /// The two sides of a comparison, A first. None for a test with one side.
    pub fn sides(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Test::ModsVsVanilla => Some(("Vanilla", "With mods")),
            Test::OneMod { .. } => Some(("Mod off", "Mod on")),
            _ => None,
        }
    }
}

/// The game a pass runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "slots")]
pub enum Variant {
    /// Mods as they are.
    AsIs,
    /// Every mod set aside (the vanilla stash).
    Vanilla,
    /// These slots switched off.
    Without(Vec<u16>),
}

/// Which side of a comparison a pass belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    A,
    B,
}

/// One planned pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassPlan {
    pub variant: Variant,
    pub side: Side,
    pub warmup: bool,
}

/// Everything the player chose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSpec {
    pub id: String,
    pub created_at: String,
    pub test: Test,
    /// None for a play session.
    pub demo: Option<Demo>,
    pub length: Length,
    /// Ticks measured, from [`window`].
    pub window: Option<(u32, u32)>,
    /// Measured passes per side.
    pub passes: u32,
    pub warmup: bool,
    #[serde(default)]
    pub display: Display,
}

/// Now, as stored in a run (`2026-11-21T18:02:00Z`).
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A stored time as the player reads it, in local time: `21 Nov, 18:02`.
pub fn local_when(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| t.with_timezone(&chrono::Local).format("%-d %b, %H:%M").to_string())
        .unwrap_or_else(|_| at.get(..10).unwrap_or("").to_string())
}

/// The id for a run starting now.
pub fn next_run_id() -> String {
    new_run_id(chrono::Utc::now())
}

/// A run id: `b` and the start time, sortable.
pub fn new_run_id(now: chrono::DateTime<chrono::Utc>) -> String {
    format!("b{}", now.format("%Y%m%d-%H%M%S"))
}

/// The passes of a test with a fixed plan, in order: the warm-up, then the sides interleaved
/// (A B A B ...), so drift lands on both sides. A cost search plans pass by pass
/// ([`CostSearch::next`]); its list here holds only the warm-up.
pub fn plan(spec: &RunSpec) -> Vec<PassPlan> {
    let passes = spec.passes.clamp(1, MAX_PASSES);
    let mut out = Vec::new();
    let one = |variant: Variant, side: Side, warmup: bool| PassPlan { variant, side, warmup };
    let sides: Option<(Variant, Variant)> = match &spec.test {
        Test::ModsVsVanilla => Some((Variant::Vanilla, Variant::AsIs)),
        Test::OneMod { slot, .. } => Some((Variant::Without(vec![*slot]), Variant::AsIs)),
        _ => None,
    };
    if matches!(spec.test, Test::PlaySession) {
        return vec![one(Variant::AsIs, Side::A, false)];
    }
    if spec.warmup {
        out.push(one(Variant::AsIs, Side::B, true));
    }
    match sides {
        Some((a, b)) => {
            for _ in 0..passes {
                out.push(one(a.clone(), Side::A, false));
                out.push(one(b.clone(), Side::B, false));
            }
        }
        None if matches!(spec.test, Test::CostSearch { .. }) => {}
        None => {
            for _ in 0..passes {
                out.push(one(Variant::AsIs, Side::A, false));
            }
        }
    }
    out
}

/// Launches a test takes, warm-up included. A cost search is an estimate.
pub fn launches(spec: &RunSpec) -> usize {
    match &spec.test {
        Test::CostSearch { pool } => plan(spec).len() + CostSearch::estimate(pool.len()),
        _ => plan(spec).len(),
    }
}

// --- the mod cost search ------------------------------------------------------------------

/// One halving step: each half switched off once, against a base measured right before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    pub base: Option<f64>,
    pub halves: [Vec<u16>; 2],
    /// Average FPS with each half switched off.
    pub fps: [Option<f64>; 2],
}

impl Level {
    fn new(pool: &[u16]) -> Self {
        let mid = pool.len().div_ceil(2);
        Level { base: None, halves: [pool[..mid].to_vec(), pool[mid..].to_vec()], fps: [None, None] }
    }

    /// FPS gained by switching each half off. None until measured.
    pub fn gains(&self) -> [Option<f64>; 2] {
        let base = self.base;
        [0, 1].map(|i| Some(self.fps[i]? - base?))
    }
}

/// How a cost search ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CostEnd {
    /// One mod holds the cost.
    Found { slot: u16 },
    /// No half cost more than the noise.
    NothingMeasurable,
}

/// "Which mod costs the most FPS": halve the suspects, switch each half off, follow the half
/// whose removal gains more, until one mod is left. Pure, like [`crate::bisect::Bisect`]: the
/// caller runs [`CostSearch::next`] and reports with [`CostSearch::record`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostSearch {
    pub pool: Vec<u16>,
    pub levels: Vec<Level>,
    pub end: Option<CostEnd>,
}

impl CostSearch {
    pub fn new(pool: &[u16]) -> Self {
        let mut pool = pool.to_vec();
        pool.sort_unstable();
        pool.dedup();
        let end = pool.is_empty().then_some(CostEnd::NothingMeasurable);
        let levels = if pool.is_empty() { Vec::new() } else { vec![Level::new(&pool)] };
        CostSearch { pool, levels, end }
    }

    /// Launches a search over `n` mods is expected to take: three per halving.
    pub fn estimate(n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let mut levels = 0;
        let mut left = n;
        while left > 1 {
            left = left.div_ceil(2);
            levels += 1;
        }
        3 * levels.max(1)
    }

    /// The next pass to run, or None when the search is over.
    pub fn next(&self) -> Option<Variant> {
        if self.end.is_some() {
            return None;
        }
        let level = self.levels.last()?;
        if level.base.is_none() {
            return Some(Variant::AsIs);
        }
        (0..2)
            .find(|i| level.fps[*i].is_none() && !level.halves[*i].is_empty())
            .map(|i| Variant::Without(level.halves[i].clone()))
    }

    /// Record the average FPS of the pass [`CostSearch::next`] asked for.
    pub fn record(&mut self, variant: &Variant, fps: f64) {
        let Some(level) = self.levels.last_mut() else { return };
        match variant {
            Variant::AsIs => level.base = Some(fps),
            Variant::Without(slots) => {
                if let Some(i) = (0..2).find(|i| &level.halves[*i] == slots) {
                    level.fps[i] = Some(fps);
                }
            }
            Variant::Vanilla => return,
        }
        // An empty half costs nothing.
        for i in 0..2 {
            if level.halves[i].is_empty() && level.fps[i].is_none() {
                level.fps[i] = level.base;
            }
        }
        let [Some(a), Some(b)] = level.gains() else { return };
        let base = level.base.unwrap_or(0.0);
        let floor = (base * COST_FLOOR_SHARE).max(1.0);
        let (gain, half) = if a >= b { (a, 0) } else { (b, 1) };
        if gain < floor {
            self.end = Some(CostEnd::NothingMeasurable);
            return;
        }
        let next = level.halves[half].clone();
        if next.len() == 1 {
            self.end = Some(CostEnd::Found { slot: next[0] });
        } else {
            self.levels.push(Level::new(&next));
        }
    }

    /// Every group measured, with the FPS its removal gained, costliest first.
    pub fn groups(&self) -> Vec<(Vec<u16>, f64)> {
        let mut out: Vec<(Vec<u16>, f64)> = self
            .levels
            .iter()
            .flat_map(|level| {
                let gains = level.gains();
                (0..2).filter_map(move |i| {
                    let gain = gains[i]?;
                    (!level.halves[i].is_empty()).then(|| (level.halves[i].clone(), gain))
                })
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(test: Test) -> RunSpec {
        RunSpec {
            id: "b1".into(),
            created_at: String::new(),
            test,
            demo: None,
            length: Length::Standard,
            window: None,
            passes: 3,
            warmup: true,
            display: Display::Game,
        }
    }

    #[test]
    fn a_fixed_display_adds_window_arguments_and_the_run_has_a_limit() {
        assert!(Display::Game.args().is_empty());
        assert_eq!(Display::ALL[2].args(), vec!["-windowed", "-noborder", "-w", "1920", "-h", "1080"]);
        assert_eq!(Display::ALL[1].label(), "720p window");
        let s = spec(Test::ModsVsVanilla);
        assert_eq!(run_limit(&s), std::time::Duration::from_secs(7 * (90 + 480) + 600));
    }

    #[test]
    fn comparisons_interleave_after_the_warmup() {
        let passes = plan(&spec(Test::ModsVsVanilla));
        assert_eq!(passes.len(), 7);
        assert!(passes[0].warmup && passes[0].variant == Variant::AsIs);
        let sides: Vec<Side> = passes[1..].iter().map(|p| p.side).collect();
        assert_eq!(sides, vec![Side::A, Side::B, Side::A, Side::B, Side::A, Side::B]);
        assert_eq!(passes[1].variant, Variant::Vanilla);
        let one = plan(&spec(Test::OneMod { slot: 4, name: "x".into() }));
        assert_eq!(one[1].variant, Variant::Without(vec![4]));
        assert_eq!(plan(&spec(Test::Measure)).len(), 4);
        assert_eq!(plan(&spec(Test::PlaySession)).len(), 1);
        let mut cold = spec(Test::Measure);
        cold.warmup = false;
        cold.passes = 9;
        assert_eq!(plan(&cold).len(), MAX_PASSES as usize);
    }

    #[test]
    fn the_window_sits_in_the_middle_of_the_replay() {
        let ticks = 30 * 60 * TICK_RATE;
        let (start, end) = window(Some(ticks), Length::Standard).unwrap();
        assert_eq!(end - start, 90 * TICK_RATE);
        assert_eq!(start + (end - start) / 2, ticks / 2);
        assert_eq!(window(Some(ticks), Length::Whole), None);
        assert_eq!(window(Some(60 * TICK_RATE), Length::Long), None, "longer than the replay");
        assert_eq!(window(None, Length::Short), None);
    }

    #[test]
    fn a_demo_names_itself_for_timedemo() {
        let mut demo = Demo {
            source: DemoSource::Game,
            path: PathBuf::from("/g/game/citadel/replays/41234567.dem"),
            label: "41234567".into(),
            map: None,
            build: Some(6857),
            ticks: None,
            seconds: None,
            bytes: 1,
            sha256: None,
        };
        assert_eq!(demo.game_name(), "replays/41234567");
        assert!(!demo.other_build(Some("6857")));
        assert!(demo.other_build(Some("6860")));
        assert!(!demo.other_build(None));
        demo.source = DemoSource::Picked;
        assert_eq!(demo.game_name(), "replays/sigil_benchmark");
    }

    /// A search where slot 7 costs 20 FPS and everything else 1 FPS each.
    fn fps_without(off: &[u16]) -> f64 {
        let all: Vec<u16> = (1..=8).collect();
        let on = all.iter().filter(|s| !off.contains(s));
        200.0 - on.map(|s| if *s == 7 { 20.0 } else { 1.0 }).sum::<f64>()
    }

    #[test]
    fn the_cost_search_follows_the_costly_half() {
        let mut search = CostSearch::new(&[8, 1, 2, 3, 4, 5, 6, 7]);
        let mut launches = 0;
        while let Some(variant) = search.next() {
            let off = match &variant {
                Variant::Without(slots) => slots.clone(),
                _ => Vec::new(),
            };
            search.record(&variant, fps_without(&off));
            launches += 1;
            assert!(launches < 30);
        }
        assert_eq!(search.end, Some(CostEnd::Found { slot: 7 }));
        assert_eq!(launches, CostSearch::estimate(8));
        let groups = search.groups();
        assert_eq!(groups[0].0, vec![5, 6, 7, 8]);
        assert!((groups[0].1 - 23.0).abs() < 1e-9);
    }

    #[test]
    fn the_cost_search_stops_when_nothing_stands_out() {
        let mut search = CostSearch::new(&[1, 2, 3, 4]);
        search.record(&Variant::AsIs, 200.0);
        search.record(&Variant::Without(vec![1, 2]), 200.5);
        assert_eq!(search.next(), Some(Variant::Without(vec![3, 4])));
        search.record(&Variant::Without(vec![3, 4]), 201.0);
        assert_eq!(search.end, Some(CostEnd::NothingMeasurable));
        assert_eq!(search.next(), None);
        assert_eq!(CostSearch::new(&[]).end, Some(CostEnd::NothingMeasurable));
    }

    #[test]
    fn replays_are_found_in_the_game_and_in_sigil() {
        let root = std::env::temp_dir().join(format!("dah-bench-demos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let replays = session::replays_dir(&root.join("game"));
        std::fs::create_dir_all(&replays).unwrap();
        std::fs::write(replays.join("41234567.dem"), b"not a demo").unwrap();
        std::fs::write(replays.join(session::DEMO_COPY), b"ours").unwrap();
        std::fs::write(replays.join("notes.txt"), b"x").unwrap();
        let workspace = root.join("ws");
        let m = crate::demo::matches_dir(&workspace).join("40000001");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join(crate::demo::MATCH_DEMO_FILE_NAME), b"also not").unwrap();
        let demos = find_demos(Some(&root.join("game")), &workspace);
        let labels: Vec<(&str, DemoSource)> = demos.iter().map(|d| (d.label.as_str(), d.source)).collect();
        assert_eq!(labels, vec![("41234567", DemoSource::Game), ("40000001", DemoSource::Sigil)]);
        assert_eq!(demos[0].build, None, "an unreadable file is listed with nothing known");
        assert_eq!(demos[0].bytes, 10);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_single_suspect_is_measured_once() {
        let mut search = CostSearch::new(&[9]);
        search.record(&Variant::AsIs, 100.0);
        assert_eq!(search.next(), Some(Variant::Without(vec![9])));
        search.record(&Variant::Without(vec![9]), 120.0);
        assert_eq!(search.end, Some(CostEnd::Found { slot: 9 }));
        assert_eq!(CostSearch::estimate(1), 3);
        assert_eq!(CostSearch::estimate(32), 15);
    }
}
