//! The brief, the four numbers for the recent window, and the progress charts.

use deadlock_core::stats::insight::{self, RankPoint, Tone};
use deadlock_core::stats::{MatchSummary, StatsSnapshot};
use rune::{Color, El, Shape, canvas, col, el, row, spacer, text};

use crate::app::State;
use crate::ui::{BODY, BODY_MED, DISPLAY, H1, H2, NUM, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::{Chart, Derived, FORM_BEADS, SPARK_MATCHES};

/// Who, then what changed, then what to work on next.
pub(super) fn brief_block(s: &State, u: &Ui, snap: &StatsSnapshot, d: &Derived) -> El {
    // A rank request that failed leaves `--`, not "Unranked": the page still loads.
    let rank = if snap.rank.reported {
        insight::badge_label(snap.rank.badge, &snap.tier_names)
    } else {
        "--".to_string()
    };
    let mut meta = vec![rank];
    // The share is only comparable with the players of the season the badge was earned in.
    if let Some(bench) = snap
        .rank
        .benchmark
        .as_ref()
        .filter(|bench| bench.current_season)
    {
        let players = thousands(bench.ranked_players);
        meta.push(match bench.top_percent_text() {
            Some(share) => crate::screens::settings::i18n::fill(
                tr("top {}% of {} ranked players"),
                &[&share, &players],
            ),
            None => crate::screens::settings::i18n::fill(
                tr("top <0.1% of {} ranked players"),
                &[&players],
            ),
        });
    }
    meta.push(format!(
        "{} matches in 30 days",
        snap.profile.matches_played_last_30d
    ));

    let who = row()
        .key("stats-identity")
        .w_full()
        .gap(12.0)
        .center_y()
        .child(avatar(u, &snap.profile.persona_name, 40.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(0.0)
                .child(
                    text(snap.profile.persona_name.clone())
                        .font(H1)
                        .color(u.ink)
                        .truncate(),
                )
                .child(text(meta.join("  ·  ")).font(TNUM).color(u.muted).truncate()),
        );

    let mut block = col().key("stats-brief").w_full().gap(20.0).child(who);

    let mut said = col().w_full().gap(4.0);
    if let Some(changed) = &d.brief.changed {
        said = said
            .child(
                text(changed.title.clone())
                    .font(DISPLAY)
                    .color(u.ink)
                    .wrap(),
            )
            .child(
                text(changed.detail.clone())
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            );
    }
    block = block.child(said);

    let mut list = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    if let Some(next) = &d.brief.next {
        list = list.child(callout(s, u, "Next", next, true).key("stats-next"));
    }
    for (index, note) in d.brief.also.iter().take(2).enumerate() {
        let label = match note.tone {
            Tone::Good => "Going well",
            Tone::Focus => "Watch",
            Tone::Neutral => "Note",
        };
        list = list.child(callout(s, u, label, note, false).key(format!("stats-note-{index}")));
    }
    block.child(list)
}

/// One insight as a row: a label column, the hero or a tone mark, then the
/// sentence and the numbers it rests on.
fn callout(s: &State, u: &Ui, label: &str, item: &insight::Insight, lead: bool) -> El {
    let mark = match item.tone {
        Tone::Good => u.teal,
        Tone::Focus => u.amber,
        Tone::Neutral => u.muted,
    };
    let lead_icon = match item.hero_id {
        Some(hero_id) => hero_chip(s, u, hero_id, &item.title, 24.0),
        None => el()
            .size(24.0, 24.0)
            .center()
            .child(el().size(6.0, 6.0).radius(3.0).fill(mark).a11y_hidden())
            .a11y_hidden(),
    };
    row()
        .w_full()
        .py(12.0)
        .gap(12.0)
        .items_start()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            text(tr(label))
                .font(BODY_MED)
                .color(mark)
                .w(BRIEF_LABEL_W)
                .pt(2.0),
        )
        .child(lead_icon)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .pt(2.0)
                .child(
                    text(item.title.clone())
                        .font(BODY_MED)
                        .color(if lead { u.ink } else { u.prose })
                        .wrap(),
                )
                .child(text(item.detail.clone()).font(BODY).color(u.muted).wrap()),
        )
}

const BRIEF_LABEL_W: f32 = 84.0;

// --- numbers ---------------------------------------------------------------

