//! Backlogged for this release (owner decision 2026-11-18): nothing starts this listener.
#![allow(dead_code)]
//! DeadlockForge 1-click installs: a local HTTP listener the DeadlockForge site sends a built
//! VPK to. Ported from Grimoire's `forgeBridge.ts` (MIT): the same ports, paths, headers,
//! origin and host checks, size limits, VPK magic, one install at a time with a cooldown.
//!
//! Safety: off by default; binds 127.0.0.1 only; answers only `https://deadlockforge.net`
//! and `https://www.deadlockforge.net` origins with a loopback `Host`; nothing installs until
//! the user presses Install in Sigil (the site never learns the answer), and a request nobody
//! answers is dropped after two minutes. It makes no outbound connection.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::{El, Waker, row, spacer, text};

use super::i18n::{fill, tr};
use super::options;
use super::{button, say};
use crate::app::State;
use crate::ui::{ActionKind, SMALL, Ui};

pub const PORTS: [u16; 5] = [43110, 43111, 43112, 43113, 43114];
pub const ORIGINS: [&str; 2] = ["https://deadlockforge.net", "https://www.deadlockforge.net"];
pub const MIN_BYTES: u64 = 1024;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
pub const VPK_MAGIC: u32 = 0x55AA_1234;
pub const ANSWER_WITHIN: Duration = Duration::from_secs(120);
const COOLDOWN: Duration = Duration::from_secs(3);

/// A VPK the site sent, waiting for the user.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub name: String,
    pub author: String,
    pub kind: String,
    pub bytes: u64,
    pub origin: String,
    pub path: PathBuf,
    pub at: Instant,
}

#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    port: AtomicU16,
    busy: AtomicBool,
    queue: Mutex<VecDeque<Request>>,
    last: Mutex<Option<Instant>>,
}

#[derive(Default)]
pub struct Bridge {
    shared: Option<Arc<Shared>>,
    pub pending: Option<Request>,
    pub install_job: crate::jobs::Job<String>,
}

impl Bridge {
    pub fn port(&self) -> Option<u16> {
        self.shared.as_ref().map(|s| s.port.load(Ordering::Relaxed)).filter(|p| *p != 0)
    }
}

fn forge_dir() -> PathBuf {
    crate::data::data_dir().join("forge")
}

// --- request checks (pure) -------------------------------------------------------------------

pub fn origin_ok(origin: &str) -> bool {
    ORIGINS.contains(&origin)
}

pub fn host_ok(host: &str, port: u16) -> bool {
    [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host))
}

/// A header value as the site URI-encodes it, decoded, without control, bidi or zero-width
/// characters, cut to `max` characters.
pub fn clean(value: &str, max: usize) -> String {
    let decoded = percent_decode(value);
    decoded
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069 | 0xFEFF)
        })
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The parsed request line and headers (names lowercased).
#[derive(Debug, Default, PartialEq)]
pub struct Head {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let head = Head {
        method: parts.next()?.to_string(),
        path: parts.next()?.to_string(),
        headers: lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect(),
    };
    Some(head)
}

/// What the listener answers before reading a body: Err is (status, reason).
pub fn check(head: &Head, port: u16, enabled: bool) -> Result<(), (u16, &'static str)> {
    if !enabled {
        return Err((403, "BRIDGE_DISABLED"));
    }
    match head.get("origin") {
        Some(o) if origin_ok(o) => {}
        _ => return Err((403, "BAD_ORIGIN")),
    }
    if !head.get("host").is_some_and(|h| host_ok(h, port)) {
        return Err((403, "BAD_HOST"));
    }
    Ok(())
}

pub fn check_install(head: &Head) -> Result<u64, (u16, &'static str)> {
    if head.get("content-type").map(|v| v.split(';').next().unwrap_or("").trim()) != Some("application/octet-stream") {
        return Err((415, "BAD_CONTENT_TYPE"));
    }
    if head.get("x-forge-protocol") != Some("1") {
        return Err((400, "BAD_PROTOCOL_HEADER"));
    }
    if head.get("transfer-encoding").is_some() {
        return Err((411, "LENGTH_REQUIRED"));
    }
    let len: u64 = head.get("content-length").and_then(|v| v.parse().ok()).ok_or((411, "LENGTH_REQUIRED"))?;
    if len > MAX_BYTES {
        return Err((413, "TOO_LARGE"));
    }
    if len < MIN_BYTES {
        return Err((400, "TOO_SMALL"));
    }
    Ok(len)
}

// --- the listener ----------------------------------------------------------------------------

