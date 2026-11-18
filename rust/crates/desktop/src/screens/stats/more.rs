//! Deadlock Mod Manager's Stats additions: the ranked and date filter, the Items view and the
//! Party view. Grimoire has the raw item endpoint only, no player against everyone join, no
//! filter and no party rating.
//!
//! - The filter (ranked, unranked or both, and a start date) scopes Items and Party on the
//!   server (`stats::filters::filter_query`) and the Matches list here, from each match's mode
//!   and start time.
//! - Items: the player's item win rates against everyone's in the same scope, and how early
//!   they buy each item (`stats::items::item_stats`). An open match also shows its purchase
//!   timeline (`item_timeline`).
//! - Party: every lineup a party can queue from its members' most played heroes, rated from
//!   pair and trio records (`stats::party::lineup_ratings`). Members are the player plus the
//!   tracked players and premades picked on the page.
//!
//! Every read runs on a worker after a press, like the teammates list: each one is several
//! requests to the shared API.

use std::collections::BTreeSet;

use deadlock_core::stats::filters::StatsFilter;
use deadlock_core::stats::items::{ItemPurchase, ItemStat};
use deadlock_core::stats::party::LineupRating;
use deadlock_core::stats::MatchSummary;
use rune::{El, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::Fetched;

/// Rows Items shows before "Show all".
const ITEM_ROWS: usize = 12;
/// Lineups Party shows.
const LINEUP_ROWS: usize = 10;

/// The date choices: label and days back. 0 is the API's own window.
pub const SINCE: [(&str, u64); 4] = [("Any time", 0), ("Last 7 days", 7), ("Last 30 days", 30), ("Last 90 days", 90)];

/// Items with their names, joined on the worker.
pub struct ItemsLoad {
    pub rows: Vec<ItemStat>,
    pub names: std::collections::HashMap<u64, String>,
}

/// The filter, the three reads and the party picks, kept in Stats' view.
pub struct More {
    /// `Some(true)` ranked, `Some(false)` unranked, `None` both.
    pub ranked: Option<bool>,
    /// Days back, 0 for the API's window.
    pub days: u64,
    pub items: Fetched<ItemsLoad>,
    pub items_job: Job<ItemsLoad>,
    pub items_all: bool,
    pub timeline: Fetched<(u64, Vec<(ItemPurchase, String)>)>,
    pub timeline_job: Job<(u64, Vec<(ItemPurchase, String)>)>,
    pub party: Fetched<Vec<LineupRating>>,
    pub party_job: Job<Vec<LineupRating>>,
    /// Other players picked for the party, by account id.
    pub members: BTreeSet<u64>,
}

impl Default for More {
    fn default() -> Self {
        More {
            ranked: None,
            days: 0,
            items: Fetched::Idle,
            items_job: Job::default(),
            items_all: false,
            timeline: Fetched::Idle,
            timeline_job: Job::default(),
            party: Fetched::Idle,
            party_job: Job::default(),
            members: BTreeSet::new(),
        }
    }
}

impl More {
    /// The filter the API reads, from the choices and the clock.
    pub fn filter(&self, now: u64) -> StatsFilter {
        StatsFilter {
            ranked: self.ranked,
            since: (self.days > 0).then(|| now.saturating_sub(self.days * 86_400)),
        }
    }
}

/// Does a match pass the filter? Mode 4 is ranked, 1 unranked; other modes pass "both" only.
pub fn keeps(more: &More, item: &MatchSummary, now: i64) -> bool {
    keeps_mode_at(more, item.match_mode, item.start_time, now)
}

fn keeps_mode_at(more: &More, match_mode: i32, start_time: i64, now: i64) -> bool {
    let mode_ok = match more.ranked {
        None => true,
        Some(true) => match_mode == 4,
        Some(false) => match_mode == 1,
    };
    let since_ok = more.days == 0 || start_time >= now - (more.days as i64) * 86_400;
    mode_ok && since_ok
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- loads -------------------------------------------------------------------------------

fn allowed(st: &State) -> Result<u32, String> {
    if !st.prefs.allow_stats {
        return Err(tr("Player stats are turned off in Settings, Privacy.").to_string());
    }
    st.stats
        .snapshot
        .as_ref()
        .and_then(|snap| u32::try_from(snap.account_id).ok())
        .ok_or_else(|| tr("Load a player first.").to_string())
}

/// Read the player's item rows for the current filter.
pub fn start_items(st: &mut State) {
    if st.stats.more.items_job.is_running() {
        return;
    }
    let account = match allowed(st) {
        Ok(account) => account,
        Err(error) => {
            st.stats.more.items = Fetched::Failed(error);
            return;
        }
    };
    let filter = st.stats.more.filter(now());
    st.stats.more.items = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.more.items_job.start(wake, move || {
        let rows = deadlock_core::stats::items::item_stats(account, &filter).map_err(|e| e.to_string())?;
        let catalog = crate::data::stats_client().fetch_item_catalog().unwrap_or_default();
        let names = rows
            .iter()
            .filter_map(|row| catalog.get(row.item_id).map(|(name, _, _)| (row.item_id, name.clone())))
            .collect();
        Ok(ItemsLoad { rows, names })
    });
}

/// Read one match's purchases for the player.
pub fn start_timeline(st: &mut State, match_id: u64) {
    if st.stats.more.timeline_job.is_running() {
        return;
    }
    let account = match allowed(st) {
        Ok(account) => account,
        Err(error) => {
            st.stats.more.timeline = Fetched::Failed(error);
            return;
        }
    };
    st.stats.more.timeline = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.more.timeline_job.start(wake, move || {
        let bought = deadlock_core::stats::items::item_timeline(match_id, account).map_err(|e| e.to_string())?;
        let catalog = crate::data::stats_client().fetch_item_catalog().unwrap_or_default();
        let rows = bought
            .into_iter()
            .map(|p| {
                let name = catalog
                    .get(p.item_id)
                    .map(|(name, _, _)| name.clone())
                    .unwrap_or_else(|| format!("Item {}", p.item_id));
                (p, name)
            })
            .collect();
        Ok((match_id, rows))
    });
}

/// Rate the party's lineups for the current filter.
pub fn start_party(st: &mut State) {
    if st.stats.more.party_job.is_running() {
        return;
    }
    let account = match allowed(st) {
        Ok(account) => account,
        Err(error) => {
            st.stats.more.party = Fetched::Failed(error);
            return;
        }
    };
    let mut party = vec![account];
    party.extend(st.stats.more.members.iter().filter_map(|id| u32::try_from(*id).ok()).filter(|id| *id != account));
    party.truncate(deadlock_core::stats::party::MAX_PARTY_SIZE);
    if party.len() < 2 {
        st.stats.more.party = Fetched::Failed(tr("Pick at least one other player for the party.").to_string());
        return;
    }
    let filter = st.stats.more.filter(now());
    st.stats.more.party = Fetched::Loading;
    let wake = st.wake.clone();
    st.stats.more.party_job.start(wake, move || {
        deadlock_core::stats::party::lineup_ratings(&party, &filter).map_err(|e| e.to_string())
    });
}

/// Take finished reads. Called from Stats' poll.
pub fn poll(st: &mut State) -> bool {
    let m = &mut st.stats.more;
    let mut changed = false;
    if let Some(result) = m.items_job.try_take() {
        m.items = match result {
            Ok(load) => Fetched::Ready(load),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = m.timeline_job.try_take() {
        m.timeline = match result {
            Ok(rows) => Fetched::Ready(rows),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = m.party_job.try_take() {
        m.party = match result {
            Ok(rows) => Fetched::Ready(rows),
            Err(error) => Fetched::Failed(error),
        };
        changed = true;
    }
    changed
}

/// A new filter: the server reads are stale, so they go back to their Load button.
fn refilter(st: &mut State) {
    let m = &mut st.stats.more;
    if !m.items_job.is_running() {
        m.items = Fetched::Idle;
    }
    if !m.party_job.is_running() {
        m.party = Fetched::Idle;
    }
}

// --- views -------------------------------------------------------------------------------

fn pill(u: &Ui, key: String, label: &str, on: bool) -> El {
    row()
        .key(key)
        .h(28.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .fill(if on { u.selected } else { rune::Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(tr(label).to_string())
        .cursor(rune::Cursor::Pointer)
        .child(text(tr(label).to_string()).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden())
}

/// The filter bar over the page's sections.
pub(super) fn filter_bar(s: &State, u: &Ui) -> El {
    let m = &s.stats.more;
    let mut modes = row().gap(2.0).padding(2.0).radius(crate::ui::r(8.0)).border(1.0, u.hair);
    for (label, value) in [("Ranked and unranked", None), ("Ranked", Some(true)), ("Unranked", Some(false))] {
        modes = modes.child(
            pill(u, format!("stats-filter-mode-{label}"), label, m.ranked == value).on_press(move |st: &mut State| {
                st.stats.more.ranked = value;
                refilter(st);
            }),
        );
    }
    let mut since = row().gap(2.0).padding(2.0).radius(crate::ui::r(8.0)).border(1.0, u.hair);
    for (label, days) in SINCE {
        since = since.child(
            pill(u, format!("stats-filter-days-{days}"), label, m.days == days).on_press(move |st: &mut State| {
                st.stats.more.days = days;
                refilter(st);
            }),
        );
    }
    row()
        .key("stats-filter")
        .w_full()
        .gap(12.0)
        .center_y()
        .child(text(tr("Show")).font(SMALL).color(u.faint))
        .child(modes)
        .child(since)
        .child(spacer())
        .child(text(tr("Filters Matches, Items and Party")).font(SMALL).color(u.faint))
}

fn minutes(seconds: f64) -> String {
    let s = seconds.round().max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn signed_points(delta: f64) -> String {
    if delta >= 0.05 {
        format!("+{delta:.1}")
    } else if delta <= -0.05 {
        format!("{delta:.1}")
    } else {
        "0.0".to_string()
    }
}

/// Items: the player's item rows against everyone's.
pub(super) fn items_section(s: &State, u: &Ui) -> El {
    let m = &s.stats.more;
    let mut section = col()
        .key("stats-items")
        .w_full()
        .gap(12.0)
        .child(section_head(u, "Items", "Your win rate with each item against everyone's, and when you buy it"));
    section = match &m.items {
        Fetched::Idle => section.child(
            row().w_full().gap(12.0).center_y().child(
                u.action(tr("Show items"), ActionKind::Secondary)
                    .key("stats-items-load")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(start_items),
            ),
        ),
        Fetched::Loading => section.child(text(tr("Loading items")).font(BODY).color(u.muted)),
        Fetched::Failed(error) => section.child(problem(u, error)),
        Fetched::Ready(load) if load.rows.is_empty() => {
            section.child(text(tr("No item has five matches in this filter yet.")).font(BODY).color(u.muted))
        }
        Fetched::Ready(load) => {
            let mut rows: Vec<&ItemStat> = load.rows.iter().collect();
            rows.sort_by_key(|row| std::cmp::Reverse(row.matches));
            let shown = if m.items_all { rows.len() } else { rows.len().min(ITEM_ROWS) };
            let mut table = col().w_full().child(head_row(
                u,
                &[
                    ("Item", 0.0, false),
                    ("Matches", 72.0, true),
                    ("Win rate", 80.0, true),
                    ("Against everyone", 120.0, true),
                    ("Bought at", 80.0, true),
                    ("Than everyone", 100.0, true),
                ],
            ));
            for item in rows.iter().take(shown) {
                let name = load.names.get(&item.item_id).cloned().unwrap_or_else(|| format!("Item {}", item.item_id));
                let better = item.win_rate_delta_percent >= 0.0;
                let earlier = item.buy_time_delta_s <= 0.0;
                table = table.child(
                    data_row(format!("stats-item-{}", item.item_id), u, 40.0)
                        .child(cell(text(name).font(BODY_MED).color(u.ink).truncate(), 0.0))
                        .child(num(item.matches.to_string(), 72.0, u.prose))
                        .child(num(format!("{:.0}%", item.win_rate_percent), 80.0, u.prose))
                        .child(num(
                            signed_points(item.win_rate_delta_percent),
                            120.0,
                            if better { u.teal } else { u.rose },
                        ))
                        .child(num(minutes(item.avg_buy_time_s), 80.0, u.prose))
                        .child(num(
                            format!("{}{}", if earlier { "-" } else { "+" }, minutes(item.buy_time_delta_s.abs())),
                            100.0,
                            if earlier { u.teal } else { u.muted },
                        )),
                );
            }
            let mut out = section.child(table);
            if rows.len() > ITEM_ROWS {
                out = out.child(
                    u.action(if m.items_all { tr("Show fewer") } else { tr("Show all") }, ActionKind::Ghost)
                        .key("stats-items-all")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.stats.more.items_all = !st.stats.more.items_all),
                );
            }
            out
        }
    };
    section
}

/// One match's purchases, under its open row.
pub(super) fn timeline(s: &State, u: &Ui, match_id: u64) -> El {
    let m = &s.stats.more;
    match &m.timeline {
        Fetched::Ready((id, rows)) if *id == match_id => {
            if rows.is_empty() {
                return text(tr("No purchases were recorded for you in this match.")).font(SMALL).color(u.faint);
            }
            let mut line = row().w_full().gap(6.0).flex_wrap();
            for (purchase, name) in rows {
                let sold = purchase.sold_at_s.map(|at| format!(", {} {}", tr("sold"), minutes(at as f64))).unwrap_or_default();
                line = line.child(
                    row()
                        .h(24.0)
                        .px(8.0)
                        .gap(6.0)
                        .center_y()
                        .radius(crate::ui::r(6.0))
                        .border(1.0, u.hair)
                        .child(text(minutes(purchase.bought_at_s as f64)).font(TNUM_SMALL).color(u.faint))
                        .child(text(format!("{name}{sold}")).font(SMALL).color(u.prose)),
                );
            }
            col().w_full().gap(6.0).child(text(tr("Items in the order you bought them")).font(SMALL).color(u.faint)).child(line)
        }
        Fetched::Loading => text(tr("Loading the item timeline")).font(SMALL).color(u.muted),
        Fetched::Failed(error) => problem(u, error),
        _ => u
            .action(tr("Item timeline"), ActionKind::Secondary)
            .key(format!("stats-timeline-{match_id}"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| start_timeline(st, match_id)),
    }
}

fn hero_label(s: &State, hero_id: u32) -> String {
    s.stats
        .snapshot
        .as_ref()
        .and_then(|snap| snap.heroes.iter().find(|h| h.hero_id == hero_id))
        .map(hero_name)
        .unwrap_or_else(|| crate::screens::settings::i18n::fill(tr("Hero {}"), &[&hero_id.to_string()]))
}

/// Party: pick the members, then every lineup they can queue, rated.
pub(super) fn party_section(s: &State, u: &Ui) -> El {
    let m = &s.stats.more;
    let me = s.stats.snapshot.as_ref().map(|snap| snap.account_id);
    let mut section = col()
        .key("stats-party")
        .w_full()
        .gap(12.0)
        .child(section_head(u, "Party", "Every lineup your party can queue, from each member's most played heroes"));
    // Members: tracked players, then premades from the teammates list.
    let mut people: Vec<(u64, String)> = s
        .stats
        .remembered
        .iter()
        .filter(|r| Some(r.account_id) != me)
        .map(|r| (r.account_id, r.name.clone()))
        .collect();
    if let Fetched::Ready(social) = &s.stats.social {
        for mate in social.mates.iter().filter(|p| p.premade) {
            if Some(mate.account_id) != me && !people.iter().any(|(id, _)| *id == mate.account_id) {
                people.push((mate.account_id, mate.persona_name.clone().unwrap_or_else(|| mate.account_id.to_string())));
            }
        }
    }
    if people.is_empty() {
        return section.child(
            text(tr("Track a friend's account or show the players you meet; premades and tracked players can join the party here."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }
    let mut chips = row().w_full().gap(6.0).flex_wrap();
    for (id, name) in people {
        let on = m.members.contains(&id);
        chips = chips.child(
            row()
                .key(format!("stats-party-member-{id}"))
                .h(28.0)
                .pl(4.0)
                .pr(10.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(14.0))
                .border(1.0, if on { u.accent } else { u.hair_strong })
                .fill(if on { u.accent.alpha(0.10) } else { rune::Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("checkbox")
                .checked(on)
                .label(name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    let members = &mut st.stats.more.members;
                    if !members.remove(&id) && members.len() + 1 < deadlock_core::stats::party::MAX_PARTY_SIZE {
                        members.insert(id);
                    }
                })
                .child(avatar(u, &name, 20.0))
                .child(text(name).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden()),
        );
    }
    section = section.child(chips).child(
        row().w_full().gap(12.0).center_y().child(
            u.action(tr("Rate lineups"), ActionKind::Secondary)
                .key("stats-party-rate")
                .disabled(m.members.is_empty())
                .cursor(rune::Cursor::Pointer)
                .on_press(start_party),
        ),
    );
    match &m.party {
        Fetched::Idle => section,
        Fetched::Loading => section.child(text(tr("Rating lineups")).font(BODY).color(u.muted)),
        Fetched::Failed(error) => section.child(problem(u, error)),
        Fetched::Ready(ratings) if ratings.is_empty() => {
            section.child(text(tr("No lineup to rate: the party has no heroes in this filter.")).font(BODY).color(u.muted))
        }
        Fetched::Ready(ratings) => {
            let expected = deadlock_core::stats::party::expected_win_rate(ratings);
            let mut table = col().w_full().child(head_row(u, &[("Lineup", 0.0, false), ("Matches", 96.0, true), ("Win rate", 96.0, true)]));
            for (index, rating) in ratings.iter().take(LINEUP_ROWS).enumerate() {
                let mut heroes = row().gap(6.0).center_y();
                for hero in &rating.hero_ids {
                    let name = hero_label(s, *hero);
                    heroes = heroes.child(hero_chip(s, u, *hero, &name, 24.0)).child(text(name).font(SMALL).color(u.prose));
                }
                let muted = rating.is_low_sample();
                let matches = match rating.matches {
                    Some(n) => thousands(n),
                    None => crate::screens::settings::i18n::fill(tr("about {}"), &[&thousands(rating.sample_matches)]),
                };
                let rate = rating.win_rate_percent.map(|r| format!("{r:.1}%")).unwrap_or_else(|| "-".to_string());
                table = table.child(
                    data_row(format!("stats-lineup-{index}"), u, 40.0)
                        .child(cell(heroes, 0.0))
                        .child(num(matches, 96.0, if muted { u.faint } else { u.prose }))
                        .child(num(rate, 96.0, if muted { u.faint } else { u.ink })),
                );
            }
            let mut out = section.child(table);
            if let Some(rate) = expected {
                out = out.child(
                    text(crate::screens::settings::i18n::fill(
                        tr("Across these lineups the party wins about {}% of its games. Grey rows have few matches behind them."),
                        &[&format!("{rate:.1}")],
                    ))
                    .font(SMALL)
                    .color(u.faint)
                    .wrap(),
                );
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_keeps_the_mode_and_window() {
        let now = 1_000_000;
        let mut more = More::default();
        assert!(keeps_mode_at(&more, 1, 0, now));
        more.ranked = Some(true);
        assert!(keeps_mode_at(&more, 4, now, now) && !keeps_mode_at(&more, 1, now, now));
        more.ranked = Some(false);
        assert!(keeps_mode_at(&more, 1, now, now) && !keeps_mode_at(&more, 4, now, now));
        more.ranked = None;
        more.days = 7;
        assert!(keeps_mode_at(&more, 1, now - 86_400, now));
        assert!(!keeps_mode_at(&more, 1, now - 8 * 86_400, now));
        assert_eq!(more.filter(10 * 86_400).since, Some(3 * 86_400));
        more.days = 0;
        assert_eq!(more.filter(10 * 86_400).since, None);
    }
}
