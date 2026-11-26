//! The match page: both teams, every final build, and the souls each side held.
//!
//! A place in the Stats stack (`nav.rs`), opened from any match row and seen as the player
//! whose page opened it: their row is tinted and "your team" is theirs. The data is one
//! metadata response, read with `disable_steam=true`; a match deadlock-api has not analysed
//! answers 404, and the page offers one "Fetch from Steam" press, which deadlock-api allows
//! three times an hour per network.

use std::time::{Duration, Instant};

use deadlock_core::stats::board::{BoardPlayer, Column, MatchScoreboard, SoulPoint};
use deadlock_core::stats::insight;
use rune::{Color, El, Shape, Tx, canvas, col, el, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

const CHART_H: f32 = 96.0;
const CHART_VB: f32 = 1000.0;
const PLAYER_W: f32 = 150.0;
const RANK_W: f32 = 28.0;
const KDA_W: f32 = 84.0;
const SOULS_W: f32 = 56.0;
const SPM_W: f32 = 44.0;
const DMG_W: f32 = 56.0;
const HEAL_W: f32 = 52.0;
const LH_W: f32 = 56.0;
/// Twelve 20 point icons, 2 apart.
const ICON: f32 = 20.0;
const BUILD_W: f32 = 12.0 * ICON + 11.0 * 2.0;
const GAP: f32 = 10.0;
const ROW_H: f32 = 40.0;
/// After a "Fetch from Steam" press the button waits this long (3 an hour per IP).
const STEAM_WAIT: Duration = Duration::from_secs(20 * 60);

/// Read the board for `match_id`, from deadlock-api's store or, on a press, from Steam.
pub(super) fn start(st: &mut State, match_id: u64, from_steam: bool) {
    if let Fetched::Ready(board) = &st.stats.board
        && board.match_id == match_id
        && !from_steam
    {
        return;
    }
    if !st.prefs.allow_stats {
        st.stats.board = Fetched::Failed(tr("Player stats are turned off in Settings, Privacy.").to_string());
        return;
    }
    let Some((_, account_id)) = st.stats.nav.match_open() else { return };
    st.stats.build_player = None;
    st.stats.board = Fetched::Loading;
    if from_steam {
        st.stats.board_steam_at = Some(Instant::now());
    }
    if st.stats.board_job.is_running() || super::nav::offline_build() {
        return;
    }
    let wake = st.wake.clone();
    st.stats.board_job.start(wake, move || {
        crate::data::stats_client()
            .fetch_match_scoreboard_from(match_id, account_id, from_steam)
            .map_err(|error| error.to_string())
    });
}

/// Take a finished board; a board for a match no longer open is dropped and the open one read.
pub(super) fn poll(st: &mut State) -> bool {
    let Some(result) = st.stats.board_job.try_take() else { return false };
    let open = st.stats.nav.match_open().map(|(id, _)| id);
    match result {
        Ok(board) if Some(board.match_id) == open => st.stats.board = Fetched::Ready(board),
        Ok(_) => {
            if let Some(id) = open {
                st.stats.board = Fetched::Idle;
                start(st, id, false);
            }
        }
        Err(error) => st.stats.board = Fetched::Failed(error),
    }
    true
}

/// The subject's filtered match list, newest first: what previous and next walk.
fn walk(s: &State) -> Vec<u64> {
    let Some(snap) = &s.stats.snapshot else { return Vec::new() };
    super::matches::kept(s, snap).into_iter().map(|item| item.match_id).collect()
}

pub(super) fn page(s: &State, u: &Ui) -> El {
    let Some((match_id, _)) = s.stats.nav.match_open() else { return col() };
    let mut page = col().key("stats-match").w_full().gap(12.0).transition(crate::motion::page_enter());
    page = page.child(top_bar(s, u, match_id)).child(super::match_views::picker(s, u));
    page = match &s.stats.board {
        Fetched::Ready(board) if board.match_id == match_id => page.child(match s.stats.match_view {
            super::match_views::MatchView::Scoreboard => body(s, u, board),
            super::match_views::MatchView::Build => super::match_views::build(s, u, board),
            super::match_views::MatchView::Lobby => super::match_views::lobby(s, u, board),
        }),
        Fetched::Failed(error) => page.child(failed(s, u, match_id, error)),
        _ => page.child(skeleton(u)),
    };
    page
}

/// Back, the match, its facts, previous and next, copy and Statlocker.
fn top_bar(s: &State, u: &Ui, match_id: u64) -> El {
    let ids = walk(s);
    let at = ids.iter().position(|id| *id == match_id);
    let newer = at.and_then(|i| i.checked_sub(1)).and_then(|i| ids.get(i)).copied();
    let older = at.and_then(|i| ids.get(i + 1)).copied();
    let step = |key: &'static str, glyph: &'static rune::Icon, label: &str, to: Option<u64>| {
        let b = u.icon_button(glyph, label).key(key);
        match to {
            Some(id) => b
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| super::nav::open_match(st, tx, id)),
            None => b.opacity(0.35).disabled(true),
        }
    };
    let mut meta = Vec::new();
    if let Fetched::Ready(board) = &s.stats.board
        && board.match_id == match_id
    {
        meta.push(mode_label(board.match_mode).to_string());
        meta.push(format!("{}m", board.duration_seconds / 60));
        meta.push(format!("{} {}", insight::date_label(board.start_time), insight::clock_label(board.start_time)));
    }
    row()
        .w_full()
        .h(36.0)
        .gap(12.0)
        .center_y()
        .child(back_button(u))
        .child(text(crate::screens::settings::i18n::fill(tr("Match {}"), &[&match_id.to_string()])).font(H1).color(u.ink))
        .child(text(meta.join("  ·  ")).font(TNUM_SMALL).color(u.muted).truncate().min_w(0.0).shrink())
        .child(spacer())
        .children(review_button(s, u, match_id))
        .child(step("stats-match-newer", &icons::CHEVRON_LEFT, tr("Newer match"), newer))
        .child(step("stats-match-older", &icons::CHEVRON_RIGHT, tr("Older match"), older))
        .child(
            u.icon_button(&icons::COPY, tr("Copy match id"))
                .key("stats-match-copy")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(match_id.to_string())),
        )
        .child(statlocker_link(u, "stats-match-statlocker".to_string(), statlocker_match_url(match_id), tr("Open on Statlocker")))
}

