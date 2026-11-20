//! Crash detection across restarts.
//!
//! The app writes a session marker (`<data>/telemetry/session.json`) when it starts and removes
//! it on every clean exit (window close, tray Quit, Restart to update). A marker found at the next
//! start means the last run did not exit cleanly. [`begin_session`] then turns what that run left
//! behind into one crash file in `<data>/telemetry/crashes/`:
//!
//! - a panic the hook wrote during that run (`kind: panic`), with the run's log tail added;
//! - a Windows exception filter record (`kind: native`), when the process died on an SEH
//!   exception the panic hook never saw;
//! - otherwise `kind: unclean_exit`: no message beyond "did not exit cleanly", and the log tail.
//!   That covers an abort, a stack overflow, a driver crash and a forced quit.
//!
//! A run that ended because the computer shut down is not a crash: when no panic or native
//! record exists and the system booted after the run started, the marker is dropped quietly.
//!
//! [`decide`] says what the app does with the file: send it at once (crash reports are on), ask
//! the player (off, or telemetry never answered: one press is one-off consent, ADR 0014), or
//! nothing (the same crash was declined in the last day). [`send_reports`] is the player-pressed
//! send; [`send_file_at`] is shared with the opted-in flush. Both pass the [`Ledger`]: a crash
//! already sent in the last day is not sent again, and at most [`PER_HOUR`] reports an hour and
//! [`PER_DAY`] a day leave this install. Everything sent is scrubbed again on the way out.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    Consent, CrashFile, EVENT_TIMEOUT, MAX_CRASH_BACKTRACE, MAX_CRASH_MESSAGE, app_version,
    crash_dir, crash_files, endpoint_at, install_id, os_name, post, prune_crashes, scrub,
    telemetry_dir, truncate,
};
use crate::error::Result;
use crate::util::write_atomic;

pub const KIND_PANIC: &str = "panic";
pub const KIND_UNCLEAN: &str = "unclean_exit";
pub const KIND_NATIVE: &str = "native";

const SESSION_FILE: &str = "session.json";
const LEDGER_FILE: &str = "crash-ledger.json";
/// The Windows exception filter writes this raw line; [`begin_session`] folds it into a report.
pub const NATIVE_FILE: &str = "native-crash.txt";

/// Reports that may leave this install in one hour and in one day, whatever their kind.
pub const PER_HOUR: usize = 3;
pub const PER_DAY: usize = 10;
/// The same crash is sent (or offered after a No) at most once in this window.
pub const REPEAT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// Ledger entries older than this are dropped.
const LEDGER_KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// The log tail kept with a report.
const MAX_CONTEXT: usize = 12 * 1024;
/// Older Workers cap a crash body at 64 KiB; stay under it so the send works against either.
const BODY_BUDGET: usize = 60 * 1024;
/// A run's panics offered together, newest first. Older ones from the same run are dropped.
const MAX_SESSION_PANICS: usize = 3;

/// The marker a running Sigil keeps on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    pub pid: u32,
    pub version: String,
    /// Milliseconds since the epoch.
    pub started: i64,
}

/// What the last run left behind, for the dialog or the quiet send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastRun {
    /// The crash files to offer, newest first. The first one carries the log tail.
    pub reports: Vec<PathBuf>,
    pub kind: String,
    /// One scrubbed line: the panic message's first line, or what an unclean exit means.
    pub headline: String,
    pub fingerprint: String,
    /// The version that crashed.
    pub version: String,
}

fn session_path(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(SESSION_FILE)
}

fn ledger_path(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(LEDGER_FILE)
}

pub fn native_path(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(NATIVE_FILE)
}

pub fn read_session(data_dir: &Path) -> Option<Session> {
    let text = fs::read_to_string(session_path(data_dir)).ok()?;
    // A marker that is there but unreadable still means the run did not end cleanly.
    Some(serde_json::from_str(&text).unwrap_or_default())
}

