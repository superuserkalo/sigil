//! Live match: the match being played right now, at the top of Stats.
//!
//! Two lanes, both local:
//!
//! - `console.log` (the game writes it with `-condebug`): matchmaking, the lobby and match id,
//!   the game state, the hero and the mode (`deadlock_core::live_match`).
//! - The live bridge, an opt-in HUD mod Sigil builds from the player's own game
//!   (`deadlock_core::live_bridge`): both teams as the top bar shows them, printed into the same
//!   log every two seconds.
//!
//! The log is read only while Stats is open and Deadlock runs, every two seconds, and only the
//! bytes the game appended. One optional network lookup: with "Look up players" on (off by
//! default) and Player stats allowed, the account ids the versus screen showed go to
//! `api.deadlock-api.com` once per match for ranks and match counts.
//!
//! Sigil does not draw over the game, read its memory or watch its traffic.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use deadlock_core::live_bridge::{self, BridgeCheck, BridgeState};
use deadlock_core::live_match::{Board, FinishedMatch, LivePlayer, LiveState, LiveTail, MatchPhase, Team};
use rune::{Color, El, Key, KeyEvent, Shape, SurfaceFit, canvas, col, el, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::fill;
use crate::screens::settings::options;
use crate::ui::{ActionKind, BODY, BODY_MED, H2, NUM, SMALL, Ui};

use super::german::tr;
use super::shared::{TNUM, TNUM_SMALL, open_statlocker, signed, souls, statlocker_match_url};

/// How often the log is read while the game runs and Stats is open.
const READ_EVERY: Duration = Duration::from_secs(2);
/// How often the bridge is compared with the game's files again.
const CHECK_EVERY: Duration = Duration::from_secs(600);
/// How often the game build is read while the bridge is on, to notice a game update.
const BUILD_EVERY: Duration = Duration::from_secs(60);
const CURVE_H: f32 = 72.0;
const CURVE_VB: f32 = 1000.0;

/// What deadlock-api knows about a player in the lobby.
#[derive(Clone, Debug, Default)]
pub struct Lookup {
    pub rank: Option<String>,
    pub matches_30d: Option<u64>,
}

#[derive(Default)]
pub struct Live {
    pub tail: Option<LiveTail>,
    job: Job<LiveTail>,
    last_read: Option<Instant>,
    pub check: Option<BridgeCheck>,
    check_job: Job<BridgeCheck>,
    checked_at: Option<Instant>,
    /// Install, rebuild, switch on or remove: one at a time.
    action_job: Job<(Action, String)>,
    pub busy: Option<Action>,
    /// The bridge explanation is open.
    pub confirm: bool,
    /// The last outcome, shown under the panel.
    pub note: Option<(String, bool)>,
    pub lookup: HashMap<u32, Lookup>,
    lookup_job: Job<HashMap<u32, Lookup>>,
    looked_up: Option<u64>,
    icons_loaded: BTreeSet<String>,
    /// Scenes: the game counts as running.
    pub scene_running: Option<bool>,
    /// The game build read last (`live_bridge::game_build`), on a worker every minute.
    build_job: Job<Option<String>>,
    build_read_at: Option<Instant>,
    /// A build the bridge has not been checked against yet, or found outdated for.
    pub update: Option<Update>,
    /// The running rebuild came from a game update, so it ends in a toast.
    auto_rebuild: bool,
    /// The last rebuild try after an update, so a refused one waits a minute before the next.
    tried_at: Option<Instant>,
}

/// A game update the live bridge has to follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub build: String,
    /// The check found the bridge outdated; it waits for the game to close.
    pub outdated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Rebuild,
    SwitchOn,
    Remove,
}

pub(super) fn running(s: &State) -> bool {
    s.stats.live.scene_running.unwrap_or_else(crate::screens::settings::watch::game_is_running)
}

/// A match is in progress in the log (for the tab's green mark).
pub(super) fn in_match(s: &State) -> bool {
    s.stats
        .live
        .tail
        .as_ref()
        .and_then(|tail| tail.state.current.as_ref())
        .is_some_and(|current| current.phase == MatchPhase::InProgress)
}

/// The Live tab: the panel, or what to do to see one.
pub(super) fn tab(s: &State, u: &Ui) -> El {
    panel(s, u).unwrap_or_else(|| {
        // Starts at the same y as every other tab's first section.
        col()
            .w_full()
            .gap(4.0)
            .child(super::shared::section_head(u, tr("No match running"), ""))
            .child(text(tr("Start Deadlock and this tab follows the match you are in.")).font(BODY).color(u.prose))
    })
}

/// The game watcher runs its process check while this is true: Stats is open with the view on.
pub fn wants_watch(st: &State) -> bool {
    options::get().live_view && st.screen == Screen::Stats
}

// --- poll -----------------------------------------------------------------------------------

/// Called from the Stats poll. Reads the log on a worker every two seconds while it matters.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok(tail)) = st.stats.live.job.try_take() {
        // The log is read every two seconds; rebuild the page only when what it shows moved.
        let same = st.stats.live.tail.as_ref().is_some_and(|old| {
            old.state == tail.state && old.found == tail.found && old.modified == tail.modified
        });
        st.stats.live.tail = Some(tail);
        changed |= !same;
    }
    if let Some(result) = st.stats.live.check_job.try_take() {
        st.stats.live.check = result.ok();
        after_check(st);
        changed = true;
    }
    changed |= follow_updates(st);
    if let Some(result) = st.stats.live.lookup_job.try_take() {
        if let Ok(found) = result {
            st.stats.live.lookup.extend(found);
        }
        changed = true;
    }
    if let Some(result) = st.stats.live.action_job.try_take() {
        finish_action(st, result);
        changed = true;
    }
    if st.demo || rune::is_headless() || !wants_watch(st) {
        return changed;
    }
    let Some(root) = st.game_root.clone() else { return changed };
    let live = &st.stats.live;
    let due = live.last_read.is_none_or(|at| at.elapsed() >= READ_EVERY);
    if due && !live.job.is_running() && (running(st) || live.tail.is_none()) {
        let mut tail = live.tail.clone().unwrap_or_default();
        let wake = st.wake.clone();
        st.stats.live.last_read = Some(Instant::now());
        st.stats.live.job.start(wake, move || {
            tail.poll(&deadlock_core::game_session::console_log_path(&root));
            Ok(tail)
        });
    }
    let live = &st.stats.live;
    if live.checked_at.is_none_or(|at| at.elapsed() >= CHECK_EVERY) && !live.check_job.is_running() {
        start_check(st);
    }
    start_lookup(st);
    load_icons(st);
    changed
}

