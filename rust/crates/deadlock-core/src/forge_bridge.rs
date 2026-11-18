//! The DeadlockForge local install bridge.
//!
//! DeadlockForge (deadlockforge.net) builds VPKs in the browser, so there is no download URL to
//! hand a `sigil:` link. The page POSTs the bytes it holds to a listener on this machine's
//! loopback address instead. The protocol and every trust rule follow Grimoire's bridge
//! (`forgeProtocol.ts` and `forgeBridge.ts`, MIT), with Sigil's own port range and app id so the
//! site can tell the managers apart and all of them can run at once.
//!
//! Trust, in order:
//! - The listener binds `127.0.0.1` only: nothing off this machine can reach it.
//! - `Origin` must be exactly `https://deadlockforge.net` or its `www` host (development builds
//!   also accept a localhost origin). A request without one is refused: browsers always send
//!   it cross-origin, so a missing origin means curl or another program.
//! - `Host` must be the loopback literal and our port, against DNS rebinding.
//! - An install must carry `Content-Type: application/octet-stream` and `x-forge-protocol: 1`.
//!   Both make the request "non-simple", so the browser sends a preflight first, which a page
//!   that is not allowlisted never gets past. That is the check the design rests on.
//! - The body is 1 KB to 512 MB, read to a temporary file with the declared length enforced
//!   while reading, and must start with the VPK signature.
//! - One install may await the player's answer at a time, and an origin waits 3 s between
//!   requests, so a page cannot stack confirmations hoping for a stray click.
//! - Nothing installs here: the app shows the name and author (cleaned of control and
//!   direction characters) and installs only after the player confirms. Responses carry no
//!   user data.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// Sigil's ports, tried in order; distinct from Grimoire's 43110-43114 and Deadlock Mod Manager's
/// 43120-43124.
pub const PORTS: [u16; 5] = [43130, 43131, 43132, 43133, 43134];
/// The name the ping answers with, so the site knows which manager it found.
pub const APP_ID: &str = "sigil";
pub const PROTOCOL_VERSION: u32 = 1;
pub const PING_PATH: &str = "/forge/v1/ping";
pub const INSTALL_PATH: &str = "/forge/v1/install";

const ALLOWED_ORIGINS: [&str; 2] = ["https://deadlockforge.net", "https://www.deadlockforge.net"];
const CONTENT_TYPE: &str = "application/octet-stream";
const PROTOCOL_HEADER: &str = "x-forge-protocol";
const NAME_HEADER: &str = "x-forge-name";
const AUTHOR_HEADER: &str = "x-forge-author";
const TYPE_HEADER: &str = "x-forge-type";
/// Every header the site sends; one left out here makes the browser refuse the request.
const ALLOW_HEADERS: &str = "Content-Type, x-forge-protocol, x-forge-name, x-forge-author, x-forge-type";

pub const MIN_BYTES: u64 = 1024;
/// Sound builds run large: about 16 KB per second of audio per replaced sound, so 600 sounds from
/// a 30 second clip approach 300 MB.
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_NAME: usize = 120;
const MAX_AUTHOR: usize = 60;
const VPK_SIGNATURE: u32 = 0x55AA_1234;
const MAX_HEAD_BYTES: usize = 16 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const COOLDOWN: Duration = Duration::from_secs(3);
/// An unanswered confirmation frees the slot after this long.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_CONNECTIONS: usize = 4;

/// A build the site sent, waiting for the player's answer. `path` is a temporary `.vpk` the
/// caller installs or deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub path: PathBuf,
    pub name: String,
    pub author: String,
    /// `sound`, `texture`, `itemart` or `hud` when the site says so.
    pub kind: Option<String>,
}

/// Why a request was refused, with the status it is answered with. No detail is echoed back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    BadOrigin,
    BadHost,
    BadContentType,
    BadProtocolHeader,
    NotFound,
    TooLarge,
    TooSmall,
    NotVpk,
    Busy,
    Cooldown,
    Internal,
}

