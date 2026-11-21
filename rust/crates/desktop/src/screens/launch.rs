//! Launch: modded, vanilla, the running game and the way back.
//!
//! `deadlock_core::launch` owns the stash and restore. This screen is the app action surface:
//! every launch and every stash sits behind a confirm dialog, a vanilla launch says what it
//! moves aside, and the running state comes from the monitor thread that watches the process
//! table (see [`Monitor`]).
//!
//! Two things sit next to the launch buttons: a crash advisory for the last session (the newest
//! dump that the launch marker has not seen, with the mod that most likely caused it, from
//! `deadlock_core::impact`) and the download queue (`screens::downloads`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deadlock_core::impact::{Change, Suspect};
use deadlock_core::launch::{GameProcess, LaunchMarker};
use rune::{Color, El, Key, KeyEvent, col, el, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, PAGE_TOP, PAGE_X, SMALL, Ui};
use crate::screens::settings::i18n::{fill, tr};

use crate::screens::conflicts;
use crate::screens::downloads;

/// What the background read found. `deadlock_core::launch` owns the stash and the marker;
/// `deadlock_core::impact` turns the journal and the newest dump into a culprit.
pub struct Read {
    pub stash: Option<deadlock_core::launch::StashStatus>,
    pub gameinfo_present: bool,
    pub gameinfo_addons: bool,
    pub addon_files: usize,
    pub enabled: usize,
    /// The last launch Sigil asked Steam for.
    pub marker: Option<LaunchMarker>,
    /// A crash after that launch that the player has not read yet.
    pub crash: Option<Crash>,
}

/// One crash advisory: the dump, its own error text, and the mod that most likely caused it.
pub struct Crash {
    pub path: String,
    pub file: String,
    /// `15 Nov, 20:45`.
    pub when: String,
    /// Unix seconds, for the acknowledgement.
    pub at_secs: i64,
    pub exception: Option<String>,
    pub comment: Option<String>,
    pub suspect: Option<Suspect>,
    /// The suspect is switched off now (the card says so instead of offering Turn off).
    pub suspect_off: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    Modded,
    Vanilla,
    Restore,
    /// Before a launch, when something the game needs is off.
    Checklist,
    /// The game is running and the player asked to close it.
    StopGame,
}

pub struct View {
    pub loaded: bool,
    pub error: Option<String>,
    pub read: Option<Read>,
    pub dialog: Dialog,
    pub busy: bool,
    pub last: Option<String>,
    /// Live process state, independent of the page's read.
    pub monitor: Monitor,
    /// The download queue, shared with its worker.
    pub downloads: downloads::Downloads,
    /// The overlap page's dialog and ignore list, kept with this area's state.
    pub overlap: conflicts::View,
    /// A launch the progress overlay is stepping through.
    pub progress: Option<crate::screens::launch_progress::Progress>,
    /// A launch stopped by the gate until the player answers (`screens::launch_gate`).
    pub gate: Option<crate::screens::launch_gate::Pending>,
    /// Stop game on its worker; `launch_progress::poll` takes the report and says when the game
    /// did not close.
    pub stop: crate::jobs::Job<deadlock_core::launch::StopReport>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            error: None,
            read: None,
            dialog: Dialog::None,
            busy: false,
            last: None,
            monitor: Monitor::default(),
            downloads: downloads::Downloads::default(),
            overlap: conflicts::View::default(),
            progress: None,
            gate: None,
            stop: crate::jobs::Job::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// The process monitor
// ---------------------------------------------------------------------------

/// Watches the process table on a thread of its own, so the running row and Stop game do not
/// depend on the Launch page being opened and re-read. One check every two seconds; the UI is
/// woken only when the answer changes.
pub struct Monitor {
    shared: Arc<Mutex<GameProcess>>,
    stop: Option<Arc<AtomicBool>>,
    started: bool,
}

impl Default for Monitor {
    fn default() -> Self {
        Monitor {
            shared: Arc::new(Mutex::new(GameProcess::default())),
            stop: None,
            started: false,
        }
    }
}

impl Monitor {
    /// Start the thread once. Headless renders never start it: a picture must not depend on
    /// this machine's processes.
    pub fn start(&mut self, wake: Option<rune::Waker>) {
        if self.started || rune::is_headless() {
            return;
        }
        self.started = true;
        let shared = Arc::clone(&self.shared);
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Some(Arc::clone(&stop));
        let wake: Option<Arc<dyn Fn() + Send + Sync>> = wake.map(|wake| {
            let wake = wake.clone();
            Arc::new(move || wake()) as Arc<dyn Fn() + Send + Sync>
        });
        let _ = std::thread::Builder::new()
            .name("sigil-game-watch".to_string())
            .spawn(move || {
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let now = deadlock_core::launch::read_game_process();
                    let changed = shared
                        .lock()
                        .map(|mut slot| {
                            let changed = *slot != now;
                            *slot = now;
                            changed
                        })
                        .unwrap_or(false);
                    if changed && let Some(wake) = &wake {
                        wake();
                    }
                    for _ in 0..8 {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            });
    }

    pub fn snapshot(&self) -> GameProcess {
        self.shared
            .lock()
            .map(|process| process.clone())
            .unwrap_or_default()
    }

    pub fn running(&self) -> bool {
        self.snapshot().running
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
    }
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.launch;
    let read = v.read.as_ref();
    let pending = read.and_then(|read| read.stash.as_ref());
    let process = v.monitor.snapshot();
    let running = process.running || s.settings.game_running;

    let mut actions = row().gap(8.0).center_y();
    actions = actions.child(
        u.icon_button(&icons::REFRESH_CW, tr("Recheck"))
            .key("launch-recheck")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| crate::data::start_launch_read(st)),
    );