/// "Review this match" when a demo of it is imported.
fn review_button(s: &State, u: &Ui, match_id: u64) -> Option<El> {
    let key = match_id.to_string();
    s.stats.demos.iter().any(|demo| demo.match_id == key).then(|| {
        u.action(tr("Review this match"), ActionKind::Secondary)
            .key(format!("stats-review-{key}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| crate::data::open_review(st, &key))
    })
}

fn mode_label(mode: i32) -> &'static str {
    match mode {
        1 => tr("Unranked"),
        2 => tr("Private lobby"),
        3 => tr("Bots"),
        4 => tr("Ranked"),
        7 => tr("Hero Labs"),
        8 => tr("Placement"),
        _ => tr("Other"),
    }
}

/// Not analysed yet (404), or another failure with Retry.
fn failed(s: &State, u: &Ui, match_id: u64, error: &str) -> El {
    if !deadlock_core::stats::is_not_found(error) {
        return problem_retry(u, error, "stats-match-retry", move |st: &mut State| {
            st.stats.board = Fetched::Idle;
            start(st, match_id, false);
        });
    }
    let wait = s.stats.board_steam_at.map(|at| STEAM_WAIT.saturating_sub(at.elapsed())).filter(|left| !left.is_zero());
    let mut button = u.action(tr("Fetch from Steam"), ActionKind::Secondary).key("stats-match-steam");
    button = match wait {
        Some(_) => button.opacity(0.5).disabled(true),
        None => button.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| start(st, match_id, true)),
    };
    let help = match wait {
        Some(left) => crate::screens::settings::i18n::fill(
            tr("deadlock-api fetches from Steam three times an hour per network. Try again in {} min."),
            &[&(left.as_secs() / 60 + 1).to_string()],
        ),
        None => tr("deadlock-api fetches from Steam three times an hour per network.").to_string(),
    };
    col()
        .w_full()
        .gap(8.0)
        .pt(16.0)
        .child(text(tr("This match is not analysed yet")).font(crate::ui::H2).color(u.ink))
        .child(text(tr("deadlock-api has not stored it. It can ask Steam for it now.")).font(BODY).color(u.prose))
        .child(row().gap(12.0).center_y().child(button).child(text(help).font(SMALL).color(u.muted)))
}

