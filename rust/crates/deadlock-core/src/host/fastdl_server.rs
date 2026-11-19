//! The map download server a hosted server runs beside it (ADR 0015 amendment): a small
//! read-only HTTP/1.1 file server over `<server>/sigil-fastdl/`, so friends who use Sigil get
//! the server's custom maps and addons with no upload and no web host.
//!
//! What it does, and nothing more:
//!
//! - GET and HEAD only. Any other method gets 405; anything it does not serve gets 404.
//! - Serves only the download layout [`super::build_fastdl`] writes: `/<maps|addons>/<file>` and
//!   `/<file>`, where `<file>` is `<name>_<hash16>.vpk.zst` (or `.vpk.bz2`). The request target
//!   is matched against that pattern before the disk is touched, so `..`, `%` escapes,
//!   backslashes, absolute paths, query strings, hidden and temporary files never resolve.
//! - No directory listing. No symlinks: each path part is read with `symlink_metadata`, and the
//!   canonical path must stay inside the canonical root.
//! - Bounded: the request head ([`Limits::max_header_bytes`]), the time to send it, connections
//!   in total and per address, an idle write and a whole connection's lifetime. One request per
//!   connection (`Connection: close`).
//! - Files stream from disk in 64 KiB blocks with `Content-Length` and
//!   `application/octet-stream`; nothing is loaded whole.
//!
//! It logs nothing: no addresses, no paths. Dropping or stopping the handle closes the listener
//! and every open connection.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::servers::valid_content_name;

/// How often the accept loop looks at the stop flag while no one connects.
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// The block size files are sent in.
const CHUNK: usize = 64 * 1024;
/// Longest request target accepted (the longest valid one is about 100 bytes).
const MAX_TARGET: usize = 256;

/// The caps one server runs under. [`Limits::default`] is what hosting uses; tests shrink them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Open connections at once, from everyone.
    pub max_connections: usize,
    /// Open connections at once from one address. Friends behind one router share an address.
    pub max_per_ip: usize,
    /// The request line and headers together. More gets 431.
    pub max_header_bytes: usize,
    /// Time to send the whole request head after connecting.
    pub header_deadline: Duration,
    /// How long one write may wait on a client that does not read.
    pub write_timeout: Duration,
    /// The longest one connection may stay open, download included.
    pub max_connection_time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_connections: 32,
            max_per_ip: 8,
            max_header_bytes: 8 * 1024,
            header_deadline: Duration::from_secs(10),
            write_timeout: Duration::from_secs(30),
            max_connection_time: Duration::from_secs(60 * 60),
        }
    }
}

/// A running download server. Dropping it stops it.
#[derive(Debug)]
pub struct FastdlServer {
    addr: SocketAddr,
    root: PathBuf,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct Shared {
    root: PathBuf,
    limits: Limits,
    stop: AtomicBool,
    slots: Mutex<Slots>,
    served: AtomicU64,
}

#[derive(Debug, Default)]
struct Slots {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
    next_id: u64,
    /// A second handle on each open connection, so a stop can close them.
    open: HashMap<u64, TcpStream>,
}

impl Slots {
    /// Take a slot for `ip`, or None when the server or that address is at its cap.
    fn claim(&mut self, ip: IpAddr, limits: &Limits) -> Option<u64> {
        let mine = self.per_ip.get(&ip).copied().unwrap_or(0);
        if self.total >= limits.max_connections || mine >= limits.max_per_ip {
            return None;
        }
        self.total += 1;
        self.per_ip.insert(ip, mine + 1);
        self.next_id += 1;
        Some(self.next_id)
    }

    fn release(&mut self, id: u64, ip: IpAddr) {
        self.total = self.total.saturating_sub(1);
        if let Some(count) = self.per_ip.get_mut(&ip) {
            *count -= 1;
            if *count == 0 {
                self.per_ip.remove(&ip);
            }
        }
        self.open.remove(&id);
    }
}

/// Gives the slot back when a connection ends, however it ends.
struct Slot {
    shared: Arc<Shared>,
    id: u64,
    ip: IpAddr,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.shared.slots.lock() {
            slots.release(self.id, self.ip);
        }
    }
}

