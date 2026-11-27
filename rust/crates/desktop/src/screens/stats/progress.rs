//! A player page that draws at once and fills in part by part.
//!
//! Track (or opening a player) puts a page with the player's name and picture on screen in the
//! same frame, then reads its parts side by side on one worker (`deadlock_core::stats::parts`):
//! profile, rank, hero rows and match history, then the peer benchmarks once the rank is in.
//! Each part is first read from the disk cache, so a player seen before shows their last page
//! at once with "Updating" in the head, then the API's answer replaces it. Every section has
//! its own loading and error state, and a part that fails leaves the others standing.
//!
//! The poll drains whatever arrived since the last frame, patches the shown snapshot in place
//! and derives the page once, not once per response.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use deadlock_core::stats::cache::Age;
use deadlock_core::stats::parts::{self, Arrival, Part, PartKind};
use rune::El;

use crate::app::State;
use crate::ui::Ui;

use super::german::tr;
use super::{Chart, Fetched};

/// Matches in a page's window.
pub const WINDOW: usize = 200;

/// One section's state for the page now.
#[derive(Clone, Debug, PartialEq)]
pub enum Section {
    /// Its data is on the page (from the API or a stored copy).
    Ready,
    Loading,
    Failed(String),
}

type Message = (Arrival, Vec<(String, Vec<u8>)>);

#[derive(Default)]
pub struct Progress {
    /// The player the running (or last) load is for.
    pub account_id: Option<u64>,
    /// A home load (Track, the account button): it writes the tracked list.
    home: bool,
    rx: Option<Receiver<Message>>,
    /// Parts of this load still to arrive.
    outstanding: Vec<PartKind>,
    status: BTreeMap<PartKind, Section>,
    ages: BTreeMap<PartKind, Age>,
    /// When the last load finished, for "Updated just now".
    pub finished: Option<Instant>,
    /// Some part arrived with data in this load.
    any_data: bool,
    first_error: Option<String>,
}

impl Progress {
    /// A load is running.
    pub fn running(&self) -> bool {
        !self.outstanding.is_empty()
    }

    /// The state of one section for `account_id`'s page. A page not loaded through here
    /// (a fixture, a kept page) is whole.
    pub fn section(&self, account_id: u64, kind: PartKind) -> Section {
        if self.account_id != Some(account_id) {
            return Section::Ready;
        }
        self.status.get(&kind).cloned().unwrap_or(Section::Ready)
    }

    fn age(&self) -> Age {
        let mut age = Age::default();
        for part in self.ages.values() {
            age.merge(*part);
        }
        age
    }
}

/// The section's state on the page shown now.
pub fn section(st: &State, kind: PartKind) -> Section {
    match st.stats.snapshot.as_ref() {
        Some(snap) => st.stats.progress.section(snap.account_id, kind),
        None => Section::Ready,
    }
}

/// The best name known for a player before their profile arrives.
fn known_name(st: &State, account_id: u64) -> String {
    let v = &st.stats;
    if let Some(row) = v.remembered.iter().find(|row| row.account_id == account_id) {
        return row.name.clone();
    }
    if let Some(user) = v.steam.iter().find(|user| u64::from(user.account_id) == account_id) {
        return user.persona.clone();
    }
    if let Some(found) = v.lookup.results.iter().find(|found| found.account_id == account_id) {
        return found.name.clone();
    }
    if let Fetched::Ready(social) = &v.social
        && let Some(record) = social.mates.iter().chain(social.enemies.iter()).find(|r| r.account_id == account_id)
        && let Some(name) = record.persona_name.clone()
    {
        return name;
    }
    String::new()
}

/// Load `account_id`'s page. `home` for Track and the account button; a visit otherwise.
/// The page shows the player at once: their last page from the cache, or their name and
/// picture with every section loading.
pub fn start(st: &mut State, account_id: u64, home: bool) {
    start_parts(st, account_id, home, &parts::ALL);
}

/// Load only `kinds` again for the page shown (a section's Try again).
pub fn retry(st: &mut State, kinds: &[PartKind]) {
    let Some(account_id) = st.stats.snapshot.as_ref().map(|snap| snap.account_id) else { return };
    let home = st.stats.home == Some(account_id);
    start_parts(st, account_id, home, kinds);
}

