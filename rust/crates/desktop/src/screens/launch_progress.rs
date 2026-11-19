//! The launch progress overlay (Deadlock Mod Manager's): what Sigil does between a launch press
//! and the game window, one step a frame, with Cancel and Hide.
//!
//! A launch is a short list of [`Step`]s. Each step is one call the launch buttons made before
//! (put back a vanilla stash, write the saved launch options into Steam with
//! `autoexec::sync_launch_options`, hand the game to Steam), then the wait for the Deadlock
//! process, which the launch monitor reports. Cancel before Steam has the game stops the launch
//! with nothing started; Cancel while waiting closes the game once it appears. Hide keeps the
//! launch going without the overlay. The wait gives up after three minutes with a note, since
//! Steam may be updating the game.

use std::time::{Duration, Instant};

use rune::{El, Key, KeyEvent, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::launch::Dialog;
use crate::screens::settings::i18n::tr;
use crate::ui::{ActionKind, BODY, H1, Ui};

/// How long the overlay waits for the game process.
pub const WAIT_FOR_GAME: Duration = Duration::from_secs(180);

/// One step of a launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Put back the mods a vanilla launch set aside.
    Restore,
    /// Roll the random mod lists and the Locker's skin shuffle (`launch_extras`).
    Roll,
    /// Repair enabled mods for the current game build, when Settings asks (`installed::repair`).
    Repair,
    /// Write the saved launch options into Steam.
    LaunchOptions,
    /// Hand the game to Steam with the mods as they are (the sidebar button).
    OpenSteam,
    /// The Launch page's modded launch (`deadlock_core::launch::launch_modded`).
    PageModded,
    /// Set the mods aside and hand the game to Steam (`launch_vanilla`).
    Vanilla,
    /// The game is already vanilla on disk: only start it.
    VanillaStashed,
    /// Wait for the Deadlock process.
    Waiting,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Restore => "Putting your mods back",
            Step::Roll => "Rolling random mods",
            Step::Repair => "Repairing mods for this game build",
            Step::LaunchOptions => "Writing launch options to Steam",
            Step::OpenSteam | Step::PageModded | Step::VanillaStashed => "Opening Steam",
            Step::Vanilla => "Setting mods aside and opening Steam",
            Step::Waiting => "Waiting for Deadlock to start",
        }
    }

    /// Once this step ran, Steam has the game: Cancel means closing it.
    fn hands_off(self) -> bool {
        matches!(self, Step::OpenSteam | Step::PageModded | Step::Vanilla | Step::VanillaStashed)
    }
}

/// A launch in progress.
#[derive(Clone, Debug)]
pub struct Progress {
    pub steps: Vec<Step>,
    /// The step running next; `steps.len()` when every step is done.
    pub at: usize,
    pub vanilla: bool,
    /// The overlay was hidden; the launch keeps going.
    pub hidden: bool,
    /// Cancel was pressed after Steam had the game: close it when it appears.
    pub closing: bool,
    /// When the wait for the game began.
    pub waiting_since: Option<Instant>,
}

/// The steps of a launch: from the sidebar (`page` false) or the Launch page.
pub fn plan(vanilla: bool, stashed: bool, page: bool, repair: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    if vanilla {
        steps.push(Step::LaunchOptions);
        steps.push(if stashed { Step::VanillaStashed } else { Step::Vanilla });
    } else {
        if stashed {
            steps.push(Step::Restore);
        }
        if repair {
            steps.push(Step::Repair);
        }
        steps.push(Step::Roll);
        steps.push(Step::LaunchOptions);
        steps.push(if page { Step::PageModded } else { Step::OpenSteam });
    }
    steps.push(Step::Waiting);
    steps
}

/// Start a launch. A modded one first asks about map mods when any are on
/// (`launch_extras::ask_first`); the answer starts it through [`begin_checked`].
pub fn begin(st: &mut State, vanilla: bool, page: bool) {
    // Every launch passes the gate first: a gameinfo.gi that would crash the game stops here.
    if crate::screens::launch_gate::blocked(st, vanilla, page) {
        return;
    }
    // A Launch map press leaves `map <name>` in autoexec.cfg; a normal launch removes it.
    crate::screens::maps::clear_stale_map_launch(st);
    if !vanilla && crate::screens::launch_extras::ask_first(st, page) {
        return;
    }

    begin_checked(st, vanilla, page);
}