struct Metric {
    key: &'static str,
    label: &'static str,
    value: String,
    before: Option<String>,
    /// Signed change, already in "better is positive" orientation.
    delta: Option<(f64, String)>,
    peer: Option<String>,
    series: Vec<f64>,
}

pub(super) fn numbers(u: &Ui, snap: &StatsSnapshot, d: &Derived) -> El {
    let recent = &d.form.recent;
    let before = &d.form.before;
    let has_before = before.matches >= insight::MIN_SAMPLE;
    let n = recent.matches;
    let tier = tier_label(snap);
    let overall = snap.peers.as_ref().map(|peers| &peers.overall);
    let peer = |value: Option<f64>, digits: usize| -> Option<String> {
        match (&tier, value) {
            (Some(tier), Some(value)) if value > 0.0 => {
                Some(format!("{tier} average {value:.digits$}"))
            }
            _ => None,
        }
    };

    let newest: Vec<&MatchSummary> = snap.matches.iter().take(SPARK_MATCHES).collect();
    let series = |f: &dyn Fn(&MatchSummary) -> f64| -> Vec<f64> {
        let values: Vec<f64> = newest.iter().rev().map(|item| f(item)).collect();
        rolling_mean(&values, 5)
    };

    let win_rate = recent.win_rate().unwrap_or(0.0);
    let metrics = vec![
        Metric {
            key: "win",
            label: "Win rate",
            value: format!("{win_rate:.0}%"),
            before: before
                .win_rate()
                .filter(|_| has_before)
                .map(|rate| format!("{rate:.0}% before")),
            delta: before
                .win_rate()
                .filter(|_| has_before)
                .map(|rate| (win_rate - rate, format!("{:+.0}", win_rate - rate))),
            peer: Some(format!("{} W {} L", recent.wins, recent.losses)),
            series: {
                let values: Vec<f64> = newest
                    .iter()
                    .rev()
                    .filter_map(|item| item.win.map(|win| if win { 100.0 } else { 0.0 }))
                    .collect();
                rolling_mean(&values, 10)
            },
        },
        Metric {
            key: "kda",
            label: "KDA",
            value: format!("{:.2}", recent.kda()),
            before: has_before.then(|| format!("{:.2} before", before.kda())),
            delta: has_before.then(|| {
                let diff = recent.kda() - before.kda();
                (diff / before.kda().max(0.01) * 100.0, format!("{diff:+.2}"))
            }),
            peer: peer(overall.map(|o| o.kda), 2),
            series: series(&|item| item.kda),
        },
        Metric {
            key: "deaths",
            label: "Deaths a match",
            value: format!("{:.1}", recent.deaths_per_match()),
            before: has_before.then(|| format!("{:.1} before", before.deaths_per_match())),
            delta: has_before.then(|| {
                let diff = recent.deaths_per_match() - before.deaths_per_match();
                (
                    -diff / before.deaths_per_match().max(0.01) * 100.0,
                    format!("{diff:+.1}"),
                )
            }),
            peer: peer(overall.map(|o| o.deaths_per_match), 1),
            series: series(&|item| item.deaths as f64),
        },
        Metric {
            key: "souls",
            label: "Souls a minute",
            value: thousands(recent.souls_per_minute().round() as u64),
            before: has_before
                .then(|| format!("{} before", thousands(before.souls_per_minute().round() as u64))),
            delta: has_before.then(|| {
                let diff = recent.souls_per_minute() - before.souls_per_minute();
                (
                    diff / before.souls_per_minute().max(1.0) * 100.0,
                    format!("{:+.0}", diff),
                )
            }),
            peer: None,
            series: series(&|item| {
                if item.duration_seconds == 0 {
                    0.0
                } else {
                    item.net_worth as f64 / (item.duration_seconds as f64 / 60.0)
                }
            }),
        },
    ];

    let mut figures = row().w_full().gap(0.0).items_start();
    for (index, metric) in metrics.into_iter().enumerate() {
        figures = figures.child(figure(u, metric, index > 0));
    }
    col()
        .key("stats-numbers")
        .w_full()
        .gap(12.0)
        .child(section_head(
            u,
            &crate::screens::settings::i18n::fill(tr("Your last {}"), &[&n.to_string()]),
            if has_before {
                "Compared with the matches before"
            } else {
                ""
            },
        ))
        .child(figures)
}