fn start_parts(st: &mut State, account_id: u64, home: bool, kinds: &[PartKind]) {
    if account_id == 0 {
        return;
    }
    if !st.prefs.allow_stats {
        let off = "Player stats are turned off in Settings, Privacy.".to_string();
        if home {
            st.stats.error = Some(off);
        } else {
            st.stats.nav.error = Some(off);
        }
        return;
    }
    let same = st.stats.snapshot.as_ref().is_some_and(|snap| snap.account_id == account_id);
    if !same {
        let placeholder = parts::placeholder(account_id, &known_name(st, account_id));
        if home {
            super::set_snapshot(&mut st.stats, placeholder);
            st.stats.heat = None;
            st.stats.heat_error = None;
        } else {
            super::nav::install(st, placeholder);
        }
    }
    st.stats.loaded = true;
    st.stats.error = None;
    st.stats.nav.error = None;
    let p = &mut st.stats.progress;
    if p.account_id != Some(account_id) {
        p.status.clear();
        p.ages.clear();
    }
    p.account_id = Some(account_id);
    p.home = home;
    p.any_data = false;
    p.first_error = None;
    p.finished = None;
    p.outstanding = kinds.to_vec();
    for kind in kinds {
        // A part already on the page stays drawn while it refreshes.
        if same && p.status.get(kind) == Some(&Section::Ready) {
            continue;
        }
        p.status.insert(*kind, Section::Loading);
    }
    super::nav::tab_opened(st);
    if super::nav::offline_build() {
        // Tests and scenes never reach the network: the parts stay loading.
        return;
    }
    let (tx, rx) = mpsc::channel::<Message>();
    st.stats.progress.rx = Some(rx);
    let wake = st.wake.clone();
    let kinds = kinds.to_vec();
    let dir = crate::data::stats_cache_dir();
    let spawned = std::thread::Builder::new().name("dah-stats-parts".to_string()).spawn(move || {
        let send = |arrival: Arrival| {
            // Hero pictures are local files: read here, never on the UI thread.
            let icons = match &arrival.part {
                Ok(Part::Heroes(rows)) => crate::data::stats_icons_for(rows.iter().map(|h| (h.hero_id, h.hero_name.clone()))),
                Ok(Part::Matches(rows)) => crate::data::stats_icons_for(rows.iter().map(|m| (m.hero_id, m.hero_name.clone()))),
                _ => Vec::new(),
            };
            let _ = tx.send((arrival, icons));
            if let Some(wake) = &wake {
                wake();
            }
        };
        let tx_send = &send;
        parts::load(Some(dir), account_id, &kinds, WINDOW, &|arrival| tx_send(arrival));
    });
    if spawned.is_err() {
        let p = &mut st.stats.progress;
        p.outstanding.clear();
        p.rx = None;
    }
}

/// The headless `stats` scene: load `account_id` on this thread and draw the result, as the
/// old one-shot load did (only with `DAH_STEAM_ID` set).
pub fn load_now(st: &mut State, account_id: u64) {
    start(st, account_id, true);
    let (tx, rx) = mpsc::channel::<Message>();
    let send = |arrival: Arrival| {
        let _ = tx.send((arrival, Vec::new()));
    };
    parts::load(Some(crate::data::stats_cache_dir()), account_id, &parts::ALL, WINDOW, &send);
    drop(send);
    apply(st, rx.try_iter().collect());
}

/// Take what arrived. Called from the Stats poll.
pub fn poll(st: &mut State) -> bool {
    let mut arrived = Vec::new();
    if let Some(rx) = &st.stats.progress.rx {
        loop {
            match rx.try_recv() {
                Ok(message) => arrived.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The worker is gone: whatever did not arrive failed.
                    let p = &mut st.stats.progress;
                    for kind in std::mem::take(&mut p.outstanding) {
                        if p.status.get(&kind) == Some(&Section::Loading) {
                            p.status.insert(kind, Section::Failed("The stats worker stopped.".to_string()));
                        }
                    }
                    p.rx = None;
                    break;
                }
            }
        }
    }
    if arrived.is_empty() {
        return false;
    }
    apply(st, arrived);
    true
}

