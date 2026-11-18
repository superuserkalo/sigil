//! The download queue: several archives at once, with progress, pause, resume, cancel and
//! retry, a mirror to fall back to, and a look inside the archive before it is installed.
//!
//! The queue is app data (`download-queue.json` under the data directory) and never touches the
//! game install. One worker thread owns it: it resolves each target, streams the archive to
//! `<workspace>/downloads/<cache key>/`, and hands the finished path to the install dialog,
//! which still asks before anything is written to `game/citadel/addons`.
//!
//! A download streams in one connection so pause and resume can keep the bytes already on disk
//! and ask the server for the rest with `Range`. The multi-range path in
//! [`crate::fast_download`] stays on the install route, where there is no pause to support;
//! [`crate::fast_download::content_range_ok`] is used here to check the resume response.
//!
//! The scan before install reads a ZIP's central directory without unpacking it. A `.7z` or
//! `.rar` has no built-in reader, so it is unpacked into a scratch folder with the same limits
//! the install uses, read, and removed. A scan that cannot run says so instead of pretending
//! the archive is clean.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::fast_download::content_range_ok;
use crate::file_servers;
use crate::install::{
    self, DownloadPolicy, GameBananaFile, InstallSource, WebUrl,
};
use crate::util::write_atomic;

mod speed;
pub use speed::{SLOW_BPS, SLOW_GAP, SLOW_HOLD, SPEED_SAMPLE};
use speed::Rate;

/// Queue file under the app data directory.
pub const QUEUE_NAME: &str = "download-queue.json";

/// Schema version written to the queue.
pub const QUEUE_SCHEMA: u32 = 1;

/// Items kept in the list. Finished ones beyond this are dropped oldest first.
pub const MAX_ITEMS: usize = 40;

/// Read size while streaming.
pub const READ_BUFFER: usize = 64 * 1024;

/// How long the worker waits between checks when there is nothing to do.
pub const IDLE: Duration = Duration::from_millis(200);

/// The shortest gap between two progress wakes of the UI while an archive streams.
pub const PROGRESS_WAKE: Duration = Duration::from_millis(150);

/// A ZIP bigger than this is scanned by name only when it is a ZIP; other formats are not
/// unpacked for the scan.
pub const SCAN_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Scratch folder for the scan of a format that has no built-in reader.
pub const SCAN_DIR_NAME: &str = "download-scan";

/// Extensions that never belong in a Deadlock mod and are worth a warning.
pub const EXECUTABLE: [&str; 12] = [
    "exe", "dll", "bat", "cmd", "com", "ps1", "sh", "vbs", "js", "scr", "msi", "hta",
];

/// Shortcut and autorun files that only make sense on Windows desktops.
pub const SHORTCUT: [&str; 4] = ["url", "lnk", "inf", "reg"];

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// Where one queued download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Queued,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Queued => "Waiting",
            State::Running => "Downloading",
            State::Paused => "Paused",
            State::Done => "Downloaded",
            State::Failed => "Failed",
            State::Cancelled => "Cancelled",
        }
    }

    /// Finished states keep their row until the player clears it or retries.
    pub fn finished(self) -> bool {
        matches!(self, State::Done | State::Failed | State::Cancelled)
    }
}

/// One warning from the scan before install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanWarning {
    pub path: String,
    pub message: String,
}

/// What the scan before install found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    /// False when the archive could not be read; `error` says why.
    pub checked: bool,
    pub files: usize,
    pub bytes: u64,
    pub warnings: Vec<ScanWarning>,
    pub error: Option<String>,
}

impl ScanReport {
    /// True when something in the archive is worth stopping for.
    pub fn suspicious(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// The line the install dialog shows above everything else.
    pub fn summary(&self) -> String {
        if let Some(error) = &self.error {
            return format!("Could not look inside the archive: {error}");
        }
        match self.warnings.len() {
            0 => String::new(),
            1 => self.warnings[0].message.clone(),
            n => format!("{} and {} more.", self.warnings[0].message, n - 1),
        }
    }
}

/// One archive in the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    /// What the player typed or dropped: a mod id, a link or a local file.
    pub target: String,
    /// Folder name a GameBanana archive lands in: the mod name the queue knows.
    pub name: String,
    pub mod_id: Option<u64>,
    /// Resolved file URL. None until the worker read the mod page.
    pub url: Option<String>,
    pub file_name: String,
    /// Absolute path of the finished archive.
    pub path: Option<PathBuf>,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub state: State,
    pub attempts: u32,
    pub error: Option<String>,
    /// The file server that answered, for the row.
    pub server: Option<String>,
    pub added_at: String,
    pub finished_at: Option<String>,
    pub scan: ScanReport,
    /// Speed and the slow window while this row streams. Cleared on a new run and never saved:
    /// a speed from the last launch means nothing.
    #[serde(skip)]
    rate: Rate,
}

impl Item {
    pub fn new(id: String, target: String) -> Self {
        Item {
            id,
            target,
            name: String::new(),
            mod_id: None,
            url: None,
            file_name: String::new(),
            path: None,
            bytes_total: 0,
            bytes_done: 0,
            state: State::Queued,
            attempts: 0,
            error: None,
            server: None,
            added_at: Utc::now().to_rfc3339(),
            finished_at: None,
            scan: ScanReport::default(),
            rate: Rate::default(),
        }
    }

