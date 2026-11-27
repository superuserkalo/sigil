use std::collections::{BTreeMap, BTreeSet};

use super::fetch::{CachedConfig, Remote};
use super::*;

const STOCK: &str = include_str!("../../tests/fixtures/stock-gameinfo.gi");

/// An upstream-shaped config built from the stock file: every shape the
/// generator has to place.
fn upstream() -> String {
    STOCK
        // engine section: change, add, comment out
        .replacen(
            "IndexBufferPoolSizeMB              \"32\"",
            "IndexBufferPoolSizeMB \"16\"",
            1,
        )
        .replacen(
            "VulkanDefrag                       \"1\"",
            "//VulkanDefrag \"1\"",
            1,
        )
        .replacen(
            "MinStreamingPoolSizeMB             \"1024\"",
            "MinStreamingPoolSizeMB \"1024\"\n        NewPoolKey \"7\"",
            1,
        )
        // root key
        .replacen("perfwizard  \"0\"", "perfwizard  \"1\"", 1)
        // excluded key and Valve's stamp
        .replacen("DistanceField \"1\"", "DistanceField \"0\"", 1)
        .replacen(
            "PGIVersion                   \"6E09",
            "PGIVersion                   \"AAAA",
            1,
        )
        // convars: edit one, add the rest
        .replacen(
            "sv_lagcomp_filterbyviewangle \"false\"",
            concat!(
                "sv_lagcomp_filterbyviewangle \"true\"\n",
                "        r_ssao \"0\"\n",
                "        r_shadows \"0\"\n",
                "        cl_phys_timescale \"1\"\n",
                "        host_timescale \"2\"\n",
                "        citadel_trooper_glow_disabled \"1\"\n",
                "        citadel_camera_hero_fov \"95\"\n",
                "        citadel_hideout_enable_testing_tools \"1\"\n",
                "        r_citadel_brand_new_glow \"1\"\n",
                "        // r_commented_convar \"1\"",
            ),
            1,
        )
}

fn game() -> GameConvars {
    GameConvars::parse(concat!(
        "r_ssao true (clientdll)\n\tAmbient occlusion.\n",
        "r_shadows true (cheat gameinfo_cannot_override)\n\tShadows.\n",
        "cl_phys_timescale 1 (clientdll)\n\t<no description>\n",
        "citadel_trooper_glow_disabled false (clientdll)\n\t<no description>\n",
        "citadel_camera_hero_fov 90 (clientdll)\n\t<no description>\n",
        "citadel_hideout_enable_testing_tools false (clientdll)\n\t<no description>\n",
        "sv_lagcomp_filterbyviewangle false (gamedll)\n\t<no description>\n",
    ))
}

fn meta(id: &str) -> CachedConfig {
    CachedConfig {
        id: id.into(),
        version: "3.9".into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        date: "2026-11-04".into(),
        path: "Sqooky's .gi/gameinfo.gi".into(),
        blob: String::new(),
        videos: Vec::new(),
    }
}

fn config(id: &str, text: &str) -> Config {
    let def = catalog::config(id).unwrap();
    Config::from_texts(
        def,
        &meta(id),
        &generate::parse(STOCK),
        text,
        &[],
        Some(&game()),
    )
}

fn all_switches(config: &Config) -> Options {
    Options {
        opt_ins: config.switchable().map(|opt| opt.key.clone()).collect(),
        engine: true,
        video: None,
        edits: BTreeMap::new(),
    }
}

#[test]
fn generator_places_every_shape() {
    let config = config("sqooky", &upstream());
    let body = &config.body;
    let op = |key: &str| body.section_ops.iter().find(|op| op.key == key);
    assert_eq!(
        op("IndexBufferPoolSizeMB").unwrap().value.as_deref(),
        Some("16")
    );
    assert!(op("VulkanDefrag").unwrap().remove);
    assert_eq!(
        op("NewPoolKey").unwrap().path,
        vec!["RenderSystem".to_string()]
    );
    assert_eq!(op("perfwizard").unwrap().path, vec!["GameInfo".to_string()]);
    assert!(op("DistanceField").is_none());
    assert!(op("PGIVersion").is_none());

    let convar = |key: &str| body.convars.iter().any(|(k, _)| k == key);
    assert!(convar("sv_lagcomp_filterbyviewangle"));
    assert!(convar("r_ssao"));
    assert!(
        convar("cl_phys_timescale"),
        "allowed despite the timescale pattern"
    );
    assert!(!convar("host_timescale"));
    assert!(
        !convar("r_commented_convar"),
        "a commented convar is not a removal"
    );

    let group = |key: &str| {
        body.opt_ins
            .iter()
            .find(|opt| opt.key == key)
            .map(|opt| opt.group)
    };
    assert_eq!(
        group("citadel_trooper_glow_disabled"),
        Some(Group::Visibility)
    );
    assert_eq!(group("citadel_camera_hero_fov"), Some(Group::Camera));
    assert_eq!(
        group("citadel_hideout_enable_testing_tools"),
        Some(Group::Devtools)
    );

    let held: Vec<&str> = body.held.iter().map(|held| held.key.as_str()).collect();
    for key in [
        "DistanceField",
        "PGIVersion",
        "host_timescale",
        "r_citadel_brand_new_glow",
    ] {
        assert!(held.contains(&key), "{key} should be held back: {held:?}");
    }
    assert!(body.held.iter().all(|held| !held.reason.is_empty()));
}