    let detail = match read {
        Some(_) if running => read
            .and_then(|read| read.marker.as_ref())
            .filter(|marker| marker.open())
            .map(started_line)
            .unwrap_or_else(|| tr("Running now").to_string()),
        Some(read) => {
            if read.enabled == 1 {
                fill(tr("{} of {} mod on."), &[&read.enabled.to_string(), &read.addon_files.to_string()])
            } else {
                fill(
                    tr("{} of {} mods on."),
                    &[&read.enabled.to_string(), &read.addon_files.to_string()],
                )
            }
        }
        None => String::new(),
    };

    let mut page = col()
        .key("launch-page")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(32.0)
        .gap(16.0)
        .child(u.page_head("", tr("Launch"), &detail, Some(actions)));

    if let Some(last) = &v.last {
        page = page.child(if is_bad(last) {
            crate::notify::feedback::error_box(u, "launch-last-error", last)
        } else {
            note_line(u, last, false)
        });
    }

    let body = if !s.game_detected {
        u.empty_state(
            "",
            tr("Deadlock was not found"),
            tr("Set the game folder in Settings to launch from here."),
            Some(
                u.action(tr("Open Settings"), ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        )
    } else if let Some(error) = &v.error {
        u.empty_state(
            "",
            tr("Launch status could not be read"),
            error,
            Some(crate::notify::feedback::button(u, "launch-error-feedback", tr("Launch status could not be read"), error)),
        )
    } else {
        let stashed = pending.is_some();
        let ready = !stashed && !v.busy && !running;
        let mods_on = match read {
            Some(read) => mods_label(read.enabled),
            None => tr("Your enabled mods").to_string(),
        };
        let mut body = col().w_full().gap(24.0);
        if running {
            body = body.child(running_row(u, &process, read));
        } else {
            body = body.child(
                row()
                    .w_full()
                    .gap(12.0)
                    .child(
                        big_action(u, tr("Launch modded"), &mods_on, !stashed, ready)
                            .key("launch-go-modded")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                if ready {
                                    press_launch(st, false);
                                }
                            }),
                    )
                    .child(
                        big_action(
                            u,
                            tr("Launch vanilla"),
                            tr("Mods set aside until you restore"),
                            false,
                            ready,
                        )
                        .key("launch-go-vanilla")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            if ready {
                                st.launch.dialog = Dialog::Vanilla;
                            }
                        }),
                    ),
            );
        }
        body = body.child(stash_line(s, u, pending));
        if let Some(read) = read
            && (!read.gameinfo_present || !read.gameinfo_addons)
            && !running
        {
            body = body.child(
                row()
                    .w_full()
                    .gap(10.0)
                    .center_y()
                    .child(note_line(u, tr("gameinfo.gi does not load mods right now."), true).grow(1.0).shrink().min_w(0.0))
                    .child(
                        u.action(tr("Fix mods"), ActionKind::Secondary)
                            .key("launch-fix-gameinfo")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(fix_gameinfo),
                    ),
            );
        }
        if let Some(crash) = read.and_then(|read| read.crash.as_ref()) {
            body = body.child(crash_block(s, u, crash));
        }
        body.child(downloads::section(s, u))
    };

    col()
        .key("launch-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(page.child(body))
}

