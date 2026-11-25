#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! Sigil: a Deadlock mod manager, native on Rune.
//!
//! `cargo run -p dah-desktop` opens the window. Headless scenes render with
//! `cargo run -p dah-desktop -- --snap out.png --scene index`. The same executable is the dah
//! CLI (`dah-desktop cli doctor`) and the agent host (`dah-desktop agent-host`).

mod agent;
mod app;
mod art;
mod bench_scenes;
mod brand;
mod banner;
mod data;
mod deeplink;
mod forge;
mod glass;
mod helpers;
mod icons;
mod jobs;
#[cfg(test)]
mod jank_tests;
#[cfg(test)]
mod scroll_bar_tests;
#[cfg(test)]
mod toast_tests;
#[cfg(test)]
mod agent_gate_tests;
mod memory;
mod model;
mod motion;
mod notify;
mod package_manager;
mod prefs;
mod publish;
mod screens;
mod shell;
mod shell_w4;
mod shell_tips;
mod signin;
mod single_instance;
mod statusbar;
mod ui;
mod updater;
mod viewer;
mod win_frame;

use app::State;
use rune::AppSpec;

/// Counts live heap bytes for Settings > About (`memory.rs`): a few relaxed atomics per call
/// around the system allocator.
#[global_allocator]
static ALLOCATOR: procwatch::alloc::Counting = procwatch::alloc::Counting::system();

/// Every named scene this build offers (`--scene NAME`). Scenes of the agent and the pages under
/// it are listed only in a build that has the agent (`prefs::AGENT`).
fn all_scenes() -> &'static [(&'static str, fn(&mut State))] {
    let all: Vec<(&'static str, fn(&mut State))> = [app::scenes(), &deeplink::SCENES[..], &screens::settings::personalize::SCENES[..], screens::w4a_scenes::SCENES, screens::upstream_scenes::SCENES, screens::explorer::SCENES, screens::project_tools::SCENES, screens::dmm_next_scenes::SCENES, screens::w4c_scenes::SCENES, screens::servers::host::scenes::SCENES, screens::servers::regions::scenes::SCENES, screens::performance::benchmark::scenes::SCENES, &viewer::SCENES[..], &viewer::PARITY_SCENES[..], &bench_scenes::SCENES[..]]
        .concat();
    Box::leak(
        all.into_iter()
            .filter(|(name, _)| prefs::AGENT || !app::AGENT_SCENES.contains(name))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

fn main() -> std::process::ExitCode {
    // The data folder's one-time move from its old name comes before anything reads or writes
    // it (`data::start_data_folder`). Windows claims the single instance first: its claim uses a
    // named pipe, not the data folder, so a launch that hands over to a running Sigil (or waits
    // for one quitting after an update) never moves the folder under it. Windows may set
    // variables once threads exist; elsewhere the art's variable must come before any thread.
    let claim_first = cfg!(target_os = "windows") && !helpers::is_helper_launch() && !rune::is_headless();
    if !claim_first {
        data::start_data_folder();
        // The built-in art, unpacked once per build, before any thread starts.
        art::install();
    }
    // A helper subcommand runs before any window or Rune state exists.
    if let Some(code) = helpers::dispatch() {
        return code;
    }
    // A link launch carries exactly one argument; more means a crafted link broke out of the
    // handler command's quotes to pass flags. Refuse it before Rune reads them.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !deeplink::launch_args_safe(&args) {
        eprintln!("Sigil: refused a link launch with extra arguments");
        return std::process::ExitCode::from(2);
    }
    // One Sigil per user, before any window: a second launch hands its links over and exits.
    // Headless runs (snapshots, scripts) render on their own.
    if !rune::is_headless() && !single_instance::claim() {
        eprintln!("Sigil is already running: handed this launch to it");
        return std::process::ExitCode::SUCCESS;
    }
    if claim_first {
        data::start_data_folder();
        art::install();
    }
    // Updates: install a staged one, or roll back a version that never drew a frame.
    if let Some(code) = updater::on_start() {
        return code;
    }
    // Sigil's own crashes: the panic hook, the session marker and what the last run left.
    screens::settings::crash::begin();
    deeplink::startup();
    brand::install_app_icon();
    let scenes = all_scenes();
    // Settings > Appearance > Transparent sidebar: a see-through window only when the setting
    // is on at launch and the system can show glass (`glass.rs`).
    let glass = glass::launch(prefs::Prefs::load().transparent_sidebar);
    let sigil = rune::App::new(AppSpec::<State> {
        scenes,
        setup: Some(app::setup),
        poll: Some(poll),
        on_key: Some(app::on_key),
        min_size: Some((1040.0, 680.0)),
        ..AppSpec::new("Sigil", (1360.0, 860.0), data::init_state, app::view)
    })
    .titlebar(rune::Titlebar::new(ui::TITLEBAR_H))
    .window_options(rune::WindowOptions { transparent: glass, ..Default::default() })
    // The tray owns the close only when "Close to tray" is on; otherwise on_close quits as before.
    .keep_running()
    .on_close(screens::settings::tray::on_close)
    // The clean-exit marker goes here: nothing after `run` executes on a normal quit.
    .on_quit(screens::settings::crash::on_quit)
    // The window handle behind always-on-top (Crosshair's pin) and the shell switch.
    .on_window(|window| {
        shell::remember_window(window);
        screens::crosshair::remember_window(window);
        glass::on_window(window);
    })
    // Once: whether the GPU surface really composites the glass's see-through pixels.
    .on_frame_drawn(|rt| glass::frame_drawn(rt.transparent))
    .on_open_url(deeplink::on_open_url);
    // Scroll bars stay up whenever content overflows on Windows and Linux, where Rune would
    // otherwise fade them out at rest; macOS follows its own "Show scroll bars" setting.
    #[cfg(not(target_os = "macos"))]
    let sigil = sigil.args(ui::scroll_bars_shown(args));
    screens::settings::tray::install(sigil.handle());
    single_instance::attach(sigil.handle());
    {
        let handle = sigil.handle();
        glass::set_waker(move || handle.wake());
    }
    sigil.run();
    // Reached only when Rune exits in order or the window never opened; `on_quit` covers a normal quit.
    screens::settings::crash::end();
    rune::release_instance();
    std::process::ExitCode::SUCCESS
}

/// The app's poll, then a link's pending file pick (it needs the mod's details first).
fn poll(s: &mut State, t: f64, tx: &mut rune::Tx) -> bool {
    let changed = app::poll(s, t, tx);
    // The 3D viewer's render threads: loaded, failed, a picture saved.
    let viewer = viewer::poll(s);
    deeplink::poll(s) || changed || viewer || glass::take_dirty()
}
