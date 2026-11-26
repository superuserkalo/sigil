//! Matches by sitting. A row opens the match page.
//!
//! The page's filter line (mode, window, Street Brawl) and the tab's own hero and result
//! filters cut the list; a totals line sums what is left. The history is one response, so
//! "Show 50 more" pages locally.

use deadlock_core::stats::insight::{self, Session};
use deadlock_core::stats::{MatchSummary, StatsSnapshot};
use rune::{El, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::shared::*;
use super::{Derived, Menu};

/// Rows a page shows, and adds on "Show 50 more".
pub const PAGE: usize = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResultFilter {
    #[default]
    Any,
    Wins,
    Losses,
}

impl ResultFilter {
    fn label(self) -> &'static str {
        match self {
            ResultFilter::Any => "Any result",
            ResultFilter::Wins => "Wins",
            ResultFilter::Losses => "Losses",
        }
    }

    fn keeps(self, item: &MatchSummary) -> bool {
        match self {
            ResultFilter::Any => true,
            ResultFilter::Wins => item.win == Some(true),
            ResultFilter::Losses => item.win == Some(false),
        }
    }
}

/// The tab's own filters and paging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListView {
    pub hero: Option<u32>,
    pub result: ResultFilter,
    pub shown: usize,
}

impl Default for ListView {
    fn default() -> Self {
        ListView { hero: None, result: ResultFilter::Any, shown: PAGE }
    }
}

/// The matches every filter keeps, newest first.
pub(super) fn kept<'a>(s: &State, snap: &'a StatsSnapshot) -> Vec<&'a MatchSummary> {
    let now = snap.generated_unix.max(item_now());
    let l = &s.stats.list;
    snap.matches
        .iter()
        .filter(|item| super::more::keeps(&s.stats.more, item, now))
        .filter(|item| l.hero.is_none_or(|hero| item.hero_id == hero))
        .filter(|item| l.result.keeps(item))
        .collect()
}

/// Show the Matches tab on one hero (a Heroes row press).
pub fn filter_hero(st: &mut State, hero_id: u32) {
    st.stats.list.hero = Some(hero_id);
    st.stats.list.shown = PAGE;
    super::nav::set_tab(st, super::Tab::Matches);
}

fn hero_label(snap: &StatsSnapshot, hero_id: u32) -> String {
    snap.heroes
        .iter()
        .find(|hero| hero.hero_id == hero_id)
        .map(hero_name)
        .or_else(|| snap.matches.iter().find(|item| item.hero_id == hero_id).and_then(|item| item.hero_name.clone()))
        .unwrap_or_else(|| format!("Hero {hero_id}"))
}

/// The tab's own line: hero, result, then the totals of what is shown.
fn toolbar(s: &State, u: &Ui, snap: &StatsSnapshot, rows: &[&MatchSummary]) -> El {
    let l = &s.stats.list;
    let hero = match l.hero {
        Some(id) => hero_label(snap, id),
        None => tr("All heroes").to_string(),
    };
    let scored: Vec<bool> = rows.iter().filter_map(|item| item.win).collect();
    let wins = scored.iter().filter(|w| **w).count();
    let (k, d, a) = rows.iter().fold((0u64, 0u64, 0u64), |(k, d, a), item| (k + item.kills as u64, d + item.deaths as u64, a + item.assists as u64));
    let mut totals = vec![crate::screens::settings::i18n::fill(tr("{} matches"), &[&thousands(rows.len() as u64)])];
    if !scored.is_empty() {
        totals.push(crate::screens::settings::i18n::fill(tr("{} win"), &[&format!("{:.0}%", wins as f64 * 100.0 / scored.len() as f64)]));
    }
    if !rows.is_empty() {
        totals.push(format!("{:.1} KDA", (k + a) as f64 / d.max(1) as f64));
    }
    let open = |menu: Menu| move |st: &mut State| st.stats.menu = if st.stats.menu.as_ref() == Some(&menu) { None } else { Some(menu.clone()) };
    row()
        .key("stats-matches-tools")
        .w_full()
        .h(28.0)
        .gap(8.0)
        .center_y()
        .child(u.select_box(&hero, 180.0).key("stats-matches-hero").expanded(s.stats.menu == Some(Menu::Hero)).on_press(open(Menu::Hero)))
        .child(u.select_box(tr(l.result.label()), 140.0).key("stats-matches-result").expanded(s.stats.menu == Some(Menu::Result)).on_press(open(Menu::Result)))
        .child(spacer())
        .child(text(totals.join("   ")).font(TNUM_SMALL).color(u.muted))
}

