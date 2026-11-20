//! Opt-in telemetry: usage events, scrubbed crash reports, player-sent diagnostics and
//! unknown-mod hash lookups, sent only to Sigil's own Cloudflare Worker.
//!
//! Nothing here sends anything until the player agrees. The consent file
//! (`<data>/telemetry/consent.json`) is the only switch, and every sending function reads it
//! itself so a caller that forgets to check cannot leak data. `asked` must be true as well as
//! the per-kind switch, so a half-written or edited file cannot turn collection on.
//!
//! Four kinds of data, each with its own switch:
//!
//! - Usage events ([`record`], [`flush`]): an allowlisted name, an optional allowlisted page, a
//!   timestamp, the app version and the OS. Queued in `<data>/telemetry/queue.json`, at most
//!   [`QUEUE_LIMIT`] events, oldest dropped, and sent in batches of [`BATCH_SIZE`].
//! - Crash reports ([`install_panic_hook`], [`send_pending_crashes`]): the panic message and a
//!   backtrace, scrubbed and written to `<data>/telemetry/crashes/<time>.json` locally even when
//!   crash consent is off. Sending is a separate call so a panic hook never blocks on a socket.
//!   [`crash`] adds the session marker: a run that did not exit cleanly is found at the next
//!   start, its log tail is added, and the app asks before sending (or sends, with consent).
//! - Diagnostics ([`send_diagnostics`]): the player presses Send, so that press is the consent;
//!   the log tail and summary are scrubbed here and again by the Worker, and the Worker answers
//!   with a reference code such as `SGL-AB12-CD34`.
//! - Unknown-mod lookups ([`identify`], [`contribute_hash`]): a sha256 and a size, behind the
//!   hash lookup switch.
//!
//! Every request goes through [`crate::net::builder`], carries `x-sigil-client: 1` and
//! `x-sigil-version`, uses a short timeout and fails quietly (one stderr line, no UI work). The
//! caller runs these from a worker thread; nothing here touches the game folder.
//!
//! [`identify`] takes no directory and no version, so the app calls [`configure`] once at
//! startup with the app data directory and version. Without it, a lookup answers "no match"
//! instead of sending. The other entry points take their directory explicitly.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::util::write_atomic;

/// The Worker root. The owner swaps in the deployed URL after `wrangler deploy`; `.invalid` is
/// reserved by RFC 2606, so a build that forgets sends nothing anywhere real.
pub const ENDPOINT: &str = "https://sigil-telemetry.gantchevkaloyan.workers.dev";

/// The event names the Worker stores. Unknown names are dropped on both sides: the app does not
/// queue them, and the Worker drops them again. `services/telemetry/src/events.ts` must keep the
/// same list; it does not exist yet, so this is the definition to copy.
pub const EVENTS: &[&str] = &[
    "app:start",
    "page:browse",
    "page:installed",
    "page:profiles",
    "page:downloads",
    "page:locker",
    "page:crosshair",
    "page:autoexec",
    "page:performance",
    "page:servers",
    "page:stats",
    "page:character",
    "page:cursors",
    "page:divine",
    "page:settings",
    "page:about",
    "browse:search",
    "browse:filter",
    "browse:open",
    "browse:download",
    "install:start",
    "install:done",
    "install:failed",
    "install:cancel",
    "identify:lookup",
    "identify:contribute",
    "profile:create",
    "profile:apply",
    "profile:share",
    "mods:toggle",
    "mods:reorder",
    "deploy:start",
    "deploy:done",
    "repair:apply",
    "launch:modded",
    "launch:vanilla",
    "launch:restore",
    "update:check",
    "update:install",
    "server:join",
    "crosshair:apply",
    "autoexec:apply",
    "performance:apply",
    "backup:restore",
    "cleanup:run",
    "diagnostics:send",
    "crash:send",
    "settings:change",
    "agent:chat",
    "publish:start",
];