/// Start a session: read what the last one left, then write this one's marker.
///
/// `boot_ms` is when the system last booted (asked only when needed), `log_tail` the end of the
/// app log as it is before this run writes to it (also asked only when needed). Returns the
/// report to offer, or None when the last run exited cleanly (or the machine shut down under it).
pub fn begin_session(
    data_dir: &Path,
    version: &str,
    now_ms: i64,
    boot_ms: impl FnOnce() -> Option<i64>,
    log_tail: impl FnOnce() -> String,
) -> Option<LastRun> {
    let previous = read_session(data_dir);
    let marker = Session {
        pid: std::process::id(),
        version: app_version(version),
        started: now_ms,
    };
    if let Ok(text) = serde_json::to_vec_pretty(&marker) {
        let _ = write_atomic(&session_path(data_dir), &text);
    }
    let previous = previous?;
    record_last_run(data_dir, &previous, now_ms, boot_ms, log_tail)
}

/// A clean exit: remove the marker. Safe to call twice.
pub fn end_session(data_dir: &Path) {
    let _ = fs::remove_file(session_path(data_dir));
}

/// The crash files of a run that did not exit cleanly, or a new one for an unclean exit.
fn record_last_run(
    data_dir: &Path,
    previous: &Session,
    now_ms: i64,
    boot_ms: impl FnOnce() -> Option<i64>,
    log_tail: impl FnOnce() -> String,
) -> Option<LastRun> {
    let dir = crash_dir(data_dir);
    let mut panics: Vec<(PathBuf, CrashFile)> = crash_files(&dir)
        .into_iter()
        .filter_map(|path| {
            let crash: CrashFile = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            let of_run = crash.ts >= previous.started && crash.ts <= now_ms;
            let panic = crash.kind.is_empty() || crash.kind == KIND_PANIC;
            (of_run && panic && crash.context.is_empty()).then_some((path, crash))
        })
        .collect();
    panics.sort_by_key(|(_, crash)| std::cmp::Reverse(crash.ts));
    for (path, _) in panics.iter().skip(MAX_SESSION_PANICS) {
        let _ = fs::remove_file(path);
    }
    panics.truncate(MAX_SESSION_PANICS);

    let native = fs::read_to_string(native_path(data_dir)).ok();
    let _ = fs::remove_file(native_path(data_dir));
    let native = native.map(|line| line.trim().to_string()).filter(|line| !line.is_empty());

    if panics.is_empty() && native.is_none() {
        // Nothing recorded and the machine started after the run did: a shutdown or restart
        // under a running Sigil (a tray app often is), not a crash.
        if boot_ms().is_some_and(|boot| boot > previous.started) {
            return None;
        }
    }

    let version = if previous.version.is_empty() {
        app_version("")
    } else {
        app_version(&previous.version)
    };
    let tail = log_tail();
    let context = |thread: &str| {
        let mut text = String::new();
        if !thread.is_empty() {
            text.push_str(&format!("thread: {thread}\n"));
        }
        text.push_str(&format!(
            "session: started {}, found at the next start {}\n",
            stamp(previous.started),
            stamp(now_ms)
        ));
        if !tail.trim().is_empty() {
            text.push_str("\nlast log lines:\n");
            text.push_str(tail.trim_end());
        }
        tail_of(&scrub(&text), MAX_CONTEXT)
    };

    if let Some((path, mut crash)) = panics.first().cloned() {
        crash.context = context(&crash.thread);
        if crash.kind.is_empty() {
            crash.kind = KIND_PANIC.to_string();
        }
        if let Ok(text) = serde_json::to_vec_pretty(&crash) {
            let _ = fs::write(&path, text);
        }
        return Some(LastRun {
            reports: panics.into_iter().map(|(path, _)| path).collect(),
            kind: KIND_PANIC.to_string(),
            headline: headline(&crash.message),
            fingerprint: fingerprint(&crash.message, &crash.backtrace),
            version: crash.version,
        });
    }

    let (kind, message, backtrace) = match native {
        Some(line) => (
            KIND_NATIVE,
            native_message(&line),
            format!("no Rust backtrace: the process ended in the Windows exception filter\n{line}"),
        ),
        None => (
            KIND_UNCLEAN,
            "Sigil did not exit cleanly (no panic was recorded)".to_string(),
            format!(
                "no panic was recorded\nlast log line: {}",
                last_log_line(&tail)
            ),
        ),
    };
    let crash = CrashFile {
        message: truncate(&scrub(&message), MAX_CRASH_MESSAGE),
        backtrace: truncate(&scrub(&backtrace), MAX_CRASH_BACKTRACE),
        version: version.clone(),
        os: os_name().to_string(),
        ts: now_ms,
        kind: kind.to_string(),
        thread: String::new(),
        context: context(""),
    };
    let text = serde_json::to_vec_pretty(&crash).ok()?;
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}-{kind}.json", file_stamp(now_ms)));
    fs::write(&path, text).ok()?;
    prune_crashes(&dir);
    Some(LastRun {
        reports: vec![path],
        kind: kind.to_string(),
        headline: headline(&crash.message),
        fingerprint: fingerprint(&crash.message, &crash.backtrace),
        version,
    })
}

