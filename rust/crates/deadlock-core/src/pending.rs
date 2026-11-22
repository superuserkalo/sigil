//! Changes that wait for Deadlock to close.
//!
//! Deadlock holds its addon VPKs open while it runs, so on Windows a delete or a switch off of
//! an enabled mod fails with a sharing violation (os error 32). Instead of an error, the player
//! can queue the change: [`enqueue`] records it in Sigil's own app data
//! (`<data_dir>/pending-changes.json`, never the game folder), and [`apply_all`] runs the queue
//! in order once the game process is gone.
//!
//! Every entry is one explicit change: its kind, its slot, the exact path of the slot's
//! directory file and the name, size and modified time of every file of the slot when the
//! player chose it. Before a change runs, the slot is read again; when anything differs the
//! change is skipped and reported, never applied to a different file. A change runs through the
//! same call as the immediate action ([`crate::trash::trash_slot`] for a delete, so the files
//! still go to Sigil's trash, and [`crate::addons::set_enabled`] for a switch off). A change that
//! fails keeps its place with the error.
//!
//! Every entry is checked against the game root the app passes in, on load and again on apply:
//! its path must be exactly the directory file of its slot in `citadel/addons` or
//! `citadel/sigil-addons2` to `sigil-addons10`, and every file name a plain file of that slot.
//! Anything else is rejected and dropped. These are player actions in the desktop app: no agent
//! tool and no CLI command calls this module (a test below holds the agent host and `dah-rs` to
//! that).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::addons::{self, AddonToggleResult};
use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::import::ModMeta;
use crate::load_order::{check_plain_name, read_slot_files, slot_of};
use crate::trash::{self, TrashEntry};
use crate::util::now_iso;

pub const PENDING_FILE: &str = "pending-changes.json";
pub const PENDING_SCHEMA: u32 = 1;

/// One reader or writer of the queue file at a time in this process. Held only around a load
/// and a save, never while files move.
static LOCK: Mutex<()> = Mutex::new(());

/// What a queued change does. Turning a mod on needs no queue: the game never opens a
/// `.disabled` file, so that rename works while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingKind {
    /// Move every file of the slot to Sigil's trash ([`trash::trash_slot`]).
    Delete,
    /// Rename `pakNN_dir.vpk` to `pakNN_dir.vpk.disabled` ([`addons::set_enabled`]).
    TurnOff,
}

/// A file of the slot as the player saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub name: String,
    pub size: u64,
    /// Modified time in milliseconds since 1970, 0 when the system has none.
    pub modified_ms: u64,
}

/// One change waiting for the game to close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    pub id: u64,
    pub kind: PendingKind,
    pub slot: u16,
    /// The slot's directory file (`pakNN_dir.vpk` or `pakNN_dir.vpk.disabled`), in full.
    pub path: PathBuf,
    /// Every file of the slot when the player chose the change, sorted by name.
    pub files: Vec<FileStamp>,
    /// The mod's name as the player saw it.
    pub name: String,
    /// The library entry, so a delete's trash entry can restore the name and preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ModMeta>,
    pub queued_at: String,
    /// The last error while applying; the change stays queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQueue {
    pub schema: u32,
    pub next_id: u64,
    pub changes: Vec<PendingChange>,
}

impl Default for PendingQueue {
    fn default() -> Self {
        PendingQueue {
            schema: PENDING_SCHEMA,
            next_id: 1,
            changes: Vec::new(),
        }
    }
}

impl PendingQueue {
    /// The change for `slot`, if one waits.
    pub fn for_slot(&self, slot: u16) -> Option<&PendingChange> {
        self.changes.iter().find(|change| change.slot == slot)
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }
}

/// What [`load`] read: the valid changes and a sentence per rejected one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub queue: PendingQueue,
    pub rejected: Vec<String>,
}

/// What a change did when it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Trashed(TrashEntry),
    TurnedOff(AddonToggleResult),
}

/// What [`apply_all`] did, change by change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub applied: Vec<(PendingChange, Applied)>,
    /// The slot changed since the player chose it: not applied, dropped from the queue.
    pub skipped: Vec<(PendingChange, String)>,
    /// The change failed: it stays queued with this error.
    pub failed: Vec<(PendingChange, String)>,
    /// Entries of the queue file that failed the path check: dropped.
    pub rejected: Vec<String>,
}

impl ApplyReport {
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty() && self.skipped.is_empty() && self.failed.is_empty() && self.rejected.is_empty()
    }
}

