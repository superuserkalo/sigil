//! Performance > Benchmark: Sigil's own in-game benchmark (`docs/design/benchmark.md`).
//!
//! The page picks a replay, a test and a length; Run hands the plan to one worker that runs
//! every pass in turn (`deadlock_core::benchmark::session`): write the cfg and the variant,
//! start Deadlock through Steam, follow `console.log` until the game quits, read the frames,
//! put everything back, store the pass. The running view reads the worker's shared state; the
//! result, comparison, history and exports read the stored runs.
//!
//! GAME WRITES and PROCESS: each pass changes the game folder (listed in the setup) and starts
//! the game. Run is the player's approval. A pass Sigil never finished is put back on the next
//! start ([`on_startup`]) or as soon as the game closes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use deadlock_core::benchmark::session::{self, End, FrameSource, Live, PassPrep};
use deadlock_core::benchmark::store::{self, Outcome, PassRecord, RunRecord};
use deadlock_core::benchmark::system::{self, Machine};
use deadlock_core::benchmark::{self as bench, Demo, DemoSource, Display, Length, PassPlan, RunSpec, Side, Test, Variant, metrics};
use rune::{TextEdit, Tx};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::fill;

pub mod card;
mod de;
pub mod scenes;
pub mod view;

pub use view::view;

/// This page's German table first, then the Performance page's and Sigil's shared ones.
pub fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    super::tr(text)
}

/// A label in the middle of a sentence: lower case in English, as written in German (nouns keep
/// their capital).
pub fn lower(text: &str) -> String {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German { text.to_string() } else { text.to_lowercase() }
}

/// The test the setup has picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pick {
    #[default]
    Measure,
    Vanilla,
    OneMod,
    Cost,
    Play,
}

impl Pick {
    pub const ALL: [Pick; 5] = [Pick::Measure, Pick::Vanilla, Pick::OneMod, Pick::Cost, Pick::Play];

    pub fn title(self) -> &'static str {
        match self {
            Pick::Measure => "Just measure",
            Pick::Vanilla => "Mods vs vanilla",
            Pick::OneMod => "One mod",
            Pick::Cost => "Find the costliest mod",
            Pick::Play => "Play session",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Pick::Measure => "The game as it is now. Run it again after any change and compare the two.",
            Pick::Vanilla => "What all your mods cost: vanilla and modded passes, taken in turns.",
            Pick::OneMod => "What one mod costs: the mod on and off, taken in turns.",
            Pick::Cost => "Halves your mods and follows the half that costs more, down to one mod.",
            Pick::Play => "Logs your frames while you play a match. Real, but never repeatable.",
        }
    }
}

/// Which part of the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Setup,
    History,
}

/// What the worker is doing, for the running view.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Preparing,
    Launching,
    Following(Live),
    Closing,
    Restoring,
    Resting,
}

impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Step::Preparing => "Writing the benchmark cfg",
            Step::Launching => "Handing the game to Steam",
            Step::Following(live) => live.stage.label(),
            Step::Closing => "Closing Deadlock",
            Step::Restoring => "Putting everything back",
            Step::Resting => "Resting before the next pass",
        }
    }
}

/// The worker's state, shared with the page.
#[derive(Clone, Debug)]
pub struct Shared {
    pub record: RunRecord,
    /// The fixed part of the plan; a cost search adds passes as it goes.
    pub plan: Vec<PassPlan>,
    /// The pass running now, from 1.
    pub at: u32,
    pub current: Option<PassPlan>,
    pub step: Step,
    pub started: Instant,
    /// Bumped on every change, so the page redraws only then.
    pub version: u64,
}

pub struct Running {
    pub shared: Arc<Mutex<Shared>>,
    pub stop: Arc<AtomicBool>,
    pub seen: u64,
}

/// A run id with its passes' frame times, as the loader hands them back.
pub type LoadedFrames = (String, Vec<(u32, Vec<f64>)>);

#[derive(Default)]
pub struct Page {
    pub open: bool,
    pub tab: Tab,
    pub demos: Option<Vec<Demo>>,
    pub demos_job: Job<Vec<Demo>>,
    /// The chosen replay, by path.
    pub chosen: Option<PathBuf>,
    pub pick: Pick,
    pub length: Length,
    pub passes: u32,
    pub no_warmup: bool,
    pub one_mod: Option<u16>,
    pub display: Display,
    /// The note being typed on a result.
    pub note: TextEdit,
    pub machine: Option<Machine>,
    pub machine_job: Job<Machine>,
    pub runs: Vec<RunRecord>,
    pub runs_loaded: bool,
    pub runs_job: Job<Vec<RunRecord>>,
    /// The run whose result is open.
    pub showing: Option<String>,
    /// Frame times per (run, pass), loaded for the graph.
    pub frames: HashMap<(String, u32), Vec<f64>>,
    pub frames_job: Job<LoadedFrames>,
    /// History: the runs picked for Compare, oldest first.
    pub picked: Vec<String>,
    pub running: Option<Running>,
    pub run_job: Job<RunRecord>,
    /// A marker was left while the game still ran: put it back when it closes.
    pub restore_pending: bool,
    pub restore_checked: Option<Instant>,
    pub restore_job: Job<Option<session::Restored>>,
    pub write_job: Job<String>,
    pub message: Option<(String, bool)>,
    /// The confirm before Delete, by run id.
    pub deleting: Option<String>,
}

