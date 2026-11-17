use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::*;

/// Scratch root under `.scratch/servers-test`, cleared at the start of each test.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.scratch/servers-test")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn cstr(value: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

/// A directory VPK (v2) listing `paths`, with no file data.
fn vpk_bytes(paths: &[&str]) -> Vec<u8> {
    let mut by_ext: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for path in paths {
        let (dir, file) = path.rsplit_once('/').unwrap_or((" ", path));
        let (stem, ext) = file.rsplit_once('.').unwrap();
        by_ext
            .entry(ext.to_string())
            .or_default()
            .entry(dir.to_string())
            .or_default()
            .push(stem.to_string());
    }
    let mut tree = Vec::new();
    for (ext, dirs) in by_ext {
        cstr(&ext, &mut tree);
        for (dir, stems) in dirs {
            cstr(&dir, &mut tree);
            for stem in stems {
                cstr(&stem, &mut tree);
                let start = tree.len();
                tree.extend_from_slice(&[0u8; 18]);
                tree[start + 6..start + 8]
                    .copy_from_slice(&crate::vpk::VPK_DIRECTORY_ARCHIVE.to_le_bytes());
                tree[start + 16..start + 18].copy_from_slice(&0xffff_u16.to_le_bytes());
            }
            cstr("", &mut tree);
        }
        cstr("", &mut tree);
    }
    cstr("", &mut tree);
    let mut out = Vec::new();
    out.extend_from_slice(&crate::vpk::VPK_MAGIC.to_le_bytes());
    out.extend_from_slice(&2_u32.to_le_bytes());
    out.extend_from_slice(&(tree.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&tree);
    out
}

fn sha_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn zstd_bytes(bytes: &[u8]) -> Vec<u8> {
    zstd::encode_all(bytes, 3).unwrap()
}

fn bzip2_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A tiny HTTP/1.1 server on loopback. `routes` maps a path to (status, extra headers, body).
type Route = (u16, Vec<(String, String)>, Vec<u8>);

fn http_server(routes: HashMap<String, Route>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes = Arc::new(routes);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    return;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, headers, body) = routes
                    .get(&path)
                    .cloned()
                    .unwrap_or((404, Vec::new(), b"missing".to_vec()));
                let mut head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    base
}

const STOCK_GAMEINFO: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\t// language paths\n\t\t\tGame_Language \"citadel_*LANGUAGE*\"\n\t\t\tGame_LowViolence \"citadel_lv\"\n\n\t\t\tGame citadel/addons\n\t\t\tGame \"citadel\"\n\t\t\tGame \"core\"\n\t\t}\n\t}\n}\n";

// --- list --------------------------------------------------------------------------------------

const STEAM_JSON: &str = r#"{"servers":[
 {"addr":"10.0.0.1:27015","gameport":27015,"steamid":"9001","name":"Arena EU","players":7,"max_players":31,"bots":0,"map":"dl_midtown","gametype":"dw1,insecure"},
 {"addr":"10.0.0.2:27016","gameport":27016,"steamid":"9002","name":"Full House","players":8,"max_players":8,"map":"dl_harbor","gametype":"empty,dw1,dwa4ac96b19,insecure"},
 {"addr":"10.0.0.3:27015","steamid":"9003","name":"Hidden","players":0,"max_players":31,"map":"dl_midtown","gametype":"dw1,dwu"},
 {"addr":"10.0.0.4:27015","steamid":"9004","name":"Vanilla","players":3,"max_players":12,"map":"dl_midtown","gametype":"empty"},
 {"addr":"not an address","steamid":"9005","name":"Broken","players":0,"max_players":1,"map":"x","gametype":"dw1"},
 {"addr":"10.0.0.6:27015","gameport":28000,"steamid":"9006","name":"","players":0,"max_players":31,"map":"start","gametype":"dw1"}
]}"#;

const REGISTRY_JSON: &str = r#"{"servers":[
 {"id":"a","name":"Arena EU","raw_address":"10.0.0.1:27015","country":"lt","online":true,"reachable":true,"player_count":7,"max_players":31,"map":"dl_midtown","version":"v0.6.4","players":[{"name":"ash","hero":"Calico","team":2,"kills":3,"deaths":1,"assists":4,"level":1}],"content_addons":[],"extra_maps":["overthrow10"]},
 {"id":"b","name":"Gone","raw_address":"10.0.0.9:27015","country":"US","online":false,"reachable":false,"player_count":0,"max_players":31,"map":"dl_midtown","version":"v0.6.4"}
]}"#;

#[test]
fn list_keeps_deadworks_servers_and_merges_the_registry() {
    let servers = parse_server_list(STEAM_JSON, Some(REGISTRY_JSON)).unwrap();
    let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Arena EU", "Full House", "Unnamed server"]);

    let arena = &servers[0];
    assert_eq!(arena.id, "9001");
    assert_eq!(arena.address, "10.0.0.1:27015");
    assert_eq!(arena.country, "LT");
    assert_eq!(arena.version, "0.6.4");
    assert!(arena.registered);
    assert_eq!(arena.roster.len(), 1);
    assert_eq!(arena.roster[0].hero, "Calico");
    assert_eq!(arena.content_hint, ["overthrow10"]);
    assert!(!arena.has_addons);

    let full = &servers[1];
    assert!(full.is_full());
    assert!(full.has_addons);
    assert!(!full.registered);
    assert_eq!(full.country, "");

    // The game port wins over the query port in `addr`.
    assert_eq!(servers[2].address, "10.0.0.6:28000");
}

#[test]
fn list_survives_a_missing_registry_and_falls_back_to_it() {
    let servers = parse_server_list(STEAM_JSON, None).unwrap();
    assert_eq!(servers.len(), 3);
    assert!(servers.iter().all(|s| !s.registered));

    let registry = parse_registry_only(REGISTRY_JSON).unwrap();
    assert_eq!(registry.len(), 1, "offline servers are dropped");
    assert_eq!(registry[0].address, "10.0.0.1:27015");
}

