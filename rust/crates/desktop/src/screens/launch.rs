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
use rune::{Color, El, Key, KeyEvent, col, icon, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, PAGE_TOP, PAGE_X, SMALL, SMALL_NUM, Ui};
use crate::screens::launch_progress::tr;
use crate::screens::settings::i18n::fill;

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
    /// The file the game was loading, the console around the crash and the HUD check
    /// (`launch_crash.rs`).
    pub evidence: crate::screens::launch_crash::Evidence,
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
    /// The crash card's evidence is open (closed by default, so the card keeps one height).
    pub crash_details: bool,
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
            crash_details: false,
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
    /// Set once the thread has read the process list, so "not running" is an answer.
    checked: Arc<AtomicBool>,
}

impl Default for Monitor {
    fn default() -> Self {
        Monitor {
            shared: Arc::new(Mutex::new(GameProcess::default())),
            stop: None,
            started: false,
            checked: Arc::new(AtomicBool::new(false)),
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
        let checked = Arc::clone(&self.checked);
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
                    let first = !checked.swap(true, Ordering::SeqCst);
                    if (changed || first) && let Some(wake) = &wake {
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

    /// True once the watch thread runs (never in headless renders).
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// True once the thread has read the process list at least once.
    pub fn has_checked(&self) -> bool {
        self.checked.load(Ordering::SeqCst)
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
    let process = v.monitor.snapshot();
    let running = process.running || s.settings.game_running;

    // One toolbar line: the title, then the last result in the room after it. The result
    // never pushes the page, and Recheck sits on the Game section, on the column's edge.
    let head = col()
        .key("launch-head")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .child(u.page_toolbar(u.page_title(tr("Launch")), vec![status_line(u, v.last.as_deref())], None, Vec::new()));

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
        let mut game = col().w_full().gap(24.0).child(
            col()
                .w_full()
                .gap(8.0)
                .child(
                    u.section_head(
                        tr("Game"),
                        None,
                        Some(
                            u.icon_button(&icons::REFRESH_CW, tr("Recheck"))
                                .size(24.0, 24.0)
                                .key("launch-recheck")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| crate::data::start_launch_read(st)),
                        ),
                    )
                    .pr(16.0 - 4.0),
                )
                .child(game_panel(s, u, read, &process, running)),
        );
        if let Some(crash) = read.and_then(|read| read.crash.as_ref()) {
            game = game.child(crash_card(s, u, crash));
        }
        // Full width: the game and its crash on the left, downloads on the right, one top
        // edge. Narrower: one reading column.
        if crate::ui::fit() == crate::ui::Fit::Full {
            row()
                .w_full()
                .gap(24.0)
                .items_start()
                .child(game.grow(1.0).min_w(0.0))
                .child(col().grow(1.0).min_w(0.0).child(downloads::section(s, u)))
        } else {
            game.max_w(COLUMN_W).child(downloads::section(s, u))
        }
    };

    col()
        .key("launch-root")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(head)
        .child(
            col()
                .key("launch-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .key("launch-page")
                        .w_full()
                        .px(PAGE_X)
                        .pt(16.0)
                        .pb(32.0)
                        .child(body),
                ),
        )
}

/// The reading column when the page is too narrow for two.
const COLUMN_W: f32 = 720.0;
/// One Game panel row, a crash suspect row and a download row: all the same 56 points.
const PANEL_ROW_H: f32 = 56.0;

/// The last result on the toolbar line after the title (the shared `status_line`, cut with a
/// tooltip). Empty when there is none, so a result never pushes the page.
fn status_line(u: &Ui, last: Option<&str>) -> El {
    let mut line = row().key("launch-status").pl(8.0).h(24.0).gap(8.0).center_y().min_w(0.0).shrink();
    if let Some(message) = last {
        let bad = is_bad(message);
        line = line.role(if bad { "alert" } else { "status" }).child(u.status_line(
            if bad { &icons::TRIANGLE_ALERT } else { &icons::CHECK },
            message,
            if bad { crate::ui::Tone::Warn } else { crate::ui::Tone::Ok },
        ));
        if bad {
            line = line.child(crate::notify::feedback::button(u, "launch-last-feedback", message, ""));
        }
    }
    line
}

/// One fixed 56 point row: icon, title over one line, controls on the right.
fn panel_row(u: &Ui, key: &str, glyph: &'static rune::Icon, tint: Color, title: &str, line: El, trailing: Option<El>, last: bool) -> El {
    let mut r = row()
        .key(key.to_string())
        .w_full()
        .h(PANEL_ROW_H)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(icon(glyph, 16.0).color(tint).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(title.to_string()).font(BODY_MED).color(u.ink).truncate())
                .child(line),
        );
    if let Some(trailing) = trailing {
        r = r.child(trailing);
    }
    if !last {
        r = r.border_sides([0.0, 0.0, 1.0, 0.0], u.hair);
    }
    r
}

fn line_text(s: impl Into<String>, color: Color) -> El {
    let s = s.into();
    text(s.clone()).font(SMALL_NUM).color(color).truncate().help_when_truncated(s)
}

/// The Game panel: three rows, always all three, at fixed heights. A state change swaps what a
/// row says, never how tall it is or where it sits.
fn game_panel(s: &State, u: &Ui, read: Option<&Read>, process: &GameProcess, running: bool) -> El {
    let v = &s.launch;
    let pending = read.and_then(|read| read.stash.as_ref());
    let stashed = pending.is_some();
    let ready = !stashed && !v.busy && !running;

    // Row 1: the game.
    let game = if running {
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
        panel_row(
            u,
            "launch-running",
            &icons::PLAY,
            u.teal,
            tr("Deadlock is running"),
            line_text(detail, u.muted),
            Some(
                row().min_w(LAUNCH_PAIR_W).gap(8.0).center_y().child(spacer()).child(
                    u.action_m(tr("Stop game"), ActionKind::Secondary)
                        .key("launch-stop")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.launch.dialog = Dialog::StopGame),
                ),
            ),
            false,
        )
    } else {
        let vanilla_help = if stashed {
            tr("Restore the mods set aside first.")
        } else {
            tr("Moves your mods aside, then starts the game. Restore them here afterwards.")
        };
        let button = |label: &str, key: &str| {
            u.action_m(label, ActionKind::Secondary)
                .key(key.to_string())
                .disabled(!ready)
                .opacity(if ready { 1.0 } else { 0.45 })
                .cursor(if ready { rune::Cursor::Pointer } else { rune::Cursor::Default })
        };
        let pair = row()
            .min_w(LAUNCH_PAIR_W)
            .gap(8.0)
            .center_y()
            .child(spacer())
            .child(
                button(tr("Launch vanilla"), "launch-go-vanilla")
                    .help(vanilla_help)
                    .on_press(move |st: &mut State| {
                        if ready {
                            st.launch.dialog = Dialog::Vanilla;
                        }
                    }),
            )
            .child(button(tr("Launch modded"), "launch-go-modded").on_press(move |st: &mut State| {
                if ready {
                    press_launch(st, false);
                }
            }));
        panel_row(
            u,
            "launch-game",
            &icons::PLAY,
            u.muted,
            if stashed { tr("Restore mods to launch") } else { tr("Ready to launch") },
            line_text(tr("Deadlock is not running"), u.muted),
            Some(pair),
            false,
        )
    };

