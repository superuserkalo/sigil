//! Players you meet: the teammates and opponents behind a match window.
//!
//! Both lists need three shared matches before a row appears, the API's own
//! floor, and a rate needs five (`MIN_RATE_SAMPLE`): below that it reads "-",
//! so one win in one game is never "100%". The teammates table sorts by matches
//! together, win rate, matches in the same lane and win rate in the same lane
//! (lanes from `deadlock_core::stats::social`). A premade is marked from the
//! same-party request, and every row opens that player's page.

use deadlock_core::stats::social::{PlayerRecord, Social};
use rune::{Color, El, col, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::{Fetched, View};

/// Rows per column before "Show all".
const ROWS: usize = 5;
const ROWS_ALL: usize = 12;

pub(super) fn social_section(s: &State, u: &Ui) -> El {
    let v: &View = &s.stats;
    let mut section = col()
        .key("stats-social")
        .w_full()
        .gap(12.0)
        .child(section_head(
            u,
            tr("Players you meet"),
            tr("Teammates and opponents, three shared matches or more"),
        ));

    section = match &v.social {
        Fetched::Idle => section
            .child(
                text(tr(
                    "Who you queue with, who you keep facing, and how those games went.",
                ))
                .font(BODY)
                .color(u.muted),
            )
            .child(
                row()
                    .w_full()
                    .child(
                        u.action(tr("Show players"), ActionKind::Secondary)
                            .key("stats-social-load")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| crate::data::start_social_load(st)),
                    ),
            ),
        Fetched::Loading => section.child(
            text(tr("Loading teammates and opponents"))
                .font(BODY)
                .color(u.muted),
        ),
        Fetched::Failed(error) => section.child(problem(u, error)),
        Fetched::Ready(social) => section.child(body(s, u, social)),
    };
    section
}

fn body(s: &State, u: &Ui, social: &Social) -> El {
    let all = s.stats.social_all;
    let limit = if all { ROWS_ALL } else { ROWS };
    let total = social.mates.len() + social.enemies.len();
    if total == 0 {
        return col()
            .w_full()
            .gap(12.0)
            .child(
                text(tr("Nobody has three shared matches yet. It fills in as you play."))
                    .font(BODY)
                    .color(u.muted),
            )
            .child(refresh(u));
    }

    let sort = s.stats.people_sort;
    let mates: Vec<&PlayerRecord> = sorted_mates(social, sort, limit);
    let enemies: Vec<&PlayerRecord> = social
        .enemies
        .iter()
        .filter(|row| row.matches >= deadlock_core::stats::social::MIN_SHARED_MATCHES)
        .take(limit)
        .collect();

    let mut mates_table = col()
        .key("stats-people-mates")
        .w_full()
        .child(text(tr("Teammates")).font(BODY_MED).color(u.muted).pb(6.0))
        .child(mates_head(u, sort, social.lanes_known));
    if mates.is_empty() {
        mates_table = mates_table.child(text(tr("None yet")).font(BODY).color(u.muted).py(10.0));
    }
    for record in &mates {
        mates_table = mates_table.child(mate_row(u, record, social.lanes_known));
    }
    if !social.lanes_known && !social.mates.is_empty() {
        mates_table = mates_table.child(
            text(tr("Lane data did not come back from the stats service, so only matches together are shown."))
                .font(SMALL)
                .color(u.muted)
                .pt(8.0)
                .wrap(),
        );
    }

    let mut enemies_table = col()
        .key("stats-people-enemies")
        .w_full()
        .child(text(tr("Frequent opponents")).font(BODY_MED).color(u.muted).pb(6.0))
        .child(head_row(u, &[("Player", 0.0, false), ("Against", COL_COUNT, true), ("Win rate", COL_RATE, true), ("", MARKS, false)]));
    if enemies.is_empty() {
        enemies_table = enemies_table.child(text(tr("None yet")).font(BODY).color(u.muted).py(10.0));
    }
    for record in &enemies {
        enemies_table = enemies_table.child(enemy_row(u, record));
    }

    let hidden = social.mates.len() > ROWS || social.enemies.len() > ROWS;
    // One foot row: show more, then refresh, on the same line.
    let mut foot = row().w_full().h(32.0).gap(8.0).center_y().child(spacer());
    if hidden {
        foot = foot.child(
            u.action(if all { tr("Show fewer") } else { tr("Show more") }, ActionKind::Ghost)
                .key("stats-social-toggle")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.stats.social_all = !st.stats.social_all),
        );
    }
    col()
        .w_full()
        .gap(24.0)
        .child(mates_table)
        .child(enemies_table)
        .child(foot.child(refresh(u)))
}