/// Two teams of six empty rows while the board loads. Nothing moves.
fn skeleton(u: &Ui) -> El {
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(crate::ui::r(2.0)).fill(u.hover);
    let mut c = col().key("stats-match-skeleton").w_full().gap(0.0).child(el().w_full().h(CHART_H + 28.0).radius(crate::ui::r(4.0)).fill(u.hover.alpha(0.5)));
    for team in 0..2 {
        c = c.child(row().w_full().h(32.0).mt(if team == 0 { 16.0 } else { 12.0 }).center_y().child(bar(160.0, 14.0)));
        for i in 0..6 {
            c = c.child(
                row()
                    .w_full()
                    .h(ROW_H)
                    .gap(GAP)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(bar(24.0, 24.0))
                    .child(bar(100.0 + (i % 3) as f32 * 24.0, 12.0)),
            );
        }
    }
    c
}

fn body(s: &State, u: &Ui, board: &MatchScoreboard) -> El {
    let brawl = board.game_mode == deadlock_core::stats::filters::GAME_MODE_STREET_BRAWL;
    let mut c = col().w_full().gap(12.0);
    if let Some(line) = subject_line(s, u, board) {
        c = c.child(line);
    }
    if super::notes::not_scored(board) {
        c = c.child(text(tr("Not scored for you")).font(BODY_MED).color(u.muted));
    }
    if let Some(note) = super::notes::board_note(u, board) {
        c = c.child(note);
    }
    if brawl {
        c = c.child(text(tr("Street Brawl: rounds, not a souls race, so no soul lead is drawn.")).font(SMALL).color(u.muted));
    } else if board.souls.len() >= 2 {
        c = c.child(souls(u, board));
    }
    let mut teams = col().w_full().gap(0.0).child(column_labels(u));
    let order = match board.my_team() {
        Some(1) => [1, 0],
        _ => [0, 1],
    };
    for team in order {
        teams = teams.child(team_block(s, u, board, team));
    }
    // Purchases live in the Build view, one press away; the table ends the page.
    c.child(teams).child(
        row()
            .w_full()
            .h(36.0)
            .gap(8.0)
            .center_y()
            .child(text(tr("Every purchase and where the souls came from:")).font(SMALL).color(u.muted))
            .child(
                u.action(tr("Open the Build view"), ActionKind::Ghost)
                    .key("stats-match-to-build")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| super::match_views::set_view(st, super::match_views::MatchView::Build)),
            ),
    )
}

/// "Win as Drifter  7 / 6 / 24  1,203 a minute  +480 rank points", with the hero average.
fn subject_line(s: &State, u: &Ui, board: &MatchScoreboard) -> Option<El> {
    let me = board.me()?;
    let snap = s.stats.snapshot.as_ref();
    let row_of = snap.and_then(|snap| snap.matches.iter().find(|item| item.match_id == board.match_id));
    let (result, color) = match (board.winning_team, me.outcome.and_then(|o| o.win())) {
        (_, Some(true)) => (tr("Win"), u.teal),
        (_, Some(false)) => (tr("Loss"), u.rose),
        (Some(team), None) if team == me.team => (tr("Win"), u.teal),
        (Some(_), None) => (tr("Loss"), u.rose),
        _ => (tr("Not scored"), u.muted),
    };
    let hero = me.hero_name.clone().unwrap_or_else(|| format!("Hero {}", me.hero_id));
    let mut r = row()
        .key("stats-match-subject")
        .w_full()
        .h(40.0)
        .px(12.0)
        .gap(16.0)
        .center_y()
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair)
        .child(el().w(3.0).h(24.0).fill(color).a11y_hidden())
        .child(hero_chip(s, u, me.hero_id, &hero, 24.0))
        .child(text(crate::screens::settings::i18n::fill(tr("{} as {}"), &[result, &hero])).font(BODY_MED).color(u.ink))
        .child(super::brief::kda_text(u, me.kills, me.deaths, me.assists))
        .child(text(crate::screens::settings::i18n::fill(tr("{} a minute"), &[&thousands(me.souls_per_minute(board.duration_seconds))])).font(TNUM_SMALL).color(u.prose))
        .child(text(format!("LH {}  DN {}", me.last_hits, me.denies)).font(TNUM_SMALL).color(u.muted));
    if let Some(delta) = row_of.and_then(|item| item.ranked_delta).filter(|d| *d != 0) {
        r = r.child(
            text(crate::screens::settings::i18n::fill(tr("{} rank points"), &[&signed(delta as i64)]))
                .font(TNUM_SMALL)
                .color(if delta > 0 { u.teal } else { u.rose }),
        );
    }
    let average = snap.and_then(|snap| snap.heroes.iter().find(|h| h.hero_id == me.hero_id)).filter(|h| h.matches_played >= 5);
    if let Some(hero) = average {
        let avg_deaths = hero.deaths as f64 / hero.matches_played.max(1) as f64;
        r = r
            .child(spacer())
            .child(text(crate::screens::settings::i18n::fill(tr("Against the {} average"), &[&hero_name(hero)])).font(SMALL).color(u.muted))
            .child(compare_chip(u, &format!("KDA {:.2}", me.kda()), me.kda() >= hero.kda))
            .child(compare_chip(u, &format!("{} {}", tr("Deaths"), me.deaths), (me.deaths as f64) <= avg_deaths));
    }
    Some(r)
}