    // Row 2: the mods and gameinfo.gi.
    let broken = read.is_some_and(|read| !read.gameinfo_present || !read.gameinfo_addons);
    let mods_title = match read {
        Some(read) => fill(
            if read.addon_files == 1 { tr("{} of {} mod on") } else { tr("{} of {} mods on") },
            &[&read.enabled.to_string(), &read.addon_files.to_string()],
        ),
        None => tr("Your enabled mods").to_string(),
    };
    let mods = if broken {
        panel_row(
            u,
            "launch-mods",
            &icons::TRIANGLE_ALERT,
            u.amber,
            &mods_title,
            line_text(tr("gameinfo.gi does not load mods"), u.prose),
            Some(
                u.action_m(tr("Fix mods"), ActionKind::Secondary)
                    .key("launch-fix-gameinfo")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(fix_gameinfo),
            ),
            false,
        )
    } else {
        panel_row(
            u,
            "launch-mods",
            &icons::CHECK,
            u.teal,
            &mods_title,
            line_text(if read.is_some() { tr("gameinfo.gi loads mods") } else { tr("Not read yet") }, u.muted),
            None,
            false,
        )
    };

    // Row 3: the vanilla stash.
    let stash = match pending {
        Some(stash) => {
            let title = fill(
                if stash.present == 1 { tr("{} file set aside") } else { tr("{} files set aside") },
                &[&stash.present.to_string()],
            );
            let line = match &stash.problem {
                Some(problem) => row()
                    .gap(6.0)
                    .center_y()
                    .min_w(0.0)
                    .child(icon(&icons::TRIANGLE_ALERT, 12.0).color(u.rose).a11y_hidden())
                    .child(line_text(problem.clone(), u.prose).shrink().min_w(0.0)),
                None => line_text(
                    fill(
                        tr("Since {}. Restore before you change mods."),
                        &[&format!(
                            "{} {}",
                            stash.created_at.chars().take(10).collect::<String>(),
                            stash.created_at.chars().skip(11).take(5).collect::<String>()
                        )],
                    ),
                    u.muted,
                ),
            };
            panel_row(
                u,
                "launch-stash",
                &icons::BOX,
                u.amber,
                &title,
                line,
                Some(
                    u.icon_action_m(&icons::ROTATE_CCW, tr("Restore mods"), ActionKind::Secondary)
                        .key("launch-restore")
                        .disabled(v.busy)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            if !st.launch.busy {
                                st.launch.dialog = Dialog::Restore;
                            }
                        }),
                ),
                true,
            )
        }
        None => panel_row(u, "launch-no-stash", &icons::BOX, u.faint, tr("Vanilla stash"), line_text(tr("Empty"), u.muted), None, true),
    };

    u.panel().key("launch-game-panel").w_full().radius(crate::ui::r(8.0)).child(game).child(mods).child(stash)
}

