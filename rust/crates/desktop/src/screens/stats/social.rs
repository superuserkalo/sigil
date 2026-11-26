//! Squad: the players you queue with, then teammates and opponents.
//!
//! The squad is the premades (`same_party=true`: same team and Steam friends at the start of the
//! match), as cards with your win rate with and without them. "Without" joins each row's
//! shared match ids with the loaded match history (`deadlock_core::stats::social::with_without`)
//! and reads "-" when the history holds less than half of them or either side has under five.
//!
//! Both lists need three shared matches before a row appears, the API's own
//! floor, and a rate needs five (`MIN_RATE_SAMPLE`): below that it reads "-",
//! so one win in one game is never "100%". The teammates table sorts by matches
//! together, win rate, matches in the same lane and win rate in the same lane
//! (lanes from `deadlock_core::stats::social`). A premade is marked from the
//! same-party request, and every row opens that player's page.

use deadlock_core::stats::MatchSummary;
use deadlock_core::stats::social::{PlayerRecord, Social, with_without};
use rune::{Color, El, col, el, icon, row, spacer, text};

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
    let section = col().key("stats-social").w_full().gap(12.0);
    match &v.social {
        Fetched::Idle | Fetched::Loading => section
            .child(section_head(u, tr("Squad"), ""))
            .child(text(tr("Loading teammates and opponents")).font(BODY).color(u.muted)),
        Fetched::Failed(error) => section.child(section_head(u, tr("Squad"), "")).child(problem(u, error)),
        Fetched::Ready(social) => section.child(body(s, u, social)),
    }
}

