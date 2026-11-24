//! The launch progress overlay (Deadlock Mod Manager's): what Sigil does between a launch press
//! and the game window, one step a frame, with Cancel and Hide.
//!
//! A launch is a short list of [`Step`]s. Each step is one call the launch buttons made before
//! (put back a vanilla stash, write the saved launch options into Steam with
//! `autoexec::sync_launch_options`, hand the game to Steam), then the wait for the Deadlock
//! process, which the launch monitor reports. Cancel before Steam has the game stops the launch
//! with nothing started; Cancel while waiting closes the game once it appears. Hide keeps the
//! launch going without the overlay. The wait gives up after three minutes with a note, since
//! Steam may be updating the game. When closing the game for a Cancel fails, the overlay comes
//! back with the error and Try again, instead of a launch that silently kept going (DMM 2.4.0).

use std::time::{Duration, Instant};

use rune::{El, Key, KeyEvent, Tx, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::screens::launch::Dialog;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

#[path = "launch_de.rs"]
mod de;

/// The overlay's own German rows, also read by the translation test so they cannot drift.
pub(crate) use de::DE as LAUNCH_DE;

/// The UI text in the player's language: this overlay's German table first, then Sigil's.
pub(crate) fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    i18n::tr(text)
}

/// What a finished Stop game says when the game is still there: `None` when it closed.
pub fn stop_failure(result: &Result<deadlock_core::launch::StopReport, String>) -> Option<String> {
    match result {
        Ok(report) if report.stopped() => None,
        Ok(report) => {
            let ids: Vec<String> = report.remaining.iter().map(u32::to_string).collect();
            Some(fill(
                tr("Deadlock is still running (process {}). Close it from Steam or the Task Manager."),
                &[&ids.join(", ")],
            ))
        }
        Err(error) => Some(fill(tr("Stopping Deadlock failed: {}"), &[error])),
    }
}

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
    /// Cancel is closing the game now; the stop worker has not reported yet.
    pub stopping: bool,
    /// Closing the game for a Cancel failed: why, shown on the overlay with Try again.
    pub cancel_error: Option<String>,
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
    // A second launch while one runs would set mods aside under a starting game: the buttons
    // are off then, and a press that still gets here (a shortcut, a dialog) says why.
    if let Some((title, detail)) = busy(st) {
        crate::notify::push(st, crate::notify::Notice::warning(tr(title)).detail(tr(detail)));
        return;
    }
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

/// Why a launch cannot start now, as a title and what to do: a launch is under way, or the
/// game is already up. `None` when a launch may start.
pub fn busy(st: &State) -> Option<(&'static str, &'static str)> {
    if st.launch.progress.is_some() {
        Some(("Deadlock is already starting", "Wait for it, or stop the launch first."))
    } else if st.launch.monitor.running() {
        Some(("Deadlock is already running", "Close the game first, then launch again."))
    } else {
        None
    }
}

/// The reason a launch button shows in its tooltip while it is off.
pub fn busy_line(st: &State) -> Option<String> {
    busy(st).map(|(title, detail)| format!("{}. {}", tr(title), tr(detail)))
}

/// A launch already handed to Steam and waiting for the game, its overlay hidden: what the
/// sidebar looks like while Deadlock starts. For scenes and tests; nothing runs.
pub fn stage_waiting(st: &mut State, vanilla: bool) {
    let steps = plan(vanilla, false, false, false);
    st.launch.progress = Some(Progress {
        at: steps.len() - 1,
        steps,
        vanilla,
        hidden: true,
        closing: false,
        waiting_since: Some(Instant::now()),
        stopping: false,
        cancel_error: None,
    });
}

/// The toast a failed vanilla launch raises when the starting game holds the mod files: what
/// happened and what to do, as a warning. `None` for any other failure, which keeps its own
/// words through the status line.
pub fn vanilla_held_notice(raw: &str) -> Option<crate::notify::Notice<State>> {
    held(raw).then(|| {
        crate::notify::Notice::warning(tr("Deadlock is using the mod files"))
            .detail(tr("Close the game, then launch vanilla again."))
    })
}

fn held(raw: &str) -> bool {
    deadlock_core::error::mentions_in_use(raw)
        || ["os error 32", "os error 33", "being used by another process", "Resource busy"]
            .iter()
            .any(|sign| raw.contains(sign))
}

/// What a failed vanilla launch says. A file the starting game holds open is the usual cause:
/// say that in plain words; anything else keeps its own message.
pub fn vanilla_failure(raw: &str) -> String {
    if held(raw) {
        format!(
            "{}. {}",
            tr("Deadlock is using the mod files"),
            tr("Close the game, then launch vanilla again.")
        )
    } else {
        raw.to_string()
    }
}

