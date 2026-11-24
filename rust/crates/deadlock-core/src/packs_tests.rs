use super::*;
use crate::gamebanana::{ModFile, ModImage, ModProfile};

/// A fresh folder per test, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let path = std::env::temp_dir().join(format!("sigil-packs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pack() -> Pack {
    Pack {
        name: "Friday stack".to_string(),
        note: Some("Hud and the quiet guns".to_string()),
        mods: vec![
            PackMod { mod_id: 741_478, file_id: Some(1_400_001), enabled: true },
            PackMod { mod_id: 741_608, file_id: None, enabled: false },
            PackMod { mod_id: 12, file_id: Some(99), enabled: true },
        ],
    }
}

fn doc(mods: Value) -> String {
    json!({
        "format": "mod-profile",
        "schemaVersion": "1.1",
        "profile": {"name": "Hostile"},
        "mods": mods,
    })
    .to_string()
}

fn one(reference: Value) -> String {
    doc(json!([{"source": "gamebanana", "ref": reference, "enabled": true, "priority": 1}]))
}

#[test]
fn round_trip_code_link_and_file() {
    let code = to_code(&pack()).expect("code");
    assert!(code.starts_with("mp1:"));
    assert!(code.len() < 400, "a small pack is a short code: {}", code.len());
    let read = read_pack_text(&code).expect("read");
    assert_eq!(read.pack, pack());
    assert_eq!((read.unsupported, read.duplicates), (0, 0));

    let link = to_link(&pack()).expect("link");
    assert!(link.starts_with(LINK_PREFIX));
    assert_eq!(read_pack_text(&link).expect("link").pack, pack());
    assert_eq!(read_pack_text(&link.to_uppercase().replace("SIGIL://PACK/", "sigil://PACK/")).is_err(), true, "the code is case sensitive");
    assert!(is_pack(&code) && is_pack(&link));

    // A code a chat app wrapped onto several lines still reads.
    let wrapped: String = code.chars().collect::<Vec<_>>().chunks(40).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("\n");
    assert_eq!(read_pack_text(&wrapped).expect("wrapped").pack, pack());

    let dir = Dir::new("round");
    let path = write_file(&dir.path().join("friday"), &pack()).expect("file");
    assert!(path.to_string_lossy().ends_with(FILE_EXTENSION));
    assert_eq!(read_pack_file(&path).expect("file").pack, pack());
}

#[test]
fn the_code_holds_only_pack_fields() {
    let json = serde_json::to_string(&to_document(&pack())).unwrap();
    for word in ["http", "://", "\\\\", "/Users", "exportedAt", "autoexec", "crosshair", "hint", "thumbnail"] {
        assert!(!json.contains(word), "{word} in {json}");
    }
}

#[test]
fn a_pack_is_also_a_profile_code_for_sigil_and_grimoire() {
    let code = to_code(&pack()).unwrap();
    let portable = crate::profiles::read_portable_input(&code).expect("profile import reads a pack");
    assert_eq!(portable.profile.name, "Friday stack");
    assert_eq!(portable.mods.iter().filter(|m| m.gamebanana_ref().is_some()).count(), 3);
}

#[test]
fn a_plain_profile_code_is_not_a_pack_but_still_reads_as_one() {
    let profile = json!({
        "format": "mod-profile", "schemaVersion": "1.0", "exportedAt": "2026-01-01T00:00:00Z",
        "exportedBy": {"tool": "grimoire", "version": "1"},
        "game": {"steamAppId": 1422450},
        "profile": {"name": "Theirs"},
        "mods": [
            {"source": "gamebanana", "ref": {"submissionId": 5, "fileId": 6, "section": "Mod"}, "enabled": true, "priority": 2,
             "hint": {"name": "B", "thumbnailUrl": "https://evil.example/x.png"}},
            {"source": "gamebanana", "ref": {"submissionId": 7, "fileId": 8}, "enabled": false, "priority": 1},
            {"source": "local", "ref": {"path": "C:\\mods\\x.vpk"}, "enabled": true, "priority": 3}
        ],
        "extensions": {"grimoire": {"autoexecCommands": ["quit"]}}
    })
    .to_string();
    let code = encode_share_code(&profile).unwrap();
    assert!(!is_pack(&code));
    let read = read_pack_text(&code).expect("read");
    assert_eq!(read.unsupported, 1);
    assert_eq!(read.pack.mods.iter().map(|m| m.mod_id).collect::<Vec<_>>(), vec![7, 5], "priority sets the order");
}