/// The running row replaces the launch buttons while Deadlock is up.
fn running_row(u: &Ui, process: &GameProcess, read: Option<&Read>) -> El {
    let since = read
        .and_then(|read| read.marker.as_ref())
        .filter(|marker| marker.open())
        .map(started_line)
        .unwrap_or_else(|| tr("Running now.").to_string());
    let detail = if process.count() > 1 {
        fill(tr("{} {}"), &[&since, &process.count_label()])
    } else {
        since
    };
    row()
        .key("launch-running")
        .w_full()
        .min_h(64.0)
        .py(12.0)
        .gap(14.0)
        .center_y()
        .child(
            icon(&icons::PLAY, 18.0)
                .color(u.teal)
                .a11y_hidden(),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(tr("Deadlock is running")).font(BODY_MED).color(u.ink))
                .child(text(detail).font(SMALL).color(u.muted).truncate()),
        )
        .child(
            u.action(tr("Stop game"), ActionKind::Secondary)
                .key("launch-stop")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.launch.dialog = Dialog::StopGame),
        )
}

/// The crash advisory for the last session: what happened, the dump's own words, and the mod
/// that most likely caused it.
fn crash_block(s: &State, u: &Ui, crash: &Crash) -> El {
    let mut head = row()
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(text(tr("Last session")).font(BODY_MED).color(u.muted))
        .child(
            text(fill(tr("crash from {}"), &[&crash.when]))
                .font(SMALL)
                .color(u.faint),
        );
    let at_secs = crash.at_secs;
    head = head.child(spacer()).child(
        u.action(tr("Dismiss"), ActionKind::Ghost)
            .key("launch-crash-dismiss")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| dismiss_crash(st, at_secs)),
    );

    let kicker = crash
        .exception
        .clone()
        .unwrap_or_else(|| tr("The game closed without an error code").to_string());
    let mut first = row()
        .key("launch-crash")
        .w_full()
        .min_h(52.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            icon(&icons::TRIANGLE_ALERT, 16.0)
                .color(u.amber)
                .a11y_hidden(),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(tr("Deadlock crashed")).font(BODY_MED).color(u.ink))
                .child(
                    text(format!("{kicker} · {}", crash.file))
                        .font(SMALL)
                        .color(u.muted)
                        .truncate(),
                ),
        );
    let path = crash.path.clone();
    first = first.child(
        u.action(tr("Show dump"), ActionKind::Ghost)
            .key("launch-crash-dump")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| crate::data::reveal_path(st, path.clone())),
    );

    let mut block = col().w_full().gap(0.0).child(head).child(first);
    if let Some(comment) = &crash.comment {
        block = block.child(
            row()
                .w_full()
                .min_h(44.0)
                .py(8.0)
                .gap(12.0)
                .items_start()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(el().w(16.0))
                .child(
                    text(comment.chars().take(400).collect::<String>())
                        .font(SMALL)
                        .color(u.prose)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap()
                        .selectable(true),
                ),
        );
    }
    if let Some(suspect) = &crash.suspect {
        let slot = suspect.slot;
        let name = suspect.name.clone();
        let reason = suspect
            .reason
            .line(suspect.file.as_deref(), suspect.change.as_ref(), now_secs());
        let blocked = s.settings.game_running || s.launch.monitor.running();
        block = block.child(
            row()
                .key("launch-crash-suspect")
                .w_full()
                .min_h(52.0)
                .py(8.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(el().w(16.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(fill(tr("Probably {}"), &[&name]))
                                .font(BODY_MED)
                                .color(u.ink),
                        )
                        .child(text(reason).font(SMALL).color(u.muted).wrap()),
                )
                .child(if crash.suspect_off {
                    // Off already (maybe since the crash): nothing to press, say so.
                    text(tr("Already turned off"))
                        .key("launch-crash-already-off")
                        .font(SMALL)
                        .color(u.muted)
                } else {
                    u.action(&fill(tr("Turn off {}"), &[&name]), ActionKind::Secondary)
                    .key("launch-crash-off")
                    .disabled(blocked)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if !crate::screens::settings::watch::refuse_while_running(st) {
                            crate::data::set_addon_enabled(st, slot, false);
                            dismiss_crash(st, at_secs);
                            crate::data::start_launch_read(st);
                        }
                    })
                }),
        );
    }
    block
}

fn dismiss_crash(st: &mut State, at_secs: i64) {
    let data = crate::data::data_dir();
    let _ = deadlock_core::launch::acknowledge_crash(&data, at_secs);
    if let Some(read) = &mut st.launch.read {
        read.crash = None;
    }
}

