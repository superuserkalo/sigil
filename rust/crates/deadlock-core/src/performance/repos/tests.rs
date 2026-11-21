use std::collections::HashMap;

use super::*;

const STOCK: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel\n\t\t}\n\t}\n\tConVars\n\t{\n\t\tr_shadows \"1\"\n\t\tfps_max \"400\"\n\t}\n}\n";
const AUTHOR: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel\n\t\t}\n\t}\n\tConVars\n\t{\n\t\tr_shadows \"0\"\n\t\tfps_max \"240\"\n\t\tr_particle_max_detail_level \"0\"\n\t\tsv_cheats \"hello\"\n\t\texec_this \"1\"\n\t}\n}\n";
const SNIPPET: &str = "// paste this\nConVars\n{\n\tr_shadows \"0\"\n}\n";
const VIDEO: &str = "\"video.cfg\"\n{\n\t\"setting.r_texture_budget\"\t\"1\"\n}\n";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

struct Fake {
    json: HashMap<String, Value>,
    bytes: HashMap<String, Vec<u8>>,
}

impl Remote for Fake {
    fn get_json(&self, url: &str) -> Result<Value> {
        self.json.get(url).cloned().ok_or_else(|| Error::msg(format!("no {url}")))
    }
    fn get_bytes(&self, url: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.bytes.get(url).cloned())
    }
}

fn blob(path: &str, text: &[u8]) -> Value {
    serde_json::json!({"path": path, "type": "blob", "mode": "100644", "sha": git_blob_sha(text), "size": text.len()})
}

fn fake() -> Fake {
    let files: Vec<(&str, &[u8])> = vec![
        ("Comp/gameinfo.gi", AUTHOR.as_bytes()),
        ("Comp/video.txt", VIDEO.as_bytes()),
        ("Snippet/gameinfo.txt", SNIPPET.as_bytes()),
        ("addons/pak01_dir.vpk", b"VPKDATA"),
        ("big/pak02_dir.vpk", b"x"),
        ("big/pak02_000.vpk", b"x"),
        ("notes.cfg", b"x"),
    ];
    let mut json = HashMap::new();
    let api = "https://api.github.com/repos/someone/fps";
    json.insert(
        format!("{api}/commits?per_page=1"),
        serde_json::json!([{"sha": COMMIT, "commit": {"committer": {"date": "2026-11-20T10:00:00Z"}}}]),
    );
    json.insert(
        format!("{api}/git/trees/{COMMIT}?recursive=1"),
        serde_json::json!({"truncated": false, "tree": files.iter().map(|(p, t)| blob(p, t)).collect::<Vec<_>>()}),
    );
    let bytes = files
        .iter()
        .map(|(p, t)| (format!("https://raw.githubusercontent.com/someone/fps/{COMMIT}/{p}"), t.to_vec()))
        .collect();
    Fake { json, bytes }
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sigil-repos-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_atomic(&fetch::file_path(&dir, super::super::catalog::BASELINE_ID, "gameinfo.gi"), STOCK.as_bytes()).unwrap();
    dir
}

#[test]
fn repository_names_are_checked() {
    assert_eq!(parse_repository("https://github.com/Someone/fps-cfg.git/").unwrap(), "Someone/fps-cfg");
    assert_eq!(parse_repository(" someone/fps ").unwrap(), "someone/fps");
    for bad in ["someone", "a/b/c", "some one/fps", "someone/..", "-x/y", "https://gitlab.com/a/b"] {
        assert!(parse_repository(bad).is_err(), "{bad}");
    }
    assert!(validate_path("a/../b").is_err());
    assert!(validate_path("a//b").is_err());
    assert!(validate_path("cfg/video.txt").is_ok());
}

#[test]
fn file_kinds_and_video_pairing() {
    assert!(is_config_path("x/gameinfo.gi") && is_config_path("Gameinfo (low).txt"));
    assert!(!is_config_path("readme.txt"));
    assert!(is_video_path("cfg/video(No Shadows).txt"));
    let videos = vec!["A/video.txt".to_string(), "A/video_low.txt".to_string(), "B/video.txt".to_string()];
    assert_eq!(suggested_video_path(Some("A/gameinfo.gi"), &videos), Ok(Some("A/video.txt".into())));
    assert_eq!(suggested_video_path(Some("C/gameinfo.gi"), &videos), Ok(None));
    assert_eq!(suggested_video_path(None, &videos), Err(()));
    let files = |paths: &[&str]| paths.iter().map(|p| RepoFile { path: p.to_string(), size: 1, sha: String::new() }).collect::<Vec<_>>();
    let standalone = standalone_addons(&files(&["a/pak01_dir.vpk", "b/pak02_dir.vpk", "b/pak02_000.vpk", "c/mod.vpk"]));
    let names: Vec<&str> = standalone.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["a/pak01_dir.vpk", "c/mod.vpk"]);
}

#[test]
fn a_convars_block_gets_a_root() {
    assert!(normalize_gameinfo(SNIPPET).starts_with("\"GameInfo\""));
    assert_eq!(normalize_gameinfo(AUTHOR), AUTHOR);
}

#[test]
fn only_plain_values_pass() {
    assert!(allowed("r_shadows", "0") && allowed("fps_max", "240.5") && allowed("x", "true") && allowed("y", "-1e3"));
    assert!(!allowed("R_Shadows", "0"));
    assert!(!allowed("sv_cheats", "hello"));
    assert!(!allowed("exec_this", "1"));
    assert!(!allowed("bind", "1"));
    assert!(!allowed("x", "1;quit"));
}

