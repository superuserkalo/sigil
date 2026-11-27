//! Sound sample playback on a worker thread.
//!
//! Rune has no audio surface, so the preview player lives here: one thread owns the output
//! device and the playing sample. The UI only sends commands and reads the state it reports,
//! so a slow download or a missing device never stalls a frame.
//!
//! What makes a press fast:
//! - The output device opens once, on the first press, while the first download runs.
//! - A sample starts as soon as its first frames arrive: a download thread fills a shared
//!   buffer, a decode thread reads it as it grows and hands decoded chunks to the output.
//! - Finished downloads stay in memory and on disk (`cache/sounds`), so a second press plays
//!   without the network, also after a restart.
//! - One HTTP agent is kept, so the next sample reuses the open GameBanana connection.
//! - Opening a sound's details fetches its sample ahead (one at a time, never played).
//!
//! One sample plays at a time. Pressing Play on another card stops the first, which is what
//! a preview should do; pressing the loading or playing one again stops it.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The most a sample may download. GameBanana previews are well under this.
const MAX_SAMPLE_BYTES: u64 = 12 * 1024 * 1024;
/// Downloaded samples the worker keeps in memory.
const MEMORY_CACHE_BYTES: usize = 32 * 1024 * 1024;
/// The disk copy of downloaded samples. The least recently played go first past this.
const DISK_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Decoded frames per chunk handed to the output (about 46 ms at 44.1 kHz).
const CHUNK_FRAMES: usize = 2048;
/// Chunks decoded ahead of the output (about 1.5 s at 44.1 kHz), so memory stays small.
const CHUNKS_AHEAD: usize = 32;
/// Downloads in flight at once. A sample left behind by a new press keeps downloading into
/// the cache (a press back on it is then instant); past this many the oldest is cancelled.
const MAX_DOWNLOADS: usize = 3;
/// How often the worker looks whether the playing sample has ended.
const END_CHECK: Duration = Duration::from_millis(150);

pub(crate) enum Command {
    Play { id: u64, url: String, volume: f32 },
    /// A sample already in memory (a game clip the Locker read from the game's files).
    PlayBytes { id: u64, bytes: Vec<u8>, volume: f32 },
    /// Download a sample into the cache without playing it.
    Prefetch { url: String },
    Stop,
    Volume(f32),
    /// From a decode thread: the first chunk is decoded and the rest follows.
    Ready { generation: u64, id: u64, source: Stream },
    /// From a decode thread: the sample cannot play.
    Failed { generation: u64, message: String },
    /// From a download thread: the whole sample arrived.
    Fetched { url: String, bytes: Arc<[u8]> },
}

pub(crate) enum Update {
    Loading(u64),
    Playing(u64),
    /// The sample came to its end by itself.
    Ended(u64),
    Stopped,
    Failed(String),
}

/// The sample player, owned by the Browse view. The worker starts on the first Play.
pub struct Player {
    tx: Option<Sender<Command>>,
    rx: Receiver<Update>,
    /// Wakes the UI when the worker reports, so the button changes without input.
    pub wake: Option<rune::Waker>,
    /// The sample now playing, by submission id.
    pub playing: Option<u64>,
    /// The sample being fetched or decoded.
    pub loading: Option<u64>,
    pub volume: f32,
    /// Why the last sample could not start, shown under the card row.
    pub error: Option<String>,
    /// The last sample asked ahead, so a frame never asks twice.
    prefetched: Option<String>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("playing", &self.playing)
            .field("loading", &self.loading)
            .field("volume", &self.volume)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Default for Player {
    fn default() -> Self {
        let (_tx, rx) = mpsc::channel();
        Player {
            tx: None,
            rx,
            wake: None,
            playing: None,
            loading: None,
            volume: 0.7,
            error: None,
            prefetched: None,
        }
    }
}

impl Player {
    fn worker(&mut self) -> Option<&Sender<Command>> {
        if self.tx.is_none() {
            let (tx, commands) = mpsc::channel::<Command>();
            let (updates, rx) = mpsc::channel::<Update>();
            self.rx = rx;
            let worker = Worker::new(
                tx.clone(),
                updates,
                self.wake.clone(),
                Some(crate::data::data_dir().join("cache").join("sounds")),
            );
            if std::thread::Builder::new()
                .name("sigil-audio".to_string())
                .spawn(move || worker.run(commands))
                .is_err()
            {
                self.error = Some("The preview player did not start.".to_string());
                self.loading = None;
                return None;
            }
            self.tx = Some(tx);
        }
        self.tx.as_ref()
    }

    /// The state a press shows at once, before the worker has done anything: the button
    /// turns into a loader on the next frame.
    fn begin(&mut self, id: u64) {
        self.error = None;
        self.loading = Some(id);
        self.playing = None;
    }