/// Start a launch with every question answered. The press is the approval; the steps run from
/// the next poll on.
pub fn begin_checked(st: &mut State, vanilla: bool, page: bool) {
    if !vanilla {
        // C11: a mod missing files loads broken or not at all; say so, then launch.
        crate::screens::installed::checks::missing_before_launch(st);
    }
    crate::screens::settings::telemetry::event(st, if vanilla { "launch:vanilla" } else { "launch:modded" }, None);
    let stashed = deadlock_core::launch::stash_status(&crate::data::data_dir()).is_some();
    st.launch_dialog = false;
    st.launch.dialog = Dialog::None;
    st.launch.progress = Some(Progress {
        steps: plan(vanilla, stashed, page, crate::screens::settings::options::get().repair_before_launch),
        at: 0,
        vanilla,
        hidden: false,
        closing: false,
        waiting_since: None,
    });
}

/// Run the next step, or follow the wait. One step a frame, so the overlay shows each one.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let Some(progress) = st.launch.progress.clone() else {
        return false;
    };
    let Some(step) = progress.steps.get(progress.at).copied() else {
        st.launch.progress = None;
        return true;
    };
    if step == Step::Waiting {
        return wait(st, tx, &progress);
    }
    // The repair runs on a worker: start it, then hold this step until it is done.
    if step == Step::Repair {
        if !st.launch_extras.repairing {
            st.launch_extras.repairing = true;
            if crate::screens::installed::repair::start_before_launch(st) {
                tx.wake_after(Duration::from_millis(250));
                return true;
            }
        } else if st.installed.repairs.work.is_running() {
            tx.wake_after(Duration::from_millis(250));
            return false;
        }
        st.launch_extras.repairing = false;
        if let Some(p) = st.launch.progress.as_mut() {
            p.at += 1;
        }
        return true;
    }
    run(st, step);
    if let Some(p) = st.launch.progress.as_mut() {
        p.at += 1;
        if p.steps.get(p.at) == Some(&Step::Waiting) {
            p.waiting_since = Some(Instant::now());
        }
    }
    if step.hands_off() {
        // The step's own line wins; `launch.last` may still hold an earlier step's result.
        if st.status.trim().is_empty()
            && let Some(line) = st.launch.last.clone()
        {
            st.status = line;
        }
        // `settings::poll` takes `settings.launched` and minimizes or closes as Settings asks.
        // A headless or demo run never starts a process to wait for.
        if st.demo || rune::is_headless() {
            st.launch.progress = None;
        }
    }
    tx.wake_after(Duration::from_millis(16));
    true
}

fn run(st: &mut State, step: Step) {
    match step {
        Step::Restore => crate::data::run_launch(st, Dialog::Restore),
        Step::Roll => crate::screens::launch_extras::roll_before_launch(st),
        Step::Repair => {}
        Step::LaunchOptions => crate::screens::autoexec::sync_launch_options(st),
        Step::OpenSteam | Step::VanillaStashed => st.launch_modded(),
        Step::PageModded => crate::data::run_launch(st, Dialog::Modded),
        Step::Vanilla => crate::data::run_launch(st, Dialog::Vanilla),
        Step::Waiting => {}
    }
}

fn wait(st: &mut State, tx: &mut Tx, progress: &Progress) -> bool {
    let running = st.launch.monitor.running();
    if running {
        st.launch.progress = None;
        if progress.closing {
            crate::screens::launch::stop_game_quietly(st);
        }
        return true;
    }
    let since = progress.waiting_since.unwrap_or_else(Instant::now);
    if since.elapsed() >= WAIT_FOR_GAME {
        st.launch.progress = None;
        if !progress.closing {
            crate::notify::push(
                st,
                crate::notify::Notice::warning(tr("Deadlock has not started yet"))
                    .detail(tr("Steam has the launch. It may be updating the game or waiting for you in its window.")),
            );
        }
        return true;
    }
    tx.wake_after(Duration::from_millis(500));
    false
}

/// Cancel: before Steam has the game nothing starts; after, the game closes once it is up.
pub fn cancel(st: &mut State) {
    let Some(progress) = st.launch.progress.as_mut() else { return };
    let handed = progress.steps[..progress.at.min(progress.steps.len())]
        .iter()
        .any(|step| step.hands_off());
    if !handed {
        st.launch.progress = None;
        crate::notify::push(st, crate::notify::Notice::info(tr("Launch cancelled")));
        return;
    }
    progress.closing = true;
    progress.hidden = true;
    if st.launch.monitor.running() {
        st.launch.progress = None;
        crate::screens::launch::stop_game(st);
    } else {
        crate::notify::push(
            st,
            crate::notify::Notice::info(tr("Launch cancelled"))
                .detail(tr("Steam already has it. Sigil closes the game as soon as it starts.")),
        );
    }
}