/// Queued events kept on disk. Older ones fall off the front.
const QUEUE_LIMIT: usize = 500;
/// Events per `/v1/events` request. The Worker rejects a larger batch.
const BATCH_SIZE: usize = 50;
/// Crash files kept on disk, newest first. A panic loop must not fill the disk.
const CRASH_LIMIT: usize = 50;
/// The Worker caps a crash at 64 KB; leave room for the JSON envelope.
const MAX_CRASH_BACKTRACE: usize = 48 * 1024;
const MAX_CRASH_MESSAGE: usize = 8 * 1024;
/// The Worker caps diagnostics at 1 MB, so the log tail keeps its last 900 KB and the summary
/// and the JSON envelope fit under the cap.
const MAX_DIAGNOSTICS_LOG: usize = 900 * 1024;
const MAX_DIAGNOSTICS_SUMMARY: usize = 8 * 1024;

/// Events are tiny; a batch that has not gone in eight seconds is not going to.
const EVENT_TIMEOUT: Duration = Duration::from_secs(8);
/// Diagnostics carry up to a megabyte, so they get a little longer.
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(20);

const DIR_NAME: &str = "telemetry";
const CONSENT_FILE: &str = "consent.json";
const INSTALL_ID_FILE: &str = "install-id";
const QUEUE_FILE: &str = "queue.json";
const CRASH_DIR: &str = "crashes";
const CLIENT_HEADER: &str = "1";

/// The player's answer to the first-start prompt and the three switches in Settings > Privacy.
///
/// `asked` records that the dialog was shown. The switches only take effect together with it,
/// so a settings file that says `usage: true` without `asked` still sends nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Consent {
    pub asked: bool,
    pub usage: bool,
    pub crashes: bool,
    pub hash_lookup: bool,
}

impl Consent {
    fn usage_on(&self) -> bool {
        self.asked && self.usage
    }

    fn crashes_on(&self) -> bool {
        self.asked && self.crashes
    }

    fn hash_lookup_on(&self) -> bool {
        self.asked && self.hash_lookup
    }
}

/// One mod the hash index knows, as `GET /v1/identify` answers it. Best match first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HashMatch {
    pub gamebanana_mod_id: u64,
    pub file_id: u64,
    /// How many installs contributed this hash. Higher is a stronger match.
    pub reports: u64,
}

/// App data directory and version for [`identify`], which takes neither. Also the fallback
/// version for [`contribute_hash`]. Call once at startup and again if either changes.
pub fn configure(data_dir: &Path, version: &str) {
    let config = Config {
        data_dir: data_dir.to_path_buf(),
        version: app_version(version),
    };
    if let Ok(mut slot) = CONFIG.lock() {
        *slot = Some(config);
    }
}

#[derive(Clone, Debug)]
struct Config {
    data_dir: PathBuf,
    version: String,
}

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

fn config() -> Option<Config> {
    CONFIG.lock().ok().and_then(|slot| slot.clone())
}

/// One queued usage event, in the shape `/v1/events` expects.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Queued {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    page: Option<String>,
    ts: i64,
}

/// A crash file on disk. `version` and `os` describe the build that crashed, and the send
/// reports that version (the current one when it is not a valid version).
///
/// `kind` is `panic` (the panic hook), `unclean_exit` (no panic, the session marker was left
/// behind) or `native` (a Windows exception filter record). `thread` names the panicking thread.
/// `context` is the scrubbed log tail of the session, added at the next start. Files from older
/// builds have none of the three and read as a panic.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CrashFile {
    message: String,
    backtrace: String,
    version: String,
    os: String,
    ts: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    thread: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    context: String,
}

fn telemetry_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

fn consent_path(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(CONSENT_FILE)
}

fn install_id_path(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(INSTALL_ID_FILE)
}

fn queue_path(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(QUEUE_FILE)
}

fn crash_dir(data_dir: &Path) -> PathBuf {
    telemetry_dir(data_dir).join(CRASH_DIR)
}