#[test]
fn priority_orders_and_duplicates_keep_their_first_place() {
    let json = doc(json!([
        {"source": "gamebanana", "ref": {"submissionId": 3}, "enabled": true, "priority": 3},
        {"source": "gamebanana", "ref": {"submissionId": 1}, "enabled": true, "priority": 1},
        {"source": "gamebanana", "ref": {"submissionId": 3, "fileId": 9}, "enabled": false, "priority": 2},
        {"source": "gamebanana", "ref": {"submissionId": 2}, "enabled": true, "priority": 2}
    ]));
    let read = parse_pack_json(&json).expect("read");
    assert_eq!(read.pack.mods.iter().map(|m| (m.mod_id, m.file_id)).collect::<Vec<_>>(), vec![(1, None), (3, Some(9)), (2, None)]);
    assert_eq!(read.duplicates, 1);
}

#[test]
fn hostile_ids_are_refused() {
    for bad in [
        json!({"submissionId": 0}),
        json!({"submissionId": -5}),
        json!({"submissionId": 1.5}),
        json!({"submissionId": "741478"}),
        json!({"submissionId": 4_294_967_296u64}),
        json!({"submissionId": u64::MAX}),
        json!({"submissionId": "../../etc/passwd"}),
        json!({"submissionId": "C:\\Windows\\system32"}),
        json!({"submissionId": [1]}),
        json!({"submissionId": {"a": 1}}),
        json!({"submissionId": null}),
        json!({"submissionId": 5, "fileId": "https://evil.example/x.zip"}),
        json!({"submissionId": 5, "fileId": -1}),
        json!({}),
    ] {
        assert!(parse_pack_json(&one(bad.clone())).is_err(), "{bad}");
    }
    // File id 0 is "the newest".
    assert_eq!(parse_pack_json(&one(json!({"submissionId": 5, "fileId": 0}))).unwrap().pack.mods[0].file_id, None);
    // Unknown fields, URLs among them, are ignored, never used.
    let read = parse_pack_json(&one(json!({"submissionId": 5, "url": "https://evil.example/x.zip", "path": "/etc"}))).expect("ok");
    assert_eq!(read.pack.mods, vec![PackMod { mod_id: 5, file_id: None, enabled: true }]);
}

#[test]
fn wrong_types_and_shapes_are_refused() {
    let cases = [
        "[]".to_string(),
        "null".to_string(),
        "\"mp1:\"".to_string(),
        json!({"format": "mod-profile", "schemaVersion": 1, "profile": {"name": "x"}, "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "2.0", "profile": {"name": "x"}, "mods": []}).to_string(),
        json!({"format": "other", "schemaVersion": "1.0", "profile": {"name": "x"}, "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": "x", "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": 7}, "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": "x", "description": ["a"]}, "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": "x"}, "mods": {"0": 1}}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": "x"}, "game": {"steamAppId": 570}, "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": "x"}, "game": "deadlock", "mods": []}).to_string(),
        json!({"format": "mod-profile", "schemaVersion": "1.1", "profile": {"name": "x"}, "mods": [], "extensions": {"sigil": {"pack": 99}}}).to_string(),
        doc(json!([])),
        doc(json!([1, 2])),
        doc(json!([{"source": "gamebanana", "ref": {"submissionId": 5}, "enabled": "yes", "priority": 1}])),
        doc(json!([{"source": "gamebanana", "ref": {"submissionId": 5}, "enabled": true, "priority": "1"}])),
        doc(json!([{"source": "gamebanana", "ref": {"submissionId": 5}, "enabled": true, "priority": -1}])),
        doc(json!([{"source": "gamebanana", "ref": {"submissionId": 5}, "enabled": true, "priority": 1e300}])),
        doc(json!([{"source": "gamebanana", "ref": [5], "enabled": true, "priority": 1}])),
        doc(json!([{"source": 5, "ref": {"submissionId": 5}, "enabled": true, "priority": 1}])),
    ];
    for case in cases {
        assert!(parse_pack_json(&case).is_err(), "{case}");
    }
}