fn figure(u: &Ui, m: Metric, divided: bool) -> El {
    let mut delta_row = row().gap(6.0).center_y().h(16.0);
    if let Some((better, label)) = &m.delta {
        // Within 3 percent (or 3 points of win rate) reads as level.
        let level = better.abs() < 3.0;
        let color = if level {
            u.faint
        } else if *better > 0.0 {
            u.teal
        } else {
            u.rose
        };
        let up = label.starts_with('+');
        if !level {
            delta_row = delta_row.child(arrow(color, up));
        }
        delta_row = delta_row.child(text(label.clone()).font(TNUM_SMALL).color(color));
    }
    if let Some(before) = &m.before {
        delta_row = delta_row.child(text(before.clone()).font(TNUM_SMALL).color(u.faint));
    }
    let mut c = col()
        .key(format!("stats-figure-{}", m.key))
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(text(tr(m.label)).font(SMALL).color(u.muted))
        .child(
            row()
                .w_full()
                .gap(4.0)
                .items_end()
                .child(text(m.value).font(NUM).color(u.ink).truncate().min_w(0.0).shrink())
                .child(spacer())
                .child(sparkline(u, &m.series, 88.0, 24.0)),
        )
        .child(delta_row);
    if let Some(peer) = m.peer {
        c = c.child(text(peer).font(TNUM_SMALL).color(u.faint));
    }
    if divided {
        c = c.pl(20.0).border_sides([0.0, 0.0, 0.0, 1.0], u.hair);
    }
    c.pr(20.0)
}

fn rolling_mean(values: &[f64], window: usize) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let window = window.max(1);
    (0..values.len())
        .map(|index| {
            let from = index.saturating_sub(window - 1);
            let slice = &values[from..=index];
            slice.iter().sum::<f64>() / slice.len() as f64
        })
        .collect()
}

// --- progress --------------------------------------------------------------

pub(super) const CHART_H: f32 = 176.0;
const CHART_VB: f32 = 1000.0;
const AXIS_W: f32 = 96.0;

pub(super) fn progress(s: &State, u: &Ui, snap: &StatsSnapshot, d: &Derived) -> El {
    let chart = s.stats.chart;
    let switch = crate::screens::subpage::segmented(
        u,
        vec![
            crate::screens::subpage::segment(u, tr("Rank"), chart == Chart::Rank)
                .key("stats-chart-rank")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.stats.chart = Chart::Rank),
            crate::screens::subpage::segment(u, tr("Win rate"), chart == Chart::WinRate)
                .key("stats-chart-win")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.stats.chart = Chart::WinRate),
        ],
    );
    let season = insight::season_start(&d.trajectory);
    let (body, caption) = match chart {
        Chart::Rank => (
            rank_chart(u, snap, &d.trajectory[season..]),
            if season > 0 {
                crate::screens::settings::i18n::fill(
                    tr("This season, {} ranked matches. Last season ended at {}"),
                    &[
                        &(d.trajectory.len() - season).to_string(),
                        &insight::badge_label(d.trajectory[season - 1].badge, &snap.tier_names),
                    ],
                )
            } else {
                format!("{} {}", d.trajectory.len(), tr("ranked matches"))
            },
        ),
        Chart::WinRate => (
            win_rate_chart(u, &d.rolling),
            crate::screens::settings::i18n::fill(
                tr("Each point is your win rate over the {} matches before it"),
                &[&insight::RECENT_MATCHES.to_string()],
            ),
        ),
    };
    col()
        .key("stats-progress")
        .w_full()
        .gap(12.0)
        .child(
            row()
                .w_full()
                .h(32.0)
                .gap(12.0)
                .center_y()
                .child(text(tr("Progress")).font(H2).color(u.ink))
                .child(text(caption).font(SMALL).color(u.faint).truncate().min_w(0.0))
                .child(spacer())
                .child(switch),
        )
        .child(body)
        .child(form_strip(u, snap, d))
}