    /// True when a press on `id` stops it (it plays or loads) instead of starting it.
    fn stops(&self, id: u64) -> bool {
        self.playing == Some(id) || self.loading == Some(id)
    }

    /// Play one sample, or stop it when it is already the one playing or loading.
    pub fn toggle(&mut self, id: u64, url: String) {
        if self.stops(id) {
            self.stop();
            return;
        }
        if rune::is_headless() {
            return;
        }
        self.begin(id);
        let volume = self.volume;
        if let Some(tx) = self.worker() {
            let _ = tx.send(Command::Play { id, url, volume });
        }
    }

    /// Play a sample already in memory, or stop it when it is the one playing. The Locker's
    /// game sound browser reads a clip on its own worker and hands the bytes here, so every
    /// preview shares this one player and the status bar's volume.
    pub fn toggle_bytes(&mut self, id: u64, bytes: Vec<u8>) {
        if self.playing == Some(id) {
            self.stop();
            return;
        }
        if rune::is_headless() {
            return;
        }
        self.begin(id);
        let volume = self.volume;
        if let Some(tx) = self.worker() {
            let _ = tx.send(Command::PlayBytes { id, bytes, volume });
        }
    }

    /// Fetch a sample into the cache ahead of a press (an open details panel). Nothing plays.
    pub fn prefetch(&mut self, url: &str) {
        if self.prefetched.as_deref() == Some(url) || rune::is_headless() {
            return;
        }
        self.prefetched = Some(url.to_string());
        if let Some(tx) = self.worker() {
            let _ = tx.send(Command::Prefetch { url: url.to_string() });
        }
    }

    pub fn stop(&mut self) {
        self.playing = None;
        self.loading = None;
        if let Some(tx) = self.tx.as_ref() {
            let _ = tx.send(Command::Stop);
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(tx) = self.tx.as_ref() {
            let _ = tx.send(Command::Volume(self.volume));
        }
    }

    /// Apply what the worker reported. True when the view must run again.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(update) => {
                    self.apply(update);
                    changed = true;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Loading(id) => {
                self.loading = Some(id);
                self.playing = None;
                self.error = None;
            }
            Update::Playing(id) => {
                self.loading = None;
                self.playing = Some(id);
                self.error = None;
            }
            Update::Ended(id) => {
                if self.playing == Some(id) {
                    self.playing = None;
                }
            }
            Update::Stopped => {
                self.loading = None;
                self.playing = None;
            }
            Update::Failed(message) => {
                self.loading = None;
                self.playing = None;
                self.error = Some(message);
            }
        }
    }
}

/// A download in flight: the one the player pressed, or one asked ahead.
struct Download {
    url: String,
    buf: Arc<Shared>,
    pressed: bool,
}

struct Worker {
    /// The worker's own command channel, handed to download and decode threads.
    tx: Sender<Command>,
    updates: Sender<Update>,
    wake: Option<rune::Waker>,
    cache_dir: Option<PathBuf>,
    output: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    volume: f32,
    /// Bumped on every press and stop; a decode thread's result for an older one is dropped.
    generation: u64,
    playing: Option<u64>,
    memory: MemoryCache,
    downloads: Vec<Download>,
    agent: Option<(u64, ureq::Agent)>,
}