fn data_dir() -> PathBuf {
    crate::data::data_dir()
}

fn simulated(st: &State) -> bool {
    st.demo || rune::is_headless()
}

/// Passes per side, as the setup shows it.
pub fn passes(p: &Page) -> u32 {
    if p.passes == 0 { bench::DEFAULT_PASSES } else { p.passes.clamp(1, bench::MAX_PASSES) }
}

// --- opening --------------------------------------------------------------------------------

/// Open the page from the Performance head.
pub fn open(st: &mut State) {
    st.performance.benchmark.open = true;
    st.performance.benchmark.message = None;
    start_reads(st);
}

pub fn close(st: &mut State) {
    st.performance.benchmark.open = false;
}

/// Replays, the machine and the stored runs, each once per visit.
pub fn start_reads(st: &mut State) {
    if simulated(st) {
        return;
    }
    let page = &mut st.performance.benchmark;
    if !page.demos_job.is_running() {
        let root = st.game_root.clone();
        let workspace = crate::data::workspace_dir();
        page.demos_job.start(st.wake.clone(), move || Ok(bench::find_demos(root.as_deref(), &workspace)));
    }
    if page.machine.is_none() && !page.machine_job.is_running() {
        page.machine_job.start(st.wake.clone(), || Ok(system::read_machine()));
    }
    start_runs(st);
}

fn start_runs(st: &mut State) {
    if simulated(st) || st.performance.benchmark.runs_job.is_running() {
        return;
    }
    st.performance.benchmark.runs_job.start(st.wake.clone(), || Ok(store::list_runs(&data_dir())));
}

/// Show a stored run's result and load its frames for the graph.
pub fn show(st: &mut State, id: String) {
    st.performance.benchmark.showing = Some(id.clone());
    load_frames(st, &id);
}

fn load_frames(st: &mut State, id: &str) {
    let sim = simulated(st);
    let page = &mut st.performance.benchmark;
    if sim || page.frames.keys().any(|(run, _)| run == id) || page.frames_job.is_running() {
        return;
    }
    let Some(run) = page.runs.iter().find(|r| r.spec.id == id).cloned() else { return };
    st.performance.benchmark.frames_job.start(st.wake.clone(), move || {
        let data = data_dir();
        let frames = run.passes.iter().map(|p| (p.index, store::load_frames(&data, &run.spec.id, p.index))).collect();
        Ok((run.spec.id.clone(), frames))
    });
}

// --- the run ----------------------------------------------------------------------------------

/// The enabled mods as (slot, name), by slot.
pub fn enabled_mods(s: &State) -> Vec<(u16, String)> {
    let mut mods: Vec<(u16, String)> = s
        .addons
        .addons
        .iter()
        .filter(|a| a.enabled)
        .map(|a| (a.slot, mod_name(s, a.slot)))
        .collect();
    mods.sort_by_key(|m| m.0);
    mods
}

pub fn mod_name(s: &State, slot: u16) -> String {
    s.installed
        .library
        .mods
        .iter()
        .find(|m| m.slot == slot && !m.name.trim().is_empty())
        .map(|m| m.name.clone())
        .unwrap_or_else(|| format!("pak{slot:02}"))
}

/// The chosen replay, when it is still in the list.
pub fn chosen_demo(s: &State) -> Option<&Demo> {
    let page = &s.performance.benchmark;
    let demos = page.demos.as_ref()?;
    match &page.chosen {
        Some(path) => demos.iter().find(|d| &d.path == path),
        None => demos.iter().find(|d| !d.other_build(current_build(s).as_deref())).or(demos.first()),
    }
}

/// The game's build, from the newest stored settings or steam.inf.
pub fn current_build(s: &State) -> Option<String> {
    s.game_root.as_deref().and_then(deadlock_core::host::read_build)
}

