//! A hero page: one player on one hero.
//!
//! A place in the Stats stack (`nav.rs`), opened from a Heroes row. The header numbers and the
//! match list come from the player's page (hero stats and match history, already loaded); the
//! matchups and the items are two analytics reads made when the page opens and kept 12 hours
//! (`deadlock_core::stats::hero_detail`), so a reopened hero costs nothing.

use deadlock_core::stats::hero_detail::Matchup;
use deadlock_core::stats::items::ItemStat;
use deadlock_core::stats::{HeroPerformance, MatchSummary, StatsSnapshot};
use rune::{El, Tx, col, el, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

/// Rows of the page's own lists.
const MATCH_ROWS: usize = 10;
const MATCHUP_ROWS: usize = 12;
const ITEM_ROWS: usize = 12;

/// What the page reads when it opens.
pub struct HeroDetail {
    pub account_id: u64,
    pub hero_id: u32,
    pub matchups: Result<Vec<Matchup>, String>,
    pub items: Result<super::more::ItemsLoad, String>,
}

pub struct HeroPage {
    pub detail: Fetched<HeroDetail>,
    job: Job<HeroDetail>,
}

impl Default for HeroPage {
    fn default() -> Self {
        HeroPage { detail: Fetched::Idle, job: Job::default() }
    }
}

/// Read the matchups and items for the open hero, unless they are already here.
pub(super) fn start(st: &mut State, account_id: u64, hero_id: u32) {
    if let Fetched::Ready(d) = &st.stats.hero_page.detail
        && d.account_id == account_id
        && d.hero_id == hero_id
    {
        return;
    }
    if !st.prefs.allow_stats {
        st.stats.hero_page.detail = Fetched::Failed(tr("Player stats are turned off in Settings, Privacy.").to_string());
        return;
    }
    st.stats.hero_page.detail = Fetched::Loading;
    if st.stats.hero_page.job.is_running() || super::nav::offline_build() {
        return;
    }
    let Ok(account) = u32::try_from(account_id) else { return };
    let filter = st.stats.more.filter(now());
    let wake = st.wake.clone();
    st.stats.hero_page.job.start(wake, move || {
        let matchups = deadlock_core::stats::hero_detail::matchups(account, hero_id, &filter).map_err(|e| e.to_string());
        let items = deadlock_core::stats::hero_detail::hero_items(account, hero_id, &filter)
            .map(|rows| {
                let catalog = crate::data::stats_client().fetch_item_catalog().unwrap_or_default();
                super::more::ItemsLoad::joined(rows, &catalog)
            })
            .map_err(|e| e.to_string());
        Ok(HeroDetail { account_id, hero_id, matchups, items })
    });
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Take a finished read; one for a hero no longer open is dropped and the open one read.
pub(super) fn poll(st: &mut State) -> bool {
    let Some(result) = st.stats.hero_page.job.try_take() else { return false };
    let open = st.stats.nav.hero_open();
    match result {
        Ok(detail) if Some((detail.account_id, detail.hero_id)) == open => st.stats.hero_page.detail = Fetched::Ready(detail),
        Ok(_) => {
            if let Some((account_id, hero_id)) = open {
                st.stats.hero_page.detail = Fetched::Idle;
                start(st, account_id, hero_id);
            }
        }
        Err(error) => st.stats.hero_page.detail = Fetched::Failed(error),
    }
    true
}

/// A hero's name from the page's rows, the history, or the hero list.
pub(super) fn hero_label(s: &State, snap: &StatsSnapshot, hero_id: u32) -> String {
    snap.heroes
        .iter()
        .find(|hero| hero.hero_id == hero_id)
        .and_then(|hero| hero.hero_name.clone())
        .or_else(|| snap.matches.iter().find(|item| item.hero_id == hero_id).and_then(|item| item.hero_name.clone()))
        .or_else(|| s.stats.w4.names.as_ref().and_then(|names| names.iter().find(|(id, _)| *id == hero_id).map(|(_, n)| n.clone())))
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(tr("Hero {}"), &[&hero_id.to_string()]))
}

pub(super) fn page(s: &State, u: &Ui) -> El {
    let Some((_, hero_id)) = s.stats.nav.hero_open() else { return col() };
    let Some(snap) = &s.stats.snapshot else {
        return col().child(row().h(36.0).gap(12.0).center_y().child(back_button(u))).child(text(tr("Loading")).font(BODY).color(u.muted));
    };
    let name = hero_label(s, snap, hero_id);
    let hero = snap.heroes.iter().find(|h| h.hero_id == hero_id);
    let on_hero: Vec<&MatchSummary> = snap.matches.iter().filter(|item| item.hero_id == hero_id).collect();
    let mut page = col().key("stats-hero-page").w_full().gap(16.0).transition(crate::motion::page_enter());
    page = page.child(
        row()
            .w_full()
            .h(40.0)
            .gap(12.0)
            .center_y()
            .child(back_button(u))
            .child(hero_chip(s, u, hero_id, &name, 32.0))
            .child(text(name.clone()).font(H1).color(u.ink))
            .child(text(crate::screens::settings::i18n::fill(tr("as {}"), &[&snap.profile.persona_name])).font(SMALL).color(u.muted))
            .child(spacer())
            .child(
                u.action(tr("All matches on this hero"), ActionKind::Secondary)
                    .key("stats-hero-matches")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        super::nav::back(st, tx);
                        super::matches::filter_hero(st, hero_id);
                    }),
            ),
    );
    page = page.child(band(u, snap, hero));
    page = page.child(
        row()
            .w_full()
            .gap(24.0)
            .flex_wrap()
            .items_start()
            .child(matchups(s, u, snap).w(0.0).grow(1.0).min_w(440.0))
            .child(items(s, u).w(0.0).grow(1.0).min_w(440.0)),
    );
    page.child(recent(s, u, snap, &on_hero))
}

