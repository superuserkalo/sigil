use super::*;

use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "dah-autoexec-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn backups(root: &Path) -> Vec<PathBuf> {
    let folder = autoexec_path(root).parent().unwrap().to_path_buf();
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("autoexec.cfg.dah-backup-")
        })
        .collect();
    found.sort();
    found
}

const SAMPLES: &[&str] = &[
    "",
    "\n",
    "\n\n\n",
    "fps_max 0",
    "fps_max 0\n",
    "fps_max 0\r\n",
    "// header\r\nfps_max 0\r\n\r\nbind \"F1\" \"say hi\"\r\n",
    "\u{feff}// bom file\nrate 1000000\n",
    "\u{feff}",
    "mixed\r\nendings\nhere\r\nlast",
    "lone\rcarriage\nreturn\n",
    "  \t// indented comment\n\t  \nweird ; chained ; line // trailing\n",
    "// dah:crosshair begin\ncitadel_crosshair_color_r 255\n// dah:crosshair end\n",
    "unicode \u{e9}\u{4e2d}\u{1f600} \"quoted value\"\n\n\n",
];

#[test]
fn parse_then_serialise_is_byte_exact() {
    for sample in SAMPLES {
        let document = parse_cfg(sample);
        assert_eq!(&serialise_cfg(&document), sample, "round trip of {sample:?}");
    }
}

#[test]
fn parse_keeps_each_line_ending() {
    let document = parse_cfg("a\r\nb\nc");
    assert_eq!(
        document.lines,
        vec![
            CfgLine { text: "a".into(), ending: LineEnding::CrLf },
            CfgLine { text: "b".into(), ending: LineEnding::Lf },
            CfgLine { text: "c".into(), ending: LineEnding::None },
        ]
    );
    assert!(!document.bom);
    assert!(parse_cfg("\u{feff}x\n").bom);
    assert!(parse_cfg("").lines.is_empty());
}

#[test]
fn edits_keep_untouched_lines_exact() {
    let original = "// keep me  \r\nunknown_thing \"a b\" ;  x\r\n\r\nfps_max 144\r\n// tail";
    let mut document = parse_cfg(original);
    document.lines[3].text = "fps_max 0".into();
    let ending = document.preferred_ending();
    assert_eq!(ending, LineEnding::CrLf);
    document.lines.push(CfgLine { text: "rate 1000000".into(), ending });
    let output = serialise_cfg(&document);
    assert_eq!(
        output,
        "// keep me  \r\nunknown_thing \"a b\" ;  x\r\n\r\nfps_max 0\r\n// tail\r\nrate 1000000\r\n"
    );
}

#[test]
fn a_moved_last_line_gets_an_ending() {
    let mut document = parse_cfg("a\nb");
    document.lines.swap(0, 1);
    assert_eq!(serialise_cfg(&document), "b\na\n");
}

#[test]
fn classify_lines() {
    assert_eq!(classify_line(""), CfgLineKind::Blank);
    assert_eq!(classify_line(" \t "), CfgLineKind::Blank);
    assert_eq!(classify_line("  // note"), CfgLineKind::Comment);
    assert_eq!(classify_line("//"), CfgLineKind::Comment);
    assert_eq!(classify_line("fps_max 0 // trailing"), CfgLineKind::Command);
    assert_eq!(classify_line("/ not a comment"), CfgLineKind::Command);
}

#[test]
fn split_comment_rebuilds_the_line() {
    for line in ["// Performance", "  //Network", "//", "\t//   spaced  "] {
        let (prefix, body) = split_comment(line).unwrap();
        assert_eq!(format!("{prefix}{body}"), line);
    }
    assert_eq!(split_comment("// Performance"), Some(("// ", "Performance")));
    assert_eq!(split_comment("fps_max 0"), None);
}