/// Room for the two launch buttons, so swapping them for Stop game moves nothing.
const LAUNCH_PAIR_W: f32 = 232.0;

/// The crash advisory: the suspect and its fix first, then the dump, then the evidence behind
/// a disclosure. Fixed 120 points while Details is closed.
fn crash_card(s: &State, u: &Ui, crash: &Crash) -> El {
    let at_secs = crash.at_secs;
    let head = row()
        .w_full()
        .h(24.0)
        .gap(8.0)
        .center_y()
        .pr(8.0)
        .child(u.section(tr("Last crash")))
        .child(text(crash.when.clone()).font(SMALL_NUM).color(u.faint))
        .child(spacer())
        .child(
            u.action(tr("Dismiss"), ActionKind::Ghost)
                .h(24.0)
                .px(8.0)
                .key("launch-crash-dismiss")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| dismiss_crash(st, at_secs)),
        );

    // Row 1: the suspect and the one action that helps.
    let blocked = s.settings.game_running || s.launch.monitor.running();
    let suspect = match &crash.suspect {
        Some(suspect) => {
            let slot = suspect.slot;
            let name = suspect.name.clone();
            let reason = crate::screens::launch_crash::reason_line(suspect, now_secs());
            let trailing = if crash.suspect_off {
                text(tr("Already off"))
                    .key("launch-crash-already-off")
                    .font(SMALL)
                    .color(u.muted)
            } else {
                u.action_m(&fill(tr("Turn off {}"), &[&name]), ActionKind::Secondary)
                    .key("launch-crash-off")
                    .max_w(200.0)
                    .disabled(blocked)
                    .opacity(if blocked { 0.45 } else { 1.0 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if !crate::screens::settings::watch::refuse_while_running(st) {
                            crate::data::set_addon_enabled(st, slot, false);
                            dismiss_crash(st, at_secs);
                            crate::data::start_launch_read(st);
                        }
                    })
            };
            panel_row(
                u,
                "launch-crash-suspect",
                &icons::TRIANGLE_ALERT,
                u.amber,
                &fill(tr("Probably {}"), &[&name]),
                line_text(reason, u.muted),
                Some(trailing),
                false,
            )
        }
        None => panel_row(
            u,
            "launch-crash-suspect",
            &icons::TRIANGLE_ALERT,
            u.amber,
            tr("Deadlock crashed"),
            line_text(tr("No installed mod is named in the report."), u.muted),
            None,
            false,
        ),
    };

    // Row 2: what the dump says and where it is.
    let kicker = crash
        .exception
        .clone()
        .unwrap_or_else(|| tr("The game closed without an error code").to_string());
    let path = crash.path.clone();
    let dump = row()
        .key("launch-crash")
        .w_full()
        .h(32.0)
        .pl(16.0)
        .pr(8.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(icon(&icons::FILE, 16.0).color(u.faint).a11y_hidden())
        .child(text(kicker).font(BODY).color(u.prose).truncate())
        .child(
            text(crash.file.clone())
                .font(MONO)
                .color(u.muted)
                .truncate_middle()
                .help_when_truncated(crash.file.clone())
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            u.action(tr("Show dump"), ActionKind::Ghost)
                .h(24.0)
                .px(8.0)
                .key("launch-crash-dump")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| crate::data::reveal_path(st, path.clone())),
        );

    // Row 3: the evidence, closed by default.
    let open = s.launch.crash_details;
    let disclosure = row()
        .key("launch-crash-details")
        .w_full()
        .h(32.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .expanded(open)
        .label(tr("Details"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.launch.crash_details = !st.launch.crash_details)
        .child(
            icon(if open { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 16.0)
                .color(u.muted)
                .a11y_hidden(),
        )
        .child(text(tr("Details")).font(BODY).color(u.prose).a11y_hidden());

    let mut card = u
        .panel()
        .key("launch-crash-card")
        .w_full()
        .radius(crate::ui::r(8.0))
        .child(suspect)
        .child(dump)
        .child(disclosure);
    if open {
        card = card.child(
            col()
                .w_full()
                .px(16.0)
                .pb(16.0)
                .child(
                    col()
                        .key("launch-crash-evidence")
                        .w_full()
                        .max_h(240.0)
                        .padding(12.0)
                        .gap(8.0)
                        .fill(u.well)
                        .radius(crate::ui::r(4.0))
                        .scroll(rune::ScrollSpec { stick_bottom: false })
                        .children(crate::screens::launch_crash::groups(u, &crash.evidence, crash.suspect.as_ref(), crash.comment.as_deref())),
                ),
        );
    }
    col().w_full().gap(8.0).child(head).child(card)
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
    let mut rows = u.panel().w_full().radius(crate::ui::r(8.0));
    let last = issues.len().saturating_sub(1);
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
            panel_row(
                u,
                &format!("launch-check-{index}"),
                &icons::TRIANGLE_ALERT,
                u.amber,
                tr(issue.title),
                line_text(tr(issue.detail), u.muted),
                Some(row().w(128.0).center_y().child(spacer()).child(
                    u.action_m(tr(fix), ActionKind::Secondary)
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
                )),
                index == last,
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
        .child(rows)
        .child(
            row()
                .w_full()
                .gap(8.0)
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
        .w(480.0)
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
                .color(u.prose)
                .max_w(432.0)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
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
                        .on_press(|st: &mut State| {
                            stop_game_now(st, true);
                        }),
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
        .w(480.0)
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
                .child(text(line).font(BODY).color(u.prose).max_w(432.0).wrap()),
        )
        .child(col().w_full().child(u.kv(tr("Mods"), &mods, u.ink)))
        .child(
            row()
                .w_full()
                .gap(8.0)
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
            "error\n\nAbortMessage({FATAL ERROR: Unable to load layout file 'file://{resources}/layout/hud/scoreboard.xml'})\nUptime( 41.2 )\nBuild: 6170 (revision 10811520)".to_string(),
        ),
        suspect: Some(Suspect {
            slot: 3,
            name: "hud pack".to_string(),
            reason: deadlock_core::impact::Reason::DumpNamesFile,
            file: Some("panorama/layout/hud/scoreboard.vxml_c".to_string()),
            change: Some(change),
        }),
        suspect_off: false,
        evidence: crate::screens::launch_crash::Evidence {
            resource: Some("panorama/layout/hud/scoreboard.vxml_c".to_string()),
            console: vec![
                "Unable to find child 'ScoreboardStreetBrawl' in layout panorama/layout/hud/scoreboard.vxml".to_string(),
                "Unable to load layout file 'panorama/layout/hud/scoreboard.vxml'".to_string(),
            ],
            hud: Some(deadlock_core::addon_checks::HudCheck {
                slot: 3,
                layout: "panorama/layout/hud/scoreboard.vxml_c".to_string(),
                status: deadlock_core::addon_checks::HudStatus::Compared,
                missing_panels: vec!["ScoreboardStreetBrawl".to_string(), "ScoreboardMvp".to_string()],
                detail: None,
                console: Vec::new(),
            }),
            build: Some("6170 (revision 10811520)".to_string()),
            uptime_seconds: Some(41.2),
        },
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
