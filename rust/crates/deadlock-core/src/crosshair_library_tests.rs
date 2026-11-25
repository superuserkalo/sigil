//! Tests for the community crosshair library client. Network tests use a loopback server, so
//! nothing leaves the machine.

use super::*;

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde_json::json;

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dah-xhair-lib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn good_config() -> Value {
    config_json(&Crosshair::default())
}

fn row(id: &str, name: &str, config: Value, official: bool) -> Value {
    json!({
        "id": id,
        "name": name,
        "config": config,
        "uses": 3,
        "official": official,
        "created_day": "2026-11-25",
    })
}

/// A loopback server answering each request with the next `(status, body)` and recording the
/// request heads and bodies.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start(answers: Vec<(u16, String)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            for (status, body) in answers {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let _ = stream.set_nonblocking(false);
                if let Some(request) = crate::telemetry::tests::read_request(&mut stream) {
                    seen.lock().unwrap().push(request);
                }
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Server {
            base,
            requests,
            thread: Some(thread),
        }
    }

    fn requests(&mut self) -> Vec<(String, String)> {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.requests.lock().unwrap().clone()
    }
}

const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde";

#[test]
fn hostile_rows_are_skipped_and_good_ones_kept() {
    let mut extra_key = good_config();
    extra_key["evil"] = json!(1);
    let mut missing = good_config();
    missing.as_object_mut().unwrap().remove("dotSize");
    let mut string_int = good_config();
    string_int["pipGap"] = json!("5");
    let mut fraction = good_config();
    fraction["pipGap"] = json!(5.5);
    let mut big = good_config();
    big["pipHeight"] = json!(65);
    let mut negative = good_config();
    negative["colorR"] = json!(-1);
    let mut opacity = good_config();
    opacity["dotOpacity"] = json!(1.5);
    let mut bool_as_int = good_config();
    bool_as_int["pipGapStatic"] = json!(1);
    let mut null = good_config();
    null["colorG"] = Value::Null;
    let mut whole_float = good_config();
    whole_float["pipGap"] = json!(7.0);
    whole_float["disableHeroSpecificCrosshairs"] = json!(true);
    let long_name = "A".repeat(33);
    let body = json!({
        "crosshairs": [
            row("sigil-dot", "Dot", good_config(), true),
            row("0123456789abcdef", "Fine one", whole_float, false),
            row("0123456789abcdee", "Extra", extra_key, false),
            row("0123456789abcded", "Missing", missing, false),
            row("0123456789abcdec", "String", string_int, false),
            row("0123456789abcdeb", "Fraction", fraction, false),
            row("0123456789abcdea", "Too big", big, false),
            row("0123456789abcde9", "Negative", negative, false),
            row("0123456789abcde8", "Opacity", opacity, false),
            row("0123456789abcde7", "Bool", bool_as_int, false),
            row("0123456789abcde6", "Null", null, false),
            row("../../etc/passwd", "Path id", good_config(), false),
            row("0123456789ABCDEF", "Upper id", good_config(), false),
            row("0123456789abcde5", &long_name, good_config(), false),
            row("0123456789abcde4", "visit x.com", good_config(), false),
            row("0123456789abcde3", "<b>bold</b>", good_config(), false),
            row("0123456789abcde2", "line\u{202e}break", good_config(), false),
            row("0123456789abcde1", "Fake official", good_config(), true),
            row("sigil-fake", "Not official", good_config(), false),
            row("sigil-dot", "Repeat", good_config(), true),
            json!("not an object"),
            json!({"id": "0123456789abcde0", "name": "No config"}),
        ],
        "next_cursor": "eyJzIjoidXNlZCJ9",
    });
    let page = parse_page(body.to_string().as_bytes()).unwrap();
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["sigil-dot", "0123456789abcdef"]);
    assert_eq!(page.rows[1].settings.pip_gap, 7);
    assert!(!page.rows[1].settings.disable_hero_specific_crosshairs);
    assert_eq!(page.skipped, 20);
    assert_eq!(page.next_cursor.as_deref(), Some("eyJzIjoidXNlZCJ9"));
}

