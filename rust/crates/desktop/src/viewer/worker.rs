//! The render thread behind one viewport. It owns a `sigil_viewer::Viewer` (its own wgpu
//! device), loads the models, and renders into the view's `rune::Pixels` at the viewport's
//! device size. It renders only when something changed or a clip plays, paced to 60 frames a
//! second, and sleeps on its channel while the view is hidden or still. Setting the pixels
//! wakes only the windows that show them; the view runs again only for status changes (loaded,
//! failed, a capture saved), through the app's waker.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sigil_viewer::{Compare, Sidecar, Viewer, ViewerError, fixture, model};

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
}

impl Default for Shared {
    fn default() -> Self {
        Shared { status: Status::Loading, clips: Vec::new(), clip: None, has_before: false, hold: false, adapter: String::new(), captured: None, frame_ms: 0.0, readback_ms: 0.0, changed: false }
    }
}

pub struct Outputs {
    pub pixels: rune::Pixels,
    pub shared: Arc<Mutex<Shared>>,
    /// Playhead as a fraction, for the seek bar's live fill.
    pub progress: rune::Live<[f32; 2]>,
    pub knob: rune::Live<[f32; 2]>,
    pub time: rune::LiveText,
    pub seek_width: f32,
    pub wake: Option<rune::Waker>,
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
/// synchronous headless path.
pub fn prepare(source: &Source, before: Option<&Source>) -> Result<Viewer, ViewerError> {
    let mut viewer = Viewer::new()?;
    let load_hero = |viewer: &mut Viewer, s: &Source| -> Result<(), String> {
        match s {
            Source::Hero { glb, sidecar } => {
                let side = sidecar.as_deref().map(Sidecar::read).unwrap_or_else(|| Sidecar::read(&glb.with_extension("json")));
                viewer.set_hero(load_model(glb)?, &side);
            }
            Source::StandIn(v) => {
                let side = Sidecar::parse(&fixture::sidecar_json());
                viewer.set_hero(model::load_slice(&fixture::hero_glb(*v)).map_err(|e| e.0)?, &side);
            }
            Source::Scene { scene, hero } => {
                let assets = sigil_viewer::load_scene(scene)?;
                let from_scene = assets.desc.hero.as_ref().map(|h| (h.glb.clone(), h.sidecar.clone()));
                viewer.set_scene(assets);
                if let Some((glb, sidecar)) = hero.clone().or(from_scene) {
                    let side = sidecar.as_deref().map(Sidecar::read).unwrap_or_default();
                    viewer.set_hero(load_model(&glb)?, &side);
                }
            }
        }
        Ok(())
    };
    load_hero(&mut viewer, source).map_err(ViewerError::Load)?;
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

pub fn run(source: Source, before: Option<Source>, rx: Receiver<Cmd>, out: Outputs) {
    let mut viewer = match prepare(&source, before.as_ref()) {
        Ok(v) => v,
        Err(ViewerError::NoGpu(e)) => {
            report(&out, |s| s.status = Status::NoGpu(e));
            return;
        }
        Err(e) => {
            report(&out, |s| s.status = Status::Failed(e.to_string()));
            return;
        }
    };
    ready(&out, &viewer);
    let frame = Duration::from_micros(16_667);
    let (mut w, mut h) = (0u32, 0u32);
    let mut visible = true;
    let mut dirty = true;
    let mut last = Instant::now();
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
            let rgba = viewer.render(w, h, dt).to_vec();
            let t = viewer.timings();
            out.pixels.set(w, h, rgba);
            publish_live(&out, &viewer);
            if let Ok(mut s) = out.shared.lock() {
                s.frame_ms = started.elapsed().as_secs_f32() * 1000.0;
                s.readback_ms = t.gpu_wait_ms + t.unpad_ms;
            }
            dirty = false;
        }
    }
}