#[test]
fn inspect_import_switches_and_delete() {
    let remote = fake();
    let dir = data_dir("flow");
    let found = inspect(&remote, "https://github.com/someone/fps", None).unwrap();
    assert_eq!(found.commit, COMMIT);
    assert_eq!(found.date, "2026-11-20");
    assert_eq!(found.config_paths, ["Comp/gameinfo.gi", "Snippet/gameinfo.txt"]);
    assert_eq!(found.addons.iter().map(|a| a.path.as_str()).collect::<Vec<_>>(), ["addons/pak01_dir.vpk"]);
    assert_eq!(found.unsupported, ["notes.cfg"]);

    let choice = ImportChoice { name: "Comp".into(), config_path: Some("Comp/gameinfo.gi".into()), video_path: None };
    let id = import(&remote, &dir, &found, &choice, "2026-11-21T00:00:00Z").unwrap();
    assert!(id.starts_with(ID_PREFIX));
    let store = read_store(&dir).unwrap();
    let preset = store.get(&id).unwrap();
    assert_eq!(preset.video_path.as_deref(), Some("Comp/video.txt"), "the video file next to it was taken");

    let base = baseline(&dir).unwrap();
    let built = build(preset, &dir, &base, None).unwrap();
    let keys: Vec<&str> = built.config.body.convars.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"r_shadows") && keys.contains(&"fps_max"));
    assert!(built.withheld.iter().any(|k| k == "sv_cheats") && built.withheld.iter().any(|k| k == "exec_this"));
    assert!(built.config.body.section_ops.is_empty());
    assert_eq!(super::super::catalog::config(&id).map(|d| d.name), Some("Comp"));

    // A switch moves the setting out of the always-on list.
    set_toggles(&dir, &id, &[Toggle { key: "r_shadows".into(), label: "  Shadows off ".into() }]).unwrap();
    let store = read_store(&dir).unwrap();
    assert_eq!(store.get(&id).unwrap().toggles[0].label, "Shadows off");
    let built = build(store.get(&id).unwrap(), &dir, &base, None).unwrap();
    assert!(built.config.body.opt_ins.iter().any(|o| o.key == "r_shadows"));
    assert!(!built.config.body.convars.iter().any(|(k, _)| k == "r_shadows"));
    assert!(set_toggles(&dir, &id, &[Toggle { key: "nope".into(), label: "x".into() }]).is_err());

    // It shows up in the catalog after the built-ins.
    let catalog = super::super::load(&dir);
    assert!(catalog.get(&id).is_some());

    set_hide_builtins(&dir, true).unwrap();
    assert!(read_store(&dir).unwrap().hide_builtins);

    delete(&dir, &id).unwrap();
    assert!(read_store(&dir).unwrap().presets.is_empty());
    assert!(!fetch::file_path(&dir, &id, "gameinfo.gi").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_snippet_and_a_video_only_preset_import() {
    let remote = fake();
    let dir = data_dir("snippet");
    let found = inspect(&remote, "someone/fps", None).unwrap();
    let snippet = ImportChoice { name: "Snip".into(), config_path: Some("Snippet/gameinfo.txt".into()), video_path: Some(None) };
    let id = import(&remote, &dir, &found, &snippet, "2026-11-21T00:00:01Z").unwrap();
    let store = read_store(&dir).unwrap();
    let built = build(store.get(&id).unwrap(), &dir, &baseline(&dir).unwrap(), None).unwrap();
    assert_eq!(built.config.body.convars, vec![("r_shadows".to_string(), "0".to_string())]);

    let video = ImportChoice { name: "Video".into(), config_path: None, video_path: Some(Some("Comp/video.txt".into())) };
    let vid = import(&remote, &dir, &found, &video, "2026-11-21T00:00:02Z").unwrap();
    let store = read_store(&dir).unwrap();
    let built = build(store.get(&vid).unwrap(), &dir, &baseline(&dir).unwrap(), None).unwrap();
    assert!(built.config.body.convars.is_empty());
    assert_eq!(built.config.videos.len(), 1);

    let neither = ImportChoice { name: "None".into(), config_path: None, video_path: Some(None) };
    assert!(import(&remote, &dir, &found, &neither, "now").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_addon_must_match_the_tree() {
    let remote = fake();
    let dir = data_dir("addon");
    let found = inspect(&remote, "someone/fps", None).unwrap();
    let choice = ImportChoice { name: "Comp".into(), config_path: Some("Comp/gameinfo.gi".into()), video_path: Some(None) };
    let id = import(&remote, &dir, &found, &choice, "2026-11-21T00:00:03Z").unwrap();
    let good = |url: &str, _limit: u64| Ok(remote.bytes.get(url).cloned().unwrap_or_default());
    let path = download_addon(&good, &dir, &id, "addons/pak01_dir.vpk").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"VPKDATA");
    let bad = |_: &str, _: u64| Ok(b"VPKDATB".to_vec());
    assert!(download_addon(&bad, &dir, &id, "addons/pak01_dir.vpk").is_err());
    assert!(download_addon(&good, &dir, &id, "notes.cfg").is_err());
    mark_imported(&dir, &id, "addons/pak01_dir.vpk").unwrap();
    mark_imported(&dir, &id, "addons/pak01_dir.vpk").unwrap();
    assert_eq!(read_store(&dir).unwrap().get(&id).unwrap().imported, ["addons/pak01_dir.vpk"]);
    let _ = std::fs::remove_dir_all(&dir);
}