/// Read the consent file. A missing or unreadable file means "not asked, everything off".
pub fn read_consent(data_dir: &Path) -> Consent {
    fs::read_to_string(consent_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Store the consent file through a temporary sibling, so an interrupted write cannot leave a
/// half answer behind. The file is written as given; [`read_consent`] applies the `asked` gate.
pub fn write_consent(data_dir: &Path, consent: &Consent) -> Result<()> {
    let text = serde_json::to_vec_pretty(consent)?;
    write_atomic(&consent_path(data_dir), &text)
}

/// The install id for this app data directory: a random UUID v4, made once and stored beside the
/// consent file. It is not a machine id and nothing else derives from it. If the file cannot be
/// written the id still answers for this process; a later call makes a new one.
pub fn install_id(data_dir: &Path) -> String {
    let path = install_id_path(data_dir);
    if let Ok(text) = fs::read_to_string(&path) {
        let id = text.trim().to_ascii_lowercase();
        if is_uuid(&id) {
            return id;
        }
    }
    let id = uuid_v4();
    let _ = write_atomic(&path, id.as_bytes());
    id
}

/// Queue one usage event locally. No-op when usage consent is off or `name` is not in
/// [`EVENTS`]. A page is kept only when it is itself an allowlisted name, so no free text from
/// the UI can reach the queue. Writing is bounded: the oldest event falls off past 500.
pub fn record(data_dir: &Path, name: &str, page: Option<&str>) {
    if !read_consent(data_dir).usage_on() || !EVENTS.contains(&name) {
        return;
    }
    let page = page
        .map(str::trim)
        .filter(|value| EVENTS.contains(value))
        .map(str::to_string);
    let event = Queued {
        name: name.to_string(),
        page,
        ts: Utc::now().timestamp_millis(),
    };

    let _guard = queue_lock();
    let mut queue = read_queue(data_dir);
    queue.push(event);
    if queue.len() > QUEUE_LIMIT {
        let drop = queue.len() - QUEUE_LIMIT;
        queue.drain(0..drop);
    }
    if let Err(error) = write_queue(data_dir, &queue) {
        note(&format!("the event queue could not be written: {error}"));
    }
}

/// Send queued events in batches of [`BATCH_SIZE`] while usage consent is on. Returns how many
/// went. Without consent the queue is deleted and nothing is sent. A failed batch stays queued
/// for the next call; the failure is one stderr line, not an error the UI has to handle.
pub fn flush(data_dir: &Path, version: &str) -> Result<usize> {
    if !read_consent(data_dir).usage_on() {
        let _ = fs::remove_file(queue_path(data_dir));
        return Ok(0);
    }
    flush_batches(data_dir, &app_version(version), ENDPOINT)
}

/// [`flush`] against a chosen base URL. The caller has already checked consent.
fn flush_batches(data_dir: &Path, version: &str, base: &str) -> Result<usize> {
    let _guard = queue_lock();
    let mut queue = read_queue(data_dir);
    if queue.is_empty() {
        return Ok(0);
    }
    let install_id = install_id(data_dir);
    let os = os_name();
    let mut sent = 0;
    while !queue.is_empty() {
        let take = queue.len().min(BATCH_SIZE);
        let batch: Vec<Queued> = queue[..take].to_vec();
        let body = json!({
            "installId": install_id,
            "version": version,
            "os": os,
            "events": batch,
        });
        match post(&endpoint_at(base, "/v1/events"), version, &body, EVENT_TIMEOUT) {
            Ok(_) => {
                queue.drain(0..take);
                sent += take;
                if let Err(error) = write_queue(data_dir, &queue) {
                    note(&format!("the event queue could not be trimmed: {error}"));
                    break;
                }
            }
            Err(error) => {
                note(&format!("queued events were not sent: {error}"));
                break;
            }
        }
    }
    Ok(sent)
}

/// The panic hook target. Updated by every [`install_panic_hook`] call; the hook itself is
/// installed once per process.
#[derive(Clone, Debug)]
struct CrashTarget {
    dir: PathBuf,
    version: String,
}

static CRASH_TARGET: Mutex<Option<CrashTarget>> = Mutex::new(None);
static HOOK_INSTALLED: Once = Once::new();

/// Install a panic hook that writes a scrubbed crash file before the previous hook runs.
///
/// The file lands in `<data>/telemetry/crashes/<time>.json` with the message, the location and a
/// forced backtrace. Writing is local and happens whether or not crash consent is on; sending
/// waits for [`send_pending_crashes`]. Installing twice updates the target and keeps one hook.
pub fn install_panic_hook(data_dir: &Path, version: &str) {
    let target = CrashTarget {
        dir: crash_dir(data_dir),
        version: app_version(version),
    };
    if let Ok(mut slot) = CRASH_TARGET.lock() {
        *slot = Some(target);
    }
    HOOK_INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let target = CRASH_TARGET.lock().ok().and_then(|slot| slot.clone());
            if let Some(target) = target {
                // A panic inside the hook would abort the process instead of unwinding.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    write_crash(&target, info);
                }));
            }
            previous(info);
        }));
    });
}

