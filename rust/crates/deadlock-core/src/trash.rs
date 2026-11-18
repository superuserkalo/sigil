//! Sigil's trash for deleted addon slots, and the undo paths that use it.
//!
//! Deleting a mod never removes its files outright. [`trash_slot`] moves every file of one
//! addon slot (`pakNN_dir.vpk`, `pakNN_dir.vpk.disabled`, `pakNN_MMM.vpk`) out of
//! `game/citadel/addons` into a folder of its own under Sigil's app data, with a small
//! manifest, so [`restore`] can put it back. [`empty_trash`] is the only call that deletes,
//! and it deletes only trash entries, never the game install.
//!
//! The only input that names game files is a slot number. File names come from reading the
//! addon folder and must match the slot patterns; a symlink or folder with a slot name is
//! refused. A move is a rename, or a copy then a delete when the trash sits on another drive
//! than the game.
//!
//! [`undo_update`] puts back the files an update replaced: the updated files go to the trash
//! first, then the install backups are copied into the slot.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::import::ModMeta;
use crate::load_order::{check_plain_name, read_slot_files, slot_of};
use crate::util::now_iso;

pub const TRASH_DIR_NAME: &str = "trash";
pub const MANIFEST_NAME: &str = "trashed.json";
pub const TRASH_SCHEMA: u32 = 1;

/// What one trash entry holds and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashManifest {
    pub schema: u32,
    pub slot: u16,
    /// The addon folder the files came from, and go back to.
    pub addons_dir: PathBuf,
    /// File names as they were in the addon folder.
    pub files: Vec<String>,
    pub bytes: u64,
    pub trashed_at: String,
    /// Whether the slot was switched on.
    pub enabled: bool,
    /// The library entry the slot had, so a restore can bring its name and preview back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ModMeta>,
}

/// One trashed slot on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// The entry folder's name.
    pub id: String,
    pub dir: PathBuf,
    pub manifest: TrashManifest,
}

/// What [`restore`] put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub slot: u16,
    pub files: Vec<String>,
    pub meta: Option<ModMeta>,
}

/// What [`empty_trash`] removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmptyReport {
    pub entries: usize,
    pub bytes: u64,
}

/// `<data_dir>/trash`.
pub fn trash_root(data_dir: &Path) -> PathBuf {
    data_dir.join(TRASH_DIR_NAME)
}

/// The files of one slot in the game's addon folder, sorted. Read only.
pub fn slot_files(game_root: &Path, slot: u16) -> Result<Vec<String>> {
    let mut files = read_slot_files(&addons_dir(game_root))?;
    Ok(files.remove(&slot).unwrap_or_default())
}

/// Move every file of `slot` from the game's addon folder into a new trash entry. Game
/// install write; refuses without `approve`, for a slot outside 01 to 99, and for a slot
/// with no files. When a move fails partway, the files already moved go back.
pub fn trash_slot(
    game_root: &Path,
    trash_root: &Path,
    slot: u16,
    meta: Option<ModMeta>,
    approve: bool,
) -> Result<TrashEntry> {
    if !crate::addons::is_slot(slot) {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to {}. Nothing was deleted.",
            crate::addons::MAX_SLOT
        )));
    }
    if !approve {
        return Err(Error::msg(format!(
            "Not approved: delete slot {slot:02}. Nothing was deleted."
        )));
    }
    let addons = addons_dir(game_root);
    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was deleted.",
            addons.display()
        )));
    }
    let files = slot_files(game_root, slot)?;
    if files.is_empty() {
        return Err(Error::msg(format!(
            "Slot {slot:02} has no files in {}. Nothing was deleted.",
            addons.display()
        )));
    }
    let folder = crate::addons::slot_folder(&addons, slot);
    for name in &files {
        let regular = fs::symlink_metadata(folder.join(name))
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        if !regular || slot_of(name) != Some(crate::addons::local_slot(slot)) {
            return Err(Error::msg(format!(
                "{name} is not a slot file of {slot:02}. Nothing was deleted."
            )));
        }
    }
    let enabled = files.iter().any(|name| name.eq_ignore_ascii_case(&crate::addons::slot_name(slot, "_dir.vpk")));
    let bytes = files
        .iter()
        .map(|name| fs::metadata(folder.join(name)).map(|m| m.len()).unwrap_or(0))
        .sum();

    make_entry(&addons, trash_root, slot, files, meta, enabled, bytes)
}