/// `<data_dir>/pending-changes.json`.
pub fn pending_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PENDING_FILE)
}

/// Read the slot as it is now: its directory file and every file's stamp. Refuses a slot that
/// is not a slot, a slot with no directory file and anything that is not a regular file.
pub fn stamp_slot(game_root: &Path, slot: u16) -> Result<(PathBuf, Vec<FileStamp>)> {
    if !addons::is_slot(slot) {
        return Err(Error::msg(format!("Slot {slot} is outside 1 to {}.", addons::MAX_SLOT)));
    }
    let addons = addons_dir(game_root);
    let folder = addons::slot_folder(&addons, slot);
    let mut names = read_slot_files(&addons)?.remove(&slot).unwrap_or_default();
    names.sort();
    let mut files = Vec::new();
    for name in names {
        let meta = fs::symlink_metadata(folder.join(&name))?;
        if !meta.file_type().is_file() {
            return Err(Error::msg(format!("{name} is not a plain file.")));
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        files.push(FileStamp { name, size: meta.len(), modified_ms });
    }
    let on = addons::slot_name(slot, "_dir.vpk");
    let off = format!("{on}{}", addons::DISABLED_SUFFIX);
    let dir_file = files
        .iter()
        .find(|f| f.name == on)
        .or_else(|| files.iter().find(|f| f.name == off))
        .map(|f| folder.join(&f.name))
        .ok_or_else(|| Error::msg(format!("Slot {} is not installed.", addons::slot_label(slot))))?;
    Ok((dir_file, files))
}

/// A change for `slot` as it is on disk now. A switch off needs the slot to be on.
pub fn new_change(
    game_root: &Path,
    kind: PendingKind,
    slot: u16,
    name: &str,
    meta: Option<ModMeta>,
) -> Result<PendingChange> {
    let (path, files) = stamp_slot(game_root, slot)?;
    let change = PendingChange {
        id: 0,
        kind,
        slot,
        path,
        files,
        name: name.to_string(),
        meta,
        queued_at: now_iso(),
        error: None,
    };
    validate(game_root, &change)?;
    Ok(change)
}

/// The path check every change passes on load, on enqueue and before it runs: the slot is a
/// slot, the path is exactly its directory file in its own addon folder under `game_root`, and
/// every file name is a plain file name of that slot.
pub fn validate(game_root: &Path, change: &PendingChange) -> Result<()> {
    let label = format!("{} ({})", change.name, change.path.display());
    if !addons::is_slot(change.slot) {
        return Err(Error::msg(format!("{label}: slot {} is not an addon slot.", change.slot)));
    }
    let addons = addons_dir(game_root);
    let on = addons::slot_path(&addons, change.slot, "_dir.vpk");
    let off = addons::slot_path(&addons, change.slot, &format!("_dir.vpk{}", addons::DISABLED_SUFFIX));
    let allowed = match change.kind {
        PendingKind::Delete => change.path == on || change.path == off,
        PendingKind::TurnOff => change.path == on,
    };
    if !allowed {
        return Err(Error::msg(format!(
            "{label} is not the file of slot {} in the game's addon folders.",
            addons::slot_label(change.slot)
        )));
    }
    let dir_name = change.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if change.files.is_empty() || !change.files.iter().any(|f| f.name == dir_name) {
        return Err(Error::msg(format!("{label}: the file list does not hold the slot's file.")));
    }
    let local = addons::local_slot(change.slot);
    for file in &change.files {
        check_plain_name(&file.name)?;
        if slot_of(&file.name) != Some(local) {
            return Err(Error::msg(format!("{label}: {} is not a file of its slot.", file.name)));
        }
    }
    Ok(())
}

/// Read the queue. A missing file is an empty queue. Changes that fail [`validate`] are left
/// out, each with a sentence in `rejected`; the next save drops them.
pub fn load(file: &Path, game_root: &Path) -> Result<Loaded> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    load_unlocked(file, game_root)
}

