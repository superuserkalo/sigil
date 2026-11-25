//! Stats additions from parity wave 4 (UI B) over `deadlock_core::stats` (C1):
//!
//! - The ranked and date filter now scopes the overview and Heroes too (DMM scopes hero and
//!   player stats; Grimoire has no such filter). With a filter set, a filtered snapshot is read
//!   on a worker and those sections draw from it; Matches keeps its own client-side filter.
//! - Party lineups picked by hand (DMM's planner): up to six members, each offering up to three
//!   heroes, rated with `stats::party::rate_lineups`.
//! - Item meta: everyone's best items in the filter's window (DMM's "Item meta" widget; 30 days
//!   when no window is set, as DMM's default). Grimoire has the endpoint but no screen.

use std::collections::HashMap;

use deadlock_core::stats::StatsSnapshot;
use deadlock_core::stats::filters::StatsFilter;
use deadlock_core::stats::party::LineupRating;
use rune::{Color, El, col, row, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::Fetched;
use super::german::tr;
use super::more::ItemsLoad;
use super::shared::*;

/// DMM: a party is up to six players, each offering up to three heroes.
pub const MAX_MEMBERS: usize = 6;
pub const MAX_PICKS: usize = 3;
/// The same snapshot window the page loads with.
const WINDOW: usize = 200;
/// DMM's item meta reads the last 30 days when no window is chosen.
const META_DAYS: u64 = 30;
/// Item meta rows shown.
const META_ROWS: usize = 15;

pub struct W4 {
    pub scoped: Fetched<StatsSnapshot>,
    pub scoped_for: Option<StatsFilter>,
    pub scoped_derived: Option<super::Derived>,
    pub scoped_job: Job<(StatsFilter, StatsSnapshot)>,
    /// Party picked by hand: member 0 is the player.
    pub hand: bool,
    pub picks: Vec<Vec<u32>>,
    /// The member the hero grid adds to.
    pub picking: Option<usize>,
    pub names: Option<Vec<(u32, String)>>,
    pub names_job: Job<HashMap<u32, String>>,
    pub hand_party: Fetched<Vec<LineupRating>>,
    pub hand_job: Job<Vec<LineupRating>>,
    pub meta: Fetched<ItemsLoad>,
    pub meta_job: Job<(ItemsLoad, Option<String>)>,
    /// Item meta reads since the newest patch (true) or the filter's window.
    pub meta_patch: bool,
    /// The patch day the meta was read from, `YYYY-MM-DD`.
    pub meta_since: Option<String>,
}

impl Default for W4 {
    fn default() -> Self {
        W4 {
            scoped: Fetched::Idle,
            scoped_for: None,
            scoped_derived: None,
            scoped_job: Job::default(),
            hand: false,
            picks: vec![Vec::new(), Vec::new()],
            picking: None,
            names: None,
            names_job: Job::default(),
            hand_party: Fetched::Idle,
            hand_job: Job::default(),
            meta: Fetched::Idle,
            meta_job: Job::default(),
            meta_patch: true,
            meta_since: None,
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A filter that narrows anything at all.
pub fn narrows(filter: &StatsFilter) -> bool {
    filter.ranked.is_some() || filter.since.is_some() || filter.until.is_some()
}

/// The snapshot and derived values the overview and Heroes draw: the filtered copy when it is
/// ready for the current filter, else the page's own.
pub fn overview(s: &State) -> Option<(&StatsSnapshot, &super::Derived)> {
    let w = &s.stats.w4;
    if !narrows(&s.stats.more.filter(now())) {
        return None;
    }
    match (&w.scoped, &w.scoped_derived) {
        (Fetched::Ready(snap), Some(derived)) => Some((snap, derived)),
        _ => None,
    }
}

/// The filter changed: read the filtered snapshot, or drop it for "everything".
pub fn refilter(st: &mut State) {
    let filter = st.stats.more.filter(now());
    st.stats.w4.hand_party = Fetched::Idle;
    st.stats.w4.meta = Fetched::Idle;
    if !narrows(&filter) {
        st.stats.w4.scoped = Fetched::Idle;
        st.stats.w4.scoped_for = None;
        st.stats.w4.scoped_derived = None;
        return;
    }
    let Some(account) = st.stats.snapshot.as_ref().map(|snap| snap.account_id) else {
        return;
    };
    if !st.prefs.allow_stats || st.stats.w4.scoped_job.is_running() {
        return;
    }
    if st.demo || rune::is_headless() {
        // Scenes: the page's own snapshot, matches cut by the filter.
        if let Some(mut snap) = st.stats.snapshot.clone() {
            snap.matches = deadlock_core::stats::filters::filter_matches(&snap.matches, &filter);
            set_scoped(st, filter, snap);
        }
        return;
    }
    st.stats.w4.scoped = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.w4.scoped_job.start(wake, move || {
        crate::data::stats_client()
            .fetch_snapshot_filtered(account, WINDOW, &filter)
            .map(|snap| (filter, snap))
            .map_err(|e| e.to_string())
    });
}

fn set_scoped(st: &mut State, filter: StatsFilter, snap: StatsSnapshot) {
    st.stats.w4.scoped_derived = Some(super::derive(&snap));
    st.stats.w4.scoped_for = Some(filter);
    st.stats.w4.scoped = Fetched::Ready(snap);
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.stats.w4.scoped_job.try_take() {
        match result {
            // A read for an older filter is dropped; the newest one runs next.
            Ok((filter, snap)) if filter.ranked == st.stats.more.ranked => set_scoped(st, filter, snap),
            Ok(_) => refilter(st),
            Err(error) => st.stats.w4.scoped = Fetched::Failed(error),
        }
        changed = true;
    }
    if let Some(result) = st.stats.w4.names_job.try_take() {
        if let Ok(names) = result {
            let mut list: Vec<(u32, String)> = names.into_iter().collect();
            list.sort_by(|a, b| a.1.cmp(&b.1));
            st.stats.w4.names = Some(list);
        }
        changed = true;
    }
    if let Some(result) = st.stats.w4.hand_job.try_take() {
        st.stats.w4.hand_party = match result {
            Ok(rows) => Fetched::Ready(rows),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = st.stats.w4.meta_job.try_take() {
        st.stats.w4.meta = match result {
            Ok((load, since)) => {
                st.stats.w4.meta_since = since;
                Fetched::Ready(load)
            }
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    changed
}

/// The line under the filter bar saying what the filter covers now.
pub fn scope_note(s: &State, u: &Ui) -> Option<El> {
    if !narrows(&s.stats.more.filter(now())) {
        return None;
    }
    let (words, color) = match &s.stats.w4.scoped {
        Fetched::Loading => (tr("Reading your stats for this filter").to_string(), u.muted),
        Fetched::Failed(error) => (crate::screens::settings::i18n::fill(tr("The filtered stats did not load: {}"), &[error]), u.rose),
        Fetched::Ready(_) => (tr("Overview and Heroes show this filter.").to_string(), u.muted),
        Fetched::Idle => return None,
    };
    Some(text(words).key("stats-scope-note").font(SMALL).color(color))
}

// --- party by hand --------------------------------------------------------------------------

fn hero_names(s: &State) -> Vec<(u32, String)> {
    if let Some(names) = &s.stats.w4.names {
        return names.clone();
    }
    let mut list: Vec<(u32, String)> = s
        .stats
        .snapshot
        .as_ref()
        .map(|snap| snap.heroes.iter().map(|h| (h.hero_id, hero_name(h))).collect())
        .unwrap_or_default();
    list.sort_by(|a, b| a.1.cmp(&b.1));
    list
}

fn name_of(s: &State, hero: u32) -> String {
    hero_names(s)
        .into_iter()
        .find(|(id, _)| *id == hero)
        .map(|(_, name)| name)
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(tr("Hero {}"), &[&hero.to_string()]))
}

/// Add or take out a hero from a member's picks, three at most.
pub fn toggle_pick(picks: &mut [Vec<u32>], member: usize, hero: u32) {
    let Some(list) = picks.get_mut(member) else { return };
    if let Some(at) = list.iter().position(|h| *h == hero) {
        list.remove(at);
    } else if list.len() < MAX_PICKS {
        list.push(hero);
    }
}

fn start_names(st: &mut State) {
    if st.stats.w4.names.is_some() || st.stats.w4.names_job.is_running() || st.demo || rune::is_headless() {
        return;
    }
    let wake = st.wake.clone();
    st.stats.w4.names_job.start(wake, || crate::data::stats_client().fetch_hero_names().map_err(|e| e.to_string()));
}

fn rate_hand(st: &mut State) {
    if st.stats.w4.hand_job.is_running() {
        return;
    }
    let picks: Vec<Vec<u32>> = st.stats.w4.picks.iter().filter(|p| !p.is_empty()).cloned().collect();
    if picks.len() < 2 {
        st.stats.w4.hand_party = Fetched::Failed(tr("Give at least two players a hero.").to_string());
        return;
    }
    if !st.prefs.allow_stats {
        st.stats.w4.hand_party = Fetched::Failed(tr("Player stats are turned off in Settings, Privacy.").to_string());
        return;
    }
    let filter = st.stats.more.filter(now());
    st.stats.w4.hand_party = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.w4.hand_job.start(wake, move || {
        deadlock_core::stats::party::rate_lineups(&picks, &filter).map_err(|e| e.to_string())
    });
}

/// The switch between Party's two ways: from members' most played heroes, or by hand.
pub fn party_mode(s: &State, u: &Ui) -> El {
    let hand = s.stats.w4.hand;
    let pill = |key: &str, label: &str, on: bool, set: bool| {
        crate::ui::SegPart::new(key, tr(label)).on(on).with(move |e| {
            e.on_press(move |st: &mut State| {
                st.stats.w4.hand = set;
                if set {
                    start_names(st);
                }
            })
        })
    };
    crate::ui::Segmented::new("stats-party-mode", tr("Party heroes"))
        .size(crate::ui::SegSize::M)
        .part(pill("stats-party-mode-auto", "Most played heroes", !hand, false))
        .part(pill("stats-party-mode-hand", "Pick heroes by hand", hand, true))
        .build(u)
        .self_start()
}

/// DMM's planner: a row per player with up to three heroes, Add player, a hero grid for the
/// highlighted player, then the rated lineups.
pub fn hand_party(s: &State, u: &Ui) -> El {
    let w = &s.stats.w4;
    let mut c = col().key("stats-party-hand").w_full().gap(10.0).child(
        text(tr("Each player offers up to 3 heroes, as in queue. Press a player, then heroes below."))
            .font(SMALL)
            .color(u.muted)
            .wrap(),
    );
    for (member, picks) in w.picks.iter().enumerate() {
        let on = w.picking == Some(member);
        let label = if member == 0 {
            tr("You").to_string()
        } else {
            crate::screens::settings::i18n::fill(tr("Player {}"), &[&(member + 1).to_string()])
        };
        let mut r = row()
            .key(format!("stats-party-hand-{member}"))
            .w_full()
            .min_h(40.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .border(1.0, if on { u.accent } else { u.hair })
            .fill(if on { u.accent.alpha(0.06) } else { Color::TRANSPARENT })
            .focusable(true)
            .role("button")
            .checked(on)
            .label(label.clone())
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.stats.w4.picking = if st.stats.w4.picking == Some(member) { None } else { Some(member) };
            })
            .child(text(label).font(BODY_MED).color(u.ink).w(80.0));
        for hero in picks {
            let name = name_of(s, *hero);
            let hero = *hero;
            r = r.child(
                row()
                    .key(format!("stats-party-hand-{member}-{hero}"))
                    .h(28.0)
                    .pl(2.0)
                    .pr(8.0)
                    .gap(6.0)
                    .center_y()
                    .radius(crate::ui::r(14.0))
                    .border(1.0, u.hair_strong)
                    .hover(u.hover)
                    .role("button")
                    .label(crate::screens::settings::i18n::fill(tr("Take out {}"), &[&name]))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        toggle_pick(&mut st.stats.w4.picks, member, hero);
                        st.stats.w4.hand_party = Fetched::Idle;
                    })
                    .child(hero_chip(s, u, hero, &name, 22.0))
                    .child(text(name).font(SMALL).color(u.prose)),
            );
        }
        if picks.is_empty() {
            r = r.child(text(tr("No heroes yet")).font(SMALL).color(u.muted));
        }
        if member >= 2 {
            r = r.child(rune::spacer()).child(
                u.icon_button(&crate::icons::X, tr("Remove player"))
                    .key(format!("stats-party-hand-remove-{member}"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if member < st.stats.w4.picks.len() {
                            st.stats.w4.picks.remove(member);
                        }
                        st.stats.w4.picking = None;
                        st.stats.w4.hand_party = Fetched::Idle;
                    }),
            );
        }
        c = c.child(r);
    }
    let mut actions = row().gap(8.0).center_y();
    if w.picks.len() < MAX_MEMBERS {
        actions = actions.child(
            u.action(tr("Add player"), ActionKind::Ghost)
                .key("stats-party-hand-add")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.stats.w4.picks.push(Vec::new());
                    st.stats.w4.picking = Some(st.stats.w4.picks.len() - 1);
                }),
        );
    }
    actions = actions.child(
        u.action(tr("Rate lineups"), ActionKind::Secondary)
            .key("stats-party-hand-rate")
            .disabled(w.picks.iter().filter(|p| !p.is_empty()).count() < 2)
            .cursor(rune::Cursor::Pointer)
            .on_press(rate_hand),
    );
    c = c.child(actions);
    if let Some(member) = w.picking {
        let picked = w.picks.get(member).cloned().unwrap_or_default();
        let mut grid = row().key("stats-party-hand-grid").w_full().gap(6.0).flex_wrap();
        for (hero, name) in hero_names(s) {
            let on = picked.contains(&hero);
            grid = grid.child(
                row()
                    .key(format!("stats-party-hero-{hero}"))
                    .h(28.0)
                    .pl(3.0)
                    .pr(10.0)
                    .gap(6.0)
                    .center_y()
                    .radius(crate::ui::r(15.0))
                    .border(1.0, if on { u.accent } else { u.hair })
                    .fill(if on { u.accent.alpha(0.10) } else { Color::TRANSPARENT })
                    .hover(u.hover)
                    .focusable(true)
                    .role("checkbox")
                    .checked(on)
                    .label(name.clone())
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        toggle_pick(&mut st.stats.w4.picks, member, hero);
                        st.stats.w4.hand_party = Fetched::Idle;
                    })
                    .child(hero_chip(s, u, hero, &name, 24.0))
                    .child(text(name).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden()),
            );
        }
        c = c.child(grid);
    }
    c.child(super::more::lineup_table(s, u, &w.hand_party, true))
}