/// `Started 12 min ago, modded.` The mode is left out when the game was started outside Sigil.
fn started_line(marker: &LaunchMarker) -> String {
    let since = deadlock_core::launch::since_label(marker.started_secs, now_secs());
    match marker.mode.as_str() {
        deadlock_core::launch::MODE_MODDED => fill(tr("Started {}, modded."), &[&since]),
        deadlock_core::launch::MODE_VANILLA => fill(tr("Started {}, vanilla."), &[&since]),
        _ => fill(tr("Started {}."), &[&since]),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn mods_label(enabled: usize) -> String {
    if enabled == 1 {
        fill(tr("{} mod on"), &[&enabled.to_string()])
    } else {
        fill(tr("{} mods on"), &[&enabled.to_string()])
    }
}

/// The sidebar's running line: shows only while Deadlock is up, with the way to stop it.
///
/// The shell mounts it with
/// `.children(screens::launch::sidebar_state(s, u).into_iter().collect::<Vec<_>>())`
/// in `app.rs`'s `sidebar`, above the launch buttons. Until that line is added the function is
/// reachable but unused, so it stays allowed here rather than deleted.
#[allow(dead_code)]
pub fn sidebar_state(s: &State, u: &Ui) -> Option<El> {
    let process = s.launch.monitor.snapshot();
    if !process.running && !s.settings.game_running {
        return None;
    }
    Some(
        row()
            .key("sidebar-running")
            .w_full()
            .h(30.0)
            .px(12.0)
            .gap(8.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(tr("Deadlock is running. Stop game"))
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.go(Screen::Launch);
                st.launch.dialog = Dialog::StopGame;
            })
            .child(
                icon(&icons::PLAY, 12.0)
                    .color(u.teal)
                    .a11y_hidden(),
            )
            .child(
                text(tr("Running"))
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0),
            )
            .child(
                icon(&icons::SQUARE_STOP, 12.0)
                    .color(u.faint)
                    .a11y_hidden(),
            ),
    )
}

/// Mounted at the root so the launch confirmation overlays the shell. The overlap and
/// downloads dialogs ride on the same hook.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    match s.launch.dialog {
        Dialog::None => conflicts::dialog(s, u).or_else(|| downloads::dialog(s, u)),
        Dialog::Modded | Dialog::Vanilla | Dialog::Restore => Some(confirm(u, s, s.launch.dialog)),
        Dialog::Checklist => Some(checklist(u, s)),
        Dialog::StopGame => Some(stop_confirm(u, s)),
    }
}

/// Pressing Launch: the checklist first when something is off, then the plain confirm.
fn press_launch(st: &mut State, vanilla: bool) {
    st.launch.dialog = if vanilla || checklist_issues(st).is_empty() {
        Dialog::Modded
    } else {
        Dialog::Checklist
    };
}

/// What a player should know before starting the game. Every entry has a page that fixes it.
struct Issue {
    title: &'static str,
    detail: &'static str,
    fix: Fix,
}

enum Fix {
    Gameinfo,
    Overlap,
    Updates,
}

fn checklist_issues(s: &State) -> Vec<Issue> {
    let mut issues = Vec::new();
    let gameinfo_off = match &s.gameinfo.state {
        crate::banner::Gameinfo::NotListed { .. } | crate::banner::Gameinfo::Missing { .. } => true,
        crate::banner::Gameinfo::Loads => false,
        _ => s
            .launch
            .read
            .as_ref()
            .is_some_and(|read| !read.gameinfo_addons),
    };
    if gameinfo_off {
        issues.push(Issue {
            title: "Mods will not load",
            detail: "gameinfo.gi lost the addons line, so the game skips every mod.",
            fix: Fix::Gameinfo,
        });
    } else if matches!(s.gameinfo.state, crate::banner::Gameinfo::Incomplete { .. }) {
        issues.push(Issue {
            title: "gameinfo.gi is missing lines",
            detail: "Without the Mod and Write paths the game can crash at start, and without the language paths text breaks.",
            fix: Fix::Gameinfo,
        });
    }
    let overlaps = conflicts::badge_count(s);
    if overlaps > 0 {
        issues.push(Issue {
            title: "Two mods replace the same files",
            detail: if overlaps == 1 {
                "One file is shipped by more than one mod. The lower slot wins."
            } else {
                "Some files are shipped by more than one mod. The lower slot wins."
            },
            fix: Fix::Overlap,
        });
    }
    let review = s
        .release
        .review
        .as_ref()
        .map(|queue| queue.review_count())
        .unwrap_or(0);
    if review > 0 {
        issues.push(Issue {
            title: "Mods to check after the update",
            detail: "A game update changed files that your mods replace.",
            fix: Fix::Updates,
        });
    }
    issues
}

