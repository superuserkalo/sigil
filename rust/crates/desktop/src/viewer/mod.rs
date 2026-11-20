//! The 3D viewer's viewport: a hero (or the Hideout) as the game draws it with the player's
//! mods, animated, orbitable, with a before and after switch against the unmodded hero.
//!
//! Each viewport owns a render thread (`worker`) with its own wgpu device (`sigil-viewer`); the
//! UI thread never renders. The thread writes frames into a `rune::Pixels`, which redraws
//! without the view running again, and only while the viewport is on screen and something
//! moves. Status changes (loaded, failed, a picture saved) wake the app's poll.
//!
//! The control strip unites both reference apps: Grimoire's playback strip (pause, seek,
//! speed, camera reset, full screen, capture) and Deadlock Mod Manager's comparison (the stock
//! hero in step with the modded clip, swapped or split with a draggable line). Input: drag
//! orbits, the wheel zooms, Shift-drag pans, double-click resets, and with the view focused the
//! arrows orbit, + and - zoom, Space plays and pauses, Home resets, Escape leaves full screen.
//!
//! The API the screens call:
//!
//! ```ignore
//! viewer::open_hero(s, "locker", "Infernus", Source::Hero { glb, sidecar }, Some(Source::Hero { .. }));
//! viewer::open_scene(s, "hideout", "Hideout", scene_json, None);
//! viewer::view(s, &u, "locker")      // the viewport El, any size
//! viewer::close(s, "locker");
//! ```
//!
//! `app::view` places `viewer::overlay` for full screen; `main::poll` calls `viewer::poll`.

// Locker, Installed and a mod's details place the viewport next (their own round); until
// then parts of the API have no caller in the app.
#![allow(dead_code)]

pub mod icons;
pub mod worker;

#[path = "../screens/settings/locales_viewer.rs"]
pub(crate) mod locales;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use rune::{Color, Drag, El, Key, KeyEvent, Live, LiveText, Phase, Pixels, Shape, SurfaceFit, Tx, WheelEvent, col, el, icon, row, text};

use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{BODY_MED, MICRO, SMALL, Ui};
pub use sigil_viewer::fixture::Variant;
pub use worker::{Source, Status};
use worker::{Cmd, Outputs, Shared};

pub const KNOB: f32 = 12.0;
const SEEK_W: f32 = 180.0;
/// The most device pixels a viewport renders (see `viewport`'s layout handler).
const MAX_PIXELS: f32 = 3_200_000.0;
const SPEEDS: [f32; 5] = [0.25, 0.5, 1.0, 1.5, 2.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompareMode {
    /// With the player's mods.
    #[default]
    After,
    /// The unmodded hero.
    Before,
    /// Unmodded left of the line, modded right of it.
    Split,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gesture {
    #[default]
    None,
    Orbit,
    Pan,
    Split,
}

pub struct View {
    title: String,
    tx: Option<Sender<Cmd>>,
    pixels: Pixels,
    shared: Arc<Mutex<Shared>>,
    progress: Live<[f32; 2]>,
    knob: Live<[f32; 2]>,
    time: LiveText,
    pub status: Status,
    pub clips: Vec<(String, f32)>,
    pub clip: Option<String>,
    pub has_before: bool,
    pub playing: bool,
    pub speed: f32,
    pub compare: CompareMode,
    pub split: f32,
    pub adapter: String,
    /// Shown instead of the 3D view when the machine has no GPU (the screens set it).
    pub still: Option<Pixels>,
    gesture: Gesture,
    last: [f32; 2],
    points: [f32; 2],
}

impl Drop for View {
    fn drop(&mut self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::Stop);
        }
    }
}

#[derive(Default)]
pub struct Viewers {
    map: HashMap<String, View>,
    fullscreen: Option<String>,
}

impl Viewers {
    pub fn get(&self, key: &str) -> Option<&View> {
        self.map.get(key)
    }
}

fn send(s: &mut State, key: &str, cmd: Cmd) {
    if let Some(tx) = s.viewers.map.get(key).and_then(|v| v.tx.as_ref()) {
        let _ = tx.send(cmd);
    }
}