fn load_unlocked(file: &Path, game_root: &Path) -> Result<Loaded> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::default()),
        Err(error) => return Err(error.into()),
    };
    let mut queue: PendingQueue = serde_json::from_str(&text)
        .map_err(|error| Error::msg(format!("{} is not a list of waiting changes: {error}", file.display())))?;
    if queue.schema != PENDING_SCHEMA {
        return Err(Error::msg(format!(
            "{} has schema {}, this Sigil reads {PENDING_SCHEMA}.",
            file.display(),
            queue.schema
        )));
    }
    let mut rejected = Vec::new();
    queue.changes.retain(|change| match validate(game_root, change) {
        Ok(()) => true,
        Err(error) => {
            rejected.push(error.to_string());
            false
        }
    });
    let top = queue.changes.iter().map(|c| c.id).max().unwrap_or(0);
    queue.next_id = queue.next_id.max(top + 1);
    Ok(Loaded { queue, rejected })
}

fn save_unlocked(file: &Path, queue: &PendingQueue) -> Result<()> {
    if queue.changes.is_empty() {
        return match fs::remove_file(file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_string_pretty(queue)
        .map_err(|error| Error::msg(format!("Could not encode the waiting changes: {error}")))?;
    crate::util::write_atomic(file, text.as_bytes())
}

/// Add a change at the end of the queue. A change already waiting for the same slot is
/// replaced, so one slot never has two. Returns the queue as saved.
pub fn enqueue(file: &Path, game_root: &Path, mut change: PendingChange) -> Result<PendingQueue> {
    validate(game_root, &change)?;
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut queue = load_unlocked(file, game_root)?.queue;
    queue.changes.retain(|c| c.slot != change.slot);
    change.id = queue.next_id;
    change.error = None;
    queue.next_id += 1;
    queue.changes.push(change);
    save_unlocked(file, &queue)?;
    Ok(queue)
}

/// Take a change out of the queue (the player's undo). Nothing in the game folder changes.
pub fn cancel(file: &Path, game_root: &Path, id: u64) -> Result<PendingQueue> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut queue = load_unlocked(file, game_root)?.queue;
    queue.changes.retain(|c| c.id != id);
    save_unlocked(file, &queue)?;
    Ok(queue)
}

/// Take every change out of the queue.
pub fn cancel_all(file: &Path, game_root: &Path) -> Result<PendingQueue> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = load_unlocked(file, game_root)?;
    let queue = PendingQueue::default();
    save_unlocked(file, &queue)?;
    Ok(queue)
}

/// How long to wait before each retry of a change whose file is still held: Windows can keep
/// a file locked for a moment after the game process is gone.
pub const RETRY_BACKOFF: [Duration; 3] = [Duration::from_secs(1), Duration::from_secs(3), Duration::from_secs(8)];

/// GAME WRITE. Run every waiting change in order. The caller makes sure the game is closed.
/// Each change is validated again and its slot compared with the stamps the player saw; a
/// changed slot is skipped and dropped, a failed change stays with its error, and an applied
/// one leaves the queue. The file is saved after every change, so a crash midway never runs
/// a change twice (the second run finds the slot changed). Refuses without `approve`.
///
/// A change that fails only because a file is in use is tried again after each pause of
/// [`RETRY_BACKOFF`] (1, 3 and 8 seconds) before it counts as failed. Run this on a worker: it
/// sleeps.
pub fn apply_all(file: &Path, game_root: &Path, trash_root: &Path, approve: bool) -> Result<ApplyReport> {
    apply_all_with(file, game_root, trash_root, approve, &RETRY_BACKOFF)
}

/// [`apply_all`] with its own retry pauses (tests pass zero length ones).
pub fn apply_all_with(
    file: &Path,
    game_root: &Path,
    trash_root: &Path,
    approve: bool,
    backoff: &[Duration],
) -> Result<ApplyReport> {
    if !approve {
        return Err(Error::msg("Not approved: apply the waiting changes. Nothing was changed."));
    }
    let mut report = ApplyReport::default();
    let ids: Vec<u64> = {
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let loaded = load_unlocked(file, game_root)?;
        report.rejected = loaded.rejected;
        // Drop rejected entries from the file now.
        save_unlocked(file, &loaded.queue)?;
        loaded.queue.changes.iter().map(|c| c.id).collect()
    };
    for id in ids {
        // The player may have cancelled it since the run began.
        let change = {
            let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            load_unlocked(file, game_root)?.queue.changes.into_iter().find(|c| c.id == id)
        };
        let Some(change) = change else { continue };
        let outcome = with_retries(backoff, || {
            // The player may cancel while a retry waits.
            let still = {
                let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                load_unlocked(file, game_root)
                    .map(|loaded| loaded.queue.changes.iter().any(|c| c.id == id))
                    .unwrap_or(false)
            };
            if still { run_one(game_root, trash_root, &change) } else { Outcome::Cancelled }
        });
        if matches!(outcome, Outcome::Cancelled) {
            continue;
        }
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut queue = load_unlocked(file, game_root)?.queue;
        match outcome {
            Outcome::Applied(applied) => {
                queue.changes.retain(|c| c.id != id);
                report.applied.push((change, applied));
            }
            Outcome::Skipped(why) => {
                queue.changes.retain(|c| c.id != id);
                report.skipped.push((change, why));
            }
            Outcome::Cancelled => {}
            Outcome::Failed(error) => {
                if let Some(kept) = queue.changes.iter_mut().find(|c| c.id == id) {
                    kept.error = Some(error.clone());
                }
                report.failed.push((change, error));
            }
        }
        save_unlocked(file, &queue)?;
    }
    Ok(report)
}