fn checklist(u: &Ui, s: &State) -> El {
    let issues = checklist_issues(s);
    let close = |st: &mut State| st.launch.dialog = Dialog::None;
    let mut rows = col().w_full().gap(0.0);
    for (index, issue) in issues.iter().enumerate() {
        let fix = match issue.fix {
            Fix::Gameinfo => "Fix mods",
            Fix::Overlap => "Open Overlap",
            Fix::Updates => "Open Updates",
        };
        let kind = match issue.fix {
            Fix::Gameinfo => 0,
            Fix::Overlap => 1,
            Fix::Updates => 2,
        };
        rows = rows.child(
            row()
                .key(format!("launch-check-{index}"))
                .w_full()
                .min_h(52.0)
                .py(8.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    icon(&icons::TRIANGLE_ALERT, 16.0)
                        .color(u.amber)
                        .a11y_hidden(),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(tr(issue.title)).font(BODY_MED).color(u.ink))
                        .child(text(tr(issue.detail)).font(SMALL).color(u.muted).wrap()),
                )
                .child(
                    u.action(tr(fix), ActionKind::Secondary)
                        .key(format!("launch-check-fix-{index}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            st.launch.dialog = Dialog::None;
                            match kind {
                                // The press is the approval; Undo puts the file back.
                                0 => fix_gameinfo(st),
                                1 => st.go(Screen::Conflicts),
                                _ => st.go(Screen::Updates),
                            }
                        }),
                ),
        );
    }
    u.dialog_shell()
        .key("launch-checklist")
        .role("dialog")
        .label(tr("Before you launch"))
        .w(520.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.launch.dialog = Dialog::None;
                true
            } else {
                false
            }
        })
        .child(text(tr("Before you launch")).font(H1).color(u.ink))
        .child(
            text(tr("A few things are off. Fix them, or start the game as it is."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(rows)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("launch-check-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.icon_action(&icons::PLAY, tr("Launch anyway"), ActionKind::Primary)
                        .key("launch-check-anyway")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.launch.dialog = Dialog::Modded),
                ),
        )
}

fn stop_confirm(u: &Ui, _s: &State) -> El {
    let close = |st: &mut State| st.launch.dialog = Dialog::None;
    u.dialog_shell()
        .key("launch-stop-confirm")
        .role("dialog")
        .label(tr("Stop Deadlock"))
        .w(440.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.launch.dialog = Dialog::None;
                true
            } else {
                false
            }
        })
        .child(text(tr("Stop Deadlock")).font(H1).color(u.ink))
        .child(
            text(tr("The game closes now. A match in progress is lost."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("launch-stop-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.icon_action(&icons::SQUARE_STOP, tr("Stop game"), ActionKind::Danger)
                        .key("launch-stop-run")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| stop_game_now(st, true)),
                ),
        )
}

/// Close the running game, as Stop game does. The launch overlay's Cancel uses it.
pub fn stop_game(st: &mut State) {
    stop_game_now(st, true);
}

/// Ask the game to close. The wait for a graceful exit runs on a worker; `launch_progress::poll`
/// takes its report, and a game that did not close is said out loud (DMM 2.4.0). A presence check
/// is asked for right away. Returns false when no worker started (headless renders).
fn stop_game_now(st: &mut State, tell: bool) -> bool {
    st.launch.dialog = Dialog::None;
    if rune::is_headless() {
        st.launch.last = Some(tr("Headless run: the game was not stopped.").to_string());
        return false;
    }
    let wake = st.wake.clone();
    st.launch.stop.start(wake, || deadlock_core::launch::stop_game(true).map_err(|error| error.to_string()));
    // Discord presence and the running row switch at once; the checks correct any surprise.
    st.settings.game_running = false;
    st.settings.last_game_check = None;
    if tell {
        crate::notify::push(
            st,
            crate::notify::Notice::info(tr("Closing Deadlock")).detail(tr("The game closes on its own.")),
        );
    }
    crate::data::start_launch_read(st);
    true
}

/// Close the game for the launch overlay's Cancel, which shows its own progress and error.
/// Returns false when no worker started.
pub fn stop_game_for_cancel(st: &mut State) -> bool {
    stop_game_now(st, false)
}

