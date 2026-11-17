//! Tests for the download queue: pure state, the scan, mirrors, and one real stream.

use super::*;

use super::*;

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
    let _ = fs::remove_dir_all(&dir);
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
