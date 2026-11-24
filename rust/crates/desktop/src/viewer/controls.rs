//! The viewer's Controls panel and recordings, the union of both reference apps:
//!
//! - Grimoire's Controls panel (HeroViewerToolbar.tsx): the grouped animation picker with
//!   friendly names, the lighting scene (Studio, Midtown dusk), the background (Grimoire's
//!   None, Transparent and a picture of the player's, Deadlock Mod Manager's studio gradient,
//!   Grimoire's ten soul gradients), and the Auto-rotate, Cloth, Particles and Bloom switches,
//!   plus Sigil's Shadows. The choices are kept across sessions in `<data>/viewer.json`.
//! - Grimoire's GIF capture (viewer-gif-capture.md): record up to four seconds, then review it
//!   looping, trim its start and end on a strip of thumbnails, save, retake or cancel.
//! - Deadlock Mod Manager's video recording: up to ten seconds, a REC pill with the clock,
//!   saved as WebM (VP8) when it stops, see `sigil_viewer::capture`.
//!
//! - Deadlock Mod Manager's exposure slider, and its camera angle kept per hero
//!   (`<data>/viewer-camera.json`, studio views only).
//!
//! Recordings and pictures go to `Pictures/Sigil`, or where the player says when "Ask where to
//! save" is on (Deadlock Mod Manager's save dialog).

use std::path::PathBuf;

use rune::{Color, Drag, El, KeyEvent, Key, Phase, Side, SurfaceFit, Tx, col, el, icon, row, text};

use super::worker::{Cmd, GifCmd, GifState, LookOptions, VideoCmd};
use super::{View, send, shade, strip_button};
use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{MICRO, SMALL, Ui};
use sigil_viewer::{Backdrop, GRADIENTS, Lighting};

/// The per-view state of the panel and the recordings.
#[derive(Clone, Debug, Default)]
pub struct ViewUi {
    pub auto_rotate: bool,
    pub look: LookOptions,
    /// The Controls panel is open.
    pub panel: bool,
    pub gif: GifState,
    /// Seconds recorded while a video records.
    pub video: Option<f32>,
    /// Name, label and group of every clip.
    pub labels: Vec<(String, String, String)>,
    /// The picture behind the hero, when one was chosen.
    pub image: Option<PathBuf>,
    /// Which GIF trim handle a drag moves: 0 start, 1 end, 2 the playhead.
    pub gif_drag: u8,
    /// Pictures and GIFs open a save dialog instead of landing in `Pictures/Sigil`.
    pub ask_where: bool,
    /// A picture is being taken: the stage flashes until it is saved.
    pub flash: bool,
    /// The device can draw the wireframe view.
    pub can_wireframe: bool,
}

/// The choices kept across sessions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    pub auto_rotate: bool,
    pub look: LookOptions,
    pub image: Option<PathBuf>,
    pub ask_where: bool,
}

fn prefs_path() -> PathBuf {
    crate::data::data_dir().join("viewer.json")
}

fn poses_path() -> PathBuf {
    crate::data::data_dir().join("viewer-camera.json")
}

/// The most heroes whose camera angle is kept.
const MAX_POSES: usize = 256;

/// Reads `{ hero: [yaw, pitch, zoom] }`, dropping anything malformed.
fn parse_poses(text: &str) -> serde_json::Map<String, serde_json::Value> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text) else { return serde_json::Map::new() };
    map.into_iter().filter(|(_, v)| pose_from(v).is_some()).take(MAX_POSES).collect()
}

fn pose_from(v: &serde_json::Value) -> Option<[f32; 3]> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    let n = |i: usize| a[i].as_f64().map(|x| x as f32).filter(|x| x.is_finite());
    let pose = [n(0)?, n(1)?, n(2)?];
    (pose[2] > 0.0).then_some(pose)
}

/// The camera angle last left on `hero`'s studio view.
pub fn load_pose(hero: &str) -> Option<[f32; 3]> {
    let map = parse_poses(&std::fs::read_to_string(poses_path()).ok()?);
    map.get(&hero.to_ascii_lowercase()).and_then(pose_from)
}