fn start_check(st: &mut State) {
    let Some(root) = st.game_root.clone() else { return };
    let wake = st.wake.clone();
    st.stats.live.checked_at = Some(Instant::now());
    st.stats.live.check_job.start(wake, move || live_bridge::check(&root).map_err(|e| e.to_string()));
}

/// Ranks and match counts for the lobby's accounts, once per match, only when the player
/// turned the lookup on and allows Player stats.
fn start_lookup(st: &mut State) {
    let o = options::get();
    if !o.live_lookup || !st.prefs.allow_stats || st.stats.live.lookup_job.is_running() {
        return;
    }
    let Some(state) = st.stats.live.tail.as_ref().map(|t| &t.state) else { return };
    let Some(current) = &state.current else { return };
    let ids: Vec<u32> = state
        .roster_accounts()
        .into_iter()
        .filter(|id| !st.stats.live.lookup.contains_key(id))
        .collect();
    if ids.is_empty() || st.stats.live.looked_up == Some(current.match_id) {
        return;
    }
    st.stats.live.looked_up = Some(current.match_id);
    let wake = st.wake.clone();
    st.stats.live.lookup_job.start(wake, move || {
        let client = crate::data::stats_client();
        let wide: Vec<u64> = ids.iter().map(|id| u64::from(*id)).collect();
        let mut out: HashMap<u32, Lookup> = HashMap::new();
        if let Ok(profiles) = client.fetch_profiles(&wide) {
            for profile in profiles {
                out.entry(profile.account_id as u32).or_default().matches_30d = Some(profile.matches_played_last_30d);
            }
        }
        for id in ids {
            if let Ok(rank) = client.fetch_rank(u64::from(id))
                && rank.badge > 0
            {
                let label = match rank.tier_name {
                    Some(name) => format!("{name} {}", rank.subrank),
                    None => format!("{}.{}", rank.rank, rank.subrank),
                };
                out.entry(id).or_default().rank = Some(label);
            }
        }
        Ok(out)
    });
}

pub fn icon_key(hero: &str) -> String {
    format!("stats-live-hero:{}", deadlock_core::locker::hero_asset_key(hero))
}

/// Hero chips from the bundled portraits, for heroes on the board. Local files only.
fn load_icons(st: &mut State) {
    let Some(board) = st.stats.live.tail.as_ref().and_then(|t| t.state.board()) else { return };
    let wanted: Vec<String> = board
        .friendly
        .iter()
        .chain(&board.enemy)
        .filter_map(|player| player.hero.clone())
        .filter(|hero| !st.stats.live.icons_loaded.contains(hero))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let dir = deadlock_core::locker::LockerAssetPaths::discover()
        .hero_portraits_dir
        .map(|dir| dir.join("chip-icons"));
    for hero in wanted {
        st.stats.live.icons_loaded.insert(hero.clone());
        let Some(dir) = &dir else { continue };
        let key = deadlock_core::locker::hero_asset_key(&hero);
        for file in [format!("{key}.png"), format!("{key}.webp"), format!("the_{key}.png"), format!("the_{key}.webp")] {
            if let Ok(bytes) = std::fs::read(dir.join(&file)) {
                st.pending_thumbs.push((icon_key(&hero), bytes));
                break;
            }
        }
    }
}

// --- bridge actions -------------------------------------------------------------------------

fn bridge_work() -> PathBuf {
    crate::data::workspace_dir().join("live-bridge")
}

/// Open the explanation before the bridge goes in.
pub fn ask_install(st: &mut State) {
    st.stats.live.confirm = true;
}

fn start_action(st: &mut State, action: Action) {
    if st.demo {
        st.stats.live.note = Some((tr("Demo data: nothing was changed.").to_string(), false));
        return;
    }
    if st.stats.live.action_job.is_running() {
        return;
    }
    // The game holds its addons open; every bridge change waits for it to close.
    if running(st) {
        st.stats.live.note = Some((tr("Close Deadlock first. The game keeps its mods open while it runs.").to_string(), true));
        return;
    }
    let Some(root) = st.game_root.clone() else {
        st.stats.live.note = Some((tr("Find the game folder first.").to_string(), true));
        return;
    };
    let check = st.stats.live.check.clone();
    let wake = st.wake.clone();
    st.stats.live.busy = Some(action);
    st.stats.live.note = None;
    st.stats.live.action_job.start(wake, move || {
        let missing = || tr("The live bridge is not installed.").to_string();
        let message = match action {
            Action::Install => install(&root)?,
            Action::Rebuild => {
                // Asked here too: the watcher that feeds `running` is not always on.
                if crate::screens::settings::tabs::game_running() {
                    return Err(tr("Close Deadlock first. The game keeps its mods open while it runs.").to_string());
                }
                let file = check.and_then(|c| c.file).ok_or_else(missing)?;
                live_bridge::rebuild_installed(&root, &file, &bridge_work(), true).map_err(|e| e.to_string())?;
                tr("Live bridge rebuilt for this game version.").to_string()
            }
            Action::SwitchOn => {
                let slot = check.and_then(|c| c.slot).ok_or_else(missing)?;
                deadlock_core::addons::set_enabled(&root, slot, true, true).map_err(|e| e.to_string())?;
                tr("Live bridge switched on.").to_string()
            }
            Action::Remove => {
                let slot = check.and_then(|c| c.slot).ok_or_else(missing)?;
                let trash = deadlock_core::trash::trash_root(&crate::data::data_dir());
                deadlock_core::trash::trash_slot(&root, &trash, slot, None, true).map_err(|e| e.to_string())?;
                tr("Live bridge removed. It is in the trash if you want it back.").to_string()
            }
        };
        Ok((action, message))
    });
}