fn respond(stream: &mut TcpStream, status: u16, origin: Option<&str>, body: &str) {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        _ => "Error",
    };
    // Header names are case-insensitive; lowercase keeps them apart from UI text.
    let mut lines = vec![
        format!("HTTP/1.1 {status} {reason}"),
        "content-type: application/json".to_string(),
        "cache-control: no-store".to_string(),
        "x-content-type-options: nosniff".to_string(),
        format!("content-length: {}", body.len()),
        "connection: close".to_string(),
    ];
    if let Some(o) = origin.filter(|o| origin_ok(o)) {
        lines.push(format!("access-control-allow-origin: {o}"));
        lines.push("access-control-allow-methods: GET, POST, OPTIONS".to_string());
        lines.push(
            "access-control-allow-headers: content-type, x-forge-protocol, x-forge-name, x-forge-author, x-forge-type"
                .to_string(),
        );
        lines.push("access-control-allow-private-network: true".to_string());
        lines.push("access-control-max-age: 600".to_string());
        lines.push("vary: origin".to_string());
    }
    let mut head = lines.join("\r\n");
    head.push_str("\r\n\r\n");
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn error(stream: &mut TcpStream, status: u16, origin: Option<&str>, code: &str) {
    respond(stream, status, origin, &format!("{{\"error\":\"{code}\"}}"));
}

fn handle(mut stream: TcpStream, shared: &Shared, port: u16, wake: Option<&Waker>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut text = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || text.len() > 16 * 1024 {
            return;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        text.push_str(&line);
    }
    let Some(head) = parse_head(&text.replace("\r\n", "\n").replace('\n', "\r\n")) else { return };
    let origin = head.get("origin").map(str::to_string);
    let origin = origin.as_deref();
    if let Err((status, code)) = check(&head, port, options::get().forge) {
        return error(&mut stream, status, origin, code);
    }
    match (head.method.as_str(), head.path.as_str()) {
        ("OPTIONS", _) => respond(&mut stream, 204, origin, ""),
        ("GET", "/forge/v1/ping") => respond(&mut stream, 200, origin, "{\"app\":\"sigil\",\"protocol\":1,\"ready\":true}"),
        ("POST", "/forge/v1/install") => {
            let len = match check_install(&head) {
                Ok(len) => len,
                Err((status, code)) => return error(&mut stream, status, origin, code),
            };
            if shared.busy.swap(true, Ordering::SeqCst) {
                return error(&mut stream, 429, origin, "BUSY");
            }
            let cooling = shared.last.lock().ok().and_then(|l| *l).is_some_and(|t| t.elapsed() < COOLDOWN);
            if cooling {
                shared.busy.store(false, Ordering::SeqCst);
                return error(&mut stream, 429, origin, "COOLDOWN");
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
            let dir = forge_dir();
            let _ = std::fs::create_dir_all(&dir);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = dir.join(format!("forge-{stamp}.vpk"));
            let result = (|| -> Result<(), (u16, &'static str)> {
                let mut file = std::fs::File::create(&path).map_err(|_| (500, "WRITE_FAILED"))?;
                let mut limited = (&mut reader).take(len);
                let mut first = [0u8; 4];
                limited.read_exact(&mut first).map_err(|_| (400, "TOO_SMALL"))?;
                if u32::from_le_bytes(first) != VPK_MAGIC {
                    return Err((400, "NOT_A_VPK"));
                }
                file.write_all(&first).map_err(|_| (500, "WRITE_FAILED"))?;
                let copied = std::io::copy(&mut limited, &mut file).map_err(|_| (400, "TRUNCATED"))?;
                if copied + 4 != len {
                    return Err((400, "TRUNCATED"));
                }
                Ok(())
            })();
            if let Ok(mut l) = shared.last.lock() {
                *l = Some(Instant::now());
            }
            match result {
                Err((status, code)) => {
                    let _ = std::fs::remove_file(&path);
                    shared.busy.store(false, Ordering::SeqCst);
                    error(&mut stream, status, origin, code);
                }
                Ok(()) => {
                    let request = Request {
                        name: {
                            let n = clean(head.get("x-forge-name").unwrap_or(""), 120);
                            if n.is_empty() { "DeadlockForge mod".into() } else { n }
                        },
                        author: clean(head.get("x-forge-author").unwrap_or(""), 60),
                        kind: clean(head.get("x-forge-type").unwrap_or(""), 12),
                        bytes: len,
                        origin: origin.unwrap_or("").to_string(),
                        path,
                        at: Instant::now(),
                    };
                    if let Ok(mut q) = shared.queue.lock() {
                        q.push_back(request);
                    }
                    respond(&mut stream, 202, origin, "{\"status\":\"pending-confirm\"}");
                    if let Some(w) = wake {
                        w();
                    }
                }
            }
        }
        _ => error(&mut stream, 404, origin, "NOT_FOUND"),
    }
}

/// Stops the older in-process listener if anything had started it. Nothing calls it while
/// DeadlockForge installs are backlogged.
pub fn sync(st: &mut State) {
    if let Some(shared) = st.settings.extra.forge.shared.take() {
        shared.stop.store(true, Ordering::SeqCst);
    }
}

/// The older listener, kept for its request checks and tests until it is removed.
#[allow(dead_code)]
fn start_legacy(st: &mut State) {
    let bridge = &mut st.settings.extra.forge;
    if bridge.shared.is_some() {
        return;
    }
    let Some((listener, port)) = PORTS
        .iter()
        .find_map(|p| TcpListener::bind(("127.0.0.1", *p)).ok().map(|l| (l, *p)))
    else {
        return say(st, tr("Every DeadlockForge port is in use."), true);
    };
    let _ = listener.set_nonblocking(true);
    let shared = Arc::new(Shared::default());
    shared.port.store(port, Ordering::SeqCst);
    let thread_shared = shared.clone();
    let wake = st.wake.clone();
    std::thread::spawn(move || {
        while !thread_shared.stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let s = thread_shared.clone();
                    let w = wake.clone();
                    std::thread::spawn(move || handle(stream, &s, port, w.as_ref()));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(150)),
            }
        }
    });
    super::log::info(format!("DeadlockForge listening on 127.0.0.1:{port}"));
    st.settings.extra.forge.shared = Some(shared);
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.extra.forge.install_job.try_take() {
        match result {
            Ok(line) => {
                st.status = line;
                crate::data::scan_addons_now(st);
                crate::data::start_addon_scan(st);
            }
            Err(e) => st.status = e,
        }
        changed = true;
    }
    // A request nobody answered goes away.
    if let Some(p) = &st.settings.extra.forge.pending
        && p.at.elapsed() >= ANSWER_WITHIN
    {
        decline(st);
        changed = true;
    }
    if st.settings.extra.forge.pending.is_none()
        && let Some(shared) = st.settings.extra.forge.shared.clone()
        && let Some(next) = shared.queue.lock().ok().and_then(|mut q| q.pop_front())
    {
        st.settings.extra.forge.pending = Some(next);
        changed = true;
    }
    changed
}