/// The column heads, once, above both teams.
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
        .gap(GAP)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(label("Player", PLAYER_W, false))
        .child(label("Hero", 0.0, false))
        .child(label("Rank", RANK_W, false))
        .child(label("K / D / A", KDA_W, true))
        .child(label("Souls", SOULS_W, true))
        .child(label("SPM", SPM_W, true))
        .child(label("Damage", DMG_W, true))
        .child(label("Healing", HEAL_W, true))
        .child(label("LH / DN", LH_W, true))
        .child(label("Build", BUILD_W, false))
}

/// Souls each side held: the page player's team minus the other, with a tick where an
/// objective fell (above the line for one the other team lost, below for one yours lost).
fn souls(u: &Ui, board: &MatchScoreboard) -> El {
    let ours = board.my_team().unwrap_or(0);
    let lead = |point: &SoulPoint| -> f64 {
        if ours == 0 { point.lead() as f64 } else { (point.team1 as i64 - point.team0 as i64) as f64 }
    };
    let values: Vec<f64> = board.souls.iter().map(lead).collect();
    let final_lead = values.last().copied().unwrap_or(0.0);
    let hi = values.iter().fold(0.0_f64, |m, v| m.max(*v)).max(1000.0);
    let lo = values.iter().fold(0.0_f64, |m, v| m.min(*v)).min(-1000.0);
    let duration = board.duration_seconds.max(1) as f64;
    let pad = 8.0;
    let span = (hi - lo).max(1.0);
    let y = |value: f64| pad + ((hi - value) / span) as f32 * (CHART_H - 2.0 * pad);
    let mid = y(0.0);
    let x = |time_s: u32| (time_s as f64 / duration).min(1.0) as f32 * CHART_VB;

    let mut shape = Shape::new().view_box(CHART_VB, CHART_H);
    shape = shape.line([0.0, mid], [CHART_VB, mid], 1.0, rune::LineCap::Butt, Color::WHITE.alpha(0.14));
    for (time, team) in &board.objectives_lost {
        let theirs = *team as usize != ours;
        let (y0, y1) = if theirs { (mid - 7.0, mid - 1.0) } else { (mid + 1.0, mid + 7.0) };
        shape = shape.line([x(*time), y0], [x(*time), y1], 1.5, rune::LineCap::Butt, if theirs { u.teal } else { u.rose });
    }
    let line: Vec<[f32; 2]> = board.souls.iter().zip(values.iter()).map(|(point, value)| [x(point.time_s), y(*value)]).collect();
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let above_a = a[1] <= mid;
        let above_b = b[1] <= mid;
        let tint = |above: bool| if above { u.teal.alpha(0.14) } else { u.rose.alpha(0.14) };
        if above_a == above_b {
            shape = shape.fill(vec![a, b, [b[0], mid], [a[0], mid]], tint(above_a));
        } else {
            let t = (mid - a[1]) / (b[1] - a[1]);
            let cross = [a[0] + t * (b[0] - a[0]), mid];
            shape = shape.fill(vec![a, cross, [a[0], mid]], tint(above_a)).fill(vec![cross, b, [b[0], mid]], tint(above_b));
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
                    text(crate::screens::settings::i18n::fill(tr("ended {}"), &[&signed(final_lead.round() as i64)]))
                        .font(TNUM_SMALL)
                        .color(if final_lead >= 0.0 { u.teal } else { u.rose }),
                )
                .child(spacer())
                .child(text(tr("Ticks: objectives lost")).font(SMALL).color(u.muted)),
        )
        .child(canvas(shape).w_full().h(CHART_H).role("img").label(tr("Soul lead through the match")).description(caption))
}

