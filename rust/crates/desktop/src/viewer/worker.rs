//! The render thread behind one viewport. It owns a `sigil_viewer::Viewer` (its own wgpu
//! device), loads the models, and renders into the view's `rune::Pixels` at the viewport's
//! device size. It renders only when something changed or a clip plays, paced to 60 frames a
//! second, and sleeps on its channel while the view is hidden or still. Setting the pixels
//! wakes only the windows that show them; the view runs again only for status changes (loaded,
//! failed, a capture saved), through the app's waker.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sigil_viewer::{Compare, LoadProgress, Sidecar, Viewer, ViewerError, fixture, model};

use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::log;

/// What a viewport shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A hero export: the glb and its sidecar (`<codename>.json`).
    Hero { glb: PathBuf, sidecar: Option<PathBuf> },
    /// A Hideout `scene.json`; its `hero` entry, when present, stands at the spawn.
    Scene { scene: PathBuf, hero: Option<(PathBuf, Option<PathBuf>)> },
    /// The bundled stand-in hero (demo scenes, a machine without Deadlock).
    StandIn(fixture::Variant),
}

pub enum Cmd {
    Size(u32, u32),
    Orbit(f32, f32),
    Zoom(f32),
    Pan(f32, f32, f32),
    ResetCamera,
    Play(bool),
    /// Seek to a fraction of the clip.
    Seek(f32),
    Speed(f32),
    Clip(String),
    Compare(Compare),
    Visible(bool),
    Capture(PathBuf),
    Stop,
    /// Panics on the render thread (tests of the catch).
    #[cfg(test)]
    Panic,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Loading,
    Ready,
    /// No GPU adapter on this machine.
    NoGpu(String),
    Failed(String),
}

/// What the render thread reports back for the view.
pub struct Shared {
    pub status: Status,
    pub clips: Vec<(String, f32)>,
    pub clip: Option<String>,
    pub has_before: bool,
    pub hold: bool,
    pub adapter: String,
    pub captured: Option<Result<PathBuf, String>>,
    /// The last frame's cost, for Settings > About and the bench note.
    pub frame_ms: f32,
    pub readback_ms: f32,
    pub changed: bool,
    /// Where the load is while `Loading` (a scene reports file by file).
    pub progress: Option<LoadProgress>,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { status: Status::Loading, clips: Vec::new(), clip: None, has_before: false, hold: false, adapter: String::new(), captured: None, frame_ms: 0.0, readback_ms: 0.0, changed: false, progress: None }
    }
}

#[derive(Clone)]
pub struct Outputs {
    pub pixels: rune::Pixels,
    pub shared: Arc<Mutex<Shared>>,
    /// Playhead as a fraction, for the seek bar's live fill.
    pub progress: rune::Live<[f32; 2]>,
    pub knob: rune::Live<[f32; 2]>,
    pub time: rune::LiveText,
    pub seek_width: f32,
    pub wake: Option<rune::Waker>,
    /// Set when the view closes: the load stops at its next file or placement.
    pub cancel: Arc<AtomicBool>,
    /// The time spent loading, "0:12", ticking while `Loading`.
    pub elapsed: rune::LiveText,
}

fn report(out: &Outputs, f: impl FnOnce(&mut Shared)) {
    if let Ok(mut s) = out.shared.lock() {
        f(&mut s);
        s.changed = true;
    }
    if let Some(wake) = &out.wake {
        wake();
    }
}

fn load_model(path: &std::path::Path) -> Result<model::Model, String> {
    model::load_path(path).map_err(|e| e.0)
}