fn release(st: &mut State) {
    if let Some(shared) = &st.settings.extra.forge.shared {
        shared.busy.store(false, Ordering::SeqCst);
    }
}

pub fn decline(st: &mut State) {
    if let Some(p) = st.settings.extra.forge.pending.take() {
        let _ = std::fs::remove_file(&p.path);
    }
    release(st);
}

/// Install the pending VPK: wrapped in a zip so it goes through the usual install steps
/// (payload checks, a free slot, landing off).
pub fn accept(st: &mut State) {
    if super::watch::refuse_while_running(st) {
        return;
    }
    let Some(p) = st.settings.extra.forge.pending.take() else { return };
    release(st);
    let Some(game_root) = st.game_root.clone() else {
        let _ = std::fs::remove_file(&p.path);
        return say(st, tr("Find the game folder first."), true);
    };
    let workspace = crate::data::workspace_dir();
    let land_disabled = st.prefs.land_disabled;
    let library = deadlock_core::import::library_path(&crate::data::data_dir());
    let wake = st.wake.clone();
    st.settings.extra.forge.install_job.start(wake, move || {
        let zip_path = p.path.with_extension("zip");
        let result = (|| -> Result<String, String> {
            let file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("pak01_dir.vpk", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored))
                .map_err(|e| e.to_string())?;
            let mut vpk = std::fs::File::open(&p.path).map_err(|e| e.to_string())?;
            std::io::copy(&mut vpk, &mut zip).map_err(|e| e.to_string())?;
            zip.finish().map_err(|e| e.to_string())?;
            let request = deadlock_core::install::InstallRequest::new(zip_path.display().to_string(), workspace)
                .map_err(|e| e.to_string())?;
            let plan = deadlock_core::install::plan_install(
                &request,
                &deadlock_core::install::PlanOptions {
                    game_root: Some(game_root.clone()),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            // Written switched off; one that should land on goes on once the safety scan passes.
            let record = deadlock_core::install::apply_install_with(&plan, true, deadlock_core::install::ApplyOptions { land_disabled: true })
                .map_err(|e| e.to_string())?;
            let size_bytes = std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0);
            let gated = (!land_disabled)
                .then(|| crate::screens::installed::safety_gate::land_checked(&game_root, plan.slot, &record.target, None))
                .flatten();
            let meta = deadlock_core::import::ModMeta {
                slot: plan.slot,
                size_bytes,
                name: p.name.clone(),
                author: (!p.author.is_empty()).then(|| p.author.clone()),
                thumbnail_url: None,
                gamebanana_id: None,
                source: "deadlockforge".into(),
                recorded_at: String::new(),
            };
            let _ = deadlock_core::import::record_meta(&library, meta.clone());
            super::tools::imprint_after_install(&game_root, plan.slot, &meta);
            Ok(match (land_disabled, gated) {
                (false, None) => format!("Installed {} in slot {:02}.", p.name, plan.slot),
                (false, Some(found)) => format!(
                    "Installed {} in slot {:02}, switched off: the mod safety check asks about {} files. Turn it on in Installed to review them.",
                    p.name,
                    plan.slot,
                    found.len()
                ),
                (true, _) => format!("Installed {} in slot {:02}, switched off.", p.name, plan.slot),
            })
        })();
        let _ = std::fs::remove_file(&p.path);
        let _ = std::fs::remove_file(&zip_path);
        result
    });
}

