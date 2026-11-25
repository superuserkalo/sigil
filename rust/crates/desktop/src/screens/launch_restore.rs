//! Mods a vanilla launch set aside come back by themselves; there is no Restore button.
//!
//! A vanilla launch moves the addon files into Sigil's stash (`deadlock_core::launch`) and a
//! modded launch puts them back as its first step (`launch_progress::plan`). This module adds the
//! other two ways back, both through the same `deadlock_core::launch::restore_vanilla` (a file
//! already in the addon folder is kept, never overwritten):
//!
//! - the launch monitor sees Deadlock close after it ran, and a stash exists;
//! - Sigil starts (or was closed during a vanilla session) and a stash exists while the game is
//!   not running.
//!
//! The restore runs on a worker ([`AutoRestore::job`]), never while the game runs, never while a
//! launch is under way or a launch step writes game files, and never while the monitor has not yet
//! read the process list. It waits until those clear. Success is one quiet toast and a fresh
//! Installed list; a failure is a warning toast with Retry and a banner on Installed that stays
//! until a restore succeeds.

use std::path::PathBuf;

use rune::{El, row, text};

use crate::app::State;
use crate::data::AddonRead;
use crate::icons;
use crate::jobs::Job;
use crate::notify::{self, Notice};
use crate::screens::launch_progress::tr;
use crate::ui::{ActionKind, BODY, PAGE_X, Ui};

/// What a restore's worker hands back: `None` when there was no stash, else how many files came
/// back (or why none did) and the addon folder read again.
pub type Outcome = Option<(Result<usize, String>, AddonRead)>;

pub struct AutoRestore {
    pub job: Job<Outcome>,
    /// The check at start is still owed: Sigil may have been closed during a vanilla session.
    startup: bool,
    /// The game was seen running since the last check: when it closes, put the mods back.
    saw_game: bool,
    /// Retry was pressed after a failure.
    retry: bool,
    /// Why the last restore failed. The Installed banner shows while this is set.
    pub failed: Option<String>,
}

impl Default for AutoRestore {
    fn default() -> Self {
        AutoRestore { job: Job::default(), startup: true, saw_game: false, retry: false, failed: None }
    }
}

/// Whether the game runs, as far as the process monitor knows. `None` until the monitor has read
/// the process list once (and always in demo data and headless renders, which never restore).
fn observed(st: &State) -> Option<bool> {
    if st.demo || rune::is_headless() {
        return None;
    }
    let monitor = &st.launch.monitor;
    (monitor.is_started() && monitor.has_checked()).then(|| monitor.running())
}

/// A launch is under way: its steps run or wait for the game, or a step still writes game files.
fn launch_in_flight(st: &State) -> bool {
    st.launch.progress.is_some() || st.launch.step.is_running() || st.launch.precheck.is_running()
}

/// Called every poll. True when something on screen changed.
pub fn poll(st: &mut State) -> bool {
    let seen = observed(st);
    tick(st, seen, crate::data::data_dir())
}

/// One step with what the monitor saw (`None`: not known yet) and the data folder that holds the
/// stash.
pub(crate) fn tick(st: &mut State, seen: Option<bool>, data: PathBuf) -> bool {
    let mut changed = false;
    if let Some(result) = st.launch.restore.job.try_take() {
        land(st, result);
        changed = true;
    }
    let Some(running) = seen else { return changed };
    if launch_in_flight(st) {
        // A launch decides the stash itself (a modded launch restores first, a vanilla one wants
        // it): the check at start is no longer owed.
        st.launch.restore.startup = false;
        if running {
            st.launch.restore.saw_game = true;
        }
        return changed;
    }
    let r = &mut st.launch.restore;
    if running {
        // Never while the game runs: it holds the files, and a vanilla session wants them out.
        r.saw_game = true;
        return changed;
    }
    if !(r.startup || r.saw_game || r.retry) || r.job.is_running() {
        return changed;
    }
    r.startup = false;
    r.saw_game = false;
    r.retry = false;
    start(st, data);
    changed
}

fn start(st: &mut State, data: PathBuf) {
    let game_root = st.game_root.clone();
    let wake = st.wake.clone();
    st.launch.restore.job.start(wake, move || {
        if deadlock_core::launch::stash_status(&data).is_none() {
            return Ok(None);
        }
        let result = deadlock_core::launch::restore_vanilla(&data, true)
            .map(|restore| restore.restored.len())
            .map_err(|error| error.to_string());
        Ok(Some((result, crate::data::read_addons(game_root.as_deref()))))
    });
}