/// Move a named set of one slot's files to the trash. Game install write; refuses without
/// `approve`, for a slot outside 01 to 99, for a name that is not a plain file of that slot and
/// for a file that is gone. Used to take a merged mod apart: only the merged file and its
/// chunks leave the addon folder.
pub fn trash_named(
    game_root: &Path,
    trash_root: &Path,
    slot: u16,
    names: &[String],
    approve: bool,
) -> Result<TrashEntry> {
    if !crate::addons::is_slot(slot) {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to {}. Nothing was deleted.",
            crate::addons::MAX_SLOT
        )));
    }
    if !approve {
        return Err(Error::msg(format!(
            "Not approved: delete slot {slot:02}. Nothing was deleted."
        )));
    }
    let addons = addons_dir(game_root);
    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was deleted.",
            addons.display()
        )));
    }
    if names.is_empty() {
        return Err(Error::msg(format!(
            "Slot {slot:02} has no files in {}. Nothing was deleted.",
            addons.display()
        )));
    }
    let mut files: Vec<String> = names.to_vec();
    files.sort();
    files.dedup();
    let folder = crate::addons::slot_folder(&addons, slot);
    for name in &files {
        check_plain_name(name)?;
        let regular = fs::symlink_metadata(folder.join(name))
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        if !regular || slot_of(name) != Some(crate::addons::local_slot(slot)) {
            return Err(Error::msg(format!(
                "{name} is not a file of slot {slot:02}. Nothing was deleted."
            )));
        }
    }
    let enabled = files
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&crate::addons::slot_name(slot, "_dir.vpk")));
    let bytes = files
        .iter()
        .map(|name| fs::metadata(folder.join(name)).map(|m| m.len()).unwrap_or(0))
        .sum();
    make_entry(&addons, trash_root, slot, files, None, enabled, bytes)
}

/// The manifest, the folder and the moves, shared by [`trash_slot`] and [`trash_named`].
fn make_entry(
    addons: &Path,
    trash_root: &Path,
    slot: u16,
    files: Vec<String>,
    meta: Option<ModMeta>,
    enabled: bool,
    bytes: u64,
) -> Result<TrashEntry> {
    let id = unique_id(trash_root, slot)?;
    let dir = trash_root.join(&id);
    fs::create_dir_all(&dir)?;
    let manifest = TrashManifest {
        schema: TRASH_SCHEMA,
        slot,
        addons_dir: addons.to_path_buf(),
        files: files.clone(),
        bytes,
        trashed_at: now_iso(),
        enabled,
        meta,
    };
    // The manifest goes first, so an interrupted move still leaves an entry that names
    // where its files belong.
    write_manifest(&dir, &manifest)?;

    let folder = crate::addons::slot_folder(addons, slot);
    let mut moved: Vec<&String> = Vec::new();
    for name in &files {
        if let Err(error) = move_file(&folder.join(name), &dir.join(name)) {
            let mut stuck = Vec::new();
            for done in moved.iter().rev() {
                if move_file(&dir.join(done), &folder.join(done)).is_err() {
                    stuck.push(done.as_str());
                }
            }
            if stuck.is_empty() {
                let _ = fs::remove_dir_all(&dir);
                return Err(Error::msg(format!(
                    "Moving {name} to the trash failed ({error}). Nothing was deleted."
                )));
            }
            return Err(Error::msg(format!(
                "Moving {name} to the trash failed ({error}). {} stayed in the trash at {}.",
                stuck.join(", "),
                dir.display()
            )));
        }
        moved.push(name);
    }

    Ok(TrashEntry { id, dir, manifest })
}