/// Keeps `pose` as `hero`'s camera angle.
pub fn save_pose(hero: &str, pose: [f32; 3]) {
    if !pose.iter().all(|x| x.is_finite()) {
        return;
    }
    let mut map = std::fs::read_to_string(poses_path()).map(|t| parse_poses(&t)).unwrap_or_default();
    map.insert(hero.to_ascii_lowercase(), serde_json::json!(pose));
    while map.len() > MAX_POSES {
        let Some(first) = map.keys().next().cloned() else { break };
        map.remove(&first);
    }
    let _ = std::fs::write(poses_path(), serde_json::Value::Object(map).to_string());
}

fn backdrop_id(b: Backdrop) -> String {
    match b {
        Backdrop::Studio => "studio".into(),
        Backdrop::Plain => "none".into(),
        Backdrop::Image => "image".into(),
        Backdrop::Gradient(i) => format!("gradient:{i}"),
    }
}

fn backdrop_from(id: &str) -> Backdrop {
    match id {
        "none" => Backdrop::Plain,
        "image" => Backdrop::Image,
        _ => id.strip_prefix("gradient:").and_then(|i| i.parse().ok()).filter(|i| *i < GRADIENTS.len()).map_or(Backdrop::Studio, Backdrop::Gradient),
    }
}

impl Prefs {
    pub fn parse(text: &str) -> Prefs {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Prefs::default() };
        let d = LookOptions::default();
        let flag = |k: &str, def: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(def);
        Prefs {
            auto_rotate: flag("autoRotate", false),
            look: LookOptions {
                backdrop: backdrop_from(v.get("background").and_then(|x| x.as_str()).unwrap_or("studio")),
                lighting: if v.get("lighting").and_then(|x| x.as_str()) == Some("midtown") { Lighting::Midtown } else { Lighting::Studio },
                bloom: flag("bloom", d.bloom),
                shadows: flag("shadows", d.shadows),
                cloth: flag("cloth", d.cloth),
                particles: flag("particles", d.particles),
                transparent: flag("transparent", false),
                wireframe: flag("wireframe", false),
                gizmo: flag("axisGizmo", false),
                exposure: v.get("exposure").and_then(|x| x.as_f64()).map(|x| x as f32).filter(|x| x.is_finite()).map_or(d.exposure, |x| x.clamp(-EXPOSURE_STOPS, EXPOSURE_STOPS)),
            },
            image: v.get("image").and_then(|x| x.as_str()).map(PathBuf::from),
            ask_where: flag("askWhere", false),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::json!({
            "autoRotate": self.auto_rotate,
            "background": backdrop_id(self.look.backdrop),
            "lighting": if self.look.lighting == Lighting::Midtown { "midtown" } else { "studio" },
            "bloom": self.look.bloom,
            "shadows": self.look.shadows,
            "cloth": self.look.cloth,
            "particles": self.look.particles,
            "transparent": self.look.transparent,
            "exposure": self.look.exposure,
            "wireframe": self.look.wireframe,
            "axisGizmo": self.look.gizmo,
            "image": self.image.as_ref().map(|p| p.display().to_string()),
            "askWhere": self.ask_where,
        })
        .to_string()
    }

    pub fn load() -> Prefs {
        std::fs::read_to_string(prefs_path()).map(|t| Prefs::parse(&t)).unwrap_or_default()
    }

    fn save(&self) {
        let _ = std::fs::write(prefs_path(), self.to_json());
    }
}

/// The commands that put a new view's renderer in the kept look.
pub fn initial(prefs: &Prefs) -> Vec<Cmd> {
    let mut cmds = vec![Cmd::Look(prefs.look), Cmd::AutoRotate(prefs.auto_rotate)];
    if prefs.look.backdrop == Backdrop::Image && prefs.image.is_some() {
        cmds.push(Cmd::BackdropImage(prefs.image.clone()));
    }
    cmds
}

fn remember(v: &View) {
    Prefs { auto_rotate: v.ui.auto_rotate, look: v.ui.look, image: v.ui.image.clone(), ask_where: v.ui.ask_where }.save();
}

/// The exposure slider's range, in stops either way.
pub const EXPOSURE_STOPS: f32 = 2.0;

fn set_exposure(s: &mut State, key: &str, stops: f32) {
    // Quarter stops, so the slider lands back on 0.
    let stops = ((stops * 4.0).round() / 4.0).clamp(-EXPOSURE_STOPS, EXPOSURE_STOPS);
    set_look(s, key, |l| l.exposure = stops);
}