/// Build from the game, then install like any local mod: the lowest free slot, switched on.
fn install(root: &std::path::Path) -> Result<String, String> {
    let work = bridge_work();
    let built = live_bridge::build(root, &work).map_err(|e| e.to_string())?;
    let request = deadlock_core::install::InstallRequest::new(built.display().to_string(), crate::data::workspace_dir())
        .map_err(|e| e.to_string())?;
    let plan = deadlock_core::install::plan_install(
        &request,
        &deadlock_core::install::PlanOptions { game_root: Some(root.to_path_buf()), ..Default::default() },
    )
    .map_err(|e| e.to_string())?;
    let record = deadlock_core::install::apply_install_with(
        &plan,
        true,
        deadlock_core::install::ApplyOptions { land_disabled: false },
    )
    .map_err(|e| e.to_string())?;
    let library = deadlock_core::import::library_path(&crate::data::data_dir());
    let _ = deadlock_core::import::record_meta(
        &library,
        deadlock_core::import::ModMeta {
            slot: plan.slot,
            size_bytes: std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0),
            name: live_bridge::MOD_NAME.to_string(),
            author: Some("Sigil".to_string()),
            thumbnail_url: None,
            gamebanana_id: None,
            source: live_bridge::MOD_SOURCE.to_string(),
            recorded_at: String::new(),
        },
    );
    let _ = std::fs::remove_file(&built);
    Ok(fill(tr("Live bridge installed in slot {}. Start Deadlock and load into a match."), &[&format!("{:02}", plan.slot)]))
}

fn finish_action(st: &mut State, result: Result<(Action, String), String>) {
    let action = st.stats.live.busy.take();
    let auto = std::mem::take(&mut st.stats.live.auto_rebuild);
    if auto && action == Some(Action::Rebuild) {
        match &result {
            Ok(_) => {
                if let Some(update) = st.stats.live.update.take() {
                    options::edit_quiet(|o| o.live_bridge_build = update.build);
                }
                crate::notify::push(
                    st,
                    crate::notify::Notice::success(tr("Live bridge rebuilt for the new game version"))
                        .detail(tr("Deadlock updated, so Sigil built the bridge again from your game. It reports from the next match.")),
                );
            }
            // The game started meanwhile: try again quietly once it closed.
            Err(error) if error == tr("Close Deadlock first. The game keeps its mods open while it runs.") => {}
            Err(error) => {
                // Not again for this build; the panel keeps Rebuild.
                if let Some(update) = st.stats.live.update.take() {
                    options::edit_quiet(|o| o.live_bridge_build = update.build);
                }
                crate::notify::push(
                    st,
                    crate::notify::Notice::warning(tr("The live bridge could not be rebuilt"))
                        .detail(error.clone())
                        .action(tr("Open Stats"), |st: &mut State| st.go(Screen::Stats)),
                );
            }
        }
    }
    match result {
        Ok((done, message)) => {
            st.stats.live.note = Some((message, false));
            match done {
                Action::Install | Action::SwitchOn => options::edit(st, |o| o.live_bridge = true),
                Action::Remove => options::edit(st, |o| o.live_bridge = false),
                Action::Rebuild => {}
            }
            if done == Action::Install && !options::get().condebug {
                crate::screens::settings::watch::turn_on_condebug(st);
            }
        }
        Err(error) => {
            if action == Some(Action::Install) {
                options::edit(st, |o| o.live_bridge = false);
            }
            st.stats.live.note = Some((error, true));
        }
    }
    crate::data::rescan_game(st);
    start_check(st);
}

// --- after a game update ------------------------------------------------------------------

/// While the bridge is on: read the game build every minute (two stats and a small read on a
/// worker) and check the bridge again when it changed. Runs on every page.
fn follow_updates(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(Ok(build)) = st.stats.live.build_job.try_take() {
        let known = options::get().live_bridge_build.clone();
        if let Some(build) = build
            && build != known
            && st.stats.live.update.as_ref().is_none_or(|u| u.build != build)
        {
            st.stats.live.update = Some(Update { build, outdated: false });
            if !st.stats.live.check_job.is_running() {
                start_check(st);
            }
            changed = true;
        }
    }
    if st.demo || rune::is_headless() || !options::get().live_bridge {
        return changed;
    }
    let Some(root) = st.game_root.clone() else { return changed };
    let live = &st.stats.live;
    if live.build_read_at.is_none_or(|at| at.elapsed() >= BUILD_EVERY) && !live.build_job.is_running() {
        st.stats.live.build_read_at = Some(Instant::now());
        let wake = st.wake.clone();
        st.stats.live.build_job.start(wake, move || Ok(live_bridge::game_build(&root)));
    }
    // An outdated bridge found while the game ran: act once it closed.
    let waited = st.stats.live.tried_at.is_none_or(|at| at.elapsed() >= BUILD_EVERY);
    if st.stats.live.update.as_ref().is_some_and(|u| u.outdated) && waited && !running(st) && st.stats.live.busy.is_none() {
        rebuild_after_update(st);
        changed = true;
    }
    changed
}

/// The check that a new build asked for came back: record it, or follow the update.
fn after_check(st: &mut State) {
    let Some(update) = st.stats.live.update.clone() else { return };
    if update.outdated {
        return;
    }
    match st.stats.live.check.as_ref().map(|c| c.state) {
        Some(BridgeState::Outdated) => {
            st.stats.live.update = Some(Update { outdated: true, ..update });
            let waited = st.stats.live.tried_at.is_none_or(|at| at.elapsed() >= BUILD_EVERY);
            if waited && !running(st) && st.stats.live.busy.is_none() {
                rebuild_after_update(st);
            }
        }
        // Current, off, not there or not comparable: nothing to follow for this build.
        Some(_) => {
            st.stats.live.update = None;
            options::edit_quiet(|o| o.live_bridge_build = update.build);
        }
        None => st.stats.live.update = None,
    }
}

