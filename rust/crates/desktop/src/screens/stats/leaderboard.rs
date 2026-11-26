//! The region leaderboard, with the player's own place marked.
//!
//! Region and season come from the API: regions by name, and the season list
//! from Valve's ranked seasons. The player's row is the one whose candidate
//! account ids contain their account id; it is marked in place when it is on the
//! first page and repeated under the list when it is not.

use deadlock_core::stats::{Leaderboard, LeaderboardEntry, StatsSnapshot};
use rune::{Color, El, col, el, row, spacer, text};

use crate::app::State;
use crate::ui::{ActionKind, BODY, BODY_MED, H2, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::{Fetched, LEADERBOARD_ROWS};

const REGIONS: [&str; 5] = ["Europe", "NAmerica", "SAmerica", "Asia", "Oceania"];

pub(super) fn leaderboard_section(s: &State, u: &Ui, snap: &StatsSnapshot) -> El {
    let region = s.stats.region().to_string();
    let mut segments = Vec::new();
    for name in REGIONS {
        let chosen = region == name;
        segments.push(
            crate::ui::SegPart::new(format!("stats-region-{name}"), tr(name)).on(chosen).with(move |e| e.on_press(move |st: &mut State| crate::data::start_leaderboard_load(st, name))),
        );
    }

    let caption = match &s.stats.leaderboard {
        Fetched::Ready(board) => match &board.season_name {
            Some(season) => crate::screens::settings::i18n::fill(
                tr("{}, {}"),
                &[season, tr(&board.region)],
            ),
            None => tr(&board.region).to_string(),
        },
        _ => tr("The region's ranked board this season").to_string(),
    };

    let mut section = col().key("stats-leaderboard").w_full().gap(12.0).child(
        row()
            .w_full()
            .h(32.0)
            .gap(12.0)
            .center_y()
            .child(text(tr("Leaderboard")).font(H2).color(u.ink))
            .child(
                text(caption)
                    .font(SMALL)
                    .color(u.muted)
                    .truncate()
                    .min_w(0.0),
            )
            .child(spacer())
            .child(crate::ui::Segmented::new("stats-region", tr("Region")).parts(segments).build(u)),
    );

    section = match &s.stats.leaderboard {
        Fetched::Idle => section
            .child(
                text(tr("Where you sit against everyone else in your region."))
                    .font(BODY)
                    .color(u.muted),
            )
            .child(
                row()
                    .w_full()
                    .child(
                        u.action(tr("Show leaderboard"), ActionKind::Secondary)
                            .key("stats-leaderboard-load")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                let region = st.stats.region().to_string();
                                crate::data::start_leaderboard_load(st, region)
                            }),
                    ),
            ),
        Fetched::Loading => section.child(
            text(tr("Loading the board"))
                .font(BODY)
                .color(u.muted),
        ),
        Fetched::Failed(error) => section.child(problem(u, error)),
        Fetched::Ready(board) => section.child(body(s, u, board, snap.account_id)),
    };
    section
}

fn body(s: &State, u: &Ui, board: &Leaderboard, account_id: u64) -> El {
    if board.entries.is_empty() {
        return col().w_full().gap(12.0).child(
            text(tr("No one is on this board yet. It fills as the season is played."))
                .font(BODY)
                .color(u.muted),
        );
    }
    let all = s.stats.leaderboard_all;
    let shown = if all {
        board.entries.len().min(100)
    } else {
        LEADERBOARD_ROWS.min(board.entries.len())
    };
    let mine = board.entry_of(account_id);
    let my_place = board.place_of(account_id);

    let mut table = col().w_full().child(head_row(
        u,
        &[
            (tr("Place"), 56.0, false),
            (tr("Player"), 0.0, false),
        ],
    ));
    let mut mine_shown = false;
    for entry in board.entries.iter().take(shown) {
        let is_me = account_id > 0 && entry.possible_account_ids.contains(&account_id);
        mine_shown |= is_me;
        table = table.child(entry_row(s, u, entry, is_me));
    }
    if !mine_shown
        && let Some(entry) = mine
    {
        table = table
            .child(
                row()
                    .w_full()
                    .h(24.0)
                    .center_y()
                    .child(text(tr("· · ·")).font(SMALL).color(u.muted)),
            )
            .child(entry_row(s, u, entry, true));
    }

    let hidden = board.entries.len() > shown;
    let foot = row()
        .w_full()
        .h(32.0)
        .gap(12.0)
        .center_y()
        .child(spacer())
        .child(match my_place {
            Some(place) => text(crate::screens::settings::i18n::fill(
                tr("Your place: {}"),
                &[&place.to_string()],
            ))
            .font(TNUM_SMALL)
            .color(u.muted),
            None => text(tr("You are not on this board yet."))
                .font(SMALL)
                .color(u.muted),
        })
        .child(if hidden {
            u.action(
                if all {
                    tr("Show fewer")
                } else {
                    tr("Show more")
                },
                ActionKind::Ghost,
            )
            .key("stats-leaderboard-toggle")
            .mr(-12.0)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.stats.leaderboard_all = !st.stats.leaderboard_all)
        } else {
            el()
        });

    col().w_full().gap(12.0).child(table).child(foot)
}

fn entry_row(s: &State, u: &Ui, entry: &LeaderboardEntry, mine: bool) -> El {
    let place = match entry.rank {
        Some(rank) => format!("#{rank}"),
        None => "-".to_string(),
    };
    let name = entry
        .account_name
        .clone()
        .unwrap_or_else(|| tr("No name").to_string());
    row()
        .key(format!(
            "lb-{}-{}",
            entry.rank.unwrap_or(0),
            entry.possible_account_ids.first().copied().unwrap_or(0)
        ))
        .w_full()
        .h(40.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .fill(if mine { u.selected } else { Color::TRANSPARENT })
        .child(
            text(place)
                .font(TNUM_MED)
                .color(if mine { u.ink } else { u.prose })
                .w(56.0),
        )
        .child(name_cell(s, u, entry, name, mine))
        .child(if mine {
            text(tr("You")).font(TNUM_SMALL).color(u.muted)
        } else {
            el()
        })
        .child(el().w(4.0))
}

/// The player: a link to their page when the board names one account, a press that lists the
/// candidates when it names several, plain text when it names none.
fn name_cell(s: &State, u: &Ui, entry: &LeaderboardEntry, name: String, mine: bool) -> El {
    let ids = entry.possible_account_ids.clone();
    let holder = row().grow(1.0).min_w(0.0).center_y();
    match ids.len() {
        1 => holder.child(super::shared::player_link(s, u, ids[0], &name, 20.0, mine)),
        0 => holder.child(text(name).font(if mine { BODY_MED } else { BODY }).color(u.ink).truncate().min_w(0.0)),
        n => holder.gap(8.0).child(
            row()
                .key(format!("lb-candidates-{}", entry.rank.unwrap_or(0)))
                .min_w(0.0)
                .h(28.0)
                .pr(6.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(2.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(crate::screens::settings::i18n::fill(tr("{} possible accounts"), &[&n.to_string()]))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| super::lookup::show_candidates(st, ids.clone()))
                .child(text(name).font(if mine { BODY_MED } else { BODY }).color(u.ink).truncate().min_w(0.0).shrink())
                .child(text(crate::screens::settings::i18n::fill(tr("{} possible accounts"), &[&n.to_string()])).font(SMALL).color(u.muted)),
        ),
    }
}