#[test]
fn convar_list_marks_ignored_and_unknown() {
    let config = config("sqooky", &upstream());
    assert_eq!(config.convar_state("r_shadows"), ConvarState::Ignored);
    assert_eq!(config.convar_state("r_ssao"), ConvarState::Read);
    assert!(config.body_convars().all(|(key, _)| key != "r_shadows"));
    let patched = apply_text(STOCK, &config, &config.default_options()).unwrap();
    assert!(
        !patched.text.contains("r_shadows"),
        "ignored convars are not written"
    );
}

#[test]
fn defaults_turn_on_visibility_and_camera_but_not_devtools() {
    let config = config("sqooky", &upstream());
    let options = config.default_options();
    assert!(options.engine);
    assert!(options.opt_ins.contains("citadel_trooper_glow_disabled"));
    assert!(options.opt_ins.contains("citadel_camera_hero_fov"));
    assert!(
        !options
            .opt_ins
            .contains("citadel_hideout_enable_testing_tools")
    );
}

#[test]
fn every_switch_combination_round_trips_byte_exact() {
    let config = config("sqooky", &upstream());
    let defaults = config.default_options();
    let everything = all_switches(&config);
    let nothing = Options {
        opt_ins: BTreeSet::new(),
        engine: false,
        video: None,
        edits: BTreeMap::new(),
    };
    for options in [defaults, everything, nothing] {
        let patched = apply_text(STOCK, &config, &options).expect("apply");
        assert_ne!(patched.text, STOCK);
        assert_eq!(
            patched.written,
            config.setting_count(&options) - patched.skipped.len()
        );
        match applied(&patched.text) {
            Applied::Config {
                id,
                version,
                switches,
                ..
            } => {
                assert_eq!(id, "sqooky");
                assert_eq!(version, "3.9");
                let (on, engine) = switches.expect("switches recorded");
                assert_eq!(on, options.opt_ins);
                assert_eq!(engine, options.engine);
            }
            other => panic!("unexpected {other:?}"),
        }
        for key in &options.opt_ins {
            assert!(
                patched.text.contains(key.as_str()),
                "{key} on but not written"
            );
        }
        assert_eq!(patched.text.contains("NewPoolKey"), options.engine);
        assert_eq!(remove_text(&patched.text), STOCK, "did not round trip");
    }
}