    /// `0.4` to `1.0`, or None while the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        (self.bytes_total > 0).then(|| (self.bytes_done as f32 / self.bytes_total as f32).min(1.0))
    }

    /// The smoothed speed in bytes a second while the row streams.
    ///
    /// None before the first sample lands, and None for a row that is not running: a paused,
    /// finished or cancelled row has no current speed. The number is lightly averaged, so it
    /// does not jump on every chunk.
    pub fn speed_bps(&self) -> Option<u64> {
        if self.state == State::Running {
            self.rate.bps()
        } else {
            None
        }
    }

    /// Whole seconds left at the smoothed speed, rounded up.
    ///
    /// None when the row is not running, when the total size is unknown ([`Item::bytes_total`]
    /// is zero), when nothing is left, or before the first speed sample. A caller that wants a
    /// label can format the pair, but the queue only hands out the number.
    pub fn eta_secs(&self) -> Option<u64> {
        if self.state != State::Running
            || self.bytes_total == 0
            || self.bytes_done >= self.bytes_total
        {
            return None;
        }
        let bps = self.speed_bps().filter(|bps| *bps > 0)?;
        Some((self.bytes_total - self.bytes_done).div_ceil(bps))
    }

    /// True when the smoothed speed stayed under [`SLOW_BPS`] for [`SLOW_HOLD`] during this
    /// run. One slow chunk is not enough, and the flag clears as soon as the speed recovers.
    pub fn is_slow(&self) -> bool {
        // 97 percent done: it finishes before a notice would help (Deadlock Mod Manager's rule).
        let nearly_done = self.bytes_total > 0 && self.bytes_done * 100 >= self.bytes_total * 97;
        self.state == State::Running && self.rate.slow() && !nearly_done
    }

    /// The line under the name: what the row is doing right now.
    pub fn detail(&self) -> String {
        match self.state {
            State::Queued => "Waiting its turn.".to_string(),
            State::Running => {
                let of = if self.bytes_total > 0 {
                    format!(" of {}", size(self.bytes_total))
                } else {
                    String::new()
                };
                format!("{}{of}", size(self.bytes_done))
            }
            State::Paused => format!("{} downloaded. Resume picks up there.", size(self.bytes_done)),
            State::Done => {
                let by = self
                    .server
                    .as_ref()
                    .map(|server| format!(" from {server}"))
                    .unwrap_or_default();
                format!("{}{by}", size(self.bytes_done))
            }
            State::Failed => self
                .error
                .clone()
                .unwrap_or_else(|| "The download stopped.".to_string()),
            State::Cancelled => "Removed from the queue.".to_string(),
        }
    }

    /// When the row was added, in Unix milliseconds. Lets a caller that sent an `Add` find
    /// its row among older ones for the same target.
    pub fn added_ms(&self) -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(&self.added_at)
            .ok()
            .map(|at| at.timestamp_millis())
    }

    /// A download that already has bytes on disk can resume instead of starting over.
    pub fn resumable(&self) -> bool {
        self.bytes_done > 0 && matches!(self.state, State::Paused | State::Failed)
    }
}

/// The queue. Serializable, so a restart keeps what was waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub schema: u32,
    pub items: Vec<Item>,
    /// The whole queue is paused; each item keeps its own state.
    pub paused: bool,
}

impl Default for Queue {
    fn default() -> Self {
        Queue {
            schema: QUEUE_SCHEMA,
            items: Vec::new(),
            paused: false,
        }
    }
}