/// GAME WRITE, from a Fix press on the Launch page or its checklist: repair `gameinfo.gi` with
/// `game_apply::apply_gameinfo_repair`, which keeps the mounts the player or another tool added
/// (from the newest backup), with `deploy::repair_gameinfo` as the fallback when it errors. The
/// toast offers Undo from the backup either one made.
pub fn fix_gameinfo(st: &mut State) {
    use crate::notify::{self, Notice};
    if st.demo {
        st.gameinfo.state = crate::banner::Gameinfo::Loads;
        notify::push(st, Notice::info(tr("Demo library: nothing was changed.")));
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        notify::push(st, Notice::warning(tr("Set the game folder in Settings first.")));
        return;
    };
    let (changed, detail, backup) = match deadlock_core::game_apply::apply_gameinfo_repair(&game_root, true) {
        Ok(Some(done)) => {
            let detail = if done.restored.is_empty() {
                String::new()
            } else {
                fill(tr("Kept {} lines another tool or you had added."), &[&done.restored.len().to_string()])
            };
            (true, detail, Some(done.backup))
        }
        Ok(None) => (false, String::new(), None),
        Err(first) => match deadlock_core::deploy::repair_gameinfo(&deadlock_core::deploy::gameinfo_path(&game_root), true) {
            Ok(repair) => (repair.changed, String::new(), repair.backup),
            Err(error) => {
                notify::push(
                    st,
                    Notice::error(tr("Could not fix gameinfo.gi")).detail(format!("{first}. {error}")),
                );
                return;
            }
        },
    };
    st.gameinfo.state = crate::banner::Gameinfo::Loads;
    crate::data::rescan_game(st);
    let mut notice = Notice::success(if changed { tr("Fixed. Mods load again") } else { tr("Already fixed. Nothing changed") })
        .detail(detail);
    if let Some(backup) = backup {
        notice = notice.undo(move |st: &mut State| match deadlock_core::backups::restore(&backup, true) {
            Ok(_) => {
                crate::data::rescan_game(st);
                notify::push(st, Notice::info(tr("Put gameinfo.gi back as it was")));
            }
            Err(error) => notify::push(st, Notice::error(tr("Could not undo the fix")).detail(error.to_string())),
        });
    }
    notify::push(st, notice);
}

fn is_bad(message: &str) -> bool {
    let lower = message.to_lowercase();
    ["not ", "could not", "failed", "refus", "error"]
        .iter()
        .any(|word| lower.contains(word))
}

/// A quiet line with a leading icon: a result, or a real problem in the warn colour.
fn note_line(u: &Ui, message: &str, bad: bool) -> El {
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(
            el().pt(2.0).child(
                icon(
                    if bad {
                        &icons::TRIANGLE_ALERT
                    } else {
                        &icons::CHECK
                    },
                    14.0,
                )
                .color(if bad { u.amber } else { u.muted })
                .a11y_hidden(),
            ),
        )
        .child(
            text(message.to_string())
                .font(BODY)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

/// One of the two launch buttons: 64 points tall, an icon, a label and one short line.
fn big_action(u: &Ui, label: &str, detail: &str, primary: bool, enabled: bool) -> El {
    let (fill, ink, sub, border) = if primary {
        (u.accent, u.accent_ink, u.accent_ink.alpha(0.72), Color::TRANSPARENT)
    } else {
        (Color::WHITE.alpha(0.035), u.ink, u.muted, u.hair_strong)
    };
    row()
        .grow(1.0)
        .min_w(0.0)
        .h(64.0)
        .px(20.0)
        .gap(14.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(fill)
        .border(1.0, border)
        .hover(if primary {
            Color::WHITE.alpha(0.10)
        } else {
            u.hover
        })
        .press(Color::WHITE.alpha(0.08))
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.45 })
        .role("button")
        .label(label)
        .child(icon(&icons::PLAY, 18.0).color(ink).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(label.to_string())
                        .font(BODY_MED)
                        .color(ink)
                        .a11y_hidden(),
                )
                .child(
                    text(detail.to_string())
                        .font(SMALL)
                        .color(sub)
                        .truncate()
                        .a11y_hidden(),
                ),
        )
}