impl FastdlServer {
    /// Serve `dir` on every interface at `port` (`0.0.0.0:<port>`), with the default limits.
    /// `dir` must exist. Fails when the port is taken.
    pub fn start(dir: &Path, port: u16) -> Result<FastdlServer> {
        Self::start_on(dir, SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port), Limits::default())
    }

    /// Serve `dir` on `addr` with `limits`. Port 0 picks a free port ([`Self::local_addr`]).
    pub fn start_on(dir: &Path, addr: SocketAddr, limits: Limits) -> Result<FastdlServer> {
        let root = fs::canonicalize(dir)
            .map_err(|e| Error::msg(format!("The download folder {} cannot be served: {e}", dir.display())))?;
        if !root.is_dir() {
            return Err(Error::msg(format!("The download folder {} is not a folder.", dir.display())));
        }
        let listener = TcpListener::bind(addr).map_err(|e| {
            Error::msg(format!(
                "Port {} for map downloads is in use ({e}). Stop the program using it or pick another download port.",
                addr.port()
            ))
        })?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            root: root.clone(),
            limits,
            stop: AtomicBool::new(false),
            slots: Mutex::new(Slots::default()),
            served: AtomicU64::new(0),
        });
        let accept_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("sigil-fastdl".to_string())
            .spawn(move || accept_loop(listener, accept_shared))?;
        Ok(FastdlServer {
            addr,
            root,
            shared,
            thread: Some(thread),
        })
    }

    /// The address it listens on.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The folder it serves, canonical.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Files sent in full so far (GET only).
    pub fn served(&self) -> u64 {
        self.shared.served.load(Ordering::Relaxed)
    }

    /// Connections open right now.
    pub fn open_connections(&self) -> usize {
        self.shared.slots.lock().map(|s| s.total).unwrap_or(0)
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Close the listener and every open connection. Safe to call twice.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Ok(slots) = self.shared.slots.lock() {
            for stream in slots.open.values() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

impl Drop for FastdlServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::SeqCst) {
        let (stream, peer) = match listener.accept() {
            Ok(pair) => pair,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
                continue;
            }
            Err(_) => {
                std::thread::sleep(ACCEPT_POLL);
                continue;
            }
        };
        // Accepted sockets inherit non-blocking mode on some systems.
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        let claimed = shared.slots.lock().ok().and_then(|mut slots| {
            let id = slots.claim(peer.ip(), &shared.limits)?;
            if let Ok(clone) = stream.try_clone() {
                slots.open.insert(id, clone);
            }
            Some(id)
        });
        let Some(id) = claimed else {
            busy(stream);
            continue;
        };
        let slot = Slot {
            shared: Arc::clone(&shared),
            id,
            ip: peer.ip(),
        };
        // A failed spawn drops the closure, its stream and its slot.
        let _ = std::thread::Builder::new()
            .name("sigil-fastdl-conn".to_string())
            .spawn(move || serve(stream, slot));
    }
}

