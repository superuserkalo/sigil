//! Discord Rich Presence over Discord's local IPC, written by hand (no crate).
//!
//! The desktop Discord client listens on `discord-ipc-0` (up to `-9`): a named pipe
//! `\\?\pipe\discord-ipc-N` on Windows, a Unix socket in `$XDG_RUNTIME_DIR`, `$TMPDIR`, `$TMP`,
//! `$TEMP` or `/tmp` elsewhere (plus the Flatpak and Snap subfolders). A frame is a
//! little-endian `u32` opcode, a little-endian `u32` length and that many bytes of JSON.
//! Sigil sends a HANDSHAKE (op 0) with the application id, then SET_ACTIVITY commands (op 1),
//! and CLOSE (op 2) when presence is turned off.
//!
//! This never opens a network connection. Discord itself shows the activity to the user's
//! friends, which is why the setting is off by default and the details can be hidden.
//!
//! Setting up the application id: sign in at <https://discord.com/developers/applications>,
//! choose New Application, name it "Sigil" (Discord shows that name as "Playing Sigil"), copy
//! the Application ID from General Information into [`APP_ID`], and upload the Sigil logo
//! under Rich Presence, Art Assets, with the key `sigil`. The id is public, not a secret.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// TODO(owner): register "Sigil" at https://discord.com/developers/applications and paste its
/// Application ID here. Until then presence reports "not configured" and sends nothing.
/// `SIGIL_DISCORD_APP_ID` in the environment overrides it for testing with your own app.
pub const APP_ID: &str = "";

/// The art asset key uploaded under Rich Presence, Art Assets.
pub const LOGO_KEY: &str = "sigil";

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;

/// Discord accepts five activity updates per 20 seconds.
const MIN_INTERVAL: Duration = Duration::from_millis(4500);
/// How long to wait between attempts while Discord is not running.
const RETRY: Duration = Duration::from_secs(20);

/// The application id in use, or None when it is not configured.
pub fn app_id() -> Option<String> {
    std::env::var("SIGIL_DISCORD_APP_ID")
        .ok()
        .filter(|id| valid_id(id))
        .or_else(|| valid_id(APP_ID).then(|| APP_ID.to_string()))
}

fn valid_id(id: &str) -> bool {
    (15..=22).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// One frame on the wire.
pub fn encode(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// The opcode and JSON of one complete frame, or None while the bytes are short or bad.
#[cfg(test)]
pub fn decode(bytes: &[u8]) -> Option<(u32, Value, usize)> {
    if bytes.len() < 8 {
        return None;
    }
    let op = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let len = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    if len > 64 * 1024 || bytes.len() < 8 + len {
        return None;
    }
    let value = serde_json::from_slice(&bytes[8..8 + len]).ok()?;
    Some((op, value, 8 + len))
}

pub fn handshake(app_id: &str) -> Value {
    json!({ "v": 1, "client_id": app_id })
}

/// What Sigil reports. Built by the app from its state; `None` lines are left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub details: String,
    pub state: Option<String>,
    /// Unix seconds the elapsed timer counts from.
    pub since: u64,
}

pub fn set_activity(pid: u32, activity: Option<&Activity>, nonce: u64) -> Value {
    let activity = activity.map(|a| {
        let mut v = json!({
            "details": a.details,
            "timestamps": { "start": a.since },
            "assets": { "large_image": LOGO_KEY, "large_text": "Sigil" },
            "instance": false,
        });
        if let Some(state) = &a.state {
            v["state"] = json!(state);
        }
        v
    });
    json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": activity },
        "nonce": nonce.to_string(),
    })
}