/// The spec the setup describes, or why there is none.
pub fn spec(s: &State) -> Result<RunSpec, &'static str> {
    let page = &s.performance.benchmark;
    let mods = enabled_mods(s);
    let test = match page.pick {
        Pick::Measure => Test::Measure,
        Pick::Vanilla => Test::ModsVsVanilla,
        Pick::OneMod => {
            let slot = page.one_mod.filter(|slot| mods.iter().any(|m| m.0 == *slot)).ok_or("Pick the mod to measure.")?;
            Test::OneMod { slot, name: mod_name(s, slot) }
        }
        Pick::Cost => {
            if mods.len() < 2 {
                return Err("The search needs two or more mods switched on.");
            }
            Test::CostSearch { pool: mods.iter().map(|m| m.0).collect() }
        }
        Pick::Play => Test::PlaySession,
    };
    if matches!(test, Test::ModsVsVanilla) && mods.is_empty() {
        return Err("No mods are switched on, so there is nothing to compare.");
    }
    let demo = match test {
        Test::PlaySession => None,
        _ => Some(chosen_demo(s).cloned().ok_or("Pick a replay first.")?),
    };
    let window = demo.as_ref().and_then(|d| bench::window(d.ticks, page.length));
    Ok(RunSpec {
        id: bench::next_run_id(),
        created_at: bench::now_rfc3339(),
        test,
        demo,
        length: page.length,
        window,
        passes: passes(page),
        warmup: !page.no_warmup && page.pick != Pick::Play,
        display: page.display,
    })
}

/// Why Run cannot start now: the core's checks and the gameinfo gate.
pub fn blockers(s: &State) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if simulated(s) {
        return out;
    }
    let Some(root) = s.game_root.as_deref() else {
        return vec![tr("Deadlock was not found. Set the game folder in Settings.").to_string()];
    };
    let running = s.launch.monitor.running();
    out.extend(session::blockers(root, &data_dir(), running).into_iter().map(|b| tr(b).to_string()));
    if deadlock_core::deploy::launch_blocker(root, false).is_some() {
        out.push(tr("gameinfo.gi needs a fix before the game can start. Open Launch and fix it first.").to_string());
    }
    out
}

/// What the setup line says Run will change.
pub fn changes(spec: &RunSpec, steam: bool) -> Vec<String> {
    let mut out = vec![tr("Writes sigil_benchmark.cfg into the game's cfg folder.").to_string()];
    if spec.demo.as_ref().is_some_and(|d| d.source != DemoSource::Game) {
        out.push(tr("Copies the replay into the game's replays folder.").to_string());
    }
    if !steam {
        out.push(tr("Adds one exec line to autoexec.cfg (Steam's program was not found).").to_string());
    }
    if let Display::Fixed { width, height } = spec.display {
        out.push(fill(tr("Starts the game in a {} window and puts your video settings back after each pass."), &[&format!("{width} x {height}")]));
    }
    match &spec.test {
        Test::ModsVsVanilla => out.push(tr("Sets your mods aside for the vanilla passes.").to_string()),
        Test::OneMod { name, .. } => out.push(fill(tr("Switches {} off for its passes."), &[name])),
        Test::CostSearch { .. } => out.push(tr("Switches groups of mods off, one group a pass.").to_string()),
        _ => {}
    }
    out.push(tr("Puts all of it back after every pass, and on the next start if anything stops it.").to_string());
    out
}

/// Minutes a run takes, roughly: a minute to start and load, the section, and the rest.
pub fn minutes(spec: &RunSpec) -> u32 {
    let section = spec.length.seconds().unwrap_or(15 * 60);
    let per = 75 + section + 5;
    ((bench::launches(spec) as u32 * per) as f32 / 60.0).ceil() as u32
}

/// RUN. Check, then hand the plan to the worker. The press is the approval.
pub fn start(st: &mut State) {
    let spec = match spec(st) {
        Ok(spec) => spec,
        Err(why) => {
            st.performance.benchmark.message = Some((tr(why).to_string(), true));
            return;
        }
    };
    let blocked = blockers(st);
    if let Some(first) = blocked.first() {
        st.performance.benchmark.message = Some((first.clone(), true));
        return;
    }
    let mods = enabled_mods(st);
    let plan = bench::plan(&spec);
    let record = RunRecord::new(spec.clone(), st.performance.benchmark.machine.clone().unwrap_or_default(), Default::default());
    let shared = Arc::new(Mutex::new(Shared {
        record,
        plan: plan.clone(),
        at: 1,
        current: plan.first().cloned(),
        step: Step::Preparing,
        started: Instant::now(),
        version: 1,
    }));
    let stop = Arc::new(AtomicBool::new(false));
    st.performance.benchmark.message = None;
    st.performance.benchmark.showing = None;
    st.performance.benchmark.running = Some(Running { shared: shared.clone(), stop: stop.clone(), seen: 0 });
    crate::screens::settings::telemetry::event(st, "benchmark:run", None);
    if simulated(st) {
        return;
    }
    let Some(root) = st.game_root.clone() else { return };
    let profile = deadlock_core::profiles::active_profile(&crate::data::workspace_dir());
    let wake = st.wake.clone();
    let mut roots = deadlock_core::discovery::default_steam_roots();
    if let Some(custom) = crate::screens::settings::options::get().steam_root.clone() {
        roots.insert(0, PathBuf::from(custom));
    }
    let ctx = Ctx { root, data: data_dir(), roots, mods, profile };
    st.performance.benchmark.run_job.start(wake.clone(), move || Ok(run_all(ctx, shared, stop, wake)));
}

