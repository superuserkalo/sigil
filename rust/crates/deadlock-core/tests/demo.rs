//! End to end tests for the local demo importer.
//!
//! Every demo used here is generated in the test. There is no real Deadlock
//! installation on the build host, so the framing layout follows the synthetic
//! files used by `source2-demo`'s own tests: magic, two little-endian offsets,
//! then the command stream at byte 16.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use deadlock_core::demo::{
    CoverageState, FieldSource, MATCH_API_FILE_NAME, MATCH_DEMO_FILE_NAME, MATCH_EVENTS_FILE_NAME,
    MATCH_FIXTURE_FILE_NAME, MATCH_META_FILE_NAME, MATCH_PROVENANCE_FILE_NAME,
    MATCH_REVIEW_FILE_NAME, MATCH_SUMMARY_FILE_NAME, import_demo, import_fixture, list_matches,
    read_meta, read_provenance, read_summary,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "dah-demo-{label}-{}-{stamp}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp workspace");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_varint(output: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn proto_varint(output: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(output, u64::from(field << 3));
    write_varint(output, value);
}

fn proto_bytes(output: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    write_varint(output, u64::from((field << 3) | 2));
    write_varint(output, bytes.len() as u64);
    output.extend_from_slice(bytes);
}

fn proto_string(output: &mut Vec<u8>, field: u32, value: &str) {
    proto_bytes(output, field, value.as_bytes());
}

fn proto_f32(output: &mut Vec<u8>, field: u32, value: f32) {
    write_varint(output, u64::from((field << 3) | 5));
    output.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn write_command(output: &mut Vec<u8>, command: u32, tick: u64, payload: &[u8]) {
    write_varint(output, u64::from(command));
    write_varint(output, tick);
    write_varint(output, payload.len() as u64);
    output.extend_from_slice(payload);
}

/// Build a small but real `PBDEMS2` container with a header, file info, one
/// string table and a stop command.
fn synthetic_demo() -> Vec<u8> {
    let mut header = Vec::new();
    proto_string(&mut header, 1, "PBDEMS2");
    proto_varint(&mut header, 2, 1234);
    proto_string(&mut header, 3, "test-server");
    proto_string(&mut header, 4, "test-client");
    proto_string(&mut header, 5, "dl_midtown");
    proto_string(&mut header, 6, "citadel");
    proto_varint(&mut header, 13, 9999);

    let mut file_info = Vec::new();
    proto_f32(&mut file_info, 1, 1234.0);
    proto_varint(&mut file_info, 2, 37_035);
    proto_varint(&mut file_info, 3, 37_035);

    let mut item = Vec::new();
    proto_string(&mut item, 1, "player-1");
    proto_bytes(&mut item, 2, &[1, 2, 3, 4]);

    let mut table = Vec::new();
    proto_string(&mut table, 1, "userinfo");
    proto_bytes(&mut table, 2, &item);

    let mut string_tables = Vec::new();
    proto_bytes(&mut string_tables, 1, &table);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PBDEMS2\0");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    write_command(&mut bytes, 1, 0, &header);
    write_command(&mut bytes, 2, 0, &file_info);
    write_command(&mut bytes, 3, 0, &[]);
    write_command(&mut bytes, 6, 0, &string_tables);
    write_command(&mut bytes, 0, 0, &[]);
    bytes
}

fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, bytes).expect("write file");
}