/// Where the Discord client may be listening, in the order to try.
pub fn candidates(env: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for n in 0..10 {
            out.push(format!(r"\\?\pipe\discord-ipc-{n}"));
        }
        return out;
    }
    let mut dirs: Vec<String> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(|name| env(name))
        .filter(|d| !d.is_empty())
        .map(|d| d.trim_end_matches('/').to_string())
        .collect();
    dirs.push("/tmp".to_string());
    let mut roots = Vec::new();
    for dir in dirs {
        for sub in ["", "/app/com.discordapp.Discord", "/snap.discord"] {
            let root = format!("{dir}{sub}");
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    for n in 0..10 {
        for root in &roots {
            out.push(format!("{root}/discord-ipc-{n}"));
        }
    }
    out
}

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

fn connect() -> Option<(Box<dyn Pipe>, String)> {
    for path in candidates(|name| std::env::var(name).ok()) {
        #[cfg(unix)]
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(&path) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            return Some((Box::new(stream), path));
        }
        #[cfg(windows)]
        if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            return Some((Box::new(file), path));
        }
    }
    None
}

fn read_frame(pipe: &mut dyn Pipe) -> Option<(u32, Value)> {
    let mut head = [0u8; 8];
    pipe.read_exact(&mut head).ok()?;
    let len = u32::from_le_bytes(head[4..8].try_into().ok()?) as usize;
    if len > 64 * 1024 {
        return None;
    }
    let mut body = vec![0u8; len];
    pipe.read_exact(&mut body).ok()?;
    Some((
        u32::from_le_bytes(head[0..4].try_into().ok()?),
        serde_json::from_slice(&body).ok()?,
    ))
}

/// What the worker reports back, for the Settings status line.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Off,
    NotConfigured,
    Waiting,
    Connected,
}

enum Msg {
    Show(Option<Activity>),
    Stop,
}

/// The handle the app keeps. Dropping it stops the worker and clears the presence.
#[derive(Default)]
pub struct Presence {
    tx: Option<Sender<Msg>>,
    status_rx: Option<Receiver<Status>>,
    pub status: Status,
    last: Option<Option<Activity>>,
}

impl Presence {
    pub fn running(&self) -> bool {
        self.tx.is_some()
    }

    /// Start the worker if needed and hand it the activity. Repeats are not sent.
    pub fn show(&mut self, activity: Option<Activity>, wake: Option<rune::Waker>) {
        if self.tx.is_none() {
            let Some(id) = app_id() else {
                self.status = Status::NotConfigured;
                return;
            };
            if rune::is_headless() {
                self.status = Status::Waiting;
                return;
            }
            let (tx, rx) = mpsc::channel();
            let (status_tx, status_rx) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("sigil-discord".into())
                .spawn(move || worker(id, rx, status_tx, wake));
            if spawned.is_err() {
                return;
            }
            self.tx = Some(tx);
            self.status_rx = Some(status_rx);
            self.status = Status::Waiting;
            self.last = None;
        }
        if self.last.as_ref() == Some(&activity) {
            return;
        }
        self.last = Some(activity.clone());
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Show(activity));
        }
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Stop);
        }
        self.status_rx = None;
        self.last = None;
        self.status = Status::Off;
    }

    /// Take status changes from the worker. True when the status changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.status_rx {
            while let Ok(status) = rx.try_recv() {
                changed |= status != self.status;
                self.status = status;
            }
        }
        changed
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker(id: String, rx: Receiver<Msg>, status: Sender<Status>, wake: Option<rune::Waker>) {
    let tell = |s: Status| {
        let _ = status.send(s);
        if let Some(wake) = &wake {
            wake();
        }
    };
    let pid = std::process::id();
    let mut pipe: Option<Box<dyn Pipe>> = None;
    let mut want: Option<Activity> = None;
    let mut dirty = true;
    let mut last_sent: Option<Instant> = None;
    let mut last_try: Option<Instant> = None;
    let mut nonce = 0u64;
    loop {
        // Wait for a change, or wake to retry and to flush a throttled update.
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Msg::Show(activity)) => {
                want = activity;
                dirty = true;
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                if let Some(mut p) = pipe.take() {
                    nonce += 1;
                    let _ = p.write_all(&encode(OP_FRAME, &set_activity(pid, None, nonce)));
                    let _ = p.write_all(&encode(OP_CLOSE, &json!({})));
                }
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if pipe.is_none() && last_try.is_none_or(|t| t.elapsed() >= RETRY) {
            last_try = Some(Instant::now());
            if let Some((mut p, path)) = connect() {
                let ok = p.write_all(&encode(OP_HANDSHAKE, &handshake(&id))).is_ok()
                    && read_frame(p.as_mut()).is_some_and(|(op, v)| op == OP_FRAME && v["evt"] == "READY");
                if ok {
                    super::log::info(format!("Discord presence connected on {path}"));
                    pipe = Some(p);
                    dirty = true;
                    tell(Status::Connected);
                } else {
                    super::log::debug("Discord refused the handshake");
                    tell(Status::Waiting);
                }
            } else {
                tell(Status::Waiting);
            }
        }
        if !dirty {
            continue;
        }
        let Some(p) = pipe.as_mut() else {
            continue;
        };
        if last_sent.is_some_and(|t| t.elapsed() < MIN_INTERVAL) {
            continue;
        }
        nonce += 1;
        let frame = encode(OP_FRAME, &set_activity(pid, want.as_ref(), nonce));
        if p.write_all(&frame).is_ok() && read_frame(p.as_mut()).is_some() {
            dirty = false;
            last_sent = Some(Instant::now());
        } else {
            super::log::info("Discord presence disconnected");
            pipe = None;
            tell(Status::Waiting);
        }
    }
}

