use super::*;
use std::sync::mpsc;

const SITE: &str = "https://deadlockforge.net";

#[test]
fn only_the_exact_site_origin_passes() {
    assert_eq!(validate_origin(Some(SITE), false).unwrap(), SITE);
    assert!(validate_origin(Some("https://www.deadlockforge.net"), false).is_ok());
    for bad in [
        "https://deadlockforge.net.evil.com",
        "https://notdeadlockforge.net",
        "http://deadlockforge.net",
        "https://deadlockforge.net:8443",
        "null",
        "",
    ] {
        assert_eq!(validate_origin(Some(bad), false), Err(Reject::BadOrigin), "{bad}");
    }
    assert_eq!(validate_origin(None, false), Err(Reject::BadOrigin));
    // Localhost only in development builds.
    assert!(validate_origin(Some("http://localhost:5173"), false).is_err());
    assert!(validate_origin(Some("http://localhost:5173"), true).is_ok());
    assert!(validate_origin(Some("http://127.0.0.1"), true).is_ok());
    assert!(validate_origin(Some("http://localhost.evil.com"), true).is_err());
}

#[test]
fn the_host_must_be_loopback_and_our_port() {
    assert!(validate_host(Some("127.0.0.1:43130"), 43130).is_ok());
    assert!(validate_host(Some("LOCALHOST:43130"), 43130).is_ok());
    assert!(validate_host(Some("[::1]:43130"), 43130).is_ok());
    assert_eq!(validate_host(Some("evil.com:43130"), 43130), Err(Reject::BadHost));
    assert_eq!(validate_host(Some("127.0.0.1:43131"), 43130), Err(Reject::BadHost));
    assert_eq!(validate_host(None, 43130), Err(Reject::BadHost));
}

#[test]
fn installs_need_the_preflight_headers_and_a_sane_length() {
    assert!(validate_install_headers(Some("application/octet-stream; charset=binary"), Some("1")).is_ok());
    assert_eq!(validate_install_headers(Some("text/plain"), Some("1")), Err(Reject::BadContentType));
    assert_eq!(validate_install_headers(Some(CONTENT_TYPE), Some("2")), Err(Reject::BadProtocolHeader));
    assert_eq!(validate_install_headers(Some(CONTENT_TYPE), None), Err(Reject::BadProtocolHeader));
    assert_eq!(validate_length(Some("10")), Err(Reject::TooSmall));
    assert_eq!(validate_length(Some("abc")), Err(Reject::TooSmall));
    assert_eq!(validate_length(Some(&(MAX_BYTES + 1).to_string())), Err(Reject::TooLarge));
    assert_eq!(validate_length(Some("4096")), Ok(4096));
}

#[test]
fn display_text_cannot_disguise_the_confirmation() {
    assert_eq!(sanitize("Haze\u{202e}gnihton\u{200b} skin\n", 120), "Hazegnihton skin");
    assert_eq!(sanitize(&"x".repeat(200), 120).len(), 120);
    assert_eq!(decode_header("Caf%C3%A9%20skin"), "Café skin");
    assert_eq!(decode_header("50%"), "50%", "a broken escape is kept as sent");
    assert_eq!(normalize_kind(Some(" Sound ")), Some("sound".into()));
    assert_eq!(normalize_kind(Some("exe")), None);
}

/// A real listener on a system-picked port, a temp folder and the builds it hands over.
fn bridge(tag: &str) -> (Bridge, mpsc::Receiver<Pending>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("sigil-forge-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let bridge = Bridge::start_on(&[0], &dir, false, move |p| {
        let _ = tx.lock().unwrap().send(p);
    })
    .unwrap();
    (bridge, rx, dir)
}

/// One raw HTTP exchange; returns the status line and the whole response.
fn exchange(port: u16, head: &str, body: &[u8]) -> (String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    (out.lines().next().unwrap_or("").to_string(), out)
}

fn vpk_bytes(len: usize) -> Vec<u8> {
    let mut bytes = VPK_SIGNATURE.to_le_bytes().to_vec();
    bytes.resize(len, 7);
    bytes
}

fn install_head(port: u16, origin: &str, len: usize) -> String {
    format!(
        "POST /forge/v1/install HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: application/octet-stream\r\nx-forge-protocol: 1\r\nx-forge-name: Crimson%20Haze\r\nx-forge-author: Someone\r\nx-forge-type: texture\r\nContent-Length: {len}\r\n\r\n"
    )
}

#[test]
fn a_refused_origin_gets_no_cors_and_the_site_gets_a_ping() {
    let (bridge, _rx, dir) = bridge("ping");
    let port = bridge.port();
    let (status, whole) = exchange(port, &format!("OPTIONS /forge/v1/install HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://evil.com\r\n\r\n"), b"");
    assert!(status.contains("403"), "{status}");
    assert!(!whole.contains("Access-Control-Allow-Origin"));

    let (status, whole) = exchange(port, &format!("OPTIONS /forge/v1/install HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {SITE}\r\n\r\n"), b"");
    assert!(status.contains("204"), "{status}");
    assert!(whole.contains("Access-Control-Allow-Private-Network: true"));

    let (status, whole) = exchange(port, &format!("GET /forge/v1/ping HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {SITE}\r\n\r\n"), b"");
    assert!(status.contains("200"), "{status}");
    assert!(whole.contains("\"app\":\"sigil\""));
    drop(bridge);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_vpk_arrives_for_confirmation_and_junk_does_not() {
    let (bridge, rx, dir) = bridge("install");
    let port = bridge.port();

    let junk = vec![0u8; 2048];
    let (status, _) = exchange(port, &install_head(port, SITE, junk.len()), &junk);
    assert!(status.contains("400"), "not a VPK: {status}");
    assert!(rx.try_recv().is_err());

    // The cooldown from the refused request has to pass before the next one.
    thread::sleep(COOLDOWN + Duration::from_millis(100));
    let body = vpk_bytes(4096);
    let (status, _) = exchange(port, &install_head(port, SITE, body.len()), &body);
    assert!(status.contains("202"), "{status}");
    let pending = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(pending.name, "Crimson Haze");
    assert_eq!(pending.kind.as_deref(), Some("texture"));
    assert_eq!(fs::read(&pending.path).unwrap(), body);

    // One waits for the player: the next build is refused until they answer.
    thread::sleep(COOLDOWN + Duration::from_millis(100));
    let (status, _) = exchange(port, &install_head(port, SITE, body.len()), &body);
    assert!(status.contains("429"), "{status}");
    bridge.answered();
    let (status, _) = exchange(port, &install_head(port, SITE, body.len()), &body);
    assert!(status.contains("202"), "{status}");
    drop(bridge);
    let _ = fs::remove_dir_all(dir);
}
