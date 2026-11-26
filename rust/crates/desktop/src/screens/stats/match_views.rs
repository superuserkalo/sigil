//! The match page's Build and Lobby views (`board.rs` holds the Scoreboard).
//!
//! Both read the one metadata response the scoreboard already parsed: no extra request, except
//! the Squad lists the Lobby view uses for "met before" (the same three reads as the Squad tab,
//! kept an hour).

use deadlock_core::stats::board::{BoardPlayer, MatchScoreboard, Purchase};
use rune::{Color, El, Shape, canvas, col, el, row, spacer, text};

use crate::app::State;
use crate::ui::{BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

/// The three views of a match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchView {
    #[default]
    Scoreboard,
    Build,
    Lobby,
}

/// The view picker under the match page's top bar.
pub(super) fn picker(s: &State, u: &Ui) -> El {
    let view = s.stats.match_view;
    let part = |key: &str, label: &str, value: MatchView| {
        crate::ui::SegPart::new(key, tr(label)).on(view == value).with(move |e| e.on_press(move |st: &mut State| set_view(st, value)))
    };
    crate::ui::Segmented::new("stats-match-view", tr("Match view"))
        .part(part("stats-match-view-board", "Scoreboard", MatchView::Scoreboard))
        .part(part("stats-match-view-build", "Build", MatchView::Build))
        .part(part("stats-match-view-lobby", "Lobby", MatchView::Lobby))
        .build(u)
}

/// Switch the match view. The Lobby view reads the Squad lists for "met before".
pub(super) fn set_view(st: &mut State, view: MatchView) {
    st.stats.match_view = view;
    if view == MatchView::Lobby && matches!(st.stats.social, Fetched::Idle) && !super::nav::offline_build() {
        crate::data::start_social_load(st);
    }
}

const CURVE_H: f32 = 120.0;
const CURVE_VB: f32 = 1000.0;
const ICON: f32 = 24.0;

/// The player the Build view shows: the picked one, else the page player, else the first.
fn build_player(s: &State, board: &MatchScoreboard) -> Option<u64> {
    let all: Vec<u64> = board.teams.iter().flatten().map(|p| p.account_id).collect();
    s.stats
        .build_player
        .filter(|id| all.contains(id))
        .or_else(|| board.me().map(|p| p.account_id))
        .or_else(|| all.first().copied())
}

/// Build: pick a player, then their souls over the match with what they bought when, where
/// their souls came from, and every purchase in order.
pub(super) fn build(s: &State, u: &Ui, board: &MatchScoreboard) -> El {
    let picked = build_player(s, board);
    let mut c = col().key("stats-match-build").w_full().gap(16.0).child(players(s, u, board, picked));
    let Some(player) = board.teams.iter().flatten().find(|p| Some(p.account_id) == picked) else {
        return c;
    };
    let hero = player.hero_name.clone().unwrap_or_else(|| format!("Hero {}", player.hero_id));
    c = c.child(
        row()
            .w_full()
            .h(32.0)
            .gap(12.0)
            .center_y()
            .child(hero_chip(s, u, player.hero_id, &hero, 24.0))
            .child(text(crate::screens::settings::i18n::fill(tr("{} on {}"), &[&player.persona_name.clone().unwrap_or_else(|| player.account_id.to_string()), &hero])).font(crate::ui::H2).color(u.ink))
            .child(text(crate::screens::settings::i18n::fill(tr("{} souls, {} a minute"), &[&thousands(player.net_worth), &thousands(player.souls_per_minute(board.duration_seconds))])).font(TNUM_SMALL).color(u.muted)),
    );
    c = c.child(timeline(s, u, board, player));
    c = c.child(sources(u, player));
    c.child(purchase_list(s, u, player))
}

