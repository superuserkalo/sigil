//! Tests for crash detection across restarts: the marker lifecycle, the records it leaves, the
//! scrub, the ask-or-send decision, the rate limit and the repeat check. File IO uses a per-test
//! folder under the system temp directory; sends go to a loopback server.

use super::*;
use crate::telemetry::tests::Server;

const HOUR: i64 = 60 * 60 * 1000;
const START: i64 = 1_790_000_000_000;

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dah-crash-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn no_boot() -> Option<i64> {
    None
}

fn tail() -> String {
    "2026-11-20 10:00:00 INFO  Sigil 0.1.30 started on windows x86_64\n2026-11-20 10:05:12 INFO  Opened the 3D view".to_string()
}

fn panic_file(dir: &Path, ts: i64, message: &str) -> PathBuf {
    let crash = CrashFile {
        message: message.to_string(),
        backtrace: "   0: sigil::viewer::render\n             at ./src/viewer.rs:12:5".to_string(),
        version: "0.1.30".to_string(),
        os: "windows".to_string(),
        ts,
        kind: KIND_PANIC.to_string(),
        thread: "main".to_string(),
        context: String::new(),
    };
    fs::create_dir_all(crash_dir(dir)).unwrap();
    let path = crash_dir(dir).join(format!("{}.json", file_stamp(ts)));
    fs::write(&path, serde_json::to_vec(&crash).unwrap()).unwrap();
    path
}