/// Patch the shown page with a batch of arrivals, then derive it once.
pub fn apply(st: &mut State, arrived: Vec<Message>) {
    let mut matches_changed = false;
    let mut any = false;
    let mut profile_name = None;
    for (arrival, icons) in arrived {
        st.pending_thumbs.extend(icons);
        let p = &mut st.stats.progress;
        if p.account_id != Some(arrival.account_id) {
            continue;
        }
        if arrival.last {
            p.outstanding.retain(|kind| *kind != arrival.kind);
        }
        let Some(snap) = st.stats.snapshot.as_mut().filter(|snap| snap.account_id == arrival.account_id) else {
            continue;
        };
        match arrival.part {
            Ok(part) => {
                p.any_data = true;
                p.status.insert(arrival.kind, Section::Ready);
                p.ages.insert(arrival.kind, arrival.age);
                if let Part::Profile(profile) = &part {
                    profile_name = Some(profile.persona_name.clone());
                }
                matches_changed |= arrival.kind == PartKind::Matches;
                parts::apply(snap, part, WINDOW);
                any = true;
            }
            Err(error) => {
                p.first_error.get_or_insert(error.clone());
                match p.status.get(&arrival.kind) {
                    // A stored copy is on the page: keep it and say it is older.
                    Some(Section::Ready) if p.ages.contains_key(&arrival.kind) => {
                        if let Some(age) = p.ages.get_mut(&arrival.kind) {
                            age.stale = true;
                        }
                    }
                    _ => {
                        p.status.insert(arrival.kind, Section::Failed(error));
                    }
                }
            }
        }
    }
    let p = &st.stats.progress;
    let (home, done, account_id) = (p.home, !p.running(), p.account_id.unwrap_or(0));
    st.stats.age = p.age();
    if any {
        rederive(&mut st.stats, matches_changed);
    }
    if home && let Some(name) = profile_name {
        crate::data::remember_stats_account(st, account_id, &name);
        st.stats.account = rune::TextEdit::new("");
        st.stats.account_open = false;
    }
    if done && st.stats.progress.finished.is_none() {
        finish(st, home, account_id);
    }
}

/// Every part is in (or failed).
fn finish(st: &mut State, home: bool, account_id: u64) {
    let p = &mut st.stats.progress;
    p.finished = Some(Instant::now());
    p.rx = None;
    if !p.any_data && !p.status.values().any(|s| *s == Section::Ready) {
        // Nothing at all: the player id is wrong or the API is down. Say so in place of
        // the page, as before.
        let error = p.first_error.clone().unwrap_or_else(|| "The stats did not load.".to_string());
        st.stats.snapshot = None;
        st.stats.derived = None;
        if home {
            st.status = error.clone();
            st.stats.error = Some(error);
        } else {
            st.stats.nav.error = Some(error);
        }
        return;
    }
    if home {
        st.status = format!("Loaded player stats for account {account_id}.");
        crate::screens::settings::product::stats(st);
    }
}

/// Derive what the view draws. The chart choice follows the matches only when they changed.
fn rederive(view: &mut super::View, matches_changed: bool) {
    let Some(snap) = view.snapshot.as_ref() else { return };
    let derived = super::derive(snap);
    if matches_changed {
        view.chart = if derived.trajectory.len() >= 2 { Chart::Rank } else { Chart::WinRate };
    }
    view.derived = Some(derived);
}

/// The page head's note: "Updating" while parts load over a shown page, then how old it is.
pub fn note(st: &State) -> Option<String> {
    let v = &st.stats;
    let snap = v.snapshot.as_ref()?;
    let p = &v.progress;
    if p.account_id != Some(snap.account_id) {
        return None;
    }
    if p.running() {
        return Some(tr("Updating").to_string());
    }
    p.finished?;
    let age = p.age();
    if age.stale {
        return Some(tr("Older copy").to_string());
    }
    let minutes = age.seconds / 60;
    Some(if minutes < 1 {
        tr("Updated just now").to_string()
    } else {
        crate::screens::settings::i18n::fill(tr("Updated {} min ago"), &[&minutes.to_string()])
    })
}