#[test]
fn sort_puts_open_then_busy_then_fast_first() {
    let mut servers = parse_server_list(STEAM_JSON, None).unwrap();
    servers.push(Server {
        id: "x".into(),
        name: "Another".into(),
        ..servers[2].clone()
    });
    let pings = HashMap::from([
        ("9006".to_string(), Some(80)),
        ("x".to_string(), Some(20)),
    ]);
    sort_servers(&mut servers, &pings);
    let ids: Vec<&str> = servers.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["9001", "x", "9006", "9002"]);
}

// --- A2S ---------------------------------------------------------------------------------------

fn rules_packet(rules: &[(&str, &str)]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFF, 0xFF, 0xFF, 0x45];
    out.extend_from_slice(&(rules.len() as u16).to_le_bytes());
    for (k, v) in rules {
        cstr(k, &mut out);
        cstr(v, &mut out);
    }
    out
}

#[test]
fn rules_replies_parse_and_junk_is_ignored() {
    assert_eq!(
        parse_rules_reply(&[0xFF, 0xFF, 0xFF, 0xFF, 0x41, 1, 2, 3, 4]),
        Some(RulesReply::Challenge([1, 2, 3, 4]))
    );
    let Some(RulesReply::Rules(rules)) = parse_rules_reply(&rules_packet(&[("dw_ver", "1"), ("a", "b")]))
    else {
        panic!("rules")
    };
    assert_eq!(rules.get("dw_ver").map(String::as_str), Some("1"));
    // A count larger than the pairs present stops at the end of the packet.
    let mut short = rules_packet(&[("k", "v")]);
    short[5] = 9;
    let Some(RulesReply::Rules(rules)) = parse_rules_reply(&short) else { panic!() };
    assert_eq!(rules.len(), 1);
    assert_eq!(parse_rules_reply(&[0xFE, 0xFF, 0xFF, 0xFF, 0x45, 0, 0]), None);
    assert_eq!(parse_rules_reply(&[0xFF, 0xFF]), None);
}

fn rules_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn advertisement_validates_every_entry() {
    assert_eq!(parse_advertisement(&rules_map(&[("dw_ver", "2")])), None);
    let ad = parse_advertisement(&rules_map(&[
        ("dw_ver", "1"),
        (
            "dw_addons",
            "harbor:0123456789abcdef:2048,BAD:0123456789abcdef,nohash,con:0123456789abcdef,x:0123456789ABCDEF,y:0123456789abcdef:12a",
        ),
        ("dw_addons_n", "7"),
        ("dw_maps", "dl_midtown:fedcba9876543210,st_dome_a2:00112233445566aa:999"),
        ("dw_fastdl", " https://cdn.example.com/dw/ "),
    ]))
    .unwrap();
    let keys: Vec<String> = ad.entries.iter().map(ContentEntry::key).collect();
    assert_eq!(keys, ["addons/harbor", "maps/dl_midtown", "maps/st_dome_a2"]);
    assert_eq!(ad.entries[0].size, Some(2048));
    assert!(ad.truncated);
    assert_eq!(ad.fastdl.as_deref(), Some("https://cdn.example.com/dw"));
}

#[test]
fn names_hashes_and_fastdl_urls_are_strict() {
    for good in ["a", "dl_midtown", "com0", "com10", "lpt", "x_1"] {
        assert!(valid_content_name(good), "{good}");
    }
    for bad in ["", "A", "a-b", "a.b", "../x", "con", "nul", "com1", "lpt9", &"a".repeat(65)] {
        assert!(!valid_content_name(bad), "{bad}");
    }
    assert_eq!(parse_fastdl("http://h.example/x").as_deref(), Some("http://h.example/x"));
    for bad in [
        "ftp://h.example",
        "https://user:pw@h.example",
        "https://h.example/?a=1",
        "https://h.example/#f",
        "/relative",
        "",
    ] {
        assert_eq!(parse_fastdl(bad), None, "{bad}");
    }
}

#[test]
fn query_rules_echoes_the_challenge_and_ping_times_a_reply() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        loop {
            let Ok((len, from)) = server.recv_from(&mut buf) else { return };
            let packet = &buf[..len];
            if packet[4] == b'T' {
                let _ = server.send_to(&[0xFF, 0xFF, 0xFF, 0xFF, 0x41, 9, 9, 9, 9], from);
            } else if packet[5..9] == [0xFF; 4] {
                let _ = server.send_to(&[0xFF, 0xFF, 0xFF, 0xFF, 0x41, 7, 7, 7, 7], from);
            } else if packet[5..9] == [7; 4] {
                let _ = server.send_to(&rules_packet(&[("dw_ver", "1"), ("dw_maps", "m:0123456789abcdef")]), from);
            }
        }
    });
    let rules = query_rules(addr, Duration::from_secs(2)).unwrap().unwrap();
    let ad = parse_advertisement(&rules).unwrap();
    assert_eq!(ad.entries[0].name, "m");
    assert!(ping(addr, Duration::from_secs(1)).is_some());
}

#[test]
fn a_silent_server_advertises_nothing() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = silent.local_addr().unwrap();
    assert_eq!(query_rules(addr, Duration::from_millis(300)).unwrap(), None);
    assert_eq!(ping(addr, Duration::from_millis(200)), None);
    drop(silent);
}

// --- plan --------------------------------------------------------------------------------------

fn entry(kind: ContentKind, name: &str, data: &[u8], with_size: bool) -> ContentEntry {
    ContentEntry {
        kind,
        name: name.to_string(),
        hash: sha_hex(data)[..16].to_string(),
        size: with_size.then_some(data.len() as u64),
    }
}

