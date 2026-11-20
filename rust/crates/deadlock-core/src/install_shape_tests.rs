//! The install regression corpus, generated: one archive per shape players upload, built in the
//! test (no game files, no third-party mods), planned against a fake game root. Real uploads are
//! covered by `tests/install_corpus.rs`; `docs/design/install-robustness.md` lists every shape.

use super::*;
use super::tests::{build_stored_zip, fake_game, make_vpk, scratch};

fn options(game_root: &Path) -> PlanOptions {
    PlanOptions { game_root: Some(game_root.to_path_buf()), tools: ToolPaths::default(), ..PlanOptions::default() }
}

fn plan_of(root: &Path, game_root: &Path, archive: &Path) -> Result<InstallPlan> {
    let request = InstallRequest::new(archive.display().to_string(), root.join("workspace")).unwrap();
    plan_install(&request, &options(game_root))
}

fn choices(root: &Path, archive: &Path) -> Vec<VpkChoice> {
    let request = InstallRequest::new(archive.display().to_string(), root.join("workspace")).unwrap();
    vpk_choices(&request, &PlanOptions { tools: ToolPaths::default(), ..PlanOptions::default() }).unwrap()
}

fn plan_choice(root: &Path, game_root: &Path, archive: &Path, choice: &str) -> Result<InstallPlan> {
    let request = InstallRequest::new(archive.display().to_string(), root.join("workspace"))
        .unwrap()
        .with_vpk(Some(choice.to_string()));
    plan_install(&request, &options(game_root))
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let with_modes: Vec<(&str, &[u8], u32)> = entries.iter().map(|(name, data)| (*name, *data, 0o100644)).collect();
    build_stored_zip(&with_modes)
}

/// A self-contained version 1 VPK (data inside the directory file, as `Josuke Remaster.vpk`
/// ships), which `pack_vpk` never writes.
fn self_contained_vpk(entries: &[(&str, &str, &str, &[u8])]) -> Vec<u8> {
    let mut tree = Vec::new();
    let mut data = Vec::new();
    for (extension, folder, name, bytes) in entries {
        for part in [*extension, *folder, *name] {
            tree.extend_from_slice(part.as_bytes());
            tree.push(0);
        }
        tree.extend_from_slice(&crc32fast::hash(bytes).to_le_bytes());
        tree.extend_from_slice(&0u16.to_le_bytes());
        tree.extend_from_slice(&0x7fffu16.to_le_bytes());
        tree.extend_from_slice(&(data.len() as u32).to_le_bytes());
        tree.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        tree.extend_from_slice(&0xffffu16.to_le_bytes());
        data.extend_from_slice(bytes);
        tree.extend_from_slice(&[0, 0]);
    }
    tree.push(0);
    let mut out = Vec::new();
    out.extend_from_slice(&0x55aa_1234u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(tree.len() as u32).to_le_bytes());
    out.extend_from_slice(&tree);
    out.extend_from_slice(&data);
    out
}

fn skin_vpk() -> Vec<u8> {
    self_contained_vpk(&[("vmat_c", "materials/heroes/shiv", "shiv_color", b"skin")])
}

fn map_vpk() -> Vec<u8> {
    self_contained_vpk(&[("vwrld_c", "maps/flat/world", "world", b"map")])
}

#[test]
fn canonical_vpk_names_drop_copy_decorations() {
    assert_eq!(shapes::canonical_vpk_name("pak98_dir (1).vpk"), "pak98_dir.vpk");
    assert_eq!(shapes::canonical_vpk_name("PAK98_DIR(2).VPK"), "pak98_dir.vpk");
    assert_eq!(shapes::canonical_vpk_name("pak98_dir - Copy.vpk"), "pak98_dir.vpk");
    assert_eq!(shapes::canonical_vpk_name("Josuke Remaster.vpk"), "josuke remaster.vpk");
    assert_eq!(shapes::canonical_vpk_name("skin (v2).vpk"), "skin (v2).vpk");
    assert!(shapes::is_junk_name("._pak01_dir.vpk") && shapes::is_junk_name("__MACOSX") && shapes::is_junk_name("Thumbs.db"));
    assert!(!shapes::is_junk_name(".disable_gb1click"));
}