// --- item meta ------------------------------------------------------------------------------

fn start_meta(st: &mut State) {
    if st.stats.w4.meta_job.is_running() {
        return;
    }
    if !st.prefs.allow_stats {
        st.stats.w4.meta = Fetched::Failed(tr("Player stats are turned off in Settings, Privacy.").to_string());
        return;
    }
    let base = st.stats.more.filter(now());
    let patch = st.stats.w4.meta_patch;
    let workspace = crate::data::workspace_dir();
    st.stats.w4.meta = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.w4.meta_job.start(wake, move || {
        // "This patch": from the newest patch in the Steam feed Sigil caches (fetched once when
        // the cache is empty); else the filter's window, 30 days when none is set.
        let (filter, since) = if patch {
            let path = deadlock_core::patch_notes::patch_note_cache_path(&workspace);
            let mut notes = deadlock_core::patch_notes::load_patch_note_cache(&path).map(|c| c.notes).unwrap_or_default();
            if !notes.iter().any(|n| n.is_patch) {
                notes = deadlock_core::patch_notes::fetch_patch_notes(10).unwrap_or_default();
            }
            let newest = newest_patch(&notes);
            (StatsFilter { since: None, ..base.clone() }.since_last_patch(&notes), newest)
        } else {
            (base, None)
        };
        let mut filter = filter;
        if filter.since.is_none() {
            filter.since = Some(now().saturating_sub(META_DAYS * 86_400));
        }
        let rows = deadlock_core::stats::items::global_item_stats(&filter).map_err(|e| e.to_string())?;
        let catalog = crate::data::stats_client().fetch_item_catalog().unwrap_or_default();
        let names = rows
            .iter()
            .filter_map(|row| catalog.get(row.item_id).map(|(name, _, _)| (row.item_id, name.clone())))
            .collect();
        Ok((ItemsLoad { rows, names }, since))
    });
}