/// Rebuild by itself when the player said so once, else ask once for this build.
fn rebuild_after_update(st: &mut State) {
    let Some(update) = st.stats.live.update.clone() else { return };
    if options::get().live_bridge_auto {
        st.stats.live.tried_at = Some(Instant::now());
        st.stats.live.auto_rebuild = true;
        start_action(st, Action::Rebuild);
        if st.stats.live.busy.is_none() {
            // Refused before it started (the game runs, a job runs): a minute later again.
            st.stats.live.auto_rebuild = false;
            st.stats.live.note = None;
        }
        return;
    }
    // Asked once per build: the answer, or no answer, does not ask again for it.
    st.stats.live.update = None;
    options::edit_quiet(|o| o.live_bridge_build = update.build.clone());
    let build = update.build;
    crate::notify::push(
        st,
        crate::notify::Notice::warning(tr("Deadlock updated: rebuild the live bridge?"))
            .detail(tr("The bridge was built for the old game version and stays silent until it is rebuilt. Settings > Game > Live match can rebuild it by itself next time."))
            .action(tr("Rebuild"), move |st: &mut State| {
                st.stats.live.update = Some(Update { build, outdated: true });
                st.stats.live.auto_rebuild = true;
                start_action(st, Action::Rebuild);
                if st.stats.live.busy.is_none() {
                    st.stats.live.auto_rebuild = false;
                }
            }),
    );
}

/// Scene: the question after a game update.
pub fn scene_update_ask(st: &mut State) {
    st.stats.live.update = Some(Update { build: "6200:1-1".into(), outdated: true });
    options::edit_quiet(|o| o.live_bridge_auto = false);
    rebuild_after_update(st);
}

/// Settings > Game > Live match: rebuild after game updates without asking.
pub fn toggle_auto_rebuild(st: &mut State) {
    options::edit(st, |o| o.live_bridge_auto = !o.live_bridge_auto);
}

/// The settings switch: on asks first, off removes the bridge.
pub fn toggle_from_settings(st: &mut State) {
    let installed = st.stats.live.check.as_ref().is_some_and(BridgeCheck::installed);
    if installed {
        start_action(st, Action::Remove);
    } else if options::get().live_bridge {
        options::edit(st, |o| o.live_bridge = false);
    } else {
        ask_install(st);
    }
}

/// Settings reads the bridge's state through this.
pub fn bridge_line(st: &State) -> &'static str {
    match st.stats.live.check.as_ref().map(|c| c.state) {
        None => "",
        Some(BridgeState::NotInstalled) => tr("Not installed."),
        Some(BridgeState::Off) => tr("Installed, switched off."),
        Some(BridgeState::Current) => tr("Installed and up to date."),
        Some(BridgeState::Outdated) => tr("Built for an older game version. Rebuild it."),
        Some(BridgeState::Incompatible) => tr("This game version moved the top bar. Turn the bridge off for now."),
        Some(BridgeState::Unknown) => tr("Installed. The game's files could not be read to compare."),
    }
}

/// Start the bridge check once when Settings shows it.
pub fn ensure_checked(st: &mut State) {
    if st.stats.live.check.is_none() && !st.stats.live.check_job.is_running() && !st.demo && !rune::is_headless() {
        start_check(st);
    }
}

// --- view -----------------------------------------------------------------------------------

fn clock(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn now_ms() -> i64 {
    deadlock_core::live_match::now_ms()
}

fn since(at: Option<i64>) -> Option<u64> {
    at.map(|at| ((now_ms() - at).max(0) / 1000) as u64)
}

fn hero_title(name: &str) -> String {
    let plain = name.trim_start_matches("hero_").replace('_', " ");
    super::shared::capitalise(&plain)
}

/// The panel at the top of Stats, or nothing when there is nothing live to show.
pub fn panel(s: &State, u: &Ui) -> Option<El> {
    if !options::get().live_view {
        return None;
    }
    let live = &s.stats.live;
    let tail = live.tail.as_ref();
    let game = running(s);
    // With the game closed, the last match stays for two hours, then the panel goes away.
    let last = tail
        .and_then(|t| t.state.last.as_ref())
        .filter(|last| game || since(last.ended_at).is_some_and(|ago| ago < 2 * 3600));
    if !game && last.is_none() {
        return None;
    }
    let mut body = col().w_full().gap(14.0);
    if game && tail.is_some_and(|t| !t.found) {
        body = body
            .child(headline(u, tr("Console log is off"), tr("Live match reads the game's console.log, which Deadlock writes only with -condebug.")))
            .child(
                row().gap(8.0).child(
                    u.action(tr("Add -condebug to the launch options"), ActionKind::Secondary)
                        .key("stats-live-condebug")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(crate::screens::settings::watch::turn_on_condebug),
                ),
            )
            .child(text(tr("Then restart Deadlock.")).font(SMALL).color(u.muted));
        return Some(frame(u, body));
    }
    let state = tail.map(|t| t.state.clone()).unwrap_or_default();
    if let Some(current) = &state.current {
        let board = state.board();
        let head = match current.phase {
            MatchPhase::Pregame => headline(u, tr("Match found"), tr("Loading in. Both teams show up when the top bar appears.")),
            MatchPhase::InProgress => match_head(s, u, &state, board.as_ref()),
            MatchPhase::Ended => headline(u, tr("Match over"), tr("The post-game screen. The board stays until you queue again.")),
        };
        body = body.child(head);
        match &board {
            Some(board) => body = body.child(board_view(s, u, board, false)),
            None if current.phase == MatchPhase::InProgress => body = body.child(no_board(s, u, &state)),
            None => {}
        }
        body = body.child(match_meta(u, current.match_id, None));
    } else if let Some(queued) = state.queued_since {
        body = body.child(headline(
            u,
            &fill(tr("Searching for a match, {}"), &[&clock(since(Some(queued)).unwrap_or(0))]),
            tr("Lineups appear on the versus screen."),
        ));
    } else if let Some(last) = last {
        body = body.child(finished(s, u, last));
    } else {
        body = body.child(headline(u, tr("No match running"), tr("You are in the menus or the hideout. The board fills in when you load into a match.")));
    }
    body = body.children(bridge_notice(s, u));
    if state.recent.len() > 1 {
        body = body.child(recent(u, &state.recent, state.current.as_ref().map(|c| c.match_id)));
    }
    if let Some((message, error)) = &live.note {
        body = body.child(text(message.clone()).font(SMALL).color(if *error { u.amber } else { u.muted }).wrap());
    }
    Some(frame(u, body))
}

fn frame(u: &Ui, body: El) -> El {
    col()
        .key("stats-live")
        .w_full()
        .padding(20.0)
        .gap(12.0)
        .radius(crate::ui::r(4.0))
        .border(1.0, u.hair_strong)
        .fill(Color::WHITE.alpha(0.02))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(el().size(8.0, 8.0).radius(4.0).fill(u.rose).a11y_hidden())
                .child(text(tr("Live match")).font(H2).color(u.ink)),
        )
        .child(body)
}

