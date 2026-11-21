//! Hero pool: win rate against the player's own rate and their rank tier.

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

pub(super) fn heroes(s: &State, u: &Ui, snap: &StatsSnapshot) -> El {
    let tier = tier_label(snap);
    let peers = snap.peers.as_ref();
    // The tab names the table; the head only counts.
    let mut table = col().key("stats-heroes").w_full().child(section_head(
        u,
        &crate::screens::settings::i18n::fill(tr("{} heroes"), &[&snap.heroes.len().to_string()]),
        "",
    ));
    if snap.heroes.is_empty() {
        return table.child(text(tr("No hero rows for this account yet.")).font(BODY).color(u.muted));
    }
    let career = snap.career.win_rate_percent;
    table = table.child(head_row(
        u,
        &[
            ("Hero", 0.0, false),
            ("", 200.0, false),
            ("Win rate", 64.0, true),
            (
                if tier.is_some() { "vs rank" } else { "vs you" },
                56.0,
                true,
            ),
            ("KDA", 48.0, true),
            ("Matches", 64.0, true),
        ],
    ));
    let shown = if s.stats.all_heroes {
        snap.heroes.len()
    } else {
        HEROES_SHOWN
    };
    for hero in snap.heroes.iter().take(shown) {
        let name = hero_name(hero);
        let thin = hero.matches_played < 10;
        let peer = peers
            .and_then(|peers| peers.hero(hero.hero_id))
            .filter(|bench| bench.matches >= 100)
            .map(|bench| bench.win_rate_percent);
        let compare = peer.unwrap_or(career);
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
        table = table.child(
            data_row(format!("hero-{}", hero.hero_id), u, 40.0)
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(12.0)
                        .center_y()
                        .child(hero_chip(s, u, hero.hero_id, &name, 24.0))
                        .child(text(name).font(BODY_MED).color(ink).truncate()),
                )
                .child(win_bar(u, hero, peer, thin).w(200.0))
                .child(num(
                    format!("{:.1}%", hero.win_rate_percent),
                    64.0,
                    if thin { u.muted } else { u.prose },
                ))
                .child(num(
                    if thin {
                        "few".to_string()
                    } else {
                        format!("{gap:+.0}")
                    },
                    56.0,
                    gap_color,
                ))
                .child(num(format!("{:.2}", hero.kda), 48.0, u.muted))
                .child(num(thousands(hero.matches_played), 64.0, u.muted)),
        );
    }
    let hidden = snap.heroes.len().saturating_sub(HEROES_SHOWN);
    let legend = row()
        .gap(16.0)
        .center_y()
        .child(legend_swatch(u, Color::WHITE.alpha(0.55), "Your win rate", 4.0))
        .child(legend_swatch(u, Color::WHITE.alpha(0.35), "Likely range", 1.0))
        .child(legend_tick(
            u,
            &match &tier {
                Some(tier) => crate::screens::settings::i18n::fill(tr("{} players on the hero"), &[tier]),
                None => "Your overall win rate".to_string(),
            },
        ));
    let mut foot = row().w_full().h(40.0).gap(12.0).center_y().child(legend).child(spacer());
    if hidden > 0 {
        let label = if s.stats.all_heroes {
            "Show fewer".to_string()
        } else {
            crate::screens::settings::i18n::fill(tr("Show all {}"), &[&snap.heroes.len().to_string()])
        };
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

/// Win rate on a 0 to 100 track: the bar is the rate, the whisker under it the
/// 95 percent Wilson range, the marker above it the peer rate, the faint line even.
fn win_bar(u: &Ui, hero: &HeroPerformance, peer: Option<f64>, thin: bool) -> El {
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