#[test]
fn deep_nesting_is_refused_without_a_crash() {
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    assert!(parse_pack_json(&deep).is_err());
    let nested_note = format!(r#"{{"format":"mod-profile","schemaVersion":"1.0","profile":{{"name":"x","description":{}1{}}},"mods":[]}}"#, "[".repeat(5000), "]".repeat(5000));
    assert!(parse_pack_json(&nested_note).is_err());
}

#[test]
fn counts_and_sizes_are_capped() {
    let many: Vec<Value> = (1..=(MAX_MODS as u64 + 1))
        .map(|id| json!({"source": "gamebanana", "ref": {"submissionId": id}, "enabled": true, "priority": id}))
        .collect();
    let err = parse_pack_json(&doc(Value::Array(many.clone()))).unwrap_err().to_string();
    assert!(err.contains("300"), "{err}");
    assert!(parse_pack_json(&doc(Value::Array(many[..MAX_MODS].to_vec()))).is_ok());

    // A big pack still fits a code.
    let full = Pack {
        name: "All".into(),
        note: None,
        mods: (1..=MAX_MODS as u64).map(|id| PackMod { mod_id: 700_000 + id, file_id: Some(1_400_000 + id), enabled: true }).collect(),
    };
    assert!(to_code(&full).expect("fits").len() < MAX_CODE_BYTES);

    assert!(read_pack_text(&"A".repeat(MAX_CODE_BYTES + 1)).is_err());
    assert!(read_pack_text(&format!("mp1:{}", "A".repeat(MAX_CODE_BYTES))).is_err());
    // A gzip bomb: tiny code, huge JSON.
    let bomb = encode_share_code(&format!("{{\"format\":\"mod-profile\",\"x\":\"{}\"}}", " ".repeat(2 * 1024 * 1024))).unwrap();
    assert!(bomb.len() < MAX_CODE_BYTES);
    assert!(read_pack_text(&bomb).is_err());

    let long_name = "n".repeat(MAX_NAME_CHARS + 1);
    let json = json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": long_name}, "mods": [{"source": "gamebanana", "ref": {"submissionId": 5}, "enabled": true, "priority": 1}]});
    assert!(parse_pack_json(&json.to_string()).is_err());
    let long_note = "n".repeat(MAX_NOTE_CHARS + 1);
    assert!(check_note(&long_note).is_err());
    assert!(check_note(&"n".repeat(MAX_NOTE_CHARS)).is_ok());
}

#[test]
fn invisible_and_control_characters_are_refused() {
    for name in ["Fri\u{200B}day", "Fri\u{202E}yad", "a\u{0007}b", "a\nb", "\u{FEFF}x", "x\u{2066}y", "x\u{E0041}"] {
        assert!(check_name(name).is_err(), "{name:?}");
        let json = json!({"format": "mod-profile", "schemaVersion": "1.0", "profile": {"name": name}, "mods": [{"source": "gamebanana", "ref": {"submissionId": 5}, "enabled": true, "priority": 1}]});
        assert!(parse_pack_json(&json.to_string()).is_err(), "{name:?}");
    }
    assert!(check_note("line one\nline two").is_ok());
    assert!(check_note("bell\u{0007}").is_err());
    let code = to_code(&pack()).unwrap();
    assert!(read_pack_text(&format!("{}\u{200B}{}", &code[..10], &code[10..])).is_err());
    assert!(read_pack_text(&format!("{code}\u{0000}")).is_err());
}

#[test]
fn text_that_is_not_a_code_is_refused_and_never_read_as_a_path() {
    let dir = Dir::new("paths");
    let file = dir.path().join("p.json");
    fs::write(&file, to_json(&pack()).unwrap()).unwrap();
    for input in [String::new(), "   ".to_string(), file.display().to_string(), "../../etc/passwd".into(), "https://gamebanana.com/mods/1".into(), "mp1:!!!!".into(), "mp1:".into(), "sigil://pack/".into(), "sigil://pack/https://evil.example".into()] {
        assert!(read_pack_text(&input).is_err(), "{input}");
    }
    assert!(!is_pack("https://gamebanana.com/mods/1"));
    assert!(!is_pack("mp1:garbage"));
}

#[test]
fn creating_leaves_local_files_out() {
    let candidates = vec![
        Candidate { label: "Haze skin".into(), mod_id: Some(5), file_id: Some(6), enabled: true },
        Candidate { label: "my_local.vpk".into(), mod_id: None, file_id: None, enabled: true },
        Candidate { label: "Haze skin part 2".into(), mod_id: Some(5), file_id: Some(7), enabled: true },
        Candidate { label: "Hud".into(), mod_id: Some(9), file_id: None, enabled: false },
    ];
    let (pack, local) = from_candidates(" Mine ", "", &candidates).expect("pack");
    assert_eq!(pack.name, "Mine");
    assert_eq!(pack.note, None);
    assert_eq!(local, vec!["my_local.vpk".to_string()]);
    assert_eq!(pack.mods, vec![PackMod { mod_id: 5, file_id: Some(6), enabled: true }, PackMod { mod_id: 9, file_id: None, enabled: false }]);
    assert!(from_candidates("", "", &candidates).is_err());
    assert!(from_candidates("x", "", &candidates[1..2]).is_err());
}

fn profile(files: Vec<ModFile>) -> ModProfile {
    ModProfile {
        id: 5,
        name: "Quiet guns".into(),
        nsfw: true,
        images: vec![ModImage { url: "https://images.gamebanana.com/img/ss/mods/x.jpg".into(), ..ModImage::default() }],
        files,
        ..serde_json::from_value(json!({
            "id": 5, "name": "", "profileUrl": "", "text": "", "version": null, "addedTs": 0, "updatedTs": 0,
            "downloads": 0, "likes": 0, "views": 0, "postCount": 0, "updatesCount": 0, "nsfw": false,
            "contentRatings": [], "category": "Weapons", "categoryIcon": null, "superCategory": "Skins",
            "submitter": null, "files": [], "archivedFiles": [], "images": []
        }))
        .expect("profile")
    }
}

fn file(id: u64, added: i64, url: &str) -> ModFile {
    ModFile { id, name: format!("file{id}.zip"), size_bytes: 1_000 * id, downloads: 0, added_ts: added, description: String::new(), download_url: url.into(), archived: false }
}

#[test]
fn preview_states() {
    let ok = "https://gamebanana.com/dl/1";
    let p = profile(vec![file(1, 10, ok), file(2, 20, "https://gamebanana.com/dl/2")]);
    let pinned = facts_from_profile(&p, Some(1));
    assert_eq!((pinned.file_id, pinned.upgraded, pinned.size, pinned.nsfw), (Some(1), false, Some(1_000), true));
    assert_eq!(pinned.thumbnail.as_deref(), Some("https://images.gamebanana.com/img/ss/mods/x.jpg"));
    assert_eq!(row_state(false, Some(&Ok(pinned.clone()))), RowState::WillInstall);
    assert_eq!(row_state(true, Some(&Ok(pinned.clone()))), RowState::Installed);
    assert_eq!(row_state(true, None), RowState::Installed);
    assert_eq!(row_state(false, None), RowState::Checking);

    let gone = facts_from_profile(&p, Some(77));
    assert_eq!((gone.file_id, gone.upgraded), (Some(2), true), "a gone file falls back to the newest");
    let newest = facts_from_profile(&p, None);
    assert_eq!((newest.file_id, newest.upgraded), (Some(2), false));

    let mut withheld = p.clone();
    withheld.withheld = true;
    assert_eq!(row_state(false, Some(&Ok(facts_from_profile(&withheld, Some(1))))), RowState::Missing(Missing::Removed));
    assert_eq!(row_state(false, Some(&Err("GameBanana returned HTTP 404 for x".into()))), RowState::Missing(Missing::Removed));
    assert!(matches!(row_state(false, Some(&Err("timeout".into()))), RowState::Missing(Missing::Unreachable(_))));
    let empty = profile(vec![]);
    assert_eq!(row_state(false, Some(&Ok(facts_from_profile(&empty, None)))), RowState::Missing(Missing::NoFiles));

    // A download on any other host is never used, nor an image elsewhere.
    let mut hostile = profile(vec![file(3, 30, "https://evil.example/dl/3"), file(4, 40, "http://gamebanana.com/dl/4"), file(5, 50, "https://gamebanana.com.evil.example/dl/5")]);
    hostile.images = vec![ModImage { url: "https://evil.example/x.jpg".into(), ..ModImage::default() }];
    let facts = facts_from_profile(&hostile, Some(3));
    assert_eq!(facts.missing, Some(Missing::NoFiles));
    assert_eq!(facts.thumbnail, None);

    let mut script = p.clone();
    script.category = "Scripts".into();
    assert_eq!(row_state(false, Some(&Ok(facts_from_profile(&script, None)))), RowState::NeedsReview);
}

#[test]
fn install_order_keeps_the_pack_order() {
    let states = vec![RowState::WillInstall, RowState::Installed, RowState::NeedsReview, RowState::Missing(Missing::Removed), RowState::WillInstall];
    assert_eq!(install_order(&states, &[]), vec![0, 2, 4]);
    assert_eq!(install_order(&states, &[false, false, false, false, true]), vec![0, 2]);
}

#[test]
fn saved_packs_round_trip_and_skip_tampered_files() {
    let dir = Dir::new("saved");
    let ws = dir.path();
    let first = save_new(ws, &pack(), true).expect("save");
    let second = save_new(ws, &pack(), false).expect("save");
    assert_eq!(first.slug, "friday-stack");
    assert_eq!(second.slug, "friday-stack-2");
    let mut renamed = pack();
    renamed.name = "Saturday".into();
    store(ws, &first.slug, &renamed).expect("store");
    let list = list_packs(ws);
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|p| p.slug == "friday-stack" && p.pack.name == "Saturday" && p.mine));

    // A hand-edited file with a hidden character or a bad id is not listed.
    let path = packs_dir(ws).join("friday-stack-2.json");
    let text = fs::read_to_string(&path).unwrap().replace("Friday stack", "Fri\u{200B}day");
    fs::write(&path, text).unwrap();
    fs::write(packs_dir(ws).join("bad.json"), r#"{"slug":"bad","name":"x","mods":[{"modId":0,"enabled":true}],"mine":true,"createdAt":"","updatedAt":""}"#).unwrap();
    assert_eq!(list_packs(ws).len(), 1);

    delete(ws, "friday-stack").unwrap();
    delete(ws, "friday-stack").unwrap();
    assert!(delete(ws, "../escape").is_err());
    assert!(store(ws, "../../x", &pack()).is_err());
    assert!(list_packs(ws).is_empty());
}

#[test]
fn file_names_are_plain() {
    assert_eq!(file_name("Friday stack"), "Friday_stack.sigilpack.json");
    assert_eq!(file_name("../../etc"), "etc.sigilpack.json");
    assert_eq!(file_name("\u{202E}"), "pack.sigilpack.json");
}