fn write_crash(target: &CrashTarget, info: &std::panic::PanicHookInfo<'_>) {
    let mut message = match info.payload().downcast_ref::<&str>() {
        Some(text) => (*text).to_string(),
        None => match info.payload().downcast_ref::<String>() {
            Some(text) => text.clone(),
            None => "panic with a non-text payload".to_string(),
        },
    };
    if let Some(location) = info.location() {
        message.push_str(&format!(
            " at {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        ));
    }
    let message = truncate(&scrub(&message), MAX_CRASH_MESSAGE);
    let backtrace = truncate(
        &scrub(&std::backtrace::Backtrace::force_capture().to_string()),
        MAX_CRASH_BACKTRACE,
    );
    let now = Utc::now();
    let thread = std::thread::current()
        .name()
        .map(str::to_string)
        .unwrap_or_else(|| "unnamed".to_string());
    let crash = CrashFile {
        message,
        backtrace,
        version: target.version.clone(),
        os: os_name().to_string(),
        ts: now.timestamp_millis(),
        kind: crash::KIND_PANIC.to_string(),
        thread: truncate(&scrub(&thread), 128),
        context: String::new(),
    };
    let Ok(text) = serde_json::to_vec_pretty(&crash) else {
        return;
    };
    if fs::create_dir_all(&target.dir).is_err() {
        return;
    }
    let name = format!("{}.json", now.format("%Y-%m-%dT%H-%M-%S%.9fZ"));
    if fs::write(target.dir.join(name), text).is_err() {
        return;
    }
    prune_crashes(&target.dir);
}

/// Keep the newest [`CRASH_LIMIT`] crash files. Names start with the time, so a plain sort is
/// oldest first.
fn prune_crashes(dir: &Path) {
    let mut files = crash_files(dir);
    if files.len() <= CRASH_LIMIT {
        return;
    }
    files.sort();
    let drop = files.len() - CRASH_LIMIT;
    for path in files.into_iter().take(drop) {
        let _ = fs::remove_file(path);
    }
}

fn crash_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .map(|entry| entry.path())
        .collect()
}

/// Send crash files written by the panic hook, oldest first, while crash consent is on. A file
/// is deleted after the Worker accepts it; the first failure stops the run and leaves the rest
/// for later. Unreadable files are removed so they cannot block the queue. Without consent
/// nothing is sent and the files stay on disk.
pub fn send_pending_crashes(data_dir: &Path, version: &str) -> Result<usize> {
    if !read_consent(data_dir).crashes_on() {
        return Ok(0);
    }
    send_crashes_at(data_dir, &app_version(version), ENDPOINT)
}

/// [`send_pending_crashes`] against a chosen base URL. The caller has already checked consent.
/// Every file passes the local rate limit and duplicate check (`crash::Ledger`): a crash already
/// sent in the last day is deleted unsent, and the run stops at the hourly or daily cap.
fn send_crashes_at(data_dir: &Path, version: &str, base: &str) -> Result<usize> {
    let mut files = crash_files(&crash_dir(data_dir));
    files.sort();
    let mut sent = 0;
    for path in files {
        match crash::send_file_at(data_dir, version, base, &path, Utc::now().timestamp_millis()) {
            crash::Sent::Yes => sent += 1,
            crash::Sent::Duplicate | crash::Sent::Unreadable => {}
            crash::Sent::Limited => break,
            crash::Sent::Failed(error) => {
                note(&format!("a crash report was not sent: {error}"));
                break;
            }
        }
    }
    Ok(sent)
}

/// Send the player's diagnostic bundle: a scrubbed summary and log tail, the app version and the
/// OS, keyed by the install id. Returns the Worker's reference code.
///
/// This is the one send that does not read the consent file: the player pressed Send, and that
/// press is the consent, every time. Nothing is written locally.
pub fn send_diagnostics(data_dir: &Path, version: &str, summary: &str, log: &str) -> Result<String> {
    let version = app_version(version);
    send_diagnostics_at(data_dir, &version, ENDPOINT, summary, log)
}

fn send_diagnostics_at(
    data_dir: &Path,
    version: &str,
    base: &str,
    summary: &str,
    log: &str,
) -> Result<String> {
    let summary = truncate(&scrub(summary), MAX_DIAGNOSTICS_SUMMARY);
    let log = tail(&scrub(log), MAX_DIAGNOSTICS_LOG);
    let body = json!({
        "installId": install_id(data_dir),
        "version": version,
        "os": os_name(),
        "summary": summary,
        "log": log,
    });
    let text = post(
        &endpoint_at(base, "/v1/diagnostics"),
        version,
        &body,
        DIAGNOSTICS_TIMEOUT,
    )?;
    parse_diagnostics_code(&text).ok_or_else(|| {
        Error::Message("The telemetry Worker did not answer with a reference code.".to_string())
    })
}

/// Look up a sha256 and size in the community hash index. Returns matches best first, or an
/// empty list when hash lookup consent is off or the app has not called [`configure`]. A lookup
/// sends only the hash and size.
pub fn identify(sha256: &str, size: u64) -> Result<Vec<HashMatch>> {
    let Some(config) = config() else {
        return Ok(Vec::new());
    };
    if !read_consent(&config.data_dir).hash_lookup_on() {
        return Ok(Vec::new());
    }
    identify_at(ENDPOINT, &config.version, sha256, size)
}

fn identify_at(base: &str, version: &str, sha256: &str, size: u64) -> Result<Vec<HashMatch>> {
    let sha256 = normal_sha256(sha256).ok_or_else(|| {
        Error::Message("That is not a sha256 hash: 64 hex characters are needed.".to_string())
    })?;
    if size == 0 {
        return Err(Error::Message(
            "A file with no size cannot be looked up.".to_string(),
        ));
    }
    let url = format!(
        "{}?sha256={sha256}&size={size}",
        endpoint_at(base, "/v1/identify")
    );
    let text = get(&url, version, EVENT_TIMEOUT)?;
    parse_matches(&text)
}

/// Add a hash to the index after installing that file from GameBanana. Only with hash lookup
/// consent. Same data as a lookup: the hash, the size and the GameBanana ids.
pub fn contribute_hash(
    data_dir: &Path,
    sha256: &str,
    size: u64,
    mod_id: u64,
    file_id: u64,
) -> Result<()> {
    if !read_consent(data_dir).hash_lookup_on() {
        return Ok(());
    }
    let version = config()
        .map(|config| config.version)
        .unwrap_or_else(|| app_version(""));
    contribute_hash_at(&version, ENDPOINT, &install_id(data_dir), sha256, size, mod_id, file_id)
}

/// The install id goes with a contribution so the Worker can count each install once (it keeps
/// only a salted hash of it). Lookups never send it.
fn contribute_hash_at(
    version: &str,
    base: &str,
    install: &str,
    sha256: &str,
    size: u64,
    mod_id: u64,
    file_id: u64,
) -> Result<()> {
    let sha256 = normal_sha256(sha256).ok_or_else(|| {
        Error::Message("That is not a sha256 hash: 64 hex characters are needed.".to_string())
    })?;
    if size == 0 || mod_id == 0 || file_id == 0 {
        return Err(Error::Message(
            "A hash needs a size, a mod id and a file id.".to_string(),
        ));
    }
    let body = json!({
        "installId": install,
        "sha256": sha256,
        "size": size,
        "gamebananaModId": mod_id,
        "fileId": file_id,
    });
    post(&endpoint_at(base, "/v1/hashes"), version, &body, EVENT_TIMEOUT).map(|_| ())
}

/// Strip anything that identifies the player or a key from text bound for a crash report or a
/// diagnostic bundle.
///
/// Covers the running user's home directory in plain and doubled-backslash form, any other
/// `/home/<user>`, `/Users/<user>`, `C:\Users\<user>` or `X:\home\<user>` path, API keys and
/// tokens (`sk-`, `gsk_`, `xai-`, `Bearer`, JWTs, 32 or more hex or base64 characters), emails,
/// and Steam ids. The Worker applies the same rules again.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();

    // The running user's home first: a user name with a space would otherwise be half redacted
    // by the structured rules and no longer match literally.
    for home in home_dirs() {
        for variant in [home.replace('\\', "\\\\"), home] {
            if variant.len() < 4 {
                continue;
            }
            let pattern = format!("(?i){}", regex::escape(&variant));
            if let Ok(regex) = Regex::new(&pattern) {
                out = regex.replace_all(&out, "<home>").into_owned();
            }
        }
    }

    for (regex, replacement) in scrub_rules() {
        out = regex.replace_all(&out, *replacement).into_owned();
    }
    out
}