fn new_view(title: &str) -> View {
    View {
        title: title.to_string(),
        tx: None,
        pixels: Pixels::new(),
        shared: Arc::new(Mutex::new(Shared::default())),
        progress: Live::new([0.0, 0.0]),
        knob: Live::new([0.0, 0.0]),
        time: LiveText::new("0:00.0 / 0:00.0"),
        status: Status::Loading,
        clips: Vec::new(),
        clip: None,
        has_before: false,
        playing: true,
        speed: 1.0,
        compare: CompareMode::After,
        split: 0.5,
        adapter: String::new(),
        still: None,
        gesture: Gesture::None,
        last: [0.0; 2],
        points: [0.0; 2],
    }
}

/// Opens (or replaces) the viewport `key` on `hero`, with `compare_with` as the unmodded side.
pub fn open_hero(s: &mut State, key: &str, title: &str, hero: Source, compare_with: Option<Source>) {
    open(s, key, title, hero, compare_with, CompareMode::After);
}

/// Opens the viewport `key` on a Hideout `scene.json`, with `hero` (glb, sidecar) at the spawn
/// or the scene's own `hero` entry.
pub fn open_scene(s: &mut State, key: &str, title: &str, scene: PathBuf, hero: Option<(PathBuf, Option<PathBuf>)>) {
    open(s, key, title, Source::Scene { scene, hero }, None, CompareMode::After);
}

fn open(s: &mut State, key: &str, title: &str, source: Source, before: Option<Source>, compare: CompareMode) {
    let mut view = new_view(title);
    view.compare = compare;
    let outputs = Outputs {
        pixels: view.pixels.clone(),
        shared: view.shared.clone(),
        progress: view.progress.clone(),
        knob: view.knob.clone(),
        time: view.time.clone(),
        seek_width: SEEK_W,
        wake: s.wake.clone(),
    };
    if rune::is_headless() {
        // Snapshots render one fixed frame on this thread: no clock, no GPU timing.
        render_still(&mut view, &outputs, &source, before.as_ref(), HEADLESS_POINTS);
    } else {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name(format!("sigil-viewer-{key}")).spawn(move || worker::run(source, before, rx, outputs));
        match spawned {
            Ok(_) => {
                if compare != CompareMode::After {
                    let c = if compare == CompareMode::Split { sigil_viewer::Compare::Split(view.split) } else { sigil_viewer::Compare::Before };
                    let _ = tx.send(Cmd::Compare(c));
                }
                view.tx = Some(tx);
            }
            Err(e) => view.status = Status::Failed(e.to_string()),
        }
    }
    s.viewers.map.insert(key.to_string(), view);
}

/// The viewport size a headless snapshot renders at: the full-screen overlay of the default
/// 1360 by 860 window, less the control strip.
pub const HEADLESS_POINTS: [f32; 2] = [1360.0, 860.0];
const HEADLESS_TIME: f32 = 1.0;

fn render_still(view: &mut View, out: &Outputs, source: &Source, before: Option<&Source>, points: [f32; 2]) {
    match worker::prepare(source, before) {
        Ok(mut viewer) => {
            viewer.playback.playing = false;
            viewer.seek(HEADLESS_TIME);
            let scale = 2.0;
            let (w, h) = ((points[0] * scale) as u32, (points[1] * scale) as u32);
            viewer.compare = match view.compare {
                CompareMode::After => sigil_viewer::Compare::After,
                CompareMode::Before => sigil_viewer::Compare::Before,
                CompareMode::Split => sigil_viewer::Compare::Split(view.split),
            };
            let rgba = viewer.render(w, h, 0.0).to_vec();
            out.pixels.set(w, h, rgba);
            worker::ready(out, &viewer);
            worker::publish_live(out, &viewer);
            view.playing = false;
        }
        Err(sigil_viewer::ViewerError::NoGpu(e)) => view.status = Status::NoGpu(e),
        Err(e) => view.status = Status::Failed(e.to_string()),
    }
    absorb(view);
}

/// Stops the viewport's thread and forgets it.
pub fn close(s: &mut State, key: &str) {
    s.viewers.map.remove(key);
    if s.viewers.fullscreen.as_deref() == Some(key) {
        s.viewers.fullscreen = None;
    }
}

pub fn set_still(s: &mut State, key: &str, still: Pixels) {
    if let Some(v) = s.viewers.map.get_mut(key) {
        v.still = Some(still);
    }
}

pub fn is_open(s: &State, key: &str) -> bool {
    s.viewers.map.contains_key(key)
}