#[test]
fn a_switched_off_opt_in_is_not_written() {
    let config = config("sqooky", &upstream());
    let mut options = config.default_options();
    options.opt_ins.remove("citadel_camera_hero_fov");
    let patched = apply_text(STOCK, &config, &options).unwrap();
    let body = patched
        .text
        .lines()
        .filter(|line| !line.contains("switches:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!body.contains("citadel_camera_hero_fov"));
    assert!(body.contains("citadel_trooper_glow_disabled"));
}

#[test]
fn crlf_files_stay_crlf_and_round_trip() {
    let crlf = STOCK.replace('\n', "\r\n");
    let config = config("sqooky", &upstream());
    let patched = apply_text(&crlf, &config, &config.default_options()).unwrap();
    assert!(
        !patched.text.replace("\r\n", "").contains('\n'),
        "bare LF leaked"
    );
    assert_eq!(remove_text(&patched.text), crlf);
}

#[test]
fn switching_configs_never_stacks_two() {
    let first = config("sqooky", &upstream());
    let second = config("boot", &upstream().replace("r_ssao \"0\"", "r_ssao \"1\""));
    let one = apply_text(STOCK, &first, &first.default_options()).unwrap();
    let two = apply_text(&one.text, &second, &all_switches(&second)).unwrap();
    assert_eq!(
        two.text.matches("Sigil Performance Config BEGIN").count(),
        1
    );
    assert!(matches!(applied(&two.text), Applied::Config { ref id, .. } if id == "boot"));
    assert_eq!(remove_text(&two.text), STOCK);
}

#[test]
fn grimoire_markers_are_left_alone() {
    let foreign = STOCK.replacen(
        "ConVars\n    {",
        "ConVars\n    {\n        // grimoire-perf added",
        1,
    );
    assert_eq!(applied(&foreign), Applied::Foreign);
    let config = config("sqooky", &upstream());
    assert!(apply_text(&foreign, &config, &config.default_options()).is_err());
    assert_eq!(remove_text(&foreign), foreign);
}

#[test]
fn older_markers_without_switches_still_read() {
    let text = STOCK.replacen(
        "ConVars\n    {",
        "ConVars\n    {\n        // ==== Sigil Performance Config BEGIN (preset=sqooky v3.4 @e2e9925cdd9d) ====",
        1,
    );
    assert_eq!(
        applied(&text),
        Applied::Config {
            id: "sqooky".into(),
            version: "3.4".into(),
            commit: "e2e9925cdd9d".into(),
            switches: None
        }
    );
}

#[test]
fn edited_lines_record_and_restore_their_value() {
    let text = "GameInfo\n{\n\tConVars\n\t{\n\t\tfoo \"1\"\n\t}\n}\n";
    let op = SectionOp {
        path: vec!["ConVars".into()],
        key: "foo".into(),
        value: Some("2".into()),
        remove: false,
    };
    let edited = apply_op(text, &op).unwrap();
    assert!(edited.contains("foo \"2\" // sigil-perf was \"1\""));
    assert_eq!(remove_markers(&edited), text);
    let gone = apply_op(
        text,
        &SectionOp {
            remove: true,
            value: None,
            ..op
        },
    )
    .unwrap();
    assert!(gone.contains("// sigil-perf removed: foo \"1\""));
    assert_eq!(remove_markers(&gone), text);
}

#[test]
fn missing_convars_section_is_refused() {
    let text = "GameInfo\n{\n\tgame \"citadel\"\n}\n";
    let config = config("sqooky", &upstream());
    assert!(apply_text(text, &config, &config.default_options()).is_err());
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sigil-perf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn install_apply_keeps_a_backup_and_removes_cleanly() {
    let root = temp_dir("install");
    let citadel = root.join("game").join("citadel");
    std::fs::create_dir_all(&citadel).unwrap();
    std::fs::write(citadel.join("gameinfo.gi"), STOCK).unwrap();
    let config = config("sqooky", &upstream());
    apply_to_install(&root, &config, &config.default_options()).unwrap();
    assert!(matches!(
        read_status(&root).unwrap(),
        Applied::Config { .. }
    ));
    assert_eq!(std::fs::read_to_string(backup_path(&root)).unwrap(), STOCK);
    // A second apply does not replace the backup with a patched file.
    apply_to_install(&root, &config, &all_switches(&config)).unwrap();
    assert_eq!(std::fs::read_to_string(backup_path(&root)).unwrap(), STOCK);
    assert!(remove_from_install(&root).unwrap());
    assert_eq!(
        std::fs::read_to_string(gameinfo_path(&root)).unwrap(),
        STOCK
    );
    assert!(!remove_from_install(&root).unwrap());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parse_keeps_the_last_duplicate_and_commented_entries() {
    let parsed = generate::parse(
        "GameInfo\n{\n\tConVars\n\t{\n\t\ta \"1\"\n\t\ta \"2\"\n\t}\n\tX\n\t{\n\t\t//b \"3\"\n\t}\n}\n",
    );
    assert_eq!(parsed.value("ConVars/a"), Some("2"));
    let stock = generate::parse("GameInfo\n{\n\tX\n\t{\n\t\tb \"3\"\n\t}\n}\n");
    let body = generate::generate(&stock, &parsed);
    assert!(body.section_ops.iter().any(|op| op.key == "b" && op.remove));
}

#[test]
fn stated_versions_come_from_comments_only() {
    assert_eq!(
        catalog::stated_version("// OptimizationLock Ver. 3.9 by Sqooky\n\tVersion 5\n").as_deref(),
        Some("3.9")
    );
    assert_eq!(
        catalog::stated_version("// OptimizationLock Maxfps Version Ver.1.8 by Sqooky").as_deref(),
        Some("1.8")
    );
    assert_eq!(catalog::stated_version("\t\tVersion 5\n"), None);
}

#[test]
fn git_blob_hash_matches_git() {
    assert_eq!(
        fetch::git_blob_sha(b""),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
    assert_eq!(
        fetch::git_blob_sha(b"hello\n"),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
}

// --- video.txt ---------------------------------------------------------------

const VIDEO: &str = "\"video.cfg\"\n{\n\t\"Version\"\t\t\"16\"\n\t\"DeviceID\"\t\t\"1234\"\n\t\"setting.defaultres\"\t\t\"2560\"\n\t\"setting.fullscreen\"\t\t\"1\"\n\t\"setting.r_citadel_antialiasing\"\t\t\"1\"\n\t\"setting.r_texture_stream_mip_bias\"\t\t\"0\"\n\t\"setting.mat_vsync\"\t\t\"1\"\n}\n";

#[test]
fn video_settings_keep_the_players_display() {
    let upstream = generate::parse_video(concat!(
        "\"setting.defaultres\" \"-1\"\n\"setting.fullscreen\" \"0\"\n\"setting.mat_vsync\" \"0\"\n",
        "\"setting.fps_max\" \"0\"\n\"setting.r_citadel_antialiasing\" \"0\"\n",
        "\"setting.r_texture_stream_mip_bias\" \"6\"\n\"setting.r_citadel_outlines\" \"0\"\n",
    ));
    let (capped, _) = generate::generate_video(&upstream, false);
    assert_eq!(
        capped,
        vec![
            ("r_citadel_antialiasing".to_string(), "0".to_string()),
            ("r_texture_stream_mip_bias".to_string(), "4".to_string()),
        ]
    );
    let (verbatim, _) = generate::generate_video(&upstream, true);
    assert_eq!(verbatim[1].1, "6");
}

#[test]
fn video_apply_and_revert_round_trip() {
    let settings = vec![
        ("r_citadel_antialiasing".to_string(), "0".to_string()),
        ("r_texture_stream_mip_bias".to_string(), "4".to_string()),
        ("r_new_setting".to_string(), "2".to_string()),
    ];
    let (next, original, written) = video::apply_text(VIDEO, &settings, None).unwrap();
    assert!(next.contains("\"setting.r_new_setting\"\t\t\"2\"\n}"));
    assert!(next.contains("\"setting.defaultres\"\t\t\"2560\""));
    let record = video::VideoRecord {
        config: "x".into(),
        file: "video.txt".into(),
        path: "video.txt".into(),
        original,
        written,
    };
    assert_eq!(video::still_written(&next, &record), 3);
    assert_eq!(video::revert_text(&next, &record), VIDEO);
    // The game rewrote 0 as 0.000000 (not a change) and the player changed AA.
    let in_game = next
        .replace(
            "\"setting.r_citadel_antialiasing\"\t\t\"0\"",
            "\"setting.r_citadel_antialiasing\"\t\t\"2\"",
        )
        .replace(
            "\"setting.r_new_setting\"\t\t\"2\"",
            "\"setting.r_new_setting\"\t\t\"2.000000\"",
        );
    let reverted = video::revert_text(&in_game, &record);
    assert!(
        reverted.contains("\"setting.r_citadel_antialiasing\"\t\t\"2\""),
        "player's value kept"
    );
    assert!(!reverted.contains("r_new_setting"));
    // Reapplying over an earlier write keeps the first originals.
    let (_, again, _) = video::apply_text(&next, &settings, Some(&record)).unwrap();
    assert_eq!(
        again.get("r_citadel_antialiasing"),
        Some(&Some("1".to_string()))
    );
    assert!(video::apply_text("not a video file", &settings, None).is_err());
}

#[test]
fn video_install_write_backs_up_and_restores() {
    let dir = temp_dir("video");
    let data = dir.join("data");
    let path = dir.join("video.txt");
    std::fs::write(&path, VIDEO.replace('\n', "\r\n")).unwrap();
    let settings = vec![("r_citadel_antialiasing".to_string(), "0".to_string())];
    assert_eq!(
        video::apply_install(&data, &path, "optilock-fps", "video.txt", &settings).unwrap(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(video::backup_path(&path)).unwrap(),
        VIDEO.replace('\n', "\r\n")
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("antialiasing\"\t\t\"0\"\r\n")
    );
    assert_eq!(video::read_record(&data).unwrap().config, "optilock-fps");
    assert!(video::revert_install(&data).unwrap());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        VIDEO.replace('\n', "\r\n")
    );
    assert!(!video::revert_install(&data).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keeping_video_values_forgets_the_record_and_leaves_the_file() {
    let dir = temp_dir("video-keep");
    let data = dir.join("data");
    let path = dir.join("video.txt");
    std::fs::write(&path, VIDEO).unwrap();
    let settings = vec![("r_citadel_antialiasing".to_string(), "0".to_string())];
    video::apply_install(&data, &path, "optilock-fps", "video.txt", &settings).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(video::keep_install(&data).unwrap());
    assert!(video::read_record(&data).is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    assert!(!video::revert_install(&data).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn video_txt_is_found_in_the_newest_steam_account() {
    let dir = temp_dir("locate");
    let game_root = dir.join("steamapps").join("common").join("Deadlock");
    std::fs::create_dir_all(game_root.join("game/citadel/cfg")).unwrap();
    std::fs::write(game_root.join("game/citadel/cfg/video.txt"), VIDEO).unwrap();
    assert_eq!(
        video::locate(&game_root, &[]),
        Some(game_root.join("game/citadel/cfg/video.txt"))
    );
    let cfg = dir.join("userdata/42/1422450/local/cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("video.txt"), VIDEO).unwrap();
    assert_eq!(video::locate(&game_root, &[]), Some(cfg.join("video.txt")));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- fetch and cache ---------------------------------------------------------

/// Serves a fake GitHub from memory and counts file downloads.
struct FakeGitHub {
    files: BTreeMap<String, Vec<u8>>,
    tamper: Option<String>,
    downloads: std::sync::Mutex<usize>,
    /// The API refuses the convar list's commit lookup (a rate limit).
    convars_api_down: bool,
}

const SQ_COMMIT: &str = "1111111111111111111111111111111111111111";
const OL_COMMIT: &str = "2222222222222222222222222222222222222222";
const OL_TAG_OBJECT: &str = "3333333333333333333333333333333333333333";
const GT_COMMIT: &str = "4444444444444444444444444444444444444444";

impl FakeGitHub {
    fn new() -> FakeGitHub {
        let mut files = BTreeMap::new();
        files.insert(
            format!("Sqooky/OptimizationLock/{}", catalog::BASELINE.path),
            STOCK.as_bytes().to_vec(),
        );
        files.insert(
            "Sqooky/OptimizationLock/Sqooky's .gi/gameinfo.gi".into(),
            upstream().into_bytes(),
        );
        files.insert(
            "Sqooky/OptimizationLock/boot's maxium fps config/gameinfo.gi".into(),
            format!("// OptimizationLock -- ver. 2 --\n{}", upstream()).into_bytes(),
        );
        files.insert(
            "dacooderr/OptiLock/OptiLock FPS Config (Recommended)/gameinfo.gi".into(),
            upstream().replace("\n", "\r\n").into_bytes(),
        );
        let video = "\"video.cfg\"\n{\n\t\"setting.r_citadel_antialiasing\"\t\"0\"\n\t\"setting.r_post_bloom\"\t\"false\"\n\t\"setting.r_depth_of_field\"\t\"false\"\n\t\"setting.fps_max\"\t\"0\"\n\t\"setting.mat_vsync\"\t\"0\"\n\t\"setting.r_low_latency\"\t\"1\"\n}\n";
        files.insert(
            "dacooderr/OptiLock/OptiLock FPS Config (Recommended)/video.txt".into(),
            video.as_bytes().to_vec(),
        );
        let mut convars = String::new();
        for index in 0..1200 {
            convars.push_str(&format!(
                "fake_convar_{index} 0 (clientdll)\n\t<no description>\n"
            ));
        }
        convars.push_str("r_shadows true (cheat gameinfo_cannot_override)\n");
        files.insert(
            format!("{}/{}", fetch::CONVARS_REPO, fetch::CONVARS_PATH),
            convars.into_bytes(),
        );
        FakeGitHub {
            files,
            tamper: None,
            downloads: std::sync::Mutex::new(0),
            convars_api_down: false,
        }
    }

    fn tree(&self, repo: &str) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .files
            .iter()
            .filter_map(|(key, bytes)| key.strip_prefix(&format!("{repo}/")).map(|path| (path, bytes)))
            .map(|(path, bytes)| serde_json::json!({"path": path, "type": "blob", "sha": fetch::git_blob_sha(bytes)}))
            .collect();
        serde_json::json!({ "tree": items })
    }
}

impl Remote for FakeGitHub {
    fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let api = "https://api.github.com/repos/";
        let rest = url.strip_prefix(api).expect("api url");
        Ok(if rest.starts_with("Sqooky/OptimizationLock/commits") {
            serde_json::json!([{ "sha": SQ_COMMIT, "commit": {"committer": {"date": "2026-11-04T10:00:00Z"}} }])
        } else if rest.starts_with("Sqooky/OptimizationLock/git/trees/") {
            self.tree("Sqooky/OptimizationLock")
        } else if rest == "dacooderr/OptiLock/releases/latest" {
            serde_json::json!({ "tag_name": "v6.1d", "published_at": "2026-11-01T00:00:00Z" })
        } else if rest == "dacooderr/OptiLock/git/ref/tags/v6.1d" {
            serde_json::json!({ "object": {"type": "tag", "sha": OL_TAG_OBJECT} })
        } else if rest == format!("dacooderr/OptiLock/git/tags/{OL_TAG_OBJECT}") {
            serde_json::json!({ "object": {"sha": OL_COMMIT} })
        } else if rest.starts_with("dacooderr/OptiLock/git/trees/") {
            self.tree("dacooderr/OptiLock")
        } else if rest.starts_with(&format!("{}/commits", fetch::CONVARS_REPO)) {
            if self.convars_api_down {
                return Err(crate::error::Error::Network(
                    "GitHub is rate limiting requests from this network. Try again in an hour.".into(),
                ));
            }
            serde_json::json!([{ "sha": GT_COMMIT }])
        } else {
            panic!("unexpected api call {url}")
        })
    }

    fn get_bytes(&self, url: &str) -> Result<Option<Vec<u8>>> {
        *self.downloads.lock().unwrap() += 1;
        let rest = url
            .strip_prefix("https://raw.githubusercontent.com/")
            .expect("raw url");
        let mut parts = rest.splitn(4, '/');
        let (owner, repo, _commit, path) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let path = path
            .replace("%20", " ")
            .replace("%27", "'")
            .replace("%28", "(")
            .replace("%29", ")");
        let key = format!("{owner}/{repo}/{path}");
        let mut bytes = self.files.get(&key).cloned();
        if self.tamper.as_deref() == Some(key.as_str())
            && let Some(bytes) = bytes.as_mut()
        {
            bytes.extend_from_slice(b"\n// injected");
        }
        Ok(bytes)
    }
}

#[test]
fn the_convar_list_downloads_alone_and_replaces_a_damaged_copy() {
    let data = temp_dir("convar-list");
    let remote = FakeGitHub::new();
    fetch::refresh_convar_list(&remote, &data).unwrap();
    assert!(!crate::autoexec::convars::load_catalog(&data).convars.is_empty());
    let commit = fetch::read_index(&data).convars_commit;
    assert!(commit.is_some());

    let path = fetch::file_path(&data, fetch::CONVARS_ID, fetch::CONVARS_FILE);
    std::fs::write(&path, b"damaged").unwrap();
    assert!(crate::autoexec::convars::load_catalog(&data).convars.is_empty());
    fetch::refresh_convar_list(&remote, &data).unwrap();
    assert!(!crate::autoexec::convars::load_catalog(&data).convars.is_empty());
    assert_eq!(fetch::read_index(&data).convars_commit, commit);
}

#[test]
fn refresh_caches_configs_and_loads_offline() {
    let data = temp_dir("fetch");
    let remote = FakeGitHub::new();
    let report = fetch::refresh(&remote, &data, "2026-11-17T00:00:00.000Z").unwrap();
    assert!(
        report.updated.contains(&"Sqooky's config".to_string()),
        "{report:?}"
    );
    let first = *remote.downloads.lock().unwrap();

    let catalog = load(&data);
    assert!(catalog.game_convars);
    let sqooky = catalog.get("sqooky").expect("sqooky cached");
    assert_eq!(sqooky.commit, SQ_COMMIT);
    assert_eq!(sqooky.convar_state("r_shadows"), ConvarState::Ignored);
    let boot = catalog.get("boot").unwrap();
    assert_eq!(boot.version, "2");
    let optilock = catalog.get("optilock-fps").unwrap();
    assert_eq!(optilock.version, "v6.1d");
    assert_eq!(optilock.commit, OL_COMMIT);
    let video = optilock.video(None).expect("video settings");
    assert!(
        video
            .settings
            .iter()
            .all(|(key, _)| key != "fps_max" && key != "mat_vsync")
    );
    assert!(catalog.get("piggy").is_none(), "not served, not cached");
    assert!(catalog.errors.contains_key("piggy"));
    // Lighter tiers first.
    assert!(
        catalog
            .configs
            .windows(2)
            .all(|pair| pair[0].def.tier <= pair[1].def.tier)
    );
    // Every cached config round trips on the stock file and on CRLF.
    for config in &catalog.configs {
        for options in [config.default_options(), all_switches(config)] {
            let patched = apply_text(STOCK, config, &options).unwrap();
            assert_eq!(remove_text(&patched.text), STOCK, "{}", config.id());
        }
    }

    // A second refresh with nothing new downloads nothing.
    fetch::refresh(&remote, &data, "2026-11-17T01:00:00.000Z").unwrap();
    assert_eq!(*remote.downloads.lock().unwrap(), first);
    assert!(!fetch::stale(
        &fetch::read_index(&data),
        chrono::DateTime::parse_from_rfc3339("2026-11-17T02:00:00Z")
            .unwrap()
            .into(),
        12
    ));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_file_that_does_not_match_its_commit_is_refused() {
    let data = temp_dir("tamper");
    let mut remote = FakeGitHub::new();
    remote.tamper = Some("Sqooky/OptimizationLock/Sqooky's .gi/gameinfo.gi".into());
    fetch::refresh(&remote, &data, "2026-11-17T00:00:00.000Z").unwrap();
    let catalog = load(&data);
    assert!(catalog.get("sqooky").is_none());
    assert!(catalog.errors["sqooky"].contains("does not match"));
    assert!(catalog.get("boot").is_some());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn an_empty_cache_loads_empty() {
    let data = temp_dir("empty");
    let catalog = load(&data);
    assert!(catalog.is_empty());
    assert!(needs_refresh(&data, 12));
    let _ = std::fs::remove_dir_all(&data);
}

/// Live: fetch every config from GitHub and round trip each on the stock
/// fixture and the fetched stock file. `cargo test -p deadlock-core
/// performance -- --ignored`.
#[test]
#[ignore = "network"]
fn live_upstream_configs_round_trip() {
    let data = temp_dir("live");
    let report = refresh(&data).expect("refresh");
    eprintln!("updated {:?}\nerrors {:?}", report.updated, report.errors);
    let catalog = load(&data);
    let baseline =
        std::fs::read_to_string(fetch::file_path(&data, catalog::BASELINE_ID, "gameinfo.gi"))
            .unwrap();
    assert!(!catalog.is_empty());
    for config in &catalog.configs {
        let video: usize = config.videos.iter().map(|set| set.settings.len()).sum();
        eprintln!(
            "{:<24} {:<8} {} {} | engine {} convars {} opt-ins {} held {} ignored {} unknown {} video {}",
            config.id(),
            config.version,
            config.date,
            &config.commit[..8],
            config.body.section_ops.len(),
            config.body.convars.len(),
            config.body.opt_ins.len(),
            config.body.held.len(),
            config.ignored.len(),
            config.unknown.len(),
            video,
        );
        for stock in [STOCK, baseline.as_str()] {
            for options in [config.default_options(), all_switches(config)] {
                let patched = apply_text(stock, config, &options).unwrap();
                assert_eq!(remove_text(&patched.text), stock, "{}", config.id());
                let crlf = stock.replace('\n', "\r\n");
                let patched = apply_text(&crlf, config, &options).unwrap();
                assert_eq!(remove_text(&patched.text), crlf, "{} crlf", config.id());
            }
        }
    }
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn player_edits_ride_with_the_config_and_leave_with_it() {
    let config = config("sqooky", &upstream());
    let mut options = all_switches(&config);
    options
        .edits
        .insert("sv_lagcomp_filterbyviewangle".into(), "false".into());
    options
        .edits
        .insert("r_added_by_player".into(), "1".into());
    let patched = apply_text(STOCK, &config, &options).expect("apply");
    assert!(patched
        .text
        .contains("sv_lagcomp_filterbyviewangle"));
    assert!(patched.text.contains("\"false\""));
    assert!(patched.text.contains("r_added_by_player"));
    assert_eq!(patched.written, config.setting_count(&options) - patched.skipped.len());
    let back = remove_text(&patched.text);
    assert!(!back.contains("r_added_by_player"));
    assert!(back.contains("sv_lagcomp_filterbyviewangle \"false\""));
    assert_eq!(convar_values(&patched.text).get("r_added_by_player").map(String::as_str), Some("1"));
}

/// Live timings for the page's load path: a cold refresh from GitHub, then `load` from the
/// cache it left. `DAH_PERF_DIR=<dir> cargo test -p deadlock-core live_perf_timings --
/// --ignored --nocapture`; the directory is kept for later runs.
#[test]
#[ignore = "network"]
fn live_perf_timings() {
    let data = std::env::var("DAH_PERF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| temp_dir("timings"));
    let started = std::time::Instant::now();
    let report = refresh(&data).expect("refresh");
    eprintln!("refresh {:?} errors {:?}", started.elapsed(), report.errors);
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let catalog = load(&data);
        eprintln!("load {:?} ({} configs)", started.elapsed(), catalog.configs.len());
    }
    let text = std::fs::read_to_string(fetch::file_path(&data, fetch::CONVARS_ID, fetch::CONVARS_FILE)).unwrap();
    let started = std::time::Instant::now();
    let game = GameConvars::parse(&text);
    eprintln!("convar list parse {:?} ({})", started.elapsed(), game.len());
    let index = fetch::read_index(&data);
    let base = generate::parse(&std::fs::read_to_string(fetch::file_path(&data, catalog::BASELINE_ID, "gameinfo.gi")).unwrap());
    let (mut parse, mut build) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    for def in catalog::CONFIGS {
        let Some(meta) = index.configs.get(def.id) else { continue };
        let text = std::fs::read_to_string(fetch::file_path(&data, def.id, "gameinfo.gi")).unwrap();
        let started = std::time::Instant::now();
        let parsed = generate::parse(&text);
        parse += started.elapsed();
        let started = std::time::Instant::now();
        let _ = Config::from_parsed(def, meta, &base, &parsed, &[], Some(&game));
        build += started.elapsed();
    }
    let started = std::time::Instant::now();
    let custom = repos::configs(&data, &base, Some(&game)).len();
    eprintln!("config parse {parse:?}, diff and classify {build:?}, custom {custom} {:?}", started.elapsed());
    let ids: Vec<&str> = load(&data).configs.iter().map(|c| c.id()).collect();
    eprintln!("{ids:?}");
}


/// OptiLock's video.txt as upstream ships it: the settings with no `"config" {` header, so it
/// ends on a closing brace with no opening one.
const HEADERLESS_VIDEO: &str = "\t\"setting.r_citadel_antialiasing\"\t\t\"0\"\n\t\"setting.r_post_bloom\"\t\t\"false\"\n\t\"setting.r_depth_of_field\"\t\t\"false\"\n\t\"setting.r_texture_stream_mip_bias\"\t\t\"6\"\n\t\"setting.r_citadel_mboit\"\t\t\"false\"\n\t\"setting.r_screen_space_shadows\"\t\t\"false\"\n}\n";

/// Potato-shaped: the shared upstream edits plus draw distance and particle cuts.
fn potato_upstream() -> String {
    upstream().replacen(
        "r_ssao \"0\"",
        "r_ssao \"0\"\n        r_farz \"4000\"\n        cl_particle_fallback_multiplier \"5\"",
        1,
    )
}

/// Regression (0.1.22 to now): the game-file guard refused Sigil's cache copy of OptiLock's
/// headerless video.txt, so every OptiLock config (FPS, Community, Potato, Ultra Potato) failed
/// its download and was missing from the list.
#[test]
fn optilock_with_a_headerless_video_txt_is_listed() {
    let data = temp_dir("headerless");
    let mut remote = FakeGitHub::new();
    remote.files.insert(
        "dacooderr/OptiLock/OptiLock Potato Config/gameinfo.gi".into(),
        potato_upstream().into_bytes(),
    );
    remote.files.insert(
        "dacooderr/OptiLock/OptiLock Potato Config/video.txt".into(),
        HEADERLESS_VIDEO.as_bytes().to_vec(),
    );
    remote.files.insert(
        "dacooderr/OptiLock/OptiLock FPS Config (Recommended)/video.txt".into(),
        HEADERLESS_VIDEO.as_bytes().to_vec(),
    );
    let report = fetch::refresh(&remote, &data, "2026-11-27T00:00:00.000Z").unwrap();
    // The fake repository ships only these two OptiLock configs; neither may fail.
    assert!(
        report.errors.iter().all(|e| !e.starts_with("OptiLock FPS") && !e.starts_with("OptiLock Potato")),
        "{report:?}"
    );
    let catalog = load(&data);
    let potato = catalog.get("optilock-potato").expect("OptiLock Potato is listed");
    assert_eq!(potato.def.tier, Tier::Potato);
    assert_eq!(potato.version, "v6.1d");
    assert!(potato.body.convars.iter().any(|(k, v)| k == "r_farz" && v == "4000"));
    let video = potato.video(None).expect("its video settings");
    assert!(!video.settings.is_empty(), "{video:?}");
    assert!(catalog.get("optilock-fps").is_some());
    // The cache copy is byte for byte what the author committed.
    let cached = std::fs::read(fetch::file_path(&data, "optilock-potato", "video.txt")).unwrap();
    assert_eq!(cached, HEADERLESS_VIDEO.as_bytes());
    let _ = std::fs::remove_dir_all(&data);
}

/// A video.txt that fails its own check leaves the video settings out, not the config.
#[test]
fn a_bad_video_txt_drops_only_the_video_settings() {
    let data = temp_dir("bad-video");
    let mut remote = FakeGitHub::new();
    remote.files.insert(
        "dacooderr/OptiLock/OptiLock FPS Config (Recommended)/video.txt".into(),
        b"not a video file".to_vec(),
    );
    fetch::refresh(&remote, &data, "2026-11-27T00:00:00.000Z").unwrap();
    let catalog = load(&data);
    let fps = catalog.get("optilock-fps").expect("still listed");
    assert!(fps.videos.is_empty());
    assert!(!catalog.errors.contains_key("optilock-fps"));
    let _ = std::fs::remove_dir_all(&data);
}

/// GitHub's API limit must not leave the convar list (Autoexec > All convars) empty: the file
/// itself comes from raw.githubusercontent.com at the default branch.
#[test]
fn the_convar_list_downloads_when_the_api_refuses() {
    let data = temp_dir("convars-raw");
    let mut remote = FakeGitHub::new();
    remote.convars_api_down = true;
    fetch::refresh_convar_list(&remote, &data).unwrap();
    assert!(crate::autoexec::convars::load_catalog(&data).convars.len() > 500);
    assert_eq!(fetch::read_index(&data).convars_commit, None, "asks the API again next time");
    // With a list on disk, a refused API keeps it and says why.
    let error = fetch::refresh_convar_list(&remote, &data).unwrap_err().to_string();
    assert!(error.contains("rate limiting"), "{error}");
    assert!(!crate::autoexec::convars::load_catalog(&data).convars.is_empty());
    let _ = std::fs::remove_dir_all(&data);
}

/// A fake install: `<root>/game/citadel/gameinfo.gi` holding the stock fixture.
fn fake_install(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = temp_dir(&format!("{name}-game"));
    let data = temp_dir(&format!("{name}-data"));
    let citadel = root.join("game").join("citadel");
    std::fs::create_dir_all(&citadel).unwrap();
    std::fs::write(citadel.join("gameinfo.gi"), STOCK).unwrap();
    (root, data)
}

/// The exact file an apply leaves, against a reviewed copy in `tests/fixtures/performance`.
/// `DAH_BLESS=1` rewrites the copy after a deliberate change.
fn assert_golden(name: &str, text: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/performance")
        .join(name);
    if std::env::var_os("DAH_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} missing", path.display()));
    assert_eq!(text, expected, "{name}");
}

fn apply_check_and_compare(id: &str, upstream_text: &str, golden: &str) {
    let (root, data) = fake_install(id);
    let config = config(id, upstream_text);
    let options = config.default_options();
    let report = apply_and_check(&root, &data, &config, &options, "2026-11-27T00:00:00Z").unwrap();
    let text = std::fs::read_to_string(gameinfo_path(&root)).unwrap();
    assert_golden(golden, &text);
    assert_eq!(report.path, gameinfo_path(&root));
    assert_eq!(report.in_file, settings_in_file(&text));
    assert!(report.in_file > 0 && report.written >= report.in_file - report.skipped.len());
    // Every value the config means is in the file, as the game reads it (last line wins).
    let values = convar_values(&text);
    for (key, value) in exchange::effective(&config, &options) {
        assert_eq!(values.get(&key), Some(&value), "{id} {key}");
    }
    assert!(matches!(applied(&text), Applied::Config { id: ref on, .. } if on == id));
    assert_eq!(read_record(&data).unwrap().id, id);
    assert!(reset_since_apply(&text, read_record(&data).as_ref()).is_none());

    // Turn off: the stock file, byte for byte.
    assert!(remove_from_install(&root).unwrap());
    forget_record(&data).unwrap();
    assert_eq!(std::fs::read_to_string(gameinfo_path(&root)).unwrap(), STOCK);
    assert!(read_record(&data).is_none());

    // A game update puts Valve's file back after an apply: the page sees the reset from the
    // file plus the record, and applying again gives the same file as the first time.
    apply_and_check(&root, &data, &config, &options, "2026-11-27T00:00:00Z").unwrap();
    std::fs::write(gameinfo_path(&root), STOCK).unwrap();
    let reset = std::fs::read_to_string(gameinfo_path(&root)).unwrap();
    assert_eq!(applied(&reset), Applied::None);
    let notice = reset_since_apply(&reset, read_record(&data).as_ref()).expect("reset noticed");
    assert_eq!(notice.id, id);
    apply_and_check(&root, &data, &config, &options, "2026-11-27T01:00:00Z").unwrap();
    assert_golden(golden, &std::fs::read_to_string(gameinfo_path(&root)).unwrap());
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn optilock_potato_applies_exactly_turns_off_and_reapplies_after_a_reset() {
    apply_check_and_compare("optilock-potato", &potato_upstream(), "optilock-potato.gi");
}

#[test]
fn sqooky_applies_exactly_turns_off_and_reapplies_after_a_reset() {
    apply_check_and_compare("sqooky", &upstream(), "sqooky.gi");
}

/// The applied file passes the game-file guard and the page's own reader.
#[test]
fn an_applied_file_is_well_formed_keyvalues() {
    let config = config("optilock-potato", &potato_upstream());
    let patched = apply_text(STOCK, &config, &all_switches(&config)).unwrap();
    assert_eq!(crate::game_apply::keyvalues_syntax_error(&patched.text), None);
}