/// [`trash_slot`] for several slots. Each slot is its own entry; a failure stops nothing
/// else. Returns the entries made and the errors, by slot.
pub fn trash_slots(
    game_root: &Path,
    trash_root: &Path,
    slots: &[(u16, Option<ModMeta>)],
    approve: bool,
) -> (Vec<TrashEntry>, Vec<(u16, String)>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for (slot, meta) in slots {
        match trash_slot(game_root, trash_root, *slot, meta.clone(), approve) {
            Ok(entry) => entries.push(entry),
            Err(error) => errors.push((*slot, error.to_string())),
        }
    }
    (entries, errors)
}

/// Put a trash entry back into the addon folder it came from, then remove the entry. Game
/// install write; refuses without `approve`, when the folder is no longer an addon folder,
/// when a manifest name is not a file of its slot, and when any of the names is taken again
/// (another mod went into the slot): nothing is overwritten.
pub fn restore(entry_dir: &Path, approve: bool) -> Result<RestoreReport> {
    if !approve {
        return Err(Error::msg("Not approved: restore from the trash. Nothing was moved."));
    }
    let manifest = read_manifest(entry_dir)?;
    let addons = &manifest.addons_dir;
    let looks_right = addons.ends_with(Path::new("citadel").join("addons"));
    if !looks_right || !addons.is_dir() {
        return Err(Error::msg(format!(
            "{} is not an addon folder any more. Nothing was moved.",
            addons.display()
        )));
    }
    for name in &manifest.files {
        check_plain_name(name)?;
        if slot_of(name) != Some(crate::addons::local_slot(manifest.slot)) {
            return Err(Error::msg(format!(
                "{name} is not a file of slot {:02}. Nothing was moved.",
                manifest.slot
            )));
        }
        if !entry_dir.join(name).is_file() {
            return Err(Error::msg(format!(
                "{name} is missing from the trash entry. Nothing was moved."
            )));
        }
    }
    let taken = slot_files_in(addons, manifest.slot)?;
    if !taken.is_empty() {
        return Err(Error::msg(format!(
            "Slot {:02} is in use again ({}). Move that mod first. Nothing was moved.",
            manifest.slot,
            taken.join(", ")
        )));
    }

    let folder = crate::addons::slot_folder(addons, manifest.slot);
    fs::create_dir_all(&folder)?;
    let mut moved: Vec<&String> = Vec::new();
    for name in &manifest.files {
        if let Err(error) = move_file(&entry_dir.join(name), &folder.join(name)) {
            for done in moved.iter().rev() {
                let _ = move_file(&folder.join(done), &entry_dir.join(done));
            }
            return Err(Error::msg(format!(
                "Moving {name} back failed ({error}). The entry stays in the trash."
            )));
        }
        moved.push(name);
    }
    let _ = fs::remove_dir_all(entry_dir);
    Ok(RestoreReport {
        slot: manifest.slot,
        files: manifest.files.clone(),
        meta: manifest.meta.clone(),
    })
}

/// Every entry in the trash, newest first. Folders without a readable manifest are skipped.
pub fn list_trash(trash_root: &Path) -> Vec<TrashEntry> {
    let Ok(read) = fs::read_dir(trash_root) else {
        return Vec::new();
    };
    let mut entries: Vec<TrashEntry> = read
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| {
            let dir = entry.path();
            let manifest = read_manifest(&dir).ok()?;
            Some(TrashEntry {
                id: entry.file_name().to_string_lossy().into_owned(),
                dir,
                manifest,
            })
        })
        .collect();
    entries.sort_by(|a, b| b.manifest.trashed_at.cmp(&a.manifest.trashed_at).then(b.id.cmp(&a.id)));
    entries
}

