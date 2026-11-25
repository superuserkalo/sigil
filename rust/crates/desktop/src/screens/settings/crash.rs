//! Sigil's own crashes, found at the next start (`deadlock_core::telemetry::crash`).
//!
//! - [`begin`] runs in `main` before the window: the panic hook, the Windows exception filter
//!   and the session marker. What the last run left behind waits in [`LAST`] for `setup`.
//! - [`end`] clears the marker on every clean exit: after `run` returns (window close, tray Quit,
//!   Cmd+Q) and before Restart to update starts the new version.
//! - [`on_startup`] decides: crash reports on, send it and say so in a toast; otherwise the calm
//!   dialog with what happened, the exact text that would go, Send report, Don't send and "Always
//!   send crash reports". Telemetry off or never answered still asks: the press is the consent for
//!   this one report (ADR 0014). The same crash declined in the last day is not asked again.
//! - Deadlock's own crashes are the Launch page's crash advisory, not this.

use std::path::PathBuf;
use std::sync::Mutex;

use deadlock_core::telemetry::{self, Consent, crash};
use rune::{El, Key, KeyEvent, col, row, spacer, text};

use super::i18n::{fill, tr};
use crate::app::State;
use crate::jobs::Job;
use crate::notify::{self, Notice};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Log lines kept with a report of a run that did not exit cleanly.
const TAIL_LINES: usize = 150;

/// What [`begin`] found, until `setup` takes it.
static LAST: Mutex<Option<crash::LastRun>> = Mutex::new(None);

/// The offer for the last run: the dialog, or the quiet send when crash reports are on.
#[derive(Default)]
pub struct Offer {
    pub last: Option<crash::LastRun>,
    /// The dialog is open.
    pub asking: bool,
    /// Exactly what Send report sends, read when the offer is made.
    pub preview: String,
    pub expanded: bool,
    /// "Always send crash reports", ticked in the dialog.
    pub always: bool,
    pub sending: Job<usize>,
    /// The send runs without the dialog (crash reports on): a failure stays quiet.
    pub quiet: bool,
}

fn data() -> PathBuf {
    crate::data::data_dir()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Before the window: install the crash hooks and start the session. Headless runs (snapshots,
/// scripts) and the helper subcommands never get here or skip it.
pub fn begin() {
    if rune::is_headless() {
        return;
    }
    let dir = data();
    telemetry::install_panic_hook(&dir, VERSION);
    #[cfg(windows)]
    native::install(&crash::native_path(&dir));
    let last = crash::begin_session(&dir, VERSION, now_ms(), boot_ms, || super::log::tail(TAIL_LINES));
    if let Some(last) = &last {
        super::log::info(format!(
            "The last run did not exit cleanly ({}); the report is waiting for an answer",
            last.kind
        ));
    }
    if let Ok(mut slot) = LAST.lock() {
        *slot = last;
    }
}

/// A clean exit: the marker goes, so the next start finds nothing.
pub fn end() {
    if rune::is_headless() {
        return;
    }
    crash::end_session(&data());
}

/// Every quit runs this before the process ends (window close, tray Quit, the quit key, quit
/// when the game starts, the end of a Windows session). Rune ends the process right after its
/// quit hooks, so code after `run` in `main` is not reached on a normal quit.
pub fn on_quit(st: &mut State, _tx: &mut rune::Tx) {
    super::product::on_quit(st);
    end();
}

/// In `setup`, once the consent file is read: send, ask or drop.
pub fn on_startup(st: &mut State) {
    let Some(last) = LAST.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    offer(st, last);
}

fn offer(st: &mut State, last: crash::LastRun) {
    let dir = data();
    let consent = st.settings.telemetry.consent;
    let ledger = crash::Ledger::read(&dir);
    match crash::decide(consent, &ledger, &last.fingerprint, now_ms()) {
        crash::Offer::Drop => crash::decline(&dir, &last.reports, &last.fingerprint),
        crash::Offer::SendNow => {
            // Offline: the files wait for the regular send once Sigil is online again.
            if st.prefs.offline {
                return;
            }
            st.settings.telemetry.crash.quiet = true;
            // The regular flush would send the same files; it waits its ten minutes instead.
            st.settings.telemetry.last_flush = Some(std::time::Instant::now());
            start_send(st, last.reports.clone(), true);
            st.settings.telemetry.crash.last = Some(last);
        }
        crash::Offer::Ask => {
            let c = &mut st.settings.telemetry.crash;
            c.preview = crash::preview(&dir, VERSION, &last.reports);
            c.asking = true;
            c.expanded = false;
            c.always = false;
            c.quiet = false;
            c.last = Some(last);
        }
    }
}

fn start_send(st: &mut State, reports: Vec<PathBuf>, flush_events: bool) {
    let wake = st.wake.clone();
    st.settings.telemetry.crash.sending.start(wake, move || {
        let dir = data();
        let sent = crash::send_reports(&dir, VERSION, &reports).map_err(|e| e.to_string());
        if flush_events {
            let _ = telemetry::flush(&dir, VERSION);
        }
        sent
    });
}

/// A finished send becomes a toast.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.settings.telemetry.crash.sending.try_take() else {
        return false;
    };
    let quiet = st.settings.telemetry.crash.quiet;
    let reports = st
        .settings
        .telemetry
        .crash
        .last
        .take()
        .map(|last| last.reports)
        .unwrap_or_default();
    match result {
        Ok(sent) if sent > 0 => {
            super::telemetry::event(st, "crash:send", None);
            let detail = if quiet {
                tr("Sigil closed unexpectedly last time. Crash reports are on in Settings, Privacy.")
            } else {
                tr("Thanks. It helps fix what went wrong.")
            };
            notify::push(st, Notice::success(tr("Crash report sent")).detail(detail));
        }
        // Nothing went: the same crash already went today. Nothing to say.
        Ok(_) => {}
        Err(error) => {
            super::log::info(format!("The crash report was not sent: {error}"));
            if !quiet {
                notify::push(
                    st,
                    Notice::error(tr("Crash report not sent")).detail(error).action(tr("Try again"), move |st: &mut State| {
                        retry(st, reports);
                    }),
                );
            }
        }
    }
    true
}

