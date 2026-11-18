//! The copies Sigil keeps before it edits a file: list them, restore one, delete one, prune old
//! ones.
//!
//! Three shapes exist on disk:
//!
//! - `<file>.dah-backup-<stamp>` next to the file it copies (gameinfo.gi repair, autoexec.cfg,
//!   Servers, Steam launch options).
//! - `<file>.<ext>.sigil-bak` next to the file (the one-time copy Performance takes of
//!   gameinfo.gi and video.txt).
//! - `<workspace>/.dah/install/backups/<stamp>/` holding addon files an install replaced.
//!
//! The target of a single-file backup is derived from its own name, never passed in, so a
//! restore can only put a file back where it came from.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

/// The marker deploy, autoexec, servers and launch options use for stamped copies.
pub const STAMPED_MARKER: &str = ".dah-backup-";
/// The suffix Performance uses for its one-time copy.
pub const ONE_TIME_SUFFIX: &str = ".sigil-bak";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    Gameinfo,
    Autoexec,
    Video,
    LaunchOptions,
    Addons,
    Other,
}

impl BackupKind {
    pub fn label(self) -> &'static str {
        match self {
            BackupKind::Gameinfo => "gameinfo.gi",
            BackupKind::Autoexec => "autoexec.cfg",
            BackupKind::Video => "video.txt",
            BackupKind::LaunchOptions => "Steam launch options",
            BackupKind::Addons => "Replaced addons",
            BackupKind::Other => "Other",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub kind: BackupKind,
    /// The backup file, or the stamp folder for addon backups.
    pub path: PathBuf,
    /// Where a restore writes. None for addon backups, which are not restored in place.
    pub target: Option<PathBuf>,
    /// Bytes on disk (summed over a folder).
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    /// Files in an addon backup folder, 1 for a single file.
    pub files: usize,
    /// A one-time copy (`.sigil-bak`) that pruning keeps.
    pub one_time: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BackupSources {
    /// `<game>/game/citadel`, for gameinfo.gi backups.
    pub citadel_dir: Option<PathBuf>,
    /// `<game>/game/citadel/cfg`, for autoexec.cfg backups.
    pub cfg_dir: Option<PathBuf>,
    /// The folder holding the video.txt the game reads.
    pub video_dir: Option<PathBuf>,
    /// The folder holding Steam's localconfig.vdf.
    pub localconfig_dir: Option<PathBuf>,
    /// The Sigil workspace, for addon install backups.
    pub workspace: Option<PathBuf>,
}

impl BackupSources {
    pub fn for_game(game_root: &Path) -> BackupSources {
        let citadel = game_root.join("game").join("citadel");
        BackupSources {
            cfg_dir: Some(citadel.join("cfg")),
            citadel_dir: Some(citadel),
            ..BackupSources::default()
        }
    }
}

/// The file a single-file backup restores to, from its name alone.
pub fn target_for(backup: &Path) -> Option<PathBuf> {
    let name = backup.file_name()?.to_str()?;
    let original = if let Some(index) = name.find(STAMPED_MARKER) {
        &name[..index]
    } else if let Some(stem) = name.strip_suffix(ONE_TIME_SUFFIX) {
        stem
    } else {
        return None;
    };
    if original.is_empty() {
        return None;
    }
    Some(backup.with_file_name(original))
}

fn kind_for(original: &str) -> BackupKind {
    match original.to_ascii_lowercase().as_str() {
        "gameinfo.gi" => BackupKind::Gameinfo,
        "autoexec.cfg" => BackupKind::Autoexec,
        "video.txt" => BackupKind::Video,
        "localconfig.vdf" => BackupKind::LaunchOptions,
        _ => BackupKind::Other,
    }
}

/// Every backup in the given places, newest first. Missing folders are skipped.
pub fn list(sources: &BackupSources) -> Vec<BackupEntry> {
    let mut out = Vec::new();
    for dir in [
        &sources.citadel_dir,
        &sources.cfg_dir,
        &sources.video_dir,
        &sources.localconfig_dir,
    ]
    .into_iter()
    .flatten()
    {
        list_files(dir, &mut out);
    }
    if let Some(workspace) = &sources.workspace {
        list_addon_backups(&crate::install::backups_dir(workspace), &mut out);
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn list_files(dir: &Path, out: &mut Vec<BackupEntry>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Some(target) = target_for(&path) else {
            continue;
        };
        let original = target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let one_time = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(ONE_TIME_SUFFIX));
        out.push(BackupEntry {
            kind: kind_for(original),
            target: Some(target),
            bytes: meta.len(),
            modified: meta.modified().ok(),
            files: 1,
            one_time,
            path,
        });
    }
}

fn list_addon_backups(root: &Path, out: &mut Vec<BackupEntry>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let (mut bytes, mut files) = (0, 0);
        if let Ok(inner) = fs::read_dir(&path) {
            for file in inner.flatten() {
                if let Ok(meta) = file.metadata()
                    && meta.is_file()
                {
                    bytes += meta.len();
                    files += 1;
                }
            }
        }
        out.push(BackupEntry {
            kind: BackupKind::Addons,
            modified: entry.metadata().and_then(|meta| meta.modified()).ok(),
            target: None,
            bytes,
            files,
            one_time: false,
            path,
        });
    }
}

#[derive(Debug, Clone)]
pub struct RestoreResult {
    pub target: PathBuf,
    /// The copy of the file the restore replaced, so the restore itself can be undone.
    pub previous: Option<PathBuf>,
}

/// GAME WRITE. Put a single-file backup back where it came from. The file being replaced is
/// copied aside first as `<name>.dah-backup-<stamp>`, then the backup's bytes are written
/// atomically. Called only from the Restore button after its confirm.
pub fn restore(backup: &Path, approve: bool) -> Result<RestoreResult> {
    if !approve {
        return Err(Error::msg(format!(
            "Restore of {} was not approved. Nothing was written.",
            backup.display()
        )));
    }
    let target = target_for(backup).ok_or_else(|| {
        Error::msg(format!("{} is not a Sigil backup.", backup.display()))
    })?;
    // Only the files Sigil itself backs up may be restored: a stray `<any>.dah-backup-x`
    // beside an executable or another file must never be written over its sibling.
    let original = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if matches!(kind_for(original), BackupKind::Other | BackupKind::Addons) {
        return Err(Error::msg(format!(
            "{} is not a file Sigil restores. Nothing was written.",
            backup.display()
        )));
    }
    let bytes = fs::read(backup)?;
    let previous = if target.is_file() {
        let name = target
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let aside = target.with_file_name(format!("{name}{STAMPED_MARKER}{}", stamp()));
        fs::copy(&target, &aside)?;
        Some(aside)
    } else {
        None
    };
    write_atomic(&target, &bytes)?;
    Ok(RestoreResult { target, previous })
}

/// Delete one backup. A single file must carry a backup name; a folder must sit directly in
/// the workspace's install backups folder. Anything else is refused.
pub fn delete(backup: &Path, workspace: Option<&Path>) -> Result<()> {
    if backup.is_file() {
        if target_for(backup).is_none() {
            return Err(Error::msg(format!(
                "{} is not a Sigil backup. Nothing was deleted.",
                backup.display()
            )));
        }
        fs::remove_file(backup)?;
        return Ok(());
    }
    if backup.is_dir() {
        let root = workspace.map(crate::install::backups_dir);
        let inside = root
            .as_deref()
            .is_some_and(|root| backup.parent() == Some(root));
        if !inside {
            return Err(Error::msg(format!(
                "{} is not an install backup folder. Nothing was deleted.",
                backup.display()
            )));
        }
        fs::remove_dir_all(backup)?;
        return Ok(());
    }
    Err(Error::msg(format!("{} does not exist.", backup.display())))
}

/// Delete stamped backups beyond the newest `keep` per target (addon folders count as one
/// target). One-time `.sigil-bak` copies are kept. Returns how many were deleted.
pub fn prune(entries: &[BackupEntry], keep: usize, workspace: Option<&Path>) -> Result<usize> {
    let mut seen: Vec<(Option<PathBuf>, usize)> = Vec::new();
    let mut doomed = Vec::new();
    // `entries` from `list` are newest first, so the first `keep` per target survive.
    let mut sorted: Vec<&BackupEntry> = entries.iter().filter(|e| !e.one_time).collect();
    sorted.sort_by(|a, b| b.modified.cmp(&a.modified));
    for entry in sorted {
        let key = entry.target.clone();
        match seen.iter_mut().find(|(target, _)| *target == key) {
            Some((_, count)) => {
                *count += 1;
                if *count > keep {
                    doomed.push(entry.path.clone());
                }
            }
            None => {
                seen.push((key, 1));
                if keep == 0 {
                    doomed.push(entry.path.clone());
                }
            }
        }
    }
    for path in &doomed {
        delete(path, workspace)?;
    }
    Ok(doomed.len())
}

/// One install backup that holds files of a single slot: what an install replaced there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotBackup {
    pub slot: u16,
    /// The stamp folder under `<workspace>/.dah/install/backups/`.
    pub folder: PathBuf,
    /// The backed-up files that still exist, by their slot file names.
    pub files: Vec<PathBuf>,
    pub bytes: u64,
    /// When the install that took the backup ran (journal `installedAt`, RFC 3339).
    pub taken_at: String,
    /// The mod that was in the slot before, from the previous journal entry for the slot. None
    /// when the slot held a file Sigil did not install.
    pub previous_label: Option<String>,
    /// The mod whose install replaced it.
    pub replaced_by: String,
    /// The install's plan id, for matching a journal line.
    pub plan_id: String,
}