fn home_dirs() -> Vec<String> {
    ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| value.len() > 3)
        .collect()
}

fn scrub_rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut rules = Vec::new();
        let mut add = |pattern: &str, replacement: &'static str| {
            rules.push((Regex::new(pattern).expect("scrub pattern"), replacement));
        };
        // Home paths that are not the running user's literal home.
        add(r#"(?i)(/home/)[^/\s'"`]+"#, "$1<user>");
        add(r#"(?i)(/Users/)[^/\s'"`]+"#, "$1<user>");
        add(r#"(?i)([A-Za-z]:\\{1,2}Users\\{1,2})[^\\\s'"`]+"#, "$1<user>");
        add(r#"(?i)([A-Za-z]:\\{1,2}home\\{1,2})[^\\\s'"`]+"#, "$1<user>");
        // Known key shapes before the generic runs, so the prefix survives.
        add(r"sk-[A-Za-z0-9_-]{8,}", "sk-<token>");
        add(r"gsk_[A-Za-z0-9_-]{8,}", "gsk_<token>");
        add(r"xai-[A-Za-z0-9_-]{8,}", "xai-<token>");
        add(r"(?i)(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}", "$1<token>");
        add(
            r"eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}",
            "<token>",
        );
        // Steam ids can appear in game logs and a crash dump.
        add(r"\b7656119\d{10}\b", "<steamid64>");
        add(r"\[U:1:\d+\]|\bU:1:\d+\b", "<steamid3>");
        // Emails before long runs, so the address is replaced whole.
        add(
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b",
            "<email>",
        );
        // Long opaque runs. Base64 excludes `/` and `.` so paths and file names survive, and
        // the missing end boundary lets `=` padding stay inside the match.
        add(r"\b[0-9a-fA-F]{32,}\b", "<token>");
        add(r"\b[A-Za-z0-9][A-Za-z0-9+=_-]{31,}", "<token>");
        rules
    })
}

/// Parse the reference code out of a `/v1/diagnostics` answer, checking its shape. The Worker
/// makes codes like `SGL-AB12-CD34`; anything else is a bug or an error page.
fn parse_diagnostics_code(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let code = value.get("code")?.as_str()?.trim();
    is_reference_code(code).then(|| code.to_string())
}

fn is_reference_code(code: &str) -> bool {
    let Some(rest) = code.strip_prefix("SGL-") else {
        return false;
    };
    let mut parts = rest.split('-');
    let (Some(first), Some(second), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [first, second].iter().all(|part| {
        part.len() == 4
            && part
                .chars()
                .all(|value| value.is_ascii_uppercase() || value.is_ascii_digit())
    })
}

fn parse_matches(text: &str) -> Result<Vec<HashMatch>> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        Error::Message(format!(
            "The telemetry Worker answered something unreadable: {error}"
        ))
    })?;
    let list = value
        .get("matches")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut matches = Vec::new();
    for item in list {
        let (Some(mod_id), Some(file_id)) = (
            item.get("gamebananaModId").and_then(Value::as_u64),
            item.get("fileId").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let reports = item.get("reports").and_then(Value::as_u64).unwrap_or(0);
        matches.push(HashMatch {
            gamebanana_mod_id: mod_id,
            file_id,
            reports,
        });
    }
    // The Worker sorts already; sorting again keeps "best first" true if that changes.
    matches.sort_by(|left, right| right.reports.cmp(&left.reports));
    Ok(matches)
}