#[test]
fn a_vpk_anywhere_under_any_name_and_case_installs() {
    let root = scratch("shape-anywhere");
    let game_root = fake_game(&root);
    let archive = root.join("Mod Name ünïcode.zip");
    fs::write(
        &archive,
        zip_of(&[
            ("Shiv Josuke ジョジョ/Version 2/JOSUKE REMASTER.VPK", &skin_vpk()),
            ("Shiv Josuke ジョジョ/preview.png", b"png"),
            ("Shiv Josuke ジョジョ/README.txt", b"read me"),
            ("__MACOSX/Shiv Josuke ジョジョ/._JOSUKE REMASTER.VPK", b"apple double"),
            (".DS_Store", b"junk"),
        ]),
    )
    .unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    assert!(matches!(plan.artifact, InstallArtifact::Vpk { .. }));
    assert_eq!(plan.files, vec!["materials/heroes/shiv/shiv_color.vmat_c".to_string()]);
    let record = apply_install(&plan, true).unwrap();
    assert!(record.target.ends_with("pak01_dir.vpk"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_copy_decorated_vpk_set_keeps_its_parts() {
    let root = scratch("shape-copy-name");
    let game_root = fake_game(&root);
    let (dir_vpk, chunks) = make_vpk(&root, "pak07", &[("sounds/hit.vsnd_c", b"hit")]);
    let chunk = fs::read(&chunks[0]).unwrap();
    let archive = root.join("hit.zip");
    fs::write(&archive, zip_of(&[("pak07_dir (1).vpk", &fs::read(&dir_vpk).unwrap()), ("pak07_000.vpk", &chunk)])).unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    match &plan.artifact {
        InstallArtifact::Vpk { chunk_paths, .. } => assert_eq!(chunk_paths.len(), 1),
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn variant_folders_with_the_same_vpk_names_each_keep_their_own_parts() {
    let root = scratch("shape-variants");
    let game_root = fake_game(&root);
    let (red_dir, red_chunks) = make_vpk(&root.join("r"), "pak83", &[("materials/red.vmat_c", b"red")]);
    let (blue_dir, blue_chunks) = make_vpk(&root.join("b"), "pak83", &[("materials/blue.vmat_c", b"blue")]);
    let archive = root.join("tailed.zip");
    fs::write(
        &archive,
        zip_of(&[
            ("Beard/pak83_dir.vpk", &fs::read(&red_dir).unwrap()),
            ("Beard/pak83_000.vpk", &fs::read(&red_chunks[0]).unwrap()),
            ("No Beard/pak83_dir.vpk", &fs::read(&blue_dir).unwrap()),
            ("No Beard/pak83_000.vpk", &fs::read(&blue_chunks[0]).unwrap()),
        ]),
    )
    .unwrap();
    let error = plan_of(&root, &game_root, &archive).unwrap_err().to_string();
    assert!(error.contains("more than one VPK"), "{error}");
    let listed = choices(&root, &archive);
    assert_eq!(listed.iter().map(|c| c.name().to_string()).collect::<Vec<_>>(), vec!["Beard/pak83_dir.vpk", "No Beard/pak83_dir.vpk"]);
    assert!(listed.iter().all(|choice| choice.chunks == 1));
    let blue = plan_choice(&root, &game_root, &archive, "No Beard/pak83_dir.vpk").unwrap();
    assert_eq!(blue.files, vec!["materials/blue.vmat_c".to_string()]);
    match &blue.artifact {
        InstallArtifact::Vpk { chunk_paths, .. } => {
            assert_eq!(chunk_paths.len(), 1);
            assert!(chunk_paths[0].to_string_lossy().contains("No Beard"));
        }
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn archives_inside_archives_are_unpacked() {
    let root = scratch("shape-nested");
    let game_root = fake_game(&root);
    let inner = zip_of(&[("pak05_dir.vpk", &skin_vpk())]);
    let middle = zip_of(&[("files/inner.zip", &inner), ("readme.txt", b"hi")]);
    let archive = root.join("outer.zip");
    fs::write(&archive, zip_of(&[("Mod/middle.zip", &middle)])).unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    assert!(matches!(plan.artifact, InstallArtifact::Vpk { .. }));
    assert_eq!(plan.files, vec!["materials/heroes/shiv/shiv_color.vmat_c".to_string()]);

    // A .zip of variant .zips is a picker.
    let variants = root.join("variants.zip");
    fs::write(
        &variants,
        zip_of(&[("red.zip", &zip_of(&[("pak01_dir.vpk", &skin_vpk())])), ("blue.zip", &zip_of(&[("pak01_dir.vpk", &skin_vpk())]))]),
    )
    .unwrap();
    let error = plan_of(&root, &game_root, &variants).unwrap_err().to_string();
    assert!(error.contains("more than one VPK"), "{error}");
    let listed: Vec<String> = choices(&root, &variants).iter().map(|c| c.path.clone()).collect();
    assert_eq!(listed, vec!["blue/pak01_dir.vpk".to_string(), "red/pak01_dir.vpk".to_string()]);

    // 7z and RAR inside a ZIP open with the built-in readers.
    let mixed = root.join("mixed.zip");
    let seven = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives/two_files_with_content_lzma.7z")).unwrap();
    let rar = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives/solid.rar")).unwrap();
    fs::write(&mixed, zip_of(&[("a.7z", &seven), ("b.rar", &rar)])).unwrap();
    let out = root.join("mixed-out");
    let report = extract_archive(&mixed, &out, &ExtractLimits::default(), &ToolPaths::default()).unwrap();
    assert!(report.files.iter().any(|f| f.starts_with("a/")) && report.files.iter().any(|f| f.starts_with("b/")), "{:?}", report.files);
    assert!(!report.files.iter().any(|f| f.ends_with(".7z") || f.ends_with(".rar")));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn loose_compiled_files_install_from_any_packaging_folder() {
    let root = scratch("shape-loose");
    let game_root = fake_game(&root);
    for (name, prefix) in [
        ("plain", ""),
        ("game", "game/citadel/"),
        ("citadel", "citadel/"),
        ("addons", "addons/"),
        ("named", "Cool Hud v2 (Final)/"),
        ("deep", "Cool Hud v2/Deadlock/game/citadel/"),
    ] {
        let path = |rest: &str| format!("{prefix}{rest}");
        let archive = root.join(format!("{name}.zip"));
        fs::write(
            &archive,
            zip_of(&[
                (&path("panorama/images/hud/a.vtex_c"), b"tex"),
                (&path("panorama/images/hud/a.png"), b"source png"),
                (&path("materials/hud/a.vmat"), b"source vmat"),
                (&path("panorama/fonts/Inter.ttf"), b"font"),
                (&path("resource/localization/citadel_gc/citadel_gc_english.txt"), b"\"lang\" {}"),
                ("README.md", b"read me"),
                ("setup.exe", b"MZ"),
                ("preview.jpg", b"jpg"),
            ]),
        )
        .unwrap();
        let plan = plan_of(&root, &game_root, &archive).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(matches!(plan.artifact, InstallArtifact::Staging { .. }), "{name}");
        assert_eq!(
            plan.files,
            vec![
                "panorama/fonts/Inter.ttf".to_string(),
                "panorama/images/hud/a.vtex_c".to_string(),
                "resource/localization/citadel_gc/citadel_gc_english.txt".to_string(),
            ],
            "{name}"
        );
        let warnings = plan.warnings.join("\n");
        assert!(warnings.contains("a.png") && warnings.contains("setup.exe"), "{name}: {warnings}");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn loose_variant_folders_are_a_choice() {
    let root = scratch("shape-loose-variants");
    let game_root = fake_game(&root);
    let archive = root.join("bars.zip");
    fs::write(
        &archive,
        zip_of(&[
            ("Single Bar/panorama/images/bar.vtex_c", b"single"),
            ("Double Bar/game/citadel/panorama/images/bar.vtex_c", b"double"),
        ]),
    )
    .unwrap();
    let error = plan_of(&root, &game_root, &archive).unwrap_err().to_string();
    assert!(error.contains("more than one VPK") && error.contains("Single Bar") && error.contains("Double Bar"), "{error}");
    let listed = choices(&root, &archive);
    assert_eq!(listed.iter().map(|c| c.name().to_string()).collect::<Vec<_>>(), vec!["Double Bar", "Single Bar"]);
    let picked = plan_choice(&root, &game_root, &archive, &listed[1].path).unwrap();
    assert_eq!(picked.files, vec!["panorama/images/bar.vtex_c".to_string()]);
    assert_eq!(picked.bytes, 6);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_map_vpk_is_payload_not_an_addon() {
    let root = scratch("shape-map");
    let game_root = fake_game(&root);
    let archive = root.join("map.zip");
    fs::write(&archive, zip_of(&[("My Map/maps/flat.vpk", &map_vpk()), ("My Map/readme.txt", b"type map flat")])).unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    assert!(matches!(plan.artifact, InstallArtifact::Staging { .. }));
    assert_eq!(plan.files, vec!["maps/flat.vpk".to_string()]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_bare_vpk_file_installs_with_its_parts() {
    let root = scratch("shape-bare");
    let game_root = fake_game(&root);
    let loose = root.join("Josuke Remaster.vpk");
    fs::write(&loose, skin_vpk()).unwrap();
    let plan_one = plan_of(&root, &game_root, &loose).unwrap();
    assert_eq!(plan_one.format, ArchiveFormat::Vpk);
    assert_eq!(plan_one.files.len(), 1);

    let folder = root.join("set");
    fs::create_dir_all(&folder).unwrap();
    let (dir_vpk, chunks) = make_vpk(&folder, "skin", &[("models/a.vmdl_c", b"model")]);
    assert_eq!(chunks.len(), 1);
    let plan_set = plan_of(&root, &game_root, &dir_vpk).unwrap();
    match &plan_set.artifact {
        InstallArtifact::Vpk { chunk_paths, .. } => assert_eq!(chunk_paths.len(), 1),
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_font_pack_installs_its_fonts() {
    let root = scratch("shape-fonts");
    let game_root = fake_game(&root);
    let archive = root.join("fonts.zip");
    fs::write(
        &archive,
        zip_of(&[("fonts/Inter-Bold.ttf", b"ttf"), ("fonts/Hello.otf", b"otf"), ("Install Fonts.ps1", b"copy"), ("README.txt", b"r")]),
    )
    .unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    assert_eq!(plan.files, vec!["panorama/fonts/Hello.otf".to_string(), "panorama/fonts/Inter-Bold.ttf".to_string()]);
    assert!(plan.warnings.join(" ").contains("Install Fonts.ps1"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_archive_with_nothing_installable_says_what_it_holds() {
    let root = scratch("shape-nothing");
    let game_root = fake_game(&root);
    type Case<'a> = (&'a str, Vec<(&'a str, &'a [u8])>, &'a [&'a str]);
    let cases: [Case; 5] = [
        ("gameinfo", vec![("piggy's config/gameinfo.gi", b"\"GameInfo\" {}"), ("piggy's config/video.txt", b"v")], &["gameinfo.gi", "Performance page"]),
        ("config", vec![("chat/cfg/chat/preset_1.cfg", b"bind"), ("chat/README.md", b"r")], &["config mod", "Autoexec"]),
        ("cursor", vec![("Cursors/cursor.bmp", b"BM"), ("Cursors/cursor_shop.bmp", b"BM")], &["cursor pack", "Locker > Cursors"]),
        ("source", vec![("materials/heroes/a.vmat", b"src"), ("materials/heroes/a.png", b"png")], &["uncompiled source", "Workshop Tools"]),
        ("installer", vec![("Edit gameinfo.gi(optional).bat", b"@echo")], &["program or script", "never runs"]),
    ];
    for (name, entries, needles) in cases {
        let archive = root.join(format!("{name}.zip"));
        fs::write(&archive, zip_of(&entries)).unwrap();
        let error = plan_of(&root, &game_root, &archive).unwrap_err().to_string();
        assert!(error.contains("No compiled _c files and no .vpk"), "{name}: {error}");
        for needle in needles {
            assert!(error.contains(needle), "{name}: {needle} missing from {error}");
        }
    }
    let empty = root.join("empty.zip");
    fs::write(&empty, zip_of(&[])).unwrap();
    assert!(plan_of(&root, &game_root, &empty).unwrap_err().to_string().contains("empty"));
    assert!(!addons_dir(&game_root).exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn damaged_password_and_wrong_files_get_clear_messages() {
    let root = scratch("shape-errors");
    let game_root = fake_game(&root);

    // A ZIP cut in half, as a dropped download leaves it.
    let full = zip_of(&[("pak01_dir.vpk", &skin_vpk()), ("pak02_dir.vpk", &skin_vpk())]);
    let cut = root.join("cut.zip");
    fs::write(&cut, &full[..full.len() / 2]).unwrap();
    let error = plan_of(&root, &game_root, &cut).unwrap_err().to_string();
    assert!(error.contains(DAMAGED_ARCHIVE_MARKER) && error.contains("Install again"), "{error}");

    // The general purpose flag says "encrypted".
    let mut locked = zip_of(&[("pak01_dir.vpk", &skin_vpk())]);
    locked[6] |= 1;
    let central = locked.windows(4).position(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
    locked[central + 8] |= 1;
    let locked_path = root.join("locked.zip");
    fs::write(&locked_path, &locked).unwrap();
    let error = plan_of(&root, &game_root, &locked_path).unwrap_err().to_string();
    assert!(error.contains("password protected"), "{error}");
    let error = plan_of(&root, &game_root, &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives/encrypted.7z"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("password protected") && error.contains("7-Zip"), "{error}");

    // An HTML error page saved under the archive's name.
    let page = root.join("mod.zip");
    fs::write(&page, b"<!DOCTYPE html><html><head><title>502</title></head></html>").unwrap();
    let error = plan_of(&root, &game_root, &page).unwrap_err().to_string();
    assert!(error.contains("web page"), "{error}");
    assert!(page.exists(), "a player's own file is never deleted");

    // A truncated VPK inside a good archive.
    let broken = root.join("broken.zip");
    fs::write(&broken, zip_of(&[("pak01_dir.vpk", &skin_vpk()[..20])])).unwrap();
    let error = plan_of(&root, &game_root, &broken).unwrap_err().to_string();
    assert!(error.contains("file list cannot be read"), "{error}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn script_mods_name_the_setting_that_allows_them() {
    let root = scratch("shape-script");
    let game_root = fake_game(&root);
    let archive = root.join("stamina.zip");
    let vpk = self_contained_vpk(&[("vdata_c", "scripts", "heroes", b"data")]);
    fs::write(&archive, zip_of(&[("pak05_dir.vpk", &vpk)])).unwrap();
    let error = plan_of(&root, &game_root, &archive).unwrap_err().to_string();
    assert!(error.contains("Allow script mods"), "{error}");
    crate::payload::set_allow_scripts(true);
    let allowed = plan_of(&root, &game_root, &archive);
    crate::payload::set_allow_scripts(false);
    assert!(allowed.is_ok(), "{allowed:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn source_files_inside_a_creators_vpk_are_a_warning() {
    let root = scratch("shape-vpk-sources");
    let game_root = fake_game(&root);
    let vpk = self_contained_vpk(&[
        ("vmat_c", "materials/a", "a", b"compiled"),
        ("vmat", "materials/a", "a", b"source"),
        ("soc", " ", "cache_123", b"tool cache"),
    ]);
    let archive = root.join("sources.zip");
    fs::write(&archive, zip_of(&[("pak01_dir.vpk", &vpk)])).unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    assert!(plan.warnings.join(" ").contains("source_file"), "{:?}", plan.warnings);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn odd_names_long_paths_and_read_only_trees_extract() {
    let root = scratch("shape-names");
    let game_root = fake_game(&root);
    let deep = format!("{}/materials/hud/a.vtex_c", ["A Very Long Folder Name With Spaces"; 6].join("/"));
    let mut bytes = zip_of(&[(deep.as_str(), b"tex"), ("QQ/readme.txt", b"gbk")]);
    // "QQ" becomes GBK bytes that are not UTF-8, as a Chinese Windows ZIP names a folder.
    for at in (0..bytes.len() - 1).rev() {
        if bytes[at] == b'Q' && bytes[at + 1] == b'Q' {
            bytes[at] = 0xba;
            bytes[at + 1] = 0xec;
        }
    }
    let archive = root.join("odd.zip");
    fs::write(&archive, &bytes).unwrap();
    let plan = plan_of(&root, &game_root, &archive).unwrap();
    assert_eq!(plan.files, vec!["materials/hud/a.vtex_c".to_string()]);

    // A read-only tree left by an earlier extract never blocks the next one.
    let locked = plan.extracted.join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("file"), b"x").unwrap();
    let mut permissions = fs::metadata(locked.join("file")).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(locked.join("file"), permissions.clone()).unwrap();
    fs::set_permissions(&locked, permissions).unwrap();
    let again = self::plan_of(&root, &game_root, &archive).unwrap();
    assert!(!again.extracted.join("locked").exists());
    let _ = fs::remove_dir_all(&root);
}

/// A deflated ZIP of one file of zeros: tiny on disk, `size` bytes unpacked.
fn deflated_zip(name: &str, size: usize) -> Vec<u8> {
    use std::io::Write as _;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer.start_file(name, options).unwrap();
    writer.write_all(&vec![0u8; size]).unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn nested_archives_share_one_size_budget() {
    let root = scratch("shape-nested-budget");
    let limits = ExtractLimits { max_total_bytes: 64 * 1024, ..ExtractLimits::default() };
    // Five inner archives of 20 KiB each: every one fits the 64 KiB cap alone, the five do not.
    let inner: Vec<(String, Vec<u8>)> =
        (0..5).map(|index| (format!("part{index}.zip"), deflated_zip(&format!("materials/{index}.vtex_c"), 20 * 1024))).collect();
    let entries: Vec<(&str, &[u8])> = inner.iter().map(|(name, data)| (name.as_str(), data.as_slice())).collect();
    let outer = root.join("bomb.zip");
    fs::write(&outer, zip_of(&entries)).unwrap();
    assert!(fs::metadata(&outer).unwrap().len() < 8 * 1024, "the outer archive itself is small");
    let out = root.join("out");
    let error = extract_archive(&outer, &out, &limits, &ToolPaths::default()).unwrap_err().to_string();
    assert!(error.contains("archives inside it unpack to more than"), "{error}");
    assert!(!out.exists(), "a refused extract leaves no files behind");

    // Under the budget, the same shape unpacks.
    let roomy = ExtractLimits { max_total_bytes: 256 * 1024, ..ExtractLimits::default() };
    let report = extract_archive(&outer, &out, &roomy, &ToolPaths::default()).unwrap();
    assert_eq!(report.files.len(), 5);

    // The entry budget is shared too: five archives of three files each against eight entries.
    let threes: Vec<(String, Vec<u8>)> = (0..5)
        .map(|index| {
            let files = [(format!("{index}/a"), b"a" as &[u8]), (format!("{index}/b"), b"b"), (format!("{index}/c"), b"c")];
            let refs: Vec<(&str, &[u8])> = files.iter().map(|(name, data)| (name.as_str(), *data)).collect();
            (format!("t{index}.zip"), zip_of(&refs))
        })
        .collect();
    let entries: Vec<(&str, &[u8])> = threes.iter().map(|(name, data)| (name.as_str(), data.as_slice())).collect();
    let wide = root.join("wide.zip");
    fs::write(&wide, zip_of(&entries)).unwrap();
    let few = ExtractLimits { max_entries: 8, ..ExtractLimits::default() };
    let error = extract_archive(&wide, &out, &few, &ToolPaths::default()).unwrap_err().to_string();
    assert!(error.contains("archives inside it unpack to more than"), "{error}");
    assert!(!out.exists());

    // And the number of inner archives is capped, however small they are.
    let many: Vec<(String, Vec<u8>)> = (0..=shapes::MAX_INNER_ARCHIVES)
        .map(|index| (format!("v{index}.zip"), deflated_zip("a.txt", 1)))
        .collect();
    let entries: Vec<(&str, &[u8])> = many.iter().map(|(name, data)| (name.as_str(), data.as_slice())).collect();
    let crowded = root.join("crowded.zip");
    fs::write(&crowded, zip_of(&entries)).unwrap();
    let error = extract_archive(&crowded, &out, &ExtractLimits::default(), &ToolPaths::default()).unwrap_err().to_string();
    assert!(error.contains("more than 64 archives"), "{error}");
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&root);
}
