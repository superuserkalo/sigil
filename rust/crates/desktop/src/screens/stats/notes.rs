//! Abandoned and unscored matches, after Deadlock Mod Manager's match details (GPL-3.0,
//! behaviour only): a match row says when the game recorded the player as leaving, and an open
//! scoreboard says who left and why a match did not count.
//!
//! The history row's `abandoned_time_s` and the metadata's per player `abandon_match_time_s`
//! and `player_match_outcome` are parsed in `deadlock_core::stats`; a not scored match is never
//! a loss there (`MatchOutcome::win`).

use deadlock_core::stats::board::MatchScoreboard;
use deadlock_core::stats::{MatchOutcome, MatchSummary};
use rune::{El, col, row, text};

use crate::screens::settings::i18n::fill;
use crate::ui::{BODY, SMALL, Ui};

use super::german::tr;

/// `12:05`.
pub(super) fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The tag on a match row where the player abandoned.
pub(super) fn abandon_tag(u: &Ui, item: &MatchSummary) -> Option<El> {
    let at = item.abandoned_seconds?;
    Some(
        row()
            .h(20.0)
            .px(6.0)
            .center()
            .radius(crate::ui::r(5.0))
            .border(1.0, u.rose.alpha(0.5))
            .help(fill(tr("The game recorded you as abandoning at {}."), &[&clock(at)]))
            .child(text(fill(tr("Left at {}"), &[&clock(at)])).font(SMALL).color(u.rose)),
    )
}

/// The match did not count for the page's player.
pub(super) fn not_scored(board: &MatchScoreboard) -> bool {
    board
        .me()
        .is_some_and(|me| me.outcome == Some(MatchOutcome::NotScored) && me.abandoned_seconds.is_none())
}

/// The line under the scoreboard's result: the player's own abandon, or for a match that did
/// not count, the teammates the game recorded as leaving.
pub(super) fn board_note(u: &Ui, board: &MatchScoreboard) -> Option<El> {
    let me = board.me()?;
    if let Some(at) = me.abandoned_seconds {
        return Some(
            text(fill(tr("You were recorded as abandoning at {}."), &[&clock(at)]))
                .font(BODY)
                .color(u.rose),
        );
    }
    let left = board.teammates_who_left();
    if me.outcome != Some(MatchOutcome::NotScored) {
        if left.is_empty() {
            return None;
        }
        let names: Vec<String> = left.iter().map(|player| leaver(player)).collect();
        return Some(
            text(fill(tr("Teammates who left: {}"), &[&names.join(", ")]))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        );
    }
    if left.is_empty() {
        return Some(
            text(tr("This match did not count for you. The data does not say why."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        );
    }
    let mut note = col().w_full().gap(2.0).child(
        text(tr("This match did not count for you, so it is not a loss. Teammates who left:"))
            .font(SMALL)
            .color(u.muted)
            .wrap(),
    );
    for player in left {
        note = note.child(text(leaver(player)).font(SMALL).color(u.prose));
    }
    Some(note)
}

fn leaver(player: &deadlock_core::stats::board::BoardPlayer) -> String {
    let hero = player
        .hero_name
        .clone()
        .unwrap_or_else(|| fill(tr("Hero {}"), &[&player.hero_id.to_string()]));
    let at = clock(player.abandoned_seconds.unwrap_or(0));
    match player.persona_name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => fill(tr("{} ({}) at {}"), &[name, &hero, &at]),
        None => fill(tr("{} at {}"), &[&hero, &at]),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_pads_seconds() {
        assert_eq!(super::clock(605), "10:05");
        assert_eq!(super::clock(59), "0:59");
    }
}