/// Delete one trash entry for good. App data only: the folder must sit directly in
/// `trash_root` and hold a manifest.
pub fn purge_entry(trash_root: &Path, entry_dir: &Path) -> Result<u64> {
    if entry_dir.parent() != Some(trash_root) || read_manifest(entry_dir).is_err() {
        return Err(Error::msg(format!(
            "{} is not a trash entry. Nothing was deleted.",
            entry_dir.display()
        )));
    }
    let bytes = read_manifest(entry_dir).map(|m| m.bytes).unwrap_or(0);
    fs::remove_dir_all(entry_dir)?;
    Ok(bytes)
}

/// Delete every trash entry for good. Refuses without `approve`. Folders in the trash
/// without a manifest are left alone.
pub fn empty_trash(trash_root: &Path, approve: bool) -> Result<EmptyReport> {
    if !approve {
        return Err(Error::msg("Not approved: empty the trash. Nothing was deleted."));
    }
    let mut report = EmptyReport::default();
    for entry in list_trash(trash_root) {
        report.bytes += purge_entry(trash_root, &entry.dir)?;
        report.entries += 1;
    }
    Ok(report)
}

/// Put back what an update replaced. Game install write; refuses without `approve`.
///
/// `backups` are the install backups of the slot (copies of its files from before the
/// update, named as they were in the addon folder). The slot's current files go to the trash
/// first, as [`trash_slot`] would, then each backup is copied into the slot. A backup whose
/// name is not a file of `slot` is refused before anything moves. The backups stay where
/// they are.
pub fn undo_update(
    game_root: &Path,
    trash_root: &Path,
    slot: u16,
    backups: &[PathBuf],
    approve: bool,
) -> Result<Option<TrashEntry>> {
    if !approve {
        return Err(Error::msg(format!(
            "Not approved: put back the old files of slot {slot:02}. Nothing was changed."
        )));
    }
    if backups.is_empty() {
        return Err(Error::msg(format!(
            "There is no backup of slot {slot:02} to put back. Nothing was changed."
        )));
    }
    let mut names = Vec::new();
    for backup in backups {
        let name = backup
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if slot_of(&name) != Some(crate::addons::local_slot(slot)) || !backup.is_file() {
            return Err(Error::msg(format!(
                "{} is not a backup of slot {slot:02}. Nothing was changed.",
                backup.display()
            )));
        }
        names.push(name);
    }
    let addons = addons_dir(game_root);
    let trashed = if slot_files(game_root, slot)?.is_empty() {
        None
    } else {
        Some(trash_slot(game_root, trash_root, slot, None, true)?)
    };
    let mut copied: Vec<PathBuf> = Vec::new();
    let folder = crate::addons::slot_folder(&addons, slot);
    for (backup, name) in backups.iter().zip(names.iter()) {
        let target = folder.join(name);
        let result = if fs::symlink_metadata(&target).is_ok() {
            Err(Error::msg(format!("{name} appeared in the slot")))
        } else {
            copy_then_check(backup, &target)
        };
        if let Err(error) = result {
            for done in &copied {
                let _ = fs::remove_file(done);
            }
            if let Some(entry) = &trashed {
                let _ = restore(&entry.dir, true);
            }
            return Err(Error::msg(format!(
                "Copying the backup {name} failed ({error}). The updated files were put back."
            )));
        }
        copied.push(target);
    }
    Ok(trashed)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn slot_files_in(addons: &Path, slot: u16) -> Result<Vec<String>> {
    let mut files = read_slot_files(addons)?;
    Ok(files.remove(&slot).unwrap_or_default())
}

fn unique_id(trash_root: &Path, slot: u16) -> Result<String> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f");
    for n in 0..1000 {
        let id = if n == 0 {
            format!("{stamp}-pak{slot:02}")
        } else {
            format!("{stamp}-pak{slot:02}-{n}")
        };
        if !trash_root.join(&id).exists() {
            return Ok(id);
        }
    }
    Err(Error::msg("Could not name a new trash entry. Nothing was deleted."))
}

