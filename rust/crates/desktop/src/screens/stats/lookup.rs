//! Find a player: the field in the page head.
//!
//! An account id or a `/profiles/` link opens that player at once. Anything else is a name:
//! after a pause in typing (or Enter) one `steam-search` request lists up to eight players
//! with recent matches, and one batch rank request ranks them. Tracked players whose name
//! matches are listed under the results from the local list, with no request. A press on a
//! row opens that player's page; nothing is tracked.

use std::time::{Duration, Instant};

use deadlock_core::stats::{AccountInput, RankBadge, parse_account_input};
use rune::{Color, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};

use crate::app::State;
use crate::jobs::Job;
use crate::ui::{BODY, BODY_MED, SMALL, Ui};

use super::german::tr;
use super::shared::{TNUM_SMALL, player_avatar, problem};

/// The pause after typing before a name is searched.
const PAUSE: Duration = Duration::from_millis(400);
/// Results asked for.
const LIMIT: usize = 8;
const FIELD: &str = "stats-lookup";

/// One player found by name, or a leaderboard candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub account_id: u64,
    pub name: String,
    pub country: Option<String>,
    pub avatar: Option<String>,
    pub rank: Option<RankBadge>,
}

pub struct Lookup {
    pub edit: TextEdit,
    pub open: bool,
    pub results: Vec<Found>,
    /// "Searching", "No active player...", or an error.
    pub note: Option<(String, bool)>,
    /// When the field last changed, for the pause before a search.
    edited_at: Option<Instant>,
    /// The text the shown results are for.
    asked: String,
    job: Job<(String, Vec<Found>)>,
    /// The results are a leaderboard entry's candidates, not a name search.
    pub candidates: bool,
}

impl Default for Lookup {
    fn default() -> Self {
        Lookup {
            edit: TextEdit::new(""),
            open: false,
            results: Vec::new(),
            note: None,
            edited_at: None,
            asked: String::new(),
            job: Job::default(),
            candidates: false,
        }
    }
}

