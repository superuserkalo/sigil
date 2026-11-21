//! The scoreboard behind a match row: both teams, every final build, and the
//! souls each side held through the match.
//!
//! Opened from the match list; the data is one metadata response, so the board
//! waits on its own worker and the page keeps scrolling.

use deadlock_core::stats::board::{BoardPlayer, MatchScoreboard, SoulPoint};
use deadlock_core::stats::insight;
use rune::{Color, El, Key, KeyEvent, ScrollSpec, Shape, canvas, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

const CHART_H: f32 = 96.0;
const CHART_VB: f32 = 1000.0;
/// Width of the board dialog: two teams of six, with five number columns.
const BOARD_W: f32 = 920.0;
const PLAYER_W: f32 = 160.0;
const KDA_W: f32 = 88.0;
/// Fixed rank and MVP columns, so every row's numbers start at the same x.
const BADGE_W: f32 = 104.0;

/// The dialog over the page: the board for `match_id`, or its loading state.
pub(super) fn board_dialog(s: &State, u: &Ui, match_id: u64) -> El {
    let close = |st: &mut State| st.stats.board_open = None;
    let tier_names = s.stats.snapshot.as_ref().map(|snap| &snap.tier_names);
    let mut head = col().w_full().gap(4.0);
    let mut body = col().w_full().gap(16.0);
    let mut footer = row().w_full().gap(8.0).pt(4.0);

    match &s.stats.board {
        Fetched::Ready(board) if board.match_id == match_id => {
            // Header, curve and column heads stay put; only the two teams scroll.
            head = head
                .child(header(u, board))
                .child(souls(u, board).mt(8.0))
                .child(column_labels(u).mt(4.0));
            for team in team_order(board) {
                body = body.child(team_block(s, u, board, team, tier_names));
            }
        }
        Fetched::Failed(error) => {
            head = head.child(title(u, match_id));
            body = body.child(problem(u, error));
        }
        _ => {
            head = head.child(title(u, match_id));
            body = body.child(
                text(tr("Loading both teams"))
                    .font(BODY)
                    .color(u.muted),
            );
        }
    }

    footer = footer
        .child(
            u.icon_action(&icons::EXTERNAL_LINK, tr("Open on Statlocker"), ActionKind::Ghost)
                .key("board-statlocker")
                .role("link")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut rune::Tx| {
                    open_statlocker(st, tx, &statlocker_match_url(match_id))
                }),
        )
        .child(spacer())
        .child(
        u.action(tr("Close"), ActionKind::Ghost)
            .key("board-close")
            .cursor(rune::Cursor::Pointer)
            .on_press(close),
    );

    u.dialog_shell()
        .key("stats-board-dialog")
        .role("dialog")
        .label(tr("Match scoreboard"))
        .w(BOARD_W)
        .max_w(BOARD_W)
        .h_full()
        .padding(24.0)
        .gap(12.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(head)
        .child(
            // The teams take what the window leaves; header and footer never move.
            body.h(0.0)
                .grow(1.0)
                .shrink()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                }),
        )
        .child(footer)
}

/// The player's team first, then the other one.
fn team_order(board: &MatchScoreboard) -> [usize; 2] {
    match board
        .teams
        .iter()
        .position(|team| team.iter().any(|player| player.account_id == board.account_id))
    {
        Some(1) => [1, 0],
        _ => [0, 1],
    }
}

/// The column heads, once, pinned above the first team.
fn column_labels(u: &Ui) -> El {
    let label = |t: &str, w: f32, end: bool| {
        let e = text(tr(t).to_string()).font(SMALL).color(u.muted);
        let e = if end { e.align(rune::Align::End) } else { e };
        if w > 0.0 { e.w(w) } else { e.grow(1.0).min_w(0.0) }
    };
    row()
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(label("Player", PLAYER_W, false))
        .child(label("Hero", 0.0, false))
        .child(label("Rank", BADGE_W, false))
        .child(label("K / D / A", KDA_W, true))
        .child(label("Souls", 72.0, true))
        .child(label("Damage", 72.0, true))
        .child(label("Healing", 64.0, true))
}

fn title(u: &Ui, match_id: u64) -> El {
    text(crate::screens::settings::i18n::fill(
        tr("Match {}"),
        &[&match_id.to_string()],
    ))
    .font(H1)
    .color(u.ink)
}