/// Takes what the render threads reported. True when a view must run again.
pub fn poll(s: &mut State) -> bool {
    let mut changed = false;
    let mut notes = Vec::new();
    for view in s.viewers.map.values_mut() {
        if let Some(note) = absorb(view) {
            notes.push(note);
        }
        changed |= view.shared.lock().map(|mut sh| std::mem::take(&mut sh.changed)).unwrap_or(false);
    }
    for note in notes {
        s.status = note;
        changed = true;
    }
    changed
}

/// Copies the thread's report into the view; a saved picture becomes a status line.
fn absorb(view: &mut View) -> Option<String> {
    let Ok(mut sh) = view.shared.lock() else { return None };
    if !sh.changed && !matches!(sh.status, Status::Ready) && view.status == sh.status {
        return None;
    }
    view.status = sh.status.clone();
    view.clips = sh.clips.clone();
    view.clip = sh.clip.clone();
    view.has_before = sh.has_before;
    view.adapter = sh.adapter.clone();
    sh.captured.take().map(|r| match r {
        Ok(path) => fill(tr("Saved a picture to {}"), &[&path.display().to_string()]),
        Err(e) => fill(tr("Could not save the picture: {}"), &[&e]),
    })
}

/// Where captures go: the user's Pictures folder, in `Sigil`.
pub fn pictures_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    let pictures = std::env::var_os("XDG_PICTURES_DIR").filter(|_| cfg!(all(unix, not(target_os = "macos")))).map(PathBuf::from).unwrap_or_else(|| home.join("Pictures"));
    pictures.join("Sigil")
}

fn capture_name(title: &str) -> String {
    let stem: String = title.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}-{secs}.png", if stem.is_empty() { "sigil" } else { &stem })
}

fn set_compare(s: &mut State, key: &str, mode: CompareMode) {
    let Some(v) = s.viewers.map.get_mut(key) else { return };
    v.compare = mode;
    let c = match mode {
        CompareMode::After => sigil_viewer::Compare::After,
        CompareMode::Before => sigil_viewer::Compare::Before,
        CompareMode::Split => sigil_viewer::Compare::Split(v.split),
    };
    send(s, key, Cmd::Compare(c));
}

fn toggle_play(s: &mut State, key: &str) {
    let Some(v) = s.viewers.map.get_mut(key) else { return };
    v.playing = !v.playing;
    let on = v.playing;
    send(s, key, Cmd::Play(on));
}

fn reset(s: &mut State, key: &str) {
    send(s, key, Cmd::ResetCamera);
}

fn toggle_fullscreen(s: &mut State, key: &str) {
    s.viewers.fullscreen = if s.viewers.fullscreen.as_deref() == Some(key) { None } else { Some(key.to_string()) };
}

fn capture(s: &mut State, key: &str) {
    let Some(v) = s.viewers.map.get(key) else { return };
    let path = pictures_dir().join(capture_name(&v.title));
    send(s, key, Cmd::Capture(path));
}

fn on_drag(s: &mut State, key: &str, d: &mut Drag, tx: &mut Tx) {
    let Some(v) = s.viewers.map.get_mut(key) else { return };
    match d.phase {
        Phase::Down => {
            v.last = [d.x, d.y];
            let near_line = v.compare == CompareMode::Split && v.has_before && (d.x - v.split * d.w).abs() < 14.0;
            v.gesture = if near_line {
                Gesture::Split
            } else if tx.mods().shift {
                Gesture::Pan
            } else {
                Gesture::Orbit
            };
            if d.clicks >= 2 {
                v.gesture = Gesture::None;
                reset(s, key);
            }
        }
        Phase::Move => {
            let (dx, dy) = (d.x - v.last[0], d.y - v.last[1]);
            v.last = [d.x, d.y];
            match v.gesture {
                Gesture::Orbit => send(s, key, Cmd::Orbit(dx, dy)),
                Gesture::Pan => send(s, key, Cmd::Pan(dx, dy, d.h)),
                Gesture::Split => {
                    v.split = (d.x / d.w.max(1.0)).clamp(0.02, 0.98);
                    let f = v.split;
                    send(s, key, Cmd::Compare(sigil_viewer::Compare::Split(f)));
                }
                Gesture::None => {}
            }
            tx.live_only();
        }
        Phase::Up => v.gesture = Gesture::None,
    }
}

