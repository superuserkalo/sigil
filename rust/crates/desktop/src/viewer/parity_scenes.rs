//! Headless scenes for the viewer's Controls panel, backgrounds, lighting and recordings, on
//! the stand-in hero: `cargo run -p dah-desktop -- --snap out.png --scene viewer-controls`.

use super::controls::Prefs;
use super::worker::{GifState, LookOptions};
use crate::app::State;
use sigil_viewer::{Backdrop, Lighting};

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: [Scene; 7] = [
    ("viewer-controls", controls),
    ("viewer-backdrop-ember", backdrop_ember),
    ("viewer-backdrop-studio-gradient", backdrop_dmm),
    ("viewer-backdrop-none", backdrop_none),
    ("viewer-transparent", transparent),
    ("viewer-gif-review", gif_review),
    ("viewer-recording", recording),
];

fn with_look(s: &mut State, f: impl FnOnce(&mut LookOptions)) {
    let mut prefs = Prefs::default();
    f(&mut prefs.look);
    s.viewers.prefs = Some(prefs);
    super::scene_viewer(s);
}

/// The Controls panel open over the strip: the grouped clips, lighting, backgrounds, switches.
fn controls(s: &mut State) {
    super::scene_viewer(s);
    if let Some(v) = s.viewers.map.get_mut("demo") {
        v.ui.panel = true;
    }
}

/// Grimoire's Warm ember soul backdrop under Midtown dusk's warm key.
fn backdrop_ember(s: &mut State) {
    with_look(s, |l| {
        l.backdrop = Backdrop::Gradient(2);
        l.lighting = Lighting::Midtown;
    });
}

/// Deadlock Mod Manager's studio gradient.
fn backdrop_dmm(s: &mut State) {
    with_look(s, |l| l.backdrop = Backdrop::Gradient(0));
}

/// Grimoire's None: the flat viewer grey.
fn backdrop_none(s: &mut State) {
    with_look(s, |l| l.backdrop = Backdrop::Plain);
}

/// Transparent: no backdrop and no floor (a capture to paste elsewhere).
fn transparent(s: &mut State) {
    with_look(s, |l| l.transparent = true);
}

/// The GIF review after a take: the frame, the trim strip with a kept range, Save, Retake.
fn gif_review(s: &mut State) {
    super::scene_viewer(s);
    let Some(v) = s.viewers.map.get_mut("demo") else { return };
    // The take's frames: the stand-in at the GIF's size.
    let (pw, ph) = sigil_viewer::capture::gif_dimensions(1360, 860);
    if let Ok(mut viewer) = super::worker::prepare(&super::Source::StandIn(super::Variant::Modded), None, &std::sync::atomic::AtomicBool::new(false), &mut |_| {}) {
        viewer.playback.playing = false;
        viewer.seek(1.0);
        let small = viewer.render(pw, ph, 0.0).to_vec();
        let thumb = sigil_viewer::capture::downscale(&small, pw, ph, 80, 64);
        let mut strip = vec![0u8; 480 * 64 * 4];
        for k in 0..6 {
            for y in 0..64 {
                strip[(y * 480 + k * 80) * 4..(y * 480 + k * 80 + 80) * 4].copy_from_slice(&thumb[y * 80 * 4..(y + 1) * 80 * 4]);
            }
        }
        v.gif_preview.set(pw, ph, small);
        v.gif_strip.set(480, 64, strip);
    }
    let review = GifState::Review { count: 48, start: 6, end: 40, pos: 20, playing: false, saving: false };
    // The poll copies the render thread's report into the view: set both.
    if let Ok(mut sh) = v.shared.lock() {
        sh.gif = review.clone();
    }
    v.ui.gif = review;
}

/// A video recording: the REC pill with its clock.
fn recording(s: &mut State) {
    super::scene_viewer(s);
    if let Some(v) = s.viewers.map.get_mut("demo") {
        v.ui.video = Some(3.4);
        if let Ok(mut sh) = v.shared.lock() {
            sh.video = Some(3.4);
        }
    }
}