fn headline(u: &Ui, title: &str, detail: &str) -> El {
    col()
        .gap(2.0)
        .child(text(title.to_string()).font(BODY_MED).color(u.ink))
        .child(text(detail.to_string()).font(SMALL).color(u.muted).wrap())
}

/// Hero, mode, party and the clock: what the console knows, plus the bridge's clock.
fn match_head(s: &State, u: &Ui, state: &LiveState, board: Option<&Board>) -> El {
    let session = &state.session;
    let mut facts = Vec::new();
    let hero = board
        .and_then(Board::me)
        .and_then(|me| me.hero.clone())
        .or_else(|| session.hero.as_deref().map(hero_title));
    if !session.mode().is_empty() {
        facts.push(tr(session.mode()).to_string());
    }
    if session.party > 1 {
        facts.push(fill(tr("party of {}"), &[&session.party.to_string()]));
    }
    let clock_s = board
        .and_then(|b| b.clock_s.map(u64::from))
        .or_else(|| since(state.current.as_ref().and_then(|c| c.started_at)));
    let mut head = row().w_full().gap(12.0).center_y();
    if let Some(hero) = &hero {
        head = head.child(chip(s, u, hero, 36.0));
    }
    head = head
        .child(
            col()
                .gap(2.0)
                .child(
                    text(match &hero {
                        Some(hero) => fill(tr("In a match as {}"), &[hero.as_str()]),
                        None => tr("In a match").to_string(),
                    })
                    .font(BODY_MED)
                    .color(u.ink),
                )
                .child(text(facts.join("  ·  ")).font(SMALL).color(u.muted)),
        )
        .child(spacer());
    if let Some(clock_s) = clock_s {
        head = head.child(
            col()
                .items_end()
                .child(text(clock(clock_s)).font(NUM).color(u.ink))
                .child(
                    text(if board.is_some() { tr("Match clock") } else { tr("Since the horn") })
                        .font(SMALL)
                        .color(u.muted),
                ),
        );
    }
    head
}

fn chip(s: &State, u: &Ui, hero: &str, size: f32) -> El {
    match s.thumbnails.get(&icon_key(hero)) {
        Some(pixels) => el()
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
            .child(text(hero.chars().next().map(String::from).unwrap_or_default()).font(SMALL).color(u.muted)),
    }
}