/// Opens the device and loads `source` (and `before`). Shared by the thread and the
/// synchronous headless path. Stops with `ViewerError::Cancelled` when `cancel` is set, and
/// writes what it found (adapter, counts, sizes, timings, what it left out) to Sigil's log, so a
/// report from a machine we cannot reach comes with data.
pub fn prepare(source: &Source, before: Option<&Source>, cancel: &AtomicBool, progress: &mut dyn FnMut(LoadProgress)) -> Result<Viewer, ViewerError> {
    let started = Instant::now();
    let mut viewer = Viewer::new()?;
    log::info(format!("viewer: device {} in {} ms; {}", viewer.adapter(), started.elapsed().as_millis(), viewer.notes().join("; ")));
    let mut load_hero = |viewer: &mut Viewer, s: &Source| -> Result<(), ViewerError> {
        match s {
            Source::Hero { glb, sidecar } => {
                let side = sidecar.as_deref().map(Sidecar::read).unwrap_or_else(|| Sidecar::read(&glb.with_extension("json")));
                viewer.set_hero(load_model(glb).map_err(ViewerError::Load)?, &side);
            }
            Source::StandIn(v) => {
                let side = Sidecar::parse(&fixture::sidecar_json());
                viewer.set_hero(model::load_slice(&fixture::hero_glb(*v)).map_err(|e| ViewerError::Load(e.0))?, &side);
            }
            Source::Scene { scene, hero } => {
                let read = Instant::now();
                let assets = sigil_viewer::load_scene_with(scene, cancel, progress)?;
                log::info(format!(
                    "viewer: scene {} read in {} ms: {} placements, {} files, {} failed, {} left out",
                    scene.display(),
                    read.elapsed().as_millis(),
                    assets.desc.models.len(),
                    assets.files,
                    assets.errors.len(),
                    assets.skipped.len()
                ));
                for line in assets.errors.iter().chain(&assets.skipped).take(20) {
                    log::info(format!("viewer: scene: {line}"));
                }
                // The hero first: it is what the player came to see, so the room's budget never
                // leaves it without textures. `set_scene` moves it to the spawn.
                let from_scene = assets.desc.hero.as_ref().map(|h| (h.glb.clone(), h.sidecar.clone()));
                if let Some((glb, sidecar)) = hero.clone().or(from_scene) {
                    let side = sidecar.as_deref().map(Sidecar::read).unwrap_or_default();
                    viewer.set_hero(load_model(&glb).map_err(ViewerError::Load)?, &side);
                }
                let upload = Instant::now();
                let report = viewer.set_scene_with(assets, cancel, progress);
                if report.cancelled {
                    return Err(ViewerError::Cancelled);
                }
                let usage = viewer.usage();
                log::info(format!(
                    "viewer: scene on the GPU in {} ms: {} placements of {} files, {} triangles, {} textures ({} MB, {} downscaled), {} MB of buffers, {} left out",
                    upload.elapsed().as_millis(),
                    report.placements,
                    report.files,
                    report.triangles,
                    usage.textures,
                    usage.texture_bytes >> 20,
                    usage.downscaled,
                    usage.buffer_bytes >> 20,
                    usage.skipped_count
                ));
                for line in usage.skipped.iter().take(20) {
                    log::info(format!("viewer: left out: {line}"));
                }
            }
        }
        Ok(())
    };
    load_hero(&mut viewer, source)?;
    if let Some(b) = before {
        let (m, side) = match b {
            Source::Hero { glb, sidecar } => (load_model(glb), sidecar.as_deref().map(Sidecar::read).unwrap_or_else(|| Sidecar::read(&glb.with_extension("json")))),
            Source::StandIn(v) => (model::load_slice(&fixture::hero_glb(*v)).map_err(|e| e.0), Sidecar::parse(&fixture::sidecar_json())),
            Source::Scene { .. } => (Err("a scene cannot be the before side".into()), Sidecar::default()),
        };
        // A failed "before" leaves the "after" up; the switch just stays off.
        if let Ok(m) = m {
            viewer.set_before(Some(m), &side);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(ViewerError::Cancelled);
    }
    log::info(format!("viewer: loaded in {} ms", started.elapsed().as_millis()));
    Ok(viewer)
}

pub fn ready(out: &Outputs, viewer: &Viewer) {
    let clips = viewer.clips();
    let clip = viewer.clip();
    let has_before = viewer.has_before();
    let hold = viewer.playmode() == sigil_viewer::Playmode::Hold;
    let adapter = viewer.adapter();
    report(out, |s| {
        s.status = Status::Ready;
        s.clips = clips;
        s.clip = clip;
        s.has_before = has_before;
        s.hold = hold;
        s.adapter = adapter;
    });
}

pub fn publish_live(out: &Outputs, viewer: &Viewer) {
    let d = viewer.duration();
    let f = if d > 0.0 { (viewer.playback.time / d).clamp(0.0, 1.0) } else { 0.0 };
    out.progress.set([0.0, f]);
    out.knob.set([f * (out.seek_width - super::KNOB), 0.0]);
    out.time.set(format!("{} / {}", clock(viewer.playback.time), clock(d)));
}

pub fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0);
    format!("{}:{:04.1}", (s / 60.0) as u32, s % 60.0)
}

