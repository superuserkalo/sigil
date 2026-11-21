//! Tests for compatibility repairs. File IO uses a per-test folder under the system temp
//! directory and is removed at the end.

use super::*;
use crate::vpk_write::{PackOptions, pack_vpk};
use std::io::Write;

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dah-compat-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Pack `files` into `<root>/<name>_dir.vpk`. The writer puts data in a `_000.vpk` chunk beside
/// it, so these archives are split; callers that need a single file use [`single_file_vpk`].
fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let staging = root.join(format!("staging-{name}"));
    for (path, data) in files {
        let target = staging.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, data).unwrap();
    }
    pack_vpk(&PackOptions::new(staging, root.join("out"), name))
        .unwrap()
        .dir_path
}

/// Write a version 2 VPK with every entry stored in the directory file itself and no checksum
/// section, the layout a hand-built or tool-packed mod can have.
fn single_file_vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let mut groups: BTreeMap<(String, String), Vec<(&str, &[u8])>> = BTreeMap::new();
    for (path, data) in files {
        let (directory, file_name) = path.rsplit_once('/').unwrap_or(("", path));
        let (stem, extension) = file_name.rsplit_once('.').unwrap_or((file_name, " "));
        groups
            .entry((extension.to_string(), directory.to_string()))
            .or_default()
            .push((stem, *data));
    }

    let mut tree = Vec::new();
    let mut data = Vec::new();
    for ((extension, directory), entries) in groups {
        tree.extend_from_slice(extension.as_bytes());
        tree.push(0);
        let directory = if directory.is_empty() {
            " "
        } else {
            directory.as_str()
        };
        tree.extend_from_slice(directory.as_bytes());
        tree.push(0);
        for (stem, bytes) in entries {
            tree.extend_from_slice(stem.as_bytes());
            tree.push(0);
            tree.extend_from_slice(&crate::vpk::crc32(bytes).to_le_bytes());
            tree.extend_from_slice(&0_u16.to_le_bytes());
            tree.extend_from_slice(&crate::vpk::VPK_DIRECTORY_ARCHIVE.to_le_bytes());
            tree.extend_from_slice(&(data.len() as u32).to_le_bytes());
            tree.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            tree.extend_from_slice(&0xffff_u16.to_le_bytes());
            data.extend_from_slice(bytes);
        }
        tree.push(0);
        tree.push(0);
    }
    tree.push(0);

    let mut output = Vec::new();
    output.extend_from_slice(&crate::vpk::VPK_MAGIC.to_le_bytes());
    output.extend_from_slice(&2_u32.to_le_bytes());
    output.extend_from_slice(&(tree.len() as u32).to_le_bytes());
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&tree);
    output.extend_from_slice(&data);

    let path = root.join(format!("{name}_dir.vpk"));
    fs::write(&path, output).unwrap();
    path
}

