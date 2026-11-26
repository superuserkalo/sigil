//! "At your rank": where the player's last 20 sit among the matches played at their rank tier.
//!
//! One `/v1/analytics/player-stats/metrics` read per tier, kept 24 hours
//! (`deadlock_core::stats::bracket`). The distribution is of single matches, so every line says
//! "of matches at <tier>", never "of players".

use std::collections::HashMap;

use deadlock_core::stats::bracket::Distribution;
use deadlock_core::stats::{MatchSummary, StatsSnapshot};
use rune::{Color, El, Shape, canvas, col, el, row, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

/// Matches the player's averages are taken over.
const RECENT: usize = 20;
const RULER_W: f32 = 1000.0;

pub struct Bracket {
    /// The tier read, and its distributions.
    pub data: Fetched<(u32, HashMap<String, Distribution>)>,
    job: Job<(u32, Option<HashMap<String, Distribution>>)>,
}

impl Default for Bracket {
    fn default() -> Self {
        Bracket { data: Fetched::Idle, job: Job::default() }
    }
}

const FIXTURE: &str = include_str!("../../../fixtures/stats-bracket.json");

/// Scenes: the captured Eternus distributions.
pub fn load_fixture(st: &mut State, badge: u32) {
    if let Ok(map) = deadlock_core::stats::bracket::parse_metrics(FIXTURE) {
        st.stats.bracket.data = Fetched::Ready((badge / 10, map));
    }
}

/// Read the subject's tier when Overview shows and it is not here yet. Called from the poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.stats.bracket.job.try_take() {
        st.stats.bracket.data = match result {
            Ok((tier, Some(map))) => Fetched::Ready((tier, map)),
            Ok((_, None)) => Fetched::Idle,
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    let Some(badge) = st.stats.snapshot.as_ref().map(|snap| snap.rank.badge).filter(|b| *b >= 10) else { return changed };
    let tier = badge / 10;
    let have = match &st.stats.bracket.data {
        Fetched::Ready((t, _)) => *t == tier,
        Fetched::Loading | Fetched::Failed(_) => true,
        Fetched::Idle => false,
    };
    if have || st.stats.tab != super::Tab::Overview || !st.prefs.allow_stats || st.stats.bracket.job.is_running() || super::nav::offline_build() {
        return changed;
    }
    st.stats.bracket.data = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.bracket.job.start(wake, move || {
        deadlock_core::stats::bracket::tier_metrics(badge).map(|map| (tier, map)).map_err(|e| e.to_string())
    });
    true
}

/// A metric: label, the API's name, the player's value, whether lower is better, and the
/// number's format.
struct Line {
    label: &'static str,
    metric: &'static str,
    value: f64,
    lower_better: bool,
    shown: String,
}

fn lines(recent: &[&MatchSummary]) -> Vec<Line> {
    let n = recent.len().max(1) as f64;
    let avg = |f: &dyn Fn(&MatchSummary) -> f64| recent.iter().map(|m| f(m)).sum::<f64>() / n;
    let kills = avg(&|m| m.kills as f64);
    let deaths = avg(&|m| m.deaths as f64);
    let assists = avg(&|m| m.assists as f64);
    let kda = avg(&|m| m.kda);
    let spm = avg(&|m| spm(m) as f64);
    let last_hits = avg(&|m| m.last_hits as f64);
    vec![
        Line { label: "Kills", metric: "kills", value: kills, lower_better: false, shown: format!("{kills:.1}") },
        Line { label: "Deaths", metric: "deaths", value: deaths, lower_better: true, shown: format!("{deaths:.1}") },
        Line { label: "Assists", metric: "assists", value: assists, lower_better: false, shown: format!("{assists:.1}") },
        Line { label: "KDA", metric: "kda", value: kda, lower_better: false, shown: format!("{kda:.2}") },
        Line { label: "Souls a minute", metric: "net_worth_per_min", value: spm, lower_better: false, shown: thousands(spm.round() as u64) },
        Line { label: "Last hits", metric: "last_hits", value: last_hits, lower_better: false, shown: format!("{last_hits:.0}") },
    ]
}

/// The section, or nothing for an unranked player or before the read lands.
pub(super) fn section(s: &State, u: &Ui, snap: &StatsSnapshot) -> Option<El> {
    use crate::screens::settings::i18n::fill;
    let tier = tier_label(snap).or_else(|| snap.tier_names.get(&(snap.rank.badge / 10)).cloned())?;
    let map = match &s.stats.bracket.data {
        Fetched::Ready((t, map)) if *t == snap.rank.badge / 10 => map,
        Fetched::Loading => return Some(col().w_full().child(section_head(u, "At your rank", "")).child(text(tr("Loading the matches at your rank")).font(SMALL).color(u.muted))),
        _ => return None,
    };
    let recent: Vec<&MatchSummary> = snap.matches.iter().filter(|m| !m.is_street_brawl()).take(RECENT).collect();
    if recent.is_empty() {
        return None;
    }
    let meta = fill(tr("the last {} matches against single matches at {}, last 30 days"), &[&recent.len().to_string(), &tier]);
    let mut cells = Vec::new();
    for line in lines(&recent) {
        let Some(dist) = map.get(line.metric) else { continue };
        let p = dist.percentile_of(line.value);
        let better = if line.lower_better { 100.0 - p } else { p };
        let words = if line.lower_better {
            fill(tr("fewer than {} of matches at {}"), &[&format!("{:.0}%", better), &tier])
        } else {
            fill(tr("beats {} of matches at {}"), &[&format!("{:.0}%", better), &tier])
        };
        cells.push(
            col()
                .min_w(0.0)
                .padding(12.0)
                .gap(6.0)
                .radius(crate::ui::r(4.0))
                .fill(u.raised)
                .border(1.0, u.hair)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(text(tr(line.label).to_string()).font(SMALL).color(u.muted).truncate().min_w(0.0).shrink())
                        .child(el().w(0.0).grow(1.0))
                        .child(text(fill(tr("median {}"), &[&median(dist, line.metric)])).font(TNUM_SMALL).color(u.muted)),
                )
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .center_y()
                        .child(text(line.shown).font(TNUM_MED).color(u.ink).w(56.0))
                        .child(row().w(0.0).grow(1.0).min_w(0.0).child(ruler(u, better))),
                )
                .child(text(words).font(TNUM_SMALL).color(if better >= 60.0 { u.teal } else if better <= 40.0 { u.rose } else { u.prose }).truncate()),
        );
    }
    // Two rows of three, equal columns, so the rulers line up.
    let mut grid = col().key("stats-bracket").w_full().gap(8.0);
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        let mut line = el().w_full().equal_columns(3).gap(8.0);
        for i in 0..3 {
            match cells.next() {
                Some(cell) => line = line.child(cell.grid_cell(i)),
                None => break,
            }
        }
        grid = grid.child(line);
    }
    Some(col().w_full().child(section_head(u, "At your rank", &meta)).child(grid))
}

