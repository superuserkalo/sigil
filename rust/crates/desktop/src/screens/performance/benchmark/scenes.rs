//! Headless scenes for the Benchmark page: setup, running, results, a comparison, the cost
//! search, history and compare. Fixtures only: no game file is read and nothing is launched.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deadlock_core::benchmark::session::{FrameSource, Live, Stage};
use deadlock_core::benchmark::store::{Outcome, PassRecord, RunRecord};
use deadlock_core::benchmark::system::{Machine, Settings};
use deadlock_core::benchmark::{self as bench, CostSearch, Demo, DemoSource, Length, RunSpec, Side, Test, Variant, metrics};

use super::{Pick, Running, Shared, Step, Tab};
use crate::app::State;
use crate::model::Screen;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("benchmark-setup", setup),
    ("benchmark-setup-vanilla", setup_vanilla),
    ("benchmark-setup-empty", setup_empty),
    ("benchmark-running", running),
    ("benchmark-result", result),
    ("benchmark-compare", compare_result),
    ("benchmark-cost", cost),
    ("benchmark-history", history),
    ("benchmark-history-compare", history_compare),
    ("benchmark-failed", failed),
];

/// A small deterministic noise source, so every snapshot draws the same frames.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / (u32::MAX as f64 / 2.0)
    }
}

/// Frame times for 90 seconds at about `fps`, heavier in the middle fight, with a few hitches.
pub fn frames(fps: f64, seed: u64) -> Vec<f64> {
    let mut noise = Noise(seed);
    let mut out = Vec::new();
    let mut clock: f64 = 0.0;
    while clock < 90_000.0 {
        let t: f64 = clock / 90_000.0;
        let fight = (-((t - 0.55) * 7.0).powi(2)).exp();
        let base = 1000.0 / fps * (1.0 + 0.32 * fight);
        let mut ms = base * (0.92 + 0.16 * noise.next());
        if noise.next() > 0.9985 {
            ms += 14.0 + 30.0 * noise.next();
        }
        out.push(ms);
        clock += ms;
    }
    out
}

fn demo() -> Demo {
    Demo {
        source: DemoSource::Game,
        path: PathBuf::from("/games/deadlock/game/citadel/replays/41234567.dem"),
        label: "41234567".into(),
        map: Some("dl_midtown".into()),
        build: Some(6857),
        ticks: Some(34 * 60 * bench::TICK_RATE),
        seconds: Some(34.0 * 60.0),
        bytes: 182_000_000,
        sha256: Some("9f2c41d0a8e3b7c65d1e0f4a2b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c".into()),
    }
}

fn demos() -> Vec<Demo> {
    vec![
        demo(),
        Demo {
            label: "41198220".into(),
            path: PathBuf::from("/games/deadlock/game/citadel/replays/41198220.dem"),
            ticks: Some(41 * 60 * bench::TICK_RATE),
            seconds: Some(41.0 * 60.0),
            ..demo()
        },
        Demo {
            source: DemoSource::Sigil,
            label: "40877311".into(),
            path: PathBuf::from("/sigil/matches/40877311/match.dem"),
            build: Some(6801),
            ticks: Some(29 * 60 * bench::TICK_RATE),
            seconds: Some(29.0 * 60.0),
            ..demo()
        },
    ]
}

fn machine() -> Machine {
    Machine {
        os: "Windows 11 Pro 24H2 build 26100".into(),
        cpu: Some("AMD Ryzen 7 7800X3D 8-Core Processor".into()),
        threads: Some(16),
        ram_mb: Some(32_712),
        gpu: Some("NVIDIA GeForce RTX 4070".into()),
        driver: Some("32.0.15.6614".into()),
        driver_date: Some("10-28-2026".into()),
    }
}

fn settings(build: &str) -> Settings {
    let video = [
        ("defaultres", "2560"),
        ("defaultresheight", "1440"),
        ("fullscreen", "0"),
        ("nowindowborder", "1"),
        ("r_citadel_fsr", "2"),
        ("mat_viewportscale", "0.77"),
        ("r_particle_max_detail_level", "2"),
        ("shaderquality", "1"),
    ];
    Settings {
        build: Some(build.into()),
        video: video.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        video_path: None,
        performance_config: Some("optilock-fps".into()),
        profile: Some("ranked".into()),
        mods: MODS.iter().map(|(s, n)| (*s, n.to_string())).collect(),
    }
}

const MODS: &[(u16, &str)] = &[
    (1, "Clean HUD"),
    (2, "Haze Neon Skin"),
    (3, "Quiet Soul Orbs"),
    (4, "Midtown Night Sky"),
    (5, "Particle Lite"),
    (6, "Abrams Knight"),
    (7, "Vindicta Raven"),
    (8, "Kill Sounds Pack"),
];