/// Unix seconds now, for an activity's start.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_little_endian_opcode_length_json() {
        let frame = encode(OP_HANDSHAKE, &handshake("123456789012345678"));
        assert_eq!(&frame[0..4], &[0, 0, 0, 0]);
        let len = u32::from_le_bytes(frame[4..8].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 8);
        let (op, value, used) = decode(&frame).unwrap();
        assert_eq!((op, used), (OP_HANDSHAKE, frame.len()));
        assert_eq!(value["v"], 1);
        assert_eq!(value["client_id"], "123456789012345678");
        assert!(decode(&frame[..frame.len() - 1]).is_none());
        assert_eq!(&encode(OP_CLOSE, &json!({}))[0..4], &[2, 0, 0, 0]);
    }

    #[test]
    fn set_activity_carries_the_lines_and_a_clear_sends_null() {
        let a = Activity {
            details: "Browsing mods".into(),
            state: Some("12 mods on".into()),
            since: 1_700_000_000,
        };
        let v = set_activity(42, Some(&a), 7);
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        assert_eq!(v["args"]["pid"], 42);
        assert_eq!(v["args"]["activity"]["details"], "Browsing mods");
        assert_eq!(v["args"]["activity"]["state"], "12 mods on");
        assert_eq!(v["args"]["activity"]["timestamps"]["start"], 1_700_000_000u64);
        assert_eq!(v["nonce"], "7");
        let hidden = set_activity(42, Some(&Activity { state: None, ..a }), 8);
        assert!(hidden["args"]["activity"].get("state").is_none());
        assert!(set_activity(42, None, 9)["args"]["activity"].is_null());
    }

    #[test]
    fn socket_candidates_cover_the_usual_folders() {
        let paths = candidates(|name| (name == "TMPDIR").then(|| "/var/folders/x/T/".to_string()));
        if cfg!(windows) {
            assert_eq!(paths[0], r"\\?\pipe\discord-ipc-0");
        } else {
            assert_eq!(paths[0], "/var/folders/x/T/discord-ipc-0");
            assert!(paths.contains(&"/tmp/discord-ipc-0".to_string()));
            assert!(paths.contains(&"/tmp/app/com.discordapp.Discord/discord-ipc-0".to_string()));
            assert!(paths.iter().any(|p| p.ends_with("discord-ipc-9")));
        }
    }

    #[test]
    fn the_app_id_must_look_like_a_snowflake() {
        assert!(valid_id("1504228345922191380"));
        assert!(!valid_id(""));
        assert!(!valid_id("abc"));
    }
}