fn body(s: &State, u: &Ui, social: &Social) -> El {
    let all = s.stats.social_all;
    let limit = if all { ROWS_ALL } else { ROWS };
    let total = social.mates.len() + social.enemies.len();
    let history: &[MatchSummary] = s.stats.snapshot.as_ref().map(|snap| snap.matches.as_slice()).unwrap_or(&[]);
    if total == 0 {
        return col()
            .w_full()
            .gap(12.0)
            .child(text(tr("Nobody has three shared matches yet. It fills in as you play.")).font(BODY).color(u.muted))
            .child(refresh(u));
    }

    let sort = s.stats.people_sort;
    let squad_only = s.stats.squad_only;
    let mates: Vec<&PlayerRecord> = sorted_mates(social, sort, limit, history, squad_only);
    let enemies: Vec<&PlayerRecord> = social
        .enemies
        .iter()
        .filter(|row| row.matches >= deadlock_core::stats::social::MIN_SHARED_MATCHES)
        .take(limit)
        .collect();

    let only = crate::ui::Segmented::new("stats-squad-only", tr("Teammates shown"))
        .size(crate::ui::SegSize::M)
        .part(crate::ui::SegPart::new("stats-squad-only-all", tr("Everyone")).on(!squad_only).with(|e| e.on_press(|st: &mut State| st.stats.squad_only = false)))
        .part(crate::ui::SegPart::new("stats-squad-only-squad", tr("Squad only")).on(squad_only).with(|e| e.on_press(|st: &mut State| st.stats.squad_only = true)))
        .build(u);
    let mut mates_table = col()
        .key("stats-people-mates")
        .w_full()
        .child(
            row()
                .w_full()
                .h(32.0)
                .gap(12.0)
                .center_y()
                .child(text(tr("Teammates")).font(BODY_MED).color(u.ink))
                .child(text(tr("three shared matches or more")).font(SMALL).color(u.muted))
                .child(spacer())
                .child(only),
        )
        .child(mates_head(u, sort, social.lanes_known));
    if mates.is_empty() {
        mates_table = mates_table.child(text(tr("None yet")).font(BODY).color(u.muted).py(10.0));
    }
    for record in &mates {
        mates_table = mates_table.child(mate_row(s, u, record, social.lanes_known, history));
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
    mates_table = mates_table.child(
        text(tr("Without: your win rate in the other matches of the loaded history. Shown when that history holds at least half of the shared matches, and both sides have 5 matches."))
            .font(SMALL)
            .color(u.muted)
            .pt(8.0)
            .wrap(),
    );

    let mut enemies_table = col()
        .key("stats-people-enemies")
        .w_full()
        .child(row().h(32.0).center_y().child(text(tr("Opponents")).font(BODY_MED).color(u.ink)))
        .child(head_row(
            u,
            &[("Player", 0.0, false), ("Against", COL_COUNT, true), ("Your win rate", COL_LANE_RATE, true), ("Last met", COL_LAST, true), ("", MARKS, false)],
        ));
    if enemies.is_empty() {
        enemies_table = enemies_table.child(text(tr("None yet")).font(BODY).color(u.muted).py(10.0));
    }
    for record in &enemies {
        enemies_table = enemies_table.child(enemy_row(s, u, record, history));
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
        .child(squad_cards(s, u, social, history))
        .child(mates_table)
        .child(enemies_table)
        .child(foot.child(refresh(u)))
}

/// The squad: the three premades played with most, as cards, with and without them.
fn squad_cards(s: &State, u: &Ui, social: &Social, history: &[MatchSummary]) -> El {
    use crate::screens::settings::i18n::fill;
    let mine = !super::nav::visiting(s);
    let mut squad: Vec<&PlayerRecord> = social.premades().collect();
    squad.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.account_id.cmp(&b.account_id)));
    let head = row()
        .w_full()
        .h(32.0)
        .gap(12.0)
        .center_y()
        .child(text(if mine { tr("Your squad") } else { tr("Their squad") }).font(crate::ui::H2).color(u.ink))
        .child(text(tr("players queued with: Steam friends on the same team")).font(SMALL).color(u.muted));
    if squad.is_empty() {
        return col().key("stats-squad-cards").w_full().gap(4.0).child(head).child(
            text(tr("No squad found. deadlock-api only knows a squad when the Steam friends list is public."))
                .font(BODY)
                .color(u.muted),
        );
    }
    let mut cards = row().w_full().gap(8.0);
    for record in squad.iter().take(3) {
        let split = with_without(record, history);
        let name = record.persona_name.clone().unwrap_or_else(|| record.account_id.to_string());
        let together = match record.rate_together() {
            Some(rate) => fill(tr("{} together, {} win"), &[&record.matches.to_string(), &format!("{rate:.0}%")]),
            None => fill(tr("{} together"), &[&record.matches.to_string()]),
        };
        let lane = match record.lane {
            Some(lane) if lane.matches > 0 => match record.rate_in_lane() {
                Some(rate) => fill(tr("same lane {}, {} win"), &[&lane.matches.to_string(), &format!("{rate:.0}%")]),
                None => fill(tr("same lane {}"), &[&lane.matches.to_string()]),
            },
            _ => String::new(),
        };
        let account_id = record.account_id;
        cards = cards.child(
            col()
                .key(format!("stats-squad-card-{account_id}"))
                .w(0.0)
                .grow(1.0)
                .min_w(0.0)
                .h(176.0)
                .padding(12.0)
                .gap(6.0)
                .radius(crate::ui::r(4.0))
                .fill(u.raised)
                .border(1.0, u.hair)
                .child(player_link(s, u, account_id, &name, 28.0, false))
                .child(text(together).font(TNUM_SMALL).color(u.prose))
                .child(lift_bar(u, record.rate_together(), split.without_rate(), split.lift_from(record.rate_together())))
                .child(text(lane).font(TNUM_SMALL).color(u.muted)),
        );
    }
    // Keep three columns: a squad of one or two leaves the room empty.
    for _ in squad.len().min(3)..3 {
        cards = cards.child(el().w(0.0).grow(1.0).min_w(0.0).h(176.0));
    }
    col().key("stats-squad-cards").w_full().gap(8.0).child(head).child(cards)
}

/// Column widths of the people tables. Fixed, so a sort or a new number never moves a column.
const COL_COUNT: f32 = 72.0;
const COL_RATE: f32 = 72.0;
const COL_LANE: f32 = 80.0;
const COL_LANE_RATE: f32 = 104.0;
const COL_LIFT: f32 = 56.0;
const COL_LAST: f32 = 96.0;
/// The row's more button.
const MARKS: f32 = 32.0;

/// How the teammates table is sorted. Every column sorts high to low; a rate below the
/// sample floor ("-") always sorts last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeopleSort {
    Together,
    #[default]
    Rate,
    Without,
    Lift,
    Lane,
    LaneRate,
}