/// The answer at the connection cap. Written on the accept thread: a few bytes into a fresh
/// socket's send buffer do not block.
fn busy(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = stream.write_all(&head(503, "Service Unavailable", 0, &["Retry-After: 5"]));
    finish(stream, Duration::from_millis(100));
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn head(status: u16, reason: &str, length: u64, extra: &[&str]) -> Vec<u8> {
    let mut text = format!("HTTP/1.1 {status} {reason}\r\n");
    if status == 200 {
        text.push_str("Content-Type: application/octet-stream\r\n");
        // Content-addressed names: a file at one URL never changes.
        text.push_str("Cache-Control: public, max-age=31536000, immutable\r\n");
    }
    text.push_str(&format!("Content-Length: {length}\r\n"));
    text.push_str("X-Content-Type-Options: nosniff\r\n");
    for line in extra {
        text.push_str(line);
        text.push_str("\r\n");
    }
    text.push_str("Connection: close\r\n\r\n");
    text.into_bytes()
}

/// Send an empty error answer and close.
fn refuse(mut stream: TcpStream, status: u16, extra: &[&str]) {
    let _ = stream.write_all(&head(status, reason(status), 0, extra));
    finish(stream, Duration::from_millis(200));
}

/// Close politely: stop sending, read and drop what the client still sends for a moment (so
/// the close is not a reset that eats the answer), then drop the socket.
fn finish(mut stream: TcpStream, linger: Duration) {
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(linger));
    let deadline = Instant::now() + linger;
    let mut sink = [0u8; 4096];
    let mut drained = 0usize;
    while Instant::now() < deadline && drained < 64 * 1024 {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained += n,
        }
    }
}

enum HeadError {
    TooLarge,
    /// Timed out, closed early, or the stop closed it: nothing to answer.
    Gone,
}

/// Read until the blank line that ends the request head, within the size and time caps.
fn read_head(stream: &mut TcpStream, limits: &Limits) -> std::result::Result<Vec<u8>, HeadError> {
    let deadline = Instant::now() + limits.header_deadline;
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(HeadError::Gone);
        }
        let _ = stream.set_read_timeout(Some(left.max(Duration::from_millis(1))));
        let read = match stream.read(&mut chunk) {
            Ok(0) => return Err(HeadError::Gone),
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(HeadError::Gone),
        };
        let from = buffer.len().saturating_sub(3);
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let end = from + end;
            if end + 4 > limits.max_header_bytes {
                return Err(HeadError::TooLarge);
            }
            buffer.truncate(end);
            return Ok(buffer);
        }
        if buffer.len() > limits.max_header_bytes {
            return Err(HeadError::TooLarge);
        }
    }
}

/// `GET /maps/x.vpk.zst HTTP/1.1` to (method, target). None for anything malformed.
fn request_line(head: &[u8]) -> Option<(&str, &str)> {
    let line = match head.windows(2).position(|w| w == b"\r\n") {
        Some(end) => &head[..end],
        None => head,
    };
    let line = std::str::from_utf8(line).ok()?;
    if line.chars().any(char::is_control) {
        return None;
    }
    let mut parts = line.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || method.is_empty()
        || !method.bytes().all(|b| b.is_ascii_uppercase())
        || !matches!(version, "HTTP/1.1" | "HTTP/1.0")
        || target.is_empty()
        || target.len() > MAX_TARGET
    {
        return None;
    }
    Some((method, target))
}

/// True for a file name the download layout uses: `<name>_<hash16>.vpk.zst` or `.vpk.bz2`.
pub fn served_name(file: &str) -> bool {
    let Some(stem) = file.strip_suffix(".vpk.zst").or_else(|| file.strip_suffix(".vpk.bz2")) else {
        return false;
    };
    let Some((name, hash)) = stem.rsplit_once('_') else {
        return false;
    };
    valid_content_name(name)
        && hash.len() == 16
        && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The request target as a path inside `root`, or None. Pure text checks first: only
/// `/<file>`, `/maps/<file>` and `/addons/<file>` with [`served_name`] files, in plain
/// `[a-z0-9_./]`. No decoding is done, so an escape can never become a separator.
pub fn target_path(root: &Path, target: &str) -> Option<PathBuf> {
    let relative = target.strip_prefix('/')?;
    if relative.is_empty()
        || !relative
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'/')
    {
        return None;
    }
    let parts: Vec<&str> = relative.split('/').collect();
    let (folder, file) = match parts.as_slice() {
        [file] => (None, *file),
        [folder, file] if matches!(*folder, "maps" | "addons") => (Some(*folder), *file),
        _ => return None,
    };
    if !served_name(file) {
        return None;
    }
    let mut path = root.to_path_buf();
    if let Some(folder) = folder {
        path.push(folder);
    }
    path.push(file);
    Some(path)
}