fn retry(st: &mut State, reports: Vec<PathBuf>) {
    if st.settings.telemetry.crash.sending.is_running() {
        return;
    }
    let reports: Vec<PathBuf> = reports.into_iter().filter(|path| path.exists()).collect();
    if reports.is_empty() {
        return;
    }
    st.settings.telemetry.crash.quiet = false;
    st.settings.telemetry.crash.last = Some(crash::LastRun {
        reports: reports.clone(),
        kind: String::new(),
        headline: String::new(),
        fingerprint: String::new(),
        version: VERSION.to_string(),
    });
    start_send(st, reports, false);
}

/// Send report: with "Always send" ticked, crash reports turn on for next time too.
fn send(st: &mut State) {
    let c = &mut st.settings.telemetry.crash;
    if c.sending.is_running() || st.demo || rune::is_headless() || st.prefs.offline {
        return;
    }
    c.asking = false;
    let reports = c.last.as_ref().map(|last| last.reports.clone()).unwrap_or_default();
    if c.always {
        let consent = always_on(st.settings.telemetry.consent);
        save(st, consent);
    }
    start_send(st, reports, false);
}

/// "Always send crash reports" answers for crash reports only; the other kinds stay as they were
/// (off when never asked) and keep their switches in Settings, Privacy.
fn always_on(mut consent: Consent) -> Consent {
    consent.asked = true;
    consent.crashes = true;
    consent
}

/// Don't send: the files go, and the same crash is not asked about again for a day.
fn dont_send(st: &mut State) {
    let c = &mut st.settings.telemetry.crash;
    c.asking = false;
    if let Some(last) = c.last.take()
        && !rune::is_headless()
        && !cfg!(test)
    {
        crash::decline(&data(), &last.reports, &last.fingerprint);
    }
}

fn save(st: &mut State, consent: Consent) {
    st.settings.telemetry.consent = consent;
    if rune::is_headless() || cfg!(test) || st.demo {
        return;
    }
    if let Err(error) = telemetry::write_consent(&data(), &consent) {
        notify::push(st, Notice::error(fill(tr("Settings could not be saved: {}"), &[&error.to_string()])));
    }
}

/// What happened, in one line for the dialog.
pub fn what_happened(last: &crash::LastRun) -> String {
    match last.kind.as_str() {
        crash::KIND_PANIC => fill(tr("It stopped on an error: {}"), &[&last.headline]),
        crash::KIND_NATIVE => fill(tr("Windows ended it with an exception: {}"), &[native_code(&last.headline)]),
        _ => tr("It ended without a clean exit and left no error message: a native crash, a graphics driver crash or a forced quit.").to_string(),
    }
}

/// `0xC0000005 (access violation)` out of the core's native headline.
fn native_code(headline: &str) -> &str {
    headline.split_once("exception ").map_or(headline, |(_, code)| code)
}

