//! Opt-in telemetry to Sigil's own Cloudflare Worker (ADR 0014). `deadlock_core::telemetry`
//! owns consent, the queue, scrubbing and every send; this is the app side.
//!
//! - The first-start question (Deadlock Mod Manager's consent dialog, behaviour only): after the
//!   first-run setup, a plain list of what is sent and Yes or No. It cannot be dismissed without
//!   an answer, and nothing is queued or sent before one.
//! - Settings > Privacy: one switch each for usage events, crash reports and unknown-mod lookup,
//!   and a link to `docs/privacy.md`.
//! - Settings > Logs and support > Send diagnostics: the scrubbed text first, sent only on the
//!   press, then the reference code with Copy.
//! - At startup the crash hook goes in; a worker sends pending crash files and flushes the event
//!   queue at startup and every 10 minutes. Every send runs on a worker, never on the UI thread.
//! - [`event`] queues a usage event (allowlisted names only) at page changes and main actions.
//!   Grimoire sends nothing; DMM sends usage events to Google Analytics after its consent.

use std::time::{Duration, Instant};

use deadlock_core::telemetry::{self, Consent};
use rune::{El, Key, KeyEvent, Tx, col, row, spacer, text};

use super::i18n::{fill, tr};
use super::{button, detail, section, setting_row, toggle_row};
use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, H1, MONO, SMALL, Ui};

/// How often pending crash files and queued events go out while Sigil runs.
pub const FLUSH_EVERY: Duration = Duration::from_secs(600);
/// What each kind of data is for, shown in Settings and in the question.
pub const PRIVACY_DOC: &str = "https://github.com/superuserkalo/sigil/blob/main/docs/privacy.md";

#[derive(Default)]
pub struct Telemetry {
    pub consent: Consent,
    pub loaded: bool,
    /// The question is open (first start, or a scene).
    pub asking: bool,
    pub last_flush: Option<Instant>,
    pub flush: Job<usize>,
    /// Send diagnostics: the scrubbed text shown before the press, then the code.
    pub preview: Option<String>,
    pub sending: Job<String>,
    pub code: Option<Result<String, String>>,
    /// Sigil's own crash in the last run: the dialog or the quiet send (`crash.rs`).
    pub crash: super::crash::Offer,
}

fn data() -> std::path::PathBuf {
    crate::data::data_dir()
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Startup: the crash hook, the configuration `identify` reads, the consent file. Headless and
/// test runs install nothing and ask nothing.
pub fn on_startup(st: &mut State) {
    if rune::is_headless() || cfg!(test) {
        return;
    }
    let dir = data();
    telemetry::configure(&dir, VERSION);
    telemetry::install_panic_hook(&dir, VERSION);
    st.settings.telemetry.consent = telemetry::read_consent(&dir);
    st.settings.telemetry.loaded = true;
    event(st, "app:start", None);
    super::crash::on_startup(st);
}

/// Queue one usage event. A no-op without usage consent, in the demo, a headless run or a test;
/// the core also drops any name outside `telemetry::EVENTS`.
pub fn event(st: &State, name: &str, page: Option<&str>) {
    if rune::is_headless() || cfg!(test) || st.demo || !st.settings.telemetry.loaded {
        return;
    }
    let c = st.settings.telemetry.consent;
    if !(c.asked && c.usage) {
        return;
    }
    telemetry::record(&data(), name, page);
}

/// The event a page change records, for the pages the Worker knows.
pub fn page_event(screen: crate::model::Screen) -> Option<&'static str> {
    use crate::model::Screen;
    Some(match screen {
        Screen::Browse => "page:browse",
        Screen::Installed => "page:installed",
        Screen::Profiles => "page:profiles",
        // The Worker knows one name for the old Launch page, which held both; a separate
        // name for each needs the Worker's list to change first.
        Screen::Launch | Screen::Downloads => "page:downloads",
        Screen::Locker => "page:locker",
        Screen::Crosshair => "page:crosshair",
        Screen::Autoexec => "page:autoexec",
        Screen::Performance => "page:performance",
        Screen::Servers => "page:servers",
        Screen::Stats => "page:stats",
        Screen::Character => "page:character",
        Screen::Settings => "page:settings",
        _ => return None,
    })
}