/// The render thread. A panic or a GPU error anywhere in it becomes a "could not open" state with
/// the reason, never a view stuck on Loading (the first Hideout on Windows sat there, then the app
/// went down).
pub fn run(source: Source, before: Option<Source>, rx: Receiver<Cmd>, out: Outputs) {
    let guard = out.clone();
    tick_while_loading(&out);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || run_inner(source, before, rx, out)));
    if let Err(panic) = outcome {
        let reason = panic_reason(panic.as_ref());
        log::error(format!("viewer: render thread stopped: {reason}"));
        report(&guard, |s| s.status = Status::Failed(reason));
    }
}

/// The message a panic carried.
pub fn panic_reason(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "the 3D view stopped unexpectedly".to_string())
}

/// Updates the elapsed time on the loading panel twice a second until the load ends or the view
/// closes. Its own thread: the render thread is busy loading.
fn tick_while_loading(out: &Outputs) {
    let (shared, elapsed, cancel) = (out.shared.clone(), out.elapsed.clone(), out.cancel.clone());
    let started = Instant::now();
    let _ = std::thread::Builder::new().name("sigil-viewer-clock".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let loading = shared.lock().map(|s| s.status == Status::Loading).unwrap_or(false);
            if !loading || cancel.load(Ordering::Relaxed) {
                return;
            }
            let secs = started.elapsed().as_secs();
            elapsed.set(format!("{}:{:02}", secs / 60, secs % 60));
        }
    });
}

fn gpu_failed(out: &Outputs, viewer: &Viewer) -> bool {
    match viewer.gpu_error() {
        Some(error) => {
            log::error(format!("viewer: GPU error on {}: {error}", viewer.adapter()));
            report(out, |s| s.status = Status::Failed(fill(tr("The graphics card reported an error: {}"), &[&error])));
            true
        }
        None => false,
    }
}

/// Progress for the loading panel, at most ten times a second (a scene reads hundreds of files).
fn progress_reporter(out: &Outputs) -> impl FnMut(LoadProgress) + '_ {
    let mut last: Option<(Instant, sigil_viewer::LoadStage)> = None;
    move |p: LoadProgress| {
        let due = last.is_none_or(|(at, stage)| stage != p.stage || at.elapsed() >= Duration::from_millis(100) || p.done == p.of);
        if due {
            last = Some((Instant::now(), p.stage));
            report(out, |s| s.progress = Some(p));
        }
    }
}