/// Stop: the worker closes the game and puts everything back.
pub fn stop(st: &mut State) {
    if let Some(running) = &st.performance.benchmark.running {
        running.stop.store(true, Ordering::Relaxed);
        if simulated(st) {
            st.performance.benchmark.running = None;
        }
    }
}

struct Ctx {
    root: PathBuf,
    data: PathBuf,
    roots: Vec<PathBuf>,
    mods: Vec<(u16, String)>,
    profile: Option<String>,
}

fn update(shared: &Arc<Mutex<Shared>>, wake: &Option<rune::Waker>, change: impl FnOnce(&mut Shared)) {
    if let Ok(mut s) = shared.lock() {
        change(&mut s);
        s.version += 1;
    }
    if let Some(wake) = wake {
        wake();
    }
}

fn game_running() -> bool {
    deadlock_core::launch::read_game_process().running
}

/// Wait for the game to be gone, up to `limit`.
fn wait_gone(limit: Duration) -> bool {
    let since = Instant::now();
    while game_running() {
        if since.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    true
}

/// The worker: every pass in turn. Always ends with the marker restored.
fn run_all(ctx: Ctx, shared: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>, wake: Option<rune::Waker>) -> RunRecord {
    let mut record = shared.lock().map(|s| s.record.clone()).expect("benchmark state");
    record.machine = system::read_machine();
    record.settings = system::read_settings(&ctx.root, &ctx.roots);
    record.settings.profile = ctx.profile.clone();
    record.settings.mods = ctx.mods.clone();
    // The replay's hash, checked before every pass (`session::verify_demo`).
    if let Some(demo) = record.spec.demo.as_mut() {
        demo.sha256 = deadlock_core::servers::file_sha256(&demo.path).ok();
    }
    let _ = store::save_run(&ctx.data, &record);
    // The run's log folder: the video settings it started with.
    if let Some(video) = record.settings.video_path.as_ref().and_then(|p| std::fs::read(p).ok()) {
        let _ = store::save_log(&ctx.data, &record.spec.id, "video-settings.txt", &video);
    }
    update(&shared, &wake, |s| s.record = record.clone());

    let steam = session::steam_exe(&ctx.roots);
    let deadline = Instant::now() + bench::run_limit(&record.spec);
    let run_dir = store::run_dir(&ctx.data, &record.spec.id).ok();
    let plan = bench::plan(&record.spec);
    let console = deadlock_core::game_session::console_log_path(&ctx.root);
    let replay = record.spec.demo.is_some();
    let mut index = 0u32;
    let outcome = loop {
        let next = match plan.get(index as usize) {
            Some(p) => Some(p.clone()),
            None => record
                .cost
                .as_ref()
                .and_then(|c| c.next())
                .map(|variant| PassPlan { variant, side: Side::A, warmup: false }),
        };
        let Some(pass) = next else { break Outcome::Complete };
        if stop.load(Ordering::Relaxed) {
            break Outcome::Stopped;
        }
        if Instant::now() >= deadline {
            break Outcome::Failed(fill(tr("The run went past its time limit of {} minutes and was stopped."), &[&(bench::run_limit(&record.spec).as_secs() / 60).to_string()]));
        }
        index += 1;
        update(&shared, &wake, |s| {
            s.at = index;
            s.current = Some(pass.clone());
            s.step = Step::Preparing;
        });
        let demo = record.spec.demo.as_ref();
        let prep = PassPrep {
            run_id: record.spec.id.clone(),
            pass: index,
            demo: demo.map(Demo::game_name),
            copy_from: demo.filter(|d| d.source != DemoSource::Game).map(|d| d.path.clone()),
            window: record.spec.window,
            variant: pass.variant.clone(),
            autoexec_fallback: steam.is_none(),
            video_backup: match (record.spec.display, &record.settings.video_path, &run_dir) {
                (Display::Fixed { .. }, Some(video), Some(dir)) => Some((video.clone(), dir.join(format!("pass-{index:02}-video-before.txt")))),
                _ => None,
            },
        };
        let _ = store::save_log(&ctx.data, &record.spec.id, &format!("pass-{index:02}.cfg"), session::cfg_text(&prep).as_bytes());
        let wall = Instant::now();
        let started_at = bench::local_when(&bench::now_rfc3339());
        let since = SystemTime::now() - Duration::from_secs(2);
        let failed = |error: String, record: &mut RunRecord| {
            record.passes.push(PassRecord {
                index,
                variant: pass.variant.clone(),
                side: pass.side,
                warmup: pass.warmup,
                source: FrameSource::None,
                summary: None,
                timedemo: None,
                fps_cap: None,
                started_at: started_at.clone(),
                wall_seconds: wall.elapsed().as_secs_f64(),
                error: Some(error),
            });
        };
        let check = demo.and_then(|d| d.sha256.clone().map(|h| (d.path.clone(), h)));
        if let Some((path, sha)) = check
            && let Err(error) = session::verify_demo(&path, &sha)
        {
            failed(error.to_string(), &mut record);
            break Outcome::Failed(tr("The replay changed during the run, so the passes would not be comparable.").to_string());
        }
        if let Err(error) = session::prepare_pass(&ctx.root, &ctx.data, &prep) {
            let _ = session::restore(&ctx.data, false);
            failed(error.to_string(), &mut record);
            break Outcome::Failed(tr("A pass could not be set up. Nothing is left changed.").to_string());
        }
        // The launch checks every Sigil launch makes: the Mod and Write lines, mods past 99.
        let _ = deadlock_core::deploy::ensure_launch_search_paths(&ctx.root);
        if pass.variant != Variant::Vanilla {
            let _ = deadlock_core::addons::sync_overflow_mount(&deadlock_core::deploy::addons_dir(&ctx.root));
        }
        let mode = if pass.variant == Variant::Vanilla { deadlock_core::launch::MODE_VANILLA } else { deadlock_core::launch::MODE_MODDED };
        let _ = deadlock_core::launch::record_launch(&ctx.data, mode);
        update(&shared, &wake, |s| s.step = Step::Launching);
        if let Err(error) = session::launch(steam.as_deref(), record.spec.display) {
            let _ = session::restore(&ctx.data, false);
            failed(error.to_string(), &mut record);
            break Outcome::Failed(tr("Steam could not be started.").to_string());
        }
        let mut limits = session::Limits::default();
        limits.run = limits.run.min(deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(60)));
        let followed = session::follow(&console, replay, limits, &stop, game_running, |live| {
            let live = live.clone();
            let changed = shared.lock().map(|s| s.step != Step::Following(live.clone())).unwrap_or(false);
            if changed {
                update(&shared, &wake, |s| s.step = Step::Following(live));
            }
        });
        if matches!(followed.end, End::Stopped | End::TimedOut) {
            update(&shared, &wake, |s| s.step = Step::Closing);
            let _ = deadlock_core::launch::stop_game(true);
        }
        update(&shared, &wake, |s| s.step = Step::Restoring);
        wait_gone(Duration::from_secs(45));
        let _ = deadlock_core::launch::record_exit(&ctx.data);
        let restored = session::restore(&ctx.data, game_running());
        let (source, frames, source_file) = session::collect_frames(&ctx.root, since, &followed);
        let _ = store::save_log(&ctx.data, &record.spec.id, &format!("pass-{index:02}-console.log"), followed.lines.join("\n").as_bytes());
        if let Some(file) = source_file
            && let (Some(name), Ok(bytes)) = (file.file_name().map(|n| n.to_string_lossy().into_owned()), std::fs::read(&file))
        {
            let _ = store::save_log(&ctx.data, &record.spec.id, &format!("pass-{index:02}-{name}"), &bytes);
        }
        let summary = session::summary_for(source, &frames, &followed);
        let error = pass_error(&followed, replay, summary.is_some());
        if !frames.is_empty() {
            let _ = store::save_frames(&ctx.data, &record.spec.id, index, &frames);
        }
        if let (Some(cost), Some(s), None, false) = (record.cost.as_mut(), summary.as_ref(), &error, pass.warmup) {
            cost.record(&pass.variant, s.avg_fps);
        }
        record.passes.push(PassRecord {
            index,
            variant: pass.variant.clone(),
            side: pass.side,
            warmup: pass.warmup,
            source,
            summary,
            timedemo: followed.timedemo().cloned(),
            fps_cap: followed.fps_cap(),
            started_at,
            wall_seconds: wall.elapsed().as_secs_f64(),
            error: error.clone(),
        });
        let _ = store::save_run(&ctx.data, &record);
        update(&shared, &wake, |s| s.record = record.clone());
        if followed.end == End::Stopped {
            break Outcome::Stopped;
        }
        if restored.is_err() {
            break Outcome::Failed(tr("Sigil could not put everything back yet. It tries again on the next start.").to_string());
        }
        if let Some(error) = error {
            break Outcome::Failed(error);
        }
        if record.spec.test == Test::PlaySession {
            break Outcome::Complete;
        }
        update(&shared, &wake, |s| s.step = Step::Resting);
        let rest = Instant::now();
        while rest.elapsed() < Duration::from_secs(5) && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    // Whatever happened, nothing stays changed.
    if session::read_marker(&ctx.data).is_some() {
        if stop.load(Ordering::Relaxed) {
            let _ = deadlock_core::launch::stop_game(true);
        }
        wait_gone(Duration::from_secs(45));
        let _ = session::restore(&ctx.data, game_running());
    }
    record.outcome = Some(outcome);
    record.finished_at = Some(bench::now_rfc3339());
    let _ = store::save_run(&ctx.data, &record);
    record
}

/// Why a pass failed, in the player's words. None when it measured.
fn pass_error(followed: &session::Followed, replay: bool, measured: bool) -> Option<String> {
    match followed.end {
        End::Stopped => return Some(tr("Stopped.").to_string()),
        End::NeverStarted => return Some(tr("Deadlock did not start within three minutes. Steam may be updating it.").to_string()),
        _ => {}
    }
    if measured {
        return None;
    }
    if let Some(line) = followed.demo_error() {
        return Some(fill(tr("The game would not play the replay: {}"), &[line]));
    }
    if !followed.cfg_ran() {
        return Some(tr("The benchmark cfg did not run. Sigil read no line of it in console.log.").to_string());
    }
    if replay {
        Some(tr("Deadlock closed before the replay finished.").to_string())
    } else {
        Some(tr("No frames were logged during the session.").to_string())
    }
}

// --- start and poll -----------------------------------------------------------------------------

/// At Sigil's start: put back a pass the last session never finished.
pub fn on_startup(st: &mut State) {
    if simulated(st) || session::read_marker(&data_dir()).is_none() {
        return;
    }
    st.performance.benchmark.restore_pending = true;
    try_restore(st);
}

fn try_restore(st: &mut State) {
    let page = &mut st.performance.benchmark;
    if page.restore_job.is_running() || page.running.is_some() {
        return;
    }
    page.restore_checked = Some(Instant::now());
    page.restore_job.start(st.wake.clone(), || {
        if game_running() {
            return Err("running".to_string());
        }
        let data = data_dir();
        let marker = session::read_marker(&data);
        let done = session::restore(&data, false).map_err(|e| e.to_string())?;
        // The run the marker belongs to ended with Sigil: record that.
        if let Some(marker) = marker
            && let Some(mut run) = store::list_runs(&data).into_iter().find(|r| r.spec.id == marker.run_id)
            && run.outcome.is_none()
        {
            run.outcome = Some(Outcome::Stopped);
            let _ = store::save_run(&data, &run);
        }
        Ok(done)
    });
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    let page = &mut st.performance.benchmark;
    if let Some(result) = page.demos_job.try_take() {
        page.demos = Some(result.unwrap_or_default());
        changed = true;
    }
    if let Some(Ok(machine)) = page.machine_job.try_take() {
        page.machine = Some(machine);
        changed = true;
    }
    if let Some(result) = page.runs_job.try_take() {
        page.runs = result.unwrap_or_default();
        page.runs_loaded = true;
        changed = true;
    }
    if let Some(Ok((id, frames))) = page.frames_job.try_take() {
        for (pass, f) in frames {
            page.frames.insert((id.clone(), pass), f);
        }
        changed = true;
    }
    if let Some(result) = page.write_job.try_take() {
        page.message = Some(match result {
            Ok(line) => (line, false),
            Err(error) => (error, true),
        });
        changed = true;
    }
    if let Some(running) = page.running.as_mut()
        && let Ok(shared) = running.shared.lock()
        && shared.version != running.seen
    {
        running.seen = shared.version;
        changed = true;
    }
    let mut finished = None;
    if let Some(result) = page.run_job.try_take() {
        page.running = None;
        changed = true;
        match result {
            Ok(record) => {
                let id = record.spec.id.clone();
                let line = match &record.outcome {
                    Some(Outcome::Complete) => (tr("Benchmark done. Everything is back as it was.").to_string(), false),
                    Some(Outcome::Stopped) => (tr("Benchmark stopped. Everything is back as it was.").to_string(), false),
                    Some(Outcome::Failed(why)) => (why.clone(), true),
                    None => (String::new(), false),
                };
                page.runs.retain(|r| r.spec.id != id);
                page.runs.insert(0, record);
                page.message = Some(line);
                finished = Some(id);
            }
            Err(error) => page.message = Some((error, true)),
        }
    }
    if let Some(id) = finished {
        show(st, id);
        start_runs(st);
    }
    let page = &mut st.performance.benchmark;
    if let Some(result) = page.restore_job.try_take() {
        changed = true;
        match result {
            Ok(Some(done)) => {
                page.restore_pending = false;
                let detail = done.done.join(". ");
                let notice = if done.problems.is_empty() {
                    crate::notify::Notice::success(tr("Benchmark changes put back"))
                } else {
                    crate::notify::Notice::warning(tr("Some benchmark changes could not be put back yet"))
                };
                let detail = if done.problems.is_empty() { detail } else { done.problems.join(". ") };
                crate::notify::push(st, notice.detail(detail));
                crate::data::rescan_game(st);
            }
            Ok(None) => page.restore_pending = false,
            Err(_) => {}
        }
    }
    let page = &st.performance.benchmark;
    if page.restore_pending && page.restore_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
        try_restore(st);
    }
    changed
}