/// What the app does with a [`LastRun`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Crash reports are on: send it now and say so in a toast.
    SendNow,
    /// Ask with the dialog. Telemetry off, or never answered, still asks: the press is the
    /// consent for this one report.
    Ask,
    /// The player declined this same crash in the last day: drop it without asking again.
    Drop,
}

pub fn decide(consent: Consent, ledger: &Ledger, fingerprint: &str, now_ms: i64) -> Offer {
    if consent.crashes_on() {
        return Offer::SendNow;
    }
    if ledger.declined_recently(fingerprint, now_ms) {
        return Offer::Drop;
    }
    Offer::Ask
}

/// Sent and declined crashes, by fingerprint, for the rate limit and the repeat check.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Ledger {
    pub sent: Vec<Entry>,
    pub declined: Vec<Entry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Entry {
    pub fingerprint: String,
    pub ts: i64,
}

impl Ledger {
    pub fn read(data_dir: &Path) -> Ledger {
        fs::read_to_string(ledger_path(data_dir))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&mut self, data_dir: &Path, now_ms: i64) {
        self.sent.retain(|entry| now_ms - entry.ts < LEDGER_KEEP_MS);
        self.declined.retain(|entry| now_ms - entry.ts < LEDGER_KEEP_MS);
        if let Ok(text) = serde_json::to_vec_pretty(self) {
            let _ = write_atomic(&ledger_path(data_dir), &text);
        }
    }

    fn within(list: &[Entry], now_ms: i64, window: i64) -> impl Iterator<Item = &Entry> {
        list.iter().filter(move |entry| entry.ts <= now_ms && now_ms - entry.ts < window)
    }

    pub fn sent_recently(&self, fingerprint: &str, now_ms: i64) -> bool {
        Self::within(&self.sent, now_ms, REPEAT_WINDOW_MS).any(|entry| entry.fingerprint == fingerprint)
    }

    pub fn declined_recently(&self, fingerprint: &str, now_ms: i64) -> bool {
        Self::within(&self.declined, now_ms, REPEAT_WINDOW_MS).any(|entry| entry.fingerprint == fingerprint)
    }

    /// The hourly or daily cap is reached.
    pub fn limited(&self, now_ms: i64) -> bool {
        Self::within(&self.sent, now_ms, 60 * 60 * 1000).count() >= PER_HOUR
            || Self::within(&self.sent, now_ms, REPEAT_WINDOW_MS).count() >= PER_DAY
    }
}

/// The outcome of one file's send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sent {
    Yes,
    /// The same crash went in the last day; the file is deleted unsent.
    Duplicate,
    /// The hourly or daily cap: the file stays for later.
    Limited,
    /// Not a crash file; deleted.
    Unreadable,
    Failed(String),
}