#[test]
fn plan_marks_current_missing_foreign_and_unavailable_files() {
    let root = scratch("plan");
    let paths = deadworks_paths(&root);
    let current = vpk_bytes(&["maps/a.vmap_c"]);
    let stock = vpk_bytes(&["maps/stock.vmap_c"]);
    fs::create_dir_all(&paths.maps_dir).unwrap();
    fs::create_dir_all(&paths.addons_dir).unwrap();
    fs::write(paths.maps_dir.join("current_map.vpk"), &current).unwrap();
    fs::write(paths.maps_dir.join("dl_midtown.vpk"), &stock).unwrap();
    fs::write(paths.addons_dir.join("old_addon.vpk"), b"an older build").unwrap();

    let ad = Advertisement {
        entries: vec![
            entry(ContentKind::Map, "current_map", &current, true),
            entry(ContentKind::Map, "dl_midtown", b"the server's midtown", false),
            entry(ContentKind::Addon, "old_addon", b"new build", true),
            entry(ContentKind::Addon, "fresh", b"fresh", true),
        ],
        truncated: false,
        fastdl: Some("https://cdn.example.com".into()),
    };
    let plan = plan_content(&root, &ad).unwrap();
    let actions: Vec<ItemAction> = plan.items.iter().map(|i| i.action).collect();
    assert_eq!(
        actions,
        [
            ItemAction::Current,
            ItemAction::KeepLocal,
            ItemAction::Download,
            ItemAction::Download
        ]
    );
    assert_eq!(plan.download_size(), (9 + 5, 0));
    assert_eq!(plan.fastdl_host().as_deref(), Some("cdn.example.com"));
    // The size differed for old_addon, so it was not hashed.
    assert!(plan.hashed.contains_key("maps/current_map"));
    assert!(!plan.hashed.contains_key("addons/old_addon"));

    let no_host = plan_content(&root, &Advertisement { fastdl: None, ..ad }).unwrap();
    assert_eq!(no_host.items[3].action, ItemAction::Unavailable);
    assert_eq!(no_host.items[0].action, ItemAction::Current);
}

// --- decode and safety -------------------------------------------------------------------------

#[test]
fn decode_reads_zstd_and_multi_stream_bzip2_and_refuses_the_rest() {
    let root = scratch("decode");
    let vpk = vpk_bytes(&["panorama/layout/a.vxml_c"]);
    let out = root.join("out.vpk");

    let zst = root.join("a.zst");
    fs::write(&zst, [zstd_bytes(&vpk[..10]), zstd_bytes(&vpk[10..])].concat()).unwrap();
    let decoded = decode_vpk(&zst, &out, MAX_FILE_BYTES).unwrap();
    assert_eq!(decoded.bytes, vpk.len() as u64);
    assert_eq!(decoded.sha256, sha_hex(&vpk));

    let bz = root.join("a.bz2");
    fs::write(&bz, [bzip2_bytes(&vpk[..7]), bzip2_bytes(&vpk[7..])].concat()).unwrap();
    assert_eq!(decode_vpk(&bz, &out, MAX_FILE_BYTES).unwrap().sha256, sha_hex(&vpk));

    let small_cap = decode_vpk(&zst, &out, 8).unwrap_err().to_string();
    assert!(small_cap.contains("over the limit"), "{small_cap}");
    assert!(!out.exists());

    let not_vpk = root.join("b.zst");
    fs::write(&not_vpk, zstd_bytes(b"hello world")).unwrap();
    assert!(decode_vpk(&not_vpk, &out, MAX_FILE_BYTES).unwrap_err().to_string().contains("not a VPK"));

    let plain = root.join("c.bin");
    fs::write(&plain, &vpk).unwrap();
    assert!(decode_vpk(&plain, &out, MAX_FILE_BYTES).is_err());
}

#[test]
fn safety_gate_blocks_executables_and_non_ui_bootstrap_files() {
    let root = scratch("safety");
    let ok = root.join("ok.vpk");
    fs::write(&ok, vpk_bytes(&["maps/x.vmap_c", "scripts/abilities.vdata_c"])).unwrap();
    assert_eq!(check_server_vpk(&ok).unwrap(), 2);
    assert!(check_bootstrap_vpk(&ok).is_err(), "game data is not allowed in the UI pack");

    let exe = root.join("exe.vpk");
    fs::write(&exe, vpk_bytes(&["bin/tool.exe"])).unwrap();
    assert!(check_server_vpk(&exe).is_err());

    let ui = root.join("ui.vpk");
    fs::write(
        &ui,
        vpk_bytes(&["panorama/layout/hud_hideout.vxml_c", "panorama/scripts/dw_bootstrap.vjs_c"]),
    )
    .unwrap();
    assert_eq!(check_bootstrap_vpk(&ui).unwrap(), 2);

    let source = root.join("src.vpk");
    fs::write(&source, vpk_bytes(&["panorama/layout/raw.xml"])).unwrap();
    assert!(check_bootstrap_vpk(&source).is_err());
}

// --- gameinfo ----------------------------------------------------------------------------------

#[test]
fn gameinfo_lines_go_above_the_first_game_entry() {
    let before = scan_gameinfo(STOCK_GAMEINFO);
    assert!(before.search_paths && !before.ready());

    let updated = add_deadworks_lines(STOCK_GAMEINFO).unwrap().unwrap();
    let after = scan_gameinfo(&updated);
    assert!(after.ready());
    let order: Vec<&str> = updated
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let at = |needle: &str| order.iter().position(|l| l.starts_with(needle)).unwrap();
    assert!(at("Game_LowViolence") < at(GAMEINFO_MARK_START));
    assert!(at("Game\t\tcitadel/deadworks_mods") < at("Mod\t\tcitadel"));
    assert!(at("Write\t\tcitadel") < at("addonroot"));
    assert!(at(GAMEINFO_MARK_END) < at("Game citadel/addons"));
    assert!(order.contains(&"Game \"citadel\""));
    // Indentation follows the first Game line.
    assert!(updated.contains("\t\t\tGame\t\tcitadel/deadworks_mods\n"));
    assert_eq!(add_deadworks_lines(&updated).unwrap(), None, "a second pass changes nothing");
}