impl Worker {
    fn new(
        tx: Sender<Command>,
        updates: Sender<Update>,
        wake: Option<rune::Waker>,
        cache_dir: Option<PathBuf>,
    ) -> Self {
        Worker {
            tx,
            updates,
            wake,
            cache_dir,
            output: None,
            player: None,
            volume: 0.7,
            generation: 0,
            playing: None,
            memory: MemoryCache::default(),
            downloads: Vec::new(),
            agent: None,
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            let command = if self.playing.is_some() {
                match commands.recv_timeout(END_CHECK) {
                    Ok(command) => command,
                    Err(RecvTimeoutError::Timeout) => {
                        self.check_end();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match commands.recv() {
                    Ok(command) => command,
                    Err(_) => break,
                }
            };
            self.handle(command);
        }
    }

    fn report(&self, update: Update) {
        let _ = self.updates.send(update);
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Play { id, url, volume } => {
                self.volume = volume;
                self.cancel_current();
                self.report(Update::Loading(id));
                let generation = self.generation;
                match plan(&mut self.memory, self.cache_dir.as_deref(), &self.downloads, &url) {
                    Plan::Cached(bytes) => {
                        self.memory.insert(&url, bytes.clone());
                        self.play_bytes(id, bytes);
                    }
                    plan => {
                        let buf = match plan {
                            Plan::Join(buf) => {
                                for download in self.downloads.iter_mut().filter(|d| d.url == url) {
                                    download.pressed = true;
                                }
                                buf
                            }
                            _ => self.start_download(&url, true),
                        };
                        let tx = self.tx.clone();
                        let hint = hint_of(&url);
                        let spawned = std::thread::Builder::new()
                            .name("sigil-audio-decode".to_string())
                            .spawn(move || stream(generation, id, buf, hint, tx));
                        if spawned.is_err() {
                            self.report(Update::Failed("The preview player did not start.".to_string()));
                            return;
                        }
                        // The device opens while the sample downloads.
                        if let Err(message) = self.ensure_device() {
                            self.generation += 1;
                            self.report(Update::Failed(message));
                        }
                    }
                }
            }
            Command::PlayBytes { id, bytes, volume } => {
                self.volume = volume;
                self.cancel_current();
                self.report(Update::Loading(id));
                self.play_bytes(id, Arc::from(bytes));
            }
            Command::Prefetch { url } => {
                if !matches!(plan(&mut self.memory, self.cache_dir.as_deref(), &self.downloads, &url), Plan::Download) {
                    return;
                }
                // A press always wins over a guess, and only one guess runs at a time.
                self.downloads.retain(|d| !d.buf.settled());
                if self.downloads.iter().any(|d| d.pressed) {
                    return;
                }
                for guess in self.downloads.drain(..) {
                    guess.buf.cancel();
                }
                self.start_download(&url, false);
            }
            Command::Stop => {
                self.cancel_current();
                self.report(Update::Stopped);
            }
            Command::Volume(value) => {
                self.volume = value;
                if let Some(player) = self.player.as_ref() {
                    player.set_volume(value);
                }
            }
            Command::Ready { generation, id, source } => {
                // Dropping a stale source ends its decode thread.
                if generation != self.generation {
                    return;
                }
                if let Err(message) = self.ensure_device() {
                    self.report(Update::Failed(message));
                    return;
                }
                if let Some(player) = self.player.as_ref() {
                    player.set_volume(self.volume);
                    player.append(source);
                    player.play();
                    self.playing = Some(id);
                    self.report(Update::Playing(id));
                }
            }
            Command::Failed { generation, message } => {
                if generation == self.generation {
                    self.report(Update::Failed(message));
                }
            }
            Command::Fetched { url, bytes } => {
                self.downloads.retain(|d| d.url != url);
                self.memory.insert(&url, bytes);
            }
        }
    }

    /// Stop what plays and make every decode in flight stale.
    fn cancel_current(&mut self) {
        self.generation += 1;
        self.playing = None;
        if let Some(player) = self.player.as_ref() {
            player.stop();
            player.clear();
        }
    }

    fn check_end(&mut self) {
        let Some(id) = self.playing else { return };
        if self.player.as_ref().is_none_or(rodio::Player::empty) {
            self.playing = None;
            self.report(Update::Ended(id));
        }
    }

    fn play_bytes(&mut self, id: u64, bytes: Arc<[u8]>) {
        if let Err(message) = self.ensure_device() {
            self.report(Update::Failed(message));
            return;
        }
        let Some(player) = self.player.as_ref() else { return };
        match rodio::Decoder::try_from(std::io::Cursor::new(bytes)) {
            Ok(source) => {
                player.set_volume(self.volume);
                player.append(source);
                player.play();
                self.playing = Some(id);
                self.report(Update::Playing(id));
            }
            Err(_) => self.report(Update::Failed("The sample could not be decoded.".to_string())),
        }
    }

    /// Start a download. Past `MAX_DOWNLOADS` in flight, the oldest is cancelled.
    fn start_download(&mut self, url: &str, pressed: bool) -> Arc<Shared> {
        self.downloads.retain(|d| !d.buf.settled());
        while self.downloads.len() >= MAX_DOWNLOADS {
            self.downloads.remove(0).buf.cancel();
        }
        let buf = Arc::new(Shared::default());
        let agent = self.agent();
        let tx = self.tx.clone();
        let cache_dir = self.cache_dir.clone();
        let thread_buf = buf.clone();
        let thread_url = url.to_string();
        let spawned = std::thread::Builder::new()
            .name("sigil-audio-fetch".to_string())
            .spawn(move || download(&agent, thread_url, &thread_buf, cache_dir.as_deref(), &tx));
        if spawned.is_err() {
            buf.fail("The sample could not be downloaded.".to_string());
        }
        self.downloads.push(Download { url: url.to_string(), buf: buf.clone(), pressed });
        buf
    }

    /// One agent for every sample, so the next one reuses the open connection. Rebuilt when
    /// the proxy setting changes.
    fn agent(&mut self) -> ureq::Agent {
        let generation = deadlock_core::net::proxy_generation();
        if let Some((built, agent)) = &self.agent {
            if *built == generation {
                return agent.clone();
            }
        }
        let agent = deadlock_core::net::with_proxy(deadlock_core::net::builder())
            .middleware(deadlock_core::net::gate)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent();
        self.agent = Some((generation, agent.clone()));
        agent
    }

    fn ensure_device(&mut self) -> Result<(), String> {
        ensure_device(&mut self.output, &mut self.player, self.volume)
    }
}

pub(crate) fn ensure_device(
    output: &mut Option<rodio::MixerDeviceSink>,
    player: &mut Option<rodio::Player>,
    volume: f32,
) -> Result<(), String> {
    if player.is_some() {
        return Ok(());
    }
    let sink = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|error| format!("No audio output was found: {error}"))?;
    let connected = rodio::Player::connect_new(sink.mixer());
    connected.set_volume(volume);
    *output = Some(sink);
    *player = Some(connected);
    Ok(())
}