enum Outcome {
    Applied(Applied),
    Skipped(String),
    Failed(String),
    Cancelled,
}

/// Run `attempt`, and again after each pause while it fails only because a file is in use.
fn with_retries(backoff: &[Duration], mut attempt: impl FnMut() -> Outcome) -> Outcome {
    let mut outcome = attempt();
    for pause in backoff {
        match &outcome {
            Outcome::Failed(error) if crate::error::mentions_in_use(error) => {
                std::thread::sleep(*pause);
                outcome = attempt();
            }
            _ => break,
        }
    }
    outcome
}

fn run_one(game_root: &Path, trash_root: &Path, change: &PendingChange) -> Outcome {
    if let Err(error) = validate(game_root, change) {
        return Outcome::Skipped(error.to_string());
    }
    match stamp_slot(game_root, change.slot) {
        Ok((path, files)) if path == change.path && files == change.files => {}
        Ok(_) | Err(_) => {
            return Outcome::Skipped(format!(
                "{} changed since you chose it, so it was left alone.",
                change.name
            ));
        }
    }
    let result = match change.kind {
        PendingKind::Delete => trash::trash_slot(game_root, trash_root, change.slot, change.meta.clone(), true)
            .map(Applied::Trashed),
        PendingKind::TurnOff => addons::set_enabled(game_root, change.slot, false, true).map(Applied::TurnedOff),
    };
    match result {
        Ok(applied) => Outcome::Applied(applied),
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        game: PathBuf,
        addons: PathBuf,
        trash: PathBuf,
        file: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/pending-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let game = root.join("Deadlock");
        let addons = game.join("game").join("citadel").join("addons");
        fs::create_dir_all(&addons).unwrap();
        let data = root.join("data");
        Fixture {
            game,
            addons,
            trash: trash::trash_root(&data),
            file: pending_path(&data),
        }
    }

    fn queue(f: &Fixture, kind: PendingKind, slot: u16) -> PendingQueue {
        let change = new_change(&f.game, kind, slot, &format!("Mod {slot}"), None).unwrap();
        enqueue(&f.file, &f.game, change).unwrap()
    }

    #[test]
    fn the_queue_survives_a_reload_in_order() {
        let f = fixture("persist");
        fs::write(f.addons.join("pak02_dir.vpk"), "b").unwrap();
        fs::write(f.addons.join("pak05_dir.vpk"), "e").unwrap();
        fs::write(f.addons.join("pak05_000.vpk"), "chunk").unwrap();
        queue(&f, PendingKind::Delete, 5);
        queue(&f, PendingKind::TurnOff, 2);
        let loaded = load(&f.file, &f.game).unwrap();
        assert!(loaded.rejected.is_empty());
        let slots: Vec<u16> = loaded.queue.changes.iter().map(|c| c.slot).collect();
        assert_eq!(slots, vec![5, 2]);
        assert_eq!(loaded.queue.changes[0].files.len(), 2);
        assert_eq!(loaded.queue.changes[0].path, f.addons.join("pak05_dir.vpk"));
        // A second change for a slot replaces the first and goes to the end.
        let q = queue(&f, PendingKind::Delete, 2);
        assert_eq!(q.len(), 2);
        assert_eq!(q.for_slot(2).unwrap().kind, PendingKind::Delete);
        let ids: Vec<u64> = q.changes.iter().map(|c| c.id).collect();
        assert!(ids[0] < ids[1]);
        // Cancel takes one out; nothing in the game folder moves.
        let q = cancel(&f.file, &f.game, q.for_slot(5).unwrap().id).unwrap();
        assert_eq!(q.len(), 1);
        assert!(f.addons.join("pak05_dir.vpk").is_file());
        cancel_all(&f.file, &f.game).unwrap();
        assert!(!f.file.exists());
    }

    #[test]
    fn apply_runs_in_order_through_the_trash_and_the_switch() {
        let f = fixture("apply");
        fs::write(f.addons.join("pak03_dir.vpk"), "c").unwrap();
        fs::write(f.addons.join("pak03_000.vpk"), "chunk").unwrap();
        fs::write(f.addons.join("pak04_dir.vpk"), "d").unwrap();
        queue(&f, PendingKind::Delete, 3);
        queue(&f, PendingKind::TurnOff, 4);
        assert!(apply_all(&f.file, &f.game, &f.trash, false).is_err());
        let report = apply_all_with(&f.file, &f.game, &f.trash, true, &[]).unwrap();
        assert_eq!(report.applied.len(), 2);
        assert_eq!(report.applied[0].0.slot, 3);
        let Applied::Trashed(entry) = &report.applied[0].1 else { panic!("delete goes to the trash") };
        assert!(entry.dir.join("pak03_dir.vpk").is_file());
        assert!(entry.dir.join("pak03_000.vpk").is_file());
        assert!(!f.addons.join("pak03_dir.vpk").exists());
        assert!(f.addons.join("pak04_dir.vpk.disabled").is_file());
        // The trash entry restores like any other.
        trash::restore(&entry.dir, true).unwrap();
        assert!(f.addons.join("pak03_dir.vpk").is_file());
        assert!(load(&f.file, &f.game).unwrap().queue.is_empty());
    }

    #[test]
    fn a_slot_that_changed_is_skipped_and_never_widened() {
        let f = fixture("identity");
        fs::write(f.addons.join("pak06_dir.vpk"), "old").unwrap();
        fs::write(f.addons.join("pak07_dir.vpk"), "seven").unwrap();
        queue(&f, PendingKind::Delete, 6);
        queue(&f, PendingKind::Delete, 7);
        // Slot 06 was replaced by another mod of a different size; slot 07 gained a chunk.
        fs::write(f.addons.join("pak06_dir.vpk"), "a different mod").unwrap();
        fs::write(f.addons.join("pak07_000.vpk"), "chunk").unwrap();
        let report = apply_all_with(&f.file, &f.game, &f.trash, true, &[]).unwrap();
        assert!(report.applied.is_empty());
        assert_eq!(report.skipped.len(), 2);
        assert!(report.skipped[0].1.contains("changed since"), "{}", report.skipped[0].1);
        assert!(f.addons.join("pak06_dir.vpk").is_file());
        assert!(f.addons.join("pak07_000.vpk").is_file());
        assert!(!f.trash.exists() || fs::read_dir(&f.trash).unwrap().next().is_none());
        assert!(load(&f.file, &f.game).unwrap().queue.is_empty());
    }

    #[test]
    fn a_tampered_queue_with_a_path_outside_the_addon_folders_is_rejected() {
        let f = fixture("tamper");
        fs::write(f.addons.join("pak08_dir.vpk"), "h").unwrap();
        queue(&f, PendingKind::Delete, 8);
        let outside = f.game.join("game").join("citadel").join("pak01_dir.vpk");
        fs::write(&outside, "game file").unwrap();
        let mut q: PendingQueue = serde_json::from_str(&fs::read_to_string(&f.file).unwrap()).unwrap();
        let good = q.changes[0].clone();
        let mut bad = good.clone();
        bad.id = 50;
        bad.slot = 1;
        bad.path = outside.clone();
        bad.files = vec![FileStamp { name: "pak01_dir.vpk".into(), size: 9, modified_ms: 0 }];
        let mut climb = good.clone();
        climb.id = 51;
        climb.path = f.addons.join("..").join("pak08_dir.vpk");
        let mut nested = good.clone();
        nested.id = 52;
        nested.files.push(FileStamp { name: "../pak08_000.vpk".into(), size: 1, modified_ms: 0 });
        let mut foreign_folder = good.clone();
        foreign_folder.id = 53;
        foreign_folder.path = f.game.join("game").join("citadel").join("sigil-addons11").join("pak08_dir.vpk");
        let mut off_for_turn_off = good.clone();
        off_for_turn_off.id = 54;
        off_for_turn_off.kind = PendingKind::TurnOff;
        off_for_turn_off.path = f.addons.join("pak08_dir.vpk.disabled");
        q.changes = vec![bad, climb, nested, foreign_folder, off_for_turn_off, good];
        fs::write(&f.file, serde_json::to_string(&q).unwrap()).unwrap();

        let loaded = load(&f.file, &f.game).unwrap();
        assert_eq!(loaded.rejected.len(), 5, "{:?}", loaded.rejected);
        assert_eq!(loaded.queue.len(), 1);
        // Enqueue refuses the same.
        let mut bad = loaded.queue.changes[0].clone();
        bad.path = outside.clone();
        assert!(enqueue(&f.file, &f.game, bad).is_err());
        // Apply drops the rejected ones and never touches the outside file.
        let report = apply_all_with(&f.file, &f.game, &f.trash, true, &[]).unwrap();
        assert_eq!(report.rejected.len(), 5);
        assert_eq!(report.applied.len(), 1);
        assert!(outside.is_file());
        // A queue saved for another game root is rejected whole.
        let other = f.game.with_file_name("Other");
        fs::write(f.addons.join("pak09_dir.vpk"), "i").unwrap();
        queue(&f, PendingKind::Delete, 9);
        assert_eq!(load(&f.file, &other).unwrap().rejected.len(), 1);
    }

    #[test]
    fn a_failed_change_stays_queued_with_its_error() {
        let f = fixture("failure");
        fs::write(f.addons.join("pak10_dir.vpk"), "j").unwrap();
        queue(&f, PendingKind::Delete, 10);
        // The trash cannot be made: a plain file sits where its folder goes.
        fs::create_dir_all(f.trash.parent().unwrap()).unwrap();
        fs::write(&f.trash, "not a folder").unwrap();
        let report = apply_all_with(&f.file, &f.game, &f.trash, true, &[]).unwrap();
        assert_eq!(report.failed.len(), 1);
        let kept = load(&f.file, &f.game).unwrap().queue;
        assert_eq!(kept.len(), 1);
        assert!(kept.changes[0].error.is_some());
        assert!(f.addons.join("pak10_dir.vpk").is_file());
        // Once the cause is gone the same change applies.
        fs::remove_file(&f.trash).unwrap();
        let report = apply_all_with(&f.file, &f.game, &f.trash, true, &[]).unwrap();
        assert_eq!(report.applied.len(), 1);
    }

    #[test]
    fn a_locked_file_is_tried_again_after_each_pause_and_other_errors_are_not() {
        let locked = || Outcome::Failed(format!("x could not be moved. {}", crate::error::IN_USE_HINT));
        let pauses = [Duration::ZERO; 3];
        let mut tries = 0;
        let outcome = with_retries(&pauses, || {
            tries += 1;
            if tries < 3 { locked() } else { Outcome::Skipped("done".into()) }
        });
        assert_eq!(tries, 3);
        assert!(matches!(outcome, Outcome::Skipped(_)));
        // Still locked after every pause: one try plus three retries, then it fails.
        let mut tries = 0;
        let outcome = with_retries(&pauses, || {
            tries += 1;
            locked()
        });
        assert_eq!(tries, 4);
        assert!(matches!(outcome, Outcome::Failed(_)));
        // Any other failure is final at once.
        let mut tries = 0;
        with_retries(&pauses, || {
            tries += 1;
            Outcome::Failed("Slot 04 is not installed.".into())
        });
        assert_eq!(tries, 1);
        assert_eq!(RETRY_BACKOFF.map(|d| d.as_secs()), [1, 3, 8]);
    }

    #[test]
    fn the_in_use_hint_marks_a_lock() {
        let held = Error::Io(std::io::Error::from(std::io::ErrorKind::ResourceBusy));
        assert!(held.is_in_use());
        assert!(crate::error::mentions_in_use(&format!("x {}", crate::error::IN_USE_HINT)));
        assert!(!crate::error::mentions_in_use("Slot 04 is not installed."));
    }

    /// Player actions only: the agent host and the CLI never reach this queue.
    #[test]
    fn no_agent_tool_or_cli_command_uses_the_queue() {
        let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        for name in ["agent-host", "dah-rs", "tinyagent"] {
            let mut stack = vec![crates.join(name).join("src")];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e == "rs") {
                        let text = fs::read_to_string(&path).unwrap_or_default();
                        assert!(
                            !text.contains("deadlock_core::pending") && !text.contains(PENDING_FILE),
                            "{} reaches the waiting-changes queue",
                            path.display()
                        );
                    }
                }
            }
        }
    }
}