#[test]
fn gameinfo_keeps_crlf_and_respects_existing_lines() {
    let crlf = STOCK_GAMEINFO.replace('\n', "\r\n");
    let updated = add_deadworks_lines(&crlf).unwrap().unwrap();
    assert!(!updated.replace("\r\n", "").contains('\n'), "every line ends in CRLF");

    let launcher = STOCK_GAMEINFO.replace(
        "\t\t\tGame citadel/addons\n",
        "\t\t\tGame                citadel/deadworks_mods\n\t\t\tMod citadel\n\t\t\tWrite citadel\n\t\t\taddonroot           citadel/deadworks_addons\n\t\t\tGame citadel/addons\n",
    );
    assert_eq!(add_deadworks_lines(&launcher).unwrap(), None);

    let partial = STOCK_GAMEINFO.replace(
        "\t\t\tGame citadel/addons\n",
        "\t\t\tMod \"citadel\"\n\t\t\tGame citadel/addons\n",
    );
    let updated = add_deadworks_lines(&partial).unwrap().unwrap();
    assert_eq!(updated.matches("Mod").count(), 1, "an existing Mod entry is not doubled");
    assert!(updated.contains("addonroot"));

    assert!(add_deadworks_lines("\"GameInfo\" { }").is_err());
    // A commented-out line does not count.
    let commented = launcher.replace("addonroot", "// addonroot");
    assert!(add_deadworks_lines(&commented).unwrap().is_some());
}

#[test]
fn ensure_gameinfo_backs_up_and_writes_once() {
    let root = scratch("gameinfo");
    let path = root.join("gameinfo.gi");
    fs::write(&path, STOCK_GAMEINFO).unwrap();
    let edit = ensure_deadworks_gameinfo(&path).unwrap();
    assert!(edit.changed);
    let backup = edit.backup.unwrap();
    assert_eq!(fs::read_to_string(&backup).unwrap(), STOCK_GAMEINFO);
    assert!(read_deadworks_gameinfo(&path).unwrap().ready());

    let again = ensure_deadworks_gameinfo(&path).unwrap();
    assert_eq!(again, GameinfoEdit { changed: false, backup: None });
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no temp file is left behind");

    assert!(ensure_deadworks_gameinfo(&root.join("missing.gi")).is_err());
    assert!(!read_deadworks_gameinfo(&root.join("missing.gi")).unwrap().present);
}

// --- install -----------------------------------------------------------------------------------

