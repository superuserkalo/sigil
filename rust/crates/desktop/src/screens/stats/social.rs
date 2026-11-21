//! Players you meet: the teammates and opponents behind a match window.
//!
//! Both lists need three shared matches before a row appears, the API's own
//! floor. A premade is marked from the same-party request, and every row opens
//! that player's page, so a name on the scoreboard becomes a history.

use deadlock_core::stats::social::{PlayerRecord, Social};
use rune::{El, col, icon, row, spacer, text};

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

    let mates: Vec<&PlayerRecord> = best_mates(social, limit);
    let enemies: Vec<&PlayerRecord> = social
        .enemies
        .iter()
        .filter(|row| row.matches >= deadlock_core::stats::social::MIN_SHARED_MATCHES)
        .take(limit)
        .collect();

    let grid = row()
        .w_full()
        .gap(48.0)
        .items_start()
        .child(column(u, tr("Best teammates"), &mates, false))
        .child(column(u, tr("Frequent opponents"), &enemies, true));

    let hidden = social.mates.len() > ROWS || social.enemies.len() > ROWS;
    // One foot row: show more, then refresh, on the same line.
    let mut foot = row().w_full().h(32.0).gap(8.0).center_y().child(spacer());
    if hidden {
        foot = foot.child(
                    u.action(
                        if all {
                            tr("Show fewer")
                        } else {
                            tr("Show more")
                        },
                        ActionKind::Ghost,
                    )
                    .key("stats-social-toggle")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.stats.social_all = !st.stats.social_all),
        );
    }
    col().w_full().gap(12.0).child(grid).child(foot.child(refresh(u)))
}

/// Teammates by win rate, needing more than a couple of games together.
fn best_mates(social: &Social, limit: usize) -> Vec<&PlayerRecord> {
    let mut rows: Vec<&PlayerRecord> = social
        .mates
        .iter()
        .filter(|row| row.matches >= deadlock_core::stats::social::BEST_MATE_MATCHES)
        .collect();
    rows.sort_by(|left, right| {
        right
            .win_rate_percent
            .total_cmp(&left.win_rate_percent)
            .then(right.matches.cmp(&left.matches))
    });
    rows.truncate(limit);
    rows
}

fn column(u: &Ui, label: &str, rows: &[&PlayerRecord], against: bool) -> El {
    let mut column = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(0.0)
        .child(
            text(label.to_string())
                .font(BODY_MED)
                .color(u.muted)
                .pb(6.0),
        );
    if rows.is_empty() {
        return column.child(
            text(tr("None yet"))
                .font(BODY)
                .color(u.muted)
                .py(10.0),
        );
    }
    for record in rows {
        column = column.child(person_row(u, record, against));
    }
    column
}

fn person_row(u: &Ui, record: &PlayerRecord, against: bool) -> El {
    let account_id = record.account_id;
    let name = record
        .persona_name
        .clone()
        .unwrap_or_else(|| account_id.to_string());
    let shared = crate::screens::settings::i18n::fill(
        tr("{} together"),
        &[&record.matches.to_string()],
    );
    let rate = crate::screens::settings::i18n::fill(
        if against {
            tr("{}% against them")
        } else {
            tr("{}% with them")
        },
        &[&format!("{:.0}", record.win_rate_percent)],
    );
    let mut r = row()
        .key(format!("stats-person-{account_id}"))
        .w_full()
        .h(44.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(crate::screens::settings::i18n::fill(
            tr("Open the stats for {}"),
            &[&name],
        ))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| crate::data::load_stats_account(st, account_id))
        .child(avatar(u, &name, 24.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(0.0)
                .child(
                    text(name)
                        .font(BODY_MED)
                        .color(u.ink)
                        .truncate(),
                )
                .child(
                    text(shared)
                        .font(SMALL)
                        .color(u.muted)
                        .truncate(),
                ),
        )
        .child(rate_text(u, rate, record.win_rate_percent));
    // The "queued together" mark keeps its slot, so the Statlocker buttons line up.
    r = r.child(if record.premade {
        row().w(16.0).center().child(icon(&icons::PIN, 12.0).color(u.muted).label(tr("Queued together")))
    } else {
        row().w(16.0)
    });
    r.child(statlocker_link(
        u,
        format!("stats-person-statlocker-{account_id}"),
        statlocker_profile_url(account_id),
        tr("Open on Statlocker"),
    ))
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
    text(rate).font(TNUM_SMALL).color(tone)
}

fn refresh(u: &Ui) -> El {
    u.icon_button(&icons::REFRESH_CW, tr("Refresh teammates and opponents"))
        .key("stats-social-refresh")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| crate::data::start_social_load(st))
}