fn toggle_ask_where(s: &mut State, key: &str) {
    let Some(v) = s.viewers.map.get_mut(key) else { return };
    v.ui.ask_where = !v.ui.ask_where;
    remember(v);
}

/// Asks where `name` goes (a save dialog in Pictures/Sigil's place), then runs `then` with it.
pub fn ask_save_path(tx: &mut Tx, name: String, ext: &'static str, then: impl FnOnce(&mut State, PathBuf) + 'static) {
    let options = rune::FileDialogOptions { title: tr("Save as").to_string(), file_name: Some(name), ..Default::default() }.filter(ext.to_ascii_uppercase(), &[ext]);
    tx.save_file(options, move |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => then(st, path),
        Ok(None) => {}
        Err(e) => st.status = fill(tr("The file picker could not open: {}"), &[&e.to_string()]),
    });
}

const EXPOSURE_W: f32 = 228.0;

/// Deadlock Mod Manager's exposure slider, in quarter stops; a double press on the value resets.
fn exposure_row(u: &Ui, key: &str, stops: f32) -> El {
    let frac = ((stops + EXPOSURE_STOPS) / (2.0 * EXPOSURE_STOPS)).clamp(0.0, 1.0);
    let (k1, k2, k3) = (key.to_string(), key.to_string(), key.to_string());
    let value = if stops == 0.0 { "0".to_string() } else { format!("{stops:+.2}") };
    col()
        .w_full()
        .px(8.0)
        .gap(2.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(text(tr("Exposure").to_string()).font(SMALL).color(Color::WHITE.alpha(0.86)).grow(1.0))
                .child(text(value).font(MICRO).color(Color::WHITE.alpha(0.6)).key(format!("viewer-{key}-exposure-reset")).role("button").label(tr("Reset exposure").to_string()).cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| set_exposure(st, &k3, 0.0))),
        )
        .child(
            el().key(format!("viewer-{key}-exposure"))
                .w(EXPOSURE_W)
                .h(20.0)
                .focusable(true)
                .role("slider")
                .label(tr("Exposure").to_string())
                .cursor(rune::Cursor::Grab)
                .child(el().absolute(0.0, 8.0).w(EXPOSURE_W).h(4.0).radius(2.0).fill(Color::WHITE.alpha(0.14)).a11y_hidden())
                .child(el().absolute(EXPOSURE_W / 2.0 - 0.5, 5.0).w(1.0).h(10.0).fill(Color::WHITE.alpha(0.3)).a11y_hidden())
                .child(el().absolute((EXPOSURE_W * frac - 7.0).clamp(0.0, EXPOSURE_W - 14.0), 3.0).size(14.0, 14.0).radius(7.0).fill(u.accent).a11y_hidden())
                .on_drag(move |st: &mut State, d: &mut Drag, _tx: &mut Tx| {
                    if d.phase == Phase::Up && d.cancelled() {
                        return;
                    }
                    let w = if d.w > 0.0 { d.w } else { EXPOSURE_W };
                    set_exposure(st, &k1, (d.x / w).clamp(0.0, 1.0) * 2.0 * EXPOSURE_STOPS - EXPOSURE_STOPS);
                })
                .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                    let step = match ev.key {
                        Key::Left | Key::Down => -0.25,
                        Key::Right | Key::Up => 0.25,
                        Key::Home => -2.0 * EXPOSURE_STOPS,
                        Key::End => 2.0 * EXPOSURE_STOPS,
                        _ => return false,
                    };
                    let now = st.viewers.map.get(&k2).map_or(0.0, |v| v.ui.look.exposure);
                    set_exposure(st, &k2, now + step);
                    true
                }),
        )
}

fn set_look(s: &mut State, key: &str, f: impl FnOnce(&mut LookOptions)) {
    let Some(v) = s.viewers.map.get_mut(key) else { return };
    f(&mut v.ui.look);
    let look = v.ui.look;
    remember(v);
    send(s, key, Cmd::Look(look));
}

pub fn toggle_rotate(s: &mut State, key: &str) {
    let Some(v) = s.viewers.map.get_mut(key) else { return };
    v.ui.auto_rotate = !v.ui.auto_rotate;
    let on = v.ui.auto_rotate;
    remember(v);
    send(s, key, Cmd::AutoRotate(on));
}