/// The question opens once the first-run setup is done; the flush worker runs every 10 minutes.
pub fn poll(st: &mut State) -> bool {
    let mut changed = super::crash::poll(st);
    let t = &mut st.settings.telemetry;
    if let Some(_result) = t.flush.try_take() {
        changed = true;
    }
    if let Some(result) = t.sending.try_take() {
        t.code = Some(result);
        t.preview = None;
        changed = true;
    }
    if !t.loaded || st.demo || rune::is_headless() {
        return changed;
    }
    if !t.consent.asked && !t.asking && !t.crash.asking && super::options::get().onboarded {
        t.asking = true;
        changed = true;
    }
    let due = t.last_flush.is_none_or(|at| at.elapsed() >= FLUSH_EVERY);
    if due && t.consent.asked && !st.prefs.offline && !t.flush.is_running() {
        t.last_flush = Some(Instant::now());
        let wake = st.wake.clone();
        t.flush.start(wake, || {
            let dir = data();
            let crashes = telemetry::send_pending_crashes(&dir, VERSION).unwrap_or(0);
            let events = telemetry::flush(&dir, VERSION).unwrap_or(0);
            Ok(crashes + events)
        });
    }
    changed
}

fn save(st: &mut State, consent: Consent) {
    st.settings.telemetry.consent = consent;
    if rune::is_headless() || cfg!(test) || st.demo {
        return;
    }
    if let Err(error) = telemetry::write_consent(&data(), &consent) {
        st.settings.message = Some((fill(tr("Settings could not be saved: {}"), &[&error.to_string()]), true));
    }
}

fn answer(st: &mut State, yes: bool) {
    st.settings.telemetry.asking = false;
    save(st, Consent { asked: true, usage: yes, crashes: yes, hash_lookup: yes });
    if yes {
        event(st, "app:start", None);
    }
}

fn flip(st: &mut State, change: fn(&mut Consent)) {
    let mut c = st.settings.telemetry.consent;
    c.asked = true;
    change(&mut c);
    save(st, c);
    event(st, "settings:change", Some("page:settings"));
}

/// Sigil's own crash in the last run first, then the first-start question.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    super::crash::dialog(s, u).or_else(|| question(s, u))
}

/// The first-start question.
fn question(s: &State, u: &Ui) -> Option<El> {
    if !s.settings.telemetry.asking {
        return None;
    }
    let item = |line: &str| {
        row()
            .w_full()
            .gap(8.0)
            .child(text("·").font(BODY).color(u.faint))
            .child(text(tr(line).to_string()).font(BODY).color(u.prose).wrap().grow(1.0).min_w(0.0))
    };
    Some(
        u.dialog_shell()
            .key("telemetry-consent")
            .role("dialog")
            .label(tr("Help improve Sigil?"))
            .w(500.0)
            .padding(24.0)
            .gap(14.0)
            // An answer is required: no scrim press and no Escape close it.
            .modal(true, |_st: &mut State| {})
            .on_key(|_st: &mut State, ev: &KeyEvent, _| ev.key == Key::Escape)
            .child(text(tr("Help improve Sigil?")).font(H1).color(u.ink))
            .child(
                text(tr("With your yes, Sigil sends these to its own server. Nothing is sent before you answer, and each one has its own switch in Settings, Privacy."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(
                col()
                    .w_full()
                    .gap(6.0)
                    .child(item("Which pages you open and which main actions you use, with Sigil's version and your system. No mod names, no paths, no account."))
                    .child(item("Crash reports: the error and where it happened, with your user name, paths, keys and emails taken out."))
                    .child(item("Unknown-mod lookup: the hash and size of a mod file you ask about, and of files you install from GameBanana with their mod and file id."))
                    .child(item("A random id made on this computer, so repeated events count once. Not your Steam id, not a machine id.")),
            )
            .child(
                text(tr("Never sent: your mods, your files, your keys, your Steam account or your IP address (the server does not keep it). Logs go only when you press Send diagnostics."))
                    .font(SMALL)
                    .color(u.faint)
                    .wrap(),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        u.action(tr("What Sigil sends"), ActionKind::Ghost)
                            .key("telemetry-doc")
                            .role("link")
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(|st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, PRIVACY_DOC)),
                    )
                    .child(spacer())
                    .child(
                        u.action(tr("No, thanks"), ActionKind::Secondary)
                            .key("telemetry-no")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| answer(st, false)),
                    )
                    .child(
                        u.action(tr("Yes, send it"), ActionKind::Primary)
                            .key("telemetry-yes")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| answer(st, true)),
                    ),
            ),
    )
}