fn header(u: &Ui, board: &MatchScoreboard) -> El {
    let mode = match board.match_mode {
        1 => tr("Unranked"),
        2 => tr("Private lobby"),
        3 => tr("Bots"),
        4 => tr("Ranked"),
        7 => tr("Hero Labs"),
        8 => tr("Placement"),
        _ => tr("Other"),
    };
    let meta = [
        mode.to_string(),
        crate::screens::settings::i18n::fill(
            tr("{} minutes"),
            &[&(board.duration_seconds / 60).to_string()],
        ),
        format!(
            "{} {}",
            insight::date_label(board.start_time),
            insight::clock_label(board.start_time)
        ),
    ];
    let mut head = col().w_full().gap(2.0).child(
        row()
            .w_full()
            .gap(12.0)
            .items_end()
            .child(title(u, board.match_id))
            .child(
                text(meta.join("  ·  "))
                    .font(TNUM_SMALL)
                    .color(u.muted)
                    .pb(3.0),
            )
            .child(spacer()),
    );
    // The team heads already say who won; only an unscored match needs a line here.
    if super::notes::not_scored(board) {
        head = head.child(text(tr("Not scored for you")).font(BODY_MED).color(u.muted));
    }
    // DMM's match note: who abandoned, and why a match was not scored.
    if let Some(note) = super::notes::board_note(u, board) {
        head = head.child(note);
    }
    head
}

/// Souls each side held: the player's team minus the other, above and below zero.
fn souls(u: &Ui, board: &MatchScoreboard) -> El {
    let ours = board
        .teams
        .iter()
        .position(|team| team.iter().any(|player| player.account_id == board.account_id))
        .unwrap_or(0);
    let lead = |point: &SoulPoint| -> f64 {
        if ours == 0 {
            point.lead() as f64
        } else {
            (point.team1 as i64 - point.team0 as i64) as f64
        }
    };
    let values: Vec<f64> = board.souls.iter().map(lead).collect();
    let final_lead = values.last().copied().unwrap_or(0.0);
    // The scale covers what happened: a match always ahead uses the whole height above zero.
    let hi = values.iter().fold(0.0_f64, |m, v| m.max(*v)).max(1000.0);
    let lo = values.iter().fold(0.0_f64, |m, v| m.min(*v)).min(-1000.0);
    let duration = board.duration_seconds.max(1) as f64;
    let pad = 8.0;
    let span = (hi - lo).max(1.0);
    let y = |value: f64| pad + ((hi - value) / span) as f32 * (CHART_H - 2.0 * pad);
    let mid = y(0.0);
    let x = |time_s: u32| (time_s as f64 / duration).min(1.0) as f32 * CHART_VB;

    let mut shape = Shape::new().view_box(CHART_VB, CHART_H);
    shape = shape.line(
        [0.0, mid],
        [CHART_VB, mid],
        1.0,
        rune::LineCap::Butt,
        Color::WHITE.alpha(0.14),
    );
    let line: Vec<[f32; 2]> = board
        .souls
        .iter()
        .zip(values.iter())
        .map(|(point, value)| [x(point.time_s), y(*value)])
        .collect();
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let above_a = a[1] <= mid;
        let above_b = b[1] <= mid;
        let tint = |above: bool| {
            if above {
                u.teal.alpha(0.14)
            } else {
                u.rose.alpha(0.14)
            }
        };
        if above_a == above_b {
            shape = shape.fill(vec![a, b, [b[0], mid], [a[0], mid]], tint(above_a));
        } else {
            let t = (mid - a[1]) / (b[1] - a[1]);
            let cross = [a[0] + t * (b[0] - a[0]), mid];
            shape = shape
                .fill(vec![a, cross, [a[0], mid]], tint(above_a))
                .fill(vec![cross, b, [b[0], mid]], tint(above_b));
        }
    }
    shape = shape.stroke(line, 1.5, u.prose);

    let caption = crate::screens::settings::i18n::fill(
        tr("Souls ahead of the other team, sampled every three minutes. Ended {}."),
        &[&signed(final_lead.round() as i64)],
    );
    col()
        .w_full()
        .gap(6.0)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(text(tr("Soul lead")).font(SMALL).color(u.muted))
                .child(
                    text(crate::screens::settings::i18n::fill(
                        tr("ended {}"),
                        &[&signed(final_lead.round() as i64)],
                    ))
                    .font(TNUM_SMALL)
                    .color(if final_lead >= 0.0 { u.teal } else { u.rose }),
                ),
        )
        .child(
            canvas(shape)
                .w_full()
                .h(CHART_H)
                .role("img")
                .label(tr("Soul lead through the match"))
                .description(caption),
        )
}

