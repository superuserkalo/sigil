//! Sound sample playback on a worker thread.
//!
//! Rune has no audio surface, so the preview player lives here: one thread owns the output
//! stream, downloads a sample, decodes it and plays it. The UI only sends commands and reads
//! the state it reports, so a slow download or a missing device never stalls a frame.
//!
//! One sample plays at a time. Pressing Play on another card stops the first, which is what
//! a preview should do.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// The most a sample may download. GameBanana previews are well under this.
const MAX_SAMPLE_BYTES: u64 = 12 * 1024 * 1024;

pub(crate) enum Command {
    Play { id: u64, url: String, volume: f32 },
    Stop,
    Volume(f32),
}

pub(crate) enum Update {
    Loading(u64),
    Playing(u64),
    Stopped,
    Failed(String),
}

/// The sample player, owned by the Browse view. The worker starts on the first Play.
#[derive(Debug)]
pub struct Player {
    tx: Option<Sender<Command>>,
    rx: Receiver<Update>,
    /// The sample now playing, by submission id.
    pub playing: Option<u64>,
    /// The sample being fetched or decoded.
    pub loading: Option<u64>,
    pub volume: f32,
    /// Why the last sample could not start, shown under the card row.
    pub error: Option<String>,
}

impl Default for Player {
    fn default() -> Self {
        let (_tx, rx) = mpsc::channel();
        Player {
            tx: None,
            rx,
            playing: None,
            loading: None,
            volume: 0.7,
            error: None,
        }
    }
}

impl Player {
    fn worker(&mut self) -> Option<&Sender<Command>> {
        if self.tx.is_none() {
            let (tx, commands) = mpsc::channel::<Command>();
            let (updates, rx) = mpsc::channel::<Update>();
            self.rx = rx;
            if std::thread::Builder::new()
                .name("sigil-audio".to_string())
                .spawn(move || run(commands, updates))
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

    /// Play one sample, or stop it when it is already the one playing.
    pub fn toggle(&mut self, id: u64, url: String) {
        if self.playing == Some(id) {
            self.stop();
            return;
        }
        if rune::is_headless() {
            return;
        }
        self.error = None;
        self.loading = Some(id);
        self.playing = None;
        let volume = self.volume;
        if let Some(tx) = self.worker() {
            let _ = tx.send(Command::Play { id, url, volume });
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
                Ok(Update::Loading(id)) => {
                    self.loading = Some(id);
                    self.playing = None;
                    self.error = None;
                    changed = true;
                }
                Ok(Update::Playing(id)) => {
                    self.loading = None;
                    self.playing = Some(id);
                    self.error = None;
                    changed = true;
                }
                Ok(Update::Stopped) => {
                    self.loading = None;
                    self.playing = None;
                    changed = true;
                }
                Ok(Update::Failed(message)) => {
                    self.loading = None;
                    self.playing = None;
                    self.error = Some(message);
                    changed = true;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }
}

pub(crate) fn run(commands: Receiver<Command>, updates: Sender<Update>) {
    use rodio::{Decoder, Player as RodioPlayer};

    let mut output: Option<rodio::MixerDeviceSink> = None;
    let mut player: Option<RodioPlayer> = None;
    let mut volume;

    while let Ok(command) = commands.recv() {
        match command {
            Command::Play { id, url, volume: wanted } => {
                volume = wanted;
                let _ = updates.send(Update::Loading(id));
                if let Err(message) = ensure_device(&mut output, &mut player, volume) {
                    let _ = updates.send(Update::Failed(message));
                    continue;
                }
                let bytes = match fetch_sample(&url) {
                    Ok(bytes) => bytes,
                    Err(message) => {
                        let _ = updates.send(Update::Failed(message));
                        continue;
                    }
                };
                let Some(player) = player.as_ref() else {
                    let _ = updates.send(Update::Failed(
                        "No audio output was found, so the sample could not play.".to_string(),
                    ));
                    continue;
                };
                player.stop();
                player.clear();
                player.set_volume(volume);
                match Decoder::try_from(std::io::Cursor::new(bytes)) {
                    Ok(source) => {
                        player.append(source);
                        player.play();
                        let _ = updates.send(Update::Playing(id));
                    }
                    Err(_) => {
                        let _ = updates.send(Update::Failed(
                            "The sample could not be decoded.".to_string(),
                        ));
                    }
                }
            }
            Command::Stop => {
                if let Some(player) = player.as_ref() {
                    player.stop();
                    player.clear();
                }
                let _ = updates.send(Update::Stopped);
            }
            Command::Volume(value) => {
                volume = value;
                if let Some(player) = player.as_ref() {
                    player.set_volume(volume);
                }
            }
        }
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

pub(crate) fn fetch_sample(url: &str) -> Result<Vec<u8>, String> {
    let agent = deadlock_core::net::with_proxy(deadlock_core::net::builder())
        .middleware(deadlock_core::net::gate)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();
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
    response
        .body_mut()
        .with_config()
        .limit(MAX_SAMPLE_BYTES + 1)
        .read_to_vec()
        .map_err(|_| "The sample could not be read.".to_string())
        .and_then(|bytes| {
            if bytes.len() as u64 > MAX_SAMPLE_BYTES {
                Err("The sample is too large to preview.".to_string())
            } else {
                Ok(bytes)
            }
        })
}