/// Twelve buttons, one per player, the page player's team first.
fn players(s: &State, u: &Ui, board: &MatchScoreboard, picked: Option<u64>) -> El {
    let order = match board.my_team() {
        Some(1) => [1, 0],
        _ => [0, 1],
    };
    let mut c = col().w_full().gap(6.0);
    for team in order {
        let mut line = row().w_full().gap(6.0);
        for player in board.team(team) {
            let id = player.account_id;
            let on = Some(id) == picked;
            let name = player.persona_name.clone().unwrap_or_else(|| id.to_string());
            let hero = player.hero_name.clone().unwrap_or_default();
            line = line.child(
                row()
                    .key(format!("stats-build-pick-{id}"))
                    .w(0.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .h(36.0)
                    .px(8.0)
                    .gap(8.0)
                    .center_y()
                    .radius(crate::ui::r(2.0))
                    .fill(if on { u.selected } else { Color::TRANSPARENT })
                    .border(1.0, if on { u.hair_strong } else { u.hair })
                    .hover(u.hover)
                    .focusable(true)
                    .role("button")
                    .checked(on)
                    .label(crate::screens::settings::i18n::fill(tr("Show the build of {}"), &[&name]))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.stats.build_player = Some(id))
                    .child(hero_chip(s, u, player.hero_id, &hero, 20.0))
                    .child(text(name).font(if on { BODY_MED } else { BODY }).color(if on { u.ink } else { u.prose }).truncate().min_w(0.0).shrink()),
            );
        }
        c = c.child(line);
    }
    c
}