/// Settings > Privacy > Help improve Sigil.
pub fn privacy_section(s: &State, u: &Ui) -> El {
    let c = s.settings.telemetry.consent;
    let on = |v: bool| c.asked && v;
    section(
        u,
        "Help improve Sigil",
        vec![
            toggle_row(
                u,
                "settings-telemetry-usage",
                "Usage events",
                "Which pages you open and which main actions you use. No mod names or paths.",
                on(c.usage),
                |st| flip(st, |c| c.usage = !c.usage),
            ),
            toggle_row(
                u,
                "settings-telemetry-crashes",
                "Crash reports",
                "When Sigil crashes: the error and where, scrubbed of paths, names, keys and emails.",
                on(c.crashes),
                |st| flip(st, |c| c.crashes = !c.crashes),
            ),
            toggle_row(
                u,
                "settings-telemetry-lookup",
                "Unknown-mod lookup",
                "Look up a mod file's hash to name it, and add the hashes of files you install from GameBanana.",
                on(c.hash_lookup),
                |st| flip(st, |c| c.hash_lookup = !c.hash_lookup),
            ),
            setting_row(
                u,
                "settings-telemetry-doc",
                "What Sigil sends",
                detail(u, "Every field, where it goes and how long it is kept."),
                button(u, "settings-telemetry-doc-open", "Open", ActionKind::Secondary)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| super::tabs::open_url(st, tx, PRIVACY_DOC)),
            ),
        ],
    )
}

/// What Send diagnostics sends: a short summary and the log tail, scrubbed the way the core
/// scrubs them before sending.
fn diagnostics_parts(st: &State) -> (String, String) {
    let summary = [
        format!("Sigil {VERSION}"),
        format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH),
        deadlock_core::diagnostics::machine_line(),
        format!("Game found: {}", st.game_detected),
        format!("Mods: {} installed, {} on", st.addons.addons.len(), st.addons.enabled_count()),
        format!("Offline mode: {}", st.prefs.offline),
        crate::glass::report(),
    ]
    .join("\n");
    (telemetry::scrub(&summary), telemetry::scrub(&super::log::tail(400)))
}

/// Settings > Logs and support > Send diagnostics.
pub fn diagnostics_row(s: &State, u: &Ui) -> El {
    let t = &s.settings.telemetry;
    let line = match &t.code {
        Some(Ok(code)) => fill(tr("Sent. Your reference code is {}."), &[code]),
        Some(Err(error)) => fill(tr("Not sent: {}"), &[error]),
        None => tr("Shows exactly what goes, scrubbed of paths, names, keys and emails, before you send it.").to_string(),
    };
    let mut trailing = row().gap(8.0).center_y();
    if let Some(Ok(code)) = &t.code {
        let code = code.clone();
        trailing = trailing.child(
            button(u, "settings-diagnostics-copy", "Copy code", ActionKind::Secondary)
                .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(code.clone())),
        );
    }
    trailing = trailing.child(
        super::disabled(
            button(u, "settings-diagnostics-open", "Send diagnostics", ActionKind::Secondary).on_press(|st: &mut State| {
                let (summary, log) = diagnostics_parts(st);
                st.settings.telemetry.code = None;
                st.settings.telemetry.preview = Some(format!("{summary}\n\n{log}"));
            }),
            t.sending.is_running() || s.prefs.offline,
        ),
    );
    setting_row(u, "settings-diagnostics", "Send diagnostics", detail(u, &line), trailing)
}