/// Send one crash file and record it, after the repeat check and the rate limit. No consent
/// check: the caller holds it (the opted-in flush, or the player's press).
pub fn send_file_at(data_dir: &Path, version: &str, base: &str, path: &Path, now_ms: i64) -> Sent {
    let Some(crash) = read_crash(path) else {
        let _ = fs::remove_file(path);
        return Sent::Unreadable;
    };
    let print = fingerprint(&crash.message, &crash.backtrace);
    let mut ledger = Ledger::read(data_dir);
    if ledger.sent_recently(&print, now_ms) {
        let _ = fs::remove_file(path);
        return Sent::Duplicate;
    }
    if ledger.limited(now_ms) {
        return Sent::Limited;
    }
    let body = body(&install_id(data_dir), version, &crash);
    match post(&endpoint_at(base, "/v1/crash"), version, &body, EVENT_TIMEOUT) {
        Ok(_) => {
            let _ = fs::remove_file(path);
            ledger.sent.push(Entry { fingerprint: print, ts: now_ms });
            ledger.write(data_dir, now_ms);
            Sent::Yes
        }
        Err(error) => Sent::Failed(error.to_string()),
    }
}

/// The player pressed Send report: send these files now, whatever the switches say (one-off
/// consent, ADR 0014). Returns how many went; an error only when none did and one failed.
pub fn send_reports(data_dir: &Path, version: &str, reports: &[PathBuf]) -> Result<usize> {
    send_reports_at(data_dir, &app_version(version), super::ENDPOINT, reports, now())
}

pub(super) fn send_reports_at(
    data_dir: &Path,
    version: &str,
    base: &str,
    reports: &[PathBuf],
    now_ms: i64,
) -> Result<usize> {
    let mut sent = 0;
    let mut failure = None;
    for path in reports {
        match send_file_at(data_dir, version, base, path, now_ms) {
            Sent::Yes => sent += 1,
            Sent::Failed(error) => {
                failure = Some(error);
                break;
            }
            Sent::Limited => {
                failure.get_or_insert_with(|| "the crash report limit for today is reached".to_string());
                break;
            }
            Sent::Duplicate | Sent::Unreadable => {}
        }
    }
    match failure {
        Some(error) if sent == 0 => Err(crate::error::Error::Network(error)),
        _ => Ok(sent),
    }
}

/// The player pressed Don't send: delete the files and remember the crash, so the same one is
/// not offered again for a day.
pub fn decline(data_dir: &Path, reports: &[PathBuf], fingerprint: &str) {
    for path in reports {
        let _ = fs::remove_file(path);
    }
    let now_ms = now();
    let mut ledger = Ledger::read(data_dir);
    ledger.declined.push(Entry {
        fingerprint: fingerprint.to_string(),
        ts: now_ms,
    });
    ledger.write(data_dir, now_ms);
}

/// Exactly what [`send_reports`] would send for these files, as readable text: every field of
/// every request body, after the scrub.
pub fn preview(data_dir: &Path, version: &str, reports: &[PathBuf]) -> String {
    let install = install_id(data_dir);
    let version = app_version(version);
    let mut parts = Vec::new();
    for (index, path) in reports.iter().enumerate() {
        let Some(crash) = read_crash(path) else {
            continue;
        };
        let body = body(&install, &version, &crash);
        let field = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut text = String::new();
        if reports.len() > 1 {
            text.push_str(&format!("Report {} of {}\n", index + 1, reports.len()));
        }
        text.push_str(&format!(
            "installId: {} (checked by the server, never stored)\nversion: {}\nos: {}\nkind: {}\n\nmessage:\n{}\n",
            field("installId"),
            field("version"),
            field("os"),
            field("kind"),
            field("message")
        ));
        let backtrace = field("backtrace");
        if !backtrace.trim().is_empty() {
            text.push_str(&format!("\nbacktrace:\n{backtrace}\n"));
        }
        let context = field("context");
        if !context.trim().is_empty() {
            text.push_str(&format!("\ncontext:\n{context}\n"));
        }
        parts.push(text);
    }
    parts.join("\n----------------\n\n")
}

