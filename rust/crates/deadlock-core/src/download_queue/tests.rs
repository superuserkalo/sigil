//! Tests for the download queue: pure state, the scan, mirrors, and one real stream.

use super::*;
use super::speed::Rate;

fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.scratch/download-test")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn the_queue_round_trips_and_running_rows_come_back_paused() {
    let dir = scratch("queue");
    let mut queue = Queue::default();
    let id = queue.add("https://gamebanana.com/mods/715100").unwrap();
    queue.get_mut(&id).unwrap().state = State::Running;
    queue.get_mut(&id).unwrap().bytes_done = 4_000;
    store_queue(&dir, &queue).unwrap();

    let loaded = load_queue(&dir);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].state, State::Paused);
    assert_eq!(loaded.items[0].bytes_done, 4_000);
    assert!(loaded.items[0].resumable());
    assert!(load_queue(&dir.join("missing")).items.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ids_are_stable_and_targets_are_validated() {
    let mut queue = Queue::default();
    let first = queue.add("715100").unwrap();
    let second = queue.add("https://files.gamebanana.com/mods/a.zip").unwrap();
    assert_eq!((first.as_str(), second.as_str()), ("d1", "d2"));
    queue.remove("d1");
    let local = std::env::temp_dir().join(format!("sigil-queue-{}.zip", std::process::id()));
    fs::write(&local, b"PK").unwrap();
    let third = queue.add(&local.display().to_string()).unwrap();
    assert_eq!(third, "d3", "a removed id is never handed out again");
    let _ = fs::remove_file(&local);
    assert!(queue.add("not a target").is_err());
    let mut big = Queue::default();
    for _ in 0..MAX_ITEMS {
        let id = big.add("1").unwrap();
        big.get_mut(&id).unwrap().state = State::Failed;
    }
    assert!(big.add("1").is_err());
    assert_eq!(big.items.len(), MAX_ITEMS);
}

#[test]
fn pause_resume_cancel_and_retry_only_move_legal_states() {
    let mut queue = Queue::default();
    let id = queue.add("715100").unwrap();
    assert!(queue.pause(&id));
    assert_eq!(queue.get(&id).unwrap().state, State::Paused);
    assert!(!queue.pause(&id));
    assert!(queue.resume(&id));
    assert_eq!(queue.get(&id).unwrap().state, State::Queued);
    assert!(queue.cancel(&id));
    assert_eq!(queue.get(&id).unwrap().state, State::Cancelled);
    assert!(!queue.retry("nope"));
    let item = queue.get_mut(&id).unwrap();
    item.bytes_done = 900;
    assert!(queue.retry(&id));
    assert_eq!(queue.get(&id).unwrap().bytes_done, 0);
    queue.get_mut(&id).unwrap().state = State::Queued;
    assert_eq!(queue.next_runnable().as_deref(), Some(id.as_str()));
    queue.paused = true;
    assert!(queue.next_runnable().is_none());
}

#[test]
fn finished_rows_clear_and_the_queue_trims() {
    let mut queue = Queue::default();
    for _ in 0..4 {
        let id = queue.add("1").unwrap();
        queue.get_mut(&id).unwrap().state = State::Done;
    }
    assert_eq!(queue.done(), 4);
    assert!(queue.clear_finished());
    assert!(queue.items.is_empty());
    assert!(!queue.clear_finished());

    let mut crowded = Queue::default();
    for index in 0..(MAX_ITEMS + 3) {
        let mut item = Item::new(format!("d{index}"), "1".to_string());
        item.state = if index % 2 == 0 { State::Done } else { State::Failed };
        crowded.items.push(item);
    }
    crowded.trim();
    assert_eq!(crowded.items.len(), MAX_ITEMS);
}

#[test]
fn scan_flags_programs_shortcuts_gameinfo_and_odd_vpks() {
    let report = scan_entries(
        &[
            "citadel/mod_readme.txt".to_string(),
            "citadel/installer.exe".to_string(),
            "citadel/download.url".to_string(),
            "citadel/gameinfo.gi".to_string(),
            "citadel/mod_files/thing.vpk".to_string(),
            "citadel/pak03_dir.vpk".to_string(),
        ],
        4096,
    );
    assert!(report.checked);
    assert_eq!(report.files, 6);
    assert_eq!(report.warnings.len(), 4);
    assert!(report.suspicious());
    assert!(report.summary().contains("installer.exe"));
    assert!(scan_entries(&["citadel/pak01_dir.vpk".into()], 10).warnings.is_empty());
}

#[test]
fn a_zip_is_scanned_without_unpacking_it() {
    let dir = scratch("zip");
    let archive = dir.join("mod.zip");
    fs::write(
        &archive,
        stored_zip(&[
            ("citadel/materials/a.vmat_c", b"x"),
            ("citadel/run.bat", b"echo hi"),
        ]),
    )
    .unwrap();
    let report = scan_archive(&archive, &dir.join("scan"));
    assert!(report.checked);
    assert_eq!(report.files, 2);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].message.contains("run.bat"));
    assert!(!dir.join("scan").exists(), "a ZIP is never unpacked");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_archive_is_reported_not_hidden() {
    let dir = scratch("missing");
    let report = scan_archive(&dir.join("gone.zip"), &dir.join("scan"));
    assert!(!report.checked);
    assert!(report.error.is_some());
    assert!(!report.suspicious());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mirrors_are_only_built_for_gamebanana_file_hosts() {
    let servers = vec!["filecache9".to_string(), "filecache4".to_string(), "evil".to_string()];
    assert_eq!(
        failover_candidates("https://files.gamebanana.com/mods/a.zip", &servers),
        vec![
            "https://files.gamebanana.com/mods/a.zip",
            "https://filecache9.gamebanana.com/mods/a.zip",
            "https://filecache4.gamebanana.com/mods/a.zip",
        ]
    );
    assert_eq!(
        failover_candidates("https://filecache2.gamebanana.com/mods/a.zip", &servers).len(),
        3
    );
    assert_eq!(
        failover_candidates("https://example.com/a.zip", &servers),
        vec!["https://example.com/a.zip"]
    );
    assert!(failover_candidates("not a url", &servers).is_empty());
}

#[test]
fn sizes_and_names_stay_plain() {
    assert_eq!(size(0), "0 B");
    assert_eq!(size(1_500), "1 KB");
    assert_eq!(size(1024 * 1024 * 3 / 2), "1.5 MB");
    assert_eq!(safe_file_name("mod.zip").as_deref(), Some("mod.zip"));
    assert!(safe_file_name("../mod.zip").is_none());
    assert!(safe_file_name("a/b.zip").is_none());
    assert!(safe_file_name("").is_none());
    assert_eq!(stem("billy_recolor-2.zip"), "billy recolor 2");
}

/// A minimal stored ZIP, the same shape the install tests build.
fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    let mut offsets = Vec::new();
    for (name, data) in entries {
        let crc = crc32fast::hash(data);
        offsets.push(output.len() as u32);
        output.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        output.extend_from_slice(&20_u16.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&crc.to_le_bytes());
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
        output.extend_from_slice(&(name.len() as u16).to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(data);
    }
    let central_start = output.len() as u32;
    for (index, (name, data)) in entries.iter().enumerate() {
        let crc = crc32fast::hash(data);
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&(((3_u16) << 8) | 20).to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&(0o100644_u32 << 16).to_le_bytes());
        central.extend_from_slice(&offsets[index].to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    output.extend_from_slice(&central);
    output.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    output.extend_from_slice(&(central.len() as u32).to_le_bytes());
    output.extend_from_slice(&central_start.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output
}

#[test]
fn a_stream_writes_the_bytes_and_reports_progress() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    let body: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = body.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let body = served.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });

    let dir = scratch("stream");
    let path = dir.join("a.zip");
    let policy = DownloadPolicy::gamebanana()
        .with_mirror(&format!("http://127.0.0.1:{port}"))
        .unwrap();
    let control = Control::new();
    let mut seen = Vec::new();
    let end = stream(
        &agent(&policy),
        &format!("http://127.0.0.1:{port}/a.zip"),
        &path,
        &policy,
        0,
        &control,
        &mut |done, total| seen.push((done, total)),
    )
    .unwrap();
    match end {
        StreamEnd::Done { bytes, sha256 } => {
            let expected: String = Sha256::digest(&body)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(bytes, body.len() as u64);
            assert_eq!(sha256, expected);
        }
        StreamEnd::Cancelled => panic!("cancelled"),
    }
    assert_eq!(fs::read(&path).unwrap(), body);
    assert!(!part_path(&path).exists());
    assert!(seen.last().unwrap().0 >= body.len() as u64 - READ_BUFFER as u64);
    assert_eq!(control.server().as_deref(), Some("127.0.0.1"), "the file server is known while it streams");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn waiting_items_have_a_place_in_line() {
    let mut queue = Queue::default();
    let a = queue.add("715100").unwrap();
    let b = queue.add("715101").unwrap();
    let c = queue.add("715102").unwrap();
    queue.get_mut(&b).unwrap().mod_id = Some(715101);
    assert_eq!(queue.position(&a), Some(1));
    assert_eq!(queue.position(&c), Some(3));
    queue.get_mut(&a).unwrap().state = State::Running;
    assert_eq!(queue.position(&a), None, "a running item is not waiting");
    assert_eq!(queue.position(&b), Some(1));
    assert_eq!(queue.positions().get(&c), Some(&2));
    let (item, place) = queue.for_mod(715101).unwrap();
    assert_eq!((item.id.as_str(), place), (b.as_str(), Some(1)));
    let (item, place) = queue.for_mod(715102).unwrap();
    assert_eq!((item.id.as_str(), place), (c.as_str(), Some(2)), "a bare id target matches too");
    assert!(queue.for_mod(1).is_none());
    assert_eq!(queue.position("missing"), None);
}

#[test]
fn cancelling_removes_the_partial_file() {
    let dir = scratch("cancel");
    let policy = DownloadPolicy::gamebanana();
    let control = Control::new();
    control.cancel();
    let path = dir.join("a.zip");
    let end = stream(
        &agent(&policy),
        "http://127.0.0.1:9/a.zip",
        &path,
        &policy,
        0,
        &control,
        &mut |_, _| {},
    );
    // The request itself fails before the cancel is seen, either way nothing survives.
    assert!(end.is_err() || end.unwrap() == StreamEnd::Cancelled);
    assert!(!part_path(&path).exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn commands_for_other_rows_wait_for_the_queue_loop_instead_of_being_lost() {
    let control = Control::new();
    // An Add for another mod while an archive streams must reach the queue loop.
    assert!(handle_mid_stream(&Command::Add("715100".into()), "d1", &control));
    assert!(handle_mid_stream(&Command::Retry("d2".into()), "d1", &control));
    assert!(handle_mid_stream(&Command::Remove("d2".into()), "d1", &control));
    // Pausing or cancelling another row leaves the running one alone.
    assert!(handle_mid_stream(&Command::Pause("d2".into()), "d1", &control));
    assert!(handle_mid_stream(&Command::Cancel("d2".into()), "d1", &control));
    assert!(!control.paused() && !control.cancelled());
    // The running row's own controls act at once and are done with.
    assert!(!handle_mid_stream(&Command::Pause("d1".into()), "d1", &control));
    assert!(control.paused());
    assert!(!handle_mid_stream(&Command::Resume("d1".into()), "d1", &control));
    assert!(!control.paused());
    assert!(!handle_mid_stream(&Command::Cancel("d1".into()), "d1", &control));
    assert!(control.cancelled());
}

#[test]
fn a_row_knows_when_it_was_added() {
    let before = Utc::now().timestamp_millis();
    let item = Item::new("d1".into(), "715100".into());
    let after = Utc::now().timestamp_millis();
    let added = item.added_ms().unwrap();
    assert!(before <= added && added <= after);
}

// ---------------------------------------------------------------------------
// Speed, time left and order
// ---------------------------------------------------------------------------

const MB: u64 = 1024 * 1024;

#[test]
fn speed_is_smoothed_across_sample_windows() {
    let mut rate = Rate::default();
    rate.observe(0, Duration::ZERO);
    // A chunk that lands inside the sample window is not a speed on its own.
    rate.observe(512 * 1024, Duration::from_millis(120));
    assert_eq!(rate.bps(), None, "the window has not passed yet");

    rate.observe(MB, Duration::from_millis(500));
    let first = rate.bps().unwrap();
    assert_eq!(first, 2 * MB, "1 MiB in half a second");

    // A steady transfer keeps the average on the real rate.
    for step in 1..=4 {
        rate.observe(MB + step * MB, Duration::from_millis(500 + step * 500));
    }
    let steady = rate.bps().unwrap();
    assert_eq!(steady, 2 * MB);

    // One very fast window lifts the number without jumping to it: the newest reading counts
    // for three tenths, so 34 MiB/s after 2 MiB/s reads as about 11.6 MiB/s.
    rate.observe(22 * MB, Duration::from_millis(3_000));
    let burst = rate.bps().unwrap();
    assert!(burst > steady, "a fast window lifts the number");
    assert!(burst < 20 * MB, "smoothing keeps one burst from taking over: {burst}");

    // One slow window pulls it down gently rather than snapping to the new rate.
    rate.observe(22 * MB + MB / 2, Duration::from_millis(3_500));
    let dip = rate.bps().unwrap();
    assert!(dip < burst && dip > 5 * MB, "no jitter on one slow window: {dip}");
}

#[test]
fn a_restarted_file_clears_the_speed() {
    let mut rate = Rate::default();
    rate.observe(0, Duration::ZERO);
    rate.observe(4 * MB, Duration::from_millis(500));
    assert_eq!(rate.bps(), Some(8 * MB));
    // A failover that starts the file again must not read as a negative or tiny rate.
    rate.observe(0, Duration::from_millis(700));
    assert_eq!(rate.bps(), None);
}

#[test]
fn eta_needs_a_total_and_a_speed() {
    let mut item = Item::new("d1".into(), "715100".into());
    item.state = State::Running;
    item.bytes_total = 10 * MB;
    item.bytes_done = MB;
    assert_eq!(item.eta_secs(), None, "no sample yet");

    item.rate.observe(0, Duration::ZERO);
    item.rate.observe(MB, Duration::from_millis(500));
    assert_eq!(item.speed_bps(), Some(2 * MB));
    assert_eq!(item.eta_secs(), Some(5), "9 MiB left at 2 MiB/s rounds up");

    item.bytes_total = 0;
    assert_eq!(item.eta_secs(), None, "the size is unknown");
    item.bytes_total = item.bytes_done;
    assert_eq!(item.eta_secs(), None, "nothing is left");
    item.bytes_total = 10 * MB;
    item.state = State::Paused;
    assert_eq!(item.eta_secs(), None);
    assert_eq!(item.speed_bps(), None);
}

#[test]
fn a_crawl_is_flagged_after_eight_seconds_and_clears_on_recovery() {
    let mut item = Item::new("d1".into(), "715100".into());
    item.state = State::Running;
    item.bytes_total = 100 * MB;
    // 64 KiB every 512 ms is 125 KB/s, under the 150 KB/s threshold.
    let chunk = 64 * 1024;
    item.rate.observe(0, Duration::ZERO);
    for step in 1..=16 {
        item.rate.observe(step * chunk, Duration::from_millis(step * 512));
        assert!(!item.is_slow(), "a short crawl is not a warning (step {step})");
    }
    item.rate.observe(17 * chunk, Duration::from_millis(17 * 512));
    assert!(item.is_slow(), "8 seconds under the threshold is a crawl");

    // One fast window clears the flag instead of holding it for the rest of the transfer.
    item.rate.observe(17 * chunk + 8 * MB, Duration::from_millis(18 * 512));
    assert!(!item.is_slow(), "a recovery clears the flag");
    assert!(item.speed_bps().unwrap() > SLOW_BPS);

    // Nearly done: no notice, it finishes before one would help (Deadlock Mod Manager's rule).
    let mut nearly = Item::new("d2".into(), "715101".into());
    nearly.state = State::Running;
    nearly.bytes_total = 100 * MB;
    nearly.bytes_done = 98 * MB;
    nearly.rate = item.rate;
    nearly.rate.observe(18 * chunk, Duration::from_millis(40 * 512));
    nearly.rate.observe(19 * chunk, Duration::from_millis(60 * 512));
    assert!(!nearly.is_slow());

    item.state = State::Paused;
    assert!(!item.is_slow());
    assert_eq!(item.speed_bps(), None);
}

#[test]
fn a_pause_restarts_the_slow_window() {
    let mut rate = Rate::default();
    let chunk = 32 * 1024;
    rate.observe(0, Duration::ZERO);
    for step in 1..=20_u64 {
        rate.observe(step * chunk, Duration::from_millis(step * 256));
    }
    assert!(!rate.slow());

    // Five silent seconds, then the same crawl. Readings start again from the resume, so the
    // window cannot span the pause and fire early.
    let resume = 5_120 + 5_000;
    rate.observe(20 * chunk, Duration::from_millis(resume));
    for step in 1..=32_u64 {
        rate.observe(20 * chunk + step * chunk, Duration::from_millis(resume + step * 256));
        assert!(!rate.slow(), "the window spans no pause (step {step})");
    }
    rate.observe(20 * chunk + 33 * chunk, Duration::from_millis(resume + 33 * 256));
    assert!(rate.slow(), "a fresh 8 seconds of crawl still fires");
}

#[test]
fn a_waiting_row_moves_and_a_started_one_does_not() {
    let mut queue = Queue::default();
    for _ in 0..3 {
        queue.add("715100").unwrap();
    }
    assert_eq!(order(&queue), vec!["d1", "d2", "d3"]);

    queue.move_to("d3", 0).unwrap();
    assert_eq!(order(&queue), vec!["d3", "d1", "d2"]);
    assert_eq!(queue.next_runnable().as_deref(), Some("d3"), "the worker picks the new first");

    queue.move_to("d3", 99).unwrap();
    assert_eq!(order(&queue), vec!["d1", "d2", "d3"], "an index past the end lands last");
    queue.move_to("d1", 1).unwrap();
    assert_eq!(order(&queue), vec!["d2", "d1", "d3"]);

    queue.get_mut("d1").unwrap().state = State::Running;
    assert!(queue.move_to("d1", 0).is_err(), "the running row keeps its place");
    queue.get_mut("d2").unwrap().state = State::Paused;
    assert!(queue.move_to("d2", 2).is_err(), "a paused row has already started");
    queue.get_mut("d3").unwrap().state = State::Done;
    assert!(queue.move_to("d3", 0).is_err(), "a finished row keeps its place");
    assert!(queue.move_to("nope", 0).is_err());
    assert_eq!(order(&queue), vec!["d2", "d1", "d3"], "a refused move changes nothing");
}

#[test]
fn a_speed_reading_never_reaches_the_queue_file() {
    let dir = scratch("speed-file");
    let mut queue = Queue::default();
    let id = queue.add("715100").unwrap();
    let item = queue.get_mut(&id).unwrap();
    item.state = State::Running;
    item.rate.observe(0, Duration::ZERO);
    item.rate.observe(4 * MB, Duration::from_millis(500));
    assert_eq!(item.speed_bps(), Some(8 * MB));
    store_queue(&dir, &queue).unwrap();
    let text = fs::read_to_string(queue_path(&dir)).unwrap();
    assert!(!text.contains("\"rate\""), "speed is runtime state, not queue data");
    let loaded = load_queue(&dir);
    assert_eq!(loaded.items[0].speed_bps(), None);
    let _ = fs::remove_dir_all(&dir);
}

fn order(queue: &Queue) -> Vec<String> {
    queue.items.iter().map(|item| item.id.clone()).collect()
}


/// A file server that honours `Range` only while `If-Range` names its current `ETag` ("v1").
fn range_server(body: Vec<u8>) -> u16 {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let body = body.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut from, mut if_range) = (None, None);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(rest) = lower.strip_prefix("range: bytes=") {
                        from = rest.trim().trim_end_matches('-').parse::<usize>().ok();
                    }
                    if lower.starts_with("if-range:") {
                        if_range = Some(line["if-range:".len()..].trim().to_string());
                    }
                }
                let head = match from {
                    Some(start) if if_range.as_deref() == Some("\"v1\"") => format!(
                        "HTTP/1.1 206 Partial Content\r\nETag: \"v1\"\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len() - 1,
                        body.len(),
                        body.len() - start
                    ),
                    _ => format!("HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
                };
                let start = match from {
                    Some(start) if head.starts_with("HTTP/1.1 206") => start,
                    _ => 0,
                };
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body[start..]);
            });
        }
    });
    port
}

