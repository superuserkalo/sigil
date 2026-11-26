//! Hero pool: a sortable table, win rate against the player's own rate and their rank tier.
//! A row opens the hero's page (`hero_page.rs`).

use deadlock_core::stats::{HeroPerformance, StatsSnapshot};
use rune::{Color, El, Shape, SurfaceFit, canvas, col, el, row, spacer, text};

use crate::app::State;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::icon_key;
use super::shared::*;
use super::HEROES_SHOWN;

// --- heroes ----------------------------------------------------------------

fn hero_chip(s: &State, u: &Ui, hero_id: u32, name: &str, size: f32) -> El {
    match s.thumbnails.get(&icon_key(hero_id)) {
        Some(pixels) => el()
            .key(format!("stats-chip-{hero_id}-{size}"))
            .size(size, size)
            .radius(crate::ui::r(4.0))
            .fill(Color::WHITE.alpha(0.04))
            .pixels_fit(pixels, SurfaceFit::Fit)
            .a11y_hidden(),
        None => el()
            .size(size, size)
            .radius(crate::ui::r(4.0))
            .fill(Color::WHITE.alpha(0.06))
            .center()
            .a11y_hidden()
            .child(
                text(name.chars().next().map(String::from).unwrap_or_default())
                    .font(SMALL)
                    .color(u.muted),
            ),
    }
}

fn hero_name(hero: &HeroPerformance) -> String {
    hero.hero_name
        .clone()
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(tr("Hero {}"), &[&hero.hero_id.to_string()]))
}

/// What the hero table sorts by. Every column sorts high to low, Last played newest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeroSort {
    #[default]
    Matches,
    WinRate,
    Gap,
    Kda,
    Spm,
    Accuracy,
    LastPlayed,
}

/// The least matches a hero needs to be listed: any, 5 or 20.
pub const MIN_CHOICES: [u64; 3] = [0, 5, 20];

const COL_MATCHES: f32 = 64.0;
const COL_BAR: f32 = 120.0;
const COL_RATE: f32 = 56.0;
const COL_GAP: f32 = 56.0;
const COL_KDA: f32 = 48.0;
const COL_SPM: f32 = 56.0;
const COL_ACC: f32 = 64.0;
const COL_LAST: f32 = 96.0;

/// The tier's win rate on a hero when the tier has 100 matches on it, else the player's own.
fn compare_of(snap: &StatsSnapshot, hero: &HeroPerformance) -> (Option<f64>, f64) {
    let peer = snap
        .peers
        .as_ref()
        .and_then(|peers| peers.hero(hero.hero_id))
        .filter(|bench| bench.matches >= 100)
        .map(|bench| bench.win_rate_percent);
    (peer, peer.unwrap_or(snap.career.win_rate_percent))
}

/// Heroes in the table's order, above the floor.
pub(super) fn sorted<'a>(snap: &'a StatsSnapshot, sort: HeroSort, min: u64) -> Vec<&'a HeroPerformance> {
    let mut rows: Vec<&HeroPerformance> = snap.heroes.iter().filter(|hero| hero.matches_played >= min.max(1)).collect();
    let key = |hero: &HeroPerformance| -> f64 {
        match sort {
            HeroSort::Matches => hero.matches_played as f64,
            HeroSort::WinRate => hero.win_rate_percent,
            HeroSort::Gap => hero.win_rate_percent - compare_of(snap, hero).1,
            HeroSort::Kda => hero.kda,
            HeroSort::Spm => hero.net_worth_per_minute,
            HeroSort::Accuracy => hero.accuracy,
            HeroSort::LastPlayed => hero.last_played_unix as f64,
        }
    };
    rows.sort_by(|a, b| key(b).total_cmp(&key(a)).then(b.matches_played.cmp(&a.matches_played)).then(a.hero_id.cmp(&b.hero_id)));
    rows
}

