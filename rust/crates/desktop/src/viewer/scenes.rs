//! Headless scenes for every place the 3D viewer appears, on the bundled stand-ins so they
//! need no game: `cargo run -p dah-desktop -- --snap out.png --scene viewer-locker`.

use super::prepare::{self, Ask, Owner, Problem};
use super::Status;
use crate::app::State;
use crate::model::Screen;

/// A headless scene: its name and how it sets the state up.
pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: [Scene; 18] = [
    ("viewer", super::scene_viewer),
    ("viewer-split", super::scene_viewer_split),
    ("viewer-hideout", super::scene_viewer_hideout),
    ("viewer-locker", locker),
    ("viewer-locker-loading", locker_loading),
    ("viewer-locker-failed", locker_failed),
    ("viewer-locker-nogame", locker_no_game),
    ("viewer-locker-nogpu", locker_no_gpu),
    ("viewer-locker-hideout", locker_hideout),
    ("viewer-hideout-loading", hideout_loading),
    ("viewer-hideout-opening", hideout_opening),
    ("viewer-hideout-failed", hideout_failed),
    ("viewer-locker-opening", locker_opening),
    ("viewer-locker-gpu-failed", locker_gpu_failed),
    ("viewer-installed-menu", installed_menu),
    ("viewer-mod-dialog", mod_dialog),
    ("viewer-mod-dialog-loading", mod_dialog_loading),
    ("viewer-details", details),
];

fn haze() -> Ask {
    Ask::Hero { hero: "Haze".into() }
}

/// Locker: Haze's page, the 3D view beside the skins, the stand-in with mods against stock.
fn locker(s: &mut State) {
    s.viewers.stand_in = true;
    crate::screens::locker::scene_view3d(s);
    prepare::sync_locker(s);
}

/// The same while the export runs: the first of two steps.
fn locker_loading(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    prepare::stage_loading(s, prepare::LOCKER_KEY, haze(), Owner::Locker, "Preparing {} with your mods", (1, 2));
}

/// The export failed: the error, Try again and Send feedback.
fn locker_failed(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    let error = "Could not decode models/heroes_staging/haze/haze.vmdl_c: unsupported mesh buffer layout (VBIB version 3).";
    prepare::stage_problem(s, prepare::LOCKER_KEY, haze(), Owner::Locker, Problem::Failed(error.into()));
}

/// No Deadlock folder: what to do about it.
fn locker_no_game(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    s.game_root = None;
    prepare::sync_locker(s);
}

/// No GPU adapter: the hero's picture in place of the 3D view, the strip gone.
fn locker_no_gpu(s: &mut State) {
    locker(s);
    let hero = s.locker.heroes.iter().find(|h| h.display_name == "Haze");
    let still = hero
        .into_iter()
        .flat_map(|h| [h.render.clone(), h.card.clone()])
        .flatten()
        .find_map(|key| s.thumbnails.get(&key).filter(|p| p.size().0 > 0).cloned());
    if let Some(v) = s.viewers.map.get_mut(prepare::LOCKER_KEY) {
        v.status = Status::NoGpu("no adapter".into());
        v.pixels = rune::Pixels::new();
        if let Ok(mut shared) = v.shared.lock() {
            shared.status = Status::NoGpu("no adapter".into());
        }
    }
    if let Some(pixels) = still {
        super::set_still(s, prepare::LOCKER_KEY, pixels);
    }
}

/// The Locker's Hideout button: the room full screen with the hero at the spawn.
fn locker_hideout(s: &mut State) {
    locker(s);
    prepare::open_hideout(s, Some("Haze".into()));
}

/// The Hideout full screen while it is built.
fn hideout_loading(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    prepare::stage_loading(s, prepare::HIDEOUT_KEY, Ask::Hideout { hero: Some("Haze".into()) }, Owner::FullScreen, "Building the Hideout with your mods", (1, 1));
}

fn hideout() -> Ask {
    Ask::Hideout { hero: Some("Haze".into()) }
}