/// The dialog, drawn in the telemetry modal slot before the first-start question.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let c = &s.settings.telemetry.crash;
    if !c.asking {
        return None;
    }
    let last = c.last.as_ref()?;
    let offline = s.prefs.offline;
    let mut body = u
        .dialog_shell()
        .key("crash-offer")
        .role("dialog")
        .label(tr("Sigil closed unexpectedly last time"))
        .w(580.0)
        .padding(24.0)
        .gap(14.0)
        // An answer is needed; Escape is Don't send.
        .modal(true, |_st: &mut State| {})
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                dont_send(st);
                true
            } else {
                false
            }
        })
        .child(text(tr("Sigil closed unexpectedly last time")).font(H1).color(u.ink))
        .child(text(what_happened(last)).font(BODY).color(u.prose).wrap().selectable(true))
        .child(
            text(tr("A crash report helps fix it. It holds the error, where it happened and the last lines of Sigil's log, with your user name, paths, keys and emails taken out. Nothing else goes, and only if you press Send report."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        );
    let toggle_label = if c.expanded { tr("Hide what is sent") } else { tr("Show what is sent") };
    body = body.child(
        row()
            .w_full()
            .child(
                u.action(toggle_label, ActionKind::Ghost)
                    .key("crash-offer-preview")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        let c = &mut st.settings.telemetry.crash;
                        c.expanded = !c.expanded;
                    }),
            )
            .child(spacer()),
    );
    if c.expanded {
        body = body.child(
            u.scroll_area(col()
                .key("crash-offer-text")
                .w_full()
                .h(240.0)
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .child(text(c.preview.clone()).font(MONO).color(u.prose).wrap_anywhere().selectable(true).w_full())),
        );
    }
    body = body.child(
        row()
            .key("crash-offer-always")
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(tr("Always send crash reports")).font(BODY_MED).color(u.ink))
                    .child(
                        text(tr("Later crash reports go without this question. The switch is in Settings, Privacy."))
                            .font(SMALL)
                            .color(u.muted)
                            .wrap(),
                    ),
            )
            .child(
                u.toggle(c.always)
                    .key("crash-offer-always-switch")
                    .label(tr("Always send crash reports").to_string())
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        let c = &mut st.settings.telemetry.crash;
                        c.always = !c.always;
                    }),
            ),
    );
    if offline {
        body = body.child(
            text(tr("Offline mode is on, so Send report is off. Turn it off in Settings to send."))
                .font(SMALL)
                .color(u.amber)
                .wrap(),
        );
    }
    Some(
        body.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(spacer())
                .child(
                    u.action(tr("Don't send"), ActionKind::Secondary)
                        .key("crash-offer-no")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(dont_send),
                )
                .child(
                    u.action(tr("Send report"), ActionKind::Primary)
                        .key("crash-offer-send")
                        .disabled(offline || s.demo)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(send),
                ),
        ),
    )
}