/// Open the file for a target after the disk checks: no part is a symlink, the file is a
/// plain file, and its canonical path is inside the canonical `root`.
fn open_target(root: &Path, target: &str) -> Option<(File, u64)> {
    let path = target_path(root, target)?;
    let parent = path.parent()?;
    if parent != root {
        let meta = fs::symlink_metadata(parent).ok()?;
        if !meta.file_type().is_dir() {
            return None;
        }
    }
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let canonical = fs::canonicalize(&path).ok()?;
    if !canonical.starts_with(root) || canonical.parent()? != fs::canonicalize(parent).ok()? {
        return None;
    }
    let file = File::open(&canonical).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The file opened is the one checked, and no hard link reaches it from outside.
        if opened.dev() != meta.dev() || opened.ino() != meta.ino() || opened.nlink() != 1 {
            return None;
        }
    }
    Some((file, opened.len()))
}

fn serve(mut stream: TcpStream, slot: Slot) {
    let shared = Arc::clone(&slot.shared);
    let limits = shared.limits;
    let started = Instant::now();
    let _ = stream.set_write_timeout(Some(limits.write_timeout));
    let request = match read_head(&mut stream, &limits) {
        Ok(request) => request,
        Err(HeadError::TooLarge) => return refuse(stream, 431, &[]),
        Err(HeadError::Gone) => return,
    };
    if shared.stop.load(Ordering::SeqCst) {
        return;
    }
    let Some((method, target)) = request_line(&request) else {
        return refuse(stream, 400, &[]);
    };
    let get = match method {
        "GET" => true,
        "HEAD" => false,
        _ => return refuse(stream, 405, &["Allow: GET, HEAD"]),
    };
    let Some((mut file, length)) = open_target(&shared.root, target) else {
        return refuse(stream, 404, &[]);
    };
    if stream.write_all(&head(200, "OK", length, &[])).is_err() {
        return;
    }
    if !get {
        return finish(stream, Duration::from_millis(100));
    }
    let mut buffer = vec![0u8; CHUNK];
    let mut sent = 0u64;
    while sent < length {
        if shared.stop.load(Ordering::SeqCst) || started.elapsed() > limits.max_connection_time {
            return;
        }
        let want = CHUNK.min((length - sent) as usize);
        let read = match file.read(&mut buffer[..want]) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if stream.write_all(&buffer[..read]).is_err() {
            return;
        }
        sent += read as u64;
    }
    shared.served.fetch_add(1, Ordering::Relaxed);
    finish(stream, Duration::from_millis(100));
    drop(slot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sigil-fastdl-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    const FILE: &str = "dl_arena_0123456789abcdef.vpk.zst";

    /// A download folder with one map file of `size` bytes, and a server on 127.0.0.1:0.
    fn served(name: &str, size: usize, limits: Limits) -> (PathBuf, FastdlServer, Vec<u8>) {
        let root = scratch(name);
        let dir = root.join("sigil-fastdl");
        fs::create_dir_all(dir.join("maps")).unwrap();
        let body: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        fs::write(dir.join("maps").join(FILE), &body).unwrap();
        fs::write(root.join("secret.txt"), b"not for joiners").unwrap();
        let server = FastdlServer::start_on(&dir, "127.0.0.1:0".parse().unwrap(), limits).unwrap();
        (root, server, body)
    }

    struct Reply {
        status: u16,
        headers: Vec<String>,
        body: Vec<u8>,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            let prefix = format!("{}: ", name.to_ascii_lowercase());
            self.headers
                .iter()
                .find(|h| h.to_ascii_lowercase().starts_with(&prefix))
                .map(|h| &h[prefix.len()..])
        }
    }

    fn send(server: &FastdlServer, raw: &[u8]) -> Reply {
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let _ = stream.write_all(raw);
        read_reply(stream)
    }

    fn read_reply(stream: TcpStream) -> Reply {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let status = line.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            let header = header.trim_end().to_string();
            if header.is_empty() {
                break;
            }
            headers.push(header);
        }
        let mut body = Vec::new();
        let _ = reader.read_to_end(&mut body);
        Reply { status, headers, body }
    }

    fn get(server: &FastdlServer, target: &str) -> Reply {
        send(server, format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
    }

    #[test]
    fn serves_a_file_whole_and_answers_head() {
        let (root, server, body) = served("get", 300 * 1024 + 7, Limits::default());
        let reply = get(&server, &format!("/maps/{FILE}"));
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("Content-Type"), Some("application/octet-stream"));
        assert_eq!(reply.header("Content-Length"), Some(body.len().to_string().as_str()));
        assert_eq!(reply.header("Connection"), Some("close"));
        assert!(reply.body == body, "the whole file, byte for byte");

        let head = send(&server, format!("HEAD /maps/{FILE} HTTP/1.1\r\n\r\n").as_bytes());
        assert_eq!(head.status, 200);
        assert_eq!(head.header("Content-Length"), Some(body.len().to_string().as_str()));
        assert!(head.body.is_empty(), "HEAD sends no body");
        assert_eq!(server.served(), 1, "HEAD is not a download");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_joiner_downloads_and_checks_through_its_own_client() {
        // The joiner's real path: content_url + ureq + zstd decode + hash check.
        let root = scratch("joiner");
        let dir = root.join("sigil-fastdl");
        fs::create_dir_all(dir.join("maps")).unwrap();
        let vpk = b"VPK fixture bytes".repeat(1000);
        let sha = {
            use sha2::{Digest, Sha256};
            Sha256::digest(&vpk).iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        let entry = crate::servers::ContentEntry {
            kind: crate::servers::ContentKind::Map,
            name: "dl_arena".into(),
            hash: sha[..16].to_string(),
            size: Some(vpk.len() as u64),
        };
        let packed = zstd::encode_all(&vpk[..], 3).unwrap();
        let server = FastdlServer::start_on(&dir, "127.0.0.1:0".parse().unwrap(), Limits::default()).unwrap();
        let url = crate::servers::content_url(&format!("http://127.0.0.1:{}", server.port()), &entry, "zst");
        let relative = url.split_once(&format!(":{}/", server.port())).unwrap().1.to_string();
        fs::write(dir.join(&relative), &packed).unwrap();
        let reply = get(&server, &format!("/{relative}"));
        assert_eq!(reply.status, 200);
        assert_eq!(zstd::decode_all(&reply.body[..]).unwrap(), vpk);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn everything_else_is_not_found() {
        let (root, server, _) = served("404", 100, Limits::default());
        fs::write(server.root().join("maps").join("dl_arena_0123456789abcdef.vpk.zst.sigil-tmp"), b"partial").unwrap();
        fs::create_dir_all(server.root().join("maps").join("dl_dir_0123456789abcdef.vpk.zst")).unwrap();
        for target in [
            "/",
            "/maps",
            "/maps/",
            "/maps/missing_0123456789abcdef.vpk.zst",
            "/maps/dl_arena_0123456789abcdef.vpk.zst.sigil-tmp",
            "/maps/dl_dir_0123456789abcdef.vpk.zst",
            "/maps/dl_arena_0123456789ABCDEF.vpk.zst",
            "/maps/dl_arena.vpk",
            "/sounds/dl_arena_0123456789abcdef.vpk.zst",
            "/maps/x/dl_arena_0123456789abcdef.vpk.zst",
            "/maps//dl_arena_0123456789abcdef.vpk.zst",
            &format!("/maps/{FILE}?x=1"),
            &format!("/maps/{FILE}#x"),
            "/robots.txt",
        ] {
            assert_eq!(get(&server, target).status, 404, "{target}");
        }
        let post = send(&server, format!("POST /maps/{FILE} HTTP/1.1\r\nContent-Length: 0\r\n\r\n").as_bytes());
        assert_eq!(post.status, 405);
        assert_eq!(post.header("Allow"), Some("GET, HEAD"));
        for raw in [
            "GET\r\n\r\n".to_string(),
            format!("GET /maps/{FILE}\r\n\r\n"),
            format!("GET /maps/{FILE} HTTP/2\r\n\r\n"),
            format!("GET  /maps/{FILE} HTTP/1.1\r\n\r\n"),
            format!("get /maps/{FILE} HTTP/1.1\r\n\r\n"),
        ] {
            assert_eq!(send(&server, raw.as_bytes()).status, 400, "{raw:?}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn traversal_and_escapes_are_refused() {
        let (root, server, _) = served("traversal", 100, Limits::default());
        for target in [
            "/../secret.txt",
            "/maps/../../secret.txt",
            "/maps/../maps/dl_arena_0123456789abcdef.vpk.zst",
            "/./maps/dl_arena_0123456789abcdef.vpk.zst",
            "/%2e%2e/secret.txt",
            "/maps%2fdl_arena_0123456789abcdef.vpk.zst",
            "/maps/%2e%2e%2f%2e%2e%2fsecret.txt",
            "/%252e%252e/secret.txt",
            "/maps\\dl_arena_0123456789abcdef.vpk.zst",
            "/..\\..\\secret.txt",
            "//etc/passwd",
            "/etc/passwd",
            "http://127.0.0.1/maps/dl_arena_0123456789abcdef.vpk.zst",
            "maps/dl_arena_0123456789abcdef.vpk.zst",
            "C:\\Windows\\win.ini",
            "/C:/Windows/win.ini",
            "*",
        ] {
            let reply = get(&server, target);
            assert_eq!(reply.status, 404, "{target}");
            assert!(reply.body.is_empty());
        }
        // Pure check, no disk: none of these ever names a path.
        assert!(target_path(Path::new("/srv"), "/maps/../../x_0123456789abcdef.vpk.zst").is_none());
        assert!(target_path(Path::new("/srv"), "/addons/turbo_0123456789abcdef.vpk.bz2").is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_hard_links_are_refused() {
        let (root, server, _) = served("symlink", 100, Limits::default());
        let maps = server.root().join("maps");
        std::os::unix::fs::symlink(root.join("secret.txt"), maps.join("link_0123456789abcdef.vpk.zst")).unwrap();
        fs::create_dir_all(root.join("outside")).unwrap();
        fs::write(root.join("outside").join(FILE), b"outside").unwrap();
        std::os::unix::fs::symlink(root.join("outside"), server.root().join("addons")).unwrap();
        fs::hard_link(root.join("secret.txt"), maps.join("hard_0123456789abcdef.vpk.zst")).unwrap();
        assert_eq!(get(&server, "/maps/link_0123456789abcdef.vpk.zst").status, 404);
        assert_eq!(get(&server, &format!("/addons/{FILE}")).status, 404, "a linked folder");
        assert_eq!(get(&server, "/maps/hard_0123456789abcdef.vpk.zst").status, 404);
        assert_eq!(get(&server, &format!("/maps/{FILE}")).status, 200, "the real file still serves");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_too_large_or_slow_request_head_is_cut_off() {
        let limits = Limits {
            header_deadline: Duration::from_millis(400),
            ..Limits::default()
        };
        let (root, server, _) = served("headers", 100, limits);
        let mut raw = format!("GET /maps/{FILE} HTTP/1.1\r\n").into_bytes();
        while raw.len() < 20 * 1024 {
            raw.extend_from_slice(b"X-Padding: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
        }
        raw.extend_from_slice(b"\r\n");
        assert_eq!(send(&server, &raw).status, 431);
        // One endless header line with no line break at all.
        assert_eq!(send(&server, &vec![b'a'; 9000]).status, 431);

        // A client that never finishes its request is dropped after the deadline.
        let mut slow = TcpStream::connect(server.local_addr()).unwrap();
        slow.write_all(b"GET /maps/").unwrap();
        slow.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let started = Instant::now();
        let mut sink = Vec::new();
        let _ = slow.read_to_end(&mut sink);
        assert!(sink.is_empty(), "no answer to a half request");
        assert!(started.elapsed() < Duration::from_secs(4), "closed at the deadline");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn connections_are_capped_and_freed() {
        let limits = Limits {
            max_connections: 4,
            max_per_ip: 4,
            header_deadline: Duration::from_secs(3),
            ..Limits::default()
        };
        let (root, server, body) = served("many", 1000, limits);
        // Four clients that connect and say nothing hold every slot.
        let idle: Vec<TcpStream> = (0..4).map(|_| TcpStream::connect(server.local_addr()).unwrap()).collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.open_connections() < 4 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(server.open_connections(), 4);
        let reply = get(&server, &format!("/maps/{FILE}"));
        assert_eq!(reply.status, 503);
        assert_eq!(reply.header("Retry-After"), Some("5"));
        drop(idle);
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.open_connections() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(server.open_connections(), 0, "closed clients give their slots back");

        // Many clients one after another, and a burst at once.
        for _ in 0..20 {
            assert_eq!(get(&server, &format!("/maps/{FILE}")).body, body);
        }
        let addr = server.local_addr();
        let burst: Vec<_> = (0..12)
            .map(|_| {
                std::thread::spawn(move || {
                    let mut stream = TcpStream::connect(addr).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                    stream.write_all(format!("GET /maps/{FILE} HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
                    read_reply(stream).status
                })
            })
            .collect();
        let statuses: Vec<u16> = burst.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(statuses.iter().all(|s| *s == 200 || *s == 503), "{statuses:?}");
        assert!(statuses.contains(&200));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn per_address_slots_are_counted_apart() {
        let limits = Limits {
            max_connections: 3,
            max_per_ip: 2,
            ..Limits::default()
        };
        let mut slots = Slots::default();
        let a: IpAddr = "203.0.113.5".parse().unwrap();
        let b: IpAddr = "198.51.100.9".parse().unwrap();
        let first = slots.claim(a, &limits).unwrap();
        slots.claim(a, &limits).unwrap();
        assert!(slots.claim(a, &limits).is_none(), "per address");
        slots.claim(b, &limits).unwrap();
        assert!(slots.claim(b, &limits).is_none(), "in total");
        slots.release(first, a);
        assert!(slots.claim(b, &limits).is_some());
    }

    #[test]
    fn stopping_closes_the_port_and_open_downloads() {
        let (root, mut server, _) = served("stop", 100, Limits::default());
        let addr = server.local_addr();
        let waiting = TcpStream::connect(addr).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.open_connections() < 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(server.is_running());
        server.stop();
        assert!(!server.is_running());
        waiting.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut sink = Vec::new();
        let _ = (&waiting).read_to_end(&mut sink);
        assert!(sink.is_empty(), "the open connection was closed without an answer");
        assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err(), "the port is closed");
        server.stop();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_folder_or_a_taken_port_is_an_error() {
        let root = scratch("errors");
        assert!(FastdlServer::start(&root.join("nope"), 0).is_err());
        let first = FastdlServer::start_on(&root, "127.0.0.1:0".parse().unwrap(), Limits::default()).unwrap();
        let taken = FastdlServer::start_on(&root, first.local_addr(), Limits::default());
        let message = taken.unwrap_err().to_string();
        assert!(message.contains("in use"), "{message}");
        let _ = fs::remove_dir_all(&root);
    }
}