/// The player's souls over the match, and below it what they bought in each stretch of the
/// match: the columns share the curve's time axis, so a purchase sits under its moment.
fn timeline(s: &State, u: &Ui, board: &MatchScoreboard, player: &BoardPlayer) -> El {
    let duration = board.duration_seconds.max(1);
    let hi = player.net_worth_at.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f32;
    // Whole five minute stretches (wider for a long match, at most twelve) sharing the width
    // equally; the curve's time axis runs to the end of the last stretch so they line up.
    let span = (duration.div_ceil(12).div_ceil(300) * 300).max(300);
    let stretches = duration.div_ceil(span).max(1);
    let end = stretches * span;
    let x = |t: u32| (t.min(end) as f32 / end as f32) * CURVE_VB;
    let y = |n: u64| 6.0 + (1.0 - n as f32 / hi) * (CURVE_H - 12.0);
    let mut points = vec![[0.0, y(0)]];
    points.extend(player.net_worth_at.iter().map(|(t, n)| [x(*t), y(*n)]));
    let mut shape = Shape::new().view_box(CURVE_VB, CURVE_H);
    let mut area = points.clone();
    area.push([points.last().map(|p| p[0]).unwrap_or(0.0), CURVE_H]);
    area.push([0.0, CURVE_H]);
    shape = shape.fill(area, u.teal.alpha(0.10)).stroke(points, 1.5, u.prose);
    for purchase in &player.purchases {
        shape = shape.line([x(purchase.bought_at), CURVE_H - 6.0], [x(purchase.bought_at), CURVE_H], 1.0, rune::LineCap::Butt, slot_color(u, &purchase.slot));
    }

    let mut grid = el().w_full().equal_columns(stretches as usize).gap(4.0);
    for i in 0..stretches {
        let (from, to) = (i * span, (i + 1) * span);
        let mut cell = col()
            .grid_cell(i as usize)
            .min_w(0.0)
            .gap(4.0)
            .pt(6.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
            .child(text(super::notes::clock(from)).font(TNUM_SMALL).color(u.muted));
        let mut tiles = row().w_full().gap(2.0).flex_wrap();
        for purchase in player.purchases.iter().filter(|p| p.bought_at >= from && (p.bought_at < to || (to == end && p.bought_at == to))) {
            tiles = tiles.child(purchase_tile(s, u, purchase));
        }
        cell = cell.child(tiles);
        grid = grid.child(cell);
    }
    col()
        .w_full()
        .gap(4.0)
        .child(
            row()
                .w_full()
                .child(text(tr("Souls through the match")).font(SMALL).color(u.muted))
                .child(spacer())
                .child(text(tr("Ticks: purchases, in their slot colour")).font(SMALL).color(u.muted)),
        )
        .child(canvas(shape).w_full().h(CURVE_H).role("img").label(tr("Souls through the match")))
        .child(grid)
}

fn purchase_tile(s: &State, u: &Ui, p: &Purchase) -> El {
    let tile = item_tile(s, u, p.item_id, &p.name, &p.slot, ICON, Some(p.bought_at));
    if p.sold_at.is_some() { tile.opacity(0.45) } else { tile }
}

/// Where the souls came from: one stacked bar, then each part with its number.
fn sources(u: &Ui, player: &BoardPlayer) -> El {
    let total = player.sources.total();
    let mut c = col().key("stats-build-sources").w_full().gap(8.0).child(text(tr("Where the souls came from")).font(SMALL).color(u.muted));
    if total == 0 {
        return c.child(text(tr("The match data has no soul sources for this player.")).font(BODY).color(u.muted));
    }
    let colors = [u.teal, u.rose, u.violet, u.amber, u.muted];
    let mut shape = Shape::new().view_box(1000.0, 12.0);
    let mut at = 0.0_f32;
    let mut legend = row().w_full().gap(20.0).flex_wrap();
    for ((label, souls), color) in player.sources.parts().into_iter().zip(colors) {
        let w = souls as f32 / total as f32 * 1000.0;
        if w > 0.0 {
            shape = shape.fill(rect(at, 0.0, at + w, 12.0), color.alpha(0.8));
        }
        at += w;
        legend = legend.child(
            row()
                .gap(6.0)
                .center_y()
                .child(el().size(8.0, 8.0).radius(1.0).fill(color.alpha(0.8)).a11y_hidden())
                .child(text(tr(label).to_string()).font(SMALL).color(u.muted))
                .child(text(format!("{} ({:.0}%)", thousands(souls), souls as f64 * 100.0 / total as f64)).font(TNUM_SMALL).color(u.prose)),
        );
    }
    c = c.child(canvas(shape).w_full().h(12.0).role("img").label(tr("Where the souls came from"))).child(legend);
    c
}

/// Every purchase in order: when, the item, and when it was sold.
fn purchase_list(s: &State, u: &Ui, player: &BoardPlayer) -> El {
    let mut c = col().key("stats-build-list").w_full().child(head_row(u, &[("Bought at", 72.0, false), ("Item", 0.0, false), ("Sold at", 72.0, true)]));
    if player.purchases.is_empty() {
        return c.child(text(tr("No purchases recorded")).font(BODY).color(u.muted).pt(8.0));
    }
    for (i, p) in player.purchases.iter().enumerate() {
        c = c.child(
            data_row(format!("stats-build-item-{i}"), u, 36.0)
                .child(cell(text(super::notes::clock(p.bought_at)).font(TNUM).color(u.muted), 72.0))
                .child(cell(
                    row().gap(10.0).center_y().child(purchase_tile(s, u, p)).child(text(p.name.clone()).font(BODY_MED).color(if p.sold_at.is_some() { u.muted } else { u.ink }).truncate().min_w(0.0)),
                    0.0,
                ))
                .child(num(p.sold_at.map(super::notes::clock).unwrap_or_default(), 72.0, u.muted)),
        );
    }
    c
}

/// Lobby: each lane with both teams' players in it and their souls at nine minutes, the
/// teams' average ranks, bans, and who the page player met before.
pub(super) fn lobby(s: &State, u: &Ui, board: &MatchScoreboard) -> El {
    use crate::screens::settings::i18n::fill;
    let ours = board.my_team().unwrap_or(0);
    let theirs = 1 - ours;
    let tier_names = s.stats.snapshot.as_ref().map(|snap| &snap.tier_names);
    let badge_line = |team: usize| -> El {
        let label = if team == ours { tr("Your team") } else { tr("Enemy team") };
        let mut r = row().gap(8.0).center_y().child(text(label.to_string()).font(BODY_MED).color(u.ink));
        if let Some(badge) = board.average_badge.get(team).copied().flatten() {
            let name = tier_names.map(|n| deadlock_core::stats::insight::badge_label(badge, n)).unwrap_or_else(|| badge.to_string());
            r = r.child(text(tr("average")).font(SMALL).color(u.muted)).child(rank_badge(s, u, badge, 20.0)).child(text(name).font(SMALL).color(u.prose));
        }
        r
    };
    let mut c = col()
        .key("stats-match-lobby")
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .h(32.0)
                .gap(24.0)
                .center_y()
                .child(badge_line(ours).w(0.0).grow(1.0).min_w(0.0))
                .child(badge_line(theirs).w(0.0).grow(1.0).min_w(0.0)),
        );
    if !board.banned_hero_ids.is_empty() {
        let mut bans = row().gap(8.0).center_y().child(text(tr("Banned")).font(SMALL).color(u.muted));
        for hero_id in &board.banned_hero_ids {
            bans = bans.child(hero_chip(s, u, *hero_id, "?", 24.0));
        }
        c = c.child(bans);
    }
    let my_lane = board.me().and_then(|me| me.lane);
    let lanes = board.lanes();
    if lanes.is_empty() {
        return c.child(text(tr("This match's data names no lanes.")).font(BODY).color(u.muted));
    }
    const AT: u32 = 540;
    for (index, (lane, sides)) in lanes.iter().enumerate() {
        let souls = |team: usize| sides[team].iter().map(|p| p.net_worth_by(AT)).sum::<u64>();
        let gap = souls(ours) as i64 - souls(theirs) as i64;
        let mut head = row()
            .w_full()
            .h(32.0)
            .gap(12.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(text(fill(tr("Lane {}"), &[&(index + 1).to_string()])).font(BODY_MED).color(u.ink));
        if my_lane == Some(*lane) {
            head = head.child(text(tr("your lane")).font(SMALL).color(u.accent));
        }
        head = head.child(spacer()).child(
            text(fill(tr("{} souls at 9:00"), &[&signed(gap)]))
                .font(TNUM_SMALL)
                .color(if gap > 0 { u.teal } else if gap < 0 { u.rose } else { u.muted }),
        );
        let side = |team: usize| {
            let mut col_ = col().w(0.0).grow(1.0).min_w(0.0);
            for p in &sides[team] {
                col_ = col_.child(lobby_row(s, u, board, p));
            }
            col_
        };
        c = c.child(col().w_full().child(head).child(row().w_full().gap(24.0).items_start().child(side(ours)).child(side(theirs))));
    }
    if matches!(s.stats.social, Fetched::Loading) {
        c = c.child(text(tr("Loading who you met before")).font(SMALL).color(u.muted));
    }
    c
}

/// One player in a lane: hero, name (a link), souls at nine minutes, and met before.
fn lobby_row(s: &State, u: &Ui, board: &MatchScoreboard, p: &BoardPlayer) -> El {
    let name = p.persona_name.clone().unwrap_or_else(|| p.account_id.to_string());
    let hero = p.hero_name.clone().unwrap_or_default();
    let mine = p.account_id == board.account_id;
    let mut r = row()
        .key(format!("stats-lobby-{}", p.account_id))
        .w_full()
        .h(40.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .fill(if mine { u.selected } else { Color::TRANSPARENT })
        .child(hero_chip(s, u, p.hero_id, &hero, 24.0))
        .child(row().grow(1.0).min_w(0.0).center_y().child(player_link(s, u, p.account_id, &name, 20.0, mine)));
    if let Some(met) = met_before(s, p.account_id).filter(|_| !mine) {
        r = r.child(text(met).font(SMALL).color(u.muted));
    }
    r.child(num(souls(p.net_worth_by(540)), 56.0, u.prose))
}

/// "12 together", "5 against": from the page player's Squad lists, when they are loaded.
fn met_before(s: &State, account_id: u64) -> Option<String> {
    let Fetched::Ready(social) = &s.stats.social else { return None };
    let with = social.mates.iter().find(|r| r.account_id == account_id).map(|r| r.matches).unwrap_or(0);
    let against = social.enemies.iter().find(|r| r.account_id == account_id).map(|r| r.matches).unwrap_or(0);
    let mut parts = Vec::new();
    if with > 0 {
        parts.push(crate::screens::settings::i18n::fill(tr("{} together"), &[&with.to_string()]));
    }
    if against > 0 {
        parts.push(crate::screens::settings::i18n::fill(tr("{} against"), &[&against.to_string()]));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}
