#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! Sigil: a Deadlock mod manager with an agent inside, native on Rune.
//!
//! `cargo run -p dah-desktop` opens the window. Headless scenes render with
//! `cargo run -p dah-desktop -- --snap out.png --scene index`. The same executable is the dah
//! CLI (`dah-desktop cli doctor`) and the agent host (`dah-desktop agent-host`).

mod agent;
mod app;
mod art;
mod brand;
mod banner;
mod data;
mod deeplink;
mod helpers;
mod icons;
mod jobs;
mod memory;
mod model;
mod notify;
mod prefs;
mod publish;
mod screens;
mod shell;
mod signin;
mod single_instance;
mod ui;
mod updater;
mod win_frame;

use app::State;
use rune::AppSpec;

/// Counts live heap bytes for Settings > About (`memory.rs`): a few relaxed atomics per call
/// around the system allocator.
#[global_allocator]
static ALLOCATOR: procwatch::alloc::Counting = procwatch::alloc::Counting::system();

fn main() -> std::process::ExitCode {
    // The built-in art, unpacked once per build, before any thread starts.
    art::install();
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
    // Updates: install a staged one, or roll back a version that never drew a frame.
    if let Some(code) = updater::on_start() {
        return code;
    }
    deeplink::startup();
    brand::install_app_icon();
    let scenes: &'static [(&'static str, fn(&mut State))] =
        Box::leak(
            [app::scenes(), &deeplink::SCENES[..], &screens::settings::personalize::SCENES[..]]
                .concat()
                .into_boxed_slice(),
        );
    let sigil = rune::App::new(AppSpec::<State> {
        scenes,
        setup: Some(app::setup),
        poll: Some(poll),
        on_key: Some(app::on_key),
        min_size: Some((1040.0, 680.0)),
        ..AppSpec::new("Sigil", (1360.0, 860.0), data::init_state, app::view)
    })
    .titlebar(rune::Titlebar::new(ui::TITLEBAR_H))
    // The tray owns the close only when "Close to tray" is on; otherwise on_close quits as before.
    .keep_running()
    .on_close(screens::settings::tray::on_close)
    // The window handle behind always-on-top (Crosshair's pin) and the shell switch.
    .on_window(|window| {
        shell::remember_window(window);
        screens::crosshair::remember_window(window);
    })
    .on_open_url(deeplink::on_open_url);
    screens::settings::tray::install(sigil.handle());
    single_instance::attach(sigil.handle());
    sigil.run();
    rune::release_instance();
    std::process::ExitCode::SUCCESS
}

/// The app's poll, then a link's pending file pick (it needs the mod's details first).
fn poll(s: &mut State, t: f64, tx: &mut rune::Tx) -> bool {
    let changed = app::poll(s, t, tx);
    deeplink::poll(s) || changed
}