#[test]
fn a_bad_response_fails_as_a_whole() {
    assert!(parse_page(b"not json").is_err());
    assert!(parse_page(br#"{"rows": []}"#).is_err());
    let huge = vec![b' '; MAX_LIST_BYTES as usize + 1];
    assert!(parse_page(&huge).is_err());
}

#[test]
fn rows_and_cursor_are_capped() {
    let rows: Vec<Value> = (0..200)
        .map(|n| row(&format!("{n:016x}"), "Many", good_config(), false))
        .collect();
    let body = json!({ "crosshairs": rows, "next_cursor": "a/b" });
    let page = parse_page(body.to_string().as_bytes()).unwrap();
    assert_eq!(page.rows.len(), MAX_PAGE_ROWS);
    assert_eq!(page.next_cursor, None, "a cursor with a slash is dropped");
    let long = json!({ "crosshairs": [], "next_cursor": "a".repeat(600) });
    assert_eq!(parse_page(long.to_string().as_bytes()).unwrap().next_cursor, None);
}

#[test]
fn names_follow_the_worker_rules() {
    assert_eq!(clean_name("  Dr.   Aim ").unwrap(), "Dr. Aim");
    assert_eq!(clean_name("Grüne Linie").unwrap(), "Grüne Linie");
    assert_eq!(clean_name("Łódź +1!").unwrap(), "Łódź +1!");
    for bad in ["a", "x.com", "a@b", "http://x", "...", "a\u{d7}b", &"a".repeat(33), "名前", "tab\u{0}"] {
        assert_eq!(clean_name(bad), Err(LibraryError::BadName), "{bad:?}");
    }
}

#[test]
fn the_config_matches_the_worker_shape() {
    let value = config_json(&Crosshair {
        pip_opacity: 0.999,
        disable_hero_specific_crosshairs: true,
        ..Crosshair::default()
    });
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 19);
    assert_eq!(value["pipOpacity"], json!(1.0));
    assert!(value.get("disableHeroSpecificCrosshairs").is_none());
    assert_eq!(parse_config(&value).unwrap().pip_opacity, 1.0);
    let a = Crosshair { pip_opacity: 0.999, disable_hero_specific_crosshairs: true, ..Crosshair::default() };
    assert!(same_look(&a, &Crosshair::default()));
    assert!(!same_look(&Crosshair { pip_gap: 9, ..a }, &Crosshair::default()));
}

#[test]
fn list_sends_the_client_headers_and_reads_a_page() {
    let body = json!({ "crosshairs": [row("sigil-dot", "Dot", good_config(), true)], "next_cursor": null });
    let mut server = Server::start(vec![(200, body.to_string())]);
    let page = Client::at(&server.base, "0.1.41").list(Sort::New, Some("abc")).unwrap();
    assert_eq!(page.rows.len(), 1);
    let requests = server.requests();
    let head = requests[0].0.to_ascii_lowercase();
    assert!(head.starts_with("get /v1/crosshairs?sort=new&cursor=abc "), "{head}");
    assert!(head.contains("x-sigil-client: 1"));
    assert!(head.contains("x-sigil-version: 0.1.41"));
}

#[test]
fn publish_success_returns_the_id_and_token() {
    let answer = json!({ "id": "00112233aabbccdd", "delete_token": TOKEN }).to_string();
    let mut server = Server::start(vec![(201, answer)]);
    let published = Client::at(&server.base, "0.1.41")
        .publish(" My  Cross ", &Crosshair::default())
        .unwrap();
    assert_eq!(published.id, "00112233aabbccdd");
    assert_eq!(published.delete_token.expose(), TOKEN);
    let requests = server.requests();
    let sent: Value = serde_json::from_str(&requests[0].1).unwrap();
    assert_eq!(sent["name"], json!("My Cross"));
    assert_eq!(sent["config"], config_json(&Crosshair::default()));
    assert_eq!(sent.as_object().unwrap().len(), 2);
}

#[test]
fn publish_maps_the_worker_errors() {
    let cases = [
        (409, json!({"error": "duplicate", "id": "sigil-dot"}), LibraryError::Duplicate("sigil-dot".to_string())),
        (429, json!({"error": "publish_limit"}), LibraryError::PublishLimit),
        (400, json!({"error": "blocked_name"}), LibraryError::BlockedName),
        (429, json!({"error": "daily_limit"}), LibraryError::DailyLimit),
        (507, json!({"error": "library_full"}), LibraryError::LibraryFull),
        (400, json!({"error": "bad_config", "field": "pipGap"}), LibraryError::BadConfig("pipGap".to_string())),
        (429, json!({"error": "rate_limited"}), LibraryError::RateLimited),
        (503, json!({"error": "not_configured"}), LibraryError::NotConfigured),
    ];
    for (status, body, expected) in cases {
        let mut server = Server::start(vec![(status, body.to_string())]);
        let error = Client::at(&server.base, "0.1.41")
            .publish("Good name", &Crosshair::default())
            .unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.message().is_empty());
        server.requests();
    }
    assert_eq!(
        LibraryError::Duplicate(String::new()).message(),
        "This crosshair is already in the library."
    );
}