// --- exports ------------------------------------------------------------------------------------

fn run_by_id<'a>(s: &'a State, id: &str) -> Option<&'a RunRecord> {
    s.performance.benchmark.runs.iter().find(|r| r.spec.id == id)
}

fn save_text(_st: &mut State, tx: &mut Tx, name: String, kind: &'static str, ext: &'static str, make: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static) {
    let options = rune::FileDialogOptions { title: tr("Save").into(), file_name: Some(name), ..Default::default() }.filter(tr(kind), &[ext]);
    let mut make = Some(make);
    tx.save_file(options, move |st: &mut State, result, _tx| {
        let Ok(Some(path)) = result else { return };
        let Some(make) = make.take() else { return };
        st.performance.benchmark.write_job.start(st.wake.clone(), move || {
            let bytes = make()?;
            std::fs::write(&path, bytes)
                .map(|_| fill(tr("Saved {}."), &[&path.display().to_string()]))
                .map_err(|e| fill(tr("The file could not be saved: {}"), &[&e.to_string()]))
        });
    });
}

pub fn export_json(st: &mut State, tx: &mut Tx, id: &str) {
    let Some(run) = run_by_id(st, id).cloned() else { return };
    save_text(st, tx, format!("sigil-benchmark-{id}.json"), "JSON", "json", move || {
        store::export_json(&run).map(String::into_bytes).map_err(|e| e.to_string())
    });
}