/// The page open on Setup with fixtures: replays, mods, the machine.
pub fn stage(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Performance;
    s.game_detected = true;
    s.game_root = Some(PathBuf::from("/games/deadlock"));
    s.performance.loaded = true;
    s.addons.addons = MODS
        .iter()
        .map(|(slot, _)| crate::model::AddonRow {
            slot: *slot,
            file: format!("pak{slot:02}_dir.vpk"),
            size_bytes: 12_000_000,
            chunk_files: Vec::new(),
            entry_count: Some(40),
            enabled: true,
        })
        .collect();
    s.addons.scanned = true;
    s.installed.library.mods = MODS
        .iter()
        .map(|(slot, name)| deadlock_core::import::ModMeta {
            slot: *slot,
            size_bytes: 12_000_000,
            name: name.to_string(),
            author: None,
            thumbnail_url: None,
            gamebanana_id: None,
            source: "browse".into(),
            recorded_at: String::new(),
        })
        .collect();
    let page = &mut s.performance.benchmark;
    page.open = true;
    page.tab = Tab::Setup;
    page.demos = Some(demos());
    page.machine = Some(machine());
    page.runs_loaded = true;
}

fn spec(id: &str, at: &str, test: Test) -> RunSpec {
    let d = demo();
    RunSpec {
        id: id.into(),
        created_at: at.into(),
        test,
        window: bench::window(d.ticks, Length::Standard),
        demo: Some(d),
        length: Length::Standard,
        passes: 3,
        warmup: true,
        display: bench::Display::Game,
    }
}

fn pass(index: u32, variant: Variant, side: Side, warmup: bool, fps: f64) -> (PassRecord, Vec<f64>) {
    let f = frames(fps, index as u64 * 7919 + fps as u64);
    let record = PassRecord {
        index,
        variant,
        side,
        warmup,
        source: FrameSource::TimedemoProfile,
        summary: Some(metrics::summarize(&f)),
        timedemo: None,
        fps_cap: Some(0.0),
        started_at: "18:02".into(),
        wall_seconds: 171.0,
        error: None,
    };
    (record, f)
}

/// A finished run with its frames loaded into the page.
fn add_run(s: &mut State, mut run: RunRecord, passes: Vec<(PassRecord, Vec<f64>)>) {
    for (p, f) in passes {
        s.performance.benchmark.frames.insert((run.spec.id.clone(), p.index), f);
        run.passes.push(p);
    }
    run.outcome.get_or_insert(Outcome::Complete);
    run.finished_at = Some(run.spec.created_at.clone());
    s.performance.benchmark.runs.push(run);
    s.performance.benchmark.runs.sort_by(|a, b| b.spec.id.cmp(&a.spec.id));
}

fn measure_run(s: &mut State, id: &str, at: &str, build: &str, fps: f64, note: &str, driver: &str) {
    let mut run = RunRecord::new(spec(id, at, Test::Measure), machine(), settings(build));
    run.machine.driver = Some(driver.into());
    run.note = note.into();
    let passes = vec![
        pass(1, Variant::AsIs, Side::B, true, fps * 0.93),
        pass(2, Variant::AsIs, Side::A, false, fps),
        pass(3, Variant::AsIs, Side::A, false, fps * 1.01),
        pass(4, Variant::AsIs, Side::A, false, fps * 0.995),
    ];
    add_run(s, run, passes);
}

fn vanilla_run(s: &mut State) {
    let run = RunRecord::new(spec("b20261121-180200", "2026-11-21T18:02:00Z", Test::ModsVsVanilla), machine(), settings("6857"));
    let mut passes = vec![pass(1, Variant::AsIs, Side::B, true, 152.0)];
    for (i, (a, b)) in [(181.0, 163.0), (183.5, 164.2), (180.2, 162.4)].iter().enumerate() {
        passes.push(pass(2 + 2 * i as u32, Variant::Vanilla, Side::A, false, *a));
        passes.push(pass(3 + 2 * i as u32, Variant::AsIs, Side::B, false, *b));
    }
    add_run(s, run, passes);
}

fn setup(s: &mut State) {
    stage(s);
}

fn setup_vanilla(s: &mut State) {
    stage(s);
    s.performance.benchmark.pick = Pick::Vanilla;
    s.performance.benchmark.chosen = Some(demos()[1].path.clone());
    s.performance.benchmark.display = bench::Display::Fixed { width: 1920, height: 1080 };
}

fn setup_empty(s: &mut State) {
    stage(s);
    s.performance.benchmark.demos = Some(Vec::new());
}