fn write_manifest(dir: &Path, manifest: &TrashManifest) -> Result<()> {
    let text = serde_json::to_string_pretty(manifest)
        .map_err(|error| Error::msg(format!("Could not encode the trash manifest: {error}")))?;
    crate::util::write_atomic(&dir.join(MANIFEST_NAME), text.as_bytes())
}

fn read_manifest(dir: &Path) -> Result<TrashManifest> {
    let path = dir.join(MANIFEST_NAME);
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text)
        .map_err(|error| Error::msg(format!("{} is not a trash manifest: {error}", path.display())))
}

/// Rename, or copy then delete when the two paths sit on different drives. Never replaces a
/// file.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(Error::msg(format!("{} already exists", to.display())));
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_then_check(from, to)?;
            if let Err(error) = fs::remove_file(from) {
                let _ = fs::remove_file(to);
                return Err(error.into());
            }
            Ok(())
        }
    }
}

/// Copy and compare sizes; a short copy is removed.
fn copy_then_check(from: &Path, to: &Path) -> Result<()> {
    let expected = fs::metadata(from)?.len();
    let copied = match fs::copy(from, to) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(to);
            return Err(error.into());
        }
    };
    if copied != expected {
        let _ = fs::remove_file(to);
        return Err(Error::msg(format!(
            "Copy of {} was short ({copied} of {expected} bytes)",
            from.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        game: PathBuf,
        addons: PathBuf,
        trash: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/trash-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let game = root.join("Deadlock");
        let addons = game.join("game").join("citadel").join("addons");
        fs::create_dir_all(&addons).unwrap();
        Fixture {
            game,
            addons,
            trash: root.join("data").join(TRASH_DIR_NAME),
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn trash_moves_only_the_slot_files_and_restore_puts_them_back() {
        let f = fixture("roundtrip");
        fs::write(f.addons.join("pak03_dir.vpk"), "dir").unwrap();
        fs::write(f.addons.join("pak03_000.vpk"), "chunk").unwrap();
        fs::write(f.addons.join("pak04_dir.vpk"), "other").unwrap();
        fs::write(f.addons.join("pak03_dir.vpk.bak"), "not a slot file").unwrap();

        let meta = ModMeta {
            slot: 3,
            name: "Three".to_string(),
            ..Default::default()
        };
        let entry = trash_slot(&f.game, &f.trash, 3, Some(meta.clone()), true).unwrap();
        assert_eq!(names(&f.addons), vec!["pak03_dir.vpk.bak", "pak04_dir.vpk"]);
        assert_eq!(entry.manifest.files, vec!["pak03_000.vpk", "pak03_dir.vpk"]);
        assert!(entry.manifest.enabled);
        assert_eq!(entry.manifest.bytes, 8);
        assert_eq!(list_trash(&f.trash).len(), 1);

        let report = restore(&entry.dir, true).unwrap();
        assert_eq!(report.slot, 3);
        assert_eq!(report.meta, Some(meta));
        assert_eq!(
            names(&f.addons),
            vec!["pak03_000.vpk", "pak03_dir.vpk", "pak03_dir.vpk.bak", "pak04_dir.vpk"]
        );
        assert_eq!(fs::read_to_string(f.addons.join("pak03_000.vpk")).unwrap(), "chunk");
        assert!(list_trash(&f.trash).is_empty());
    }

    #[test]
    fn trash_refuses_without_approval_bad_slots_and_empty_slots() {
        let f = fixture("refuse");
        fs::write(f.addons.join("pak01_dir.vpk.disabled"), "off").unwrap();
        assert!(trash_slot(&f.game, &f.trash, 1, None, false).is_err());
        assert!(trash_slot(&f.game, &f.trash, 0, None, true).is_err());
        assert!(trash_slot(&f.game, &f.trash, 100, None, true).is_err());
        assert!(trash_slot(&f.game, &f.trash, 2, None, true).is_err());
        assert_eq!(names(&f.addons), vec!["pak01_dir.vpk.disabled"]);

        let entry = trash_slot(&f.game, &f.trash, 1, None, true).unwrap();
        assert!(!entry.manifest.enabled);
        assert!(names(&f.addons).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_with_a_slot_name_is_never_followed() {
        let f = fixture("symlink");
        let outside = f.game.join("precious.txt");
        fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, f.addons.join("pak05_dir.vpk")).unwrap();
        assert!(trash_slot(&f.game, &f.trash, 5, None, true).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    }

    #[test]
    fn restore_never_overwrites_a_reused_slot() {
        let f = fixture("reused");
        fs::write(f.addons.join("pak02_dir.vpk"), "old").unwrap();
        let entry = trash_slot(&f.game, &f.trash, 2, None, true).unwrap();
        fs::write(f.addons.join("pak02_dir.vpk.disabled"), "new").unwrap();
        let error = restore(&entry.dir, true).unwrap_err().to_string();
        assert!(error.contains("in use"), "{error}");
        assert_eq!(fs::read_to_string(f.addons.join("pak02_dir.vpk.disabled")).unwrap(), "new");
        assert!(entry.dir.join("pak02_dir.vpk").is_file());
    }

    #[test]
    fn restore_refuses_a_tampered_manifest() {
        let f = fixture("tampered");
        fs::write(f.addons.join("pak02_dir.vpk"), "old").unwrap();
        let entry = trash_slot(&f.game, &f.trash, 2, None, true).unwrap();
        let mut manifest = entry.manifest.clone();
        manifest.files = vec!["../../evil.vpk".to_string()];
        write_manifest(&entry.dir, &manifest).unwrap();
        assert!(restore(&entry.dir, true).is_err());
        assert!(names(&f.addons).is_empty());
    }

    #[test]
    fn empty_trash_removes_entries_only() {
        let f = fixture("empty");
        fs::write(f.addons.join("pak01_dir.vpk"), "a").unwrap();
        fs::write(f.addons.join("pak02_dir.vpk"), "bb").unwrap();
        let (entries, errors) = trash_slots(&f.game, &f.trash, &[(1, None), (2, None), (3, None)], true);
        assert_eq!(entries.len(), 2);
        assert_eq!(errors.len(), 1);
        fs::create_dir_all(f.trash.join("not-an-entry")).unwrap();
        assert!(empty_trash(&f.trash, false).is_err());
        let report = empty_trash(&f.trash, true).unwrap();
        assert_eq!(report, EmptyReport { entries: 2, bytes: 3 });
        assert_eq!(names(&f.trash), vec!["not-an-entry"]);
        assert!(purge_entry(&f.trash, &f.trash.join("not-an-entry")).is_err());
    }

    #[test]
    fn undo_update_puts_the_backup_back_and_trashes_the_new_files() {
        let f = fixture("undo-update");
        let backups = f.game.join("backups");
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("pak06_dir.vpk.disabled"), "old").unwrap();
        fs::write(f.addons.join("pak06_dir.vpk.disabled"), "new").unwrap();
        fs::write(f.addons.join("pak06_000.vpk"), "new-chunk").unwrap();

        let wrong = vec![backups.join("pak06_dir.vpk.disabled"), f.game.join("x.vpk")];
        assert!(undo_update(&f.game, &f.trash, 6, &wrong, true).is_err());
        assert_eq!(fs::read_to_string(f.addons.join("pak06_dir.vpk.disabled")).unwrap(), "new");

        let trashed = undo_update(&f.game, &f.trash, 6, &[backups.join("pak06_dir.vpk.disabled")], true)
            .unwrap()
            .unwrap();
        assert_eq!(names(&f.addons), vec!["pak06_dir.vpk.disabled"]);
        assert_eq!(fs::read_to_string(f.addons.join("pak06_dir.vpk.disabled")).unwrap(), "old");
        assert_eq!(trashed.manifest.files.len(), 2);
        assert!(backups.join("pak06_dir.vpk.disabled").is_file());
    }
}