#[test]
fn imports_synthetic_demo_then_merges_api_on_reimport() {
    let workspace = TempWorkspace::new("merge");
    let demo_path = workspace.path().join("137990616.dem");
    let bytes = synthetic_demo();
    write_file(&demo_path, &bytes);

    let first = import_demo(workspace.path(), &demo_path, Some("42")).expect("import demo");
    let directory = first.directory.clone();

    for name in [
        MATCH_DEMO_FILE_NAME,
        MATCH_META_FILE_NAME,
        MATCH_EVENTS_FILE_NAME,
        MATCH_SUMMARY_FILE_NAME,
        MATCH_PROVENANCE_FILE_NAME,
    ] {
        assert!(
            directory.join(name).is_file(),
            "expected {name} in the match store"
        );
    }

    assert_eq!(first.summary.match_id, "42");
    assert_eq!(first.summary.source, "demo");
    assert_eq!(
        first
            .summary
            .match_info
            .map
            .as_ref()
            .map(|value| value.value.as_str()),
        Some("dl_midtown")
    );
    assert_eq!(
        first
            .summary
            .match_info
            .duration_seconds
            .as_ref()
            .map(|value| value.value),
        Some(1234)
    );
    assert_eq!(
        first.summary.coverage.fields.get("positions"),
        Some(&CoverageState::Absent)
    );
    assert!(!first.summary.heatmap.available);
    assert!(first.summary.players.is_empty());
    assert!(first.summary.timeline.is_empty());

    assert_eq!(
        first.provenance.crc32,
        format!("{:08x}", crc32fast::hash(&bytes))
    );
    assert_eq!(first.provenance.source.kind, "demo");
    assert_eq!(first.provenance.extractor, "container");
    assert!(
        first
            .provenance
            .missing_fields
            .iter()
            .any(|field| field == "positions")
    );
    assert!(
        first
            .provenance
            .demo
            .as_ref()
            .map(|demo| demo.string_tables)
            .unwrap_or_default()
            >= 1
    );

    // A prior review must survive a re-import.
    write_file(&directory.join(MATCH_REVIEW_FILE_NAME), b"# prior review\n");

    // The cached API record uses the same match id, so it should merge.
    let api = r#"{
      "fetchedAt": "2026-11-16T10:00:00Z",
      "match_info": {
        "match_id": 42,
        "map_name": "dl_midtown",
        "duration_s": 1234,
        "start_time": 1760000000,
        "game_mode": 1,
        "average_badge": 55,
        "players": [
          {
            "account_id": 111,
            "player_name": "Alice",
            "hero_name": "Haze",
            "player_team": 0,
            "player_kills": 7,
            "player_deaths": 2,
            "player_assists": 5,
            "net_worth": 30000,
            "ranked_badge": 55
          }
        ]
      }
    }"#;
    write_file(&directory.join(MATCH_API_FILE_NAME), api.as_bytes());

    let second = import_demo(workspace.path(), &demo_path, Some("42")).expect("re-import demo");

    assert!(second.existed);
    assert!(second.review_preserved);
    assert!(directory.join(MATCH_REVIEW_FILE_NAME).is_file());
    assert_eq!(second.provenance.revision, 2);
    assert_eq!(
        second.provenance.imported_at, first.provenance.imported_at,
        "re-import keeps the first import timestamp"
    );
    assert_eq!(second.summary.source, "demo+api");
    assert!(!second.summary.join.inferred);

    let player = second
        .summary
        .players
        .first()
        .expect("API player row merged");
    assert_eq!(player.key, "acct:111");
    assert_eq!(player.kills.as_ref().map(|value| value.value), Some(7));
    assert_eq!(
        player.kills.as_ref().map(|value| value.source),
        Some(FieldSource::Api)
    );
    assert_eq!(
        player.hero.as_ref().map(|value| value.value.as_str()),
        Some("Haze")
    );
    assert_eq!(player.kda.as_ref().map(|value| value.value), Some(6.0));
    assert_eq!(
        second
            .summary
            .match_info
            .rank
            .as_ref()
            .map(|value| value.value.as_str()),
        Some("average badge 55")
    );
    assert_eq!(
        second
            .summary
            .match_info
            .start_time_unix
            .as_ref()
            .map(|value| value.value),
        Some(1_760_000_000)
    );
    assert!(second.summary.disagreements.is_empty());

    let meta = read_meta(workspace.path(), "42").expect("read meta");
    assert!(meta.has_api);
    assert_eq!(meta.player_count, 1);

    let provenance = read_provenance(workspace.path(), "42").expect("read provenance");
    assert!(
        provenance
            .api
            .as_ref()
            .map(|api| api.merged)
            .unwrap_or(false)
    );

    let summary = read_summary(workspace.path(), "42").expect("read summary");
    assert_eq!(summary.source, "demo+api");

    let list = list_matches(workspace.path()).expect("list matches");
    assert_eq!(list.matches.len(), 1);
    assert_eq!(list.matches[0].match_id, "42");
}

#[test]
fn refuses_a_file_without_the_source2_magic() {
    let workspace = TempWorkspace::new("refuse");
    let path = workspace.path().join("not-a-demo.dem");
    write_file(&path, b"HL2DEMO\0 this is source 1, not a Deadlock demo");

    let error = import_demo(workspace.path(), &path, Some("bad")).expect_err("must refuse");
    let message = error.to_string();

    assert!(message.contains("PBDEMS2"), "message was: {message}");
    assert!(
        message.contains("Expected the magic"),
        "message was: {message}"
    );
    assert!(
        message.contains("Source 1"),
        "the Source 1 hint is useful: {message}"
    );
    assert!(
        !workspace.path().join("matches").join("bad").exists(),
        "a refused import must not create a match folder"
    );
}