impl Queue {
    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    /// Downloads, including running and paused ones.
    pub fn active(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item.state, State::Queued | State::Running | State::Paused))
            .count()
    }

    pub fn done(&self) -> usize {
        self.items.iter().filter(|item| item.state == State::Done).count()
    }

    pub fn total_bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|item| item.state != State::Cancelled)
            .map(|item| item.bytes_total)
            .sum()
    }

    pub fn done_bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|item| item.state != State::Cancelled)
            .map(|item| item.bytes_done)
            .sum()
    }

    /// Where a waiting item stands in line: 1 runs next. None for an item that is running,
    /// paused, finished or unknown. The order is the one [`Queue::next_runnable`] follows.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.items
            .iter()
            .filter(|item| item.state == State::Queued)
            .position(|item| item.id == id)
            .map(|index| index + 1)
    }

    /// [`Queue::position`] for every waiting item, by id.
    pub fn positions(&self) -> std::collections::BTreeMap<String, usize> {
        self.items
            .iter()
            .filter(|item| item.state == State::Queued)
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index + 1))
            .collect()
    }

    /// The newest queue row for a GameBanana mod, with its place in line when it waits, for a
    /// Browse card ("Queued #3", "Downloading from 2.gamebanana.com").
    pub fn for_mod(&self, mod_id: u64) -> Option<(&Item, Option<usize>)> {
        let item = self.items.iter().rev().find(|item| {
            item.mod_id == Some(mod_id) || item.target.trim() == mod_id.to_string()
        })?;
        Some((item, self.position(&item.id)))
    }

    /// The next item the worker should pick up.
    pub fn next_runnable(&self) -> Option<String> {
        if self.paused {
            return None;
        }
        self.items
            .iter()
            .find(|item| item.state == State::Queued)
            .map(|item| item.id.clone())
    }

    /// Add a target. Refuses a target the install pipeline cannot read, and a queue past
    /// [`MAX_ITEMS`].
    pub fn add(&mut self, target: &str) -> Result<String> {
        InstallSource::parse(target)?;
        if self.items.len() >= MAX_ITEMS {
            return Err(Error::msg(format!(
                "The queue holds {MAX_ITEMS} downloads already. Clear one first."
            )));
        }
        let mut number = self
            .items
            .iter()
            .filter_map(|item| item.id.strip_prefix('d')?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        while self.items.iter().any(|item| item.id == format!("d{number}")) {
            number += 1;
        }
        let id = format!("d{number}");
        self.items.push(Item::new(id.clone(), target.trim().to_string()));
        Ok(id)
    }

    /// Put a finished item back in the queue.
    pub fn retry(&mut self, id: &str) -> bool {
        let Some(item) = self.get_mut(id) else {
            return false;
        };
        match item.state {
            State::Failed | State::Cancelled | State::Done => {
                item.state = State::Queued;
                item.error = None;
                item.attempts = 0;
                item.finished_at = None;
                item.bytes_done = 0;
                item.scan = ScanReport::default();
                item.rate = Rate::default();
                true
            }
            _ => false,
        }
    }

    /// Pause one item. The bytes on disk stay, so Resuming continues.
    pub fn pause(&mut self, id: &str) -> bool {
        let Some(item) = self.get_mut(id) else {
            return false;
        };
        if matches!(item.state, State::Queued | State::Running) {
            item.state = State::Paused;
            true
        } else {
            false
        }
    }

    /// Put a paused item back in the queue.
    pub fn resume(&mut self, id: &str) -> bool {
        let Some(item) = self.get_mut(id) else {
            return false;
        };
        if item.state == State::Paused {
            item.state = State::Queued;
            true
        } else {
            false
        }
    }

    /// Forget an item. Cancelled means the partial file is gone too.
    pub fn cancel(&mut self, id: &str) -> bool {
        let Some(item) = self.get_mut(id) else {
            return false;
        };
        if matches!(item.state, State::Cancelled) {
            return false;
        }
        item.state = State::Cancelled;
        item.error = None;
        item.bytes_done = 0;
        item.finished_at = Some(Utc::now().to_rfc3339());
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        self.items.len() != before
    }

    /// Move a waiting row to `index`, counted in the whole item list and clamped to the end.
    ///
    /// Only a row that has not started can move: a running row is already streaming and a
    /// finished one is history, so both are refused with an error and keep their place. A
    /// paused row counts as started too, because it already has bytes on disk.
    pub fn move_to(&mut self, id: &str, index: usize) -> Result<()> {
        let Some(from) = self.items.iter().position(|item| item.id == id) else {
            return Err(Error::msg(format!("There is no download {id} in the queue.")));
        };
        match self.items[from].state {
            State::Queued => {}
            State::Running => {
                return Err(Error::msg(
                    "A download that is running keeps its place in the queue.",
                ));
            }
            _ => {
                return Err(Error::msg(
                    "Only a waiting download can be moved. This one has already started or finished.",
                ));
            }
        }
        let item = self.items.remove(from);
        let to = index.min(self.items.len());
        self.items.insert(to, item);
        Ok(())
    }

    /// Drop every finished row. True when something went.
    pub fn clear_finished(&mut self) -> bool {
        let before = self.items.len();
        self.items.retain(|item| !item.state.finished());
        self.items.len() != before
    }

    /// Keep the newest [`MAX_ITEMS`] finished rows, oldest first out.
    pub fn trim(&mut self) {
        while self.items.len() > MAX_ITEMS {
            match self
                .items
                .iter()
                .position(|item| item.state.finished() && item.state != State::Done)
                .or_else(|| self.items.iter().position(|item| item.state.finished()))
            {
                Some(index) => {
                    self.items.remove(index);
                }
                None => break,
            }
        }
    }
}

/// `1.2 MB`, `300 KB`, `0 B`.
pub fn size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

/// A plain archive file name: no separators, no control characters, not a dot name.
pub fn safe_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 180 || name == "." || name == ".." {
        return None;
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return None;
    }
    Some(name.to_string())
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Where the queue lives.
pub fn queue_path(data_dir: &Path) -> PathBuf {
    data_dir.join(QUEUE_NAME)
}

/// Read the queue. A missing or unreadable file is an empty queue.
///
/// A row left `Running` belongs to a process that is gone: it comes back as `Paused`, which
/// is the truth on disk (the partial file is still there) and gives the player Resume.
pub fn load_queue(data_dir: &Path) -> Queue {
    let Ok(text) = fs::read_to_string(queue_path(data_dir)) else {
        return Queue::default();
    };
    let Ok(mut queue) = serde_json::from_str::<Queue>(&text) else {
        return Queue::default();
    };
    if queue.schema != QUEUE_SCHEMA {
        return Queue::default();
    }
    for item in &mut queue.items {
        if item.state == State::Running {
            item.state = State::Paused;
        }
    }
    queue
}

/// Write the queue. App data write, atomic.
pub fn store_queue(data_dir: &Path, queue: &Queue) -> Result<()> {
    let mut text = serde_json::to_string_pretty(queue)?;
    text.push('\n');
    write_atomic(&queue_path(data_dir), text.as_bytes())
}

mod scan;
pub use scan::{scan_archive, scan_entries, list_zip};