#[test]
fn detects_animation_overrides_from_compiled_names() {
    let root = root("animations");
    let plain = vpk(
        &root,
        "plain",
        &[
            ("models/heroes/atlas/atlas.vmdl_c", b"model"),
            ("materials/heroes/atlas/atlas_body.vmat_c", b"material"),
            ("anims/heroes/atlas/atlas_root.vanim", b"source only"),
        ],
    );
    let animated = vpk(
        &root,
        "animated",
        &[(
            "anims/heroes/atlas/atlas_root.vnmskel_c",
            b"compiled skeleton",
        )],
    );
    let uppercase = vpk(
        &root,
        "uppercase",
        &[(
            "ANIMS/HEROES/ATLAS/ATLAS_RUN.VNMSKEL_C",
            b"compiled skeleton",
        )],
    );

    assert!(!has_animation_overrides(&[]));
    assert!(!has_animation_overrides(&[plain.clone()]));
    assert!(has_animation_overrides(&[plain, animated]));
    assert!(
        has_animation_overrides(&[uppercase]),
        "compiled animation names match without regard to case"
    );
    assert!(has_animation_overrides(&[root.join("missing_dir.vpk")]));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn split_archives_are_combined_before_a_repair() {
    let root = root("split");
    let base = single_file_vpk(&root, "pak01", &[("materials/rock.vmat_c", b"base")]);
    let mod_vpk = vpk(&root, "pak03", &[("materials/rock.vmat_c", b"mod")]);
    assert!(crate::vpk_write::is_split(&mod_vpk).unwrap());
    let staging = root.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let out = staging.join("staged_dir.vpk");

    let findings = check(&base, &mod_vpk, RepairKind::Data, false).unwrap();
    assert!(findings.iter().all(|f| !f.repairable), "nothing here is a repairable resource");

    let applied = repair_to(&base, &mod_vpk, &out, RepairKind::Cameras, false).unwrap();
    assert!(applied.is_empty());
    assert!(!out.exists());
    assert_eq!(
        fs::read_dir(&staging).unwrap().count(),
        0,
        "the combined copy is removed after the run"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn missing_files_are_errors_not_panics() {
    let root = root("missing");
    let base = single_file_vpk(&root, "pak01", &[("scripts/heroes.vdata_c", b"base")]);
    let mod_vpk = single_file_vpk(&root, "pak03", &[("scripts/heroes.vdata_c", b"mod")]);
    let out = root.join("staged_dir.vpk");
    let missing = root.join("nope_dir.vpk");

    assert!(check(&missing, &mod_vpk, RepairKind::Data, false).is_err());
    assert!(check(&base, &missing, RepairKind::Cameras, false).is_err());
    assert!(repair_to(&missing, &mod_vpk, &out, RepairKind::Data, false).is_err());
    assert!(repair_to(&base, &missing, &out, RepairKind::Data, false).is_err());
    assert!(!out.exists());

    fs::write(&out, b"already staged").unwrap();
    let error = repair_to(&base, &mod_vpk, &out, RepairKind::Data, false).unwrap_err();
    assert!(error.to_string().contains("already exists"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn verification_rejects_a_changed_entry_set() {
    let root = root("entry-set");
    let before = single_file_vpk(
        &root,
        "before",
        &[("scripts/a.vdata_c", b"a"), ("scripts/b.vdata_c", b"b")],
    );
    let after = single_file_vpk(
        &root,
        "after",
        &[
            ("scripts/a.vdata_c", b"a"),
            ("scripts/b.vdata_c", b"b"),
            ("scripts/c.vdata_c", b"c"),
        ],
    );

    let error = verify_output(&before, &after, &BTreeSet::new()).unwrap_err();
    assert!(
        error.to_string().contains("entry set"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn verification_only_allows_the_reported_entries_to_change() {
    let root = root("changes");
    let before = single_file_vpk(
        &root,
        "before",
        &[("scripts/a.vdata_c", b"a"), ("scripts/b.vdata_c", b"b")],
    );

    let reported = BTreeSet::from(["scripts/a.vdata_c".to_string()]);
    let edited = single_file_vpk(
        &root,
        "edited",
        &[
            ("scripts/a.vdata_c", b"a repaired value"),
            ("scripts/b.vdata_c", b"b"),
        ],
    );
    verify_output(&before, &edited, &reported).unwrap();

    let unreported = single_file_vpk(
        &root,
        "unreported",
        &[
            ("scripts/a.vdata_c", b"a"),
            ("scripts/b.vdata_c", b"b repaired value"),
        ],
    );
    let error = verify_output(&before, &unreported, &reported).unwrap_err();
    assert!(
        error.to_string().contains("no repair reported"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_changed_game_pak01_fails_the_run() {
    let root = root("base-stamp");
    let base = vpk(&root, "pak01", &[("scripts/heroes.vdata_c", b"base")]);
    let stamp = file_stamp(&base).unwrap();
    ensure_unchanged(&base, stamp).unwrap();

    let mut file = fs::OpenOptions::new().append(true).open(&base).unwrap();
    file.write_all(b"update").unwrap();
    file.flush().unwrap();

    let error = ensure_unchanged(&base, stamp).unwrap_err();
    assert!(error.to_string().contains("changed during the repair"));

    let _ = fs::remove_dir_all(&root);
}

/// Needs an installed game and a single-file mod without chunk siblings:
/// `DEADLOCK_PAK01=.../game/citadel/pak01_dir.vpk DEADLOCK_TEST_MOD=.../citadel/addons/pak03_dir.vpk`.
#[test]
#[ignore = "needs DEADLOCK_PAK01 and DEADLOCK_TEST_MOD"]
fn repairs_a_real_game_mod() {
    let base = PathBuf::from(
        std::env::var_os("DEADLOCK_PAK01").expect("set DEADLOCK_PAK01 to pak01_dir.vpk"),
    );
    let mod_vpk =
        PathBuf::from(std::env::var_os("DEADLOCK_TEST_MOD").expect("set DEADLOCK_TEST_MOD"));

    let findings = check(&base, &mod_vpk, RepairKind::Data, false).unwrap();
    for finding in &findings {
        assert!(!finding.entry.is_empty());
        if !finding.repairable {
            assert!(
                !finding.detail.is_empty(),
                "{} has no reason",
                finding.entry
            );
        }
    }

    let out = std::env::temp_dir().join(format!("sigil-compat-{}-stage.vpk", std::process::id()));
    let _ = fs::remove_file(&out);
    let applied = repair_to(&base, &mod_vpk, &out, RepairKind::Data, false).unwrap();

    let expected: Vec<RepairFinding> = findings
        .into_iter()
        .filter(|finding| finding.repairable)
        .collect();
    assert_eq!(applied, expected);
    assert_eq!(out.exists(), !applied.is_empty());

    let _ = fs::remove_file(&out);
}

#[test]
fn apply_needs_approval_and_leaves_a_clean_mod_alone() {
    let root = root("apply");
    let base = single_file_vpk(&root, "pak01", &[("materials/rock.vmat_c", b"base")]);
    let addons = root.join("addons");
    fs::create_dir_all(&addons).unwrap();
    let made = single_file_vpk(&root, "pak03", &[("materials/rock.vmat_c", b"mod")]);
    let mod_vpk = addons.join("pak03_dir.vpk");
    fs::rename(&made, &mod_vpk).unwrap();
    let before = fs::read(&mod_vpk).unwrap();

    let refused = apply_repair(&base, &mod_vpk, &[RepairKind::Data], false, &root.join("backups"), &root.join("staging"), false);
    assert!(refused.unwrap_err().to_string().contains("not approved"));

    let applied = apply_repair(
        &base,
        &mod_vpk,
        &[RepairKind::Data, RepairKind::Cameras],
        false,
        &root.join("backups"),
        &root.join("staging"),
        true,
    )
    .unwrap();
    assert!(applied.is_none(), "nothing in this mod needs a repair");
    assert_eq!(fs::read(&mod_vpk).unwrap(), before);
    assert_eq!(fs::read_dir(addons).unwrap().count(), 1, "no stray file in the addons folder");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn revert_puts_the_backup_back_only_over_the_repaired_file() {
    let root = root("revert");
    let mod_vpk = root.join("pak03_dir.vpk");
    let backup = root.join("backup.vpk");
    fs::write(&backup, b"original").unwrap();
    fs::write(&mod_vpk, b"repaired").unwrap();
    let repaired = crate::identify::sha256_file(&mod_vpk).unwrap();

    assert!(revert_repair(&backup, &mod_vpk, &repaired, false).is_err());
    fs::write(&mod_vpk, b"updated since").unwrap();
    let error = revert_repair(&backup, &mod_vpk, &repaired, true).unwrap_err();
    assert!(error.to_string().contains("changed since"), "{error}");
    assert_eq!(fs::read(&mod_vpk).unwrap(), b"updated since");

    fs::write(&mod_vpk, b"repaired").unwrap();
    revert_repair(&backup, &mod_vpk, &repaired, true).unwrap();
    assert_eq!(fs::read(&mod_vpk).unwrap(), b"original");
    let _ = fs::remove_dir_all(&root);
}

const TEXT_ENTRY: &str = "resource/localization/citadel_heroes/citadel_heroes_english.txt";

fn text_table(rows: &[(&str, &str)]) -> Vec<u8> {
    let mut text = String::from("\"lang\"\n{\n\t\"Tokens\"\n\t{\n");
    for (key, value) in rows {
        text.push_str(&format!("\t\t\"{key}\"\t\t\"{value}\"\n"));
    }
    text.push_str("\t}\n}\n");
    text.into_bytes()
}

#[test]
fn a_complete_text_table_opens_and_a_damaged_one_says_why() {
    assert!(strict_text_table(&text_table(&[("Hero_Atlas", "Atlas")])).is_ok());
    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(
            "\"lang\" { \"Tokens\" { \"Hero_Atlas\" \"Atlas\" } }"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    assert!(strict_text_table(&utf16).is_ok());
    let bare = b"lang { Tokens { \"Hero_Atlas\" \"Atlas\" } }".to_vec();
    assert!(strict_text_table(&bare).unwrap_err().contains("unquoted"));

    let unclosed = b"\"lang\" { \"Tokens\" { \"Hero_Atlas\" \"Atlas\" }".to_vec();
    assert!(strict_text_table(&unclosed).unwrap_err().contains("unclosed"));
    let duplicate = text_table(&[("Hero_Atlas", "Atlas"), ("hero_atlas:0", "Dupe")]);
    assert!(strict_text_table(&duplicate).unwrap_err().contains("twice"));
    let no_tokens = b"\"lang\" { \"Other\" \"x\" \"More\" \"y\" }".to_vec();
    assert!(strict_text_table(&no_tokens).unwrap_err().contains("Tokens block"));
    assert!(strict_text_table(b"\xff\xfe\x00").is_err());
}

#[test]
fn a_damaged_game_text_file_stops_a_repair_and_names_the_file() {
    let root = root("game-text");
    let citadel = root.join("citadel");
    fs::create_dir_all(&citadel).unwrap();
    let base = single_file_vpk(&citadel, "pak01", &[(TEXT_ENTRY, text_table(&[("Hero_Atlas", "Atlas")]).as_slice())]);
    let mod_vpk = single_file_vpk(&root, "pak03", &[(TEXT_ENTRY, text_table(&[("Hero_Atlas", "Old Atlas")]).as_slice())]);

    assert_eq!(game_text_problem(&base, &mod_vpk).unwrap(), None, "a complete table is usable");

    // A patch or verify wrote half the game file: the repair must not merge from it.
    let loose = citadel.join(TEXT_ENTRY);
    fs::create_dir_all(loose.parent().unwrap()).unwrap();
    fs::write(&loose, b"\"lang\" { \"Tokens\" { \"Hero_Atlas\" \"Atlas\"").unwrap();
    let problem = game_text_problem(&base, &mod_vpk).unwrap().expect("the damaged loose file is found");
    assert_eq!(problem.entry, TEXT_ENTRY);
    assert!(problem.detail.contains("never closes") || problem.detail.contains("unclosed"), "{problem:?}");

    let error = check(&base, &mod_vpk, RepairKind::Data, false).unwrap_err().to_string();
    assert!(error.contains(TEXT_ENTRY), "{error}");
    assert!(error.contains("Verify integrity"), "{error}");
    assert!(error.contains("Nothing was repaired"), "{error}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_mod_with_its_own_text_is_not_blocked_by_the_game_lacking_it() {
    let root = root("game-text-own");
    let base = single_file_vpk(&root, "pak01", &[("materials/rock.vmat_c", b"base")]);
    let mod_vpk = single_file_vpk(&root, "pak04", &[(TEXT_ENTRY, text_table(&[("Sigil_Test", "Hi")]).as_slice())]);
    assert_eq!(game_text_problem(&base, &mod_vpk).unwrap(), None);
    let _ = fs::remove_dir_all(&root);
}