/// Matches, win rate and range, KDA, souls a minute, accuracy, against the tier.
fn band(u: &Ui, snap: &StatsSnapshot, hero: Option<&HeroPerformance>) -> El {
    use crate::screens::settings::i18n::fill;
    let Some(hero) = hero else {
        return text(tr("No hero record for this player yet.")).font(BODY).color(u.muted);
    };
    let peer = snap.peers.as_ref().and_then(|p| p.hero(hero.hero_id)).filter(|b| b.matches >= 100);
    let cell = |label: &str, value: String, sub: String| {
        col()
            .min_w(0.0)
            .gap(2.0)
            .pl(12.0)
            .border_sides([0.0, 0.0, 0.0, 1.0], u.hair)
            .child(text(tr(label).to_string()).font(SMALL).color(u.muted))
            .child(text(value).font(TNUM_BIG).color(u.ink).truncate())
            .child(text(sub).font(TNUM_SMALL).color(u.muted).truncate())
    };
    el().key("stats-hero-band")
        .w_full()
        .h(88.0)
        .py(12.0)
        .pr(12.0)
        .equal_columns(5)
        .gap(8.0)
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair)
        .child(cell("Matches", thousands(hero.matches_played), format!("{} W {} L", hero.wins, hero.losses)))
        .child(cell(
            "Win rate",
            format!("{:.1}%", hero.win_rate_percent),
            fill(tr("Likely {} to {}"), &[&format!("{:.0}", hero.win_rate_low_percent), &format!("{:.0}%", hero.win_rate_high_percent)]),
        ))
        .child(cell(
            "Against the rank",
            peer.map(|b| format!("{:+.0}", hero.win_rate_percent - b.win_rate_percent)).unwrap_or_else(|| "-".to_string()),
            peer.map(|b| fill(tr("{} win on it at your rank"), &[&format!("{:.0}%", b.win_rate_percent)])).unwrap_or_default(),
        ))
        .child(cell("KDA", format!("{:.2}", hero.kda), fill(tr("{} deaths a match"), &[&format!("{:.1}", hero.deaths as f64 / hero.matches_played.max(1) as f64)])))
        .child(cell(
            "Souls a minute",
            thousands(hero.net_worth_per_minute.round() as u64),
            if hero.accuracy > 0.0 { fill(tr("{} accuracy"), &[&format!("{:.0}%", hero.accuracy * 100.0)]) } else { String::new() },
        ))
}

fn list_head(u: &Ui, title: &str, meta: &str) -> El {
    row()
        .w_full()
        .h(32.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(tr(title).to_string()).font(crate::ui::H2).color(u.ink))
        .child(text(tr(meta).to_string()).font(SMALL).color(u.muted))
}