/// In a match without a board: why, and what turns it on.
fn no_board(s: &State, u: &Ui, state: &LiveState) -> El {
    let installed = s.stats.live.check.as_ref().is_some_and(BridgeCheck::installed);
    if !installed {
        return col()
            .gap(8.0)
            .child(
                text(tr("The console only says that a match runs. The live bridge adds both teams' souls, K/D/A, levels and the clock."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(
                row().child(
                    u.action(tr("Set up the live bridge"), ActionKind::Secondary)
                        .key("stats-live-setup")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(ask_install),
                ),
            );
    }
    let reason = if state.bridge_heard {
        tr("The bridge went quiet. It reports a few seconds after the top bar appears.")
    } else {
        tr("No report yet. It starts a few seconds after the top bar appears. If you installed the bridge while Deadlock was open, restart the game.")
    };
    headline(u, tr("Waiting for the live bridge"), reason)
}

/// The board: the top bar summary, the player's own line, the soul lead and both teams.
fn board_view(s: &State, u: &Ui, board: &Board, finished: bool) -> El {
    let quiet = crate::ui::Tone::Quiet;
    let ours = Board::souls(&board.friendly);
    let theirs = Board::souls(&board.enemy);
    let mut top = row()
        .w_full()
        .gap(32.0)
        .child(u.stat(signed(board.lead()), tr("Soul lead"), quiet))
        .child(u.stat(format!("{}  :  {}", souls(ours), souls(theirs)), tr("Team souls"), quiet))
        .child(u.stat(
            format!("{}  :  {}", Board::kills(&board.friendly), Board::kills(&board.enemy)),
            tr("Kills"),
            quiet,
        ));
    if let Some(me) = board.me() {
        top = top.child(u.stat(me.souls.map(|n| souls(u64::from(n))).unwrap_or_else(|| "-".into()), tr("Your souls"), quiet));
        top = top.child(u.stat(
            board.souls_per_minute().map(|n| super::shared::thousands(u64::from(n))).unwrap_or_else(|| "-".into()),
            tr("Your souls a minute"),
            quiet,
        ));
    }
    let mut out = col().w_full().gap(16.0).child(top);
    if board.samples.len() >= 2 {
        out = out.child(curve(u, board));
    }
    let names = |team: Team| match team {
        Team::HiddenKing => tr("Hidden King"),
        Team::Archmother => tr("Archmother"),
    };
    let other = match board.team {
        Team::HiddenKing => Team::Archmother,
        Team::Archmother => Team::HiddenKing,
    };
    out = out
        .child(team_table(s, u, &fill(tr("Your team, {}"), &[names(board.team)]), &board.friendly, "friendly"))
        .child(team_table(s, u, &fill(tr("The other team, {}"), &[names(other)]), &board.enemy, "enemy"));
    if !finished {
        let (line, color) = match board.reported_at {
            Some(at) if board.stale(now_ms()) => {
                (fill(tr("No signal from the bridge for {}."), &[&clock(since(Some(at)).unwrap_or(0))]), u.amber)
            }
            Some(at) => (fill(tr("From the live bridge, {} s ago."), &[&since(Some(at)).unwrap_or(0).to_string()]), u.muted),
            None => (tr("From the live bridge.").to_string(), u.muted),
        };
        out = out.child(text(line).font(SMALL).color(color));
    }
    out
}

/// The soul lead along the match clock, from the bridge's own samples.
fn curve(u: &Ui, board: &Board) -> El {
    let values: Vec<f64> = board.samples.iter().map(|sample| sample.lead() as f64).collect();
    let end = board.samples.last().map(|s| s.clock_s).unwrap_or(1).max(1) as f32;
    let start = board.samples.first().map(|s| s.clock_s).unwrap_or(0) as f32;
    let span = (end - start).max(1.0);
    let peak = values.iter().fold(1000.0_f64, |peak, v| peak.max(v.abs()));
    let mid = CURVE_H / 2.0;
    let pad = 8.0;
    let points: Vec<[f32; 2]> = board
        .samples
        .iter()
        .zip(&values)
        .map(|(sample, value)| {
            [(sample.clock_s as f32 - start) / span * CURVE_VB, mid - (*value / peak) as f32 * (mid - pad)]
        })
        .collect();
    let mut shape = Shape::new()
        .view_box(CURVE_VB, CURVE_H)
        .line([0.0, mid], [CURVE_VB, mid], 1.0, rune::LineCap::Butt, Color::WHITE.alpha(0.14));
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let tint = if (a[1] + b[1]) / 2.0 <= mid { u.teal.alpha(0.14) } else { u.rose.alpha(0.14) };
        shape = shape.fill(vec![a, b, [b[0], mid], [a[0], mid]], tint);
    }
    shape = shape.stroke(points, 1.5, u.prose);
    col()
        .w_full()
        .gap(4.0)
        .child(
            row()
                .w_full()
                .child(text(tr("Soul lead this match")).font(SMALL).color(u.muted))
                .child(spacer())
                .child(text(tr("Above the line: your team ahead")).font(SMALL).color(u.muted)),
        )
        .child(canvas(shape).w_full().h(CURVE_H).role("img").label(tr("Soul lead this match")))
}

/// Number columns: level, K/D/A, souls, objective damage, then the lookup's rank and matches.
const COLS: [(&str, f32); 6] = [
    ("Lvl", 36.0),
    ("K / D / A", 84.0),
    ("Souls", 64.0),
    ("Obj dmg", 64.0),
    ("Rank", 120.0),
    ("30 days", 64.0),
];

fn team_table(s: &State, u: &Ui, title: &str, players: &[LivePlayer], key: &str) -> El {
    let lookup = options::get().live_lookup && s.prefs.allow_stats;
    let shown = if lookup { COLS.len() } else { 4 };
    let mut head = row()
        .w_full()
        .h(28.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(title.to_string()).font(SMALL).color(u.muted).grow(1.0).min_w(0.0));
    for (label, w) in COLS.iter().take(shown) {
        head = head.child(text(tr(label)).font(SMALL).color(u.muted).w(*w).align(rune::Align::End));
    }
    let mut table = col().w_full().child(head);
    if players.is_empty() {
        return table.child(text(tr("No report from this team yet.")).font(SMALL).color(u.muted).pt(8.0));
    }
    let mut sorted: Vec<&LivePlayer> = players.iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.souls.unwrap_or(0)));
    for (index, player) in sorted.into_iter().enumerate() {
        let dash = || "-".to_string();
        let num = |v: Option<i32>| v.map(|n| n.to_string()).unwrap_or_else(dash);
        let found = player.account_id.and_then(|id| s.stats.live.lookup.get(&id));
        let values = [
            num(player.level),
            format!("{} / {} / {}", num(player.kills), num(player.deaths), num(player.assists)),
            player.souls.map(|n| souls(u64::from(n))).unwrap_or_else(dash),
            player.objective_damage.map(|n| souls(n.max(0) as u64)).unwrap_or_else(dash),
            found.and_then(|f| f.rank.clone()).unwrap_or_else(dash),
            found.and_then(|f| f.matches_30d).map(|n| n.to_string()).unwrap_or_else(dash),
        ];
        let hero = player.hero.clone().unwrap_or_default();
        let face = if hero.is_empty() { player.name.clone() } else { hero.clone() };
        let mut name = row()
            .grow(1.0)
            .min_w(0.0)
            .gap(10.0)
            .center_y()
            .child(chip(s, u, &face, 24.0))
            .child(
                col()
                    .min_w(0.0)
                    .child(
                        text(player.name.clone())
                            .font(if player.local { BODY_MED } else { BODY })
                            .color(if player.dead { u.muted } else { u.ink })
                            .truncate(),
                    )
                    .child(text(hero).font(SMALL).color(u.muted).truncate()),
            );
        if player.local {
            name = name.child(text(tr("You")).font(SMALL).color(u.accent));
        }
        if player.dead {
            name = name.child(text(tr("Respawning")).font(SMALL).color(u.rose));
        }
        let mut line = row()
            .key(format!("stats-live-{key}-{index}"))
            .w_full()
            .h(44.0)
            .gap(12.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(name);
        for (value, (_, w)) in values.into_iter().zip(COLS.iter()).take(shown) {
            line = line.child(text(value).font(TNUM).color(u.prose).w(*w).align(rune::Align::End));
        }
        table = table.child(line);
    }
    table
}

fn match_meta(u: &Ui, match_id: u64, extra: Option<El>) -> El {
    let mut r = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(text(fill(tr("Match {}"), &[&match_id.to_string()])).font(TNUM_SMALL).color(u.muted))
        .child(spacer());
    if let Some(extra) = extra {
        r = r.child(extra);
    }
    r.child(
        u.icon_action(&crate::icons::EXTERNAL_LINK, tr("Open on Statlocker"), ActionKind::Ghost)
            .key(format!("stats-live-statlocker-{match_id}"))
            .role("link")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut rune::Tx| open_statlocker(st, tx, &statlocker_match_url(match_id))),
    )
}