// ---------------------------------------------------------------------------
// Mirrors
// ---------------------------------------------------------------------------

/// The file URLs to try for `url`, in order: the one GameBanana chose, then the same path on
/// each healthy server. Only GameBanana file hosts are rewritten.
pub fn failover_candidates(url: &str, servers: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if WebUrl::parse(url).is_err() {
        return out;
    }
    out.push(url.to_string());
    let Some(rest) = url.strip_prefix("https://") else {
        return out;
    };
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, path),
        None => (rest, ""),
    };
    let is_file_host = host == "files.gamebanana.com" || file_servers::valid_name(host.split('.').next().unwrap_or(""));
    if !is_file_host {
        return out;
    }
    for server in servers {
        if !file_servers::valid_name(server) {
            continue;
        }
        let candidate = format!("https://{}.gamebanana.com/{path}", server.trim());
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// Pause and cancel flags shared by the worker and the stream loop.
#[derive(Debug, Default)]
pub struct Control {
    pause: AtomicBool,
    cancel: AtomicBool,
    /// The host the current stream reads from, once its redirects are followed.
    server: Mutex<Option<String>>,
}

impl Control {
    pub fn new() -> Arc<Control> {
        Arc::new(Control::default())
    }

    pub fn pause(&self, on: bool) {
        self.pause.store(on, Ordering::SeqCst);
    }

    pub fn paused(&self) -> bool {
        self.pause.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Record the file server the stream is reading from.
    pub fn set_server(&self, host: Option<String>) {
        if let Ok(mut slot) = self.server.lock() {
            *slot = host;
        }
    }

    /// The file server the stream is reading from, while it runs.
    pub fn server(&self) -> Option<String> {
        self.server.lock().ok().and_then(|slot| slot.clone())
    }
}

/// How a stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEnd {
    /// Every byte arrived. `path` holds the file.
    Done { bytes: u64, sha256: String },
    /// The player cancelled or the app is closing; the partial file was removed.
    Cancelled,
}

/// The result of a whole download with failover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub path: PathBuf,
    pub bytes: u64,
    pub bytes_total: u64,
    pub sha256: String,
    /// The host that answered.
    pub server: Option<String>,
}

fn agent(policy: &DownloadPolicy) -> ureq::Agent {
    crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(policy.timeout_seconds)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn resolve_location(base: &WebUrl, location: &str) -> Result<WebUrl> {
    let value = location.trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        return WebUrl::parse(value);
    }
    if value.starts_with('/') {
        return WebUrl::parse(&format!("{}{value}", base.origin()));
    }
    Err(Error::Network(format!(
        "The download redirect {value:?} is not an absolute or root-relative URL."
    )))
}

/// Open the file for reading, following redirects inside the allowlist.
///
/// `server` forces GameBanana file hosts to one mirror. `from` asks for the rest of a partial
/// file with `Range`.
fn open(
    agent: &ureq::Agent,
    url: &str,
    policy: &DownloadPolicy,
    server: Option<&str>,
    from: u64,
) -> Result<(WebUrl, ureq::http::Response<ureq::Body>)> {
    let mut current = WebUrl::parse(url)?;
    for _ in 0..=install::MAX_REDIRECTS {
        if !policy.allows(&current) {
            return Err(Error::Network(format!(
                "Refusing to download from {}. That origin is not in the install allowlist.",
                current.origin()
            )));
        }
        let mut request = agent
            .get(&current.to_string())
            .header("User-Agent", "dah/0.0.0")
            .header("Accept", "application/octet-stream, */*");
        if from > 0 {
            request = request.header("Range", &format!("bytes={from}-"));
        }
        let response = request
            .call()
            .map_err(|error| Error::Network(format!("{current} did not answer: {error}")))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| {
                    Error::Network(format!(
                        "{current} answered HTTP {status} with no Location header."
                    ))
                })?;
            let mut next = resolve_location(&current, &location)?;
            if let Some(server) = server {
                let routed = crate::net::rewrite_file_host(&next.to_string(), server);
                if let Ok(routed) = WebUrl::parse(&routed)
                    && policy.allows(&routed)
                {
                    next = routed;
                }
            }
            current = next;
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(Error::Network(format!(
                "{current} answered HTTP {status}."
            )));
        }
        return Ok((current, response));
    }
    Err(Error::Network(format!(
        "The download from {url} used more than {} redirects.",
        install::MAX_REDIRECTS
    )))
}

/// The file server's host name, or None for GameBanana's own pages (not a file server).
fn file_host(url: &WebUrl) -> Option<String> {
    Some(url.host.clone()).filter(|host| host != "gamebanana.com" && host != "www.gamebanana.com")
}

fn part_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    path.with_file_name(format!("{name}.part"))
}