fn toggle_panel(s: &mut State, key: &str) {
    if let Some(v) = s.viewers.map.get_mut(key) {
        v.ui.panel = !v.ui.panel;
    }
}

fn pick_image(s: &mut State, key: &str, tx: &mut Tx) {
    let options = rune::FileDialogOptions { title: tr("Choose a background picture").to_string(), ..Default::default() }.filter(tr("Pictures"), &["png", "jpg", "jpeg", "webp"]);
    let k = key.to_string();
    let _ = s;
    tx.open_file(options, move |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            // Grimoire's limits: 12 MB on disk.
            if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX) > 12 * 1024 * 1024 {
                st.status = tr("That picture is over 12 MB.").to_string();
                return;
            }
            if let Some(v) = st.viewers.map.get_mut(&k) {
                v.ui.image = Some(path.clone());
                v.ui.look.backdrop = Backdrop::Image;
                remember(v);
            }
            send(st, &k, Cmd::BackdropImage(Some(path)));
        }
        Ok(None) => {}
        Err(e) => st.status = fill(tr("The file picker could not open: {}"), &[&e.to_string()]),
    });
}

/// Where a recording of `title` goes, with its extension.
pub fn recording_path(title: &str, ext: &str) -> PathBuf {
    super::pictures_dir().join(super::capture_name(title).replace(".png", &format!(".{ext}")))
}

fn gif(s: &mut State, key: &str, cmd: GifCmd) {
    send(s, key, Cmd::Gif(cmd));
}

fn toggle_gif(s: &mut State, key: &str) {
    let Some(v) = s.viewers.map.get(key) else { return };
    let cmd = match v.ui.gif {
        GifState::Idle => GifCmd::Start,
        GifState::Recording { .. } => GifCmd::Stop,
        GifState::Review { .. } => return,
    };
    gif(s, key, cmd);
}

fn toggle_video(s: &mut State, key: &str) {
    let Some(v) = s.viewers.map.get(key) else { return };
    let ext = sigil_viewer::capture::native_video_format().extension();
    let cmd = if v.ui.video.is_some() { VideoCmd::Stop } else { VideoCmd::Start(recording_path(&v.title, ext)) };
    send(s, key, Cmd::Video(cmd));
}

/// The strip's extra buttons: auto-rotate, the Controls panel, record a GIF, record a video.
pub fn tools(v: &View, u: &Ui, key: &str) -> Vec<El> {
    let (k1, k2, k3, k4) = (key.to_string(), key.to_string(), key.to_string(), key.to_string());
    let recording_gif = matches!(v.ui.gif, GifState::Recording { .. });
    let in_review = matches!(v.ui.gif, GifState::Review { .. });
    let recording_video = v.ui.video.is_some();
    let mut out = vec![
        strip_button(u, &super::icons::ROTATE_CW, tr("Auto-rotate"), v.ui.auto_rotate).checked(v.ui.auto_rotate).on_press(move |st: &mut State| toggle_rotate(st, &k1)),
        strip_button(u, &crate::icons::SLIDERS_HORIZONTAL, tr("Controls"), v.ui.panel).key(format!("viewer-{key}-controls")).on_press(move |st: &mut State| toggle_panel(st, &k2)),
    ];
    if !in_review {
        let (glyph, label) = if recording_gif { (&crate::icons::SQUARE_STOP, tr("Stop recording")) } else { (&super::icons::FILM, tr("Record a GIF")) };
        out.push(strip_button(u, glyph, label, recording_gif).on_press(move |st: &mut State| toggle_gif(st, &k3)));
    }
    let (glyph, label) = if recording_video { (&crate::icons::SQUARE_STOP, tr("Stop recording")) } else { (&super::icons::VIDEO, tr("Record a video (up to 10 seconds)")) };
    out.push(strip_button(u, glyph, label, recording_video).on_press(move |st: &mut State| toggle_video(st, &k4)));
    out
}

fn section(label: &str) -> El {
    text(label.to_string()).font(MICRO).color(Color::WHITE.alpha(0.5)).pt(6.0).pb(2.0)
}