/// The field, 32 high, in the page head.
pub fn field(s: &State, u: &Ui) -> El {
    u.page_search(
        rune::text_input(&s.stats.lookup.edit, BODY)
            .key(FIELD)
            .label(tr("Find a player"))
            .single_line()
            .grow(1.0)
            .min_w(0.0)
            .color(u.ink)
            .placeholder(tr("Find a player"), u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(|st: &mut State| {
                st.stats.lookup.edited_at = Some(Instant::now());
                &mut st.stats.lookup.edit
            })
            .on_key(|st: &mut State, ev: &KeyEvent, tx: &mut Tx| match ev.key {
                Key::Enter => {
                    run(st, tx);
                    true
                }
                Key::Escape if st.stats.lookup.open => {
                    st.stats.lookup.open = false;
                    true
                }
                _ => {
                    tx.wake_after(PAUSE);
                    false
                }
            }),
    )
    .key("stats-lookup-field")
}

/// Act on the field now: open an id, or search a name.
pub fn run(st: &mut State, tx: &mut Tx) {
    let text = st.stats.lookup.edit.text.clone();
    st.stats.lookup.edited_at = None;
    match parse_account_input(&text) {
        AccountInput::Account(account_id) => {
            st.stats.lookup.edit = TextEdit::new("");
            st.stats.lookup.open = false;
            super::nav::open_player(st, tx, account_id);
        }
        AccountInput::Search(query) => search(st, query),
        AccountInput::Empty => st.stats.lookup.open = false,
    }
}

fn search(st: &mut State, query: String) {
    let l = &mut st.stats.lookup;
    l.open = true;
    l.candidates = false;
    if l.asked == query && !l.results.is_empty() {
        return;
    }
    if !st.prefs.allow_stats {
        l.note = Some((tr("Player stats are turned off in Settings, Privacy.").to_string(), true));
        return;
    }
    l.asked = query.clone();
    l.results.clear();
    l.note = Some((tr("Searching").to_string(), false));
    if l.job.is_running() || super::nav::offline_build() {
        return;
    }
    let wake = st.wake.clone();
    l.job.start(wake, move || {
        let client = crate::data::stats_client();
        let rows = client.search_players(&query, LIMIT).map_err(|error| error.to_string())?;
        Ok((query, ranked(&client, rows)))
    });
}

/// Profiles to rows, with one batch rank request for all of them.
fn ranked(client: &deadlock_core::stats::StatsClient, rows: Vec<deadlock_core::stats::PlayerProfile>) -> Vec<Found> {
    let ids: Vec<u64> = rows.iter().map(|row| row.account_id).collect();
    let ranks = client.fetch_ranks(&ids).unwrap_or_default();
    rows.into_iter()
        .map(|row| Found {
            account_id: row.account_id,
            rank: ranks.iter().find(|rank| rank.account_id == row.account_id).copied(),
            avatar: (!row.avatar_medium.is_empty()).then(|| row.avatar_medium.clone()),
            name: row.persona_name,
            country: row.country_code,
        })
        .collect()
}

/// A leaderboard entry names several possible accounts: list them to pick from.
pub fn show_candidates(st: &mut State, ids: Vec<u64>) {
    let l = &mut st.stats.lookup;
    l.open = true;
    l.candidates = true;
    l.results.clear();
    l.asked = String::new();
    l.note = Some((tr("Loading the possible accounts").to_string(), false));
    if l.job.is_running() || super::nav::offline_build() {
        return;
    }
    let wake = st.wake.clone();
    l.job.start(wake, move || {
        let client = crate::data::stats_client();
        let rows = client.fetch_profiles(&ids).map_err(|error| error.to_string())?;
        Ok((String::new(), ranked(&client, rows)))
    });
}

/// Search after the pause, and take finished searches. Called from the Stats poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.stats.lookup.job.try_take() {
        let l = &mut st.stats.lookup;
        match result {
            Ok((query, rows)) if query == l.asked => {
                l.note = if rows.is_empty() {
                    Some((tr("No active player with that name. Paste a profile link or id instead.").to_string(), false))
                } else {
                    None
                };
                l.results = rows;
            }
            // An older search: ask again for what the field holds now.
            Ok(_) => {
                let query = l.asked.clone();
                l.asked.clear();
                search(st, query);
            }
            Err(error) => l.note = Some((error, true)),
        }
        changed = true;
    }
    let l = &st.stats.lookup;
    if l.edited_at.is_some_and(|at| at.elapsed() >= PAUSE) {
        st.stats.lookup.edited_at = None;
        match parse_account_input(&st.stats.lookup.edit.text) {
            AccountInput::Search(query) if query.chars().count() >= 2 => {
                search(st, query);
                changed = true;
            }
            AccountInput::Empty => {
                st.stats.lookup.open = false;
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// The results under the field, over the page.
pub fn popover(s: &State, u: &Ui) -> Option<El> {
    let l = &s.stats.lookup;
    if !l.open {
        return None;
    }
    let names = s.stats.snapshot.as_ref().map(|snap| &snap.tier_names);
    let rank_label = |rank: Option<RankBadge>| -> String {
        match (rank, names) {
            (Some(rank), Some(names)) if rank.badge > 0 => deadlock_core::stats::insight::badge_label(rank.badge, names),
            (Some(rank), None) if rank.badge > 0 => rank.badge.to_string(),
            _ => String::new(),
        }
    };
    let mut list = col().w_full();
    if !l.candidates {
        list = list.child(
            row().h(24.0).px(8.0).center_y().child(text(tr("Search shows players with recent matches.")).font(SMALL).color(u.muted)),
        );
    }
    if let Some((note, error)) = &l.note {
        list = list.child(if *error {
            col().w_full().gap(8.0).padding(8.0).child(problem(u, note)).child(
                row().child(
                    u.action(tr("Try again"), crate::ui::ActionKind::Secondary)
                        .key("stats-lookup-retry")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            let query = st.stats.lookup.asked.clone();
                            st.stats.lookup.asked.clear();
                            search(st, query);
                        }),
                ),
            )
        } else {
            row().h(32.0).px(8.0).center_y().child(text(note.clone()).font(BODY).color(u.muted).wrap())
        });
    }
    for found in &l.results {
        list = list.child(result_row(s, u, found, &rank_label(found.rank), false));
    }
    // Tracked players whose name holds the text, from the local list.
    let needle = l.edit.text.trim().to_lowercase();
    let tracked: Vec<&super::Remembered> = if needle.is_empty() || l.candidates {
        Vec::new()
    } else {
        s.stats.remembered.iter().filter(|r| r.name.to_lowercase().contains(&needle)).collect()
    };
    if !tracked.is_empty() {
        list = list.child(row().h(28.0).px(8.0).center_y().child(text(tr("Tracked")).font(SMALL).color(u.muted)));
        for r in tracked {
            let found = Found { account_id: r.account_id, name: r.name.clone(), country: None, avatar: None, rank: None };
            list = list.child(result_row(s, u, &found, "", true));
        }
    }
    let menu = col()
        .key("stats-lookup-results")
        .anchored("stats-lookup-field", rune::Side::Bottom, rune::Align::End, 4.0)
        .transition(crate::motion::pop(false))
        .w(440.0)
        .max_h(440.0)
        .padding(4.0)
        .radius(crate::ui::r(4.0))
        .fill(u.raised)
        .border(1.0, u.hair_strong)
        .shadow(4.0, 16.0, Color::BLACK.alpha(0.35))
        .role("dialog")
        .label(tr("Players found"))
        .child(u.scroll_area(col().w_full().min_h(0.0).shrink().scroll(rune::ScrollSpec { stick_bottom: false }).child(list)));
    let scrim = el().on_press(|st: &mut State| st.stats.lookup.open = false);
    Some(u.popover_layer("stats-lookup-layer", scrim, menu))
}

fn result_row(s: &State, u: &Ui, found: &Found, rank: &str, tracked: bool) -> El {
    let account_id = found.account_id;
    row()
        .key(format!("stats-lookup-{}{account_id}", if tracked { "t-" } else { "" }))
        .w_full()
        .h(40.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(2.0))
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(crate::screens::settings::i18n::fill(tr("Open the stats for {}"), &[&found.name]))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
            st.stats.lookup.open = false;
            st.stats.lookup.edit = TextEdit::new("");
            super::nav::open_player(st, tx, account_id);
        })
        .child(player_avatar(s, u, account_id, &found.name, 24.0))
        .child(text(found.name.clone()).font(BODY_MED).color(u.ink).truncate().grow(1.0).min_w(0.0))
        .child(text(found.country.clone().unwrap_or_default()).font(TNUM_SMALL).color(u.muted).w(24.0))
        .child(row().w(120.0).gap(6.0).center_y().child(super::shared::rank_badge(s, u, found.rank.map(|r| r.badge).unwrap_or(0), 20.0)).child(text(rank.to_string()).font(SMALL).color(u.prose).truncate().min_w(0.0).shrink()))
        .child(text(account_id.to_string()).font(TNUM_SMALL).color(u.muted).w(80.0).align(rune::Align::End))
}