/// Stream one URL into `path`, asking for the rest when `from` is above zero.
///
/// `progress` runs after every read and may pause the stream with [`Control`]. Returns
/// [`StreamEnd::Cancelled`] when the player cancels; any other failure removes the partial file
/// only when `from` was zero, so a resume can try again.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    agent: &ureq::Agent,
    url: &str,
    path: &Path,
    policy: &DownloadPolicy,
    from: u64,
    control: &Control,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<StreamEnd> {
    let (final_url, mut response) = open(agent, url, policy, None, from)?;
    control.set_server(file_host(&final_url));
    let status = response.status().as_u16();
    let length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let part = part_path(path);
    let resumed = status == 206 && from > 0;
    let total = if resumed { from + length.unwrap_or(0) } else { length.unwrap_or(0) };
    let mut done = if resumed { from } else { 0 };

    let mut file = if resumed {
        let header = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !content_range_ok(header, from, from + length.unwrap_or(0).saturating_sub(1), total) {
            // The server answered 206 but not for the range asked. A retry starts again.
            let _ = fs::remove_file(&part);
            return Err(Error::Network(format!(
                "{final_url} answered an unexpected range ({header}). Retry starts the file again."
            )));
        }
        OpenOptions::new().append(true).open(&part)?
    } else {
        let _ = fs::remove_file(&part);
        File::create(&part)?
    };

    if total > 0 {
        progress(done, Some(total));
    } else {
        progress(done, None);
    }

    let mut hasher = Sha256::new();
    if done > 0 {
        // A resumed file was hashed in its earlier session only up to the pause. Hashing from
        // scratch here keeps the digest honest at the cost of one read of the local file.
        let mut existing = File::open(&part)?;
        let mut buffer = vec![0u8; READ_BUFFER];
        loop {
            let read = existing.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }

    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0u8; READ_BUFFER];
    let mut last = Instant::now();
    let mut announced_pause = false;
    loop {
        if control.cancelled() {
            drop(file);
            let _ = fs::remove_file(&part);
            return Ok(StreamEnd::Cancelled);
        }
        if control.paused() {
            if !announced_pause {
                file.flush()?;
                announced_pause = true;
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        announced_pause = false;
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                drop(file);
                return Err(Error::Network(format!(
                    "{final_url} stopped after {}: {error}",
                    size(done)
                )));
            }
        };
        if read == 0 {
            break;
        }
        done += read as u64;
        if done > policy.max_bytes {
            drop(file);
            let _ = fs::remove_file(&part);
            return Err(Error::Network(format!(
                "The download passed the {} cap. Nothing was kept.",
                size(policy.max_bytes)
            )));
        }
        file.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        if last.elapsed() >= Duration::from_millis(120) {
            last = Instant::now();
            progress(done, (total > 0).then_some(total));
        }
    }
    file.flush()?;
    drop(file);
    if total > 0 && done < total {
        // Play it safe: a short body is a failed download, and the part file keeps the bytes.
        return Err(Error::Network(format!(
            "Only {} of {} arrived. Retry resumes from there.",
            size(done),
            size(total)
        )));
    }
    let _ = fs::remove_file(path);
    fs::rename(&part, path)?;
    progress(done, Some(done.max(total)));
    let digest = hasher.finalize();
    let sha256: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(StreamEnd::Done {
        bytes: done,
        sha256,
    })
}

/// The file URL a GameBanana link redirects to, without reading its body.
fn final_url(agent: &ureq::Agent, url: &str, policy: &DownloadPolicy) -> Result<WebUrl> {
    let mut current = WebUrl::parse(url)?;
    for _ in 0..=install::MAX_REDIRECTS {
        if !policy.allows(&current) {
            return Err(Error::Network(format!(
                "Refusing to download from {}. That origin is not in the install allowlist.",
                current.origin()
            )));
        }
        let response = agent
            .get(&current.to_string())
            .header("User-Agent", "dah/0.0.0")
            .header("Accept", "application/octet-stream, */*")
            .header("Range", "bytes=0-0")
            .call()
            .map_err(|error| Error::Network(format!("{current} did not answer: {error}")))?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(current);
        }
        if !(300..400).contains(&status) {
            return Err(Error::Network(format!("{current} answered HTTP {status}.")));
        }
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| Error::Network(format!("{current} answered HTTP {status} with no Location header.")))?;
        current = resolve_location(&current, &location)?;
    }
    Err(Error::Network(format!(
        "The download from {url} used more than {} redirects.",
        install::MAX_REDIRECTS
    )))
}

/// Download `url` into `path`, trying the GameBanana file servers in turn when the first host
/// fails.
///
/// Resume continues from the part file's length on every attempt, so a mirror that is
/// unreachable costs one request and moves on. The file host is learned once, on the first
/// failure, by following the redirect without keeping the body.
pub fn download_with_failover(
    url: &str,
    path: &Path,
    policy: &DownloadPolicy,
    from: u64,
    control: &Control,
    servers: &mut dyn FnMut() -> Vec<String>,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Fetched> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let agent = agent(policy);
    let attempt = |url: &str, progress: &mut dyn FnMut(u64, Option<u64>)| -> Result<StreamEnd> {
        let from = if part_path(path).is_file() {
            part_path(path).metadata().map(|meta| meta.len()).unwrap_or(0)
        } else {
            from
        };
        stream(&agent, url, path, policy, from, control, progress)
    };
    let host_of = |url: &str| {
        WebUrl::parse(url)
            .ok()
            .map(|url| url.host)
            .filter(|host| host != "gamebanana.com" && host != "www.gamebanana.com")
    };

    let mut last_error = match attempt(url, progress) {
        Ok(StreamEnd::Done { bytes, sha256 }) => {
            return Ok(Fetched {
                path: path.to_path_buf(),
                bytes,
                bytes_total: bytes,
                sha256,
                server: host_of(url),
            });
        }
        Ok(StreamEnd::Cancelled) => return Err(Error::msg("Cancelled.")),
        Err(error) => error,
    };

    // The first host failed. Learn the file URL, then try the other GameBanana servers.
    if control.cancelled() {
        return Err(Error::msg("Cancelled."));
    }
    if let Ok(file) = final_url(&agent, url, policy) {
        for candidate in failover_candidates(&file.to_string(), &servers())
            .into_iter()
            .skip(1)
        {
            if control.cancelled() {
                return Err(Error::msg("Cancelled."));
            }
            match attempt(&candidate, progress) {
                Ok(StreamEnd::Done { bytes, sha256 }) => {
                    return Ok(Fetched {
                        path: path.to_path_buf(),
                        bytes,
                        bytes_total: bytes,
                        sha256,
                        server: host_of(&candidate),
                    });
                }
                Ok(StreamEnd::Cancelled) => return Err(Error::msg("Cancelled.")),
                Err(error) => last_error = error,
            }
        }
    }
    Err(last_error)
}