fn land(st: &mut State, result: Result<Outcome, String>) {
    let (result, read) = match result {
        Ok(None) => {
            // No stash (or it was put back another way): nothing is set aside any more.
            st.launch.restore.failed = None;
            return;
        }
        Ok(Some((result, read))) => (result, Some(read)),
        Err(error) => (Err(error), None),
    };
    if let Some(read) = read {
        crate::data::apply_addon_read(st, read);
    }
    match result {
        Ok(count) => {
            crate::screens::settings::log::info(format!("Put back {count} file(s) a vanilla launch set aside"));
            st.launch.restore.failed = None;
            notify::push(st, Notice::info(tr("Your mods are back")));
        }
        Err(error) => {
            crate::screens::settings::log::error(format!("Putting the vanilla stash back failed: {error}"));
            st.launch.restore.failed = Some(error.clone());
            notify::push(
                st,
                Notice::warning(tr("Your mods are still set aside"))
                    .detail(error)
                    .action(tr("Try again"), retry),
            );
        }
    }
    crate::data::start_launch_read(st);
    crate::banner::recheck(st);
}

/// Retry, from the toast or the banner: the restore runs on the next poll, or once the game
/// closes and no launch is under way.
pub fn retry(st: &mut State) {
    st.launch.restore.retry = true;
    if crate::screens::installed::waiting::game_running(st) {
        notify::push(
            st,
            Notice::info(tr("Deadlock is running")).detail(tr("Your mods come back when the game closes.")),
        );
    }
    if let Some(wake) = &st.wake {
        wake();
    }
}

/// True while a restore moves files into the addon folder: other changes to it wait.
pub fn running(st: &State) -> bool {
    st.launch.restore.job.is_running()
}

/// The strip over Installed while the last restore failed. None otherwise.
pub fn banner(s: &State, u: &Ui) -> Option<El> {
    let why = s.launch.restore.failed.as_ref()?;
    let line = tr("Your mods are still set aside from a vanilla launch. The game starts without them.");
    Some(
        row()
            .key("restore-banner")
            .role("alert")
            .label(line)
            .w_full()
            .h(48.0)
            .px(PAGE_X)
            .gap(12.0)
            .center_y()
            .fill(u.amber.alpha(0.07))
            .border_sides([1.0, 0.0, 1.0, 0.0], u.amber.alpha(0.22))
            .child(rune::icon(&icons::TRIANGLE_ALERT, 16.0).color(u.amber).a11y_hidden())
            .child(
                text(line)
                    .font(BODY)
                    .color(u.ink)
                    .grow(1.0)
                    .shrink()
                    .min_w(0.0)
                    .truncate()
                    .help(why.clone())
                    .a11y_hidden(),
            )
            .child(
                u.action(tr("Try again"), ActionKind::Secondary)
                    .key("restore-retry")
                    .disabled(running(s))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(retry),
            ),
    )
}