fn on_wheel(s: &mut State, key: &str, w: WheelEvent) -> bool {
    let step = if w.precise { w.dy * 0.004 } else { w.dy.signum() * 0.12 };
    send(s, key, Cmd::Zoom((-step).exp()));
    true
}

fn on_key(s: &mut State, key: &str, ev: &KeyEvent) -> bool {
    match &ev.key {
        Key::Left => send(s, key, Cmd::Orbit(-24.0, 0.0)),
        Key::Right => send(s, key, Cmd::Orbit(24.0, 0.0)),
        Key::Up => send(s, key, Cmd::Orbit(0.0, -18.0)),
        Key::Down => send(s, key, Cmd::Orbit(0.0, 18.0)),
        Key::Home => reset(s, key),
        Key::Escape if s.viewers.fullscreen.as_deref() == Some(key) => s.viewers.fullscreen = None,
        Key::Text(t) if t == " " => toggle_play(s, key),
        Key::Text(t) if t == "+" || t == "=" => send(s, key, Cmd::Zoom(0.85)),
        Key::Text(t) if t == "-" => send(s, key, Cmd::Zoom(1.0 / 0.85)),
        Key::Text(t) if t == "0" => reset(s, key),
        Key::Text(t) if t == "f" => toggle_fullscreen(s, key),
        _ => return false,
    }
    true
}

/// The viewport for `key`: the 3D view filling its parent, its states and the control strip.
/// Nothing (an empty box) when no viewport is open under that key.
pub fn view(s: &State, u: &Ui, key: &str) -> El {
    if s.viewers.fullscreen.as_deref() == Some(key) {
        return el().key(format!("viewer-{key}-away")).w_full().h_full().radius(crate::ui::r(10.0)).fill(shade("#080809")).center().child(text(tr("Shown full screen")).font(SMALL).color(u.muted));
    }
    viewport(s, u, key, false)
}

/// The full-screen viewer over the whole window, when one is.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    let key = s.viewers.fullscreen.as_deref()?;
    s.viewers.map.get(key)?;
    Some(el().key("viewer-fullscreen").absolute(0.0, 0.0).w_full().h_full().fill(shade("#000000")).child(viewport(s, u, key, true)))
}

fn viewport(s: &State, u: &Ui, key: &str, fullscreen: bool) -> El {
    let Some(v) = s.viewers.map.get(key) else { return el().w_full().h_full() };
    let k = key.to_string();
    let radius = if fullscreen { 0.0 } else { crate::ui::r(10.0) };
    let label = fill(tr("3D view of {}"), &[&v.title]);
    let mut stage = el()
        .key(format!("viewer-{key}{}", if fullscreen { "-full" } else { "" }))
        .w_full()
        .h_full()
        .radius(radius)
        .clip_rounded(radius)
        .fill(shade("#050506"))
        .focusable(true)
        .role("img")
        .label(label)
        .cursor(rune::Cursor::Grab);
    match (&v.status, &v.still) {
        (Status::NoGpu(_), Some(still)) => stage = stage.pixels_fit(still, SurfaceFit::Fill),
        _ => stage = stage.pixels_fit(&v.pixels, SurfaceFit::Fill),
    }
    let (k1, k2, k3, k4, k5) = (k.clone(), k.clone(), k.clone(), k.clone(), k.clone());
    stage = stage
        .on_layout(move |st: &mut State, rect: [f32; 4]| {
            let scale = crate::data::pixel_scale().max(1.0);
            if let Some(v) = st.viewers.map.get_mut(&k1) {
                v.points = [rect[2], rect[3]];
            }
            // Device pixels, capped near 2240 by 1400: every frame crosses to the UI as RGBA, so a 4K
            // full screen renders softer instead of copying 60 MB a frame.
            let pixels = rect[2] * rect[3] * scale * scale;
            let scale = if pixels > MAX_PIXELS { scale * (MAX_PIXELS / pixels).sqrt() } else { scale };
            let (w, h) = ((rect[2] * scale).round().max(1.0) as u32, (rect[3] * scale).round().max(1.0) as u32);
            send(st, &k1, Cmd::Size(w, h));
        })
        .on_visible(move |st: &mut State, on: bool, _tx: &mut Tx| send(st, &k2, Cmd::Visible(on)))
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| on_drag(st, &k3, d, tx))
        .on_wheel(move |st: &mut State, w: WheelEvent, _tx: &mut Tx| on_wheel(st, &k4, w))
        .on_key(move |st: &mut State, ev: &KeyEvent, _tx: &mut Tx| on_key(st, &k5, ev));
    match &v.status {
        Status::Loading => stage = stage.child(notice(u, tr("Loading the 3D view"), None)),
        Status::NoGpu(_) if v.still.is_none() => stage = stage.child(notice(u, tr("The 3D view needs a working graphics driver."), Some(tr("This machine has no GPU adapter Sigil can use.").to_string()))),
        Status::NoGpu(_) => {}
        Status::Failed(e) => stage = stage.child(notice(u, tr("The 3D view could not open."), Some(e.clone()))),
        Status::Ready => stage = stage.child(strip(v, u, key, fullscreen)),
    }
    stage
}

