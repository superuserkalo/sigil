//! Headless scenes for Deadlock Mod Manager's upcoming game, stats and creator features
//! (2026-11-21): the Fun run, the crash cause per mod, the convar catalogue, creator pages and
//! abandoned matches. Registered in `main.rs` next to the app's own list.

use crate::app::State;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("next-stats-roll", stats_roll),
    ("next-stats-roll-entry", stats_roll_entry),
    ("next-convars", convars),
    ("next-convars-preset", convars_preset),
    ("next-launch-crash", launch_crash),
    ("next-browse-creator", browse_creator),
    ("next-perf-custom-add", perf_custom_add),
    ("next-stats-season", stats_season),
];

/// Performance > Add from GitHub after a Look up.
pub fn perf_custom_add(s: &mut State) {
    crate::screens::performance::scene(s, "performance-on");
    crate::screens::performance::custom::scene_add(s);
}

/// Stats with the season list read and the newest season picked as the window.
pub fn stats_season(s: &mut State) {
    crate::screens::stats::scene_people(s);
    crate::screens::stats::more::stage_season(s);
}

/// Stats with a player loaded and a fixed Fun run at the end of the page.
pub fn stats_roll(s: &mut State) {
    crate::screens::stats::scene_profile(s);
    crate::screens::stats::roll::stage_scene(s);
}

/// Stats before any player is loaded: the Fun run sits under the account entry.
pub fn stats_roll_entry(s: &mut State) {
    crate::screens::stats::scene_entry(s);
    crate::screens::stats::roll::stage_scene(s);
}

/// Autoexec with the convar catalogue open on a search, one value from the game's own file.
pub fn convars(s: &mut State) {
    crate::screens::autoexec::scene_file(s);
    crate::screens::autoexec_convars::stage_scene(s);
}

/// A built-in game preset's preview over the Autoexec page.
pub fn convars_preset(s: &mut State) {
    crate::screens::autoexec::scene_file(s);
    crate::screens::autoexec_convars::stage_preview(s);
}

/// The crash card naming the HUD layout the game was loading, its console lines and the HUD check.
pub fn launch_crash(s: &mut State) {
    crate::screens::launch::scene_crash(s);
}

/// Browse scoped to one creator, with their page over the grid.
pub fn browse_creator(s: &mut State) {
    crate::screens::browse::scenes::scene_with(s, |_| {});
    crate::screens::browse::creator_page::stage_scene(s);
}