/// The install backups of one slot, newest first, from the install journal. A slot number is
/// global (1 to 990), so an overflow slot's `pakNN` files never mix with `citadel/addons`'s
/// own `pakNN`. Backups whose files were deleted or pruned are left out.
pub fn for_slot(workspace: &Path, slot: u16) -> Vec<SlotBackup> {
    let Ok(journal) = crate::install::read_journal(workspace) else {
        return Vec::new();
    };
    let root = crate::install::backups_dir(workspace);
    let mut out = Vec::new();
    for (index, entry) in journal.iter().enumerate() {
        if entry.slot != slot || entry.backups.is_empty() {
            continue;
        }
        let files: Vec<PathBuf> = entry
            .backups
            .iter()
            .filter(|path| path.is_file() && path.parent().and_then(Path::parent) == Some(&root))
            .cloned()
            .collect();
        let Some(folder) = files.first().and_then(|path| path.parent()).map(Path::to_path_buf)
        else {
            continue;
        };
        let bytes = files
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum();
        let previous_label = journal[..index]
            .iter()
            .rev()
            .find(|earlier| earlier.slot == slot)
            .map(|earlier| earlier.source_label.clone());
        out.push(SlotBackup {
            slot,
            folder,
            files,
            bytes,
            taken_at: entry.installed_at.clone(),
            previous_label,
            replaced_by: entry.source_label.clone(),
            plan_id: entry.plan_id.clone(),
        });
    }
    out.reverse();
    out
}