/// The `/v1/crash` body for one file, scrubbed again and fitted under [`BODY_BUDGET`]: the log
/// tail shrinks first (keeping its newest lines), then the backtrace (keeping its first frames).
fn body(install: &str, version: &str, crash: &CrashFile) -> Value {
    let reported = if crash.version.trim().is_empty() {
        version.to_string()
    } else {
        app_version(&crash.version)
    };
    let kind = if crash.kind.is_empty() { KIND_PANIC } else { crash.kind.as_str() };
    let message = truncate(&scrub(&crash.message), MAX_CRASH_MESSAGE);
    let mut backtrace = truncate(&scrub(&crash.backtrace), MAX_CRASH_BACKTRACE);
    let mut context = tail_of(&scrub(&crash.context), MAX_CONTEXT);
    loop {
        let mut value = json!({
            "installId": install,
            "version": reported,
            "os": os_name(),
            "kind": kind,
            "message": message,
            "backtrace": backtrace,
        });
        if !context.is_empty() {
            value["context"] = Value::String(context.clone());
        }
        let size = serde_json::to_vec(&value).map(|bytes| bytes.len()).unwrap_or(usize::MAX);
        if size <= BODY_BUDGET {
            return value;
        }
        if !context.is_empty() {
            context = tail_of(&context, context.len() / 2);
            if context.len() < 256 {
                context.clear();
            }
        } else if backtrace.len() > 1024 {
            backtrace = truncate(&backtrace, backtrace.len() / 2);
        } else {
            return value;
        }
    }
}

fn read_crash(path: &Path) -> Option<CrashFile> {
    let crash: CrashFile = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    (!crash.message.trim().is_empty()).then_some(crash)
}

/// The Worker's fingerprint (`services/telemetry/src/fingerprint.ts`): the first message line and
/// the first five backtrace lines, without addresses, line numbers and columns.
pub fn fingerprint(message: &str, backtrace: &str) -> String {
    static RULES: std::sync::OnceLock<(Regex, Regex, Regex)> = std::sync::OnceLock::new();
    let (address, place, space) = RULES.get_or_init(|| {
        (
            Regex::new(r"0x[0-9a-fA-F]+").expect("address"),
            Regex::new(r":\d+:\d+").expect("place"),
            Regex::new(r"\s+").expect("space"),
        )
    });
    let first: String = message.lines().next().unwrap_or_default().trim().chars().take(200).collect();
    let mut lines = vec![first];
    for line in backtrace.lines().map(str::trim).filter(|line| !line.is_empty()).take(5) {
        let line = address.replace_all(line, "0x");
        let line = place.replace_all(&line, "");
        lines.push(space.replace_all(&line, " ").into_owned());
    }
    let digest = Sha256::digest(lines.join("\n").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The panic message's first line, short enough for the dialog.
fn headline(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= 160 {
        return line.to_string();
    }
    let cut: String = line.chars().take(160).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// The last log line without its time stamp, so two runs that died at the same place match.
fn last_log_line(tail: &str) -> String {
    static STAMP: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let stamp = STAMP.get_or_init(|| Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\s+").expect("stamp"));
    let line = tail.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or("(the log was empty)");
    let line = stamp.replace(line, "");
    line.chars().take(300).collect()
}

/// A readable line for the exception filter's record: `code=0xC0000005 ...`.
fn native_message(line: &str) -> String {
    let code = line
        .split_whitespace()
        .find_map(|part| part.strip_prefix("code="))
        .unwrap_or("unknown");
    let name = match code.to_ascii_uppercase().as_str() {
        "0XC0000005" => "access violation",
        "0XC00000FD" => "stack overflow",
        "0XC0000409" => "stack buffer overrun",
        "0XC000001D" => "illegal instruction",
        "0XC0000094" => "integer divide by zero",
        "0XC0000374" => "heap corruption",
        "0X80000003" => "breakpoint",
        _ => "unhandled exception",
    };
    format!("Sigil stopped on a Windows exception {code} ({name})")
}

fn tail_of(text: &str, max: usize) -> String {
    super::tail(text, max)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn file_stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.format("%Y-%m-%dT%H-%M-%S%.9fZ").to_string())
        .unwrap_or_else(|| "0000".to_string())
}

#[cfg(test)]
#[path = "telemetry_crash_tests.rs"]
mod tests;
