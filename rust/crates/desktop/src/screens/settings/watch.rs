//! Game watching: is Deadlock running, what happens when it closes, and what the console log
//! says while it runs. Settings > Game turns each part on:
//!
//! - Block changes while the game runs: installs and toggles ask the user to wait
//!   ([`refuse_while_running`] is the guard other screens call).
//! - Remove the mod path when the game closes: `gameinfo.gi` loses `Game citadel/addons`
//!   (a backup first); the next modded launch from Sigil puts it back.
//! - Check for crashes after a launch: a new crash dump, or a crash exit code in Steam's log,
//!   raises a notification.
//! - Fix before launch / reapply at startup: `gameinfo.gi` is repaired when a game update or
//!   the close cleanup took the path out, and a pending vanilla stash goes back at startup.
//! - Discord game presence reads `console.log` (the game writes it with `-condebug`).
//!
//! The process check runs off the UI thread every few seconds, only while one of these is on.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use deadlock_core::game_session::{self, Session, Tail};
use rune::{El, Tx, col};

use super::i18n::{fill, tr};
use super::options;
use super::{LIST_GAP, detail, disabled, section, setting_row, toggle_row};
use crate::app::State;
use crate::jobs::Job;
use crate::notify::{self, Notice};
use crate::ui::{ActionKind, Ui};

/// The last answer of the process check, for guards in other screens.
static RUNNING: AtomicBool = AtomicBool::new(false);

const CHECK_EVERY: Duration = Duration::from_secs(4);
const TAIL_EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct Watch {
    pub job: Job<bool>,
    pub last_check: Option<Instant>,
    pub running: bool,
    /// When the current session started (a launch from Sigil, or the game showing up).
    pub session_from: Option<SystemTime>,
    pub tail_job: Job<(Tail, bool)>,
    pub tail: Option<Tail>,
    pub last_tail: Option<Instant>,
    /// Crash dumps the last session left, newest first.
    pub last_crash: Vec<PathBuf>,
    pub started: bool,
}

/// True while the process check last saw Deadlock.
pub fn game_is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// True when changes to the addons folder should wait for the game to close.
pub fn changes_blocked() -> bool {
    options::block_while_running() && game_is_running()
}

/// The guard other screens call before writing to the addons folder. Shows why and returns
/// true when the change has to wait.
pub fn refuse_while_running(st: &mut State) -> bool {
    if !changes_blocked() {
        return false;
    }
    notify::push(
        st,
        Notice::warning(tr("Deadlock is running"))
            .detail(tr("Close the game first, or turn off Block changes while the game runs in Settings.")),
    );
    true
}

/// The console session, when game presence is on and the log has been read.
pub fn session(st: &State) -> Option<&Session> {
    st.settings.extra.watch.tail.as_ref().map(|t| &t.session)
}

fn wanted(st: &State) -> bool {
    let o = options::get();
    o.block_while_running
        || o.remove_paths_on_close
        || (o.crash_check && st.settings.extra.watch.session_from.is_some())
        || (o.presence_game && st.prefs.discord)
}

/// Startup work: reapply mods when asked.
pub fn on_startup(st: &mut State) {
    if st.demo || rune::is_headless() {
        return;
    }
    if options::get().auto_reapply {
        reapply(st);
    }
}

/// Put mods back after a game update or a vanilla run: restore a pending vanilla stash, then
/// repair gameinfo.gi. Both keep their own backups.
pub fn reapply(st: &mut State) {
    let Some(root) = st.game_root.clone() else { return };
    let data = crate::data::data_dir();
    let mut did = Vec::new();
    if deadlock_core::launch::stash_status(&data).is_some()
        && deadlock_core::launch::restore_vanilla(&data, true).is_ok()
    {
        did.push(tr("put back the mods a vanilla run set aside"));
    }
    let gi = deadlock_core::deploy::gameinfo_path(&root);
    if let Ok(repair) = deadlock_core::deploy::repair_gameinfo(&gi, true)
        && repair.changed
    {
        did.push(tr("added citadel/addons to gameinfo.gi again"));
    }
    if !did.is_empty() {
        super::log::info(format!("Reapplied mods: {}", did.join(", ")));
        notify::push(st, Notice::success(tr("Mods reapplied")).detail(did.join(". ")));
        crate::data::rescan_game(st);
    }
}

/// Called right after Steam was asked to start the game.
pub fn on_launch(st: &mut State) {
    let o = options::get();
    if st.demo || rune::is_headless() {
        return;
    }
    let vanilla = st.launch_vanilla && st.prefs.single_launch;
    if !vanilla
        && (o.fix_before_launch || o.remove_paths_on_close)
        && let Some(root) = st.game_root.clone()
    {
        let gi = deadlock_core::deploy::gameinfo_path(&root);
        if let Ok(repair) = deadlock_core::deploy::repair_gameinfo(&gi, true)
            && repair.changed
        {
            super::log::info("Added citadel/addons to gameinfo.gi before launch");
        }
    }
    let w = &mut st.settings.extra.watch;
    w.session_from = Some(SystemTime::now());
    w.last_check = None;
    w.last_crash.clear();
}