impl Reject {
    pub fn status(self) -> u16 {
        match self {
            Reject::BadOrigin | Reject::BadHost => 403,
            Reject::BadContentType => 415,
            Reject::BadProtocolHeader | Reject::TooSmall | Reject::NotVpk => 400,
            Reject::NotFound => 404,
            Reject::TooLarge => 413,
            Reject::Busy | Reject::Cooldown => 429,
            Reject::Internal => 500,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Reject::BadOrigin => "BAD_ORIGIN",
            Reject::BadHost => "BAD_HOST",
            Reject::BadContentType => "BAD_CONTENT_TYPE",
            Reject::BadProtocolHeader => "BAD_PROTOCOL_HEADER",
            Reject::NotFound => "NOT_FOUND",
            Reject::TooLarge => "TOO_LARGE",
            Reject::TooSmall => "TOO_SMALL",
            Reject::NotVpk => "NOT_A_VPK",
            Reject::Busy => "BUSY",
            Reject::Cooldown => "COOLDOWN",
            Reject::Internal => "INTERNAL",
        }
    }
}

/// The allowlisted origin, exactly. A development build also takes `http://localhost`,
/// `127.0.0.1` or `[::1]` with any port, so the site can be worked on locally; a release never.
pub fn validate_origin(origin: Option<&str>, allow_dev: bool) -> std::result::Result<String, Reject> {
    let origin = origin.ok_or(Reject::BadOrigin)?;
    if ALLOWED_ORIGINS.contains(&origin) || (allow_dev && is_dev_origin(origin)) {
        Ok(origin.to_string())
    } else {
        Err(Reject::BadOrigin)
    }
}

fn is_dev_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else { return false };
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && host != "[" => (host, Some(port)),
        _ => (rest, None),
    };
    let host_ok = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    host_ok && port_ok
}