fn team_block(s: &State, u: &Ui, board: &MatchScoreboard, index: usize) -> El {
    let team = board.team(index);
    if team.is_empty() {
        return col();
    }
    let label = match board.my_team() {
        Some(mine) if mine == index => tr("Your team"),
        Some(_) => tr("Enemy team"),
        None if index == 0 => tr("Team 0"),
        None => tr("Team 1"),
    };
    let souls: u64 = team.iter().map(|player| player.net_worth).sum();
    let wins = board.winning_team == Some(index as i32);
    let tier_names = s.stats.snapshot.as_ref().map(|snap| &snap.tier_names);
    let mut head = row()
        .w_full()
        .h(36.0)
        .mt(8.0)
        .px(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(label.to_string()).font(BODY_MED).color(u.ink))
        .child(text(if wins { tr("Won") } else { tr("Lost") }).font(SMALL).color(if wins { u.teal } else { u.rose }))
        .child(text(crate::screens::settings::i18n::fill(tr("{} souls"), &[&thousands(souls)])).font(TNUM_SMALL).color(u.muted));
    if let Some(badge) = board.average_badge.get(index).copied().flatten() {
        let name = tier_names.map(|names| insight::badge_label(badge, names)).unwrap_or_else(|| badge.to_string());
        head = head
            .child(spacer())
            .child(text(tr("Average")).font(SMALL).color(u.muted))
            .child(rank_badge(s, u, badge, 20.0))
            .child(text(name).font(SMALL).color(u.prose));
    }
    let mut block = col().w_full().child(head);
    for player in team {
        block = block.child(player_row(s, u, board, player));
    }
    block
}

fn player_row(s: &State, u: &Ui, board: &MatchScoreboard, player: &BoardPlayer) -> El {
    let name = player.persona_name.clone().unwrap_or_else(|| player.account_id.to_string());
    let hero = player
        .hero_name
        .clone()
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(tr("Hero {}"), &[&player.hero_id.to_string()]));
    let mine = player.account_id == board.account_id && board.account_id > 0;
    let id = player.account_id;
    let best = |column: Column| board.is_best(column, id);
    let figure = |value: String, w: f32, column: Column| {
        let on = best(column);
        cell(text(value).font(if on { TNUM_BOLD } else { TNUM }).color(if on { u.ink } else { u.prose }).align(rune::Align::End), w)
    };
    let mut build = row().w(BUILD_W).gap(2.0).center_y();
    for item in player.build.iter().take(12) {
        build = build.child(item_icon(s, u, item, ICON));
    }
    if player.build.is_empty() {
        build = build.child(text(tr("No items recorded")).font(SMALL).color(u.muted));
    }
    let tier_names = s.stats.snapshot.as_ref().map(|snap| &snap.tier_names);
    let badge_help = match (player.badge, tier_names) {
        (Some(badge), Some(names)) => insight::badge_label(badge, names),
        (Some(badge), None) => badge.to_string(),
        (None, _) => tr("No rank").to_string(),
    };
    let mvp = player.mvp_rank == Some(1);
    row()
        .key(format!("board-player-{}", player.account_id))
        .w_full()
        .h(ROW_H)
        .px(8.0)
        .gap(GAP)
        .center_y()
        .fill(if mine { u.selected } else { Color::TRANSPARENT })
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(row().w(PLAYER_W).min_w(0.0).center_y().child(player_link(s, u, player.account_id, &name, 20.0, mine)))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(hero_chip(s, u, player.hero_id, &hero, 20.0))
                .child(text(hero).font(BODY).color(u.prose).truncate().min_w(0.0).shrink())
                .children(mvp.then(|| {
                    row().h(18.0).px(4.0).center().radius(2.0).fill(u.amber.alpha(0.14)).child(text(tr("MVP")).font(crate::ui::MICRO).color(u.amber))
                })),
        )
        .child(row().w(RANK_W).center_y().help(badge_help).child(rank_badge(s, u, player.badge.unwrap_or(0), 20.0)))
        .child(super::brief::kda_text(u, player.kills, player.deaths, player.assists).w(KDA_W))
        .child(figure(thousands(player.net_worth), SOULS_W, Column::Souls))
        .child(figure(thousands(player.souls_per_minute(board.duration_seconds)), SPM_W, Column::SoulsPerMinute))
        .child(figure(thousands(player.player_damage), DMG_W, Column::Damage))
        .child(figure(thousands(player.player_healing), HEAL_W, Column::Healing))
        .child(cell(text(format!("{} / {}", player.last_hits, player.denies)).font(if best(Column::LastHits) { TNUM_BOLD } else { TNUM }).color(u.muted).align(rune::Align::End), LH_W))
        .child(build)
}