/// A 64 character hex sha256, lowercased, or None.
fn normal_sha256(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    (value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()))
        .then_some(value)
}

fn queue_lock() -> MutexGuard<'static, ()> {
    QUEUE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static QUEUE_LOCK: Mutex<()> = Mutex::new(());

fn read_queue(data_dir: &Path) -> Vec<Queued> {
    let Ok(text) = fs::read_to_string(queue_path(data_dir)) else {
        return Vec::new();
    };
    let mut queue: Vec<Queued> = serde_json::from_str(&text).unwrap_or_default();
    // A hand-edited file must not get an unknown name or page past the allowlist.
    queue.retain(|event| EVENTS.contains(&event.name.as_str()));
    for event in &mut queue {
        if event.page.as_deref().is_some_and(|page| !EVENTS.contains(&page)) {
            event.page = None;
        }
    }
    if queue.len() > QUEUE_LIMIT {
        let drop = queue.len() - QUEUE_LIMIT;
        queue.drain(0..drop);
    }
    queue
}

fn write_queue(data_dir: &Path, queue: &[Queued]) -> Result<()> {
    let text = serde_json::to_vec(queue)?;
    write_atomic(&queue_path(data_dir), &text)
}

fn endpoint_at(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn agent(timeout: Duration) -> ureq::Agent {
    crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

fn post(url: &str, version: &str, body: &Value, timeout: Duration) -> Result<String> {
    let payload = serde_json::to_vec(body)?;
    let mut response = agent(timeout)
        .post(url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "Sigil")
        .header("x-sigil-client", CLIENT_HEADER)
        .header("x-sigil-version", version)
        .send(payload.as_slice())
        .map_err(|error| Error::Network(error.to_string()))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "the telemetry Worker answered HTTP {status}"
        )));
    }
    Ok(text)
}