/// The vanilla stash as one row: what is set aside, and the action that brings it back.
fn stash_line(s: &State, u: &Ui, stash: Option<&deadlock_core::launch::StashStatus>) -> El {
    let (title, detail) = match stash {
        Some(stash) => (
            if stash.present == 1 {
                fill(tr("{} file set aside for a vanilla run"), &["1"])
            } else {
                fill(
                    tr("{} files set aside for a vanilla run"),
                    &[&stash.present.to_string()],
                )
            },
            fill(
                tr("Since {}. Restore before you change mods."),
                &[&format!(
                    "{} {}",
                    stash.created_at.chars().take(10).collect::<String>(),
                    stash.created_at.chars().skip(11).take(5).collect::<String>()
                )],
            ),
        ),
        None => (
            tr("Nothing set aside").to_string(),
            tr("A vanilla launch moves your mods aside and lists them here.").to_string(),
        ),
    };
    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(title).font(BODY_MED).color(u.ink).truncate())
        .child(text(detail).font(SMALL).color(u.faint).truncate());
    if let Some(problem) = stash.and_then(|stash| stash.problem.as_ref()) {
        text_col = text_col.child(text(problem.clone()).font(SMALL).color(u.rose).wrap());
    }
    let mut line = row()
        .key(if stash.is_some() {
            "launch-stash"
        } else {
            "launch-no-stash"
        })
        .w_full()
        .min_h(52.0)
        .py(8.0)
        .gap(16.0)
        .center_y()
        .border_sides([1.0, 0.0, 1.0, 0.0], u.hair)
        .child(text_col);
    if stash.is_some() {
        line = line.child(
            u.icon_action(&icons::ROTATE_CCW, tr("Restore mods"), ActionKind::Primary)
                .key("launch-restore")
                .disabled(s.launch.busy)
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.launch.dialog = Dialog::Restore),
        );
    } else {
        line = line.child(spacer());
    }
    line
}