/// Teammates in the table's order, `limit` of them.
pub(super) fn sorted_mates<'a>(
    social: &'a Social,
    sort: PeopleSort,
    limit: usize,
    history: &[MatchSummary],
    squad_only: bool,
) -> Vec<&'a PlayerRecord> {
    let mut rows: Vec<&PlayerRecord> = social
        .mates
        .iter()
        .filter(|row| row.matches >= deadlock_core::stats::social::MIN_SHARED_MATCHES)
        .filter(|row| !squad_only || row.premade)
        .collect();
    let key = |row: &PlayerRecord| -> (Option<f64>, u64) {
        match sort {
            PeopleSort::Without => (with_without(row, history).without_rate(), row.matches),
            PeopleSort::Lift => (with_without(row, history).lift_from(row.rate_together()), row.matches),
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
        ("Without", tr("Your win rate in the other matches of the loaded history"), COL_RATE, PeopleSort::Without),
        ("Lift", tr("With minus without, in points"), COL_LIFT, PeopleSort::Lift),
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
    r.child(cell(text(tr("Last together")).font(SMALL).color(u.muted).align(rune::Align::End).pr(12.0), COL_LAST)).child(row().w(MARKS))
}

/// "Yesterday", "3 d ago": when a match in the history was played.
fn ago(s: &State, at: Option<i64>) -> String {
    let now = s.stats.snapshot.as_ref().map(|snap| snap.generated_unix).unwrap_or(0);
    match at {
        Some(at) => deadlock_core::stats::insight::day_label(at, now),
        None => "-".to_string(),
    }
}

fn text_cell(u: &Ui, value: String, w: f32) -> El {
    cell(text(value).font(TNUM_SMALL).color(u.muted).align(rune::Align::End).pr(12.0).truncate(), w)
}

fn lift_cell(u: &Ui, lift: Option<f64>, w: f32) -> El {
    match lift {
        Some(points) => cell(
            text(format!("{points:+.0}")).font(TNUM).color(if points >= 3.0 { u.teal } else if points <= -3.0 { u.rose } else { u.prose }).align(rune::Align::End).pr(12.0),
            w,
        ),
        None => count_cell(None, w, u.muted),
    }
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

/// The player cell: picture and name, a link to their page, with "squad" after a premade.
fn name_cell(s: &State, u: &Ui, record: &PlayerRecord) -> El {
    let name = record.persona_name.clone().unwrap_or_else(|| record.account_id.to_string());
    let mut cell = row().grow(1.0).min_w(0.0).gap(8.0).center_y().child(player_link(s, u, record.account_id, &name, 24.0, false));
    if record.premade {
        cell = cell.child(
            text(tr("squad"))
                .font(SMALL)
                .color(u.muted)
                .help(tr("Queued together: on your team and Steam friends at the start of the match")),
        );
    }
    cell
}

fn person(u: &Ui, record: &PlayerRecord) -> El {
    row()
        .key(format!("stats-person-{}", record.account_id))
        .w_full()
        .h(40.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
}

/// The row's more button: Track, Statlocker and Copy id.
fn marks(u: &Ui, record: &PlayerRecord) -> El {
    let account_id = record.account_id;
    let key = format!("stats-person-more-{account_id}");
    row().w(MARKS).center_y().justify_end().child(
        u.more_button(tr("More for this player")).key(key.clone()).cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| {
            st.stats.menu = Some(super::Menu::Person(account_id, key.clone()));
        }),
    )
}

fn mate_row(s: &State, u: &Ui, record: &PlayerRecord, lanes: bool, history: &[MatchSummary]) -> El {
    let id = record.account_id;
    let split = with_without(record, history);
    person(u, record)
        .child(name_cell(s, u, record))
        .child(count_cell(Some(record.matches), COL_COUNT, u.prose))
        .child(rate_cell(u, record.rate_together(), COL_RATE, format!("stats-person-rate-{id}")))
        .child(rate_cell(u, split.without_rate(), COL_RATE, format!("stats-person-without-{id}")))
        .child(lift_cell(u, split.lift_from(record.rate_together()), COL_LIFT))
        .child(count_cell(record.lane.map(|lane| lane.matches), COL_LANE, if lanes { u.prose } else { u.muted }))
        .child(rate_cell(u, record.rate_in_lane(), COL_LANE_RATE, format!("stats-person-lane-rate-{id}")))
        .child(text_cell(u, ago(s, split.last_together), COL_LAST))
        .child(marks(u, record))
}

fn enemy_row(s: &State, u: &Ui, record: &PlayerRecord, history: &[MatchSummary]) -> El {
    let id = record.account_id;
    let split = with_without(record, history);
    person(u, record)
        .child(name_cell(s, u, record))
        .child(count_cell(Some(record.matches), COL_COUNT, u.prose))
        .child(rate_cell(u, record.rate_together(), COL_LANE_RATE, format!("stats-person-rate-{id}")))
        .child(text_cell(u, ago(s, split.last_together), COL_LAST))
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