#[test]
fn command_names() {
    assert_eq!(command_name("fps_max 0").as_deref(), Some("fps_max"));
    assert_eq!(command_name("  FPS_MAX   240 // cap").as_deref(), Some("fps_max"));
    assert_eq!(command_name("\"rate\" \"1000000\"").as_deref(), Some("rate"));
    assert_eq!(command_name("fps_max 0; rate 1"), None);
    assert_eq!(command_name("// fps_max 0"), None);
    assert_eq!(command_name(""), None);
    assert_eq!(command_name("bind \"F1\" \"say a;b\"").as_deref(), None);
}

#[test]
fn same_command_ignores_spacing_case_and_quotes() {
    assert!(same_command("fps_max 0", "fps_max 0"));
    assert!(same_command("  FPS_MAX   0  // mine", "fps_max 0"));
    assert!(same_command("\"fps_max\" \"0\"", "fps_max 0"));
    assert!(!same_command("fps_max 144", "fps_max 0"));
    assert!(!same_command("// fps_max 0", "fps_max 0"));
    assert!(!same_command("fps_max", "fps_max 0"));
}

#[test]
fn place_command_finds_replaces_or_appends() {
    let lines = [
        "// perf",
        "fps_max 144",
        "rate 1000000",
        "fps_max 0; rate 1",
        "bind F1 \"fps_max 0\"",
    ];
    assert_eq!(place_command(&lines, "rate 1000000"), Placement::Present(2));
    assert_eq!(place_command(&lines, "fps_max 240"), Placement::Replace(1));
    assert_eq!(place_command(&lines, "r_low_latency 2"), Placement::Append);
    let empty: [&str; 0] = [];
    assert_eq!(place_command(&empty, "fps_max 0"), Placement::Append);
    // A chained line is never replaced: it may set other things too.
    assert_eq!(place_command(&["fps_max 0; rate 1"], "fps_max 240"), Placement::Append);
}

#[test]
fn the_catalog_is_grimoires_full_list() {
    let catalog = autoexec_presets();
    let names: Vec<&str> = catalog.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["Performance", "Network", "HUD and UI", "Minimap", "Matchmaking", "Mouse"]
    );
    let total: usize = catalog.iter().map(|c| c.presets.len()).sum();
    assert_eq!(total, 22);
    for preset in catalog.iter().flat_map(|c| c.presets.iter()) {
        assert!(!preset.name.is_empty() && !preset.description.is_empty());
        assert!(command_name(&preset.command).is_some(), "{}", preset.command);
    }
    assert_eq!(
        preset_for_line("  fps_max   0").map(|p| p.name.as_str()),
        Some("Uncap FPS")
    );
    assert!(preset_for_line("fps_max 99").is_none());
}

#[test]
fn normalize_keeps_the_file_style() {
    assert_eq!(normalize_trailing_newline("a\n\n\n"), "a\n");
    assert_eq!(normalize_trailing_newline("a\r\nb\r\n\r\n"), "a\r\nb\r\n");
    assert_eq!(normalize_trailing_newline("a"), "a\n");
    assert_eq!(normalize_trailing_newline("\n\n"), "");
}