fn read(path: &Path) -> CrashFile {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// The marker
// ---------------------------------------------------------------------------

#[test]
fn a_clean_exit_clears_the_marker_and_the_next_start_finds_nothing() {
    let dir = root("clean");
    assert!(begin_session(&dir, "0.1.30", START, no_boot, tail).is_none());
    let marker = read_session(&dir).expect("the marker is written at start");
    assert_eq!(marker.version, "0.1.30");
    assert_eq!(marker.started, START);
    end_session(&dir);
    assert!(read_session(&dir).is_none());
    // Twice is harmless.
    end_session(&dir);
    assert!(begin_session(&dir, "0.1.30", START + HOUR, no_boot, tail).is_none());
    assert!(crash_files(&crash_dir(&dir)).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_crash_leaves_the_marker_and_the_next_start_records_an_unclean_exit() {
    let dir = root("unclean");
    begin_session(&dir, "0.1.29", START, no_boot, tail);
    // No end_session: the process died.
    let last = begin_session(&dir, "0.1.30", START + HOUR, no_boot, tail).expect("an unclean exit");
    assert_eq!(last.kind, KIND_UNCLEAN);
    assert_eq!(last.version, "0.1.29", "the version that crashed");
    assert_eq!(last.reports.len(), 1);
    let crash = read(&last.reports[0]);
    assert_eq!(crash.kind, KIND_UNCLEAN);
    assert!(crash.message.contains("did not exit cleanly"));
    assert!(crash.backtrace.contains("last log line: INFO  Opened the 3D view"), "{}", crash.backtrace);
    assert!(crash.context.contains("Opened the 3D view"));
    // The new run's marker replaced the old one; a clean exit now clears it.
    assert_eq!(read_session(&dir).unwrap().started, START + HOUR);
    end_session(&dir);
    assert!(begin_session(&dir, "0.1.30", START + 2 * HOUR, no_boot, tail).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_panic_of_the_run_is_offered_with_its_log_tail() {
    let dir = root("panic-run");
    let old = panic_file(&dir, START - HOUR, "an older panic from a run that ended cleanly");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    let path = panic_file(&dir, START + 10_000, "index out of bounds at src/viewer.rs:12:5");
    let last = begin_session(&dir, "0.1.30", START + HOUR, no_boot, tail).expect("a panic");
    assert_eq!(last.kind, KIND_PANIC);
    assert_eq!(last.reports, vec![path.clone()]);
    assert!(last.headline.starts_with("index out of bounds"));
    let crash = read(&path);
    assert!(crash.context.starts_with("thread: main\n"), "{}", crash.context);
    assert!(crash.context.contains("last log lines:"));
    // A panic outside the run is not part of this offer.
    assert!(old.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_shutdown_under_a_running_sigil_is_not_a_crash() {
    let dir = root("shutdown");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    // The system booted after the run started and nothing was recorded.
    assert!(begin_session(&dir, "0.1.30", START + 10 * HOUR, || Some(START + 9 * HOUR), tail).is_none());
    // Booted before the run started: Sigil died on its own.
    begin_session(&dir, "0.1.30", START + 11 * HOUR, no_boot, tail);
    let last = begin_session(&dir, "0.1.30", START + 12 * HOUR, || Some(START), tail);
    assert_eq!(last.map(|last| last.kind).as_deref(), Some(KIND_UNCLEAN));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_panic_is_a_crash_even_after_a_reboot() {
    let dir = root("panic-reboot");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    panic_file(&dir, START + 1000, "boom");
    let last = begin_session(&dir, "0.1.30", START + HOUR, || Some(START + 2000), tail);
    assert_eq!(last.map(|last| last.kind).as_deref(), Some(KIND_PANIC));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_windows_exception_record_becomes_a_native_report() {
    let dir = root("native");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    fs::write(native_path(&dir), "code=0xC0000005 address=Sigil.exe+0x1a2b3c thread=4242\n").unwrap();
    let last = begin_session(&dir, "0.1.30", START + HOUR, no_boot, tail).expect("native");
    assert_eq!(last.kind, KIND_NATIVE);
    assert!(last.headline.contains("0xC0000005 (access violation)"), "{}", last.headline);
    let crash = read(&last.reports[0]);
    assert!(crash.backtrace.contains("Sigil.exe+0x1a2b3c"));
    assert!(!native_path(&dir).exists(), "the raw record is folded in once");
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Scrubbing and the body
// ---------------------------------------------------------------------------

#[test]
fn the_record_and_the_preview_are_scrubbed() {
    let dir = root("scrub");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    let dirty = || {
        [
            "2026-11-20 10:00:00 INFO  Library at C:\\Users\\Alice Smith\\Deadlock",
            "2026-11-20 10:00:01 INFO  Key sk-ant-REALKEY1234567890abcdef in a message",
            "2026-11-20 10:00:02 INFO  mail alice@example.com, Steam 76561198000000001",
            "2026-11-20 10:00:03 ERROR stopped in /Users/bob/Library/Application Support/x",
        ]
        .join("\n")
    };
    let last = begin_session(&dir, "0.1.30", START + HOUR, no_boot, dirty).expect("unclean");
    let crash = read(&last.reports[0]);
    let preview = preview(&dir, "0.1.30", &last.reports);
    for text in [&crash.context, &crash.backtrace, &preview] {
        for secret in ["Alice", "REALKEY", "alice@example.com", "76561198000000001", "bob"] {
            assert!(!text.contains(secret), "{secret} leaked into {text}");
        }
    }
    assert!(preview.contains("kind: unclean_exit"));
    assert!(preview.contains("version: 0.1.30"));
    assert!(preview.contains("context:"));
    assert!(preview.contains("<user>"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_large_report_fits_the_body_budget() {
    let crash = CrashFile {
        message: "m".repeat(20_000),
        backtrace: "   0: frame\n".repeat(10_000),
        version: "0.1.30".to_string(),
        os: "windows".to_string(),
        ts: START,
        kind: KIND_PANIC.to_string(),
        thread: "main".to_string(),
        context: "INFO line with \"quotes\"\n".repeat(5_000),
    };
    let value = body("3f2504e0-4f89-41d3-9a0c-0305e82c3301", "0.1.30", &crash);
    let size = serde_json::to_vec(&value).unwrap().len();
    assert!(size <= BODY_BUDGET, "{size}");
    assert_eq!(value["kind"], "panic");
    assert!(value["message"].as_str().unwrap().len() <= MAX_CRASH_MESSAGE + 20);
}

#[test]
fn the_fingerprint_ignores_addresses_and_line_numbers() {
    let a = fingerprint("boom\nmore", "   0: 0x7ff6 sigil::run\n at src/app.rs:10:4");
    let b = fingerprint("boom", "0: 0x1234 sigil::run\n at src/app.rs:99:1");
    assert_eq!(a, b);
    assert_ne!(a, fingerprint("other", "0: 0x1234 sigil::run"));
    assert_eq!(a.len(), 64);
}

// ---------------------------------------------------------------------------
// Ask, send or drop
// ---------------------------------------------------------------------------

#[test]
fn the_decision_follows_the_crash_switch() {
    let ledger = Ledger::default();
    let opted_in = Consent { asked: true, usage: false, crashes: true, hash_lookup: false, ..Consent::default() };
    let opted_out = Consent { asked: true, usage: true, crashes: false, hash_lookup: true, ..Consent::default() };
    let all_off = Consent { asked: true, ..Consent::default() };
    let never_asked = Consent::default();
    // A switch without the shown prompt does not count as consent.
    let not_asked_but_on = Consent { asked: false, crashes: true, ..Consent::default() };
    assert_eq!(decide(opted_in, &ledger, "fp", START), Offer::SendNow);
    assert_eq!(decide(opted_out, &ledger, "fp", START), Offer::Ask);
    assert_eq!(decide(all_off, &ledger, "fp", START), Offer::Ask);
    assert_eq!(decide(never_asked, &ledger, "fp", START), Offer::Ask);
    assert_eq!(decide(not_asked_but_on, &ledger, "fp", START), Offer::Ask);
}

#[test]
fn a_declined_crash_is_not_asked_about_again_for_a_day() {
    let dir = root("decline");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    let last = begin_session(&dir, "0.1.30", START + HOUR, no_boot, tail).unwrap();
    decline(&dir, &last.reports, &last.fingerprint);
    assert!(!last.reports[0].exists(), "Don't send deletes the report");
    let ledger = Ledger::read(&dir);
    let now = ledger.declined[0].ts;
    assert_eq!(decide(Consent::default(), &ledger, &last.fingerprint, now + HOUR), Offer::Drop);
    assert_eq!(decide(Consent::default(), &ledger, "another crash", now + HOUR), Offer::Ask);
    assert_eq!(decide(Consent::default(), &ledger, &last.fingerprint, now + 25 * HOUR), Offer::Ask);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_pressed_send_goes_once_and_repeats_and_floods_are_held_back() {
    let dir = root("send");
    let server = Server::start("{\"repeats\":1}");
    begin_session(&dir, "0.1.30", START, no_boot, tail);
    let last = begin_session(&dir, "0.1.30", START + HOUR, no_boot, tail).unwrap();
    let now = START + HOUR;
    assert_eq!(send_reports_at(&dir, "0.1.30", &server.base, &last.reports, now).unwrap(), 1);
    assert!(!last.reports[0].exists());
    let bodies = server.bodies();
    let value: Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(value["kind"], "unclean_exit");
    assert!(value["context"].as_str().unwrap().contains("last log lines:"));

    // The same crash again within a day: deleted, not sent.
    let again = crash_dir(&dir).join("again.json");
    fs::write(&again, serde_json::to_vec(&CrashFile {
        message: "Sigil did not exit cleanly (no panic was recorded)".to_string(),
        backtrace: "no panic was recorded\nlast log line: INFO  Opened the 3D view".to_string(),
        version: "0.1.30".to_string(),
        ts: now + 1,
        ..CrashFile::default()
    }).unwrap()).unwrap();
    assert_eq!(send_reports_at(&dir, "0.1.30", &server.base, std::slice::from_ref(&again), now + 2).unwrap(), 0);
    assert!(!again.exists());
    assert_eq!(server.hits(), 1);

    // Distinct crashes: PER_HOUR go, the next one waits.
    let mut paths = Vec::new();
    for index in 0..PER_HOUR {
        paths.push(panic_file(&dir, now + 10 + index as i64, &format!("crash number {index}")));
    }
    let sent = send_reports_at(&dir, "0.1.30", &server.base, &paths, now + 20).unwrap();
    assert_eq!(sent, PER_HOUR - 1, "one of the hour's reports already went");
    assert!(paths.last().unwrap().exists(), "a limited report stays for later");
    let limited = send_reports_at(&dir, "0.1.30", &server.base, &[paths.last().unwrap().clone()], now + 30);
    assert!(limited.is_err());
    // An hour later it goes.
    assert_eq!(send_reports_at(&dir, "0.1.30", &server.base, &[paths.last().unwrap().clone()], now + 2 * HOUR).unwrap(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_ledger_caps_a_day() {
    let mut ledger = Ledger::default();
    for index in 0..PER_DAY {
        ledger.sent.push(Entry { fingerprint: format!("{index}"), ts: START + index as i64 * 2 * HOUR });
    }
    assert!(ledger.limited(START + 20 * HOUR));
    assert!(!ledger.limited(START + 50 * HOUR));
}

#[test]
fn a_marker_from_before_the_quit_fix_is_not_an_unclean_exit() {
    let dir = root("legacy-marker");
    // 0.1.28 and earlier left the marker on every normal quit.
    begin_session(&dir, "0.1.28", START, no_boot, tail);
    assert!(begin_session(&dir, "0.1.29", START + HOUR, no_boot, tail).is_none());
    // A panic recorded by such a run is still a crash.
    let dir = root("legacy-marker-panic");
    begin_session(&dir, "0.1.20", START, no_boot, tail);
    panic_file(&dir, START + 5, "boom");
    assert!(begin_session(&dir, "0.1.29", START + HOUR, no_boot, tail).is_some());
}

#[test]
fn only_versions_with_the_quit_fix_have_a_reliable_marker() {
    assert!(!super::marker_is_reliable("0.1.28"));
    assert!(!super::marker_is_reliable("v0.1.9"));
    assert!(super::marker_is_reliable("0.1.29"));
    assert!(super::marker_is_reliable("0.2.0"));
    assert!(super::marker_is_reliable("1.0.0-beta"));
    assert!(super::marker_is_reliable(""));
}