fn steam_roots(st: &State) -> Vec<PathBuf> {
    let mut roots = deadlock_core::discovery::default_steam_roots();
    if let Some(custom) = options::get().steam_root {
        roots.insert(0, PathBuf::from(custom));
    }
    if let Some(root) = st.game_root.as_ref().and_then(|g| g.ancestors().nth(3)) {
        roots.push(root.to_path_buf());
    }
    roots
}

fn on_game_closed(st: &mut State) {
    let o = options::get();
    let root = st.game_root.clone();
    if o.remove_paths_on_close
        && let Some(root) = &root
    {
        match game_session::remove_addons_path(root, true) {
            Ok(true) => {
                super::log::info("Removed citadel/addons from gameinfo.gi after the game closed");
                notify::push(
                    st,
                    Notice::info(tr("Mods detached"))
                        .detail(tr("Sigil took citadel/addons out of gameinfo.gi. Launching modded from Sigil puts it back.")),
                );
                crate::data::rescan_game(st);
            }
            Ok(false) => {}
            Err(error) => super::log::error(format!("gameinfo.gi cleanup failed: {error}")),
        }
    }
    let since = st.settings.extra.watch.session_from.take();
    if o.crash_check
        && let Some(since) = since
    {
        let folders = game_session::dump_folders(root.as_deref(), &steam_roots(st));
        let new = game_session::dumps_since(&game_session::list_dumps(&folders), since);
        let exit = steam_roots(st)
            .iter()
            .find_map(|r| std::fs::read_to_string(r.join("logs").join("gameprocess_log.txt")).ok())
            .and_then(|log| game_session::steam_exit_code(&log));
        let clean = root
            .as_deref()
            .is_some_and(|r| game_session::console_ended_cleanly(&game_session::console_log_path(r)));
        if game_session::classify(new.len(), exit, clean) == game_session::Ending::Crash {
            st.settings.extra.watch.last_crash = new.iter().map(|d| d.path.clone()).collect();
            let newest = new.first().map(|d| d.path.clone());
            super::log::info(format!("Deadlock crashed (dumps: {}, exit: {exit:?})", new.len()));
            notify::push(
                st,
                Notice::warning(tr("Deadlock crashed"))
                    .detail(tr("A crash dump or crash exit code was found after the last launch."))
                    .action(tr("Details"), move |st: &mut State| {
                        super::extra::open_on(st, super::Tab::Logs);
                        super::support::read_crashes(st);
                        if let Some(path) = newest {
                            st.settings.extra.dialog = Some(super::extra::Dialog::Crash(path));
                        }
                    }),
            );
        }
    }
    st.settings.extra.watch.tail = None;
}

/// Runs every poll. Returns true when something on screen changed.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !st.settings.extra.watch.started {
        st.settings.extra.watch.started = true;
        on_startup(st);
    }
    if let Some(Ok(running)) = st.settings.extra.watch.job.try_take() {
        let was = st.settings.extra.watch.running;
        st.settings.extra.watch.running = running;
        st.settings.game_running = running;
        RUNNING.store(running, Ordering::Relaxed);
        if running && !was && st.settings.extra.watch.session_from.is_none() {
            st.settings.extra.watch.session_from = Some(SystemTime::now());
        }
        if was && !running {
            on_game_closed(st);
        }
        changed |= was != running;
    }
    if let Some(Ok((tail, moved))) = st.settings.extra.watch.tail_job.try_take() {
        st.settings.extra.watch.tail = Some(tail);
        changed |= moved;
    }
    if st.demo || rune::is_headless() || !wanted(st) {
        return changed;
    }
    let w = &st.settings.extra.watch;
    if w.last_check.is_none_or(|t| t.elapsed() >= CHECK_EVERY) && !w.job.is_running() {
        let wake = st.wake.clone();
        st.settings.extra.watch.last_check = Some(Instant::now());
        st.settings.extra.watch.job.start(wake, || Ok(super::tabs::game_running()));
    }
    let o = options::get();
    let w = &st.settings.extra.watch;
    if o.presence_game
        && st.prefs.discord
        && w.running
        && w.last_tail.is_none_or(|t| t.elapsed() >= TAIL_EVERY)
        && !w.tail_job.is_running()
        && let Some(root) = st.game_root.clone()
    {
        let mut tail = w.tail.clone().unwrap_or_default();
        let wake = st.wake.clone();
        st.settings.extra.watch.last_tail = Some(Instant::now());
        st.settings.extra.watch.tail_job.start(wake, move || {
            let moved = tail.poll(&game_session::console_log_path(&root));
            Ok((tail, moved))
        });
    }
    tx.wake_after(TAIL_EVERY);
    changed
}