/// The confirm a site request opens, over whatever page is showing.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let p = s.settings.extra.forge.pending.as_ref()?;
    let mut kv = vec![(tr("Mod"), p.name.clone())];
    if !p.author.is_empty() {
        kv.push((tr("Author"), p.author.clone()));
    }
    kv.push((tr("Size"), super::size_label(p.bytes)));
    let mut body = super::extra::shell(u, "settings-forge-dialog", tr("Install this mod?"));
    for (k, v) in kv {
        body = body.child(u.kv(k, &v, u.ink));
    }
    Some(
        body.child(
            text(fill(tr("Sent from {}. It lands switched off."), &[&p.origin]))
                .font(SMALL)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(button(u, "settings-forge-decline", "Decline", ActionKind::Ghost).on_press(decline))
                .child(button(u, "settings-forge-accept", "Install", ActionKind::Primary).on_press(accept)),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(lines: &str) -> Head {
        parse_head(lines).unwrap()
    }

    #[test]
    fn only_the_site_on_loopback_gets_through() {
        let ok = head("POST /forge/v1/install HTTP/1.1\r\nHost: 127.0.0.1:43110\r\nOrigin: https://deadlockforge.net\r\nContent-Type: application/octet-stream\r\nx-forge-protocol: 1\r\nContent-Length: 2048");
        assert_eq!(check(&ok, 43110, true), Ok(()));
        assert_eq!(check_install(&ok), Ok(2048));
        assert_eq!(check(&ok, 43110, false), Err((403, "BRIDGE_DISABLED")));
        assert_eq!(check(&ok, 43111, true), Err((403, "BAD_HOST")));
        let evil = head("GET /forge/v1/ping HTTP/1.1\r\nHost: 127.0.0.1:43110\r\nOrigin: https://evil.example");
        assert_eq!(check(&evil, 43110, true), Err((403, "BAD_ORIGIN")));
        let none = head("GET /forge/v1/ping HTTP/1.1\r\nHost: 127.0.0.1:43110");
        assert_eq!(check(&none, 43110, true), Err((403, "BAD_ORIGIN")));
        let rebind = head("GET / HTTP/1.1\r\nHost: attacker.example:43110\r\nOrigin: https://deadlockforge.net");
        assert_eq!(check(&rebind, 43110, true), Err((403, "BAD_HOST")));
    }

    #[test]
    fn install_headers_are_checked() {
        let base = "POST /forge/v1/install HTTP/1.1\r\nContent-Type: application/octet-stream\r\nx-forge-protocol: 1\r\n";
        assert_eq!(check_install(&head(&format!("{base}Content-Length: 10"))), Err((400, "TOO_SMALL")));
        assert_eq!(check_install(&head(&format!("{base}Content-Length: 999999999999"))), Err((413, "TOO_LARGE")));
        assert_eq!(check_install(&head(base.trim_end())), Err((411, "LENGTH_REQUIRED")));
        assert_eq!(
            check_install(&head("POST / HTTP/1.1\r\nContent-Type: text/plain\r\nContent-Length: 2048")),
            Err((415, "BAD_CONTENT_TYPE"))
        );
    }

    #[test]
    fn names_are_decoded_and_cleaned() {
        assert_eq!(clean("Blue%20Haze%E2%80%8B", 120), "Blue Haze");
        assert_eq!(clean("a\u{202E}b", 120), "ab");
        assert_eq!(clean(&"x".repeat(200), 60).len(), 60);
    }
}