/// The last match, kept until the next queue.
fn finished(s: &State, u: &Ui, last: &FinishedMatch) -> El {
    let length = last.duration_s().map(|d| clock(u64::from(d))).unwrap_or_else(|| "-".into());
    let ago = since(last.ended_at).map(|s| (s / 60).to_string()).unwrap_or_else(|| "?".into());
    let mut out = col().w_full().gap(14.0).child(headline(
        u,
        &fill(tr("Match ended, {} long"), &[&length]),
        &fill(tr("Ended {} min ago. The full scoreboard opens once deadlock-api has the match, usually a few minutes."), &[&ago]),
    ));
    if let Some(board) = &last.board {
        out = out.child(board_view(s, u, board, true));
    }
    let match_id = last.match_id;
    let scoreboard = (s.stats.snapshot.is_some() && s.prefs.allow_stats).then(|| {
        u.action(tr("Scoreboard"), ActionKind::Secondary)
            .key("stats-live-scoreboard")
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |st: &mut State, tx: &mut rune::Tx| super::nav::open_match(st, tx, match_id))
    });
    out.child(match_meta(u, match_id, scoreboard))
}

fn recent(u: &Ui, ids: &[u64], current: Option<u64>) -> El {
    let earlier: Vec<String> = ids.iter().filter(|id| Some(**id) != current).rev().take(6).map(u64::to_string).collect();
    text(fill(tr("Earlier this session: {}"), &[&earlier.join(", ")])).font(TNUM_SMALL).color(u.muted).wrap()
}

/// One line about the bridge when it needs the player: off, outdated, shadowed, incompatible.
fn bridge_notice(s: &State, u: &Ui) -> Option<El> {
    let live = &s.stats.live;
    let check = live.check.as_ref()?;
    let busy = live.busy.is_some();
    let (message, action): (String, Option<(&str, Action)>) = match check.state {
        BridgeState::Off => (
            tr("The live bridge is installed but switched off, so the game does not load it.").to_string(),
            Some((tr("Switch on"), Action::SwitchOn)),
        ),
        BridgeState::Outdated => (
            tr("The live bridge was built for an older game version. Rebuild it from your current game.").to_string(),
            Some((tr("Rebuild"), Action::Rebuild)),
        ),
        BridgeState::Incompatible => (
            tr("This game version changed the top bar in a way the bridge cannot follow yet. Remove it until Sigil is updated, or the top bar may look wrong.").to_string(),
            Some((tr("Remove"), Action::Remove)),
        ),
        _ => {
            let slot = check.shadowed()?;
            (
                fill(tr("The mod in slot {} also replaces the top bar and wins over the bridge, so the bridge stays silent."), &[&format!("{slot:02}")]),
                None,
            )
        }
    };
    let mut r = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(text(message).font(SMALL).color(u.amber).wrap().grow(1.0).min_w(0.0));
    if let Some((label, action)) = action {
        r = r.child(
            u.action(if busy { tr("Working") } else { label }, ActionKind::Secondary)
                .key("stats-live-bridge-action")
                .opacity(if busy { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| start_action(st, action)),
        );
    }
    Some(r)
}

/// The explanation before the bridge is installed.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if !s.stats.live.confirm {
        return None;
    }
    let close = |st: &mut State| st.stats.live.confirm = false;
    let line = |t: &str| text(t.to_string()).font(BODY).color(u.prose).wrap().w_full();
    Some(
        u.dialog_shell()
            .key("stats-live-dialog")
            .role("dialog")
            .label(tr("Install the live bridge?"))
            .w(520.0)
            .padding(24.0)
            .gap(12.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(text(tr("Install the live bridge?")).font(crate::ui::H1).color(u.ink))
            .child(line(tr("The live bridge is a small HUD mod Sigil builds from your game's own top bar. Every two seconds it prints what the top bar shows (heroes, souls, levels, K/D/A, objective damage and the clock) into the game's console log, and Sigil reads it from there.")))
            .child(line(tr("It draws nothing in game and sends nothing anywhere. It needs -condebug in the launch options; Sigil adds it if it is missing.")))
            .child(line(tr("It goes into a free addon slot like any mod and is removed when you turn it off. After a game update Sigil asks to rebuild it. Another mod that replaces the top bar can hide it.")))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .pt(8.0)
                    .child(spacer())
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key("stats-live-dialog-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(
                        u.action(tr("Install the bridge"), ActionKind::Primary)
                            .key("stats-live-dialog-install")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                st.stats.live.confirm = false;
                                start_action(st, Action::Install);
                            }),
                    ),
            ),
    )
}

// --- scenes ---------------------------------------------------------------------------------

/// A console line stamped `ago` seconds before now.
fn stamped(ago: i64, line: &str) -> String {
    format!("{} {line}", deadlock_core::live_match::stamp_ago(ago))
}

fn fold_scene(s: &mut State, lines: &[String], bridge: Option<BridgeState>) {
    let mut state = LiveState::default();
    deadlock_core::live_match::fold_text_now(&mut state, &lines.join("\n"));
    s.stats.live.tail = Some(LiveTail::with_state(state));
    s.stats.live.scene_running = Some(true);
    s.stats.live.check = bridge.map(|state| BridgeCheck {
        state,
        slot: (state != BridgeState::NotInstalled).then_some(12),
        file: None,
        others: Vec::new(),
        note: None,
    });
}

/// One scene player: name, hero, souls, level, K/D/A, objective damage, local, dead.
type ScenePlayer<'a> = (&'a str, &'a str, u32, i32, [i32; 3], i32, bool, bool);