fn option_row(u: &Ui, key: String, label: &str, on: bool) -> El {
    row()
        .key(key)
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(if on { u.accent.alpha(0.18) } else { Color::TRANSPARENT })
        .hover(Color::WHITE.alpha(0.08))
        .focusable(true)
        .role("menuitemradio")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(text(label.to_string()).font(SMALL).color(if on { u.accent } else { Color::WHITE.alpha(0.86) }).grow(1.0).truncate().a11y_hidden())
        .child(if on { icon(&crate::icons::CHECK, 12.0).color(u.accent).a11y_hidden() } else { el().size(12.0, 12.0) })
}

fn switch_row(u: &Ui, key: String, label: &str, on: bool) -> El {
    row()
        .key(key)
        .w_full()
        .h(30.0)
        .px(8.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .hover(Color::WHITE.alpha(0.08))
        .focusable(true)
        .role("switch")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(text(label.to_string()).font(SMALL).color(Color::WHITE.alpha(0.86)).grow(1.0).a11y_hidden())
        .child(u.toggle_mark(on))
}

const GROUPS: [(&str, &str); 4] = [("idle", "Idle"), ("movement", "Movement"), ("combat", "Combat"), ("other", "Other")];

/// The Controls panel over the strip, attached at the app's root (`app::view`).
pub fn popover(s: &State, u: &Ui) -> Option<El> {
    let (key, v) = s.viewers.map.iter().find(|(k, v)| v.ui.panel && (s.viewers.fullscreen.is_none() || s.viewers.fullscreen.as_deref() == Some(k.as_str())))?;
    let key = key.as_str();
    let mut body = col().w_full().gap(1.0);
    if v.clips.len() > 1 {
        body = body.child(section(tr("Animation")));
        for (group, title) in GROUPS {
            let items: Vec<&(String, String, String)> = v.ui.labels.iter().filter(|(_, _, g)| g == group).collect();
            if items.is_empty() {
                continue;
            }
            body = body.child(text(tr(title).to_string()).font(MICRO).color(Color::WHITE.alpha(0.38)).px(8.0).pt(2.0));
            for (name, label, _) in items {
                let (k, n) = (key.to_string(), name.clone());
                let on = v.clip.as_deref() == Some(name.as_str());
                body = body.child(option_row(u, format!("viewer-{key}-clip-{name}"), label, on).on_press(move |st: &mut State| {
                    if let Some(v) = st.viewers.map.get_mut(&k) {
                        v.clip = Some(n.clone());
                        v.playing = true;
                    }
                    send(st, &k, Cmd::Clip(n.clone()));
                    send(st, &k, Cmd::Play(true));
                }));
            }
        }
    }
    if !v.scene {
        body = body.child(section(tr("Lighting")));
        for (lighting, label) in [(Lighting::Studio, tr("Studio")), (Lighting::Midtown, tr("Midtown dusk"))] {
            let k = key.to_string();
            body = body.child(option_row(u, format!("viewer-{key}-light-{label}"), label, v.ui.look.lighting == lighting).on_press(move |st: &mut State| set_look(st, &k, |l| l.lighting = lighting)));
        }
        body = body.child(section(tr("Background")));
        let mut choices: Vec<(Backdrop, bool, String)> = vec![(Backdrop::Studio, false, tr("Studio").to_string()), (Backdrop::Plain, false, tr("None").to_string()), (Backdrop::Studio, true, tr("Transparent").to_string())];
        choices.extend(GRADIENTS.iter().enumerate().map(|(i, g)| (Backdrop::Gradient(i), false, tr(g.name).to_string())));
        if v.ui.image.is_some() {
            choices.push((Backdrop::Image, false, tr("My picture").to_string()));
        }
        for (b, transparent, label) in choices {
            let k = key.to_string();
            let on = v.ui.look.transparent == transparent && (transparent || v.ui.look.backdrop == b);
            body = body.child(option_row(u, format!("viewer-{key}-bg-{label}"), &label, on).on_press(move |st: &mut State| {
                set_look(st, &k, |l| {
                    l.transparent = transparent;
                    if !transparent {
                        l.backdrop = b;
                    }
                })
            }));
        }
        let k = key.to_string();
        body = body.child(option_row(u, format!("viewer-{key}-bg-open"), tr("Open a picture..."), false).role("menuitem").on_press_tx(move |st: &mut State, tx: &mut Tx| pick_image(st, &k, tx)));
    }
    body = body.child(section(tr("Show")));
    let k = key.to_string();
    body = body.child(switch_row(u, format!("viewer-{key}-sw-rotate"), tr("Auto-rotate"), v.ui.auto_rotate).on_press(move |st: &mut State| toggle_rotate(st, &k)));
    type Flag = fn(&mut LookOptions) -> &mut bool;
    let mut flags: Vec<(&str, bool, Flag)> = vec![
        ("Cloth", v.ui.look.cloth, |l| &mut l.cloth),
        ("Particles", v.ui.look.particles, |l| &mut l.particles),
        ("Bloom", v.ui.look.bloom, |l| &mut l.bloom),
        ("Shadows", v.ui.look.shadows, |l| &mut l.shadows),
        ("Axis gizmo", v.ui.look.gizmo, |l| &mut l.gizmo),
    ];
    if v.ui.can_wireframe {
        flags.push(("Wireframe", v.ui.look.wireframe, |l| &mut l.wireframe));
    }
    for (label, on, field) in flags {
        let k = key.to_string();
        body = body.child(switch_row(u, format!("viewer-{key}-sw-{label}"), tr(label), on).on_press(move |st: &mut State| {
            set_look(st, &k, |l| {
                let f = field(l);
                *f = !*f;
            })
        }));
    }
    body = body.child(exposure_row(u, key, v.ui.look.exposure));
    body = body.child(section(tr("Pictures and recordings")));
    let k = key.to_string();
    body = body.child(switch_row(u, format!("viewer-{key}-sw-ask-where"), tr("Ask where to save"), v.ui.ask_where).on_press(move |st: &mut State| toggle_ask_where(st, &k)));
    let k = key.to_string();
    Some(
        col()
            .key(format!("viewer-{key}-panel"))
            .role("menu")
            .label(tr("Controls").to_string())
            .w(260.0)
            .padding(6.0)
            .fill(shade("#121214").alpha(0.97))
            .radius(crate::ui::r(10.0))
            .border(1.0, Color::WHITE.alpha(0.1))
            .shadow(12.0, 32.0, Color::BLACK.alpha(0.5))
            .anchored_fit(format!("viewer-{key}-controls"), Side::Top, rune::Align::End, 8.0)
            .transition(crate::motion::pop(true))
            .modal(true, move |st: &mut State| {
                if let Some(v) = st.viewers.map.get_mut(&k) {
                    v.ui.panel = false;
                }
            })
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    for v in st.viewers.map.values_mut() {
                        v.ui.panel = false;
                    }
                    true
                } else {
                    false
                }
            })
            .child(u.scroll_area(col().w_full().shrink().min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(body))),
    )
}