/// Heroes in the subject's history, most played first.
pub(super) fn hero_menu(s: &State, u: &Ui) -> El {
    let mut menu = u.menu("stats-hero-menu", "stats-matches-hero", 220.0).max_h(420.0);
    let Some(snap) = &s.stats.snapshot else { return menu };
    let mut counts: Vec<(u32, usize)> = Vec::new();
    for item in &snap.matches {
        match counts.iter_mut().find(|(id, _)| *id == item.hero_id) {
            Some((_, n)) => *n += 1,
            None => counts.push((item.hero_id, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    let item = |key: String, label: String, on: bool| {
        row()
            .key(key)
            .w_full()
            .h(28.0)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(2.0))
            .fill(if on { u.selected } else { rune::Color::TRANSPARENT })
            .hover(u.hover)
            .focusable(true)
            .role("menuitemradio")
            .checked(on)
            .label(label.clone())
            .cursor(rune::Cursor::Pointer)
            .child(text(label).font(SMALL).color(if on { u.ink } else { u.prose }).truncate().grow(1.0).min_w(0.0))
    };
    menu = menu.child(item("stats-hero-any".into(), tr("All heroes").to_string(), s.stats.list.hero.is_none()).on_press(|st: &mut State| {
        st.stats.list.hero = None;
        st.stats.list.shown = PAGE;
        st.stats.menu = None;
    }));
    let mut list = col().w_full();
    for (hero_id, n) in counts {
        let label = format!("{}  ({n})", hero_label(snap, hero_id));
        list = list.child(item(format!("stats-hero-{hero_id}"), label, s.stats.list.hero == Some(hero_id)).on_press(move |st: &mut State| {
            st.stats.list.hero = Some(hero_id);
            st.stats.list.shown = PAGE;
            st.stats.menu = None;
        }));
    }
    menu.child(u.scroll_area(col().w_full().max_h(380.0).min_h(0.0).shrink().scroll(rune::ScrollSpec { stick_bottom: false }).child(list)))
}

pub(super) fn result_menu(s: &State, u: &Ui) -> El {
    let mut menu = u.menu("stats-result-menu", "stats-matches-result", 160.0);
    for value in [ResultFilter::Any, ResultFilter::Wins, ResultFilter::Losses] {
        let on = s.stats.list.result == value;
        menu = menu.child(
            u.menu_item(&format!("stats-result-{value:?}"), if on { &icons::CHECK } else { &icons::MINUS }, tr(value.label()), false).on_press(move |st: &mut State| {
                st.stats.list.result = value;
                st.stats.list.shown = PAGE;
                st.stats.menu = None;
            }),
        );
    }
    menu
}

pub(super) fn matches(s: &State, u: &Ui, snap: &StatsSnapshot, d: &Derived) -> El {
    let mut table = col().key("stats-matches").w_full();
    if snap.matches.is_empty() {
        return table.child(text(tr("No matches stored for this player yet. New matches show up a little while after they end.")).font(BODY).color(u.muted));
    }
    let rows = kept(s, snap);
    table = table.child(toolbar(s, u, snap, &rows).mb(12.0));
    if super::nav::visiting(s) {
        table = table.child(
            text(crate::screens::settings::i18n::fill(tr("deadlock-api has stored {} matches for this player."), &[&snap.matches.len().to_string()]))
                .font(SMALL)
                .color(u.muted)
                .pb(8.0),
        );
    }
    if rows.is_empty() {
        return table.child(
            row()
                .w_full()
                .gap(12.0)
                .pt(8.0)
                .center_y()
                .child(text(tr("No matches with these filters.")).font(BODY).color(u.muted))
                .child(
                    u.action(tr("Reset"), ActionKind::Secondary)
                        .key("stats-matches-reset")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            st.stats.list = ListView::default();
                            st.stats.more.ranked = None;
                            st.stats.more.days = 0;
                            st.stats.more.season = None;
                            st.stats.more.brawl = super::more::Brawl::Show;
                            super::w4::refilter(st);
                        }),
                ),
        );
    }
    table = table.child(head_row(
        u,
        &[
            ("Result", 47.0, false),
            ("Hero", 0.0, false),
            ("K / D / A", 88.0, true),
            ("KDA", 44.0, true),
            ("Souls", 56.0, true),
            ("SPM", 48.0, true),
            ("Rank", 48.0, true),
            ("Length", 48.0, true),
            ("Time", 48.0, true),
            ("", 16.0, false),
        ],
    ));
    let keep: std::collections::HashSet<u64> = rows.iter().map(|item| item.match_id).collect();
    let mut left = s.stats.list.shown;
    for (index, sitting) in d.sessions.iter().enumerate() {
        if left == 0 {
            break;
        }
        let shown: Vec<&MatchSummary> = sitting
            .indices
            .iter()
            .map(|i| &snap.matches[*i])
            .filter(|item| keep.contains(&item.match_id))
            .take(left)
            .collect();
        if shown.is_empty() {
            continue;
        }
        left -= shown.len();
        table = table.child(sitting_head(u, snap, sitting, index));
        for item in shown {
            table = table.child(match_row(s, u, item));
        }
    }
    let more = rows.len().saturating_sub(s.stats.list.shown);
    if more > 0 {
        table = table.child(
            row().w_full().h(40.0).center_y().child(spacer()).child(
                u.action(&crate::screens::settings::i18n::fill(tr("Show {} more"), &[&more.min(PAGE).to_string()]), ActionKind::Ghost)
                    .key("stats-more-matches")
                    .mr(-12.0)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.stats.list.shown += PAGE),
            ),
        );
    }
    table
}