fn get(url: &str, version: &str, timeout: Duration) -> Result<String> {
    let mut response = agent(timeout)
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "Sigil")
        .header("x-sigil-client", CLIENT_HEADER)
        .header("x-sigil-version", version)
        .call()
        .map_err(|error| Error::Network(error.to_string()))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "the telemetry Worker answered HTTP {status}"
        )));
    }
    Ok(text)
}

/// One line on stderr, the only logging the core does. Nothing here carries a key or a token.
fn note(message: &str) {
    eprintln!("[sigil telemetry] {message}");
}

fn os_name() -> &'static str {
    std::env::consts::OS
}

/// A version the Worker accepts: semver-ish, starting with a digit. Anything else falls back to
/// the workspace version.
fn app_version(version: &str) -> String {
    let value = version.trim();
    let valid = !value.is_empty()
        && value.len() <= 32
        && value.starts_with(|character: char| character.is_ascii_digit())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+'));
    if valid {
        value.to_string()
    } else {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// A random UUID v4. `getrandom` reads the OS source; if that fails, a sha256 of the clock and
/// the process id still gives 16 different bytes.
fn uuid_v4() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let mut hasher = Sha256::new();
        hasher.update(nanos.to_le_bytes());
        hasher.update(std::process::id().to_le_bytes());
        let digest = hasher.finalize();
        bytes.copy_from_slice(&digest[..16]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value
            .char_indices()
            .all(|(index, character)| match index {
                8 | 13 | 18 | 23 => character == '-',
                _ => character.is_ascii_hexdigit(),
            })
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n... truncated", &text[..end])
}

fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Crash detection across restarts: the session marker, the record of a run that did not exit
/// cleanly, the decision to ask or send, and the local rate limit.
#[path = "telemetry_crash.rs"]
pub mod crash;

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