fn clock(seconds: f32) -> String {
    format!("0:{:02}", seconds.max(0.0) as u32)
}

/// The REC pill while a video or a GIF records (Deadlock Mod Manager's pill, top centre).
pub fn recording_pill(v: &View, u: &Ui, key: &str) -> Option<El> {
    let (line, k) = match (&v.ui.gif, v.ui.video) {
        (_, Some(t)) => (format!("REC {} / 0:{:02}", clock(t), sigil_viewer::capture::VIDEO_MAX_SECONDS), key.to_string()),
        (GifState::Recording { frames, max }, None) => (fill(tr("GIF {} of {} frames"), &[&frames.to_string(), &max.to_string()]), key.to_string()),
        _ => return None,
    };
    let video = v.ui.video.is_some();
    Some(
        row()
            .key(format!("viewer-{key}-rec"))
            .gap(8.0)
            .px(12.0)
            .h(30.0)
            .center_y()
            .radius(15.0)
            .fill(shade("#000000").alpha(0.7))
            .border(1.0, Color::WHITE.alpha(0.12))
            .focusable(true)
            .role("button")
            .label(tr("Stop recording").to_string())
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| if video { send(st, &k, Cmd::Video(VideoCmd::Stop)) } else { gif(st, &k, GifCmd::Stop) })
            .child(el().size(8.0, 8.0).radius(4.0).fill(shade("#e5484d")).a11y_hidden())
            .child(text(line).font(SMALL).color(Color::WHITE.alpha(0.92)).a11y_hidden())
            .child(icon(&crate::icons::SQUARE_STOP, 12.0).color(u.accent).a11y_hidden()),
    )
}