fn rank_chart(u: &Ui, snap: &StatsSnapshot, points: &[RankPoint]) -> El {
    if points.len() < 2 {
        return chart_empty(u, "Not enough ranked matches to chart yet.");
    }
    let lo_step = points.iter().map(|p| p.step).min().unwrap_or(1);
    let hi_step = points.iter().map(|p| p.step).max().unwrap_or(1);
    let top = 11 * insight::SUBRANKS;
    let mut lo = lo_step.saturating_sub(1).max(1);
    let mut hi = (hi_step + 1).min(top);
    while hi - lo < 4 {
        if hi < top {
            hi += 1;
        } else if lo > 1 {
            lo -= 1;
        } else {
            break;
        }
    }
    let span = (hi - lo) as f32;
    let pad = 10.0;
    let y = |step: f32| pad + (hi as f32 - step) / span * (CHART_H - 2.0 * pad);
    let n = points.len();
    let x = |index: usize| index as f32 / (n - 1) as f32 * (CHART_VB - 8.0);

    let mut shape = Shape::new().view_box(CHART_VB, CHART_H);
    let mut labels = el().w(AXIS_W).h(CHART_H);
    let dense = hi - lo > 10;
    // Under 18 points a step, label every other subrank so labels never touch.
    let tight = (CHART_H - 2.0 * pad) / span < 18.0;
    for step in lo..=hi {
        let division = (step - 1) % insight::SUBRANKS == 0;
        let yy = y(step as f32);
        shape = shape.line(
            [0.0, yy],
            [CHART_VB, yy],
            1.0,
            rune::LineCap::Butt,
            if division {
                Color::WHITE.alpha(0.09)
            } else {
                Color::WHITE.alpha(0.04)
            },
        );
        let badge = insight::step_badge(step);
        let current = badge == snap.rank.badge;
        let sparse_ok = !tight || (step - 1) % 2 == 0;
        if (!dense && sparse_ok) || division || current {
            labels = labels.child(
                text(insight::badge_label(badge, &snap.tier_names))
                    .font(if current { TNUM_MED } else { TNUM_SMALL })
                    .color(if current { u.ink } else { u.faint })
                    .absolute(0.0, yy - if current { 10.0 } else { 8.0 }),
            );
        }
    }
    // A step line: rank is discrete, so the line moves only when the badge does.
    let mut line: Vec<[f32; 2]> = Vec::with_capacity(n * 2);
    for (index, point) in points.iter().enumerate() {
        let yy = y(point.step as f32);
        if index > 0 {
            line.push([x(index), line.last().map(|p| p[1]).unwrap_or(yy)]);
        }
        line.push([x(index), yy]);
    }
    let mut area = line.clone();
    area.push([x(n - 1), CHART_H]);
    area.push([0.0, CHART_H]);
    let end = *line.last().expect("two or more points");
    shape = shape
        .fill(area, Color::WHITE.alpha(0.035))
        .stroke(line, 1.5, u.prose)
        .line([end[0] - 0.5, end[1]], [end[0], end[1]], 7.0, rune::LineCap::Round, u.ink);
    let first = points.first().map(|p| p.start_time).unwrap_or(0);
    let last = points.last().map(|p| p.start_time).unwrap_or(0);
    chart_frame(u, labels, canvas(shape).grow(1.0).min_w(0.0).h(CHART_H), first, last)
        .role("img")
        .label(format!(
            "Rank over {} ranked matches, now {}",
            n,
            insight::badge_label(snap.rank.badge, &snap.tier_names)
        ))
}