#[test]
fn reveal_selects_the_file_or_opens_the_nearest_folder() {
    let root = temp_root("reveal");
    assert_eq!(autoexec_reveal_target(&root), RevealTarget::OpenFolder(root.clone()));
    let citadel = root.join("game").join("citadel");
    fs::create_dir_all(&citadel).unwrap();
    assert_eq!(autoexec_reveal_target(&root), RevealTarget::OpenFolder(citadel.clone()));
    fs::create_dir_all(citadel.join("cfg")).unwrap();
    assert_eq!(
        autoexec_reveal_target(&root),
        RevealTarget::OpenFolder(citadel.join("cfg"))
    );
    fs::write(autoexec_path(&root), "fps_max 0\n").unwrap();
    assert_eq!(
        autoexec_reveal_target(&root),
        RevealTarget::SelectFile(autoexec_path(&root))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn windows_arguments_quote_and_use_backslashes() {
    let path = Path::new("c:/program files (x86)/steam/steamapps/common/Deadlock/game/citadel/cfg/autoexec.cfg");
    assert_eq!(
        windows_select_argument(path),
        "/select,\"c:\\program files (x86)\\steam\\steamapps\\common\\Deadlock\\game\\citadel\\cfg\\autoexec.cfg\""
    );
    assert_eq!(windows_folder_argument(Path::new("C:/a b/cfg")), "\"C:\\a b\\cfg\"");
}

#[test]
fn write_refuses_without_approval_and_writes_nothing() {
    let root = temp_root("refuse");
    assert!(write_autoexec(&root, "fps_max 0", false).is_err());
    assert!(!autoexec_path(&root).exists());
    assert!(save_autoexec_if_unchanged(&root, "fps_max 0", "", false).is_err());
    assert!(!autoexec_path(&root).exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn write_refuses_nul_bytes() {
    let root = temp_root("nul");
    assert!(write_autoexec(&root, "fps_max\0 0", true).is_err());
    assert!(!autoexec_path(&root).exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn write_creates_the_file_then_backs_up_before_replacing() {
    let root = temp_root("write");
    let first = write_autoexec(&root, "fps_max 0", true).unwrap();
    assert_eq!(first.backup, None);
    assert_eq!(fs::read_to_string(autoexec_path(&root)).unwrap(), "fps_max 0\n");

    let second = write_autoexec(&root, "fps_max 240\n\n", true).unwrap();
    let backup = second.backup.expect("a backup of the first file");
    assert_eq!(fs::read_to_string(&backup).unwrap(), "fps_max 0\n");
    assert_eq!(fs::read_to_string(autoexec_path(&root)).unwrap(), "fps_max 240\n");
    assert_eq!(backups(&root), vec![backup]);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn checked_save_writes_when_the_file_is_unchanged() {
    let root = temp_root("checked");
    fs::create_dir_all(autoexec_path(&root).parent().unwrap()).unwrap();
    fs::write(autoexec_path(&root), "// mine\r\nfps_max 144\r\n").unwrap();
    let opened = read_autoexec(&root).unwrap().unwrap();
    let mut document = parse_cfg(&opened);
    document.lines[1].text = "fps_max 0".into();
    let text = serialise_cfg(&document);
    match save_autoexec_if_unchanged(&root, &text, &opened, true).unwrap() {
        AutoexecSave::Written(result) => assert!(result.backup.is_some()),
        other => panic!("expected a write, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(autoexec_path(&root)).unwrap(),
        "// mine\r\nfps_max 0\r\n"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn checked_save_refuses_when_another_writer_changed_the_file() {
    let root = temp_root("conflict");
    fs::create_dir_all(autoexec_path(&root).parent().unwrap()).unwrap();
    fs::write(autoexec_path(&root), "fps_max 144\n").unwrap();
    let opened = "fps_max 144\n";
    fs::write(autoexec_path(&root), "fps_max 144\n// dah:crosshair begin\n").unwrap();
    let outcome = save_autoexec_if_unchanged(&root, "fps_max 0\n", opened, true).unwrap();
    assert_eq!(
        outcome,
        AutoexecSave::ChangedOnDisk {
            current: "fps_max 144\n// dah:crosshair begin\n".into()
        }
    );
    assert_eq!(
        fs::read_to_string(autoexec_path(&root)).unwrap(),
        "fps_max 144\n// dah:crosshair begin\n"
    );
    assert!(backups(&root).is_empty());

    // A file that did not exist when the editor opened must still not exist.
    let fresh = temp_root("conflict-fresh");
    fs::create_dir_all(autoexec_path(&fresh).parent().unwrap()).unwrap();
    fs::write(autoexec_path(&fresh), "rate 1\n").unwrap();
    assert!(matches!(
        save_autoexec_if_unchanged(&fresh, "fps_max 0", "", true).unwrap(),
        AutoexecSave::ChangedOnDisk { .. }
    ));
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&fresh).unwrap();
}