pub(super) fn heroes(s: &State, u: &Ui, snap: &StatsSnapshot) -> El {
    use crate::screens::settings::i18n::fill;
    let tier = tier_label(snap);
    let sort = s.stats.hero_sort;
    let min = s.stats.hero_min;
    let rows = sorted(snap, sort, min);
    let hidden_under = snap.heroes.len() - rows.len();
    let floor = crate::ui::Segmented::new("stats-heroes-min", tr("Least matches"))
        .size(crate::ui::SegSize::M)
        .parts(MIN_CHOICES.iter().map(|n| {
            let n = *n;
            let label = if n == 0 { tr("Any").to_string() } else { fill(tr("{} matches or more"), &[&n.to_string()]) };
            crate::ui::SegPart::new(format!("stats-heroes-min-{n}"), label).on(min == n).with(move |e| {
                e.on_press(move |st: &mut State| {
                    st.stats.hero_min = n;
                    st.stats.all_heroes = false;
                })
            })
        }))
        .build(u);
    let mut tools = row()
        .w_full()
        .h(28.0)
        .gap(12.0)
        .center_y()
        .child(floor)
        .child(spacer())
        .child(text(fill(tr("{} heroes"), &[&rows.len().to_string()])).font(TNUM_SMALL).color(u.muted));
    if hidden_under > 0 {
        tools = tools.child(text(fill(tr("{} under the floor"), &[&hidden_under.to_string()])).font(TNUM_SMALL).color(u.muted));
    }
    let mut table = col().key("stats-heroes").w_full().child(tools.mb(12.0));
    if snap.heroes.is_empty() {
        return table.child(text(tr("No hero rows for this account yet.")).font(BODY).color(u.muted));
    }
    table = table.child(head(u, sort, tier.is_some()));
    let shown = if s.stats.all_heroes { rows.len() } else { rows.len().min(HEROES_SHOWN) };
    let now = snap.generated_unix;
    for hero in rows.iter().take(shown) {
        let name = hero_name(hero);
        let thin = hero.matches_played < 10;
        let (peer, compare) = compare_of(snap, hero);
        let gap = hero.win_rate_percent - compare;
        // Colour the gap only when the comparison sits outside the hero's likely range.
        let clear = compare < hero.win_rate_low_percent || compare > hero.win_rate_high_percent;
        let gap_color = if thin || !clear {
            u.muted
        } else if gap > 0.0 {
            u.teal
        } else {
            u.rose
        };
        let ink = if thin { u.muted } else { u.ink };
        let hero_id = hero.hero_id;
        table = table.child(
            data_row(format!("hero-{hero_id}"), u, 40.0)
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(fill(tr("Open {}"), &[&name]))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut rune::Tx| super::nav::open_hero(st, tx, hero_id))
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(12.0)
                        .center_y()
                        .child(hero_chip(s, u, hero_id, &name, 24.0))
                        .child(text(name).font(BODY_MED).color(ink).truncate()),
                )
                .child(num(thousands(hero.matches_played), COL_MATCHES, u.prose))
                .child(win_bar(u, hero, peer, thin).w(COL_BAR))
                .child(num(format!("{:.1}%", hero.win_rate_percent), COL_RATE, if thin { u.muted } else { u.prose }))
                .child(num(if thin { tr("few").to_string() } else { format!("{gap:+.0}") }, COL_GAP, gap_color))
                .child(num(format!("{:.2}", hero.kda), COL_KDA, u.muted))
                .child(num(thousands(hero.net_worth_per_minute.round() as u64), COL_SPM, u.muted))
                .child(num(if hero.accuracy > 0.0 { format!("{:.0}%", hero.accuracy * 100.0) } else { "-".to_string() }, COL_ACC, u.muted))
                .child(num(if hero.last_played_unix > 0 { deadlock_core::stats::insight::day_label(hero.last_played_unix, now) } else { "-".to_string() }, COL_LAST, u.muted))
                .child(el().w(16.0).center().child(rune::icon(&crate::icons::CHEVRON_RIGHT, 14.0).color(u.muted).a11y_hidden())),
        );
    }
    let hidden = rows.len().saturating_sub(HEROES_SHOWN);
    let legend = row()
        .gap(16.0)
        .center_y()
        .child(legend_swatch(u, Color::WHITE.alpha(0.55), "Your win rate", 4.0))
        .child(legend_swatch(u, Color::WHITE.alpha(0.35), "Likely range", 1.0))
        .child(legend_tick(
            u,
            &match &tier {
                Some(tier) => fill(tr("{} players on the hero"), &[tier]),
                None => "Your overall win rate".to_string(),
            },
        ));
    let mut foot = row().w_full().h(40.0).gap(12.0).center_y().child(legend).child(spacer());
    if hidden > 0 {
        let label = if s.stats.all_heroes { tr("Show fewer").to_string() } else { fill(tr("Show all {}"), &[&rows.len().to_string()]) };
        foot = foot.child(
            u.action(&label, ActionKind::Ghost)
                .key("stats-heroes-toggle")
                .mr(-12.0)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.stats.all_heroes = !st.stats.all_heroes),
        );
    }
    table.child(foot)
}