#[test]
fn a_resume_continues_while_the_file_is_the_same_and_starts_over_when_it_changed() {
    let body: Vec<u8> = (0..120_000).map(|i| (i % 241) as u8).collect();
    let port = range_server(body.clone());
    let dir = scratch("resume");
    let path = dir.join("a.zip");
    let policy = DownloadPolicy::gamebanana().with_mirror(&format!("http://127.0.0.1:{port}")).unwrap();
    let url = format!("http://127.0.0.1:{port}/a.zip");

    // The bytes a closed Sigil left, from the same file: only the rest is fetched.
    fs::write(part_path(&path), &body[..50_000]).unwrap();
    let control = Control::new();
    control.set_validator(Some("\"v1\"".into()));
    let mut first = None;
    stream(&agent(&policy), &url, &path, &policy, 50_000, &control, &mut |done, _| {
        first.get_or_insert(done);
    })
    .unwrap();
    assert_eq!(first, Some(50_000), "it picked up where the part file ends");
    assert_eq!(fs::read(&path).unwrap(), body);

    // The file changed since (another validator): the server sends all of it and the stream
    // starts over instead of appending to bytes from another file.
    let _ = fs::remove_file(&path);
    fs::write(part_path(&path), vec![7u8; 50_000]).unwrap();
    let control = Control::new();
    control.set_validator(Some("\"v0\"".into()));
    stream(&agent(&policy), &url, &path, &policy, 50_000, &control, &mut |_, _| {}).unwrap();
    assert_eq!(fs::read(&path).unwrap(), body);
    assert_eq!(control.validator().as_deref(), Some("\"v1\""), "the new validator is kept");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn validators_skip_weak_tags_and_odd_bytes() {
    assert_eq!(validator_of(Some("\"abc\""), Some("Tue, 01 Jan 2026 00:00:00 GMT")).as_deref(), Some("\"abc\""));
    assert_eq!(validator_of(Some("W/\"abc\""), Some("Tue, 01 Jan 2026 00:00:00 GMT")).as_deref(), Some("Tue, 01 Jan 2026 00:00:00 GMT"));
    assert_eq!(validator_of(None, None), None);
    assert_eq!(validator_of(Some("\"a\r\nX: y\""), None), None, "no header can ride along");
}

#[test]
fn a_restart_lists_cut_off_waiting_and_uninstalled_rows() {
    let dir = scratch("restart");
    let mut queue = Queue::default();
    let streaming = queue.add("715100").unwrap();
    let waiting = queue.add("715101").unwrap();
    let paused = queue.add("715102").unwrap();
    let landed = queue.add("715103").unwrap();
    let done = queue.add("715104").unwrap();
    queue.get_mut(&streaming).unwrap().state = State::Running;
    queue.get_mut(&paused).unwrap().state = State::Paused;
    queue.get_mut(&landed).unwrap().state = State::Done;
    queue.get_mut(&landed).unwrap().install = true;
    queue.get_mut(&done).unwrap().state = State::Done;
    store_queue(&dir, &queue).unwrap();
    let left: Vec<String> = left_after_restart(&dir).into_iter().map(|i| i.id).collect();
    assert_eq!(left, vec![streaming.clone(), waiting, landed], "a paused row the player paused stays out");
    let mut loaded = load_queue(&dir);
    assert!(loaded.get(&streaming).unwrap().interrupted);
    assert!(loaded.resume(&streaming));
    assert!(!loaded.get(&streaming).unwrap().interrupted, "Resume clears the mark");
    let _ = fs::remove_dir_all(&dir);
}