pub fn export_csv(st: &mut State, tx: &mut Tx, id: &str) {
    let Some(run) = run_by_id(st, id).cloned() else { return };
    save_text(st, tx, format!("sigil-benchmark-{id}.csv"), "CSV", "csv", move || {
        let data = data_dir();
        let mut text = store::export_summary_csv(&run);
        text.push('\n');
        text.push_str(&store::export_frames_csv(&data, &run));
        Ok(text.into_bytes())
    });
}

pub fn save_card(st: &mut State, tx: &mut Tx, id: &str) {
    let Some(run) = run_by_id(st, id).cloned() else { return };
    let frames = graph_frames(st, &run);
    save_text(st, tx, format!("sigil-benchmark-{id}.png"), "PNG image", "png", move || card::png(&run, &frames));
}

/// The frames the graph and the card draw: the first measured pass of each side.
pub fn graph_frames(s: &State, run: &RunRecord) -> Vec<(Side, Vec<f64>)> {
    let mut out = Vec::new();
    for side in [Side::A, Side::B] {
        if let Some(p) = run.passes.iter().find(|p| !p.warmup && p.side == side && p.ok())
            && let Some(f) = s.performance.benchmark.frames.get(&(run.spec.id.clone(), p.index))
            && !f.is_empty()
        {
            out.push((side, f.clone()));
        }
    }
    out
}