/// Against each enemy hero: matches, the player's win rate, and the soul gap.
fn matchups(s: &State, u: &Ui, snap: &StatsSnapshot) -> El {
    let mut c = col().key("stats-hero-matchups").child(list_head(u, "Matchups", "against each enemy hero"));
    let rows = match &s.stats.hero_page.detail {
        Fetched::Ready(d) => match &d.matchups {
            Ok(rows) => rows,
            Err(error) => return c.child(problem(u, error).pt(8.0)),
        },
        Fetched::Failed(error) => return c.child(problem(u, error).pt(8.0)),
        _ => return c.child(text(tr("Loading matchups")).font(BODY).color(u.muted).pt(8.0)),
    };
    if rows.is_empty() {
        return c.child(text(tr("No enemy hero has two matches against this hero yet.")).font(BODY).color(u.muted).pt(8.0));
    }
    c = c.child(head_row(u, &[("Enemy hero", 0.0, false), ("Matches", 64.0, true), ("Your win rate", 96.0, true), ("Soul gap", 72.0, true)]));
    for m in rows.iter().take(MATCHUP_ROWS) {
        let name = hero_label(s, snap, m.enemy_hero_id);
        let rate = m.rate();
        c = c.child(
            data_row(format!("stats-matchup-{}", m.enemy_hero_id), u, 40.0)
                .child(row().grow(1.0).min_w(0.0).gap(10.0).center_y().child(hero_chip(s, u, m.enemy_hero_id, &name, 24.0)).child(text(name).font(BODY_MED).color(u.ink).truncate()))
                .child(num(m.matches.to_string(), 64.0, u.prose))
                .child(num(
                    rate.map(|r| format!("{r:.0}%")).unwrap_or_else(|| "-".to_string()),
                    96.0,
                    match rate {
                        Some(r) if r >= 60.0 => u.teal,
                        Some(r) if r <= 40.0 => u.rose,
                        _ => u.prose,
                    },
                ))
                .child(num(signed(m.soul_gap.round() as i64), 72.0, if m.soul_gap >= 0.0 { u.muted } else { u.rose })),
        );
    }
    c
}

/// The items bought most on this hero, with win rate and buy time.
fn items(s: &State, u: &Ui) -> El {
    let mut c = col().key("stats-hero-items").child(list_head(u, "Items", "bought most on this hero"));
    let load = match &s.stats.hero_page.detail {
        Fetched::Ready(d) => match &d.items {
            Ok(load) => load,
            Err(error) => return c.child(problem(u, error).pt(8.0)),
        },
        Fetched::Failed(error) => return c.child(problem(u, error).pt(8.0)),
        _ => return c.child(text(tr("Loading items")).font(BODY).color(u.muted).pt(8.0)),
    };
    let rows: Vec<&ItemStat> = load.rows.iter().take(ITEM_ROWS).collect();
    if rows.is_empty() {
        return c.child(text(tr("No item has three matches on this hero yet.")).font(BODY).color(u.muted).pt(8.0));
    }
    c = c.child(head_row(u, &[("Item", 0.0, false), ("Matches", 64.0, true), ("Win rate", 72.0, true), ("Bought at", 72.0, true)]));
    for item in rows {
        let name = load.names.get(&item.item_id).cloned().unwrap_or_else(|| format!("Item {}", item.item_id));
        let slot = load.slots.get(&item.item_id).map(String::as_str).unwrap_or("other");
        let secs = item.avg_buy_time_s.round().max(0.0) as u32;
        c = c.child(
            data_row(format!("stats-hero-item-{}", item.item_id), u, 40.0)
                .child(row().grow(1.0).min_w(0.0).gap(10.0).center_y().child(item_tile(s, u, item.item_id, &name, slot, 24.0, None)).child(text(name).font(BODY_MED).color(u.ink).truncate()))
                .child(num(item.matches.to_string(), 64.0, u.prose))
                .child(num(format!("{:.0}%", item.win_rate_percent), 72.0, u.prose))
                .child(num(super::notes::clock(secs), 72.0, u.muted)),
        );
    }
    c
}

/// The newest matches on this hero in the loaded history; a row opens the match page.
fn recent(s: &State, u: &Ui, snap: &StatsSnapshot, rows: &[&MatchSummary]) -> El {
    let meta = crate::screens::settings::i18n::fill(tr("{} in the loaded history"), &[&rows.len().to_string()]);
    let mut c = col().key("stats-hero-recent").w_full().child(list_head(u, "Recent matches", &meta));
    if rows.is_empty() {
        return c.child(text(tr("No match on this hero in the loaded history.")).font(BODY).color(u.muted).pt(8.0));
    }
    c = c.child(super::matches::columns_head(u));
    for item in rows.iter().take(MATCH_ROWS) {
        c = c.child(super::matches::match_row(s, u, item));
    }
    let _ = snap;
    c
}