// ---------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------

/// What the UI asks the worker to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Add(String),
    Retry(String),
    Pause(String),
    Resume(String),
    Cancel(String),
    Remove(String),
    ClearFinished,
    /// Pause or resume the whole queue.
    PauseAll(bool),
    /// Move a waiting row to `index` in the list ([`Queue::move_to`]). A running or finished
    /// row stays where it is.
    MoveTo { id: String, index: usize },
    Shutdown,
}

/// Wakes the UI when the worker changes the queue. The desktop hands in Rune's waker; the core
/// does not depend on the UI crate.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The worker's shared state and its command channel.
pub struct Handle {
    shared: Arc<Mutex<Queue>>,
    tx: Option<Sender<Command>>,
    data_dir: PathBuf,
    workspace: PathBuf,
    wake: Option<Wake>,
}

impl Handle {
    /// Start the worker. The queue file is read on the worker thread, never on the UI thread.
    pub fn start(
        data_dir: PathBuf,
        workspace: PathBuf,
        wake: Option<Wake>,
    ) -> Handle {
        let shared = Arc::new(Mutex::new(Queue::default()));
        let (tx, rx) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let worker_data = data_dir.clone();
        let worker_workspace = workspace.clone();
        let worker_wake = wake.clone();
        let spawned = std::thread::Builder::new()
            .name("sigil-downloads".to_string())
            .spawn(move || worker(worker_data, worker_workspace, worker_shared, rx, worker_wake));
        if spawned.is_err() {
            return Handle {
                shared,
                tx: None,
                data_dir,
                workspace,
                wake,
            };
        }
        Handle {
            shared,
            tx: Some(tx),
            data_dir,
            workspace,
            wake,
        }
    }

    pub fn running(&self) -> bool {
        self.tx.is_some()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// A copy of the queue for the frame that is drawing it.
    pub fn snapshot(&self) -> Queue {
        self.shared
            .lock()
            .map(|queue| queue.clone())
            .unwrap_or_default()
    }

    pub fn send(&self, command: Command) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(command);
            if let Some(wake) = &self.wake {
                wake();
            }
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Command::Shutdown);
        }
    }
}