/// Bins for the graph.
pub fn bins(frames: &[f64]) -> Vec<metrics::Bin> {
    metrics::graph(frames, 300)
}

/// Open logs: the run's folder (run.json, frames, each pass's console log, cfg and frame file).
pub fn open_logs(st: &mut State, id: &str) {
    if let Ok(dir) = store::run_dir(&data_dir(), id) {
        crate::data::reveal_path(st, dir.display().to_string());
    }
}

/// Append the typed note to a run. Earlier notes stay as they are.
pub fn add_note(st: &mut State, id: String) {
    let text = st.performance.benchmark.note.text.trim().to_string();
    if text.is_empty() {
        return;
    }
    st.performance.benchmark.note = TextEdit::default();
    let at = bench::now_rfc3339();
    if let Some(run) = st.performance.benchmark.runs.iter_mut().find(|r| r.spec.id == id) {
        run.notes.push(store::RunNote { at: at.clone(), text: text.clone() });
    }
    if simulated(st) {
        return;
    }
    st.performance.benchmark.write_job.start(st.wake.clone(), move || {
        store::add_note(&data_dir(), &id, &text, &at).map(|_| tr("Note added.").to_string()).map_err(|e| e.to_string())
    });
}

pub fn delete(st: &mut State, id: String) {
    st.performance.benchmark.deleting = None;
    st.performance.benchmark.runs.retain(|r| r.spec.id != id);
    st.performance.benchmark.picked.retain(|p| p != &id);
    if st.performance.benchmark.showing.as_deref() == Some(id.as_str()) {
        st.performance.benchmark.showing = None;
    }
    if simulated(st) {
        return;
    }
    st.performance.benchmark.write_job.start(st.wake.clone(), move || {
        store::delete_run(&data_dir(), &id).map(|_| tr("Run deleted.").to_string()).map_err(|e| e.to_string())
    });
}