fn running(s: &mut State) {
    stage(s);
    let spec = spec("b20261121-180200", "2026-11-21T18:02:00Z", Test::ModsVsVanilla);
    let plan = bench::plan(&spec);
    let mut record = RunRecord::new(spec, machine(), settings("6857"));
    record.passes.push(pass(1, Variant::AsIs, Side::B, true, 152.0).0);
    record.passes.push(pass(2, Variant::Vanilla, Side::A, false, 181.0).0);
    record.passes.push(pass(3, Variant::AsIs, Side::B, false, 163.0).0);
    let shared = Shared {
        record,
        current: plan.get(3).cloned(),
        plan,
        at: 4,
        step: Step::Following(Live {
            stage: Stage::Measuring,
            last_line: Some("Requesting timedemo of replays/41234567".into()),
            elapsed: Duration::from_secs(64),
        }),
        started: Instant::now() - Duration::from_secs(9 * 60 + 12),
        version: 1,
    };
    s.performance.benchmark.running = Some(Running {
        shared: Arc::new(Mutex::new(shared)),
        stop: Arc::new(AtomicBool::new(false)),
        seen: 1,
    });
}

/// The result page of one measure run, frames loaded.
pub fn stage_result(s: &mut State) {
    stage(s);
    measure_run(s, "b20261121-171500", "2026-11-21T17:15:00Z", "6857", 163.0, "", "32.0.15.6614");
    s.performance.benchmark.runs[0].notes.push(deadlock_core::benchmark::store::RunNote {
        at: "2026-11-21T17:31:00Z".into(),
        text: "Discord and the browser closed. Particle Lite on.".into(),
    });
    let id = s.performance.benchmark.runs[0].spec.id.clone();
    s.performance.benchmark.showing = Some(id);
}

fn result(s: &mut State) {
    stage_result(s);
}

fn compare_result(s: &mut State) {
    stage(s);
    vanilla_run(s);
    s.performance.benchmark.showing = Some("b20261121-180200".into());
}

fn cost(s: &mut State) {
    stage(s);
    let pool: Vec<u16> = MODS.iter().map(|m| m.0).collect();
    let mut run = RunRecord::new(spec("b20261121-190000", "2026-11-21T19:00:00Z", Test::CostSearch { pool: pool.clone() }), machine(), settings("6857"));
    let cost_of = |slot: u16| match slot {
        2 => 11.0,
        6 => 3.0,
        _ => 0.6,
    };
    let fps_without = |off: &[u16]| 182.0 - pool.iter().filter(|s| !off.contains(s)).map(|s| cost_of(*s)).sum::<f64>();
    let mut search = CostSearch::new(&pool);
    let mut passes = vec![pass(1, Variant::AsIs, Side::B, true, 150.0)];
    let mut index = 2;
    while let Some(variant) = search.next() {
        let off = match &variant {
            Variant::Without(s) => s.clone(),
            _ => Vec::new(),
        };
        let fps = fps_without(&off);
        search.record(&variant, fps);
        passes.push(pass(index, variant, Side::A, false, fps));
        index += 1;
    }
    run.cost = Some(search);
    add_run(s, run, passes);
    s.performance.benchmark.showing = Some("b20261121-190000".into());
}

/// Runs across a driver update, plus a comparison.
pub fn stage_history(s: &mut State) {
    stage(s);
    measure_run(s, "b20261112-201000", "2026-11-12T20:10:00Z", "6857", 158.0, "", "32.0.15.6590");
    measure_run(s, "b20261114-191500", "2026-11-14T19:15:00Z", "6857", 159.5, "Before the driver update", "32.0.15.6590");
    measure_run(s, "b20261118-213000", "2026-11-18T21:30:00Z", "6857", 166.0, "New driver", "32.0.15.6614");
    measure_run(s, "b20261121-171500", "2026-11-21T17:15:00Z", "6857", 163.0, "", "32.0.15.6614");
    vanilla_run(s);
    s.performance.benchmark.tab = Tab::History;
}

fn history(s: &mut State) {
    stage_history(s);
}

fn history_compare(s: &mut State) {
    stage_history(s);
    if let Some(r) = s.performance.benchmark.runs.iter_mut().find(|r| r.spec.id == "b20261118-213000") {
        r.settings.video.insert("r_citadel_fsr".into(), "0".into());
        r.settings.mods.retain(|m| m.0 != 6);
    }
    s.performance.benchmark.picked = vec!["b20261114-191500".into(), "b20261118-213000".into()];
}

fn failed(s: &mut State) {
    stage(s);
    let why = "The game would not play the replay: demo recorded for game 'citadel' build 6857, current is 6860";
    let mut run = RunRecord::new(spec("b20261121-200000", "2026-11-21T20:00:00Z", Test::Measure), machine(), settings("6860"));
    run.passes.push(PassRecord {
        index: 1,
        variant: Variant::AsIs,
        side: Side::B,
        warmup: true,
        source: FrameSource::None,
        summary: None,
        timedemo: None,
        fps_cap: None,
        started_at: "20:00".into(),
        wall_seconds: 48.0,
        error: Some(why.into()),
    });
    run.outcome = Some(Outcome::Failed(why.into()));
    add_run(s, run, Vec::new());
    s.performance.benchmark.showing = Some("b20261121-200000".into());
}