// --- Settings > Game --------------------------------------------------------------------

pub fn game_rows(s: &State, u: &Ui) -> El {
    let o = options::get();
    let launch_has = s
        .settings
        .launch_read
        .as_ref()
        .and_then(|r| r.value.as_deref())
        .is_some_and(game_session::has_condebug);
    col()
        .w_full()
        .gap(LIST_GAP)
        .child(section(
            u,
            "While the game runs",
            vec![
                toggle_row(
                    u,
                    "settings-watch-block",
                    "Block changes while the game runs",
                    "Installs, updates and toggles wait until Deadlock closes.",
                    o.block_while_running,
                    |st| options::edit(st, |o| o.block_while_running = !o.block_while_running),
                ),
                toggle_row(
                    u,
                    "settings-watch-remove",
                    "Remove the mod path when the game closes",
                    "Takes citadel/addons out of gameinfo.gi. A modded launch from Sigil adds it back.",
                    o.remove_paths_on_close,
                    |st| options::edit(st, |o| o.remove_paths_on_close = !o.remove_paths_on_close),
                ),
                toggle_row(
                    u,
                    "settings-watch-crash",
                    "Check for crashes after a launch",
                    "Looks for a new crash dump when the game closes and tells you.",
                    o.crash_check,
                    |st| options::edit(st, |o| o.crash_check = !o.crash_check),
                ),
                setting_row(
                    u,
                    "settings-watch-status",
                    "Deadlock",
                    detail(
                        u,
                        if s.settings.extra.watch.running {
                            "Running now."
                        } else if wanted(s) {
                            "Not running. Sigil checks every few seconds."
                        } else {
                            "Not watched. Turn on one of the switches above."
                        },
                    ),
                    rune::el(),
                ),
            ],
        ))
        .child(section(
            u,
            "Keeping mods applied",
            vec![
                toggle_row(
                    u,
                    "settings-fix-before-launch",
                    "Fix mods before launch",
                    "Puts citadel/addons back in gameinfo.gi when an update took it out.",
                    o.fix_before_launch,
                    |st| options::edit(st, |o| o.fix_before_launch = !o.fix_before_launch),
                ),
                toggle_row(
                    u,
                    "settings-auto-reapply",
                    "Reapply mods when Sigil starts",
                    "Puts back mods a vanilla run set aside and repairs gameinfo.gi at startup.",
                    o.auto_reapply,
                    |st| options::edit(st, |o| o.auto_reapply = !o.auto_reapply),
                ),
                setting_row(
                    u,
                    "settings-reapply-now",
                    "Reapply now",
                    detail(u, "The same steps, once."),
                    disabled(
                        super::button(u, "settings-reapply-run", "Reapply", ActionKind::Secondary)
                            .on_press(|st: &mut State| {
                                if st.demo {
                                    return super::say(st, tr("Demo data: nothing was changed."), false);
                                }
                                reapply(st);
                                super::say(st, tr("Checked. Anything missing was put back."), false);
                            }),
                        s.game_root.is_none(),
                    ),
                ),
                toggle_row(
                    u,
                    "settings-condebug",
                    "Console log (-condebug)",
                    if launch_has {
                        "In your Steam launch options. Discord game presence reads console.log."
                    } else {
                        "Adds -condebug to Steam's launch options, for Discord game presence."
                    },
                    o.condebug,
                    toggle_condebug,
                ),
            ],
        ))
}

/// Flip `-condebug`: write the Steam launch options through the usual confirm. Turning it off
/// while game presence reads the log asks to turn presence off first.
fn toggle_condebug(st: &mut State) {
    let o = options::get();
    if o.condebug && o.presence_game {
        return super::say(
            st,
            tr("Turn off Discord game presence first. It reads the console log."),
            true,
        );
    }
    let on = !o.condebug;
    options::edit(st, |o| o.condebug = on);
    let current = st.settings.launch_options.text.clone();
    let value = game_session::with_condebug(&current, on);
    if value.trim() != current.trim() {
        st.settings.launch_options = rune::TextEdit::new(value.clone());
        st.settings.confirm = Some(super::Confirm::LaunchOptions(value));
    }
}

/// One line for Settings > Game when the console log is old or missing.
pub fn console_line(s: &State) -> String {
    match s.game_root.as_ref().map(|r| game_session::console_log_path(r)) {
        Some(p) if p.is_file() => fill(tr("console.log: {}."), &[&p.display().to_string()]),
        _ => tr("No console.log yet. Start the game once with -condebug.").to_string(),
    }
}