/// A fixed dark from hex (the viewer's stage is always dark, whatever the theme).
fn shade(hex: &str) -> Color {
    rune::color::hex(hex).unwrap_or(Color::BLACK)
}

fn notice(u: &Ui, title: &str, detail: Option<String>) -> El {
    let mut c = col().gap(6.0).max_w(420.0).padding(16.0).radius(crate::ui::r(10.0)).fill(shade("#000000").alpha(0.55)).child(text(title.to_string()).font(BODY_MED).color(u.ink));
    if let Some(d) = detail {
        c = c.child(text(d).font(SMALL).color(u.muted));
    }
    el().absolute(0.0, 0.0).w_full().h_full().center().child(c)
}

fn strip_button(u: &Ui, glyph: &'static rune::Icon, label: &str, on: bool) -> El {
    el().size(30.0, 30.0)
        .center()
        .radius(crate::ui::r(7.0))
        .fill(if on { u.accent.alpha(0.22) } else { Color::TRANSPARENT })
        .hover(Color::WHITE.alpha(0.10))
        .press(Color::WHITE.alpha(0.16))
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .child(icon(glyph, 16.0).color(if on { u.accent } else { Color::WHITE.alpha(0.86) }).a11y_hidden())
}

fn strip_text(u: &Ui, label: String, a11y: &str, on: bool) -> El {
    row()
        .h(30.0)
        .px(9.0)
        .center()
        .radius(crate::ui::r(7.0))
        .fill(if on { u.accent.alpha(0.22) } else { Color::TRANSPARENT })
        .hover(Color::WHITE.alpha(0.10))
        .press(Color::WHITE.alpha(0.16))
        .focusable(true)
        .role("button")
        .label(a11y.to_string())
        .child(text(label).font(SMALL).color(if on { u.accent } else { Color::WHITE.alpha(0.86) }).a11y_hidden())
}

fn seek_bar(v: &View, key: &str) -> El {
    let k = key.to_string();
    let k2 = key.to_string();
    let track = Shape::new().line([KNOB / 2.0, 12.0], [SEEK_W - KNOB / 2.0, 12.0], 3.0, rune::LineCap::Round, Color::WHITE.alpha(0.18));
    let filled = Shape::new().line([KNOB / 2.0, 12.0], [SEEK_W - KNOB / 2.0, 12.0], 3.0, rune::LineCap::Round, Color::WHITE.alpha(0.9));
    el().key(format!("viewer-{key}-seek"))
        .w(SEEK_W)
        .h(24.0)
        .focusable(true)
        .role("slider")
        .label(tr("Seek").to_string())
        .cursor(rune::Cursor::Pointer)
        .child(el().absolute(0.0, 0.0).w(SEEK_W).h(24.0).shape(track).a11y_hidden())
        .child(el().absolute(0.0, 0.0).w(SEEK_W).h(24.0).shape(filled).live_trim(&v.progress).a11y_hidden())
        .child(el().absolute(0.0, 6.0).size(KNOB, KNOB).radius(KNOB / 2.0).fill(Color::WHITE).shadow(1.0, 3.0, Color::BLACK.alpha(0.4)).live_offset(&v.knob).a11y_hidden())
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| {
            if d.phase == Phase::Up && d.cancelled() {
                return;
            }
            let f = ((d.x - KNOB / 2.0) / (SEEK_W - KNOB)).clamp(0.0, 1.0);
            if let Some(v) = st.viewers.map.get_mut(&k) {
                v.playing = false;
            }
            send(st, &k, Cmd::Seek(f));
            if d.phase == Phase::Move {
                tx.live_only();
            }
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
            let Some(v) = st.viewers.map.get(&k2) else { return false };
            let now = v.progress.get()[1];
            let step = match ev.key {
                Key::Left | Key::Down => -0.05,
                Key::Right | Key::Up => 0.05,
                Key::Home => -1.0,
                Key::End => 1.0,
                _ => return false,
            };
            if let Some(v) = st.viewers.map.get_mut(&k2) {
                v.playing = false;
            }
            send(st, &k2, Cmd::Seek((now + step).clamp(0.0, 1.0)));
            true
        })
}