#[test]
fn install_entry_downloads_verifies_and_records() {
    let root = scratch("install");
    let addon = vpk_bytes(&["scripts/mode.vdata_c"]);
    let map = vpk_bytes(&["maps/arena.vmap_c"]);
    let addon_entry = entry(ContentKind::Addon, "mode", &addon, true);
    let map_entry = entry(ContentKind::Map, "arena", &map, true);
    let wrong_entry = ContentEntry {
        hash: "0000000000000000".into(),
        ..entry(ContentKind::Addon, "drift", &addon, true)
    };
    let base = http_server(HashMap::from([
        (
            format!("/dl/addons/mode_{}.vpk.zst", addon_entry.hash),
            (200, vec![], zstd_bytes(&addon)),
        ),
        // The map is only on the bzip2 name: the zst request 404s first.
        (
            format!("/dl/maps/arena_{}.vpk.bz2", map_entry.hash),
            (200, vec![], bzip2_bytes(&map)),
        ),
        (
            "/dl/addons/drift_0000000000000000.vpk.zst".into(),
            (302, vec![("Location".into(), "/real/drift.zst".into())], Vec::new()),
        ),
        ("/real/drift.zst".into(), (200, vec![], zstd_bytes(&addon))),
    ]));
    let fastdl = format!("{base}/dl");
    let paths = deadworks_paths(&root);

    let mut seen = 0;
    let outcome = install_entry(&root, &fastdl, &addon_entry, true, false, &mut |done, _| seen = done)
        .unwrap();
    assert_eq!(outcome, EntryOutcome::Installed);
    assert_eq!(fs::read(paths.addons_dir.join("mode.vpk")).unwrap(), addon);
    assert!(seen > 0);
    assert_eq!(
        install_entry(&root, &fastdl, &map_entry, true, false, &mut |_, _| {}).unwrap(),
        EntryOutcome::Installed
    );
    assert_eq!(fs::read(paths.maps_dir.join("arena.vpk")).unwrap(), map);

    let ledger = read_ledger(&paths);
    assert!(ledger.files["addons/mode"].ours && ledger.files["addons/mode"].advertised);
    assert!(ledger.files["maps/arena"].ours);

    // The host's copy differs from the advertised hash: nothing is installed until accepted.
    let drift = install_entry(&root, &fastdl, &wrong_entry, true, false, &mut |_, _| {}).unwrap();
    assert!(matches!(drift, EntryOutcome::Mismatch { .. }));
    assert!(!paths.addons_dir.join("drift.vpk").exists());
    let accepted = install_entry(&root, &fastdl, &wrong_entry, true, true, &mut |_, _| {}).unwrap();
    assert_eq!(accepted, EntryOutcome::Installed);
    assert!(!read_ledger(&paths).files["addons/drift"].advertised);

    // Loopback is refused unless the game server itself is local.
    let refused = install_entry(&root, &fastdl, &addon_entry, false, false, &mut |_, _| {});
    let refused = refused.unwrap_err().to_string();
    assert!(refused.contains("local or private"), "{refused}");

    let leftovers: Vec<String> = fs::read_dir(&paths.addons_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part") || n.ends_with(".download"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn install_entry_never_replaces_a_map_it_did_not_install() {
    let root = scratch("foreign-map");
    let paths = deadworks_paths(&root);
    fs::create_dir_all(&paths.maps_dir).unwrap();
    fs::write(paths.maps_dir.join("dl_midtown.vpk"), b"valve's map").unwrap();
    let wanted = entry(ContentKind::Map, "dl_midtown", b"other", true);
    let error = install_entry(&root, "http://127.0.0.1:9", &wanted, true, true, &mut |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("not installed by Sigil"), "{error}");
    assert_eq!(fs::read(paths.maps_dir.join("dl_midtown.vpk")).unwrap(), b"valve's map");
}

#[test]
fn bootstrap_only_comes_from_deadworks_and_clears_stray_vpks() {
    let root = scratch("bootstrap");
    let manifest = parse_bootstrap_manifest(
        r#"{"version":8,"compressed_size":10,"sha256":"89feb195cb1f613883d7ebc98f956855373844358a4318a069649e44b10f4a92","download_url":"https://evil.example/8.vpk.bz2","min_version":0}"#,
    )
    .unwrap();
    let error = install_bootstrap(&root, &manifest, &mut |_, _| {}).unwrap_err().to_string();
    assert!(error.contains("api.deadworks.net"), "{error}");
    assert!(parse_bootstrap_manifest(r#"{"version":1,"sha256":"zz","download_url":"x"}"#).is_err());

    let paths = deadworks_paths(&root);
    fs::create_dir_all(&paths.mods_dir).unwrap();
    for name in [BOOTSTRAP_FILE, "pak02_dir.vpk", "notes.txt"] {
        fs::write(paths.mods_dir.join(name), b"x").unwrap();
    }
    remove_stray_vpks(&paths.mods_dir).unwrap();
    let mut left: Vec<String> = fs::read_dir(&paths.mods_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["notes.txt", BOOTSTRAP_FILE]);

    // State: the file on disk is not the published one.
    let state = bootstrap_state(&root, Ok(Some(manifest.clone()))).unwrap();
    assert!(matches!(state, BootstrapState::Needed { .. }));
    assert_eq!(bootstrap_state(&root, Ok(None)).unwrap(), BootstrapState::NotPublished);
    assert!(matches!(
        bootstrap_state(&root, Err("offline".into())).unwrap(),
        BootstrapState::Unknown { .. }
    ));
}

#[test]
fn prepare_join_edits_gameinfo_then_installs_content() {
    let root = scratch("prepare");
    let paths = deadworks_paths(&root);
    fs::create_dir_all(&paths.citadel).unwrap();
    fs::write(&paths.gameinfo, STOCK_GAMEINFO).unwrap();
    let addon = vpk_bytes(&["scripts/mode.vdata_c"]);
    let wanted = entry(ContentKind::Addon, "mode", &addon, true);
    let base = http_server(HashMap::from([(
        format!("/addons/mode_{}.vpk.zst", wanted.hash),
        (200, vec![], zstd_bytes(&addon)),
    )]));
    let ad = Advertisement {
        entries: vec![wanted],
        truncated: false,
        fastdl: Some(base),
    };
    let check = JoinCheck {
        address: "127.0.0.1:27015".into(),
        advertised: true,
        plan: plan_content(&root, &ad).unwrap(),
        gameinfo: read_deadworks_gameinfo(&paths.gameinfo).unwrap(),
        bootstrap: BootstrapState::NotPublished,
        allow_private: true,
    };
    assert!(check.blockers().is_empty());
    let writes = check.writes();
    assert_eq!(writes.len(), 2, "{writes:?}");

    let lines = std::sync::Mutex::new(Vec::new());
    let report = prepare_join(&root, &check, &[], &|p| lines.lock().unwrap().push(p.line)).unwrap();
    assert!(report.gameinfo_backup.is_some());
    assert_eq!(report.installed, ["addons/mode"]);
    assert!(report.mismatched.is_empty());
    assert!(read_deadworks_gameinfo(&paths.gameinfo).unwrap().ready());
    assert_eq!(lines.lock().unwrap().last().map(String::as_str), Some("Ready"));

    // Nothing to do on a second check.
    let again = JoinCheck {
        plan: plan_content(&root, &ad).unwrap(),
        gameinfo: read_deadworks_gameinfo(&paths.gameinfo).unwrap(),
        ..check
    };
    assert!(!again.needs_writes());
}

#[test]
fn a_join_without_a_download_host_is_blocked_before_any_write() {
    let root = scratch("blocked");
    let paths = deadworks_paths(&root);
    fs::create_dir_all(&paths.citadel).unwrap();
    fs::write(&paths.gameinfo, STOCK_GAMEINFO).unwrap();
    let ad = Advertisement {
        entries: vec![entry(ContentKind::Addon, "mode", b"x", true)],
        truncated: false,
        fastdl: None,
    };
    let check = JoinCheck {
        address: "10.0.0.1:27015".into(),
        advertised: true,
        plan: plan_content(&root, &ad).unwrap(),
        gameinfo: read_deadworks_gameinfo(&paths.gameinfo).unwrap(),
        bootstrap: BootstrapState::NotPublished,
        allow_private: true,
    };
    assert_eq!(check.blockers().len(), 1);
    assert!(prepare_join(&root, &check, &[], &|_| {}).is_err());
    assert_eq!(fs::read_to_string(&paths.gameinfo).unwrap(), STOCK_GAMEINFO);
}

// --- join --------------------------------------------------------------------------------------

#[test]
fn connect_urls_take_only_literal_addresses() {
    assert_eq!(connect_url("1.2.3.4:27015", None).unwrap(), "steam://connect/1.2.3.4:27015");
    assert_eq!(connect_url("[::1]:27015", None).unwrap(), "steam://connect/[::1]:27015");
    assert_eq!(
        connect_url("1.2.3.4:27015", Some("hunter2")).unwrap(),
        "steam://connect/1.2.3.4:27015/hunter2"
    );
    assert_eq!(connect_url("1.2.3.4:27015", Some("  ")).unwrap(), "steam://connect/1.2.3.4:27015");
    for bad in ["host.example:27015", "1.2.3.4", "1.2.3.4:0", "1.2.3.4:27015;rm", "steam://x"] {
        assert!(connect_url(bad, None).is_err(), "{bad}");
    }
    assert!(connect_url("1.2.3.4:27015", Some("a/b")).is_err());
}

#[test]
fn connect_passwords_are_allowlisted() {
    for bad in ["a&calc", "a|b", "%PATH%", "a^b", "a<b", "a>b", "a b", "a\"b", "pässword", &"x".repeat(65)] {
        assert!(connect_url("1.2.3.4:27015", Some(bad)).is_err(), "{bad}");
    }
    assert_eq!(
        connect_url("1.2.3.4:27015", Some("abc-1.2_x~")).unwrap(),
        "steam://connect/1.2.3.4:27015/abc-1.2_x~"
    );
}

#[test]
fn guarded_resolver_refuses_local_addresses() {
    use ureq::unversioned::resolver::Resolver;
    let config = crate::net::builder().build();
    let resolve = |url: &str, allow_private: bool| {
        let uri: ureq::http::Uri = url.parse().unwrap();
        GuardedResolver { allow_private }.resolve(
            &uri,
            &config,
            ureq::unversioned::transport::NextTimeout::default(),
        )
    };
    for url in ["http://127.0.0.1/x", "http://10.1.2.3:8080/x", "http://[::1]/x", "https://192.168.1.5/x", "http://[::ffff:127.0.0.1]/x"] {
        let error = resolve(url, false).unwrap_err().to_string();
        assert!(error.contains("local or private"), "{url}: {error}");
        assert!(resolve(url, true).is_ok(), "{url} is allowed for a LAN server");
    }
    let public = resolve("https://1.1.1.1/x", false).unwrap();
    assert_eq!(public[0], "1.1.1.1:443".parse::<std::net::SocketAddr>().unwrap());
}

#[test]
fn private_addresses_are_not_public() {
    for ip in ["127.0.0.1", "10.1.2.3", "192.168.0.1", "172.16.0.1", "169.254.1.1", "100.64.0.1", "0.0.0.0", "::1", "fe80::1", "fd00::1", "::ffff:10.0.0.1", "240.0.0.1"] {
        assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["1.1.1.1", "146.103.38.196", "2606:4700::1111"] {
        assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
    }
}

#[test]
fn content_urls_follow_the_spec_layout() {
    let e = ContentEntry {
        kind: ContentKind::Map,
        name: "arena".into(),
        hash: "0123456789abcdef".into(),
        size: None,
    };
    assert_eq!(
        content_url("https://h.example/dw/", &e, "zst"),
        "https://h.example/dw/maps/arena_0123456789abcdef.vpk.zst"
    );
}

/// Live: the real list, pings and one server's rules. `cargo test -p deadlock-core --lib
/// servers::tests::live -- --ignored --nocapture`
#[test]
#[ignore = "network"]
fn live_list_ping_and_rules() {
    let list = fetch_servers(DEADWORKS_API_BASE).unwrap();
    assert!(!list.servers.is_empty());
    let targets: Vec<(String, std::net::SocketAddr)> = list
        .servers
        .iter()
        .filter_map(|s| s.socket_addr().map(|a| (s.id.clone(), a)))
        .collect();
    let pings = ping_many(targets, Duration::from_millis(1500));
    let answered = pings.values().filter(|p| p.is_some()).count();
    println!("{} servers, {answered} answered a ping, registry {}", list.servers.len(), list.registry_ok);
    let root = scratch("live");
    // The real UI pack, installed into the scratch root through the guarded downloader.
    if let Some(manifest) = fetch_bootstrap_manifest(DEADWORKS_API_BASE).unwrap() {
        install_bootstrap(&root, &manifest, &mut |_, _| {}).unwrap();
        let state = bootstrap_state(&root, Ok(Some(manifest.clone()))).unwrap();
        assert_eq!(state, BootstrapState::Current { version: manifest.version });
        println!("bootstrap v{} installed and verified", manifest.version);
    }
    for server in list.servers.iter().filter(|s| s.has_addons).take(3) {
        let check = check_join(&root, &server.address, DEADWORKS_API_BASE).unwrap();
        println!(
            "{} {}: advertised {}, items {:?}, host {:?}, bootstrap {:?}",
            server.name,
            server.address,
            check.advertised,
            check.plan.items.iter().map(|i| (i.entry.key(), i.entry.size, i.action)).collect::<Vec<_>>(),
            check.plan.fastdl_host(),
            check.bootstrap.installed_version()
        );
    }
}

/// Live: install the smallest advertised addon from a real server into a scratch root.
#[test]
#[ignore = "network"]
fn live_install_one_addon() {
    let list = fetch_servers(DEADWORKS_API_BASE).unwrap();
    let root = scratch("live-addon");
    for server in list.servers.iter().filter(|s| s.has_addons) {
        let Ok(check) = check_join(&root, &server.address, DEADWORKS_API_BASE) else { continue };
        let Some(item) = check
            .plan
            .downloads()
            .filter(|i| i.entry.kind == ContentKind::Addon && i.entry.size.is_some_and(|s| s < 64 << 20))
            .min_by_key(|i| i.entry.size)
        else {
            continue;
        };
        let fastdl = check.plan.fastdl.clone().unwrap();
        let outcome =
            install_entry(&root, &fastdl, &item.entry, check.allow_private, false, &mut |_, _| {})
                .unwrap();
        println!("{}: {} -> {outcome:?}", server.name, item.entry.key());
        let _ = fs::remove_dir_all(&root);
        return;
    }
    println!("no server advertised a small addon");
}

// ---------------------------------------------------------------------------------------------
// Regions, filters, sort, favorites, maps
// ---------------------------------------------------------------------------------------------

fn sample_server(id: &str, name: &str, map: &str, country: &str, players: u32) -> Server {
    Server {
        id: id.to_string(),
        name: name.to_string(),
        address: format!("203.0.113.{id}:27015"),
        map: map.to_string(),
        players,
        max_players: 24,
        bots: 0,
        country: country.to_string(),
        version: String::new(),
        game_mode: String::new(),
        registered: !country.is_empty(),
        password: false,
        has_addons: false,
        roster: Vec::new(),
        content_hint: Vec::new(),
    }
}

#[test]
fn countries_group_into_continents() {
    assert_eq!(Continent::of("eu"), Continent::Europe);
    assert_eq!(Continent::of("DE"), Continent::Europe);
    assert_eq!(Continent::of("gb"), Continent::Europe);
    assert_eq!(Continent::of("us"), Continent::NorthAmerica);
    assert_eq!(Continent::of("MX"), Continent::NorthAmerica);
    assert_eq!(Continent::of("jp"), Continent::AsiaPacific);
    assert_eq!(Continent::of("sg"), Continent::AsiaPacific);
    assert_eq!(Continent::of("au"), Continent::Oceania);
    assert_eq!(Continent::of("NZ"), Continent::Oceania);
    assert_eq!(Continent::of("ae"), Continent::MiddleEast);
    assert_eq!(Continent::of("ZA"), Continent::MiddleEast);
    assert_eq!(Continent::of(""), Continent::Other);
    assert_eq!(Continent::of("zz"), Continent::Other);
    assert_eq!(region_label("de"), "DE");
    assert_eq!(region_label(""), "Unknown");
    assert_eq!(Continent::from_id("apac"), Some(Continent::AsiaPacific));
}

#[test]
fn a_mode_comes_from_the_registry_or_the_map() {
    let mut server = sample_server("1", "one", "overthrow10", "US", 4);
    assert_eq!(server.mode_label(), "Overthrow");
    server.map = "st_dome_a2".to_string();
    assert_eq!(server.mode_label(), "Street Brawl");
    server.map = "dl_midtown".to_string();
    assert_eq!(server.mode_label(), "Standard");
    server.map = "start".to_string();
    assert_eq!(server.mode_label(), "Standard");
    server.map = "custom_arena".to_string();
    assert_eq!(server.mode_label(), "Custom map");
    server.map.clear();
    assert_eq!(server.mode_label(), "Unknown");
    server.map = "dl_midtown".to_string();
    server.game_mode = "street_brawl".to_string();
    assert_eq!(server.mode_label(), "Street brawl");
}

#[test]
fn filters_narrow_the_list_and_count_one_chip_each() {
    let mut open = sample_server("1", "Open EU", "dl_midtown", "DE", 6);
    open.password = false;
    let mut locked = sample_server("2", "Locked EU", "overthrow10", "FR", 12);
    locked.password = true;
    let mut full = sample_server("3", "Full US", "st_dome_a2", "US", 24);
    full.max_players = 24;
    let idle = sample_server("4", "Idle APAC", "dl_harbor", "SG", 0);
    let servers = [open, locked, full, idle];

    let filter = ServerFilter::default();
    assert_eq!(filter.active_count(), 0);
    assert!(servers.iter().all(|server| filter.matches(server)));

    let joinable = ServerFilter::joinable();
    assert!(joinable.is_joinable());
    assert_eq!(joinable.active_count(), 3);
    let shown: Vec<&str> = servers
        .iter()
        .filter(|server| joinable.matches(server))
        .map(|server| server.id.as_str())
        .collect();
    assert_eq!(shown, vec!["1"], "no password, somebody on, room left");

    let europe = ServerFilter {
        continent: Some(Continent::Europe),
        ..ServerFilter::default()
    };
    assert_eq!(
        servers.iter().filter(|server| europe.matches(server)).count(),
        2
    );
    let password = ServerFilter {
        password: PasswordFilter::Password,
        ..ServerFilter::default()
    };
    assert_eq!(
        servers.iter().filter(|server| password.matches(server)).count(),
        1
    );
    let mode = ServerFilter {
        mode: Some("Street Brawl".to_string()),
        ..ServerFilter::default()
    };
    assert_eq!(servers.iter().filter(|server| mode.matches(server)).count(), 1);
    let map = ServerFilter {
        map: Some("dl_harbor".to_string()),
        ..ServerFilter::default()
    };
    assert_eq!(servers.iter().filter(|server| map.matches(server)).count(), 1);
    let search = ServerFilter {
        query: "apac".to_string(),
        ..ServerFilter::default()
    };
    assert_eq!(servers.iter().filter(|server| search.matches(server)).count(), 1);
    let space = ServerFilter {
        has_space: true,
        ..ServerFilter::default()
    };
    assert!(servers.iter().any(|server| !space.matches(server)));
    assert_eq!(
        ServerFilter {
            country: Some("fr".to_string()),
            ..ServerFilter::default()
        }
        .active_count(),
        1
    );
}

#[test]
fn sorting_cycles_and_missing_values_stay_last() {
    let fast = sample_server("1", "Alpha", "dl_midtown", "DE", 2);
    let slow = sample_server("2", "Bravo", "overthrow10", "US", 12);
    let unknown = sample_server("3", "Charlie", "dl_harbor", "", 8);
    let servers = vec![&fast, &slow, &unknown];
    let mut pings = HashMap::new();
    pings.insert("1".to_string(), Some(30));
    pings.insert("2".to_string(), Some(180));

    let first = next_sort(None, ServerSortKey::Ping).unwrap();
    assert_eq!(first.direction, SortDirection::Asc);
    let names: Vec<&str> = sorted_servers(servers.clone(), Some(first), &pings)
        .iter()
        .map(|server| server.id.as_str())
        .collect();
    assert_eq!(names, vec!["1", "2", "3"], "no reply sorts last");

    let flipped = next_sort(Some(first), ServerSortKey::Ping).unwrap();
    let names: Vec<&str> = sorted_servers(servers.clone(), Some(flipped), &pings)
        .iter()
        .map(|server| server.id.as_str())
        .collect();
    assert_eq!(names, vec!["2", "1", "3"], "the missing ping stays last");

    assert_eq!(next_sort(Some(flipped), ServerSortKey::Ping), None);
    let players = next_sort(None, ServerSortKey::Players).unwrap();
    assert_eq!(players.direction, SortDirection::Desc);
    assert_eq!(next_sort(Some(players), ServerSortKey::Players).unwrap().direction, SortDirection::Asc);
    let region = next_sort(None, ServerSortKey::Region).unwrap();
    let names: Vec<&str> = sorted_servers(servers.clone(), Some(region), &pings)
        .iter()
        .map(|server| server.id.as_str())
        .collect();
    assert_eq!(names, vec!["1", "2", "3"], "Europe, North America, unknown");
    assert_eq!(sorted_servers(servers.clone(), None, &pings).len(), 3);
}

#[test]
fn favorites_and_recent_servers_survive_a_restart() {
    let root = scratch("saved");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let mut server = sample_server("1", "Harbor", "dl_harbor", "US", 4);
    server.address = "203.0.113.1:27015".to_string();

    assert!(!is_favorite(&read_saved_servers(&workspace), &server.address));
    assert!(toggle_favorite(&workspace, &server).unwrap());
    assert!(toggle_favorite(&workspace, &server).unwrap() == false);
    assert!(toggle_favorite(&workspace, &server).unwrap());
    let saved = read_saved_servers(&workspace);
    assert_eq!(saved.favorites.len(), 1);
    assert_eq!(saved.favorites[0].name, "Harbor");

    for index in 0..(RECENT_LIMIT + 3) {
        let mut other = server.clone();
        other.address = format!("203.0.113.{}:27015", index + 10);
        mark_recent(&workspace, &other).unwrap();
    }
    mark_recent(&workspace, &server).unwrap();
    let saved = read_saved_servers(&workspace);
    assert_eq!(saved.recent.len(), RECENT_LIMIT);
    assert_eq!(saved.recent[0].address, server.address);
    assert!(saved.recent.iter().filter(|entry| entry.address == server.address).count() == 1);
    forget_recent(&workspace, &server.address).unwrap();
    assert!(read_saved_servers(&workspace)
        .recent
        .iter()
        .all(|entry| entry.address != server.address));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn direct_connect_takes_only_an_address() {
    let parsed = parse_direct_address("  203.0.113.7:27015 ").unwrap();
    assert_eq!(parsed.address, "203.0.113.7:27015");
    assert_eq!(parsed.id, parsed.address);
    assert!(!parsed.registered);
    assert_eq!(
        parse_direct_address("steam://connect/203.0.113.7:27015").unwrap().address,
        "203.0.113.7:27015"
    );
    assert!(parse_direct_address("example.com:27015").is_err());
    assert!(parse_direct_address("203.0.113.7").is_err());
    assert!(parse_direct_address("[::1]:27015").is_ok());
}

const MAPS_JSON: &str = r#"{"_aMetadata":{"_nRecordCount":47,"_bIsComplete":false},
"_aRecords":[
 {"_idRow":741438,"_sName":"Custom Maps Menu","_sProfileUrl":"https://gamebanana.com/mods/741438",
  "_tsDateAdded":1794805757,"_tsDateUpdated":1794806005,"_nLikeCount":2,
  "_aPreviewMedia":{"_aImages":[{"_sBaseUrl":"https://images.gamebanana.com/img/ss/mods","_sFile":"a.jpg","_sFile530":"530-90_a.jpg"}]},
  "_aSubmitter":{"_sName":"Eldric_"}},
 {"_idRow":0,"_sName":"no id, dropped"},
 {"_sName":"no id either"}]}"#;

#[test]
fn map_rows_parse_with_a_thumbnail_and_an_author() {
    let maps = parse_maps(MAPS_JSON).unwrap();
    assert_eq!(maps.len(), 1);
    let map = &maps[0];
    assert_eq!(map.id, 741438);
    assert_eq!(map.name, "Custom Maps Menu");
    assert_eq!(map.author, "Eldric_");
    assert_eq!(map.likes, 2);
    assert_eq!(map.updated_unix, 1794806005);
    assert_eq!(
        map.thumbnail.as_deref(),
        Some("https://images.gamebanana.com/img/ss/mods/530-90_a.jpg")
    );
    assert_eq!(map.page_url, "https://gamebanana.com/mods/741438");
}

#[test]
fn a_console_name_comes_from_the_vpk_path_or_the_map_command() {
    assert_eq!(
        console_map_name("Install into citadel/maps/my_arena.vpk and then run it."),
        Some("my_arena".to_string())
    );
    assert_eq!(
        console_map_name("Open the console and type:\n\nmap harbor_night\n"),
        Some("harbor_night".to_string())
    );
    assert_eq!(console_map_name("a skin pack with no map"), None);
    assert_eq!(console_map_name("maps/not a name.vpk"), None);
    assert!(valid_map_name("dl_midtown"));
    assert!(!valid_map_name("dl-midtown"));
    assert!(!valid_map_name(""));
}

#[test]
fn the_map_launch_block_is_set_replaced_and_removed() {
    let text = "fps_max 240\n";
    let with = set_map_launch(text, Some("my_arena")).unwrap();
    assert!(with.contains("map my_arena"));
    assert!(with.contains(MAP_BLOCK_BEGIN));
    assert_eq!(map_launch(&with).as_deref(), Some("my_arena"));
    assert_eq!(set_map_launch(&with, Some("my_arena")), None);

    let replaced = set_map_launch(&with, Some("harbor_night")).unwrap();
    assert_eq!(map_launch(&replaced).as_deref(), Some("harbor_night"));
    assert_eq!(replaced.matches(MAP_BLOCK_BEGIN).count(), 1);

    let removed = set_map_launch(&replaced, None).unwrap();
    assert_eq!(removed, "fps_max 240\n");
    assert_eq!(set_map_launch(text, None), None);
    assert_eq!(set_map_launch(text, Some("../escape")), None);
}

#[test]
fn writing_a_launch_map_backs_the_autoexec_up() {
    let root = scratch("map-launch");
    let game = root.join("game");
    fs::create_dir_all(crate::autoexec::autoexec_path(&game).parent().unwrap()).unwrap();
    fs::write(crate::autoexec::autoexec_path(&game), "fps_max 120\n").unwrap();

    assert!(write_map_launch(&game, Some("my_arena"), false).is_err());
    let backup = write_map_launch(&game, Some("my_arena"), true).unwrap();
    let backup = backup.expect("a backup was made");
    assert!(backup.is_file());
    let text = fs::read_to_string(crate::autoexec::autoexec_path(&game)).unwrap();
    assert!(text.starts_with("fps_max 120\n"));
    assert_eq!(map_launch(&text).as_deref(), Some("my_arena"));

    assert_eq!(write_map_launch(&game, None, true).unwrap().is_some(), true);
    let text = fs::read_to_string(crate::autoexec::autoexec_path(&game)).unwrap();
    assert_eq!(map_launch(&text), None);
    assert_eq!(write_map_launch(&game, None, true).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn installed_maps_are_the_vpk_stems() {
    let root = scratch("installed-maps");
    let maps = root.join("game/game/citadel/maps");
    fs::create_dir_all(&maps).unwrap();
    fs::write(maps.join("my_arena.vpk"), b"x").unwrap();
    fs::write(maps.join("harbor_night.vpk.disabled"), b"x").unwrap();
    fs::write(maps.join("notes.txt"), b"x").unwrap();
    let names = installed_maps(&root.join("game"));
    assert_eq!(names, vec!["harbor_night".to_string(), "my_arena".to_string()]);
    let _ = fs::remove_dir_all(root);
}