/// The preview before Send.
pub fn diagnostics_dialog(s: &State, u: &Ui) -> Option<El> {
    let preview = s.settings.telemetry.preview.as_ref()?;
    let sending = s.settings.telemetry.sending.is_running();
    let close = |st: &mut State| st.settings.telemetry.preview = None;
    Some(
        u.dialog_shell()
            .key("diagnostics-preview")
            .role("dialog")
            .label(tr("Send diagnostics"))
            .w(620.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .child(text(tr("Send diagnostics")).font(H1).color(u.ink))
            .child(
                text(tr("This is everything that goes to Sigil's server, after the scrub. You get a reference code to give to whoever helps you."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(
                u.scroll_area(col()
                    .key("diagnostics-text")
                    .w_full()
                    .h(280.0)
                    .padding(12.0)
                    .radius(crate::ui::r(8.0))
                    .fill(u.well)
                    .border(1.0, u.hair)
                    .scroll(rune::ScrollSpec { stick_bottom: false })
                    .child(text(preview.clone()).font(MONO).color(u.prose).wrap_anywhere().selectable(true).w_full())),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(spacer())
                    .child(
                        u.action(tr("Cancel"), ActionKind::Ghost)
                            .key("diagnostics-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    )
                    .child(
                        u.action(if sending { tr("Sending") } else { tr("Send") }, ActionKind::Primary)
                            .key("diagnostics-send")
                            .disabled(sending)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(send),
                    ),
            ),
    )
}

fn send(st: &mut State) {
    if st.settings.telemetry.sending.is_running() || st.demo || rune::is_headless() {
        return;
    }
    let (summary, log) = diagnostics_parts(st);
    event(st, "diagnostics:send", Some("page:settings"));
    let wake = st.wake.clone();
    st.settings.telemetry.sending.start(wake, move || {
        telemetry::send_diagnostics(&data(), VERSION, &summary, &log).map_err(|e| e.to_string())
    });
}

/// Unknown-mod lookup: the matches for one slot file, from its hash and size. Runs on a worker;
/// the core checks consent again.
pub fn lookup(path: &std::path::Path) -> Result<Vec<telemetry::HashMatch>, String> {
    let sha = deadlock_core::identify::sha256_file(path).map_err(|e| e.to_string())?;
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    telemetry::identify(&sha, size).map_err(|e| e.to_string())
}

/// After a GameBanana install: add the installed file's hash to the index, on a short-lived
/// worker thread. Needs lookup consent (checked here and in the core) and a GameBanana file id
/// from the download link.
pub fn contribute(st: &State, mod_id: u64, entry: Option<&deadlock_core::install::InstallJournalEntry>) {
    let c = st.settings.telemetry.consent;
    if !(c.asked && c.hash_lookup) || st.demo || rune::is_headless() || cfg!(test) || st.prefs.offline {
        return;
    }
    let Some(entry) = entry else { return };
    let Some(file_id) = entry.download_url.as_deref().and_then(file_id_of) else { return };
    let (sha, size) = (entry.vpk_sha256.clone(), entry.vpk_bytes);
    let _ = std::thread::Builder::new().name("sigil-hash".to_string()).spawn(move || {
        let _ = telemetry::contribute_hash(&data(), &sha, size, mod_id, file_id);
    });
}

/// The GameBanana file id in a download link (`/mmdl/<id>` or `/dl/<id>`).
pub fn file_id_of(url: &str) -> Option<u64> {
    let path = url.split(['?', '#']).next()?;
    let mut parts = path.trim_end_matches('/').rsplit('/');
    let id = parts.next()?.parse::<u64>().ok()?;
    matches!(parts.next(), Some("mmdl") | Some("dl")).then_some(id)
}

/// Scenes: the first-start question, and the diagnostics preview.
pub fn stage_scene(st: &mut State, preview: bool) {
    st.settings.telemetry.loaded = true;
    if preview {
        let (summary, log) = diagnostics_parts(st);
        st.settings.telemetry.preview = Some(format!("{summary}\n\n{log}"));
    } else {
        st.settings.telemetry.asking = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ids_come_from_gamebanana_download_links_only() {
        assert_eq!(file_id_of("https://gamebanana.com/mmdl/1414905"), Some(1_414_905));
        assert_eq!(file_id_of("https://files.gamebanana.com/dl/77/"), Some(77));
        assert_eq!(file_id_of("https://gamebanana.com/mods/600009"), None);
        assert_eq!(file_id_of("https://gamebanana.com/mmdl/x"), None);
    }

    #[test]
    fn every_page_event_is_one_the_worker_knows() {
        use crate::model::Screen;
        for screen in [Screen::Browse, Screen::Installed, Screen::Locker, Screen::Stats, Screen::Settings, Screen::Launch] {
            let name = page_event(screen).unwrap();
            assert!(telemetry::EVENTS.contains(&name), "{name}");
        }
        assert!(page_event(Screen::Agent).is_none());
    }

    #[test]
    fn nothing_is_queued_before_an_answer() {
        let st = State::default();
        assert!(!st.settings.telemetry.consent.asked);
        // A test run never records; the consent gate is the second guard.
        event(&st, "page:browse", None);
    }
}
