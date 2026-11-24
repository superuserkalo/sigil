//! Tests for opt-in product data (ADR 0017). File IO uses a per-test folder under the system
//! temp directory; network tests use the loopback server from the telemetry tests.

use super::*;

use std::collections::BTreeSet;

use serde_json::Value;

use crate::telemetry::tests::Server;
use crate::telemetry::write_consent;

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dah-product-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn consent(asked: bool, on: bool) -> Consent {
    Consent { asked: true, usage: true, crashes: true, hash_lookup: true, product_asked: asked, product: on }
}

fn id(value: u64) -> ModId {
    ModId::new(value).unwrap()
}

fn code(value: &str) -> Code {
    Code::new(value).unwrap()
}

/// One record of every type. `every_type_has_a_sample` keeps this in step with `KINDS`.
fn samples() -> Vec<Record> {
    vec![
        Record::ModAction { mod_id: id(512_345), action: ModAction::NotInterested },
        Record::ModKept { mod_id: id(512_345), kept: KeptBucket::D7To30 },
        Record::EnabledSet { mods: vec![SlotMod { mod_id: id(1), slot: 1 }, SlotMod { mod_id: id(2), slot: 7 }] },
        Record::VariantChosen { mod_id: id(512_345), file_id: id(1_414_905) },
        Record::InstallResult { mod_id: id(9), ok: false, reason: Some(code("download_failed")) },
        Record::ModPair { a: id(3), b: id(4), kind: PairKind::Overlap },
        Record::CompatFinding { mod_id: id(9), finding: code("path_moved"), game_build: Some(5_911) },
        Record::RepairResult { mod_id: id(9), repair: code("migrate_paths"), ok: true, game_build: Some(5_911) },
        Record::OffAfterCrash { mod_id: id(9) },
        Record::CrashSuspect { mod_id: id(9) },
        Record::RecShown { row: code("for_you"), position: 3, mod_id: id(77), outcome: RecOutcome::Installed },
        Record::BrowseQuery {
            sort: Some(code("most_liked")),
            filters: vec![code("skins"), code("hero_haze")],
            search: scrub_search("  Haze   SKIN "),
            results: 41,
        },
        Record::TopHeroes { heroes: vec![HeroPlay { hero_id: 13, playtime: PlaytimeBucket::H10To50 }] },
        Record::RankTier { tier: 7 },
        Record::PerfConfig { config: code("low_end_fps"), version: 3 },
        Record::Benchmark {
            avg_fps: 144,
            low_fps: 97,
            scene: code("hideout"),
            width: 2560,
            height: 1440,
            preset: Some(code("medium")),
        },
        Record::Hardware {
            gpu: model_name("NVIDIA GeForce RTX 3060 Laptop GPU"),
            cpu: model_name("AMD Ryzen 7 5800X 8-Core Processor"),
            ram_gb: 32,
            width: 1920,
            height: 1080,
            refresh_hz: 144,
        },
        Record::FeatureUse { feature: code("crosshair_editor"), count: 2 },
        Record::Settings { theme: code("dark"), density: code("compact"), language: code("de"), mature: false },
        Record::Launch { modded: true },
        Record::Session { length: SessionBucket::M30To120 },
    ]
}

// --- consent gate --------------------------------------------------------------------------

#[test]
fn the_gate_needs_both_the_question_and_the_yes() {
    assert!(!allowed(&Consent::default()));
    assert!(!allowed(&consent(false, true)));
    assert!(!allowed(&consent(true, false)));
    assert!(allowed(&consent(true, true)));
    // The other switches do not open it.
    let others = Consent { asked: true, usage: true, crashes: true, hash_lookup: true, ..Consent::default() };
    assert!(!allowed(&others));
}

