//! Matches by sitting, and the detail a row opens to.

use std::collections::BTreeMap;

use deadlock_core::stats::insight::{self, Session};
use deadlock_core::stats::{HeroPerformance, MatchSummary, StatsSnapshot};
use rune::{Color, El, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::{Derived, SITTINGS_SHOWN};

// --- matches ---------------------------------------------------------------

pub(super) fn matches(s: &State, u: &Ui, snap: &StatsSnapshot, d: &Derived) -> El {
    let mut table = col().key("stats-matches").w_full().child(section_head(
        u,
        "Matches",
        "By sitting. Open a match for its detail",
    ));
    if snap.matches.is_empty() {
        return table.child(
            text(tr("No matches yet. New matches show up a little while after they end."))
                .font(BODY)
                .color(u.muted),
        );
    }
    table = table.child(head_row(
        u,
        &[
            ("Result", 48.0, false),
            ("Hero", 0.0, false),
            ("K / D / A", 96.0, true),
            ("KDA", 48.0, true),
            ("Souls", 56.0, true),
            ("Rank", 56.0, true),
            ("Length", 48.0, true),
            ("Time", 48.0, true),
            ("", 16.0, false),
        ],
    ));
    let averages: BTreeMap<u32, &HeroPerformance> =
        snap.heroes.iter().map(|hero| (hero.hero_id, hero)).collect();
    for (index, sitting) in d.sessions.iter().take(s.stats.sittings_shown).enumerate() {
        // The page's ranked and date filter (`more.rs`): a sitting with nothing left is skipped.
        let now = snap.generated_unix.max(item_now());
        let kept: Vec<usize> = sitting
            .indices
            .iter()
            .copied()
            .filter(|i| super::more::keeps(&s.stats.more, &snap.matches[*i], now))
            .collect();
        if kept.is_empty() {
            continue;
        }
        table = table.child(sitting_head(u, snap, sitting, index));
        for &match_index in &kept {
            let item = &snap.matches[match_index];
            let open = s.stats.expanded == Some(item.match_id);
            table = table.child(match_row(s, u, item, open));
            if open {
                table = table.child(match_detail(
                    s,
                    u,
                    snap,
                    item,
                    averages.get(&item.hero_id).copied(),
                ));
            }
        }
    }
    let more = d.sessions.len().saturating_sub(s.stats.sittings_shown);
    if more > 0 {
        table = table.child(
            row().w_full().h(40.0).center_y().child(spacer()).child(
                u.action(
                    &crate::screens::settings::i18n::fill(tr("Show earlier sittings ({})"), &[&more.to_string()]),
                    ActionKind::Ghost,
                )
                .key("stats-more-sittings")
                .mr(-12.0)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.stats.sittings_shown += SITTINGS_SHOWN),
            ),
        );
    }
    table
}

fn sitting_head(u: &Ui, snap: &StatsSnapshot, sitting: &Session, index: usize) -> El {
    let day = capitalise(&insight::day_label(sitting.start_time, snap.generated_unix));
    let day = format!("{day}, {}", insight::clock_label(sitting.start_time));
    let mut r = row()
        .key(format!("sitting-{index}"))
        .w_full()
        .h(40.0)
        .pt(12.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(day).font(BODY_MED).color(u.prose))
        .child(
            text(format!(
                "{} {}  ·  {} W {} L",
                sitting.indices.len(),
                if sitting.indices.len() == 1 {
                    "match"
                } else {
                    "matches"
                },
                sitting.wins,
                sitting.losses
            ))
            .font(TNUM_SMALL)
            .color(u.faint),
        )
        .child(spacer());
    if sitting.ranked > 0 && sitting.rank_delta != 0 {
        r = r.child(
            text(format!("{} rank points", signed(sitting.rank_delta)))
                .font(TNUM_SMALL)
                .color(if sitting.rank_delta > 0 { u.teal } else { u.rose }),
        );
    }
    r.child(el().w(32.0))
}

fn match_row(s: &State, u: &Ui, item: &MatchSummary, open: bool) -> El {
    let (result, color) = match item.win {
        Some(true) => (tr("Win").to_string(), u.teal),
        Some(false) => (tr("Loss").to_string(), u.rose),
        None => (crate::ui::sentence(item.outcome.label()), u.faint),
    };
    let name = item
        .hero_name
        .clone()
        .unwrap_or_else(|| format!("Hero {}", item.hero_id));
    let id = item.match_id;
    let rank = match item.ranked_delta {
        Some(delta) if delta != 0 => (format!("{delta:+}"), if delta > 0 { u.teal } else { u.rose }),
        _ => (String::new(), u.faint),
    };
    let mut r = data_row(format!("match-{id}"), u, 48.0)
        .fill(if open { u.hover } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{result} on {name}, open match"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.stats.expanded = if st.stats.expanded == Some(id) {
                None
            } else {
                Some(id)
            }
        })
        .child(cell(text(result).font(BODY_MED).color(color), 48.0))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .center_y()
                .child(hero_chip(s, u, item.hero_id, &name, 24.0))
                .child(text(name).font(BODY).color(u.ink).truncate())
                // Grimoire's tag: Street Brawl souls and K/D do not compare with normal games.
                .children(item.is_street_brawl().then(|| {
                    row()
                        .h(20.0)
                        .px(6.0)
                        .center()
                        .radius(crate::ui::r(5.0))
                        .border(1.0, u.amber.alpha(0.5))
                        .child(text(tr("Street Brawl")).font(SMALL).color(u.amber))
                }))
                // DMM's abandoned mark: the game recorded the player as leaving this match.
                .children(super::notes::abandon_tag(u, item)),
        )
        .child(num(
            format!("{} / {} / {}", item.kills, item.deaths, item.assists),
            96.0,
            u.prose,
        ))
        .child(num(format!("{:.2}", item.kda), 48.0, u.muted))
        .child(num(souls(item.net_worth), 56.0, u.muted))
        .child(num(rank.0, 56.0, rank.1))
        .child(num(format!("{}m", item.duration_seconds / 60), 48.0, u.muted))
        .child(num(insight::clock_label(item.start_time), 48.0, u.faint));
    r = r.child(
        el().w(16.0).center().child(
            icon(
                if open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                14.0,
            )
            .color(u.faint)
            .a11y_hidden(),
        ),
    );
    r
}