/// A section that is still loading or failed, in place of its content; `None` once its data
/// is on the page (or when the page has some rows of it already).
pub fn gate(st: &State, u: &Ui, kind: PartKind, has_rows: bool, rows: usize) -> Option<El> {
    if has_rows {
        return None;
    }
    match section(st, kind) {
        Section::Ready => None,
        Section::Loading => Some(loading_rows(u, kind, rows)),
        Section::Failed(error) => Some(failed(u, kind, &error)),
    }
}

/// What a loading section says, for its screen reader label and the empty state.
fn loading_label(kind: PartKind) -> &'static str {
    match kind {
        PartKind::Matches => "Loading matches",
        PartKind::Heroes => "Loading heroes",
        PartKind::Rank => "Loading the rank",
        PartKind::Profile | PartKind::Peers => "Loading",
    }
}

/// Grey rows where a section's rows will be.
pub fn loading_rows(u: &Ui, kind: PartKind, n: usize) -> El {
    use rune::{col, el, row};
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(crate::ui::r(4.0)).fill(u.hover);
    let mut c = col()
        .key(format!("stats-loading-{kind:?}").to_lowercase())
        .w_full()
        .role("status")
        .label(tr(loading_label(kind)).to_string());
    for i in 0..n {
        c = c.child(
            row()
                .w_full()
                .h(40.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(bar(24.0, 24.0))
                .child(bar(120.0 + (i % 3) as f32 * 40.0, 12.0))
                .a11y_hidden(),
        );
    }
    c
}

/// A failed section with its own Try again, which reads that part only.
pub fn failed(u: &Ui, kind: PartKind, error: &str) -> El {
    let what = match kind {
        PartKind::Matches => tr("The match history did not load."),
        PartKind::Heroes => tr("The hero rows did not load."),
        PartKind::Rank => tr("The rank did not load."),
        PartKind::Peers => tr("The rank comparison did not load."),
        PartKind::Profile => tr("The profile did not load."),
    };
    let message = format!("{what} {error}");
    let mut kinds = vec![kind];
    if kind == PartKind::Rank {
        kinds.push(PartKind::Peers);
    }
    super::shared::problem_retry(u, &message, &format!("stats-retry-{kind:?}").to_lowercase(), move |st: &mut State| {
        retry(st, &kinds)
    })
}

/// Scenes: the page for `account_id` with `parts` on it and the rest loading (or failed).
pub fn scene_parts(st: &mut State, account_id: u64, home: bool, failed: &[(PartKind, &str)]) {
    let p = &mut st.stats.progress;
    p.account_id = Some(account_id);
    p.home = home;
    p.finished = None;
    p.status.clear();
    p.outstanding.clear();
    for kind in parts::ALL {
        let state = match st.stats.snapshot.as_ref() {
            Some(snap) if has_part(snap, kind) => Section::Ready,
            _ => Section::Loading,
        };
        if state == Section::Loading {
            p.outstanding.push(kind);
        }
        p.status.insert(kind, state);
    }
    for (kind, error) in failed {
        p.outstanding.retain(|k| k != kind);
        p.status.insert(*kind, Section::Failed(error.to_string()));
    }
}

fn has_part(snap: &deadlock_core::stats::StatsSnapshot, kind: PartKind) -> bool {
    match kind {
        PartKind::Profile => !snap.profile.avatar.is_empty() || snap.profile.country_code.is_some(),
        PartKind::Rank => snap.rank.reported,
        PartKind::Heroes => !snap.heroes.is_empty(),
        PartKind::Matches => !snap.matches.is_empty(),
        PartKind::Peers => snap.peers.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut s = State::default();
        super::super::load_fixture(&mut s);
        s
    }

    fn arrival(account_id: u64, part: Result<Part, String>, kind: PartKind) -> Message {
        (Arrival { account_id, kind, part, from_cache: false, last: true, age: Age::default() }, Vec::new())
    }

    #[test]
    fn track_draws_the_player_at_once_with_every_section_loading() {
        let mut s = state();
        s.stats.remembered.push(super::super::Remembered { account_id: 4_242, name: "Wren".into(), primary: false });
        start(&mut s, 4_242, true);
        let snap = s.stats.snapshot.as_ref().expect("a page at once");
        assert_eq!(snap.account_id, 4_242);
        assert_eq!(snap.profile.persona_name, "Wren", "the name known before any request");
        for kind in parts::ALL {
            assert_eq!(section(&s, kind), Section::Loading, "{kind:?}");
        }
        assert!(s.stats.progress.running());
        assert_eq!(note(&s).as_deref(), Some("Updating"));
    }

    #[test]
    fn a_failed_part_does_not_blank_the_page() {
        let mut s = state();
        start(&mut s, 4_242, true);
        let fixture = deadlock_core::stats::snapshot_from_bundle(super::super::FIXTURE).unwrap().snapshot;
        apply(
            &mut s,
            vec![
                arrival(4_242, Ok(Part::Matches(fixture.matches.clone())), PartKind::Matches),
                arrival(4_242, Err("HTTP 429".into()), PartKind::Heroes),
                arrival(4_242, Ok(Part::Profile(fixture.profile.clone())), PartKind::Profile),
            ],
        );
        let snap = s.stats.snapshot.as_ref().expect("the page stays");
        assert_eq!(snap.matches.len(), fixture.matches.len());
        assert!(s.stats.derived.is_some());
        assert_eq!(section(&s, PartKind::Matches), Section::Ready);
        assert!(matches!(section(&s, PartKind::Heroes), Section::Failed(e) if e.contains("429")));
        assert_eq!(section(&s, PartKind::Rank), Section::Loading, "still coming");
        apply(
            &mut s,
            vec![
                arrival(4_242, Err("down".into()), PartKind::Rank),
                arrival(4_242, Err("down".into()), PartKind::Peers),
            ],
        );
        assert!(!s.stats.progress.running());
        assert!(s.stats.snapshot.is_some(), "the parts that came stay");
        assert!(s.stats.error.is_none());
    }

    #[test]
    fn nothing_arriving_says_why_in_place_of_the_page() {
        let mut s = state();
        start(&mut s, 4_242, true);
        let gone: Vec<Message> =
            parts::ALL.iter().map(|kind| arrival(4_242, Err("No Steam profile found".into()), *kind)).collect();
        apply(&mut s, gone);
        assert!(s.stats.snapshot.is_none());
        assert!(s.stats.error.as_deref().is_some_and(|e| e.contains("No Steam profile")));
    }

    #[test]
    fn a_stored_copy_stays_when_the_refresh_fails() {
        let mut s = state();
        start(&mut s, 4_242, true);
        let rows = deadlock_core::stats::snapshot_from_bundle(super::super::FIXTURE).unwrap().snapshot.matches;
        let mut cached = arrival(4_242, Ok(Part::Matches(rows.clone())), PartKind::Matches);
        cached.0.from_cache = true;
        cached.0.last = false;
        cached.0.age = Age { seconds: 900, stale: false };
        apply(&mut s, vec![cached]);
        assert_eq!(section(&s, PartKind::Matches), Section::Ready);
        apply(&mut s, vec![arrival(4_242, Err("timeout".into()), PartKind::Matches)]);
        assert_eq!(section(&s, PartKind::Matches), Section::Ready, "the copy is kept");
        assert_eq!(s.stats.snapshot.as_ref().unwrap().matches.len(), rows.len());
        assert!(s.stats.age.stale, "and the head says it is older");
    }

    #[test]
    fn arrivals_for_a_player_no_longer_shown_are_dropped() {
        let mut s = state();
        start(&mut s, 4_242, true);
        start(&mut s, 5_151, true);
        apply(&mut s, vec![arrival(4_242, Ok(Part::Heroes(Vec::new())), PartKind::Heroes)]);
        assert_eq!(s.stats.snapshot.as_ref().unwrap().account_id, 5_151);
        assert_eq!(section(&s, PartKind::Heroes), Section::Loading);
    }
}