/// When the system last booted, to tell a shutdown under a running Sigil from a crash.
fn boot_ms() -> Option<i64> {
    #[cfg(windows)]
    {
        // SAFETY: GetTickCount64 takes nothing and only reads a counter.
        let up = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() } as i64;
        Some(now_ms() - up)
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let line = stat.lines().find(|line| line.starts_with("btime "))?;
        let secs: i64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(secs * 1000)
    }
    #[cfg(target_os = "macos")]
    {
        // `{ sec = 1790000000, usec = 0 } Thu Sep ...`; read once, only when a marker was left.
        let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "kern.boottime"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let secs: i64 = text.split("sec = ").nth(1)?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
        Some(secs * 1000)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// The Windows exception filter: a process that dies on an unhandled SEH exception (an access
/// violation in a driver or a native library) never reaches the panic hook. The filter writes one
/// line (code, address, offset in Sigil.exe, thread id) with no allocation, then hands on to the
/// previous filter or Windows Error Reporting. [`begin`] folds the line into a report next start.
#[cfg(windows)]
mod native {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile};
    use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;

    static PATH: OnceLock<Vec<u16>> = OnceLock::new();
    static BASE: AtomicUsize = AtomicUsize::new(0);
    static PREVIOUS: AtomicUsize = AtomicUsize::new(0);

    type Filter = unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32;

    pub fn install(path: &Path) {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        if PATH.set(wide).is_err() {
            return;
        }
        // SAFETY: a null name asks for the executable's own module handle; nothing is freed.
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        BASE.store(base, Ordering::Relaxed);
        // SAFETY: `filter` is a plain function that lives for the whole process.
        let previous = unsafe { SetUnhandledExceptionFilter(Some(filter)) };
        PREVIOUS.store(previous.map_or(0, |f| f as usize), Ordering::Relaxed);
    }

    /// A fixed buffer with hex formatting: nothing here may allocate in a crashed process.
    struct Line {
        bytes: [u8; 160],
        len: usize,
    }

    impl Line {
        fn push(&mut self, text: &[u8]) {
            for &byte in text {
                if self.len < self.bytes.len() {
                    self.bytes[self.len] = byte;
                    self.len += 1;
                }
            }
        }

        fn hex(&mut self, value: u64, width: usize) {
            const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
            self.push(b"0x");
            let mut started = false;
            for index in (0..16).rev() {
                let nibble = ((value >> (index * 4)) & 0xF) as usize;
                if nibble != 0 || index < width || started {
                    started = true;
                    self.push(&[DIGITS[nibble]]);
                }
            }
        }

        fn decimal(&mut self, mut value: u64) {
            let mut digits = [0u8; 20];
            let mut count = 0;
            loop {
                digits[count] = b'0' + (value % 10) as u8;
                count += 1;
                value /= 10;
                if value == 0 {
                    break;
                }
            }
            for index in (0..count).rev() {
                self.push(&[digits[index]]);
            }
        }
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        // SAFETY: Windows passes valid pointers for the faulting thread; each is checked for null.
        unsafe {
            let mut line = Line { bytes: [0; 160], len: 0 };
            let (mut code, mut address) = (0u32, 0usize);
            if !info.is_null() && !(*info).ExceptionRecord.is_null() {
                let record = &*(*info).ExceptionRecord;
                code = record.ExceptionCode as u32;
                address = record.ExceptionAddress as usize;
            }
            line.push(b"code=");
            line.hex(u64::from(code), 8);
            line.push(b" address=");
            let base = BASE.load(Ordering::Relaxed);
            if base != 0 && address >= base && address - base < 0x4000_0000 {
                line.push(b"Sigil.exe+");
                line.hex((address - base) as u64, 1);
            } else {
                line.hex(address as u64, 1);
            }
            line.push(b" thread=");
            line.decimal(u64::from(GetCurrentThreadId()));
            line.push(b"\n");
            if let Some(path) = PATH.get() {
                let handle = CreateFileW(
                    path.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                );
                if handle != INVALID_HANDLE_VALUE {
                    let mut written = 0u32;
                    WriteFile(handle, line.bytes.as_ptr(), line.len as u32, &mut written, std::ptr::null_mut());
                    CloseHandle(handle);
                }
            }
            match PREVIOUS.load(Ordering::Relaxed) {
                0 => 0, // EXCEPTION_CONTINUE_SEARCH: Windows Error Reporting goes on as before.
                previous => {
                    let previous: Filter = std::mem::transmute::<usize, Filter>(previous);
                    previous(info)
                }
            }
        }
    }
}

/// Scenes: the dialog for a panic (collapsed or with the text shown), and an unclean exit.
pub fn stage_scene(st: &mut State, kind: &str, expanded: bool) {
    let (headline, preview) = match kind {
        crash::KIND_PANIC => (
            "called `Option::unwrap()` on a `None` value at rust/crates/desktop/src/viewer/worker.rs:212:41",
            "installId: 3f2504e0-4f89-41d3-9a0c-0305e82c3301 (checked by the server, never stored)\nversion: 0.1.16\nos: windows\nkind: panic\n\nmessage:\ncalled `Option::unwrap()` on a `None` value at rust/crates/desktop/src/viewer/worker.rs:212:41\n\nbacktrace:\n   0: std::backtrace::Backtrace::force_capture\n   1: deadlock_core::telemetry::write_crash\n   2: std::panicking::rust_panic_with_hook\n   3: dah_desktop::viewer::worker::render\n             at ./crates/desktop/src/viewer/worker.rs:212:41\n\ncontext:\nthread: sigil-viewer\nsession: started 2026-11-20 09:58:02 UTC, found at the next start 2026-11-20 10:06:40 UTC\n\nlast log lines:\n2026-11-20 10:05:11 INFO  Opened the 3D view for Haze\n2026-11-20 10:05:12 ERROR panic: called `Option::unwrap()` on a `None` value",
        ),
        _ => (
            "Sigil did not exit cleanly (no panic was recorded)",
            "installId: 3f2504e0-4f89-41d3-9a0c-0305e82c3301 (checked by the server, never stored)\nversion: 0.1.16\nos: windows\nkind: unclean_exit\n\nmessage:\nSigil did not exit cleanly (no panic was recorded)\n\nbacktrace:\nno panic was recorded\nlast log line: INFO  Opened the 3D view for Haze\n\ncontext:\nsession: started 2026-11-20 09:58:02 UTC, found at the next start 2026-11-20 10:06:40 UTC\n\nlast log lines:\n2026-11-20 09:58:02 INFO  Sigil 0.1.16 started on windows x86_64\n2026-11-20 10:05:11 INFO  Opened the 3D view for Haze",
        ),
    };
    let c = &mut st.settings.telemetry.crash;
    c.last = Some(crash::LastRun {
        reports: Vec::new(),
        kind: kind.to_string(),
        headline: headline.to_string(),
        fingerprint: "scene".to_string(),
        version: VERSION.to_string(),
    });
    c.preview = preview.to_string();
    c.asking = true;
    c.expanded = expanded;
}