/// The newest patch's day, `YYYY-MM-DD`.
pub fn newest_patch(notes: &[deadlock_core::patch_notes::PatchNote]) -> Option<String> {
    notes
        .iter()
        .filter(|n| n.is_patch)
        .map(|n| n.published_at.chars().take(10).collect::<String>())
        .max()
}

/// Everyone's best items in the window: win rate, matches and when they are bought.
pub fn meta_section(s: &State, u: &Ui) -> El {
    let m = &s.stats.w4.meta;
    let mut section = col().key("stats-item-meta").w_full().gap(12.0).child(section_head(
        u,
        "Item meta",
        "Win rate and buy time of every item across all players",
    ));
    // DMM's "best items this patch", or the filter's window.
    let patch = s.stats.w4.meta_patch;
    let pick = |key: &str, label: &str, on: bool, set: bool| {
        crate::ui::SegPart::new(key, tr(label)).on(on).with(move |e| {
            e.on_press(move |st: &mut State| {
                if st.stats.w4.meta_patch != set {
                    st.stats.w4.meta_patch = set;
                    st.stats.w4.meta = Fetched::Idle;
                    st.stats.w4.meta_since = None;
                }
            })
        })
    };
    section = section.child(
        crate::ui::Segmented::new("stats-item-meta-window", tr("Item window"))
            .size(crate::ui::SegSize::M)
            .part(pick("stats-item-meta-patch", "This patch", patch, true))
            .part(pick("stats-item-meta-filter", "The filter's window", !patch, false))
            .build(u)
            .self_start(),
    );
    section = match m {
        Fetched::Idle => section.child(
            row().child(
                u.action(tr("Show item meta"), ActionKind::Secondary)
                    .key("stats-item-meta-load")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(start_meta),
            ),
        ),
        Fetched::Loading => section.child(text(tr("Loading items")).font(BODY).color(u.muted)),
        Fetched::Failed(error) => section.child(problem_retry(u, error, "stats-item-meta-retry", start_meta)),
        Fetched::Ready(load) if load.rows.is_empty() => {
            section.child(text(tr("No item has enough matches in this window.")).font(BODY).color(u.muted))
        }
        Fetched::Ready(load) => {
            let mut table = col().w_full().child(head_row(
                u,
                &[("Item", 0.0, false), ("Matches", 96.0, true), ("Win rate", 80.0, true), ("Bought at", 80.0, true)],
            ));
            for item in load.rows.iter().take(META_ROWS) {
                let name = load.names.get(&item.item_id).cloned().unwrap_or_else(|| format!("Item {}", item.item_id));
                let secs = item.avg_buy_time_s.round().max(0.0) as u64;
                table = table.child(
                    data_row(format!("stats-meta-{}", item.item_id), u, 40.0)
                        .child(cell(text(name).font(BODY_MED).color(u.ink).truncate(), 0.0))
                        .child(num(thousands(item.matches), 96.0, u.prose))
                        .child(num(format!("{:.1}%", item.win_rate_percent), 80.0, u.ink))
                        .child(num(format!("{}:{:02}", secs / 60, secs % 60), 80.0, u.prose)),
                );
            }
            let note = match (&s.stats.w4.meta_since, patch) {
                (Some(day), true) => crate::screens::settings::i18n::fill(
                    tr("Everyone's games since the patch on {}. Items need 500 matches to show."),
                    &[day],
                ),
                (None, true) => tr("No patch date was found, so this is the last 30 days. Items need 500 matches to show.").to_string(),
                _ => tr("Everyone's games in the filter's window, 30 days when none is set. Items need 500 matches to show.").to_string(),
            };
            section.child(table).child(text(note).font(SMALL).color(u.muted).wrap())
        }
    };
    section
}