#[test]
fn a_bad_name_never_leaves_the_pc() {
    // No server: a request would fail as Offline, not BadName.
    let error = Client::at("http://127.0.0.1:9", "0.1.41")
        .publish("x.com", &Crosshair::default())
        .unwrap_err();
    assert_eq!(error, LibraryError::BadName);
}

#[test]
fn offline_mode_blocks_every_call() {
    crate::net::TEST_OFFLINE.with(|flag| flag.set(true));
    let client = Client::at("http://127.0.0.1:9", "0.1.41");
    assert_eq!(client.list(Sort::Used, None).unwrap_err(), LibraryError::Offline);
    assert_eq!(client.publish("Good name", &Crosshair::default()).unwrap_err(), LibraryError::Offline);
    assert_eq!(client.send_use("sigil-dot").unwrap_err(), LibraryError::Offline);
    crate::net::TEST_OFFLINE.with(|flag| flag.set(false));
}

#[test]
fn delete_use_and_report_hit_their_routes() {
    let mut server = Server::start(vec![
        (200, json!({"deleted": true}).to_string()),
        (200, json!({"counted": true}).to_string()),
        (200, json!({"counted": false}).to_string()),
    ]);
    let client = Client::at(&server.base, "0.1.41");
    let published = Published {
        id: "00112233aabbccdd".to_string(),
        delete_token: DeleteToken::parse(TOKEN).unwrap(),
    };
    client.delete(&published).unwrap();
    assert!(client.send_use("sigil-dot").unwrap());
    assert!(!client.report("00112233aabbccdd").unwrap());
    let requests = server.requests();
    assert!(requests[0].0.starts_with("POST /v1/crosshairs/00112233aabbccdd/delete "));
    assert_eq!(requests[0].1, json!({"delete_token": TOKEN}).to_string());
    assert!(requests[1].0.starts_with("POST /v1/crosshairs/sigil-dot/use "));
    assert!(requests[2].0.starts_with("POST /v1/crosshairs/00112233aabbccdd/report "));
    // An id that is not a library id is refused before any request.
    assert_eq!(client.send_use("../admin").unwrap_err(), LibraryError::NotFound);
}

#[test]
fn the_token_file_round_trips_and_stays_private() {
    let dir = root("store");
    let path = dir.join(STORE_FILE);
    let mut store = Store::default();
    store.published.insert(
        "preset_1".to_string(),
        Published {
            id: "00112233aabbccdd".to_string(),
            delete_token: DeleteToken::parse(TOKEN).unwrap(),
        },
    );
    store.add_reported("sigil-dot");
    store.add_reported("not an id");
    save_store(&path, &store).unwrap();
    let loaded = load_store(&path);
    assert_eq!(loaded.published, store.published);
    assert_eq!(loaded.reported, ["sigil-dot"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // The token never shows in a Debug print or a formatted line.
    let printed = format!("{loaded:?} {:?} {}", loaded.published["preset_1"], loaded.published["preset_1"].delete_token);
    assert!(!printed.contains(TOKEN), "{printed}");
    assert!(printed.contains("hidden"));
    let error = LibraryError::WrongToken;
    assert!(!format!("{error:?} {error}").contains(TOKEN));
    // A corrupt file reads as empty.
    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(load_store(&path), Store::default());
    let _ = std::fs::remove_dir_all(&dir);
}
