use std::fs;
use std::path::{Path, PathBuf};

use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sigil-app-data-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn free(_: &Path) -> bool {
    true
}

#[test]
fn old_folder_is_renamed_once() {
    let base = temp("move");
    let old = base.join("DeadlockAgentHarness");
    fs::create_dir_all(old.join("backups")).unwrap();
    fs::write(old.join("settings.json"), "{}").unwrap();
    let (dir, outcome) = migrate_in(&base, &free);
    assert_eq!(dir, base.join("Sigil"));
    assert_eq!(outcome, Outcome::Moved { from: old.clone() });
    assert!(!old.exists());
    assert_eq!(fs::read_to_string(dir.join("settings.json")).unwrap(), "{}");
    assert!(dir.join("backups").is_dir());
    // The next start finds the new folder and nothing to do.
    assert_eq!(migrate_in(&base, &free), (dir, Outcome::Current));
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn the_linux_cli_name_moves_when_it_is_the_only_one() {
    let base = temp("linux");
    fs::create_dir_all(base.join("deadlock-agent-harness/workspace")).unwrap();
    let (dir, outcome) = migrate_in(&base, &free);
    assert!(matches!(outcome, Outcome::Moved { .. }));
    assert!(dir.join("workspace").is_dir());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn both_folders_keep_the_new_one_and_touch_nothing() {
    let base = temp("both");
    let old = base.join("DeadlockAgentHarness");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("settings.json"), "old").unwrap();
    fs::create_dir_all(base.join("Sigil")).unwrap();
    fs::write(base.join("Sigil/settings.json"), "new").unwrap();
    let (dir, outcome) = migrate_in(&base, &free);
    assert_eq!(dir, base.join("Sigil"));
    assert_eq!(outcome, Outcome::BothKept { old: old.clone(), first: true });
    assert!(outcome.log_line().is_some());
    assert_eq!(fs::read_to_string(old.join("settings.json")).unwrap(), "old");
    assert_eq!(fs::read_to_string(dir.join("settings.json")).unwrap(), "new");
    // Logged once: the second start stays quiet.
    let (_, again) = migrate_in(&base, &free);
    assert_eq!(again, Outcome::BothKept { old, first: false });
    assert!(again.log_line().is_none());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn neither_folder_is_a_fresh_start() {
    let base = temp("fresh");
    let (dir, outcome) = migrate_in(&base, &free);
    assert_eq!((dir, outcome), (base.join("Sigil"), Outcome::Fresh));
    // Nothing is created until the app writes.
    assert!(!base.join("Sigil").exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_busy_old_folder_is_used_as_it_is_and_left_whole() {
    let base = temp("busy");
    let old = base.join("DeadlockAgentHarness");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("settings.json"), "{}").unwrap();
    let (dir, outcome) = migrate_in(&base, &|_| false);
    assert_eq!(dir, old);
    assert!(matches!(outcome, Outcome::Deferred { .. }));
    assert!(outcome.log_line().unwrap().contains("trying again next start"));
    assert!(!base.join("Sigil").exists());
    assert!(old.join("settings.json").is_file());
    // Free next start: the move happens then.
    let (dir, outcome) = migrate_in(&base, &free);
    assert_eq!(dir, base.join("Sigil"));
    assert!(matches!(outcome, Outcome::Moved { .. }));
    let _ = fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_failed_rename_keeps_the_old_folder_and_copies_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let base = temp("denied");
    let old = base.join("DeadlockAgentHarness");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("settings.json"), "{}").unwrap();
    // A parent the process may not write to refuses the rename.
    fs::set_permissions(&base, fs::Permissions::from_mode(0o555)).unwrap();
    let (dir, outcome) = migrate_in(&base, &free);
    fs::set_permissions(&base, fs::Permissions::from_mode(0o755)).unwrap();
    if fs::metadata(&base).is_ok() && dir == base.join("Sigil") {
        // Running as root: permissions do not stop the rename. Nothing to check.
        let _ = fs::remove_dir_all(&base);
        return;
    }
    assert_eq!(dir, old);
    assert!(matches!(outcome, Outcome::Deferred { .. }));
    assert!(!base.join("Sigil").exists());
    assert!(old.join("settings.json").is_file());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_held_instance_lock_means_busy() {
    let base = temp("lock");
    let lock = base.join(INSTANCE_LOCK);
    assert!(lock_free(&lock), "a missing lock file is free");
    let held = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock).unwrap();
    assert!(lock_free(&lock), "an unlocked file is free");
    held.lock().unwrap();
    assert!(!lock_free(&lock));
    assert!(!wait_until_free(&base, false));
    held.unlock().unwrap();
    assert!(wait_until_free(&base, false));
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn rebase_maps_old_prefixes_and_leaves_others() {
    let data = Path::new("/home/p/AppData/Roaming/Sigil");
    assert_eq!(
        rebase_onto(Path::new("/home/p/AppData/Roaming/DeadlockAgentHarness/trash/1/pak03_dir.vpk"), data),
        data.join("trash/1/pak03_dir.vpk")
    );
    assert_eq!(
        rebase_onto(Path::new("/home/p/AppData/Roaming/deadlock-agent-harness/workspace"), data),
        data.join("workspace")
    );
    // Already under the folder in use, or outside any data folder: unchanged.
    let inside = data.join("vanilla-stash/addons/pak01_dir.vpk");
    assert_eq!(rebase_onto(&inside, data), inside);
    let game = Path::new("/games/Deadlock/game/citadel/addons/pak01_dir.vpk");
    assert_eq!(rebase_onto(game, data), game);
    // A deferred run (old folder in use) maps a new-name path back onto it.
    let old = Path::new("/home/p/AppData/Roaming/DeadlockAgentHarness");
    assert_eq!(rebase_onto(&data.join("x"), old), old.join("x"));
    // A sibling that only shares a prefix of the name is not inside.
    let sibling = Path::new("/home/p/AppData/Roaming/DeadlockAgentHarnessBackup/x");
    assert_eq!(rebase_onto(sibling, data), sibling);
}

#[test]
fn windows_paths_rebase_on_any_os() {
    // Pure path logic: the separators of the running OS do not matter as long as the stored
    // record and the data folder were built the same way, which is the case on the player's PC.
    let base = PathBuf::from("C:").join("Users").join("p").join("AppData").join("Roaming");
    let data = base.join("Sigil");
    let stored = base.join("DeadlockAgentHarness").join("vanilla-stash").join("pak01_dir.vpk");
    assert_eq!(rebase_onto(&stored, &data), data.join("vanilla-stash").join("pak01_dir.vpk"));
}

#[test]
fn undo_and_redo_for_a_rollback() {
    let base = temp("undo");
    let data = base.join("Sigil");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("settings.json"), "{}").unwrap();
    let old = undo_for_older_build(&data).expect("renamed back");
    assert_eq!(old, base.join("DeadlockAgentHarness"));
    assert!(old.join("settings.json").is_file());
    assert!(!data.exists());
    redo_after_failed_rollback(&old, &data);
    assert!(data.join("settings.json").is_file());
    // With an old folder already there, nothing moves.
    fs::create_dir_all(&old).unwrap();
    assert!(undo_for_older_build(&data).is_none());
    assert!(data.is_dir());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn picking_without_init_moves_nothing() {
    let base = temp("pick");
    assert_eq!(pick_in(&base), base.join("Sigil"));
    fs::create_dir_all(base.join("DeadlockAgentHarness")).unwrap();
    assert_eq!(pick_in(&base), base.join("DeadlockAgentHarness"));
    assert!(!base.join("Sigil").exists());
    fs::create_dir_all(base.join("Sigil")).unwrap();
    assert_eq!(pick_in(&base), base.join("Sigil"));
    let _ = fs::remove_dir_all(&base);
}

/// The records the real code writes into the old folder still lead to their files after the
/// move: Launch vanilla's stash restores, a trashed mod comes back with Undo, the host settings,
/// the download queue, a mod's picture and an install record's backups point into the new folder.
#[test]
fn records_written_under_the_old_name_work_after_the_move() {
    use crate::{download_queue, host, install, launch, mod_extras, trash};

    let base = temp("records");
    let game = base.join("Deadlock");
    let addons = crate::deploy::addons_dir(&game);
    fs::create_dir_all(&addons).unwrap();
    fs::write(addons.join("pak01_dir.vpk"), b"one").unwrap();
    fs::write(addons.join("pak02_dir.vpk"), b"two").unwrap();
    let old = base.join("DeadlockAgentHarness");
    fs::create_dir_all(&old).unwrap();

    // Delete slot 02 to the trash, then Launch vanilla's stash takes slot 01.
    trash::trash_slot(&game, &trash::trash_root(&old), 2, None, true).unwrap();
    let stash = launch::stash_vanilla(&game, &old, true).unwrap();
    assert!(stash.files[0].to.starts_with(&old));
    assert!(!addons.join("pak01_dir.vpk").exists());

    // Host settings with the default folders inside the data folder.
    let settings = host::HostSettings {
        server_dir: Some(old.join("host").join("server")),
        steamcmd_dir: Some(old.join("host").join("steamcmd")),
        ..Default::default()
    };
    host::save_settings(&old, &settings).unwrap();
    // A finished download waiting to be installed.
    let mut item = download_queue::Item::new("d1".into(), "https://gamebanana.com/mods/1".into());
    item.path = Some(old.join("workspace").join("downloads").join("1").join("mod.zip"));
    let queue = download_queue::Queue { items: vec![item], ..Default::default() };
    download_queue::store_queue(&old, &queue).unwrap();
    // A picture the player gave a mod.
    let picture = old.join("mod-images").join("slot01-u.png");
    fs::create_dir_all(picture.parent().unwrap()).unwrap();
    fs::write(&picture, b"png").unwrap();
    let extras_path = mod_extras::extras_path(&old);
    mod_extras::edit_extras(&extras_path, |e| {
        e.mods.push(mod_extras::ModExtra { uid: "u".into(), slot: 1, image: Some(picture.clone()), ..Default::default() });
    })
    .unwrap();
    // An install record whose backup sits in the workspace.
    let workspace = old.join("workspace");
    let backup = install::backups_dir(&workspace).join("2026-11-01").join("pak01_dir.vpk");
    fs::create_dir_all(backup.parent().unwrap()).unwrap();
    fs::write(&backup, b"before").unwrap();
    let record = serde_json::json!({ "backups": [backup], "target": addons.join("pak01_dir.vpk") });

    // The move.
    let (data, outcome) = migrate_in(&base, &free);
    assert!(matches!(outcome, Outcome::Moved { .. }));
    assert!(!old.exists());

    // Restore from the stash finds its files and puts slot 01 back.
    let status = launch::stash_status(&data).expect("stash still recorded");
    assert!(status.problem.is_none(), "{:?}", status.problem);
    assert_eq!((status.present, status.missing), (1, 0));
    let restored = launch::restore_vanilla(&data, true).unwrap();
    assert_eq!(restored.restored.len(), 1);
    assert_eq!(fs::read(addons.join("pak01_dir.vpk")).unwrap(), b"one");

    // Undo from the trash finds the entry and puts slot 02 back.
    let entries = trash::list_trash(&trash::trash_root(&data));
    assert_eq!(entries.len(), 1);
    trash::restore(&entries[0].dir, true).unwrap();
    assert_eq!(fs::read(addons.join("pak02_dir.vpk")).unwrap(), b"two");

    // The host settings point into the new folder.
    let loaded = host::load_settings(&data).unwrap();
    assert_eq!(loaded.server_dir, Some(data.join("host").join("server")));
    assert_eq!(loaded.steamcmd_dir, Some(data.join("host").join("steamcmd")));
    // So does the download waiting to be installed.
    let queue = download_queue::load_queue(&data);
    assert_eq!(queue.items[0].path, Some(data.join("workspace").join("downloads").join("1").join("mod.zip")));
    // The mod's picture is found.
    let extras = mod_extras::read_extras(&mod_extras::extras_path(&data)).unwrap();
    let image = extras.mods[0].image.clone().unwrap();
    assert!(image.is_file(), "{}", image.display());
    // The install record's backup is found; the game path is untouched.
    #[derive(serde::Deserialize)]
    struct Record {
        backups: Vec<PathBuf>,
        target: PathBuf,
    }
    let record: Record =
        parse_in_workspace(&record.to_string(), &data.join("workspace")).unwrap();
    assert!(record.backups[0].is_file());
    assert_eq!(record.target, addons.join("pak01_dir.vpk"));
    let _ = fs::remove_dir_all(&base);
}