fn report(side: &str, clock: &str, players: &[ScenePlayer]) -> String {
    let rows: Vec<String> = players
        .iter()
        .map(|(name, hero, souls, level, kda, obj, me, dead)| {
            format!(
                r#"{{"name":"{name}","hero":"{hero}","souls":"{souls}","level":{level},"k":{},"d":{},"a":{},"obj":{obj},"me":{me},"dead":{dead}}}"#,
                kda[0], kda[1], kda[2]
            )
        })
        .collect();
    format!(
        "[PanoramaScript] {} {{\"v\":1,\"side\":\"{side}\",\"clock\":\"{clock}\",\"players\":[{}]}}",
        deadlock_core::live_match::BRIDGE_TAG,
        rows.join(",")
    )
}

fn lobby(created_ago: i64) -> Vec<String> {
    vec![
        stamped(created_ago + 400, "[GCClient] Send msg 9010 (k_EMsgClientToGCStartMatchmaking), 263 bytes"),
        stamped(created_ago + 300, "Lobby 98014987589037600 for Match 97000001 created"),
        stamped(created_ago + 290, "OnGameStateChanged: GameInProgress (7)"),
        stamped(created_ago + 120, "Lobby 98014987589037600 for Match 97000001 destroyed"),
        stamped(created_ago + 60, "[GCClient] Send msg 9010 (k_EMsgClientToGCStartMatchmaking), 263 bytes"),
        stamped(created_ago, "Lobby 102014987589037613 for Match 97003071 created"),
    ]
}

fn running_match(bridge: bool) -> Vec<String> {
    let mut lines = lobby(1500);
    lines.push(stamped(1490, "[Client] Map: \"dl_streets\""));
    lines.push(stamped(1480, "[Server] Loaded hero 13/hero_haze"));
    lines.push(stamped(1460, "OnGameStateChanged: GameInProgress (7)"));
    if bridge {
        for step in 0..=(24 * 6) {
            let t = step * 10;
            let ago = 1460 - t as i64;
            let clock = format!("{}:{:02}", t / 60, t % 60);
            let wave = (f64::from(t) / 240.0).sin();
            let ours = |base: u32| base * t / 60 + (wave * 400.0).max(0.0) as u32;
            let theirs = |base: u32| base * t / 60 + ((-wave) * 500.0).max(0.0) as u32;
            let k = (t / 120) as i32;
            let l = |plus: i32| (k + plus).clamp(1, 30);
            lines.push(stamped(
                ago,
                &report(
                    "friendly",
                    &clock,
                    &[
                        ("Wren", "Haze", ours(1010), l(1), [k + 2, k / 2, k], 4200 * k / 12, true, false),
                        ("Ivo", "Abrams", ours(880), l(1), [k / 2, k / 2 + 1, k + 3], 9100 * k / 12, false, step % 9 == 0),
                        ("Mara", "Dynamo", ours(760), l(0), [k / 3, k / 3, k + 5], 2100 * k / 12, false, false),
                        ("Tess", "Seven", ours(940), l(1), [k, k / 2, k / 2], 6100 * k / 12, false, false),
                        ("Odd", "McGinnis", ours(720), l(0), [k / 4, k / 3, k + 1], 12000 * k / 12, false, false),
                        ("Kit", "Lady Geist", ours(810), l(0), [k / 2, k / 2 + 1, k], 3000 * k / 12, false, false),
                    ],
                ),
            ));
            lines.push(stamped(
                ago,
                &report(
                    "enemy",
                    &clock,
                    &[
                        ("Rook", "Wraith", theirs(990), l(1), [k + 1, k / 2 + 1, k / 2], 3900 * k / 12, false, false),
                        ("Pell", "Bebop", theirs(780), l(0), [k / 2, k / 2, k + 2], 2600 * k / 12, false, false),
                        ("Juno", "Kelvin", theirs(700), l(0), [k / 4, k / 3, k + 4], 1800 * k / 12, false, step % 7 == 0),
                        ("Sable", "Pocket", theirs(930), l(1), [k, k / 2, k / 2], 5200 * k / 12, false, false),
                        ("Nox", "Viscous", theirs(760), l(0), [k / 3, k / 2, k], 7400 * k / 12, false, false),
                        ("Ash", "Warden", theirs(840), l(0), [k / 2, k / 2, k / 2], 6600 * k / 12, false, false),
                    ],
                ),
            ));
        }
    }
    lines
}

/// The game runs, no match: the hideout.
pub fn scene_idle(s: &mut State) {
    super::load_fixture(s);
    fold_scene(s, &[stamped(300, "[Client] Map: \"dl_hideout\"")], Some(BridgeState::NotInstalled));
}

/// In a match, no bridge: the console's facts and the setup offer.
pub fn scene_console(s: &mut State) {
    super::load_fixture(s);
    fold_scene(s, &running_match(false), Some(BridgeState::NotInstalled));
}

/// In a match with the bridge reporting.
pub fn scene_bridge(s: &mut State) {
    super::load_fixture(s);
    fold_scene(s, &running_match(true), Some(BridgeState::Current));
}

/// The bridge installed, no report yet, and built for an older game.
pub fn scene_waiting(s: &mut State) {
    super::load_fixture(s);
    fold_scene(s, &running_match(false), Some(BridgeState::Outdated));
}

/// The match ended: the last board stays with the scoreboard offer.
pub fn scene_ended(s: &mut State) {
    super::load_fixture(s);
    let mut lines = running_match(true);
    lines.push(stamped(0, "Lobby 102014987589037613 for Match 97003071 destroyed"));
    fold_scene(s, &lines, Some(BridgeState::Current));
    s.stats.live.scene_running = Some(false);
}

/// The explanation before the bridge goes in.
pub fn scene_confirm(s: &mut State) {
    scene_console(s);
    s.stats.live.confirm = true;
}

/// The game runs without -condebug.
pub fn scene_condebug(s: &mut State) {
    super::load_fixture(s);
    s.stats.live.tail = Some(LiveTail::default());
    s.stats.live.scene_running = Some(true);
}

/// Settings > Game with the Live match rows, the bridge installed and current.
pub fn scene_settings(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Game);
    s.stats.live.check = Some(BridgeCheck {
        state: BridgeState::Current,
        slot: Some(12),
        file: None,
        others: Vec::new(),
        note: None,
    });
    options::edit(s, |o| o.live_bridge = true);
}