/// Column widths of the people tables. Fixed, so a sort or a new number never moves a column.
const COL_COUNT: f32 = 72.0;
const COL_RATE: f32 = 72.0;
const COL_LANE: f32 = 80.0;
const COL_LANE_RATE: f32 = 104.0;
/// The queued-together mark and the Statlocker button.
const MARKS: f32 = 52.0;

/// How the teammates table is sorted. Every column sorts high to low; a rate below the
/// sample floor ("-") always sorts last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeopleSort {
    Together,
    #[default]
    Rate,
    Lane,
    LaneRate,
}

/// Teammates in the table's order, `limit` of them.
pub(super) fn sorted_mates(social: &Social, sort: PeopleSort, limit: usize) -> Vec<&PlayerRecord> {
    let mut rows: Vec<&PlayerRecord> = social
        .mates
        .iter()
        .filter(|row| row.matches >= deadlock_core::stats::social::MIN_SHARED_MATCHES)
        .collect();
    let key = |row: &PlayerRecord| -> (Option<f64>, u64) {
        match sort {
            PeopleSort::Together => (Some(row.matches as f64), row.wins),
            PeopleSort::Rate => (row.rate_together(), row.matches),
            PeopleSort::Lane => (row.lane.map(|lane| lane.matches as f64), row.lane.map(|lane| lane.wins).unwrap_or(0)),
            PeopleSort::LaneRate => (row.rate_in_lane(), row.lane.map(|lane| lane.matches).unwrap_or(0)),
        }
    };
    rows.sort_by(|left, right| {
        let (a, a_tie) = key(left);
        let (b, b_tie) = key(right);
        match (a, b) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then(b_tie.cmp(&a_tie))
        .then(right.matches.cmp(&left.matches))
        .then(left.account_id.cmp(&right.account_id))
    });
    rows.truncate(limit);
    rows
}

/// The teammates head: each number column is a button that sorts by it. The active one is
/// ink with a down chevron in a slot every column keeps, so labels never move.
fn mates_head(u: &Ui, sort: PeopleSort, lanes: bool) -> El {
    let mut r = row()
        .w_full()
        .h(28.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(cell(text(tr("Player")).font(SMALL).color(u.muted), 0.0));
    let lane_help = if lanes {
        tr("Matches where you two had the same lane")
    } else {
        tr("Lane data did not come back from the stats service.")
    };
    for (label, help, width, key) in [
        ("Together", tr("Matches on the same team"), COL_COUNT, PeopleSort::Together),
        ("Win rate", tr("Your win rate in those matches, from 5 matches up"), COL_RATE, PeopleSort::Rate),
        ("Same lane", lane_help, COL_LANE, PeopleSort::Lane),
        ("Lane win rate", tr("Your win rate when you shared the lane, from 5 matches up"), COL_LANE_RATE, PeopleSort::LaneRate),
    ] {
        let on = sort == key;
        r = r.child(
            row()
                .key(format!("stats-people-sort-{label}"))
                .w(width)
                .h(28.0)
                .gap(2.0)
                .center_y()
                .focusable(true)
                .role("button")
                .label(crate::screens::settings::i18n::fill(tr("Sort by {}"), &[tr(label)]))
                .help(help.to_string())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.stats.people_sort = key)
                .child(spacer())
                .child(text(tr(label)).font(SMALL).color(if on { u.ink } else { u.muted }).align(rune::Align::End).a11y_hidden())
                .child(
                    row()
                        .w(12.0)
                        .center()
                        .child(icon(&icons::CHEVRON_DOWN, 10.0).color(if on { u.ink } else { Color::TRANSPARENT }).a11y_hidden()),
                ),
        );
    }
    r.child(row().w(MARKS))
}