fn confirm(u: &Ui, s: &State, dialog: Dialog) -> El {
    let (title, line, action) = match dialog {
        Dialog::Modded => (
            tr("Launch Deadlock"),
            tr("Steam starts the game with the mods that are on."),
            tr("Launch"),
        ),
        Dialog::Vanilla => (
            tr("Launch vanilla"),
            tr("Your mods are moved aside, then Steam starts the game. Restore them here afterwards."),
            tr("Launch vanilla"),
        ),
        Dialog::Restore => (
            tr("Restore mods"),
            tr("Mods move back into the addon folder. A file already there is kept, not overwritten."),
            tr("Restore"),
        ),
        _ => (tr("Launch"), tr("Nothing is pending."), tr("Close")),
    };
    let mods = match (dialog, s.launch.read.as_ref()) {
        (Dialog::Restore, Some(read)) => read
            .stash
            .as_ref()
            .map(|stash| {
                if stash.present == 1 {
                    fill(tr("{} file to restore"), &["1"])
                } else {
                    fill(
                        tr("{} files to restore"),
                        &[&stash.present.to_string()],
                    )
                }
            })
            .unwrap_or_else(|| tr("Nothing to restore").to_string()),
        (Dialog::Vanilla, Some(read)) => fill(
            tr("{} files move aside"),
            &[&read.addon_files.to_string()],
        ),
        (_, Some(read)) => fill(
            tr("{} of {} on"),
            &[&read.enabled.to_string(), &read.addon_files.to_string()],
        ),
        (_, None) => tr("Not read yet").to_string(),
    };
    let close = |st: &mut State| st.launch.dialog = Dialog::None;
    u.dialog_shell()
        .key("launch-confirm")
        .role("dialog")
        .label(title)
        .w(460.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(title).font(H1).color(u.ink))
                .child(text(line).font(BODY).color(u.muted).wrap()),
        )
        .child(col().w_full().child(u.kv(tr("Mods"), &mods, u.ink)))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action(action, ActionKind::Primary)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| match dialog {
                            // A launch runs through the progress overlay, which writes the
                            // launch options into Steam first.
                            Dialog::Modded => crate::screens::launch_progress::begin(st, false, true),
                            Dialog::Vanilla => crate::screens::launch_progress::begin(st, true, true),
                            other => crate::data::run_launch(st, other),
                        }),
                ),
        )
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A queue with one of every row, for the layout check and the picture.
pub fn stage_queue(st: &mut State) {
    st.launch.downloads.stage_demo();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_like_a_sentence() {
        assert_eq!(mods_label(1), "1 mod on");
        assert_eq!(mods_label(4), "4 mods on");
        assert!(is_bad("Mods were not restored"));
        assert!(!is_bad("Steam has the launch request"));
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn demo_read(running: bool, crash: Option<Crash>) -> Read {
    let now = now_secs();
    Read {
        stash: None,
        gameinfo_present: true,
        gameinfo_addons: true,
        addon_files: 6,
        enabled: 5,
        marker: running.then(|| LaunchMarker {
            schema: deadlock_core::launch::LAUNCH_STATE_SCHEMA,
            mode: deadlock_core::launch::MODE_MODDED.to_string(),
            started_at: String::new(),
            started_secs: now - 12 * 60,
            exited_at: None,
            exit_secs: None,
            acknowledged_secs: None,
        }),
        crash,
    }
}

fn demo_crash() -> Crash {
    let change = Change {
        slot: 3,
        name: "hud pack".to_string(),
        at: String::new(),
        at_secs: now_secs() - 2 * 86_400,
        file: "pak03_dir.vpk".to_string(),
    };
    Crash {
        path: "~/Steam/steamapps/common/Deadlock/game/bin/win64/deadlock_2026_1115_204501_1_crash.mdmp"
            .to_string(),
        file: "deadlock_2026_1115_204501_1_crash.mdmp".to_string(),
        when: "15 Nov, 20:45".to_string(),
        at_secs: now_secs() - 2 * 86_400,
        exception: Some("Access violation".to_string()),
        comment: Some(
            "FATAL ERROR: panorama/layout/hud/scoreboard.vxml_c failed to compile".to_string(),
        ),
        suspect: Some(Suspect {
            slot: 3,
            name: "hud pack".to_string(),
            reason: deadlock_core::impact::Reason::DumpNamesFile,
            file: Some("panorama/layout/hud/scoreboard.vxml_c".to_string()),
            change: Some(change),
        }),
        suspect_off: false,
    }
}

fn scene_base(st: &mut State) {
    st.load_demo();
    st.screen = Screen::Launch;
    st.launch.loaded = true;
}

/// `launch`: the page with nothing running.
pub fn scene_page(st: &mut State) {
    scene_base(st);
    st.launch.read = Some(demo_read(false, None));
}

/// `launch-running`: the running row and Stop game.
pub fn scene_running(st: &mut State) {
    scene_base(st);
    st.launch.read = Some(demo_read(true, None));
    st.settings.game_running = true;
}

/// `launch-crash`: the crash advisory with a likely culprit.
pub fn scene_crash(st: &mut State) {
    scene_base(st);
    st.launch.read = Some(demo_read(false, Some(demo_crash())));
}

/// `launch-checklist`: the overlay before a launch with a stale gameinfo.gi and overlaps.
pub fn scene_checklist(st: &mut State) {
    scene_base(st);
    st.launch.read = Some(demo_read(false, None));
    st.gameinfo.state = crate::banner::Gameinfo::NotListed {
        path: std::path::PathBuf::from("~/Steam/steamapps/common/Deadlock/game/citadel/gameinfo.gi"),
    };
    st.launch.dialog = Dialog::Checklist;
}

/// `launch-stop`: the confirm that closes the game.
pub fn scene_stop(st: &mut State) {
    scene_running(st);
    st.launch.dialog = Dialog::StopGame;
}

/// `launch-vanilla`: the vanilla confirm.
pub fn scene_vanilla(st: &mut State) {
    scene_page(st);
    st.launch.dialog = Dialog::Vanilla;
}

/// `launch-downloads`: the queue with one row of every state.
pub fn scene_downloads(st: &mut State) {
    scene_base(st);
    st.launch.read = Some(demo_read(false, None));
    stage_queue(st);
}

/// `launch-downloads-review`: the look inside a suspicious archive.
pub fn scene_downloads_review(st: &mut State) {
    scene_downloads(st);
    st.launch.downloads.dialog = downloads::Dialog::Review("d5".to_string());
}

// --- parity wave scenes --------------------------------------------------------------------------

/// `downloads-indicator`: the titlebar indicator over Installed while the queue runs.
pub fn scene_downloads_indicator(st: &mut State) {
    crate::app::scene_demo_installed(st);
    stage_queue(st);
}

/// `downloads-panel`: the indicator's list open over the page.
pub fn scene_downloads_panel(st: &mut State) {
    scene_downloads_indicator(st);
    st.launch.downloads.panel = true;
}

/// `downloads-slow`: the running row has crawled under 150 KB/s for a while.
pub fn scene_downloads_slow(st: &mut State) {
    scene_downloads(st);
    st.launch.downloads.demo_rates.insert("d1".to_string(), (412_000, true));
}

/// `downloads-drag`: the last waiting row dragged above the first.
pub fn scene_downloads_drag(st: &mut State) {
    scene_downloads(st);
    let items = st.launch.downloads.snapshot().items;
    let from = items.iter().position(|item| item.id == "d7").unwrap_or(0);
    let to = items.iter().position(|item| item.id == "d3").unwrap_or(0);
    st.launch.downloads.drag = Some(("d7".to_string(), from, to));
}

/// `launch-progress`: the overlay while Sigil writes the launch options.
pub fn scene_progress(st: &mut State) {
    crate::app::scene_demo_installed(st);
    crate::screens::launch_progress::stage(st, false, 1);
}

/// `launch-progress-waiting`: Steam has the game; the overlay waits for it.
pub fn scene_progress_waiting(st: &mut State) {
    crate::app::scene_demo_installed(st);
    crate::screens::launch_progress::stage(st, false, 3);
}