#[test]
fn records_api_match_id_conflicts_instead_of_merging() {
    let workspace = TempWorkspace::new("conflict");
    let demo_path = workspace.path().join("conflict.dem");
    write_file(&demo_path, &synthetic_demo());

    let first = import_demo(workspace.path(), &demo_path, Some("42")).expect("import demo");
    write_file(
        &first.directory.join(MATCH_API_FILE_NAME),
        br#"{"match_info":{"match_id":99,"players":[{"account_id":1,"player_name":"Wrong","player_kills":1}]}}"#,
    );

    let second = import_demo(workspace.path(), &demo_path, Some("42")).expect("re-import demo");

    assert_eq!(second.summary.source, "demo");
    assert!(second.summary.players.is_empty());
    assert!(
        second
            .provenance
            .api
            .as_ref()
            .map(|api| !api.merged)
            .unwrap_or(false)
    );
    let conflict = second
        .summary
        .disagreements
        .iter()
        .find(|entry| entry.field == "match.id")
        .expect("match id conflict recorded");
    assert_eq!(conflict.resolution, "demo");
    assert_eq!(
        conflict.api.as_ref().and_then(|value| value.as_u64()),
        Some(99)
    );
}

#[test]
fn fixture_import_produces_events_positions_and_heatmap() {
    let workspace = TempWorkspace::new("fixture");
    let fixture_path = workspace.path().join("fixture.json");
    let fixture = r#"{
      "schema": "dah.match.fixture/1",
      "matchId": "fixture-1",
      "map": "dl_midtown",
      "durationSeconds": 1800,
      "result": "win",
      "gameMode": "standard",
      "players": [
        {"accountId": 1, "name": "Alice", "hero": "Haze", "team": 0},
        {"accountId": 2, "name": "Bob", "hero": "Mina", "team": 1}
      ],
      "events": [
        {
          "tick": 100,
          "timeSeconds": 10.5,
          "kind": "kill",
          "actor": "Alice",
          "target": "Bob",
          "assists": ["Alice"],
          "x": 1024,
          "y": 2048
        }
      ],
      "positions": [
        {"timeSeconds": 10.5, "x": 1000, "y": 2000, "team": 1, "player": "Bob", "kind": "death"},
        {"timeSeconds": 20.0, "x": 1010, "y": 2010, "team": 1, "player": "Bob", "kind": "death"}
      ]
    }"#;
    write_file(&fixture_path, fixture.as_bytes());

    let result = import_fixture(workspace.path(), &fixture_path, None).expect("import fixture");

    assert_eq!(result.match_id, "fixture-1");
    assert_eq!(result.summary.source, "fixture");
    assert_eq!(result.summary.players.len(), 2);
    assert_eq!(result.summary.timeline.len(), 1);
    assert_eq!(
        result.summary.coverage.fields.get("events.timeline"),
        Some(&CoverageState::Present)
    );
    assert_eq!(
        result.summary.coverage.fields.get("positions"),
        Some(&CoverageState::Present)
    );

    let alice = result
        .summary
        .players
        .iter()
        .find(|player| player.key == "acct:1")
        .expect("Alice row");
    assert_eq!(alice.kills.as_ref().map(|value| value.value), Some(1));
    assert_eq!(alice.assists.as_ref().map(|value| value.value), Some(1));
    assert_eq!(alice.kda.as_ref().map(|value| value.value), Some(2.0));
    assert_eq!(
        alice.kills.as_ref().map(|value| value.source),
        Some(FieldSource::Fixture)
    );
    assert_eq!(
        alice.kda.as_ref().map(|value| value.source),
        Some(FieldSource::Derived)
    );

    let heatmap = &result.summary.heatmap;
    assert!(heatmap.available);
    assert_eq!(heatmap.basis.as_deref(), Some("deaths"));
    assert_eq!(heatmap.samples, 2);
    assert_eq!(heatmap.cells.len(), 1);
    assert_eq!(heatmap.cells[0].x, 512);
    assert_eq!(heatmap.cells[0].y, 1536);
    assert_eq!(heatmap.cells[0].samples, 2);

    assert!(result.directory.join(MATCH_FIXTURE_FILE_NAME).is_file());
    assert!(!result.directory.join(MATCH_DEMO_FILE_NAME).exists());
    assert_eq!(result.provenance.source.kind, "fixture");
    assert_eq!(result.provenance.extractor, "fixture");

    let events =
        fs::read_to_string(result.directory.join(MATCH_EVENTS_FILE_NAME)).expect("read events");
    assert_eq!(events.lines().count(), 1);
    let event: serde_json::Value = serde_json::from_str(events.lines().next().unwrap()).unwrap();
    assert_eq!(event["kind"], "kill");
}