/// Every slot with at least one install backup, with its count, for a badge on the mod card.
pub fn slots_with_backups(workspace: &Path) -> std::collections::BTreeMap<u16, usize> {
    let mut counts = std::collections::BTreeMap::new();
    let Ok(journal) = crate::install::read_journal(workspace) else {
        return counts;
    };
    for entry in journal {
        if entry.backups.iter().any(|path| path.is_file()) {
            *counts.entry(entry.slot).or_insert(0) += 1;
        }
    }
    counts
}

fn stamp() -> String {
    format!("{}-{}", now_iso().replace([':', '.'], "-"), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dah-backups-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn game(root: &Path) -> PathBuf {
        let citadel = root.join("game").join("citadel");
        fs::create_dir_all(citadel.join("cfg")).unwrap();
        fs::write(citadel.join("gameinfo.gi"), "current").unwrap();
        citadel
    }

    #[test]
    fn restore_refuses_files_sigil_does_not_back_up() {
        let dir = scratch("unknown");
        fs::write(dir.join("Sigil.exe"), "real").unwrap();
        let stray = dir.join("Sigil.exe.dah-backup-20261117T000000");
        fs::write(&stray, "evil").unwrap();
        assert!(restore(&stray, true).is_err());
        assert_eq!(fs::read_to_string(dir.join("Sigil.exe")).unwrap(), "real");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn targets_come_from_the_name() {
        let dir = Path::new("/g/citadel");
        assert_eq!(
            target_for(&dir.join("gameinfo.gi.dah-backup-2026-01-01T00-00-00-000Z-12")),
            Some(dir.join("gameinfo.gi"))
        );
        assert_eq!(
            target_for(&dir.join("gameinfo.gi.sigil-bak")),
            Some(dir.join("gameinfo.gi"))
        );
        assert_eq!(target_for(&dir.join("gameinfo.gi")), None);
        assert_eq!(target_for(&dir.join(".dah-backup-x")), None);
    }

    #[test]
    fn lists_each_shape_and_skips_other_files() {
        let root = scratch("list");
        let citadel = game(&root);
        fs::write(citadel.join("gameinfo.gi.dah-backup-a"), "old").unwrap();
        fs::write(citadel.join("gameinfo.gi.sigil-bak"), "first").unwrap();
        fs::write(citadel.join("cfg").join("autoexec.cfg.dah-backup-b"), "x").unwrap();
        fs::write(citadel.join("cfg").join("config.cfg"), "x").unwrap();
        let workspace = root.join("ws");
        let stamp = crate::install::backups_dir(&workspace).join("2026");
        fs::create_dir_all(&stamp).unwrap();
        fs::write(stamp.join("pak01_dir.vpk"), "1234").unwrap();

        let mut sources = BackupSources::for_game(&root);
        sources.workspace = Some(workspace);
        let entries = list(&sources);
        assert_eq!(entries.len(), 4);
        let addons = entries.iter().find(|e| e.kind == BackupKind::Addons).unwrap();
        assert_eq!((addons.files, addons.bytes), (1, 4));
        assert!(addons.target.is_none());
        assert_eq!(
            entries.iter().filter(|e| e.kind == BackupKind::Gameinfo).count(),
            2
        );
        assert!(entries.iter().any(|e| e.kind == BackupKind::Autoexec));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_needs_approval_and_keeps_the_replaced_file() {
        let root = scratch("restore");
        let citadel = game(&root);
        let backup = citadel.join("gameinfo.gi.dah-backup-a");
        fs::write(&backup, "old").unwrap();

        assert!(restore(&backup, false).is_err());
        assert_eq!(fs::read_to_string(citadel.join("gameinfo.gi")).unwrap(), "current");

        let result = restore(&backup, true).unwrap();
        assert_eq!(result.target, citadel.join("gameinfo.gi"));
        assert_eq!(fs::read_to_string(&result.target).unwrap(), "old");
        let previous = result.previous.unwrap();
        assert_eq!(fs::read_to_string(&previous).unwrap(), "current");
        assert_eq!(target_for(&previous), Some(citadel.join("gameinfo.gi")));
        // The backup itself stays.
        assert!(backup.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_refuses_a_file_that_is_not_a_backup() {
        let root = scratch("refuse");
        let citadel = game(&root);
        assert!(restore(&citadel.join("gameinfo.gi"), true).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_refuses_live_files_and_foreign_folders() {
        let root = scratch("delete");
        let citadel = game(&root);
        assert!(delete(&citadel.join("gameinfo.gi"), None).is_err());
        assert!(citadel.join("gameinfo.gi").is_file());
        assert!(delete(&citadel, Some(&root.join("ws"))).is_err());
        assert!(citadel.is_dir());

        let backup = citadel.join("gameinfo.gi.dah-backup-a");
        fs::write(&backup, "old").unwrap();
        delete(&backup, None).unwrap();
        assert!(!backup.exists());

        let workspace = root.join("ws");
        let stamp = crate::install::backups_dir(&workspace).join("s1");
        fs::create_dir_all(&stamp).unwrap();
        delete(&stamp, Some(&workspace)).unwrap();
        assert!(!stamp.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prune_keeps_the_newest_per_target_and_one_time_copies() {
        let root = scratch("prune");
        let citadel = game(&root);
        for (index, name) in ["a", "b", "c"].iter().enumerate() {
            let path = citadel.join(format!("gameinfo.gi.dah-backup-{name}"));
            fs::write(&path, name).unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + index as u64))
                .unwrap();
        }
        fs::write(citadel.join("gameinfo.gi.sigil-bak"), "first").unwrap();
        fs::write(citadel.join("cfg").join("autoexec.cfg.dah-backup-z"), "z").unwrap();

        let entries = list(&BackupSources::for_game(&root));
        let removed = prune(&entries, 1, None).unwrap();
        assert_eq!(removed, 2);
        assert!(citadel.join("gameinfo.gi.dah-backup-c").is_file());
        assert!(!citadel.join("gameinfo.gi.dah-backup-a").exists());
        assert!(!citadel.join("gameinfo.gi.dah-backup-b").exists());
        assert!(citadel.join("gameinfo.gi.sigil-bak").is_file());
        assert!(citadel.join("cfg").join("autoexec.cfg.dah-backup-z").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    fn journal_line(workspace: &Path, slot: u16, label: &str, at: &str, backups: Vec<PathBuf>) {
        let entry = crate::install::InstallJournalEntry {
            schema: 1,
            plan_id: format!("plan-{label}"),
            installed_at: at.to_string(),
            source: label.to_string(),
            source_label: label.to_string(),
            download_url: None,
            archive: PathBuf::new(),
            archive_sha256: String::new(),
            slot,
            file: crate::addons::slot_name(slot, "_dir.vpk"),
            target: PathBuf::new(),
            chunk_files: Vec::new(),
            entries: 1,
            bytes: 1,
            vpk_bytes: 1,
            vpk_sha256: String::new(),
            backups,
            removed_chunks: Vec::new(),
            removed_files: Vec::new(),
            gameinfo_repaired: false,
            warnings: Vec::new(),
        };
        let path = crate::install::journal_path(workspace);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = fs::read_to_string(&path).unwrap_or_default();
        text.push_str(&serde_json::to_string(&entry).unwrap());
        text.push('\n');
        fs::write(&path, text).unwrap();
    }

    #[test]
    fn slot_backups_come_from_the_journal_newest_first() {
        let workspace = scratch("slot");
        let root = crate::install::backups_dir(&workspace);
        let first = root.join("2026-01-01T00-00-00Z-1");
        let second = root.join("2026-02-01T00-00-00Z-1");
        let overflow = root.join("2026-03-01T00-00-00Z-1");
        for dir in [&first, &second, &overflow] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(first.join("pak05_dir.vpk"), b"aaaa").unwrap();
        fs::write(second.join("pak05_dir.vpk"), b"bb").unwrap();
        fs::write(second.join("pak05_000.vpk"), b"ccc").unwrap();
        // Slot 104 is pak05 in the second addon folder: the same file name, another slot.
        fs::write(overflow.join("pak05_dir.vpk"), b"d").unwrap();
        journal_line(&workspace, 5, "Alpha", "2026-01-01T00:00:00Z", Vec::new());
        journal_line(&workspace, 5, "Beta", "2026-01-01T00:00:01Z", vec![first.join("pak05_dir.vpk")]);
        journal_line(
            &workspace,
            5,
            "Gamma",
            "2026-02-01T00:00:00Z",
            vec![second.join("pak05_dir.vpk"), second.join("pak05_000.vpk")],
        );
        journal_line(&workspace, 104, "Delta", "2026-03-01T00:00:00Z", vec![overflow.join("pak05_dir.vpk")]);
        // A pruned backup is skipped.
        journal_line(&workspace, 5, "Gone", "2026-04-01T00:00:00Z", vec![root.join("x").join("pak05_dir.vpk")]);

        let list = for_slot(&workspace, 5);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].replaced_by, "Gamma");
        assert_eq!(list[0].previous_label.as_deref(), Some("Beta"));
        assert_eq!(list[0].files.len(), 2);
        assert_eq!(list[0].bytes, 5);
        assert_eq!(list[0].folder, second);
        assert_eq!(list[1].replaced_by, "Beta");
        assert_eq!(list[1].previous_label.as_deref(), Some("Alpha"));
        assert_eq!(list[1].taken_at, "2026-01-01T00:00:01Z");

        let overflow_list = for_slot(&workspace, 104);
        assert_eq!(overflow_list.len(), 1);
        assert_eq!(overflow_list[0].previous_label, None);

        let counts = slots_with_backups(&workspace);
        assert_eq!(counts.get(&5), Some(&2));
        assert_eq!(counts.get(&104), Some(&1));
        assert!(for_slot(&workspace, 7).is_empty());
        let _ = fs::remove_dir_all(&workspace);
    }
}