/// Scenes: the ranked filter on (the overview reads the filtered copy), a party picked by hand
/// with its rated lineups, and the item meta.
pub fn scene(s: &mut State) {
    use deadlock_core::stats::items::ItemStat;
    super::scene_people(s);
    s.stats.more.ranked = Some(true);
    refilter(s);
    let heroes: Vec<u32> = s
        .stats
        .snapshot
        .as_ref()
        .map(|snap| snap.heroes.iter().map(|h| h.hero_id).take(6).collect())
        .unwrap_or_default();
    let at = |i: usize| heroes.get(i).copied().unwrap_or(i as u32 + 1);
    s.stats.w4.hand = true;
    s.stats.w4.picks = vec![vec![at(0), at(1)], vec![at(2), at(3), at(4)], vec![at(5)]];
    s.stats.w4.picking = Some(1);
    let lineup = |a: u32, b: u32, c: u32, rate: f64, matches: u64| LineupRating {
        account_ids: vec![1, 2, 3],
        hero_ids: vec![a, b, c],
        win_rate_percent: Some(rate),
        matches: Some(matches),
        sample_matches: matches,
        estimated: false,
    };
    s.stats.w4.hand_party = Fetched::Ready(vec![
        lineup(at(0), at(2), at(5), 56.4, 1_240),
        lineup(at(1), at(3), at(5), 53.1, 860),
        lineup(at(0), at(4), at(5), 49.8, 120),
    ]);
    let item = |id: u64, matches: u64, rate: f64, buy: f64| ItemStat {
        item_id: id,
        matches,
        wins: (matches as f64 * rate / 100.0) as u64,
        losses: matches - (matches as f64 * rate / 100.0) as u64,
        players: 1_000,
        win_rate_percent: rate,
        global_win_rate_percent: rate,
        win_rate_delta_percent: 0.0,
        avg_buy_time_s: buy,
        global_avg_buy_time_s: buy,
        buy_time_delta_s: 0.0,
    };
    let names = [(21, "Leech"), (22, "Boundless Spirit"), (23, "Siphon Bullets"), (24, "Curse")]
        .into_iter()
        .map(|(id, name)| (id, name.to_string()))
        .collect();
    s.stats.w4.meta = Fetched::Ready(super::more_items(
        vec![item(21, 182_000, 58.2, 1_640.0), item(22, 96_000, 57.4, 2_010.0), item(23, 240_000, 55.9, 1_220.0), item(24, 71_000, 55.1, 1_790.0)],
        names,
    ));
    s.stats.w4.meta_since = Some("2026-11-12".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_offers_three_heroes_at_most() {
        let mut picks = vec![Vec::new(), Vec::new()];
        for hero in [1, 2, 3, 4] {
            toggle_pick(&mut picks, 0, hero);
        }
        assert_eq!(picks[0], vec![1, 2, 3]);
        toggle_pick(&mut picks, 0, 2);
        assert_eq!(picks[0], vec![1, 3]);
        toggle_pick(&mut picks, 5, 1);
        assert_eq!(picks.len(), 2);
    }

    #[test]
    fn only_a_real_filter_narrows() {
        assert!(!narrows(&StatsFilter::default()));
        assert!(narrows(&StatsFilter { ranked: Some(true), ..Default::default() }));
        assert!(narrows(&StatsFilter { since: Some(5), ..Default::default() }));
        assert!(narrows(&StatsFilter { until: Some(5), ..Default::default() }));
    }
}
