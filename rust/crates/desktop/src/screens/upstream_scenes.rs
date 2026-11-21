//! Headless scenes for the upstream round (2026-11-21): the mod safety review list, custom
//! mod sources in Browse, GameBanana notices, hidden mods and creators, and Street Brawl in
//! Stats. Registered in `main.rs` next to the app's own list.

use crate::app::State;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("upstream-safety-review", safety_review),
    ("upstream-safety-notice", safety_notice),
    ("upstream-browse-source", browse_source),
    ("upstream-browse-source-details", browse_source_details),
    ("upstream-hidden", hidden),
    ("upstream-notices", notices),
    ("upstream-stats-brawl", stats_brawl),
];

/// The review list: two mods in Review and one Allowed, open on its Disallow.
pub fn safety_review(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::safety_review::stage_scene(s);
}

/// The Installed page with the "N mods need attention" notice.
pub fn safety_notice(s: &mut State) {
    crate::app::scene_demo_installed(s);
    crate::screens::installed::safety_review::stage_scene(s);
    s.installed.tools.review.open = false;
}

/// Browse on a custom mod source: the store switch in the header and the source's cards.
pub fn browse_source(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.screen = crate::model::Screen::Browse;
    crate::screens::browse::sources::stage_scene(s, false);
}

/// A source mod's details: Markdown read as text, two files with their own Install, history.
pub fn browse_source_details(s: &mut State) {
    browse_source(s);
    crate::screens::browse::sources::stage_scene(s, true);
}

/// Hidden items with dates, a creator hidden from one of their mods, and a note.
pub fn hidden(s: &mut State) {
    crate::screens::browse::scenes::stage_hidden(s);
}

/// A withheld and obsolete mod's details with GameBanana's own words.
pub fn notices(s: &mut State) {
    crate::screens::browse::scenes::stage_notices(s);
}

/// Stats with two Street Brawl games in the list, tagged, out of the win rate and form.
pub fn stats_brawl(s: &mut State) {
    crate::screens::stats::scene_profile(s);
    if let Some(mut snap) = s.stats.snapshot.clone() {
        for item in snap.matches.iter_mut().skip(1).take(2) {
            item.game_mode = deadlock_core::stats::filters::GAME_MODE_STREET_BRAWL;
        }
        crate::screens::stats::set_snapshot(&mut s.stats, snap);
    }
}