/// Start a launch with every question answered. The press is the approval; the steps run from
/// the next poll on.
pub fn begin_checked(st: &mut State, vanilla: bool, page: bool) {
    if !vanilla {
        // C11: a mod missing files loads broken or not at all; say so, then launch.
        crate::screens::installed::checks::missing_before_launch(st);
    }
    crate::screens::settings::telemetry::event(st, if vanilla { "launch:vanilla" } else { "launch:modded" }, None);
    // Blocked relay regions make queues longer; say so at every launch while any is on.
    crate::screens::servers::regions::remind_at_launch(st);
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
        stopping: false,
        cancel_error: None,
    });
}

/// Run the next step, or follow the wait. One step a frame, so the overlay shows each one.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let stopped = take_stop(st);
    let Some(progress) = st.launch.progress.clone() else {
        return stopped;
    };
    if progress.stopping || progress.cancel_error.is_some() {
        return stopped;
    }
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
        Step::OpenSteam => st.launch_modded(false),
        Step::VanillaStashed => st.launch_modded(true),
        Step::PageModded => crate::data::run_launch(st, Dialog::Modded),
        Step::Vanilla => crate::data::run_launch(st, Dialog::Vanilla),
        Step::Waiting => {}
    }
}

/// Take a finished Stop game. For a Cancel, success ends the launch and a failure brings the
/// overlay back with the error; any other Stop game failure is a toast. True when the view changed.
fn take_stop(st: &mut State) -> bool {
    let Some(result) = st.launch.stop.try_take() else {
        return false;
    };
    match stop_failure(&result) {
        None => crate::screens::settings::log::info("Stopped Deadlock"),
        Some(why) => crate::screens::settings::log::error(format!("Deadlock did not close: {why}")),
    }
    apply_stop(st, &result);
    // The running row follows the process again.
    crate::data::start_launch_read(st);
    true
}

fn apply_stop(st: &mut State, result: &Result<deadlock_core::launch::StopReport, String>) {
    let failure = stop_failure(result);
    let cancelling = st.launch.progress.as_ref().is_some_and(|p| p.stopping);
    match (cancelling, failure) {
        (true, None) => {
            st.launch.progress = None;
            crate::notify::push(
                st,
                crate::notify::Notice::info(tr("Launch cancelled")).detail(tr("Deadlock was closed.")),
            );
        }
        (true, Some(why)) => {
            if let Some(p) = st.launch.progress.as_mut() {
                p.stopping = false;
                p.hidden = false;
                p.cancel_error = Some(why);
            }
        }
        (false, Some(why)) => crate::notify::push(
            st,
            crate::notify::Notice::error(tr("Deadlock did not close"))
                .detail(why)
                // One more press asks Windows to end it outright; stop_game already forced
                // after its grace period, so a retry is the offer to end a stuck game.
                .action(tr("Try again"), |st: &mut State| crate::screens::launch::stop_game(st)),
        ),
        (false, None) => {}
    }
}

/// Close the game for a Cancel, keeping the launch on screen until the stop worker reports.
fn close_for_cancel(st: &mut State) {
    if let Some(p) = st.launch.progress.as_mut() {
        p.stopping = true;
        p.cancel_error = None;
    }
    if !crate::screens::launch::stop_game_for_cancel(st) {
        st.launch.progress = None;
    }
}