/// History's Compare: pick up to two runs; a third replaces the older pick.
pub fn toggle_pick(st: &mut State, id: String) {
    let picked = &mut st.performance.benchmark.picked;
    if let Some(at) = picked.iter().position(|p| p == &id) {
        picked.remove(at);
        return;
    }
    picked.push(id);
    if picked.len() > 2 {
        picked.remove(0);
    }
    // Run ids are start times, so sorting puts the older run first (A).
    picked.sort();
}

/// Pick a `.dem` file outside the lists.
pub fn pick_file(_st: &mut State, tx: &mut Tx) {
    let options = rune::FileDialogOptions { title: tr("Choose a replay").into(), ..Default::default() }.filter(tr("Deadlock replay"), &["dem"]);
    tx.open_file(options, move |st: &mut State, result, _tx| {
        let Ok(Some(path)) = result else { return };
        st.performance.benchmark.chosen = Some(path.clone());
        st.performance.benchmark.demos_job.start(st.wake.clone(), {
            let root = st.game_root.clone();
            move || {
                let mut demos = bench::find_demos(root.as_deref(), &crate::data::workspace_dir());
                demos.retain(|d| d.path != path);
                demos.insert(0, bench::read_demo(&path, DemoSource::Picked));
                Ok(demos)
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_needs_a_replay_and_mods_for_its_tests() {
        let mut s = State::default();
        scenes::stage(&mut s);
        assert!(spec(&s).is_ok());
        s.performance.benchmark.pick = Pick::OneMod;
        assert_eq!(spec(&s).unwrap_err(), "Pick the mod to measure.");
        s.performance.benchmark.one_mod = Some(5);
        assert!(matches!(spec(&s).unwrap().test, Test::OneMod { slot: 5, .. }));
        s.performance.benchmark.demos = Some(Vec::new());
        assert_eq!(spec(&s).unwrap_err(), "Pick a replay first.");
        s.performance.benchmark.pick = Pick::Play;
        assert!(spec(&s).unwrap().demo.is_none());
    }

    #[test]
    fn the_setup_says_what_it_changes() {
        let mut s = State::default();
        scenes::stage(&mut s);
        s.performance.benchmark.pick = Pick::Vanilla;
        let spec = spec(&s).unwrap();
        let lines = changes(&spec, true);
        assert!(lines.iter().any(|l| l.contains("aside")));
        assert!(lines.last().unwrap().contains("back"));
        assert!(changes(&spec, false).iter().any(|l| l.contains("autoexec")));
        assert!(minutes(&spec) >= 7);
    }

    #[test]
    fn pass_errors_read_like_a_sentence() {
        let followed = session::Followed { events: Vec::new(), end: End::Exited, last_line: None, lines: Vec::new() };
        assert!(pass_error(&followed, true, false).unwrap().contains("cfg did not run"));
        assert_eq!(pass_error(&followed, true, true), None);
        let never = session::Followed { end: End::NeverStarted, ..followed.clone() };
        assert!(pass_error(&never, true, false).unwrap().contains("three minutes"));
    }

    #[test]
    fn compare_keeps_two_picks_in_age_order() {
        let mut s = State::default();
        scenes::stage_history(&mut s);
        let ids: Vec<String> = s.performance.benchmark.runs.iter().map(|r| r.spec.id.clone()).collect();
        toggle_pick(&mut s, ids[0].clone());
        toggle_pick(&mut s, ids[2].clone());
        toggle_pick(&mut s, ids[1].clone());
        let picked = &s.performance.benchmark.picked;
        assert_eq!(picked.len(), 2);
        assert!(picked[0] < picked[1], "{picked:?}");
    }
}