const TIMELINE_W: f32 = 320.0;

fn frame_at(x: f32, count: usize) -> usize {
    ((x / TIMELINE_W).clamp(0.0, 0.999_9) * count as f32) as usize
}

fn timeline(v: &View, u: &Ui, key: &str, count: usize, start: usize, end: usize, pos: usize) -> El {
    let n = count.max(1) as f32;
    let (sx, ex, px) = (start as f32 / n * TIMELINE_W, end as f32 / n * TIMELINE_W, (pos as f32 + 0.5) / n * TIMELINE_W);
    let k = key.to_string();
    let k2 = key.to_string();
    el().key(format!("viewer-{key}-gif-timeline"))
        .w(TIMELINE_W)
        .h(40.0)
        .radius(crate::ui::r(6.0))
        .clip_rounded(crate::ui::r(6.0))
        .fill(Color::WHITE.alpha(0.06))
        .focusable(true)
        .role("slider")
        .label(tr("GIF position").to_string())
        .cursor(rune::Cursor::Pointer)
        .child(el().absolute(0.0, 0.0).w(TIMELINE_W).h(40.0).pixels_fit(&v.gif_strip, SurfaceFit::Fill).opacity(0.5).a11y_hidden())
        .child(el().absolute(sx, 0.0).w((ex - sx).max(2.0)).h(40.0).fill(u.accent.alpha(0.15)).border(2.0, u.accent).a11y_hidden())
        .child(el().absolute(sx, 0.0).w(12.0).h(40.0).fill(u.accent).a11y_hidden())
        .child(el().absolute((ex - 12.0).max(0.0), 0.0).w(12.0).h(40.0).fill(u.accent).a11y_hidden())
        .child(el().absolute((px - 1.0).max(0.0), 0.0).w(2.0).h(40.0).fill(Color::WHITE).a11y_hidden())
        .on_drag(move |st: &mut State, d: &mut Drag, tx: &mut Tx| {
            let Some(v) = st.viewers.map.get_mut(&k) else { return };
            let GifState::Review { count, start, end, .. } = v.ui.gif else { return };
            let n = count.max(1) as f32;
            let at = frame_at(d.x, count);
            if d.phase == Phase::Down {
                let (sx, ex) = (start as f32 / n * TIMELINE_W, end as f32 / n * TIMELINE_W);
                v.ui.gif_drag = if (d.x - sx).abs() < 14.0 { 0 } else if (d.x - ex).abs() < 14.0 { 1 } else { 2 };
            }
            let cmd = match v.ui.gif_drag {
                // A handle keeps at least one frame and never crosses the other (Grimoire's timeline).
                0 => GifCmd::Trim(at.min(end.saturating_sub(1)), end),
                1 => GifCmd::Trim(start, (at + 1).max(start + 1)),
                _ => GifCmd::Seek(at.clamp(start, end.saturating_sub(1))),
            };
            send(st, &k, Cmd::Gif(cmd));
            if d.phase == Phase::Move {
                tx.live_only();
            }
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            let Some(v) = st.viewers.map.get(&k2) else { return false };
            let GifState::Review { start, end, pos, .. } = v.ui.gif else { return false };
            let step = if tx.mods().shift { 12 } else { 1 };
            let to = match ev.key {
                Key::Left => pos.saturating_sub(step).max(start),
                Key::Right => (pos + step).min(end.saturating_sub(1)),
                Key::Home => start,
                Key::End => end.saturating_sub(1),
                _ => return false,
            };
            send(st, &k2, Cmd::Gif(GifCmd::Seek(to)));
            true
        })
}