fn sitting_head(u: &Ui, snap: &StatsSnapshot, sitting: &Session, index: usize) -> El {
    let day = capitalise(&insight::day_label(sitting.start_time, snap.generated_unix));
    let day = format!("{day}, {}", insight::clock_label(sitting.start_time));
    let n = sitting.indices.len();
    let mut r = row()
        .key(format!("sitting-{index}"))
        .w_full()
        .h(32.0)
        .mt(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(day).font(BODY_MED).color(u.prose))
        .child(
            text(format!(
                "{}  ·  {} W {} L",
                crate::screens::settings::i18n::fill(if n == 1 { tr("{} match") } else { tr("{} matches") }, &[&n.to_string()]),
                sitting.wins,
                sitting.losses
            ))
            .font(TNUM_SMALL)
            .color(u.muted),
        )
        .child(spacer());
    if sitting.ranked > 0 && sitting.rank_delta != 0 {
        r = r.child(
            text(crate::screens::settings::i18n::fill(tr("{} rank points"), &[&signed(sitting.rank_delta)]))
                .font(TNUM_SMALL)
                .color(if sitting.rank_delta > 0 { u.teal } else { u.rose }),
        );
    }
    // Lines the rank points up with the Rank column: Length, Time and the chevron with their
    // 16 point gaps, less this row's own 12 point gap.
    r.child(el().w(48.0 + 48.0 + 16.0 + 3.0 * 16.0 - 12.0))
}

fn match_row(s: &State, u: &Ui, item: &MatchSummary) -> El {
    let (result, color) = match item.win {
        Some(true) => (tr("Win").to_string(), u.teal),
        Some(false) => (tr("Loss").to_string(), u.rose),
        None => (crate::ui::sentence(item.outcome.label()), u.muted),
    };
    let name = item.hero_name.clone().unwrap_or_else(|| format!("Hero {}", item.hero_id));
    let id = item.match_id;
    let rank = match item.ranked_delta {
        Some(delta) if delta != 0 => (format!("{delta:+}"), if delta > 0 { u.teal } else { u.rose }),
        _ => (String::new(), u.muted),
    };
    let edge = match item.win {
        Some(true) => u.teal,
        Some(false) => u.rose,
        None => u.hair_strong,
    };
    data_row(format!("match-{id}"), u, 40.0)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{result} on {name}, open match"))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| super::nav::open_match(st, tx, id))
        .child(
            row()
                .w(47.0)
                .h(40.0)
                .gap(8.0)
                .center_y()
                .child(el().w(3.0).h(40.0).fill(edge).a11y_hidden())
                .child(text(result).font(BODY_MED).color(color).truncate()),
        )
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
                        .radius(crate::ui::r(2.0))
                        .border(1.0, u.amber.alpha(0.5))
                        .child(text(tr("Street Brawl")).font(SMALL).color(u.amber))
                }))
                // DMM's abandoned mark: the game recorded the player as leaving this match.
                .children(super::notes::abandon_tag(u, item)),
        )
        .child(super::brief::kda_text(u, item.kills, item.deaths, item.assists).w(88.0))
        .child(num(format!("{:.2}", item.kda), 44.0, u.muted))
        .child(num(souls(item.net_worth), 56.0, u.muted))
        .child(num(thousands(spm(item)), 48.0, u.muted))
        .child(num(rank.0, 48.0, rank.1))
        .child(num(format!("{}m", item.duration_seconds / 60), 48.0, u.muted))
        .child(num(insight::clock_label(item.start_time), 48.0, u.muted))
        .child(el().w(16.0).center().child(icon(&icons::CHEVRON_RIGHT, 14.0).color(u.muted).a11y_hidden()))
}

/// The clock, for the date filter on the match list.
fn item_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_and_result_filters_cut_the_list() {
        let mut s = State::default();
        super::super::load_fixture(&mut s);
        let snap = s.stats.snapshot.clone().unwrap();
        let all = kept(&s, &snap).len();
        assert_eq!(all, snap.matches.len());
        let hero = snap.matches[0].hero_id;
        s.stats.list.hero = Some(hero);
        assert!(kept(&s, &snap).iter().all(|item| item.hero_id == hero));
        s.stats.list.result = ResultFilter::Wins;
        assert!(kept(&s, &snap).iter().all(|item| item.win == Some(true) && item.hero_id == hero));
        s.stats.list = ListView::default();
        s.stats.list.result = ResultFilter::Losses;
        let losses = kept(&s, &snap).len();
        s.stats.list.result = ResultFilter::Wins;
        assert!(kept(&s, &snap).len() + losses <= all);
    }
}