/// `crash-sent`: the toast a player with crash reports on sees at start.
pub fn stage_sent(st: &mut State) {
    notify::push(
        st,
        Notice::success(tr("Crash report sent"))
            .detail(tr("Sigil closed unexpectedly last time. Crash reports are on in Settings, Privacy.")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last(kind: &str) -> crash::LastRun {
        crash::LastRun {
            reports: Vec::new(),
            kind: kind.to_string(),
            headline: "Sigil stopped on a Windows exception 0xC0000005 (access violation)".to_string(),
            fingerprint: "fp".to_string(),
            version: VERSION.to_string(),
        }
    }

    #[test]
    fn each_kind_has_its_one_line() {
        assert!(what_happened(&last(crash::KIND_NATIVE)).ends_with("0xC0000005 (access violation)"));
        assert!(what_happened(&last(crash::KIND_UNCLEAN)).contains("without a clean exit"));
        let mut panic = last(crash::KIND_PANIC);
        panic.headline = "index out of bounds".to_string();
        assert_eq!(what_happened(&panic), "It stopped on an error: index out of bounds");
    }

    #[test]
    fn opted_out_players_are_asked_and_dont_send_closes_without_sending() {
        let mut st = State::default();
        st.settings.telemetry.consent = Consent { asked: true, ..Consent::default() };
        stage_scene(&mut st, crash::KIND_PANIC, false);
        assert!(dialog_open(&st));
        dont_send(&mut st);
        assert!(!dialog_open(&st));
        assert!(st.settings.telemetry.crash.last.is_none());
        assert!(!st.settings.telemetry.crash.sending.is_running());
    }

    #[test]
    fn always_send_turns_on_crash_reports_only() {
        let never = always_on(Consent::default());
        assert!(never.asked && never.crashes && !never.usage && !never.hash_lookup);
        let usage = always_on(Consent { asked: true, usage: true, ..Consent::default() });
        assert!(usage.usage && usage.crashes && !usage.hash_lookup);
    }

    /// Every UI literal above the scenes has German in one of the tables (read from source, so
    /// this file needs no entry in `i18n`'s routed list).
    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn every_string_is_translated() {
        let tables = [
            include_str!("locales.rs"),
            include_str!("locales_parity.rs"),
            include_str!("locales_wave.rs"),
            include_str!("locales_w4a.rs"),
            include_str!("locales_w4b.rs"),
        ];
        let keys: std::collections::HashSet<String> = tables
            .iter()
            .flat_map(|source| source.lines())
            .filter_map(|line| line.trim_start().strip_prefix("(\""))
            .filter_map(|rest| rest.split_once("\", \"").map(|(en, _)| en.to_string()))
            .chain(
                tables
                    .iter()
                    .flat_map(|source| source.lines())
                    .map(str::trim)
                    .filter(|line| line.starts_with('"') && line.ends_with("\","))
                    .map(|line| line.trim_end_matches(',').trim_matches('"').to_string()),
            )
            .collect();
        let source = include_str!("crash.rs");
        let code = source.split("/// Scenes:").next().unwrap_or(source);
        let mut missing = Vec::new();
        for call in code.split("tr(\"").skip(1) {
            let literal = call.split('"').next().unwrap_or_default();
            if !keys.contains(literal) {
                missing.push(literal.to_string());
            }
        }
        assert!(missing.is_empty(), "untranslated: {missing:#?}");
    }

    fn dialog_open(st: &State) -> bool {
        st.settings.telemetry.crash.asking && st.settings.telemetry.crash.last.is_some()
    }
}