/// Grimoire's GIF review: the take looping, its trim timeline, Save GIF, Retake and Cancel.
pub fn gif_review(v: &View, u: &Ui, key: &str) -> Option<El> {
    let GifState::Review { count, start, end, pos, playing, saving } = v.ui.gif else { return None };
    let fps = sigil_viewer::capture::GIF_FPS;
    let readout = format!("{:.2} / {:.2} s", (pos.saturating_sub(start)) as f32 / fps, (end - start) as f32 / fps);
    let (k1, k2, k3, k4) = (key.to_string(), key.to_string(), key.to_string(), key.to_string());
    let title = v.title.clone();
    let play = if playing { super::strip_button(u, &super::icons::PAUSE, tr("Pause"), false) } else { super::strip_button(u, &crate::icons::PLAY, tr("Play"), false) };
    Some(
        col()
            .key(format!("viewer-{key}-gif-review"))
            .role("dialog")
            .label(tr("Review the GIF").to_string())
            .w(TIMELINE_W + 24.0)
            .padding(12.0)
            .gap(8.0)
            .radius(crate::ui::r(12.0))
            .fill(shade("#0d0d0f").alpha(0.94))
            .border(1.0, Color::WHITE.alpha(0.1))
            .child(el().w(TIMELINE_W).h(144.0).radius(crate::ui::r(6.0)).clip_rounded(crate::ui::r(6.0)).fill(shade("#000000")).pixels_fit(&v.gif_preview, SurfaceFit::Fit).role("img").label(tr("GIF preview").to_string()))
            .child(row().w_full().gap(6.0).center_y().child(play.on_press(move |st: &mut State| gif(st, &k1, GifCmd::Play(!playing)))).child(text(readout).font(MICRO).color(Color::WHITE.alpha(0.7))))
            .child(timeline(v, u, key, count, start, end, pos))
            .child(
                row()
                    .w_full()
                    .gap(6.0)
                    .child(super::strip_text(u, if saving { tr("Saving...").to_string() } else { tr("Save GIF").to_string() }, tr("Save GIF"), true).on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        let path = recording_path(&title, "gif");
                        if !st.viewers.map.get(&k2).is_some_and(|v| v.ui.ask_where) {
                            return gif(st, &k2, GifCmd::Save(path));
                        }
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let k = k2.clone();
                        ask_save_path(tx, name, "gif", move |st, path| gif(st, &k, GifCmd::Save(path)));
                    }))
                    .child(super::strip_text(u, tr("Retake").to_string(), tr("Retake"), false).on_press(move |st: &mut State| {
                        gif(st, &k3, GifCmd::Cancel);
                        gif(st, &k3, GifCmd::Start);
                    }))
                    .child(super::strip_text(u, tr("Cancel").to_string(), tr("Cancel"), false).on_press(move |st: &mut State| gif(st, &k4, GifCmd::Cancel))),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_survive_junk() {
        let p = Prefs {
            auto_rotate: true,
            look: LookOptions { backdrop: Backdrop::Gradient(3), lighting: Lighting::Midtown, bloom: false, shadows: true, cloth: false, particles: true, transparent: false, exposure: -0.75, wireframe: true, gizmo: true },
            image: Some(PathBuf::from("/tmp/x.png")),
            ask_where: true,
        };
        assert_eq!(Prefs::parse(&p.to_json()), p);
        assert_eq!(Prefs::parse(r#"{"exposure": 99}"#).look.exposure, EXPOSURE_STOPS);
        assert_eq!(Prefs::parse(r#"{"exposure": "x"}"#).look.exposure, 0.0);
        assert_eq!(Prefs::parse("nope"), Prefs::default());
        assert_eq!(backdrop_from("gradient:99"), Backdrop::Studio);
    }

    #[test]
    fn camera_poses_parse_and_drop_junk() {
        let map = parse_poses(r#"{"inferno": [0.4, -0.2, 1.5], "bad": [1, 2], "zero": [0, 0, 0], "text": "x", "nan": [1, 2, "y"]}"#);
        assert_eq!(map.keys().collect::<Vec<_>>(), ["inferno"]);
        assert_eq!(pose_from(&map["inferno"]), Some([0.4, -0.2, 1.5]));
        assert!(parse_poses("not json").is_empty());
    }

    #[test]
    fn recordings_land_in_pictures_sigil() {
        let p = recording_path("Haze", "gif");
        assert!(p.starts_with(super::super::pictures_dir()));
        assert_eq!(p.extension().and_then(|e| e.to_str()), Some("gif"));
    }

    #[test]
    fn timeline_positions_map_to_frames() {
        assert_eq!(frame_at(0.0, 48), 0);
        assert_eq!(frame_at(TIMELINE_W, 48), 47);
        assert_eq!(frame_at(TIMELINE_W / 2.0, 48), 24);
    }
}