/// The table head: every number column sorts by itself on a press, the sorted one in ink with
/// a chevron in a slot every column keeps.
fn head(u: &Ui, sort: HeroSort, tier: bool) -> El {
    let mut r = row()
        .w_full()
        .h(28.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(cell(text(tr("Hero")).font(SMALL).color(u.muted), 0.0));
    let sortable = |label: &'static str, w: f32, key: HeroSort| {
        let on = sort == key;
        row()
            .key(format!("stats-heroes-sort-{key:?}"))
            .w(w)
            .h(28.0)
            .gap(2.0)
            .center_y()
            .focusable(true)
            .role("button")
            .label(crate::screens::settings::i18n::fill(tr("Sort by {}"), &[tr(label)]))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.stats.hero_sort = key)
            .child(spacer())
            // The chevron sits left of the label, so the label ends on the column's right edge
            // like the numbers under it.
            .child(rune::icon(&crate::icons::CHEVRON_DOWN, 10.0).color(if on { u.ink } else { Color::TRANSPARENT }).a11y_hidden())
            .child(text(tr(label)).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden())
    };
    r = r
        .child(sortable("Matches", COL_MATCHES, HeroSort::Matches))
        .child(row().w(COL_BAR))
        .child(sortable("Win rate", COL_RATE, HeroSort::WinRate))
        .child(sortable(if tier { "vs rank" } else { "vs you" }, COL_GAP, HeroSort::Gap))
        .child(sortable("KDA", COL_KDA, HeroSort::Kda))
        .child(sortable("SPM", COL_SPM, HeroSort::Spm))
        .child(sortable("Accuracy", COL_ACC, HeroSort::Accuracy))
        .child(sortable("Last played", COL_LAST, HeroSort::LastPlayed))
        .child(row().w(16.0));
    r
}

/// Win rate on a 0 to 100 track: the bar is the rate, the whisker under it the
/// 95 percent Wilson range, the marker above it the peer rate, the faint line even.
pub(super) fn win_bar(u: &Ui, hero: &HeroPerformance, peer: Option<f64>, thin: bool) -> El {
    let w = 100.0;
    let h = 20.0;
    let bar = 9.0;
    let rate = hero.win_rate_percent.clamp(0.0, 100.0) as f32;
    let low = hero.win_rate_low_percent.clamp(0.0, 100.0) as f32;
    let high = hero.win_rate_high_percent.clamp(0.0, 100.0) as f32;
    let whisker = bar + 6.0;
    let mut shape = Shape::new()
        .view_box(w, h)
        .fill(rect(0.0, bar - 2.0, w, bar + 2.0), Color::WHITE.alpha(0.05))
        .fill(
            rect(0.0, bar - 2.0, rate, bar + 2.0),
            Color::WHITE.alpha(if thin { 0.22 } else { 0.55 }),
        )
        .line([50.0, bar - 5.0], [50.0, whisker + 2.0], 1.0, rune::LineCap::Butt, Color::WHITE.alpha(0.12))
        .line([low, whisker], [high, whisker], 1.0, rune::LineCap::Butt, Color::WHITE.alpha(0.35));
    if let Some(peer) = peer {
        let p = peer.clamp(0.0, 100.0) as f32;
        shape = shape.fill(vec![[p - 1.6, 0.5], [p + 1.6, 0.5], [p, 4.5]], u.amber);
    }
    canvas(shape).h(h).a11y_hidden()
}

fn legend_swatch(u: &Ui, color: Color, label: &str, thickness: f32) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(el().size(12.0, thickness).fill(color).a11y_hidden())
        .child(text(tr(label)).font(SMALL).color(u.muted))
}

fn legend_tick(u: &Ui, label: &str) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(
            canvas(Shape::new().fill(vec![[0.0, 0.0], [8.0, 0.0], [4.0, 6.0]], u.amber))
                .size(8.0, 6.0)
                .a11y_hidden(),
        )
        .child(text(tr(label)).font(SMALL).color(u.muted))
}

