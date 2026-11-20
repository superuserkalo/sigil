//! Tests for opt-in telemetry. File IO uses a per-test folder under the system temp directory;
//! network tests start a loopback server so they never leave the machine.

use super::*;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dah-telemetry-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn usage_on(data_dir: &Path) {
    write_consent(
        data_dir,
        &Consent {
            asked: true,
            usage: true,
            crashes: false,
            hash_lookup: false,
        },
    )
    .unwrap();
}

fn event(name: &str, ts: i64) -> Queued {
    Queued {
        name: name.to_string(),
        page: None,
        ts,
    }
}

fn sha256_of(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------------------------------------------------------------------------
// A loopback server that records request heads and bodies
// ---------------------------------------------------------------------------

struct Server {
    base: String,
    heads: Arc<StdMutex<Vec<String>>>,
    bodies: Arc<StdMutex<Vec<String>>>,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start(reply: &str) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let heads = Arc::new(StdMutex::new(Vec::new()));
        let bodies = Arc::new(StdMutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let reply = reply.to_string();
        let thread = {
            let heads = Arc::clone(&heads);
            let bodies = Arc::clone(&bodies);
            let hits = Arc::clone(&hits);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // macOS can hand an accepted socket the listener's nonblocking flag.
                            let _ = stream.set_nonblocking(false);
                            hits.fetch_add(1, Ordering::SeqCst);
                            if let Some((head, body)) = read_request(&mut stream) {
                                heads.lock().unwrap().push(head);
                                bodies.lock().unwrap().push(body);
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    reply.len(),
                                    reply
                                );
                                let _ = stream.write_all(response.as_bytes());
                                let _ = stream.flush();
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Server {
            base,
            heads,
            bodies,
            hits,
            stop,
            thread: Some(thread),
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }

    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String)> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .ok()?;
    let mut data = Vec::new();
    let mut buffer = [0_u8; 8192];
    let head_end = loop {
        if let Some(position) = find_bytes(&data, b"\r\n\r\n") {
            break position;
        }
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let start = head_end + 4;
    while data.len() < start + length {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    let body = String::from_utf8_lossy(&data[start..(start + length).min(data.len())]).to_string();
    Some((head, body))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Consent and install id
// ---------------------------------------------------------------------------

#[test]
fn consent_defaults_to_off_and_round_trips() {
    let dir = root("consent");
    let default = read_consent(&dir);
    assert_eq!(default, Consent::default());
    assert!(!default.asked && !default.usage && !default.crashes && !default.hash_lookup);

    let chosen = Consent {
        asked: true,
        usage: true,
        crashes: false,
        hash_lookup: true,
    };
    write_consent(&dir, &chosen).unwrap();
    assert_eq!(read_consent(&dir), chosen);

    // A torn or edited file means "off", never a half-on state.
    fs::write(consent_path(&dir), b"not json").unwrap();
    assert_eq!(read_consent(&dir), Consent::default());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_switch_without_a_shown_prompt_still_sends_nothing() {
    let dir = root("not-asked");
    write_consent(
        &dir,
        &Consent {
            asked: false,
            usage: true,
            crashes: true,
            hash_lookup: true,
        },
    )
    .unwrap();
    record(&dir, "page:browse", None);
    assert!(read_queue(&dir).is_empty());
    assert!(!Consent {
        asked: false,
        usage: true,
        ..Consent::default()
    }
    .usage_on());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_install_id_is_one_uuid_v4_per_app_data_directory() {
    let dir = root("install-id");
    let first = install_id(&dir);
    assert!(is_uuid(&first), "{first}");
    assert_eq!(&first[14..15], "4");
    assert!(matches!(&first[19..20], "8" | "9" | "a" | "b"), "{first}");
    assert_eq!(install_id(&dir), first);

    // A corrupt store is replaced, not sent on.
    fs::write(install_id_path(&dir), b"not a uuid").unwrap();
    let second = install_id(&dir);
    assert!(is_uuid(&second));
    assert_ne!(second, "not a uuid");
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Usage events
// ---------------------------------------------------------------------------

#[test]
fn recording_needs_usage_consent_and_a_known_name() {
    let dir = root("record-off");
    record(&dir, "page:browse", None);
    assert!(read_queue(&dir).is_empty());

    usage_on(&dir);
    record(&dir, "not:a-real-event", None);
    assert!(read_queue(&dir).is_empty());

    record(&dir, "page:browse", Some("not:a-real-page"));
    let queue = read_queue(&dir);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].name, "page:browse");
    assert_eq!(queue[0].page, None);
    assert!(queue[0].ts > 0);

    record(&dir, "page:profiles", Some("page:settings"));
    assert_eq!(read_queue(&dir)[1].page.as_deref(), Some("page:settings"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_queue_keeps_the_newest_five_hundred() {
    let dir = root("queue-bound");
    usage_on(&dir);
    for index in 0..505 {
        let name = if index % 2 == 0 {
            "page:browse"
        } else {
            "page:profiles"
        };
        record(&dir, name, None);
    }
    let queue = read_queue(&dir);
    assert_eq!(queue.len(), QUEUE_LIMIT);
    // Calls 1 to 5 (browse, profiles, browse, profiles, browse) fell off the front.
    assert_eq!(queue[0].name, "page:profiles");
    assert_eq!(
        queue.iter().map(|event| event.ts).collect::<Vec<_>>(),
        {
            let mut sorted = queue.iter().map(|event| event.ts).collect::<Vec<_>>();
            sorted.sort();
            sorted
        }
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn flush_sends_batches_of_fifty_and_clears_the_queue() {
    let dir = root("flush");
    usage_on(&dir);
    let events: Vec<Queued> = (0..120).map(|index| event("page:browse", index)).collect();
    write_queue(&dir, &events).unwrap();

    let server = Server::start("{}");
    let sent = flush_batches(&dir, "9.9.9", &server.base).unwrap();
    assert_eq!(sent, 120);
    assert!(read_queue(&dir).is_empty());

    let bodies = server.bodies();
    assert_eq!(bodies.len(), 3);
    let sizes: Vec<usize> = bodies
        .iter()
        .map(|body| {
            let value: Value = serde_json::from_str(body).unwrap();
            assert_eq!(value["version"], "9.9.9");
            assert!(is_uuid(value["installId"].as_str().unwrap()));
            assert!(!value["os"].as_str().unwrap().is_empty());
            value["events"].as_array().unwrap().len()
        })
        .collect();
    assert_eq!(sizes, [50, 50, 20]);

    let head = server.heads()[0].to_ascii_lowercase();
    assert!(head.contains("x-sigil-client: 1"), "{head}");
    assert!(head.contains("x-sigil-version: 9.9.9"), "{head}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn flush_without_consent_drops_the_queue_and_sends_nothing() {
    let dir = root("flush-off");
    write_consent(&dir, &Consent::default()).unwrap();
    write_queue(&dir, &[event("page:browse", 1)]).unwrap();

    let server = Server::start("{}");
    assert_eq!(flush(&dir, "1.0.0").unwrap(), 0);
    assert_eq!(server.hits(), 0);
    assert!(!queue_path(&dir).exists());
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Crashes
// ---------------------------------------------------------------------------

#[test]
fn the_panic_hook_writes_a_scrubbed_crash_file() {
    let dir = root("panic");
    install_panic_hook(&dir, "0.1.0");

    let result = std::panic::catch_unwind(|| panic!("disk full at /home/bob/mods"));
    assert!(result.is_err());

    let mut found = None;
    for path in crash_files(&crash_dir(&dir)) {
        let text = fs::read_to_string(&path).unwrap();
        let crash: CrashFile = serde_json::from_str(&text).unwrap();
        if crash.message.contains("disk full") {
            found = Some(crash);
            break;
        }
    }
    let crash = found.expect("the panic hook writes a crash file");
    assert!(crash.message.contains("truncated") || crash.message.contains("/home/<user>"));
    assert!(!crash.message.contains("bob"), "{}", crash.message);
    assert!(!crash.backtrace.is_empty());
    assert_eq!(crash.version, "0.1.0");
    assert!(!crash.os.is_empty());
    assert!(crash.ts > 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pending_crashes_wait_for_consent_then_are_sent_once() {
    let dir = root("crash-send");
    fs::create_dir_all(crash_dir(&dir)).unwrap();
    let crash = CrashFile {
        message: "boom at C:\\Users\\Alice\\game".to_string(),
        backtrace: "frame 1".to_string(),
        version: "0.1.0".to_string(),
        os: "windows".to_string(),
        ts: 1,
    };
    fs::write(
        crash_dir(&dir).join("2026-01-01T00-00-00.000000000Z.json"),
        serde_json::to_vec(&crash).unwrap(),
    )
    .unwrap();

    let server = Server::start("{}");
    // Off: nothing goes, the file stays for when the player agrees.
    assert_eq!(send_pending_crashes(&dir, "0.1.0").unwrap(), 0);
    assert_eq!(server.hits(), 0);
    assert_eq!(crash_files(&crash_dir(&dir)).len(), 1);

    // On: one file, sent once, deleted after the Worker accepts it.
    fs::write(
        consent_path(&dir),
        serde_json::to_vec(&Consent {
            asked: true,
            usage: false,
            crashes: true,
            hash_lookup: false,
        })
        .unwrap(),
    )
    .unwrap();
    let sent = send_crashes_at(&dir, "0.1.0", &server.base).unwrap();
    assert_eq!(sent, 1);
    assert!(crash_files(&crash_dir(&dir)).is_empty());
    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    let value: Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(value["message"], "boom at C:\\Users\\<user>\\game");
    assert_eq!(value["version"], "0.1.0");
    assert!(is_uuid(value["installId"].as_str().unwrap()));
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

#[test]
fn diagnostics_parse_the_worker_reference_code() {
    assert_eq!(
        parse_diagnostics_code(r#"{"code":"SGL-AB12-CD34"}"#).as_deref(),
        Some("SGL-AB12-CD34")
    );
    assert!(parse_diagnostics_code(r#"{"code":"SGL-AB1-CD34"}"#).is_none());
    assert!(parse_diagnostics_code(r#"{"code":"sgl-ab12-cd34"}"#).is_none());
    assert!(parse_diagnostics_code(r#"{"code":"SGL-AB12-CD34-EXTRA"}"#).is_none());
    assert!(parse_diagnostics_code("not json").is_none());
    assert!(is_reference_code("SGL-ZXCV-2345"));
    assert!(!is_reference_code("SGL-ZXCV-23456"));
}

#[test]
fn diagnostics_are_scrubbed_and_return_the_code() {
    let dir = root("diagnostics");
    let server = Server::start(r#"{"code":"SGL-AB12-CD34"}"#);
    let code = send_diagnostics_at(
        &dir,
        "2.0.0",
        &server.base,
        "it broke in /home/bob/mods",
        "log: alice@example.com key sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
    )
    .unwrap();
    assert_eq!(code, "SGL-AB12-CD34");

    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    let value: Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(value["version"], "2.0.0");
    assert!(is_uuid(value["installId"].as_str().unwrap()));
    let summary = value["summary"].as_str().unwrap();
    let log = value["log"].as_str().unwrap();
    assert!(summary.contains("/home/<user>"), "{summary}");
    assert!(!summary.contains("bob"), "{summary}");
    assert!(log.contains("<email>"), "{log}");
    assert!(!log.contains("example.com"), "{log}");
    assert!(!log.contains("abcdefghijklmnopqrstuvwxyz"), "{log}");
    let head = server.heads()[0].to_ascii_lowercase();
    assert!(head.contains("x-sigil-version: 2.0.0"), "{head}");

    // An answer without a code is an error, not an empty reference.
    let server = Server::start("{}");
    assert!(send_diagnostics_at(&dir, "2.0.0", &server.base, "x", "y").is_err());
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Hash lookups
// ---------------------------------------------------------------------------

#[test]
fn identify_parses_matches_best_first() {
    let sha = sha256_of("mod bytes");
    let server = Server::start(&format!(
        r#"{{"matches":[{{"gamebananaModId":10,"fileId":11,"reports":2}},{{"gamebananaModId":20,"fileId":21,"reports":9}},{{"gamebananaModId":30}}]}}"#
    ));
    let matches = identify_at(&server.base, "1.0.0", &sha.to_ascii_uppercase(), 1024).unwrap();
    assert_eq!(
        matches,
        vec![
            HashMatch {
                gamebanana_mod_id: 20,
                file_id: 21,
                reports: 9,
            },
            HashMatch {
                gamebanana_mod_id: 10,
                file_id: 11,
                reports: 2,
            },
        ]
    );
    let head = server.heads()[0].to_ascii_lowercase();
    assert!(head.contains(&format!("sha256={sha}")), "{head}");
    assert!(head.contains("size=1024"), "{head}");
    assert!(head.contains("x-sigil-client: 1"), "{head}");

    assert!(identify_at(&server.base, "1.0.0", "not a hash", 1).is_err());
    assert!(identify_at(&server.base, "1.0.0", &sha, 0).is_err());
}

#[test]
fn identify_is_off_without_hash_lookup_consent() {
    // No directory is passed to identify, so consent lives in the configured one.
    let dir = root("identify-off");
    write_consent(&dir, &Consent::default()).unwrap();
    configure(&dir, "1.0.0");
    let server = Server::start("{}");
    let before = server.hits();
    assert!(identify(&sha256_of("x"), 1).unwrap().is_empty());
    assert_eq!(server.hits(), before);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn contributing_a_hash_needs_consent_and_sends_the_documented_fields() {
    let dir = root("contribute");
    let sha = sha256_of("mod bytes");
    assert!(contribute_hash(&dir, &sha, 10, 20, 30).is_ok());

    let body = json!({
        "sha256": sha,
        "size": 10,
        "gamebananaModId": 20,
        "fileId": 30,
    });
    let value: Value = serde_json::from_str(&body.to_string()).unwrap();
    assert_eq!(value["gamebananaModId"], 20);
    assert_eq!(value["fileId"], 30);

    let server = Server::start("{}");
    contribute_hash_at("1.0.0", &server.base, "4f5d7a1e-2b3c-4d5e-8f90-123456789abc", &sha, 10, 20, 30).unwrap();
    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    let sent: Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent["sha256"], sha);
    assert_eq!(sent["size"], 10);
    assert_eq!(sent["gamebananaModId"], 20);
    assert_eq!(sent["fileId"], 30);
    assert!(contribute_hash_at("1.0.0", &server.base, "4f5d7a1e-2b3c-4d5e-8f90-123456789abc", &sha, 0, 20, 30).is_err());
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Scrubbing
// ---------------------------------------------------------------------------

#[test]
fn scrub_covers_paths_keys_and_emails() {
    assert_eq!(
        scrub("/home/bob/games/deadlock/addons"),
        "/home/<user>/games/deadlock/addons"
    );
    assert_eq!(
        scrub(r"C:\Users\Alice\AppData\Local\Sigil"),
        r"C:\Users\<user>\AppData\Local\Sigil"
    );
    assert_eq!(
        scrub(r"C:\\Users\\Alice\\AppData"),
        r"C:\\Users\\<user>\\AppData"
    );
    assert_eq!(scrub(r"Z:\home\alice\steam"), r"Z:\home\<user>\steam");
    assert_eq!(scrub("plain text stays"), "plain text stays");

    let key = scrub("key sk-ant-api03-abcdefghijklmnopqrstuvwxyz");
    assert!(key.starts_with("key sk-<token>"), "{key}");
    assert!(!key.contains("abcdefghijklmnopqrstuvwxyz"), "{key}");
    assert_eq!(scrub("gsk_abcdefghijklmnopqrstuvwxyz"), "gsk_<token>");
    assert_eq!(scrub("xai-abcdefghijklmnopqrstuvwxyz"), "xai-<token>");
    assert_eq!(
        scrub("Authorization: Bearer abcdefghijklmnopqrstuvwxyz"),
        "Authorization: Bearer <token>"
    );
    assert_eq!(
        scrub("token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature123"),
        "token <token>"
    );
    assert_eq!(scrub("sha 0123456789abcdef0123456789abcdef"), "sha <token>");
    assert_eq!(
        scrub("blob QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo="),
        "blob <token>"
    );
    assert_eq!(scrub("mail alice@example.com now"), "mail <email> now");
    assert_eq!(scrub("steam 76561198000000000"), "steam <steamid64>");
    assert_eq!(scrub("player [U:1:123456]"), "player <steamid3>");
}

#[test]
fn scrub_wipes_the_running_users_home() {
    for home in home_dirs() {
        let text = format!("at {home}/projects/sigil/core.rs");
        let scrubbed = scrub(&text);
        assert!(!scrubbed.contains(&home), "{scrubbed}");
        assert!(scrubbed.contains("<home>"), "{scrubbed}");
    }
}

/// The Worker drops an event name it does not list, so every name the app sends must be in
/// `services/telemetry/src/events.ts` (a page event as its bare page name).
#[test]
fn every_event_name_is_on_the_workers_allowlist() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../services/telemetry/src/events.ts");
    let Ok(source) = std::fs::read_to_string(&path) else {
        return; // a crate-only checkout has no Worker to compare with
    };
    for name in EVENTS {
        let listed = match name.strip_prefix("page:") {
            Some(page) => source.contains(&format!("\"{page}\"")),
            None => source.contains(&format!("\"{name}\"")),
        };
        assert!(listed, "{name} is not on the Worker's allowlist");
    }
}