/// A rate, or "-" with the reason in a tooltip below the sample floor.
fn rate_cell(u: &Ui, rate: Option<f64>, w: f32, key: String) -> El {
    match rate {
        Some(percent) => cell(
            rate_text(u, format!("{percent:.0}%"), percent).align(rune::Align::End).pr(12.0),
            w,
        ),
        None => cell(
            text("-".to_string())
                .key(key)
                .font(TNUM)
                .color(u.muted)
                .align(rune::Align::End)
                .pr(12.0)
                .help(crate::screens::settings::i18n::fill(
                    tr("Fewer than {} matches: too few for a rate"),
                    &[&deadlock_core::stats::social::MIN_RATE_SAMPLE.to_string()],
                )),
            w,
        ),
    }
}

fn count_cell(value: Option<u64>, w: f32, color: Color) -> El {
    cell(
        text(value.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()))
            .font(TNUM)
            .color(color)
            .align(rune::Align::End)
            .pr(12.0),
        w,
    )
}

fn name_cell(u: &Ui, record: &PlayerRecord) -> (String, El) {
    let name = record
        .persona_name
        .clone()
        .unwrap_or_else(|| record.account_id.to_string());
    let el = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(12.0)
        .center_y()
        .child(avatar(u, &name, 24.0))
        .child(text(name.clone()).font(BODY_MED).color(u.ink).truncate());
    (name, el)
}

fn person(u: &Ui, record: &PlayerRecord, name: &str) -> El {
    let account_id = record.account_id;
    row()
        .key(format!("stats-person-{account_id}"))
        .w_full()
        .h(40.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(crate::screens::settings::i18n::fill(tr("Open the stats for {}"), &[name]))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| crate::data::load_stats_account(st, account_id))
}

/// The queued-together mark keeps its slot, so the Statlocker buttons line up.
fn marks(u: &Ui, record: &PlayerRecord) -> El {
    let account_id = record.account_id;
    row()
        .w(MARKS)
        .gap(8.0)
        .center_y()
        .child(if record.premade {
            row().w(16.0).center().child(icon(&icons::PIN, 12.0).color(u.muted).label(tr("Queued together")))
        } else {
            row().w(16.0)
        })
        .child(statlocker_link(
            u,
            format!("stats-person-statlocker-{account_id}"),
            statlocker_profile_url(account_id),
            tr("Open on Statlocker"),
        ))
}

fn mate_row(u: &Ui, record: &PlayerRecord, lanes: bool) -> El {
    let (name, name_el) = name_cell(u, record);
    let id = record.account_id;
    person(u, record, &name)
        .child(name_el)
        .child(count_cell(Some(record.matches), COL_COUNT, u.prose))
        .child(rate_cell(u, record.rate_together(), COL_RATE, format!("stats-person-rate-{id}")))
        .child(count_cell(record.lane.map(|lane| lane.matches), COL_LANE, if lanes { u.prose } else { u.muted }))
        .child(rate_cell(u, record.rate_in_lane(), COL_LANE_RATE, format!("stats-person-lane-rate-{id}")))
        .child(marks(u, record))
}

fn enemy_row(u: &Ui, record: &PlayerRecord) -> El {
    let (name, name_el) = name_cell(u, record);
    let id = record.account_id;
    person(u, record, &name)
        .child(name_el)
        .child(count_cell(Some(record.matches), COL_COUNT, u.prose))
        .child(rate_cell(u, record.rate_together(), COL_RATE, format!("stats-person-rate-{id}")))
        .child(marks(u, record))
}

/// The rate. Wins are the player's wins in the shared games, in both lists, so a
/// high number is good whether the row is a teammate or an opponent.
fn rate_text(u: &Ui, rate: String, percent: f64) -> El {
    let tone = if percent >= 60.0 {
        u.teal
    } else if percent <= 40.0 {
        u.rose
    } else {
        u.prose
    };
    text(rate).font(TNUM).color(tone)
}

fn refresh(u: &Ui) -> El {
    u.icon_button(&icons::REFRESH_CW, tr("Refresh teammates and opponents"))
        .key("stats-social-refresh")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| crate::data::start_social_load(st))
}