fn hide(st: &mut State) {
    if let Some(progress) = st.launch.progress.as_mut() {
        progress.hidden = true;
    }
}

/// The overlay card, while a launch runs and is not hidden.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    let progress = s.launch.progress.as_ref().filter(|p| !p.hidden)?;
    let mut list = col().w_full().gap(10.0).pt(4.0);
    for (index, step) in progress.steps.iter().enumerate() {
        let done = index < progress.at;
        let active = index == progress.at;
        let mark = if done {
            icon(&icons::CIRCLE_CHECK, 16.0).color(u.accent).a11y_hidden()
        } else if active {
            icon(&icons::LOADER, 14.0).color(u.muted).a11y_hidden()
        } else {
            el().size(6.0, 6.0).radius(3.0).fill(u.faint.alpha(0.6)).a11y_hidden()
        };
        list = list.child(
            row()
                .key(format!("launch-progress-step-{index}"))
                .w_full()
                .gap(10.0)
                .center_y()
                .child(el().size(16.0, 16.0).center().child(mark))
                .child(
                    text(tr(step.label()))
                        .font(BODY)
                        .color(if done || active { u.ink } else { u.muted })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
        );
    }
    let share = progress.at as f32 / progress.steps.len().max(1) as f32;
    let bar = row()
        .w_full()
        .h(3.0)
        .radius(1.5)
        .clip_rounded(1.5)
        .fill(u.hair_strong)
        .child(el().h_full().grow(share.max(0.02)).fill(u.accent))
        .child(el().h_full().grow((1.0 - share).max(0.0)));
    Some(
        u.dialog_shell()
            .key("launch-progress")
            .role("status")
            .label(tr("Launching Deadlock"))
            .w(340.0)
            .padding(24.0)
            .gap(16.0)
            .on_key(|st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
                if ev.key == Key::Escape {
                    hide(st);
                    true
                } else {
                    false
                }
            })
            .child(
                row()
                    .w_full()
                    .center_y()
                    .child(
                        text(tr(if progress.vanilla { "Launching vanilla" } else { "Launching Deadlock" }))
                            .font(H1)
                            .color(u.ink)
                            .grow(1.0),
                    )
                    .child(
                        u.icon_button(&icons::X, tr("Hide. The launch keeps going."))
                            .key("launch-progress-hide")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(hide),
                    ),
            )
            .child(list)
            .child(bar)
            .child(
                row().w_full().child(spacer()).child(
                    u.action(tr("Cancel launch"), ActionKind::Secondary)
                        .key("launch-progress-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(cancel),
                ),
            ),
    )
}

/// A launch stopped at `step`, for the scenes.
pub fn stage(st: &mut State, vanilla: bool, at: usize) {
    let steps = plan(vanilla, false, false, false);
    st.launch.progress = Some(Progress {
        at: at.min(steps.len() - 1),
        steps,
        vanilla,
        hidden: false,
        closing: false,
        waiting_since: Some(Instant::now()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_options_are_written_before_steam_gets_the_game() {
        for (vanilla, stashed, page) in [(false, false, false), (false, true, true), (true, false, false), (true, true, false)] {
            let steps = plan(vanilla, stashed, page, true);
            let options = steps.iter().position(|s| *s == Step::LaunchOptions).unwrap();
            let steam = steps.iter().position(|s| s.hands_off()).unwrap();
            assert!(options < steam, "{steps:?}");
            assert_eq!(steps.last(), Some(&Step::Waiting));
        }
    }

    #[test]
    fn a_stash_is_put_back_before_a_modded_launch() {
        assert_eq!(plan(false, true, false, false), vec![Step::Restore, Step::Roll, Step::LaunchOptions, Step::OpenSteam, Step::Waiting]);
        assert_eq!(plan(true, true, false, false), vec![Step::LaunchOptions, Step::VanillaStashed, Step::Waiting]);
        assert_eq!(plan(false, false, true, false), vec![Step::Roll, Step::LaunchOptions, Step::PageModded, Step::Waiting]);
    }

    #[test]
    fn cancel_before_steam_starts_nothing() {
        let mut st = State::default();
        st.demo = true;
        stage(&mut st, false, 1);
        cancel(&mut st);
        assert!(st.launch.progress.is_none());
    }

    #[test]
    fn cancel_after_steam_closes_the_game_later() {
        let mut st = State::default();
        st.demo = true;
        stage(&mut st, false, 3);
        cancel(&mut st);
        let progress = st.launch.progress.as_ref().expect("still waiting for the game");
        assert!(progress.closing && progress.hidden);
    }
}