fn strip(v: &View, u: &Ui, key: &str, fullscreen: bool) -> El {
    let k = |_: ()| key.to_string();
    let (kp, kr, kc, kf, ks, kclip) = (k(()), k(()), k(()), k(()), k(()), k(()));
    let animated = v.clips.iter().any(|(_, d)| *d > 0.0);
    let mut bar = row().gap(2.0).px(6.0).py(5.0).center().radius(crate::ui::r(10.0)).fill(shade("#0d0d0f").alpha(0.78)).border(1.0, Color::WHITE.alpha(0.08));
    if animated {
        let play = if v.playing { strip_button(u, &icons::PAUSE, tr("Pause"), false) } else { strip_button(u, &crate::icons::PLAY, tr("Play"), false) };
        bar = bar
            .child(play.on_press(move |st: &mut State| toggle_play(st, &kp)))
            .child(el().px(6.0).child(seek_bar(v, key)))
            .child(text(String::new()).live_text(&v.time).font(MICRO).color(Color::WHITE.alpha(0.7)).w(92.0))
            .child(strip_text(u, format!("{}x", trim_speed(v.speed)), tr("Playback speed"), false).on_press(move |st: &mut State| {
                let Some(v) = st.viewers.map.get_mut(&ks) else { return };
                let i = SPEEDS.iter().position(|x| (*x - v.speed).abs() < 1e-3).unwrap_or(2);
                v.speed = SPEEDS[(i + 1) % SPEEDS.len()];
                let speed = v.speed;
                send(st, &ks, Cmd::Speed(speed));
            }));
        if v.clips.len() > 1 {
            let name = v.clip.clone().unwrap_or_else(|| tr("No animation").to_string());
            bar = bar.child(strip_text(u, name, tr("Animation"), false).on_press(move |st: &mut State| {
                let Some(v) = st.viewers.map.get_mut(&kclip) else { return };
                let i = v.clip.as_ref().and_then(|c| v.clips.iter().position(|(n, _)| n == c)).map_or(0, |i| (i + 1) % v.clips.len());
                let name = v.clips[i].0.clone();
                v.clip = Some(name.clone());
                v.playing = true;
                send(st, &kclip, Cmd::Clip(name));
                send(st, &kclip, Cmd::Play(true));
            }));
        }
    }
    if v.has_before {
        bar = bar.child(el().w(1.0).h(18.0).mx(4.0).fill(Color::WHITE.alpha(0.12)).a11y_hidden());
        for (mode, label) in [(CompareMode::After, tr("With mods")), (CompareMode::Before, tr("Without mods")), (CompareMode::Split, tr("Split"))] {
            let kk = key.to_string();
            bar = bar.child(strip_text(u, label.to_string(), label, v.compare == mode).on_press(move |st: &mut State| set_compare(st, &kk, mode)));
        }
    }
    bar = bar
        .child(el().w(1.0).h(18.0).mx(4.0).fill(Color::WHITE.alpha(0.12)).a11y_hidden())
        .child(strip_button(u, &crate::icons::ROTATE_CCW, tr("Reset camera"), false).on_press(move |st: &mut State| reset(st, &kr)))
        .child(strip_button(u, &icons::CAMERA, tr("Save a picture"), false).on_press(move |st: &mut State| capture(st, &kc)))
        .child(
            if fullscreen { strip_button(u, &icons::MINIMIZE, tr("Exit full screen"), false) } else { strip_button(u, &icons::MAXIMIZE, tr("Full screen"), false) }
                .on_press(move |st: &mut State| toggle_fullscreen(st, &kf)),
        );
    let mut layer = col().absolute(0.0, 0.0).w_full().h_full().padding(12.0).gap(8.0);
    layer = layer.child(row().w_full().child(
        col().gap(2.0).child(text(v.title.clone()).font(BODY_MED).color(Color::WHITE.alpha(0.92))).child(text(tr("Drag to turn, scroll to zoom, Shift-drag to move, double-click to reset.").to_string()).font(MICRO).color(Color::WHITE.alpha(0.5))),
    ));
    layer.child(el().grow(1.0)).child(row().w_full().center().child(bar))
}