/// The Hideout's files are made; the render thread reads them: a way out, what it does, how long.
fn hideout_opening(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    let progress = sigil_viewer::LoadProgress { stage: sigil_viewer::LoadStage::Reading, done: 137, of: 412 };
    prepare::stage_view(s, prepare::HIDEOUT_KEY, hideout(), Owner::FullScreen, Status::Loading, Some(progress));
}

/// The render thread could not open the Hideout: the reason, Try again, Send feedback, Close.
fn hideout_failed(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    let error = "The graphics card reported an error: Validation Error: Buffer size 3221225472 is greater than the maximum buffer size (2147483648).";
    prepare::stage_view(s, prepare::HIDEOUT_KEY, hideout(), Owner::FullScreen, Status::Failed(error.into()), None);
}

/// The Locker's 3D view while its render thread loads the hero.
fn locker_opening(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    prepare::stage_view(s, prepare::LOCKER_KEY, haze(), Owner::Locker, Status::Loading, None);
}

/// The Locker's 3D view when the device failed to build its pipelines.
fn locker_gpu_failed(s: &mut State) {
    crate::screens::locker::scene_view3d(s);
    let error = "NVIDIA GeForce GTX 1060 (Dx12) could not build the 3D view's shaders: Internal error in ShaderStages(FRAGMENT) shader: FXC D3DCompile error (0x80004005)";
    prepare::stage_view(s, prepare::LOCKER_KEY, haze(), Owner::Locker, Status::Failed(error.into()), None);
}

fn demo_heroes(s: &mut State) {
    let fixed = &mut s.viewers.mod_heroes.fixed;
    fixed.insert(2, vec!["Paige".into()]);
    fixed.insert(4, vec!["Abrams".into(), "Haze".into(), "Wraith".into()]);
    fixed.insert(5, vec!["Sinclair".into()]);
    fixed.insert(9, vec!["Drifter".into()]);
    fixed.insert(11, vec!["Paradox".into()]);
}

/// Installed: the card menu of a switched-off skin offers Preview in 3D.
fn installed_menu(s: &mut State) {
    crate::app::scene_demo_installed(s);
    demo_heroes(s);
    // The switched-off group first in view, so the card the menu drops from is on screen.
    s.installed.collapsed_enabled = true;
    s.installed.menu = Some(crate::screens::installed::Menu::Card(9));
    s.installed.hovered = Some(9);
}

fn dialog_points(s: &mut State) {
    // The dialog's viewport at the default window: 1120 by 820 less the window padding, the
    // header and the hero picker.
    super::headless_points(s, prepare::DIALOG_KEY, [1088.0, 690.0]);
}

/// Preview in 3D for a mod that changes three heroes, on its first.
fn mod_dialog(s: &mut State) {
    crate::app::scene_demo_installed(s);
    demo_heroes(s);
    dialog_points(s);
    super::dialog::open(s, 4, "The Rebelcapin Collection");
}

/// Preview in 3D for a switched-off mod while its export runs.
fn mod_dialog_loading(s: &mut State) {
    crate::app::scene_demo_installed(s);
    demo_heroes(s);
    s.viewers.dialog = Some(super::dialog::Dialog { slot: 9, name: "Taokaka over Drifter".into(), heroes: vec!["Drifter".into()], hero: 0 });
    let ask = Ask::Mod { hero: "Drifter".into(), slot: 9, name: "Taokaka over Drifter".into() };
    prepare::stage_loading(s, prepare::DIALOG_KEY, ask, Owner::Dialog, "Preparing {} with this mod", (1, 2));
}

/// A mod's details for an installed mod: the Preview in 3D button beside its slot.
fn details(s: &mut State) {
    crate::screens::browse::scene_details(s);
    let Some(id) = s.browse_view.details.as_ref().map(|d| d.id) else { return };
    s.installed.library.mods.push(deadlock_core::import::ModMeta {
        slot: 6,
        size_bytes: 0,
        name: "Details fixture".into(),
        author: None,
        thumbnail_url: None,
        gamebanana_id: Some(id),
        source: "browse".into(),
        recorded_at: String::new(),
    });
    s.viewers.mod_heroes.fixed.insert(6, vec!["Haze".into()]);
    s.screen = Screen::Browse;
}