/// The match, opened: what the history holds, against the hero's average, and
/// the way to the moment (a reviewed demo).
fn match_detail(
    s: &State,
    u: &Ui,
    snap: &StatsSnapshot,
    item: &MatchSummary,
    average: Option<&HeroPerformance>,
) -> El {
    let minutes = (item.duration_seconds as f64 / 60.0).max(1.0);
    let mode = match item.match_mode {
        1 => tr("Unranked"),
        2 => tr("Private lobby"),
        3 => tr("Bots"),
        4 => tr("Ranked"),
        7 => tr("Hero Labs"),
        8 => tr("Placement"),
        _ => tr("Other"),
    };
    let mut facts = row()
        .w_full()
        .gap(32.0)
        .items_start()
        .child(fact(u, "Souls a minute", thousands((item.net_worth as f64 / minutes).round() as u64)))
        .child(fact(u, "Last hits", item.last_hits.to_string()))
        .child(fact(u, "Denies", item.denies.to_string()))
        .child(fact(u, "Mode", mode.to_string()));
    if let Some(badge) = item.ranked_badge.filter(|badge| *badge > 0) {
        facts = facts.child(fact(u, "Rank", insight::badge_label(badge, &snap.tier_names)));
    }
    facts = facts.child(
        col()
            .gap(2.0)
            .child(text(tr("Match")).font(SMALL).color(u.faint))
            .child(
                text(item.match_id.to_string())
                    .font(MONO)
                    .color(u.prose)
                    .selectable(true),
            ),
    );

    let mut body = col().w_full().gap(16.0).child(facts);
    if let Some(hero) = average.filter(|hero| hero.matches_played >= 5) {
        let avg_deaths = hero.deaths as f64 / hero.matches_played.max(1) as f64;
        let name = hero_name(hero);
        let kda_better = item.kda >= hero.kda;
        let deaths_better = (item.deaths as f64) <= avg_deaths;
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(format!("Against your {name} average"))
                        .font(SMALL)
                        .color(u.muted),
                )
                .child(compare_chip(
                    u,
                    &format!("KDA {:.2} vs {:.2}", item.kda, hero.kda),
                    kda_better,
                ))
                .child(compare_chip(
                    u,
                    &format!("Deaths {} vs {avg_deaths:.1}", item.deaths),
                    deaths_better,
                )),
        );
    }
    let key = item.match_id.to_string();
    let review = s.stats.demos.iter().find(|demo| demo.match_id == key);
    body = body.child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                u.action(tr("Scoreboard"), ActionKind::Secondary)
                    .key(format!("stats-board-{key}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press({
                        let id = item.match_id;
                        move |st: &mut State| crate::data::start_scoreboard_load(st, id)
                    }),
            )
            .child(text(tr("Both teams, final builds and the souls curve")).font(SMALL).color(u.faint))
            .child(spacer())
            .child(
                u.icon_action(&icons::EXTERNAL_LINK, tr("Open on Statlocker"), ActionKind::Ghost)
                    .key(format!("stats-match-statlocker-{key}"))
                    .role("link")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx({
                        let url = statlocker_match_url(item.match_id);
                        move |st: &mut State, tx: &mut rune::Tx| open_statlocker(st, tx, &url)
                    }),
            ),
    );
    body = body.child(super::more::timeline(s, u, item.match_id));
    body = body.child(match review {
        Some(_) => row().gap(12.0).center_y().child(
            u.action(tr("Review this match"), ActionKind::Secondary)
                .key(format!("stats-review-{key}"))
                .cursor(rune::Cursor::Pointer)
                .on_press({
                    let id = key.clone();
                    move |st: &mut State| crate::data::open_review(st, &id)
                }),
        ),
        None => text(tr("Moment by moment review needs this match's demo. Import it under Match reviews below."))
            .font(SMALL)
            .color(u.faint),
    });

    col()
        .key(format!("match-detail-{}", item.match_id))
        .w_full()
        .pl(48.0 + 16.0)
        .pt(12.0)
        .pb(16.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(body)
}

/// The clock, for the date filter on the match list.
fn item_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
