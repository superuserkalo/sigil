//! Deadlock Modding Workbench: a native Rune desktop app.
//!
//! `cargo run -p dah-desktop` opens the window. Headless scenes render with
//! `cargo run -p dah-desktop -- --snap out.png --scene index`.

mod agent;
mod app;
mod data;
mod model;
mod screens;
mod ui;

use app::State;
use rune::AppSpec;

fn main() {
    rune::run(AppSpec::<State> {
        scenes: app::scenes(),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(app::on_key),
        min_size: Some((1120.0, 720.0)),
        ..AppSpec::new(
            "Deadlock Modding Workbench",
            (1440.0, 960.0),
            data::init_state,
            app::view,
        )
    });
}