/// `installed-restore-failed`: the last restore failed; Installed shows the banner.
pub fn scene_failed(st: &mut State) {
    crate::app::scene_demo_installed(st);
    st.launch.restore.failed = Some(
        "Restore refused: pak03_dir.vpk is both in the addon folder and in the stash. Remove or rename one first. Nothing was moved."
            .to_string(),
    );
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    /// A game install with an addon folder, and a data folder, under a fresh temp root.
    fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("sigil-restore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let game = root.join("Deadlock");
        let addons = deadlock_core::deploy::addons_dir(&game);
        fs::create_dir_all(&addons).unwrap();
        fs::write(game.join("game/citadel/gameinfo.gi"), "\"GameInfo\" {}\n").unwrap();
        let data = root.join("data");
        fs::create_dir_all(&data).unwrap();
        (game, addons, data)
    }

    /// Set two mods aside as a vanilla launch does.
    fn stash(game: &Path, addons: &Path, data: &Path) {
        fs::write(addons.join("pak01_dir.vpk"), b"one").unwrap();
        fs::write(addons.join("pak02_dir.vpk"), b"two").unwrap();
        deadlock_core::launch::stash_vanilla(game, data, true).unwrap();
        assert!(!addons.join("pak01_dir.vpk").exists(), "set aside");
    }

    fn state(game: &Path) -> State {
        let mut st = State::default();
        st.game_root = Some(game.to_path_buf());
        st
    }

    /// Poll with what the monitor saw until the worker lands.
    fn settle(st: &mut State, seen: Option<bool>, data: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        tick(st, seen, data.to_path_buf());
        while st.launch.restore.job.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            tick(st, seen, data.to_path_buf());
        }
    }

    fn toast_titles(st: &State) -> Vec<String> {
        st.notices.visible().map(|t| t.notice.title.clone()).collect()
    }

    #[test]
    fn at_start_a_stash_comes_back_when_the_game_is_not_running() {
        let (game, addons, data) = setup("start");
        stash(&game, &addons, &data);
        let mut st = state(&game);
        // The monitor has not read the process list yet: nothing happens.
        settle(&mut st, None, &data);
        assert!(!addons.join("pak01_dir.vpk").exists());
        settle(&mut st, Some(false), &data);
        assert_eq!(fs::read(addons.join("pak01_dir.vpk")).unwrap(), b"one");
        assert_eq!(fs::read(addons.join("pak02_dir.vpk")).unwrap(), b"two");
        assert!(deadlock_core::launch::stash_status(&data).is_none());
        assert_eq!(toast_titles(&st), vec!["Your mods are back".to_string()]);
        assert!(st.launch.restore.failed.is_none());
        // Once: a later poll with no new session does nothing more.
        assert!(!tick(&mut st, Some(false), data.clone()));
        assert!(!st.launch.restore.job.is_running());
    }

    #[test]
    fn at_start_while_the_game_runs_it_waits_for_the_close() {
        let (game, addons, data) = setup("running");
        stash(&game, &addons, &data);
        let mut st = state(&game);
        for _ in 0..3 {
            settle(&mut st, Some(true), &data);
            assert!(!addons.join("pak01_dir.vpk").exists(), "never while the game runs");
        }
        settle(&mut st, Some(false), &data);
        assert!(addons.join("pak01_dir.vpk").exists(), "back once it closed");
    }

    #[test]
    fn the_game_closing_after_a_vanilla_session_brings_them_back() {
        let (game, addons, data) = setup("close");
        let mut st = state(&game);
        // Start: nothing set aside yet, so the check finds nothing.
        settle(&mut st, Some(false), &data);
        assert!(toast_titles(&st).is_empty());
        // A vanilla launch: its steps run, then the game appears.
        stash(&game, &addons, &data);
        crate::screens::launch_progress::stage_waiting(&mut st, true);
        settle(&mut st, Some(false), &data);
        assert!(!addons.join("pak01_dir.vpk").exists(), "never while a launch is under way");
        st.launch.progress = None;
        settle(&mut st, Some(true), &data);
        assert!(!addons.join("pak01_dir.vpk").exists(), "never while the game runs");
        // The game closes.
        settle(&mut st, Some(false), &data);
        assert!(addons.join("pak01_dir.vpk").exists());
        assert_eq!(toast_titles(&st), vec!["Your mods are back".to_string()]);
    }

    #[test]
    fn a_file_already_in_the_addon_folder_is_kept() {
        let (game, addons, data) = setup("kept");
        stash(&game, &addons, &data);
        // The player put one back by hand, with other contents, and removed it from the stash.
        let status = deadlock_core::launch::stash_status(&data).unwrap();
        let entry = status.files.iter().find(|f| f.from.ends_with("pak01_dir.vpk")).unwrap();
        fs::remove_file(&entry.to).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), b"mine").unwrap();
        let mut st = state(&game);
        settle(&mut st, Some(false), &data);
        assert_eq!(fs::read(addons.join("pak01_dir.vpk")).unwrap(), b"mine", "kept, not overwritten");
        assert_eq!(fs::read(addons.join("pak02_dir.vpk")).unwrap(), b"two");
        assert!(st.launch.restore.failed.is_none());
    }

    #[test]
    fn a_failure_warns_with_retry_and_keeps_the_banner_until_it_works() {
        let (game, addons, data) = setup("fail");
        stash(&game, &addons, &data);
        // A file both in the addon folder and in the stash: the restore refuses and moves nothing.
        fs::write(addons.join("pak02_dir.vpk"), b"other").unwrap();
        let mut st = state(&game);
        settle(&mut st, Some(false), &data);
        assert!(st.launch.restore.failed.as_deref().is_some_and(|why| why.contains("pak02_dir.vpk")));
        assert_eq!(fs::read(addons.join("pak02_dir.vpk")).unwrap(), b"other", "nothing overwritten");
        let toast = st.notices.visible().last().expect("a toast");
        assert_eq!(toast.notice.title, "Your mods are still set aside");
        assert_eq!(toast.notice.action_label(), Some("Try again"), "Retry is offered");
        // Nothing runs again by itself.
        settle(&mut st, Some(false), &data);
        assert!(st.launch.restore.failed.is_some());

        // The player clears the clash and presses Retry: the mods come back, the banner goes.
        fs::remove_file(addons.join("pak02_dir.vpk")).unwrap();
        retry(&mut st);
        settle(&mut st, Some(false), &data);
        assert!(st.launch.restore.failed.is_none());
        assert_eq!(fs::read(addons.join("pak02_dir.vpk")).unwrap(), b"two");
        assert!(toast_titles(&st).contains(&"Your mods are back".to_string()));
    }

    #[test]
    fn the_banner_shows_only_while_a_restore_failed() {
        let _globals = crate::ui::test_globals();
        let render = |st: State| {
            let mut rt = rune::Runtime::new(st, crate::app::view, (1280.0, 800.0), rune::Env::new(true, 1.0));
            rt.check_live_only(false);
            for _ in 0..4 {
                rt.frame(1.0 / 60.0);
            }
            rt.elements().iter().any(|e| e.key.as_deref() == Some("restore-banner"))
        };
        let mut st = State::default();
        crate::app::scene_demo_installed(&mut st);
        assert!(!render(st));
        let mut st = State::default();
        scene_failed(&mut st);
        assert!(render(st));
    }
}