fn trim_speed(k: f32) -> String {
    let s = format!("{k:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Demo: the stand-in hero full screen, modded, with the stock one to compare against.
pub fn scene_viewer(s: &mut State) {
    open_hero(s, "demo", "Stand-in hero", Source::StandIn(Variant::Modded), Some(Source::StandIn(Variant::Stock)));
    s.viewers.fullscreen = Some("demo".into());
}

/// Demo: the same, split down the middle (stock left, modded right).
pub fn scene_viewer_split(s: &mut State) {
    open(s, "demo", "Stand-in hero", Source::StandIn(Variant::Modded), Some(Source::StandIn(Variant::Stock)), CompareMode::Split);
    s.viewers.fullscreen = Some("demo".into());
}

/// Demo: the stand-in Hideout (a room, lights, fog, sky) with the hero at the spawn. Its
/// files are written once under the data folder's `viewer-demo`.
pub fn scene_viewer_hideout(s: &mut State) {
    let dir = PathBuf::from(&s.paths.data).join("viewer-demo").join("hideout");
    match sigil_viewer::fixture::write_hideout(&dir, Variant::Modded) {
        Ok(scene) => open_scene(s, "demo", "Hideout", scene, None),
        Err(e) => s.status = e.to_string(),
    }
    s.viewers.fullscreen = Some("demo".into());
}

/// A headless scene: its name and how it sets the state up.
type Scene = (&'static str, fn(&mut State));

pub const SCENES: [Scene; 3] = [("viewer", scene_viewer), ("viewer-split", scene_viewer_split), ("viewer-hideout", scene_viewer_hideout)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_land_in_pictures_sigil_with_a_safe_name() {
        assert!(pictures_dir().ends_with("Sigil"));
        let name = capture_name("Infernus: Ember / v2");
        assert!(name.starts_with("Infernus__Ember___v2-") && name.ends_with(".png"), "{name}");
    }

    /// The render thread loads, renders at the size it is told, renders again for a camera
    /// move, and stops. Skips on a machine without a GPU adapter.
    #[test]
    fn the_render_thread_draws_into_the_pixels() {
        let view = new_view("test");
        let outputs = Outputs { pixels: view.pixels.clone(), shared: view.shared.clone(), progress: view.progress.clone(), knob: view.knob.clone(), time: view.time.clone(), seek_width: SEEK_W, wake: None };
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || worker::run(Source::StandIn(Variant::Stock), Some(Source::StandIn(Variant::Modded)), rx, outputs));
        tx.send(Cmd::Size(96, 64)).unwrap();
        tx.send(Cmd::Play(false)).unwrap();
        let waited = std::time::Instant::now();
        while view.pixels.size() != (96, 64) && waited.elapsed().as_secs() < 30 {
            if matches!(view.shared.lock().unwrap().status, Status::NoGpu(_)) {
                eprintln!("skipped: no GPU adapter");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(view.pixels.size(), (96, 64));
        {
            let shared = view.shared.lock().unwrap();
            assert_eq!(shared.status, Status::Ready);
            assert!(shared.has_before);
            assert_eq!(shared.clip.as_deref(), Some("hero_shop"));
        }
        tx.send(Cmd::Size(48, 48)).unwrap();
        let waited = std::time::Instant::now();
        while view.pixels.size() != (48, 48) && waited.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(view.pixels.size(), (48, 48));
        tx.send(Cmd::Stop).unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn speed_labels_drop_trailing_zeros() {
        assert_eq!(trim_speed(1.0), "1");
        assert_eq!(trim_speed(0.25), "0.25");
        assert_eq!(trim_speed(1.5), "1.5");
    }

    #[test]
    fn the_clock_reads_minutes_and_tenths() {
        assert_eq!(worker::clock(3.2), "0:03.2");
        assert_eq!(worker::clock(61.05), "1:01.0");
    }
}