#[test]
fn no_consent_means_no_record_exists_on_disk() {
    let dir = root("no-consent");
    let recorder = Recorder::open(&dir);
    for record in samples() {
        assert!(!recorder.record(record));
    }
    assert_eq!(recorder.pending_len(), 0);
    assert_eq!(recorder.persist().unwrap(), 0);
    assert!(!recorder.once_per_version("hardware", "0.1.0"));
    // Not one file under the product folder, and not the folder.
    assert!(!dir.join("telemetry").join("product").exists());

    // A switch that says on without the question being shown is still off.
    write_consent(&dir, &consent(false, true)).unwrap();
    let recorder = Recorder::open(&dir);
    assert!(!recorder.record(Record::Launch { modded: true }));
    recorder.persist().unwrap();
    assert!(!recorder.queue_path().exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn consent_on_queues_and_turning_it_off_clears_everything() {
    let dir = root("toggle");
    write_consent(&dir, &consent(true, true)).unwrap();
    let recorder = Recorder::open(&dir);
    for record in samples() {
        assert!(recorder.record(record), "a valid sample was refused");
    }
    assert_eq!(recorder.persist().unwrap(), samples().len());
    assert_eq!(recorder.read_queue().len(), samples().len());
    fs::write(recorder.log_path(), b"[]").unwrap();

    // Off in the app: memory, queue and log go at once.
    write_consent(&dir, &consent(true, false)).unwrap();
    recorder.apply_consent(&consent(true, false));
    assert!(!recorder.enabled());
    assert!(!recorder.queue_path().exists());
    assert!(!recorder.log_path().exists());
    assert!(!recorder.record(Record::Launch { modded: false }));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persist_reads_the_consent_file_again() {
    let dir = root("file-off");
    write_consent(&dir, &consent(true, true)).unwrap();
    let recorder = Recorder::open(&dir);
    assert!(recorder.record(Record::Launch { modded: true }));
    // The file says off (another window, a hand edit): nothing is written and the switch drops.
    write_consent(&dir, &consent(true, false)).unwrap();
    assert_eq!(recorder.persist().unwrap(), 0);
    assert!(!recorder.queue_path().exists());
    assert!(!recorder.enabled());
    let _ = fs::remove_dir_all(&dir);
}

// --- limits --------------------------------------------------------------------------------

#[test]
fn memory_queue_and_age_are_capped() {
    let dir = root("caps");
    write_consent(&dir, &consent(true, true)).unwrap();
    let recorder = Recorder::open(&dir);
    for _ in 0..(PENDING_LIMIT + 40) {
        recorder.record(Record::Launch { modded: true });
    }
    assert_eq!(recorder.pending_len(), PENDING_LIMIT);

    // A queue file past every cap comes back within them.
    let now = now_ms();
    let mut list: Vec<Stamped> = (0..(QUEUE_LIMIT + 50))
        .map(|i| Stamped { at: now - i as i64, record: Record::Launch { modded: true } })
        .collect();
    list.push(Stamped { at: now - QUEUE_MAX_AGE_MS - 1, record: Record::Launch { modded: false } });
    fs::create_dir_all(recorder.queue_path().parent().unwrap()).unwrap();
    fs::write(recorder.queue_path(), serde_json::to_vec(&list).unwrap()).unwrap();
    let queue = recorder.read_queue();
    assert_eq!(queue.len(), QUEUE_LIMIT);
    assert!(queue.iter().all(|s| now - s.at <= QUEUE_MAX_AGE_MS));
    recorder.persist().unwrap();
    assert!(fs::metadata(recorder.queue_path()).unwrap().len() as usize <= QUEUE_MAX_BYTES);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn feature_counts_merge_in_memory() {
    let dir = root("features");
    write_consent(&dir, &consent(true, true)).unwrap();
    let recorder = Recorder::open(&dir);
    for _ in 0..5 {
        recorder.record(Record::FeatureUse { feature: code("viewer"), count: 1 });
    }
    recorder.record(Record::FeatureUse { feature: code("forge"), count: 1 });
    assert_eq!(recorder.pending_len(), 2);
    recorder.persist().unwrap();
    assert!(recorder.read_queue().iter().any(|s| s.record == Record::FeatureUse { feature: code("viewer"), count: 5 }));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn records_are_bounded() {
    let many: Vec<SlotMod> = (1..=200).map(|i| SlotMod { mod_id: id(i), slot: i as u16 }).collect();
    let Some(Record::EnabledSet { mods }) = (Record::EnabledSet { mods: many }).bounded() else { panic!() };
    assert_eq!(mods.len(), MAX_SET);
    let heroes: Vec<HeroPlay> = (1..=30).map(|i| HeroPlay { hero_id: i, playtime: PlaytimeBucket::Under1h }).collect();
    let Some(Record::TopHeroes { heroes }) = (Record::TopHeroes { heroes }).bounded() else { panic!() };
    assert_eq!(heroes.len(), MAX_HEROES);
    assert!(Record::RankTier { tier: 12 }.bounded().is_none());
    assert!(Record::ModPair { a: id(5), b: id(5), kind: PairKind::Conflict }.bounded().is_none());
    assert_eq!(
        Record::ModPair { a: id(9), b: id(5), kind: PairKind::Conflict }.bounded(),
        Some(Record::ModPair { a: id(5), b: id(9), kind: PairKind::Conflict })
    );
    let bad_bench = Record::Benchmark { avg_fps: 60, low_fps: 90, scene: code("x"), width: 1920, height: 1080, preset: None };
    assert!(bad_bench.bounded().is_none());
}

// --- scrubbers -----------------------------------------------------------------------------

#[test]
fn search_text_is_trimmed_lowered_and_cut() {
    assert_eq!(scrub_search("  Haze   SKIN\t").unwrap().as_str(), "haze skin");
    let long = "a ".repeat(100);
    assert!(scrub_search(&long).unwrap().as_str().chars().count() <= MAX_SEARCH_CHARS);
    assert_eq!(scrub_search("ÄÖÜ Skin").unwrap().as_str(), "äöü skin");
    assert!(scrub_search("   ").is_none());
    assert_eq!(scrub_search("rtx 4090").unwrap().as_str(), "rtx 4090");
}

#[test]
fn search_text_that_looks_private_is_dropped() {
    for text in [
        "me@example.com",
        "name at host @ x",
        "C:\\Users\\kalo\\mods",
        "/Users/kalo/Desktop",
        "d: drive mods",
        "~ home",
        "https://gamebanana.com/mods/1",
        "www.example",
        "gamebanana.com",
        "76561198012345678",
        "12345",
        "765 611 980 123",
        "[U:1:52345]",
        "steam_0:1:1234",
        "pak01_dir.vpk",
    ] {
        assert!(scrub_search(text).is_none(), "{text}");
    }
}

#[test]
fn hardware_names_become_model_names() {
    assert_eq!(model_name("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz").unwrap().as_str(), "Intel Core i7-9700K");
    assert_eq!(model_name("AMD Ryzen 7 5800X 8-Core Processor").unwrap().as_str(), "AMD Ryzen 7 5800X");
    assert_eq!(model_name("NVIDIA GeForce RTX 3060 Laptop GPU").unwrap().as_str(), "NVIDIA GeForce RTX 3060 Laptop GPU");
    assert_eq!(model_name("Apple M2 Pro").unwrap().as_str(), "Apple M2 Pro");
    // Paths and serials do not survive.
    assert_eq!(model_name("NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2").unwrap().as_str(), "NVIDIA GeForce GTX 1050 Ti");
    assert!(model_name("C:\\Users\\kalo\\gpu").is_none());
    assert!(model_name("PCI\\VEN_10DE&DEV_2504").is_none());
    assert!(model_name("GPU SN 123456789").is_none());
    assert!(model_name("76561198012345678").is_none());
    assert!(model_name("   ").is_none());
    assert!(model_name(&"Radeon ".repeat(20)).unwrap().as_str().len() <= MAX_MODEL_CHARS);
}

#[test]
fn ram_rank_and_time_are_coarse() {
    assert_eq!(round_ram_gb(16_384), 16);
    assert_eq!(round_ram_gb(15_900), 16);
    assert_eq!(round_ram_gb(24_000), 24);
    assert_eq!(round_ram_gb(1_000), 4);
    assert_eq!(round_ram_gb(13_000), 12);
    assert_eq!(rank_tier(116), Some(11));
    assert_eq!(rank_tier(73), Some(7));
    assert_eq!(rank_tier(5), Some(0));
    assert_eq!(rank_tier(4_812), None);
    assert_eq!(PlaytimeBucket::from_secs(59 * 60), PlaytimeBucket::Under1h);
    assert_eq!(PlaytimeBucket::from_secs(12 * 3600), PlaytimeBucket::H10To50);
    assert_eq!(PlaytimeBucket::from_secs(900 * 3600), PlaytimeBucket::Over200h);
    assert_eq!(SessionBucket::from_secs(4 * 60), SessionBucket::Under5m);
    assert_eq!(SessionBucket::from_secs(45 * 60), SessionBucket::M30To120);
    assert_eq!(KeptBucket::from_secs(3 * 86_400), KeptBucket::D1To7);
}

#[test]
fn mods_without_a_gamebanana_id_cannot_be_recorded() {
    assert!(ModId::new(0).is_none());
    assert!(ModId::new(76_561_198_012_345_678).is_none());
    assert!(serde_json::from_str::<Record>(r#"{"type":"crash_suspect","mod_id":0}"#).is_err());
}

#[test]
fn codes_refuse_anything_but_short_identifiers() {
    assert_eq!(code("Most Liked").as_str(), "most_liked");
    assert_eq!(code("pt-BR").as_str(), "pt_br");
    for bad in ["", "1abc", "c:/users", "a.b", "me@x", "76561198012345678", "abc12345", &"a".repeat(40), "two words!"] {
        assert!(Code::new(bad).is_none(), "{bad}");
    }
    assert!(serde_json::from_str::<Code>("\"has space\"").is_err());
}

// --- the allowlist -------------------------------------------------------------------------

#[test]
fn every_type_has_a_sample_and_kinds_match() {
    let kinds: BTreeSet<&str> = samples().iter().map(Record::kind).collect();
    let listed: BTreeSet<&str> = KINDS.iter().copied().collect();
    assert_eq!(kinds, listed);
    assert_eq!(KINDS.len(), listed.len());
    for record in samples() {
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["type"], record.kind());
        // Round trip.
        assert_eq!(serde_json::from_value::<Record>(value).unwrap(), record);
    }
}

#[test]
fn unknown_types_and_fields_are_refused() {
    assert!(serde_json::from_str::<Record>(r#"{"type":"steam_profile","steam_id":"76561198012345678"}"#).is_err());
    assert!(serde_json::from_str::<Record>(r#"{"type":"launch","modded":true,"path":"C:\\x"}"#).is_err());
    assert!(serde_json::from_str::<Record>(r#"{"type":"launch","modded":true}"#).is_ok());
}

/// Fails if any record type can carry a Steam id, a path or free text other than the scrubbed
/// search field. Every string in every sample must be the type name, a [`Code`], the scrubbed
/// `search`, or a [`ModelName`] (`gpu`, `cpu`). Every number must be too small to be a Steam id.
#[test]
fn no_record_type_can_carry_a_steam_id_a_path_or_free_text() {
    fn walk(key: &str, value: &Value, out: &mut Vec<String>) {
        match value {
            Value::String(text) => {
                let ok = match key {
                    "type" => KINDS.contains(&text.as_str()),
                    "search" => scrub_search(text).is_some_and(|s| s.as_str() == text),
                    "gpu" | "cpu" => model_name(text).is_some_and(|m| m.as_str() == text),
                    _ => is_code(text),
                };
                let path_like = text.contains('/') || text.contains('\\') || text.contains(':');
                let steam_like = text.chars().filter(char::is_ascii_digit).count() >= 6;
                if !ok || path_like || steam_like {
                    out.push(format!("{key}={text}"));
                }
            }
            Value::Number(number) => {
                if number.as_u64().is_none_or(|n| n > MAX_GAMEBANANA_ID) {
                    out.push(format!("{key}={number}"));
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(key, item, out)),
            Value::Object(map) => map.iter().for_each(|(k, v)| walk(k, v, out)),
            Value::Bool(_) | Value::Null => {}
        }
    }
    let mut bad = Vec::new();
    for record in samples() {
        walk("", &serde_json::to_value(&record).unwrap(), &mut bad);
    }
    assert!(bad.is_empty(), "fields outside the rules: {bad:?}");

    // And the types refuse such values when they come from outside (a hand-edited queue).
    for (field, value) in [
        ("theme", "\"76561198012345678\""),
        ("theme", "\"C:\\\\Users\\\\kalo\""),
        ("theme", "\"free text here\""),
    ] {
        let text = format!(r#"{{"type":"settings","{field}":{value},"density":"compact","language":"de","mature":false}}"#);
        assert!(serde_json::from_str::<Record>(&text).is_err(), "{text}");
    }
    let search = r#"{"type":"browse_query","sort":null,"filters":[],"search":"/users/kalo","results":1}"#;
    assert!(serde_json::from_str::<Record>(search).is_err());
    let gpu = r#"{"type":"hardware","gpu":"SN 1234567","cpu":null,"ram_gb":16,"width":1920,"height":1080,"refresh_hz":60}"#;
    assert!(serde_json::from_str::<Record>(gpu).is_err());
}

/// The CLI and the agent host cannot emit or read product data: neither crate names the module.
#[test]
fn the_cli_and_the_agent_host_do_not_touch_product_data() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for name in ["dah-rs", "agent-host", "tinyagent"] {
        let mut stack = vec![crates.join(name).join("src")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = fs::read_to_string(&path).unwrap();
                    assert!(!text.contains("product_data"), "{} names product_data", path.display());
                }
            }
        }
    }
}

// --- sending -------------------------------------------------------------------------------

#[test]
fn flush_sends_the_envelope_logs_it_and_backs_off() {
    let dir = root("flush");
    write_consent(&dir, &consent(true, true)).unwrap();
    let recorder = Recorder::open(&dir);
    for record in samples() {
        recorder.record(record);
    }
    let context = Context { version: "0.1.38".into(), os_build: OsBuild::from_text("Windows 10.0.22631"), game_build: Some(5_911) };
    let server = Server::start(r#"{"accepted":21,"dropped":0}"#);
    assert_eq!(recorder.flush_at(&server.base, &context).unwrap(), samples().len());
    let bodies = server.bodies();
    let body: Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(body["installId"], recorder.install_id());
    assert_eq!(body["osBuild"], "10.0.22631");
    assert_eq!(body["gameBuild"], 5_911);
    assert_eq!(body["records"].as_array().unwrap().len(), samples().len());
    assert!(body["records"][0].get("at").is_none(), "local time is not sent");
    let keys: BTreeSet<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["installId", "version", "os", "osBuild", "gameBuild", "records"]));
    assert!(recorder.read_queue().is_empty());
    assert_eq!(recorder.shared_log().len(), samples().len());
    drop(server);

    // A failed send keeps the queue and waits before the next try.
    recorder.record(Record::Launch { modded: true });
    assert_eq!(recorder.flush_at("http://127.0.0.1:9", &context).unwrap(), 0);
    assert_eq!(recorder.read_queue().len(), 1);
    let server = Server::start("{}");
    assert_eq!(recorder.flush_at(&server.base, &context).unwrap(), 0);
    assert_eq!(server.hits(), 0, "backoff holds the next try");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_my_data_clears_local_state_and_makes_a_new_id() {
    let dir = root("delete");
    write_consent(&dir, &consent(true, true)).unwrap();
    let recorder = Recorder::open(&dir);
    let before = recorder.install_id();
    recorder.record(Record::Launch { modded: true });
    recorder.persist().unwrap();
    fs::write(recorder.log_path(), b"[]").unwrap();

    // A failed delete changes nothing locally.
    assert!(recorder.delete_at("http://127.0.0.1:9", "0.1.38").is_err());
    assert_eq!(recorder.install_id(), before);
    assert!(recorder.queue_path().exists());

    let server = Server::start(r#"{"deleted":4}"#);
    assert_eq!(recorder.delete_at(&server.base, "0.1.38").unwrap(), 4);
    let body: Value = serde_json::from_str(&server.bodies()[0]).unwrap();
    assert_eq!(body, serde_json::json!({ "installId": before }));
    assert!(!recorder.queue_path().exists());
    assert!(!recorder.log_path().exists());
    assert_ne!(recorder.install_id(), before);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn related_reads_mod_ids_only() {
    let server = Server::start(r#"{"mods":[{"modId":12,"installs":14},{"modId":0},{"modId":"x"}]}"#);
    assert_eq!(related_at(&server.base, id(5), "0.1.38").unwrap(), vec![id(12)]);
}