/// The `Host` header names the loopback address and our port (against DNS rebinding).
pub fn validate_host(host: Option<&str>, port: u16) -> std::result::Result<(), Reject> {
    let host = host.ok_or(Reject::BadHost)?.to_ascii_lowercase();
    let allowed = [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
    if allowed.contains(&host) { Ok(()) } else { Err(Reject::BadHost) }
}

/// The two headers that force a browser preflight, so only allowlisted pages can send at all.
pub fn validate_install_headers(content_type: Option<&str>, protocol: Option<&str>) -> std::result::Result<(), Reject> {
    let base = content_type.unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if base != CONTENT_TYPE {
        return Err(Reject::BadContentType);
    }
    if protocol.unwrap_or("").trim() != PROTOCOL_VERSION.to_string() {
        return Err(Reject::BadProtocolHeader);
    }
    Ok(())
}

/// A display string with control, direction-override and zero-width characters removed, so a
/// name cannot disguise what the confirmation shows; truncated, never refused.
pub fn sanitize(raw: &str, max: usize) -> String {
    raw.chars()
        .filter(|c| {
            let u = *c as u32;
            !(u <= 0x1f
                || (0x7f..=0x9f).contains(&u)
                || matches!(u, 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069 | 0x200b..=0x200d | 0xfeff))
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect()
}

/// A percent-encoded header value (the site encodes non-ASCII names). A malformed encoding is
/// kept as sent.
pub fn decode_header(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16);
    while i < bytes.len() {
        if bytes[i] == b'%' {
            match (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
                (Some(hi), Some(lo)) => {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                _ => return raw.to_string(),
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

/// The site's kind label, when it is one Sigil knows; anything else is dropped, not refused.
pub fn normalize_kind(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim().to_ascii_lowercase();
    matches!(value.as_str(), "sound" | "texture" | "itemart" | "hud").then_some(value)
}

/// The declared body size, checked before a byte is read.
pub fn validate_length(length: Option<&str>) -> std::result::Result<u64, Reject> {
    let length: u64 = length.and_then(|l| l.trim().parse().ok()).ok_or(Reject::TooSmall)?;
    if length < MIN_BYTES {
        Err(Reject::TooSmall)
    } else if length > MAX_BYTES {
        Err(Reject::TooLarge)
    } else {
        Ok(length)
    }
}

struct Shared {
    port: u16,
    allow_dev: bool,
    dir: PathBuf,
    on_pending: Box<dyn Fn(Pending) + Send + Sync>,
    /// When an install started waiting for the player; `None` when none waits.
    waiting: Mutex<Option<Instant>>,
    last_accept: Mutex<HashMap<String, Instant>>,
    connections: AtomicUsize,
    stop: AtomicBool,
    next_id: AtomicUsize,
}

/// A running listener. Dropping it, or [`Bridge::stop`], closes it.
pub struct Bridge {
    shared: Arc<Shared>,
}

impl Bridge {
    /// Listen on the first free port of [`PORTS`]. Received builds are written under `dir` and
    /// handed to `on_pending` (on the connection's thread: hand them on, do not block).
    pub fn start(dir: &Path, on_pending: impl Fn(Pending) + Send + Sync + 'static) -> Result<Bridge> {
        Self::start_on(&PORTS, dir, cfg!(debug_assertions), on_pending)
    }

    /// [`Bridge::start`] on given ports (0 lets the system pick, for tests).
    pub fn start_on(
        ports: &[u16],
        dir: &Path,
        allow_dev: bool,
        on_pending: impl Fn(Pending) + Send + Sync + 'static,
    ) -> Result<Bridge> {
        fs::create_dir_all(dir)?;
        let listener = ports
            .iter()
            .find_map(|port| TcpListener::bind(("127.0.0.1", *port)).ok())
            .ok_or_else(|| Error::msg("Every DeadlockForge port (43130 to 43134) is in use."))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            port,
            allow_dev,
            dir: dir.to_path_buf(),
            on_pending: Box::new(on_pending),
            waiting: Mutex::new(None),
            last_accept: Mutex::new(HashMap::new()),
            connections: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            next_id: AtomicUsize::new(0),
        });
        let accept = shared.clone();
        thread::Builder::new()
            .name("sigil-forge".into())
            .spawn(move || accept_loop(listener, accept))
            .map_err(|e| Error::msg(format!("Could not start the DeadlockForge listener: {e}")))?;
        Ok(Bridge { shared })
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    /// The player answered (installed or declined): the next build may come in.
    pub fn answered(&self) {
        if let Ok(mut waiting) = self.shared.waiting.lock() {
            *waiting = None;
        }
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if shared.connections.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                    shared.connections.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let conn = shared.clone();
                let spawned = thread::Builder::new().name("sigil-forge-conn".into()).spawn(move || {
                    let _ = serve(stream, &conn);
                    conn.connections.fetch_sub(1, Ordering::Relaxed);
                });
                if spawned.is_err() {
                    shared.connections.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(150)),
            Err(_) => thread::sleep(Duration::from_millis(500)),
        }
    }
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

/// Request line and headers, capped in size and time (a stalled client cannot hold a thread).
fn read_head(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut total = 0usize;
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    total += line.len();
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.split('?').next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header).ok()?;
        total += n;
        if n == 0 || total > MAX_HEAD_BYTES {
            return None;
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    Some(Request { method, path, headers })
}

fn respond(stream: &mut TcpStream, status: u16, origin: Option<&str>, body: Option<&str>) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n");
    // CORS only for an allowlisted origin, so a refused page cannot read the answer either.
    if let Some(origin) = origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {ALLOW_HEADERS}\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Max-Age: 600\r\nVary: Origin\r\n"
        ));
    }
    let body = body.unwrap_or("");
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn reject(stream: &mut TcpStream, why: Reject, origin: Option<&str>) -> std::io::Result<()> {
    respond(stream, why.status(), origin, Some(&format!("{{\"error\":\"{}\"}}", why.code())))
}

fn serve(stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(HEAD_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let Some(request) = read_head(&mut reader) else { return Ok(()) };
    let header = |name: &str| request.headers.get(name).map(String::as_str);

    let origin = match validate_origin(header("origin"), shared.allow_dev) {
        Ok(origin) => origin,
        Err(why) => return reject(&mut writer, why, None),
    };
    if let Err(why) = validate_host(header("host"), shared.port) {
        return reject(&mut writer, why, None);
    }
    let origin = Some(origin.as_str());

    match (request.method.as_str(), request.path.as_str()) {
        ("OPTIONS", _) => respond(&mut writer, 204, origin, None),
        ("GET", PING_PATH) => respond(
            &mut writer,
            200,
            origin,
            Some(&format!("{{\"app\":\"{APP_ID}\",\"protocol\":{PROTOCOL_VERSION},\"ready\":true}}")),
        ),
        ("POST", INSTALL_PATH) => match receive(&mut reader, &request, shared, origin.unwrap_or("")) {
            Ok(pending) => {
                (shared.on_pending)(pending);
                respond(&mut writer, 202, origin, Some("{\"status\":\"pending-confirm\"}"))
            }
            Err(why) => reject(&mut writer, why, origin),
        },
        _ => reject(&mut writer, Reject::NotFound, origin),
    }
}

/// Check, read and keep one build. Holds the waiting slot from here until the player answers.
fn receive(reader: &mut BufReader<TcpStream>, request: &Request, shared: &Shared, origin: &str) -> std::result::Result<Pending, Reject> {
    let header = |name: &str| request.headers.get(name).map(String::as_str);
    validate_install_headers(header("content-type"), header(PROTOCOL_HEADER))?;
    let length = validate_length(header("content-length"))?;
    {
        let mut waiting = shared.waiting.lock().map_err(|_| Reject::Internal)?;
        if waiting.is_some_and(|since| since.elapsed() < ANSWER_TIMEOUT) {
            return Err(Reject::Busy);
        }
        let mut last = shared.last_accept.lock().map_err(|_| Reject::Internal)?;
        if last.get(origin).is_some_and(|at| at.elapsed() < COOLDOWN) {
            return Err(Reject::Cooldown);
        }
        last.insert(origin.to_string(), Instant::now());
        *waiting = Some(Instant::now());
    }
    let release = || {
        if let Ok(mut waiting) = shared.waiting.lock() {
            *waiting = None;
        }
    };

    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    let path = shared.dir.join(format!("forge-{}-{id}.vpk", std::process::id()));
    let written = (|| -> std::result::Result<(), Reject> {
        reader.get_ref().set_read_timeout(Some(BODY_TIMEOUT)).map_err(|_| Reject::Internal)?;
        let mut file = File::create(&path).map_err(|_| Reject::Internal)?;
        let copied = std::io::copy(&mut reader.take(length), &mut file).map_err(|_| Reject::TooSmall)?;
        if copied != length {
            return Err(Reject::TooSmall);
        }
        drop(file);
        let mut head = [0u8; 4];
        File::open(&path).and_then(|mut f| f.read_exact(&mut head)).map_err(|_| Reject::NotVpk)?;
        if u32::from_le_bytes(head) != VPK_SIGNATURE {
            return Err(Reject::NotVpk);
        }
        Ok(())
    })();
    if let Err(why) = written {
        let _ = fs::remove_file(&path);
        release();
        return Err(why);
    }
    Ok(Pending {
        path,
        name: sanitize(&decode_header(header(NAME_HEADER).unwrap_or("")), MAX_NAME),
        author: sanitize(&decode_header(header(AUTHOR_HEADER).unwrap_or("")), MAX_AUTHOR),
        kind: normalize_kind(header(TYPE_HEADER)),
    })
}

#[cfg(test)]
#[path = "forge_bridge_tests.rs"]
mod tests;