fn win_rate_chart(u: &Ui, rolling: &[(i64, f64)]) -> El {
    if rolling.len() < 2 {
        return chart_empty(
            u,
            "The rolling win rate needs more than 20 decided matches in the window.",
        );
    }
    let lo_v = rolling.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let hi_v = rolling.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let lo = ((lo_v.min(40.0) - 5.0) / 10.0).floor() * 10.0;
    let hi = ((hi_v.max(60.0) + 5.0) / 10.0).ceil() * 10.0;
    let (lo, hi) = (lo.max(0.0) as f32, hi.min(100.0) as f32);
    let pad = 10.0;
    let y = |v: f32| pad + (hi - v) / (hi - lo) * (CHART_H - 2.0 * pad);
    let n = rolling.len();
    let x = |index: usize| index as f32 / (n - 1) as f32 * CHART_VB;
    let mid = y(50.0);

    let mut shape = Shape::new().view_box(CHART_VB, CHART_H);
    let mut labels = el().w(AXIS_W).h(CHART_H);
    let mut tick = lo;
    while tick <= hi + 0.1 {
        let yy = y(tick);
        let even = (tick - 50.0).abs() < 0.1;
        shape = shape.line(
            [0.0, yy],
            [CHART_VB, yy],
            1.0,
            rune::LineCap::Butt,
            if even {
                Color::WHITE.alpha(0.14)
            } else {
                Color::WHITE.alpha(0.04)
            },
        );
        if ((tick - 50.0).abs() as i32) % 20 == 0 {
            labels = labels.child(
                text(format!("{tick:.0}%"))
                    .font(TNUM_SMALL)
                    .color(if even { u.muted } else { u.faint })
                    .absolute(0.0, yy - 8.0),
            );
        }
        tick += 10.0;
    }
    let line: Vec<[f32; 2]> = rolling
        .iter()
        .enumerate()
        .map(|(index, (_, value))| [x(index), y(*value as f32)])
        .collect();
    // Shade between the line and even: green above, red below, split at crossings.
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let above_a = a[1] <= mid;
        let above_b = b[1] <= mid;
        let tint = |above: bool| {
            if above {
                u.teal.alpha(0.16)
            } else {
                u.rose.alpha(0.16)
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
    let first = rolling.first().map(|p| p.0).unwrap_or(0);
    let last = rolling.last().map(|p| p.0).unwrap_or(0);
    let now = rolling.last().map(|p| p.1).unwrap_or(0.0);
    chart_frame(u, labels, canvas(shape).grow(1.0).min_w(0.0).h(CHART_H), first, last)
        .role("img")
        .label(crate::screens::settings::i18n::fill(tr("Rolling win rate, now {} percent"), &[&format!("{now:.0}")]))
}

fn chart_frame(u: &Ui, labels: El, plot: El, first: i64, last: i64) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(row().w_full().child(labels).child(plot))
        .child(
            row()
                .w_full()
                .pl(AXIS_W)
                .child(
                    text(insight::date_label(first))
                        .font(TNUM_SMALL)
                        .color(u.faint),
                )
                .child(spacer())
                .child(
                    text(insight::date_label(last))
                        .font(TNUM_SMALL)
                        .color(u.faint),
                ),
        )
}

fn chart_empty(u: &Ui, message: &str) -> El {
    col()
        .w_full()
        .h(CHART_H)
        .center()
        .border(1.0, u.hair)
        .radius(crate::ui::r(10.0))
        .child(text(tr(message)).font(BODY).color(u.muted))
}

/// The last 30 decided results as beads: a win stands up, a loss hangs down.
fn form_strip(u: &Ui, snap: &StatsSnapshot, d: &Derived) -> El {
    let results: Vec<bool> = snap
        .matches
        .iter()
        .filter_map(|item| item.win)
        .take(FORM_BEADS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let (bead, gap, h) = (6.0_f32, 3.0_f32, 24.0_f32);
    let base = 14.0;
    let mut shape = Shape::new();
    for (index, win) in results.iter().enumerate() {
        let x0 = index as f32 * (bead + gap);
        shape = if *win {
            shape.fill(rect(x0, 2.0, x0 + bead, base - 1.0), u.teal.alpha(0.85))
        } else {
            shape.fill(rect(x0, base + 1.0, x0 + bead, h - 2.0), u.rose.alpha(0.75))
        };
    }
    let width = results.len() as f32 * (bead + gap);
    let wins = results.iter().filter(|w| **w).count();
    let mut words = vec![crate::screens::settings::i18n::fill(
        tr("{} W {} L in your last {}"),
        &[
            &wins.to_string(),
            &(results.len() - wins).to_string(),
            &results.len().to_string(),
        ],
    )];
    if let Some(streak) = d.streak.filter(|streak| streak.length >= 2) {
        words.push(crate::screens::settings::i18n::fill(
            tr("{} {} in a row now"),
            &[
                &streak.length.to_string(),
                if streak.win { tr("wins") } else { tr("losses") },
            ],
        ));
    }
    row()
        .key("stats-form")
        .w_full()
        .gap(16.0)
        .center_y()
        .child(text(tr("Form")).font(SMALL).color(u.muted).w(AXIS_W - 16.0))
        .child(
            canvas(shape)
                .size(width.max(1.0), h)
                .role("img")
                .label(words[0].clone()),
        )
        .child(text(words.join("  ·  ")).font(TNUM_SMALL).color(u.faint))
}