fn run_inner(source: Source, before: Option<Source>, rx: Receiver<Cmd>, out: Outputs) {
    let prepared = {
        let mut progress = progress_reporter(&out);
        prepare(&source, before.as_ref(), &out.cancel, &mut progress)
    };
    let mut viewer = match prepared {
        Ok(v) => v,
        // The view closed while it loaded: nobody is waiting.
        Err(ViewerError::Cancelled) => {
            log::info("viewer: load cancelled");
            return;
        }
        Err(ViewerError::NoGpu(e)) => {
            log::info(format!("viewer: no GPU adapter: {e}"));
            report(&out, |s| s.status = Status::NoGpu(e));
            return;
        }
        Err(e) => {
            log::error(format!("viewer: could not open: {e}"));
            report(&out, |s| s.status = Status::Failed(e.to_string()));
            return;
        }
    };
    if gpu_failed(&out, &viewer) {
        return;
    }
    ready(&out, &viewer);
    let frame = Duration::from_micros(16_667);
    let (mut w, mut h) = (0u32, 0u32);
    let mut visible = true;
    let mut dirty = true;
    let mut last = Instant::now();
    let mut first_frame = true;
    loop {
        // Sleep until the next frame while playing; otherwise until a command arrives.
        let animating = visible && viewer.animating() && w > 0;
        let first = if animating {
            match rx.recv_timeout(frame.saturating_sub(last.elapsed())) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        let mut capture = None;
        for cmd in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok())) {
            match cmd {
                Cmd::Size(nw, nh) => {
                    (w, h) = (nw, nh);
                }
                Cmd::Orbit(dx, dy) => viewer.camera.orbit(dx, dy),
                Cmd::Zoom(f) => viewer.camera.zoom(f),
                Cmd::Pan(dx, dy, height) => viewer.camera.pan(dx, dy, height),
                Cmd::ResetCamera => viewer.reset_camera(),
                Cmd::Play(on) => {
                    if on && viewer.playmode() == sigil_viewer::Playmode::Hold && viewer.playback.time >= viewer.duration() {
                        viewer.seek(0.0);
                    }
                    viewer.playback.playing = on;
                    last = Instant::now();
                }
                // Seeking pauses, as Grimoire's strip does.
                Cmd::Seek(f) => {
                    viewer.playback.playing = false;
                    viewer.seek(f.clamp(0.0, 1.0) * viewer.duration());
                }
                Cmd::Speed(k) => viewer.playback.speed = k,
                Cmd::Clip(name) => {
                    viewer.set_clip(&name);
                    let hold = viewer.playmode() == sigil_viewer::Playmode::Hold;
                    report(&out, |s| {
                        s.clip = Some(name);
                        s.hold = hold;
                    });
                }
                Cmd::Compare(c) => viewer.compare = c,
                Cmd::Visible(v) => {
                    visible = v;
                    last = Instant::now();
                }
                Cmd::Capture(path) => capture = Some(path),
                Cmd::Stop => return,
                #[cfg(test)]
                Cmd::Panic => panic!("a test panic on the render thread"),
            }
            dirty = true;
        }
        if !visible || w == 0 || h == 0 {
            continue;
        }
        if let Some(path) = capture {
            let result = viewer.capture_png(w, h).map_err(|e| e.to_string()).and_then(|png| {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, png).map_err(|e| e.to_string()).map(|_| path.clone())
            });
            report(&out, |s| s.captured = Some(result));
        }
        if dirty || viewer.animating() {
            let dt = last.elapsed().as_secs_f32().min(0.1);
            last = Instant::now();
            let started = Instant::now();
            // The frame is what the device renders: a viewport past its edge limit is clamped.
            let edge = viewer.max_edge();
            let (fw, fh) = (w.min(edge), h.min(edge));
            let rgba = viewer.render(fw, fh, dt).to_vec();
            let t = viewer.timings();
            if gpu_failed(&out, &viewer) {
                return;
            }
            if first_frame {
                first_frame = false;
                log::info(format!("viewer: first frame {fw}x{fh} in {} ms (GPU wait {:.1} ms)", started.elapsed().as_millis(), t.gpu_wait_ms));
            }
            out.pixels.set(fw, fh, rgba);
            publish_live(&out, &viewer);
            if let Ok(mut s) = out.shared.lock() {
                s.frame_ms = started.elapsed().as_secs_f32() * 1000.0;
                s.readback_ms = t.gpu_wait_ms + t.unpad_ms;
            }
            dirty = false;
        }
    }
}