fn worker(
    data_dir: PathBuf,
    workspace: PathBuf,
    shared: Arc<Mutex<Queue>>,
    rx: Receiver<Command>,
    wake: Option<Wake>,
) {
    let notify = || {
        if let Some(wake) = &wake {
            wake();
        }
    };
    let mut queue = load_queue(&data_dir);
    queue.trim();
    // Saving at startup records the promoted Paused states.
    let _ = store_queue(&data_dir, &queue);
    if let Ok(mut slot) = shared.lock() {
        *slot = queue.clone();
    }
    notify();

    let mut control: Option<Arc<Control>> = None;
    // Commands that arrived while an archive streamed and belong to the queue loop: an Add for
    // another mod, a Retry, a Remove, a Pause of a waiting row.
    let mut deferred: Vec<Command> = Vec::new();
    loop {
        match rx.recv_timeout(IDLE) {
            Ok(command) => match apply(&command, &mut queue, &shared, &mut control) {
                Flow::Continue => {}
                Flow::Stop => {
                    let _ = store_queue(&data_dir, &queue);
                    return;
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(command) = rx.try_recv() {
            if apply(&command, &mut queue, &shared, &mut control) == Flow::Stop {
                let _ = store_queue(&data_dir, &queue);
                return;
            }
        }
        publish(&shared, &queue);
        let Some(id) = queue.next_runnable() else {
            continue;
        };
        run_item(
            &data_dir,
            &workspace,
            &shared,
            &rx,
            &mut queue,
            &id,
            &mut control,
            &wake,
            &mut deferred,
        );
        for command in std::mem::take(&mut deferred) {
            if apply(&command, &mut queue, &shared, &mut control) == Flow::Stop {
                let _ = store_queue(&data_dir, &queue);
                return;
            }
        }
        publish(&shared, &queue);
        let _ = store_queue(&data_dir, &queue);
        notify();
    }
}

#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

fn apply(
    command: &Command,
    queue: &mut Queue,
    shared: &Arc<Mutex<Queue>>,
    control: &mut Option<Arc<Control>>,
) -> Flow {
    #[allow(unused_assignments)]
    let mut changed = false;
    match command {
        Command::Add(target) => {
            match queue.add(target) {
                Ok(id) => {
                    if let Some(item) = queue.get_mut(&id) {
                        item.error = None;
                    }
                    changed = true;
                }
                Err(error) => {
                    // A bad target is reported on the row the player would have got.
                    let id = format!("bad{}", queue.items.len() + 1);
                    let mut item = Item::new(id.clone(), target.clone());
                    item.state = State::Failed;
                    item.error = Some(error.to_string());
                    item.finished_at = Some(Utc::now().to_rfc3339());
                    queue.items.push(item);
                    changed = true;
                }
            }
        }
        Command::Retry(id) => changed = queue.retry(id),
        Command::Pause(id) => {
            changed = queue.pause(id);
            if let Some(control) = control
                && queue.get(id).is_some_and(|item| item.state == State::Paused)
            {
                control.pause(true);
            }
        }
        Command::Resume(id) => {
            changed = queue.resume(id);
            if let Some(control) = control {
                control.pause(false);
            }
        }
        Command::Cancel(id) => {
            changed = queue.cancel(id);
            if let Some(control) = control {
                control.cancel();
            }
        }
        Command::Remove(id) => changed = queue.remove(id),
        Command::ClearFinished => changed = queue.clear_finished(),
        Command::PauseAll(paused) => {
            queue.paused = *paused;
            if let Some(control) = control {
                control.pause(*paused);
            }
            changed = true;
        }
        Command::MoveTo { id, index } => changed = queue.move_to(id, *index).is_ok(),
        Command::Shutdown => return Flow::Stop,
    }
    if changed {
        publish(shared, queue);
    }
    Flow::Continue
}

fn publish(shared: &Arc<Mutex<Queue>>, queue: &Queue) {
    if let Ok(mut slot) = shared.lock() {
        *slot = queue.clone();
    }
}

/// Resolve, stream and scan one item.
#[allow(clippy::too_many_arguments)]
fn run_item(
    _data_dir: &Path,
    workspace: &Path,
    shared: &Arc<Mutex<Queue>>,
    rx: &Receiver<Command>,
    queue: &mut Queue,
    id: &str,
    control: &mut Option<Arc<Control>>,
    wake: &Option<Wake>,
    deferred: &mut Vec<Command>,
) {
    let Some(item) = queue.get(id).cloned() else {
        return;
    };
    let item_control = Control::new();
    *control = Some(Arc::clone(&item_control));

    let set_state = |state: State| {
        if let Ok(mut slot) = shared.lock()
            && let Some(live) = slot.get_mut(id)
        {
            live.state = state;
        }
    };
    set_state(State::Running);
    // The master copy says Running too, or the publish below would show the row waiting again
    // until the first bytes arrive.
    if let Some(live) = queue.get_mut(id) {
        live.state = State::Running;
        live.rate = Rate::default();
    }
    publish(shared, queue);
    if let Some(wake) = wake {
        wake();
    }

    // Resolve the target. A local file needs no download.
    let resolved = match resolve(&item, workspace) {
        Ok(resolved) => resolved,
        Err(error) => {
            if let Some(live) = queue.get_mut(id) {
                live.state = State::Failed;
                live.error = Some(error.to_string());
                live.finished_at = Some(Utc::now().to_rfc3339());
            }
            *control = None;
            return;
        }
    };
    if let Some(live) = queue.get_mut(id) {
        live.name = resolved.name.clone();
        live.mod_id = resolved.mod_id;
        live.url = resolved.url.clone();
        live.file_name = resolved.file_name.clone();
        live.bytes_total = resolved.bytes_total;
        live.path = Some(resolved.path.clone());
        live.attempts += 1;
    }

    let Some(url) = resolved.url.clone() else {
        // A local archive: nothing to fetch. Scan it and finish.
        finish(workspace, shared, queue, id, resolved.path, 0);
        *control = None;
        return;
    };

    let policy = match DownloadPolicy::from_environment() {
        Ok(policy) => policy,
        Err(error) => {
            if let Some(live) = queue.get_mut(id) {
                live.state = State::Failed;
                live.error = Some(error.to_string());
            }
            *control = None;
            return;
        }
    };

    // Progress, pause, cancel and new commands all flow through this closure.
    let progress_shared = Arc::clone(shared);
    let progress_control = Arc::clone(&item_control);
    let progress_wake = wake.clone();
    let pending = &mut *deferred;
    let mut last_wake: Option<Instant> = None;
    // One clock for the whole transfer: speed is bytes over time since this call, so the
    // window never spans the resolve that came before it.
    let started = Instant::now();
    let mut progress = move |done: u64, total: Option<u64>| {
        // Commands that arrive mid-stream are handled here so Pause and Cancel act at once;
        // the rest wait for the queue loop instead of being lost.
        while let Ok(command) = rx.try_recv() {
            if handle_mid_stream(&command, id, &progress_control) {
                pending.push(command);
            }
        }
        if let Ok(mut slot) = progress_shared.lock()
            && let Some(live) = slot.get_mut(id)
        {
            live.bytes_done = done;
            live.rate.observe(done, started.elapsed());
            if let Some(server) = progress_control.server() {
                live.server = Some(server);
            }
            if let Some(total) = total.filter(|total| *total > 0) {
                live.bytes_total = total;
            }
            live.state = if progress_control.paused() {
                State::Paused
            } else {
                State::Running
            };
        }
        // A few repaints a second, so a progress bar moves without a frame per chunk.
        if last_wake.is_none_or(|at| at.elapsed() >= PROGRESS_WAKE)
            && let Some(wake) = &progress_wake
        {
            last_wake = Some(Instant::now());
            wake();
        }
    };

    // A part file holds the bytes a pause or a failure left; ask for the rest.
    let part = part_path(&resolved.path);
    let from = if part.is_file() {
        part.metadata().map(|meta| meta.len()).unwrap_or(0)
    } else {
        0
    };

    // The server list costs one GameBanana request, so it is only fetched after a failure.
    let mut servers = healthy_servers;
    let outcome = download_with_failover(
        &url,
        &resolved.path,
        &policy,
        from,
        &item_control,
        &mut servers,
        &mut progress,
    );
    // Commands left over after the stream finished.
    while let Ok(command) = rx.try_recv() {
        if handle_mid_stream(&command, id, &item_control) {
            deferred.push(command);
        }
    }
    *control = None;

    match outcome {
        Ok(fetched) => {
            let bytes = fetched.bytes;
            if let Some(live) = queue.get_mut(id) {
                live.server = fetched.server.clone().or_else(|| item_control.server());
            }
            finish(workspace, shared, queue, id, fetched.path, bytes);
        }
        Err(error) => {
            let cancelled = item_control.cancelled();
            if let Some(live) = queue.get_mut(id) {
                if cancelled {
                    live.state = State::Cancelled;
                    live.error = None;
                    live.bytes_done = 0;
                } else {
                    live.state = State::Failed;
                    live.error = Some(error.to_string());
                }
                live.finished_at = Some(Utc::now().to_rfc3339());
            }
            if cancelled {
                let _ = fs::remove_file(part_path(&resolved.path));
            }
        }
    }
}

/// Commands that are safe to honour while archive `id` is streaming. True when the command
/// also needs the queue loop, which owns the item list: an Add or a Retry, a command for
/// another row, a whole-queue pause and a shutdown.
fn handle_mid_stream(command: &Command, id: &str, control: &Control) -> bool {
    match command {
        Command::Pause(target) if target == id => {
            control.pause(true);
            false
        }
        Command::Resume(target) if target == id => {
            control.pause(false);
            false
        }
        Command::Cancel(target) if target == id => {
            control.cancel();
            false
        }
        Command::Shutdown => {
            control.cancel();
            true
        }
        Command::PauseAll(paused) => {
            control.pause(*paused);
            true
        }
        _ => true,
    }
}

fn finish(
    workspace: &Path,
    shared: &Arc<Mutex<Queue>>,
    queue: &mut Queue,
    id: &str,
    path: PathBuf,
    bytes: u64,
) {
    let scan = scan_archive(&path, &workspace.join(SCAN_DIR_NAME));
    let bytes_done = if bytes > 0 {
        bytes
    } else {
        fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0)
    };
    if let Some(live) = queue.get_mut(id) {
        live.state = State::Done;
        live.path = Some(path);
        live.bytes_done = bytes_done;
        if live.bytes_total == 0 {
            live.bytes_total = bytes_done;
        }
        live.error = None;
        live.scan = scan;
        live.finished_at = Some(Utc::now().to_rfc3339());
    }
    publish(shared, queue);
}

/// A mod id, link or path resolved into something downloadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub name: String,
    pub mod_id: Option<u64>,
    pub url: Option<String>,
    pub file_name: String,
    pub bytes_total: u64,
    pub path: PathBuf,
}

/// Turn a target into a URL and a destination under `<workspace>/downloads`.
fn resolve(item: &Item, workspace: &Path) -> Result<Resolved> {
    let source = InstallSource::parse(&item.target)?;
    let directory = install::download_dir(workspace, &source);
    match source {
        InstallSource::LocalFile(path) => {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "archive".to_string());
            Ok(Resolved {
                name: stem(&file_name),
                mod_id: None,
                url: None,
                file_name,
                bytes_total: fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                path,
            })
        }
        InstallSource::GameBananaId(id) => {
            let file: GameBananaFile = install::fetch_gamebanana_file(id)?;
            let file_name = safe_file_name(&file.file_name)
                .ok_or_else(|| Error::msg(format!("GameBanana returned an odd file name: {:?}", file.file_name)))?;
            Ok(Resolved {
                name: stem(&file_name),
                mod_id: Some(id),
                url: Some(file.download_url),
                file_name: file_name.clone(),
                bytes_total: file.size_bytes.unwrap_or(0),
                path: directory.join(file_name),
            })
        }
        InstallSource::DirectUrl(url) => {
            let file_name = safe_file_name(WebUrl::parse(&url)?.file_name().as_deref().unwrap_or("archive"))
                .ok_or_else(|| Error::msg("The link has no usable file name."))?;
            Ok(Resolved {
                name: stem(&file_name),
                mod_id: None,
                url: Some(url),
                file_name: file_name.clone(),
                bytes_total: 0,
                path: directory.join(file_name),
            })
        }
    }
}

fn stem(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| file_name.to_string())
        .replace(['_', '-'], " ")
        .trim()
        .to_string()
}

/// GameBanana's healthy file servers, best first. One network call per worker, and only for a
/// download that is already running.
fn healthy_servers() -> Vec<String> {
    match file_servers::fetch_directory() {
        Ok(directory) => directory
            .servers
            .into_iter()
            .filter(|server| server.up)
            .map(|server| server.name)
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