fn team_block(
    s: &State,
    u: &Ui,
    board: &MatchScoreboard,
    index: usize,
    tier_names: Option<&std::collections::BTreeMap<u32, String>>,
) -> El {
    let ours = board
        .teams
        .iter()
        .position(|team| team.iter().any(|player| player.account_id == board.account_id));
    let team = board.team(index);
    if team.is_empty() {
        return col();
    }
    let label = match ours {
        Some(mine) if mine == index => tr("Your team"),
        Some(_) => tr("The other team"),
        None if index == 0 => tr("Team 0"),
        None => tr("Team 1"),
    };
    let souls: u64 = team.iter().map(|player| player.net_worth).sum();
    let wins = board.winning_team == Some(index as i32);

    let mut block = col().w_full().gap(0.0).child(
        row()
            .w_full()
            .h(32.0)
            .px(8.0)
            .gap(12.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(
                text(label.to_string())
                    .font(BODY_MED)
                    .color(u.ink),
            )
            .child(
                text(if wins { tr("Won") } else { tr("Lost") })
                    .font(SMALL)
                    .color(if wins { u.teal } else { u.rose }),
            )
            .child(spacer())
            .child(
                text(crate::screens::settings::i18n::fill(
                    tr("{} souls"),
                    &[&thousands(souls)],
                ))
                .font(TNUM_SMALL)
                .color(u.muted),
            ),
    );
    for player in team {
        block = block.child(player_row(s, u, player, board.account_id, tier_names));
    }
    block
}

fn player_row(
    s: &State,
    u: &Ui,
    player: &BoardPlayer,
    me: u64,
    tier_names: Option<&std::collections::BTreeMap<u32, String>>,
) -> El {
    let name = player
        .persona_name
        .clone()
        .unwrap_or_else(|| player.account_id.to_string());
    let hero = player
        .hero_name
        .clone()
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(
            tr("Hero {}"),
            &[&player.hero_id.to_string()],
        ));
    let mine = player.account_id == me;
    // Items the catalog does not know would print as raw ids; count them instead.
    let known: Vec<String> = player
        .build
        .iter()
        .filter(|item| item.slot != "other")
        .map(|item| item.name.clone())
        .collect();
    let unknown = player.build.len() - known.len();
    let build = if player.build.is_empty() {
        tr("No items recorded").to_string()
    } else if unknown == 0 {
        known.join(", ")
    } else if known.is_empty() {
        crate::screens::settings::i18n::fill(tr("{} items not in the catalog"), &[&unknown.to_string()])
    } else {
        format!("{}  +{unknown}", known.join(", "))
    };
    let badge = match (player.badge, tier_names) {
        (Some(badge), Some(names)) => Some(insight::badge_label(badge, names)),
        (Some(badge), None) => Some(badge.to_string()),
        (None, _) => None,
    };

    // Items: known names on one line, the rest counted at the end.
    let line = row()
        .w_full()
        .h(40.0)
        .gap(12.0)
        .center_y()
        .child(
            text(name)
                .font(if mine { BODY_MED } else { BODY })
                .color(u.ink)
                .truncate()
                .w(PLAYER_W),
        )
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(hero_chip(s, u, player.hero_id, &hero, 20.0))
                .child(text(hero).font(BODY).color(u.prose).truncate()),
        )
        .child(
            row()
                .w(BADGE_W)
                .gap(6.0)
                .center_y()
                .child(text(badge.unwrap_or_default()).font(TNUM_SMALL).color(u.muted).truncate().min_w(0.0).shrink())
                .children((player.mvp_rank == Some(1)).then(|| {
                    row()
                        .h(20.0)
                        .px(6.0)
                        .center()
                        .radius(2.0)
                        .fill(u.amber.alpha(0.14))
                        .child(text(tr("MVP")).font(crate::ui::MICRO).color(u.amber))
                })),
        )
        .child(super::brief::kda_text(u, player.kills, player.deaths, player.assists).w(KDA_W))
        .child(num(thousands(player.net_worth), 72.0, u.prose))
        .child(num(thousands(player.player_damage), 72.0, u.muted))
        .child(num(thousands(player.player_healing), 64.0, u.muted));

    col()
        .key(format!("board-player-{}", player.account_id))
        .w_full()
        .h(60.0)
        .px(8.0)
        .fill(if mine { u.selected } else { Color::TRANSPARENT })
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(line)
        .child(
            text(build)
                .font(SMALL)
                .color(u.muted)
                .truncate()
                .pl(PLAYER_W + 12.0),
        )
}