/// Where a press gets its bytes.
pub(crate) enum Plan {
    /// In memory or on disk: no network.
    Cached(Arc<[u8]>),
    /// A download in flight is this sample (asked ahead, or left by an earlier press): read
    /// it as it arrives.
    Join(Arc<Shared>),
    Download,
}

fn plan(
    memory: &mut MemoryCache,
    cache_dir: Option<&Path>,
    downloads: &[Download],
    url: &str,
) -> Plan {
    if let Some(bytes) = memory.get(url) {
        return Plan::Cached(bytes);
    }
    if let Some(bytes) = cache_dir.and_then(|dir| disk_read(dir, url)) {
        return Plan::Cached(bytes);
    }
    match downloads.iter().find(|d| d.url == url && !d.buf.failed()) {
        Some(download) => Plan::Join(download.buf.clone()),
        None => Plan::Download,
    }
}

/// The format hint for the decoder, from the address.
fn hint_of(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    (ext.len() <= 4 && ext.chars().all(|c| c.is_ascii_alphanumeric())).then_some(ext)
}

/// The download thread: fill `buf`, then keep the whole sample in the caches.
fn download(agent: &ureq::Agent, url: String, buf: &Shared, cache_dir: Option<&Path>, tx: &Sender<Command>) {
    match fetch_into(agent, &url, buf) {
        Ok(()) => {
            let bytes = buf.finish();
            if let Some(dir) = cache_dir {
                disk_write(dir, &url, &bytes);
            }
            let _ = tx.send(Command::Fetched { url, bytes });
        }
        Err(message) => buf.fail(message),
    }
}

fn fetch_into(agent: &ureq::Agent, url: &str, buf: &Shared) -> Result<(), String> {
    let mut response = agent
        .get(url)
        .header("User-Agent", deadlock_core::gamebanana::GAMEBANANA_USER_AGENT)
        .call()
        .map_err(|_| "The sample could not be downloaded.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "The sample could not be downloaded (HTTP {}).",
            response.status().as_u16()
        ));
    }
    if response.body().content_length().is_some_and(|len| len > MAX_SAMPLE_BYTES) {
        return Err(TOO_LARGE.to_string());
    }
    let reader = response.body_mut().with_config().limit(MAX_SAMPLE_BYTES + 1).reader();
    copy_capped(reader, buf)
}

const TOO_LARGE: &str = "The sample is too large to preview.";

/// Copy a body into `buf` as it arrives, refusing more than the cap.
fn copy_capped(mut reader: impl Read, buf: &Shared) -> Result<(), String> {
    let mut chunk = [0u8; 16 * 1024];
    loop {
        if buf.cancelled() {
            return Err("Cancelled.".to_string());
        }
        let read = match reader.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err("The sample could not be read.".to_string()),
        };
        buf.push(&chunk[..read])?;
    }
}

/// A download's bytes so far, shared between the download thread and the decode thread.
#[derive(Default)]
pub(crate) struct Shared {
    state: Mutex<SharedState>,
    grew: Condvar,
}

#[derive(Default)]
struct SharedState {
    bytes: Vec<u8>,
    done: bool,
    error: Option<String>,
    cancelled: bool,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, SharedState> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Add bytes. Past the cap the download fails: the body is untrusted.
    fn push(&self, data: &[u8]) -> Result<(), String> {
        let mut state = self.lock();
        if state.cancelled {
            return Err("Cancelled.".to_string());
        }
        if (state.bytes.len() + data.len()) as u64 > MAX_SAMPLE_BYTES {
            state.error = Some(TOO_LARGE.to_string());
            self.grew.notify_all();
            return Err(TOO_LARGE.to_string());
        }
        state.bytes.extend_from_slice(data);
        self.grew.notify_all();
        Ok(())
    }

    fn finish(&self) -> Arc<[u8]> {
        let mut state = self.lock();
        state.done = true;
        self.grew.notify_all();
        Arc::from(state.bytes.as_slice())
    }

    fn fail(&self, message: String) {
        let mut state = self.lock();
        if state.error.is_none() {
            state.error = Some(message);
        }
        self.grew.notify_all();
    }

    fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        self.grew.notify_all();
    }

    fn cancelled(&self) -> bool {
        self.lock().cancelled
    }

    fn failed(&self) -> bool {
        let state = self.lock();
        state.error.is_some() || state.cancelled
    }

    fn error(&self) -> Option<String> {
        self.lock().error.clone()
    }

    /// Done, failed or cancelled: nothing more will arrive.
    fn settled(&self) -> bool {
        let state = self.lock();
        state.done || state.error.is_some() || state.cancelled
    }

    /// Block until the download settles; the whole sample, or why not.
    fn wait_all(&self) -> Result<Arc<[u8]>, String> {
        let mut state = self.lock();
        loop {
            if let Some(error) = &state.error {
                return Err(error.clone());
            }
            if state.cancelled {
                return Err("Cancelled.".to_string());
            }
            if state.done {
                return Ok(Arc::from(state.bytes.as_slice()));
            }
            state = self.grew.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Reads a download as it grows, blocking (on the decode thread, never the output) until
/// the bytes it wants arrive.
pub(crate) struct SharedReader {
    shared: Arc<Shared>,
    pos: u64,
}

impl SharedReader {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        SharedReader { shared, pos: 0 }
    }

    /// Wait until `want` bytes exist or the download settles; the length then.
    fn wait_for(&self, want: u64) -> std::io::Result<u64> {
        let mut state = self.shared.lock();
        loop {
            let len = state.bytes.len() as u64;
            if len >= want || state.done {
                return Ok(len);
            }
            if state.cancelled || state.error.is_some() {
                return Err(std::io::Error::other("the sample download stopped"));
            }
            state = self.shared.grew.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

impl Read for SharedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        self.wait_for(self.pos + 1)?;
        let state = self.shared.lock();
        let start = (self.pos as usize).min(state.bytes.len());
        let n = out.len().min(state.bytes.len() - start);
        out[..n].copy_from_slice(&state.bytes[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SharedReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => self.wait_for(u64::MAX)? as i64 + d,
        };
        if target < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start"));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

/// The decode thread: decode the sample as it arrives and hand chunks to the output.
fn stream(generation: u64, id: u64, buf: Arc<Shared>, hint: Option<String>, tx: Sender<Command>) {
    let mut builder = rodio::Decoder::builder()
        .with_data(SharedReader::new(buf.clone()))
        .with_seekable(false);
    if let Some(hint) = hint.as_deref() {
        builder = builder.with_hint(hint);
    }
    let fail = |message: String| {
        let _ = tx.send(Command::Failed { generation, message });
    };
    let started = match builder.build() {
        Ok(decoder) => pump(decoder, generation, id, &tx),
        // Not readable from the front: wait for the whole sample and try once more.
        Err(_) => match buf.wait_all() {
            Ok(bytes) => match rodio::Decoder::try_from(std::io::Cursor::new(bytes)) {
                Ok(decoder) => pump(decoder, generation, id, &tx),
                Err(_) => return fail("The sample could not be decoded.".to_string()),
            },
            Err(message) => return fail(message),
        },
    };
    // A sample that played and then lost its download simply ends early.
    if !started {
        fail(buf.error().unwrap_or_else(|| "The sample could not be decoded.".to_string()));
    }
}

/// Decode chunk by chunk into a bounded queue the output drains. False when nothing decoded.
fn pump<S: rodio::Source<Item = f32>>(mut source: S, generation: u64, id: u64, tx: &Sender<Command>) -> bool {
    let channels = source.channels();
    let rate = source.sample_rate();
    let first = read_chunk(&mut source, channels.get() as usize, CHUNK_FRAMES);
    if first.is_empty() {
        return false;
    }
    let (chunks, queue) = mpsc::sync_channel::<Vec<f32>>(CHUNKS_AHEAD);
    let _ = chunks.send(first);
    let _ = tx.send(Command::Ready { generation, id, source: Stream::new(queue, channels, rate) });
    // A sample that changes format mid-way ends there; previews do not.
    while source.channels() == channels && source.sample_rate() == rate {
        let chunk = read_chunk(&mut source, channels.get() as usize, CHUNK_FRAMES);
        // Empty: the sample ended. A failed send: the output dropped the stream (a new press).
        if chunk.is_empty() || chunks.send(chunk).is_err() {
            break;
        }
    }
    true
}

/// Up to `frames` whole frames.
fn read_chunk(source: &mut impl Iterator<Item = f32>, channels: usize, frames: usize) -> Vec<f32> {
    let mut chunk: Vec<f32> = source.by_ref().take(frames * channels.max(1)).collect();
    chunk.truncate(chunk.len() - chunk.len() % channels.max(1));
    chunk
}

/// The output side of a streamed sample. It never blocks the audio callback: when the
/// decoder falls behind it plays silent frames until the next chunk arrives.
pub(crate) struct Stream {
    chunks: Receiver<Vec<f32>>,
    current: Vec<f32>,
    at: usize,
    silence: usize,
    channels: rodio::ChannelCount,
    rate: rodio::SampleRate,
}

impl Stream {
    pub(crate) fn new(chunks: Receiver<Vec<f32>>, channels: rodio::ChannelCount, rate: rodio::SampleRate) -> Self {
        Stream { chunks, current: Vec::new(), at: 0, silence: 0, channels, rate }
    }
}

impl Iterator for Stream {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(&sample) = self.current.get(self.at) {
                self.at += 1;
                return Some(sample);
            }
            if self.silence > 0 {
                self.silence -= 1;
                return Some(0.0);
            }
            match self.chunks.try_recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.at = 0;
                }
                // One whole silent frame, so the channels stay in step.
                Err(TryRecvError::Empty) => self.silence = self.channels.get() as usize,
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

impl rodio::Source for Stream {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Recently played samples, least recently used first out.
#[derive(Default)]
pub(crate) struct MemoryCache {
    order: VecDeque<String>,
    map: HashMap<String, Arc<[u8]>>,
    bytes: usize,
}

impl MemoryCache {
    pub(crate) fn get(&mut self, url: &str) -> Option<Arc<[u8]>> {
        let bytes = self.map.get(url)?.clone();
        self.order.retain(|u| u != url);
        self.order.push_back(url.to_string());
        Some(bytes)
    }

    pub(crate) fn insert(&mut self, url: &str, bytes: Arc<[u8]>) {
        if bytes.len() > MEMORY_CACHE_BYTES {
            return;
        }
        if let Some(old) = self.map.remove(url) {
            self.bytes -= old.len();
            self.order.retain(|u| u != url);
        }
        self.bytes += bytes.len();
        self.map.insert(url.to_string(), bytes);
        self.order.push_back(url.to_string());
        while self.bytes > MEMORY_CACHE_BYTES {
            let Some(oldest) = self.order.pop_front() else { break };
            if let Some(old) = self.map.remove(&oldest) {
                self.bytes -= old.len();
            }
        }
    }
}

fn disk_name(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{hex}.sample")
}

/// A cached sample, or `None`. A file past the cap is never read.
pub(crate) fn disk_read(dir: &Path, url: &str) -> Option<Arc<[u8]>> {
    let path = dir.join(disk_name(url));
    let file = std::fs::File::open(&path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 || len > MAX_SAMPLE_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(MAX_SAMPLE_BYTES).read_to_end(&mut bytes).ok()?;
    // Recently played files are the last to be pruned.
    if let Ok(file) = std::fs::File::options().write(true).open(&path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
    Some(Arc::from(bytes))
}

/// Keep a downloaded sample, then prune the folder to its budget.
pub(crate) fn disk_write(dir: &Path, url: &str, bytes: &[u8]) {
    if bytes.is_empty() || bytes.len() as u64 > MAX_SAMPLE_BYTES || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join(disk_name(url));
    let part = path.with_extension("part");
    if std::fs::write(&part, bytes).is_err() || std::fs::rename(&part, &path).is_err() {
        let _ = std::fs::remove_file(&part);
        return;
    }
    prune(dir, DISK_CACHE_BYTES);
}

fn prune(dir: &Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "sample"))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-audio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn worker(cache_dir: Option<PathBuf>) -> (Worker, Receiver<Update>) {
        let (tx, _commands) = mpsc::channel();
        let (updates, rx) = mpsc::channel();
        (Worker::new(tx, updates, None, cache_dir), rx)
    }

    const URL: &str = "https://files.gamebanana.com/preview/sounds/a.mp3";

    #[test]
    fn a_press_shows_the_loader_at_once_and_a_second_press_cancels() {
        let mut player = Player::default();
        player.error = Some("old".to_string());
        player.begin(7);
        assert_eq!(player.loading, Some(7));
        assert_eq!(player.playing, None);
        assert_eq!(player.error, None);
        // Pressing the loading one again stops it, with no worker needed.
        player.toggle(7, URL.to_string());
        assert_eq!(player.loading, None);
        assert_eq!(player.playing, None);
    }

    #[test]
    fn worker_reports_move_the_button() {
        let mut player = Player::default();
        player.begin(1);
        player.apply(Update::Playing(1));
        assert_eq!((player.loading, player.playing), (None, Some(1)));
        // An end for another sample leaves this one alone.
        player.apply(Update::Ended(2));
        assert_eq!(player.playing, Some(1));
        // A sample that ends by itself turns back into Play, so one press replays it.
        player.apply(Update::Ended(1));
        assert_eq!(player.playing, None);
        player.apply(Update::Failed("no".to_string()));
        assert_eq!(player.error.as_deref(), Some("no"));
    }

    #[test]
    fn a_finished_download_makes_the_second_play_need_no_network() {
        let (mut worker, _updates) = worker(None);
        assert!(matches!(plan(&mut worker.memory, None, &[], URL), Plan::Download));
        worker.handle(Command::Fetched { url: URL.to_string(), bytes: Arc::from(&b"ID3 sample"[..]) });
        match plan(&mut worker.memory, None, &[], URL) {
            Plan::Cached(bytes) => assert_eq!(&bytes[..], b"ID3 sample"),
            _ => panic!("expected the cache"),
        }
    }

    #[test]
    fn the_disk_copy_serves_a_play_after_a_restart() {
        let dir = scratch("disk");
        disk_write(&dir, URL, b"sample bytes");
        let mut fresh = MemoryCache::default();
        match plan(&mut fresh, Some(&dir), &[], URL) {
            Plan::Cached(bytes) => assert_eq!(&bytes[..], b"sample bytes"),
            _ => panic!("expected the disk copy"),
        }
        assert!(matches!(plan(&mut fresh, Some(&dir), &[], "https://files.gamebanana.com/b.mp3"), Plan::Download));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_press_joins_the_download_asked_ahead() {
        let mut memory = MemoryCache::default();
        let ahead = [Download { url: URL.to_string(), buf: Arc::new(Shared::default()), pressed: false }];
        assert!(matches!(plan(&mut memory, None, &ahead, URL), Plan::Join(_)));
        assert!(matches!(plan(&mut memory, None, &ahead, "https://x/b.mp3"), Plan::Download));
        ahead[0].buf.cancel();
        assert!(matches!(plan(&mut memory, None, &ahead, URL), Plan::Download));
    }

    #[test]
    fn a_new_press_drops_the_stale_stream() {
        let (mut worker, updates) = worker(None);
        let old = worker.generation;
        // A press on another sample (here a stop) makes the first one stale.
        worker.handle(Command::Stop);
        assert!(matches!(updates.try_recv(), Ok(Update::Stopped)));
        let (chunks, queue) = mpsc::sync_channel::<Vec<f32>>(1);
        let channels = rodio::ChannelCount::new(2).unwrap();
        let rate = rodio::SampleRate::new(44_100).unwrap();
        worker.handle(Command::Ready { generation: old, id: 1, source: Stream::new(queue, channels, rate) });
        // Nothing plays, and the decode thread sees its output gone and stops.
        assert!(updates.try_recv().is_err());
        assert!(chunks.send(vec![0.0; 2]).is_err());
        worker.handle(Command::Failed { generation: old, message: "late".to_string() });
        assert!(updates.try_recv().is_err());
    }

    #[test]
    fn cancelling_a_download_wakes_its_reader() {
        let shared = Arc::new(Shared::default());
        shared.push(b"abc").unwrap();
        let mut reader = SharedReader::new(shared.clone());
        let mut out = [0u8; 8];
        assert_eq!(reader.read(&mut out).unwrap(), 3);
        let waiting = std::thread::spawn(move || reader.read(&mut out).is_err());
        std::thread::sleep(Duration::from_millis(20));
        shared.cancel();
        assert!(waiting.join().unwrap());
        assert!(shared.push(b"more").is_err());
    }

    #[test]
    fn the_reader_streams_bytes_as_they_arrive() {
        let shared = Arc::new(Shared::default());
        let mut reader = SharedReader::new(shared.clone());
        let feeder = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                for part in [&b"he"[..], b"llo"] {
                    std::thread::sleep(Duration::from_millis(5));
                    shared.push(part).unwrap();
                }
                shared.finish();
            })
        };
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        feeder.join().unwrap();
        assert_eq!(all, b"hello");
        assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), 3);
    }

    #[test]
    fn samples_past_the_cap_are_refused() {
        let shared = Shared::default();
        let body = std::io::repeat(0).take(MAX_SAMPLE_BYTES + 1);
        assert_eq!(copy_capped(body, &shared), Err(TOO_LARGE.to_string()));
        assert_eq!(shared.error().as_deref(), Some(TOO_LARGE));
        let shared = Shared::default();
        assert!(copy_capped(std::io::repeat(1).take(1024), &shared).is_ok());
        // A cached file past the cap is never read back.
        let dir = scratch("cap");
        std::fs::create_dir_all(&dir).unwrap();
        let file = std::fs::File::create(dir.join(disk_name(URL))).unwrap();
        file.set_len(MAX_SAMPLE_BYTES + 1).unwrap();
        assert!(disk_read(&dir, URL).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stream_plays_silent_frames_while_the_decoder_catches_up() {
        let (chunks, queue) = mpsc::sync_channel::<Vec<f32>>(4);
        let channels = rodio::ChannelCount::new(2).unwrap();
        let rate = rodio::SampleRate::new(44_100).unwrap();
        let mut stream = Stream::new(queue, channels, rate);
        chunks.send(vec![0.5, -0.5]).unwrap();
        assert_eq!((stream.next(), stream.next()), (Some(0.5), Some(-0.5)));
        // Nothing decoded yet: a whole silent frame, never half of one.
        assert_eq!((stream.next(), stream.next()), (Some(0.0), Some(0.0)));
        chunks.send(vec![0.25, 0.25]).unwrap();
        assert_eq!((stream.next(), stream.next()), (Some(0.25), Some(0.25)));
        drop(chunks);
        assert_eq!(stream.next(), None);
    }

    #[test]
    fn chunks_hold_whole_frames() {
        let mut samples = (0..7).map(|n| n as f32);
        assert_eq!(read_chunk(&mut samples, 2, 2), vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(read_chunk(&mut samples, 2, 2), vec![4.0, 5.0]);
        assert!(read_chunk(&mut samples, 2, 2).is_empty());
    }

    #[test]
    fn the_memory_cache_stays_in_budget() {
        let mut memory = MemoryCache::default();
        let big: Arc<[u8]> = Arc::from(vec![0u8; MEMORY_CACHE_BYTES / 2 + 1]);
        memory.insert("a", big.clone());
        memory.insert("b", big.clone());
        assert!(memory.get("a").is_none());
        assert!(memory.get("b").is_some());
        assert!(memory.bytes <= MEMORY_CACHE_BYTES);
    }

    #[test]
    fn the_disk_cache_prunes_the_oldest_first() {
        let dir = scratch("prune");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, age) in [("old", 30), ("mid", 20), ("new", 10)] {
            let path = dir.join(format!("{name}.sample"));
            std::fs::write(&path, [0u8; 100]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(std::time::SystemTime::now() - Duration::from_secs(age)).unwrap();
        }
        prune(&dir, 200);
        assert!(!dir.join("old.sample").exists());
        assert!(dir.join("mid.sample").exists() && dir.join("new.sample").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hint_comes_from_the_address() {
        assert_eq!(hint_of(URL).as_deref(), Some("mp3"));
        assert_eq!(hint_of("https://x/a.OGG?x=1").as_deref(), Some("ogg"));
        assert_eq!(hint_of("https://x/noext"), None);
    }

    /// Press-to-sound on this machine against real GameBanana previews. Needs the network and
    /// an output device: `cargo test -p dah-desktop --bin dah-desktop audio_timings -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn audio_timings() {
        let urls = [
            "https://files.gamebanana.com/preview/sounds/ult_62ccc.mp3",
            "https://files.gamebanana.com/preview/sounds/melee_parry_success_f517f.mp3",
            "https://files.gamebanana.com/preview/sounds/punkgoat_blasted_lp125.mp3",
            "https://files.gamebanana.com/preview/sounds/portishead-glory-box-live-instrumen.mp3",
        ];
        let dir = scratch("timings");
        let start = |dir: &Path| {
            let (tx, commands) = mpsc::channel();
            let (updates, rx) = mpsc::channel();
            let worker = Worker::new(tx.clone(), updates, None, Some(dir.to_path_buf()));
            std::thread::spawn(move || worker.run(commands));
            (tx, rx)
        };
        let press = |tx: &Sender<Command>, rx: &Receiver<Update>, id: u64, url: &str| {
            let at = std::time::Instant::now();
            tx.send(Command::Play { id, url: url.to_string(), volume: 0.0 }).unwrap();
            loop {
                match rx.recv().unwrap() {
                    Update::Playing(got) if got == id => return at.elapsed(),
                    Update::Failed(message) => panic!("{message}"),
                    _ => {}
                }
            }
        };
        let (tx, rx) = start(&dir);
        for (n, url) in urls.iter().enumerate() {
            eprintln!("cold   {url}: {:?}", press(&tx, &rx, n as u64, url));
        }
        std::thread::sleep(Duration::from_secs(2));
        for (n, url) in urls.iter().enumerate() {
            eprintln!("memory {url}: {:?}", press(&tx, &rx, 10 + n as u64, url));
        }
        drop((tx, rx));
        let (tx, rx) = start(&dir);
        for (n, url) in urls.iter().enumerate() {
            eprintln!("disk   {url}: {:?}", press(&tx, &rx, 20 + n as u64, url));
        }
        // Details opened 1.5 s before the press, and a press right after opening (joins).
        for (n, (url, rest)) in [
            ("https://files.gamebanana.com/preview/sounds/lash_death_slam_crowd_cheer.mp3", 1500),
            ("https://files.gamebanana.com/preview/sounds/train-half-life2.mp3", 0),
        ]
        .into_iter()
        .enumerate()
        {
            tx.send(Command::Prefetch { url: url.to_string() }).unwrap();
            std::thread::sleep(Duration::from_millis(rest));
            eprintln!("ahead {rest}ms {url}: {:?}", press(&tx, &rx, 30 + n as u64, url));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