fn wait(st: &mut State, tx: &mut Tx, progress: &Progress) -> bool {
    let running = st.launch.monitor.running();
    if running {
        if progress.closing {
            close_for_cancel(st);
        } else {
            st.launch.progress = None;
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
    if st.launch.monitor.running() {
        // The overlay stays up, reading "Closing Deadlock", until the game is gone or the stop
        // failed.
        close_for_cancel(st);
    } else {
        progress.hidden = true;
        crate::notify::push(
            st,
            crate::notify::Notice::info(tr("Launch cancelled"))
                .detail(tr("Steam already has it. Sigil closes the game as soon as it starts.")),
        );
    }
}

/// Close the error and leave the game as it is.
fn dismiss(st: &mut State) {
    st.launch.progress = None;
}

fn hide(st: &mut State) {
    if let Some(progress) = st.launch.progress.as_mut() {
        progress.hidden = true;
    }
}

/// The overlay card, while a launch runs and is not hidden. The step list is fixed when the
/// launch starts and the footer is one fixed line, so the card keeps one height throughout.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    let progress = s.launch.progress.as_ref().filter(|p| !p.hidden)?;
    let mut list = col().w_full();
    for (index, step) in progress.steps.iter().enumerate() {
        let done = index < progress.at;
        let active = index == progress.at;
        let mark = if done {
            icon(&icons::CHECK, 16.0).color(u.teal).a11y_hidden()
        } else if active {
            icon(&icons::LOADER, 16.0).color(u.muted).a11y_hidden()
        } else {
            el().size(6.0, 6.0).radius(3.0).fill(u.faint).a11y_hidden()
        };
        list = list.child(
            row()
                .key(format!("launch-progress-step-{index}"))
                .w_full()
                .h(24.0)
                .gap(8.0)
                .center_y()
                .child(el().size(16.0, 16.0).center().child(mark))
                .child(
                    text(tr(step.label()))
                        .font(BODY)
                        .color(if active { u.ink } else if done { u.prose } else { u.muted })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
        );
    }
    let mut body = col().w_full().padding(16.0).gap(12.0).child(
        row()
            .w_full()
            .h(24.0)
            .center_y()
            .child(
                text(tr(if progress.vanilla { "Launching vanilla" } else { "Launching Deadlock" }))
                    .font(BODY_MED)
                    .color(u.ink)
                    .role("heading")
                    .grow(1.0),
            )
            .child(
                u.icon_button(&icons::X, tr("Hide. The launch keeps going."))
                    .size(24.0, 24.0)
                    .key("launch-progress-hide")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(hide),
            ),
    );
    body = body.child(list);
    if let Some(why) = &progress.cancel_error {
        body = body.child(
            col()
                .key("launch-progress-error")
                .role("alert")
                .w_full()
                .gap(4.0)
                .child(text(tr("The launch could not be cancelled")).font(BODY_MED).color(u.rose).wrap())
                .child(text(why.clone()).font(SMALL).color(u.prose).wrap()),
        );
    }
    let footer = row().key("launch-progress-footer").w_full().h(28.0).gap(8.0).center_y();
    let footer = if progress.cancel_error.is_some() {
        footer
            .child(spacer())
            .child(
                u.action_m(tr("Close"), ActionKind::Ghost)
                    .key("launch-progress-dismiss")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(dismiss),
            )
            .child(
                u.action_m(tr("Try again"), ActionKind::Secondary)
                    .key("launch-progress-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close_for_cancel),
            )
    } else if progress.stopping {
        // Closing the game takes the button's place: same line, same height.
        footer
            .key("launch-progress-closing")
            .child(el().size(16.0, 16.0).center().child(icon(&icons::LOADER, 16.0).color(u.muted).a11y_hidden()))
            .child(text(tr("Closing Deadlock")).font(BODY).color(u.ink).grow(1.0).min_w(0.0).truncate())
    } else {
        footer.child(spacer()).child(
            u.action_m(tr("Cancel launch"), ActionKind::Secondary)
                .key("launch-progress-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(cancel),
        )
    };
    body = body.child(footer);
    let share = progress.at as f32 / progress.steps.len().max(1) as f32;
    // A 2 point line on the card's bottom edge.
    let bar = row()
        .key("launch-progress-bar")
        .w_full()
        .h(2.0)
        .a11y_hidden()
        .child(el().h_full().grow(share.max(0.02)).fill(u.accent))
        .child(el().h_full().grow((1.0 - share).max(0.0)).fill(u.hair));
    Some(
        u.dialog_shell()
            .key("launch-progress")
            .role("status")
            .label(tr("Launching Deadlock"))
            .w(340.0)
            .clip_rounded(crate::ui::r(14.0))
            .on_key(|st: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
                if ev.key == Key::Escape {
                    hide(st);
                    true
                } else {
                    false
                }
            })
            .child(body)
            .child(bar),
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
        stopping: false,
        cancel_error: None,
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
    fn a_failed_cancel_keeps_the_launch_on_screen_with_the_error() {
        use deadlock_core::launch::StopReport;
        let mut st = State::default();
        st.demo = true;
        stage(&mut st, false, 3);
        if let Some(p) = st.launch.progress.as_mut() {
            p.closing = true;
            p.stopping = true;
            p.hidden = true;
        }
        apply_stop(&mut st, &Ok(StopReport { pids: vec![7], forced: true, remaining: vec![7] }));
        let progress = st.launch.progress.as_ref().expect("the launch stays on screen");
        assert!(!progress.hidden && !progress.stopping);
        assert!(progress.cancel_error.as_deref().is_some_and(|why| why.contains('7')));

        close_for_cancel(&mut st);
        // Headless: no worker starts, so the launch is let go instead of spinning forever.
        assert!(st.launch.progress.is_none() || st.launch.progress.as_ref().is_some_and(|p| p.stopping));

        stage(&mut st, false, 3);
        st.launch.progress.as_mut().unwrap().stopping = true;
        apply_stop(&mut st, &Ok(StopReport { pids: vec![7], forced: false, remaining: Vec::new() }));
        assert!(st.launch.progress.is_none(), "a closed game ends the launch");
        assert!(stop_failure(&Err("denied".into())).unwrap().contains("denied"));
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