fn median(d: &Distribution, metric: &str) -> String {
    match metric {
        "kda" => format!("{:.2}", d.percentile50),
        "net_worth_per_min" => thousands(d.percentile50.round() as u64),
        _ => format!("{:.0}", d.percentile50),
    }
}

/// A 0 to 100 track with quarter ticks and a marker at the player's place (right is better).
fn ruler(u: &Ui, better: f64) -> El {
    let x = (better.clamp(0.0, 100.0) / 100.0) as f32 * RULER_W;
    let mut shape = Shape::new()
        .view_box(RULER_W, 14.0)
        .fill(rect(0.0, 6.0, RULER_W, 8.0), Color::WHITE.alpha(0.08))
        .fill(rect(0.0, 6.0, x, 8.0), if better >= 50.0 { u.teal.alpha(0.6) } else { u.rose.alpha(0.5) });
    for q in [0.25_f32, 0.5, 0.75] {
        shape = shape.line([q * RULER_W, 3.0], [q * RULER_W, 11.0], 1.0, rune::LineCap::Butt, Color::WHITE.alpha(if q == 0.5 { 0.3 } else { 0.15 }));
    }
    shape = shape.fill(rect(x - 3.0, 0.0, x + 3.0, 14.0), u.ink);
    canvas(shape).w_full().h(14.0).a11y_hidden()
}
