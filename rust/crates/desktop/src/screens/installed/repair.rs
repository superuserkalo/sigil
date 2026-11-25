//! Compatibility repairs after a game update, in the Compatibility review. Both reference apps
//! repair: Grimoire runs vpkmerge's compatibility and attachment heals on the mod file, with a
//! backup, a verify step, a split-archive refusal and an animation-override question; Deadlock
//! Mod Manager writes fixes into a separate package, can fix before each launch, and reverts.
//! Sigil takes Grimoire's in-place repair with its guard (`deadlock_core::compat_repair`), DMM's
//! "Fix mods automatically before launch" switch, and a Revert for every repair.
//!
//! - The review's worker also runs the dry run (`compat_repair::check`, data and cameras) over
//!   every enabled mod, and notes which enabled mods ship animation files.
//! - Repair, Fix all and Revert run on a worker through `apply_repair` and `revert_repair`, the
//!   core's approved app actions: a backup under `<data>/compat-backups`, staging under
//!   `<data>/staging/compat`, never a partial file in the game folder.
//! - When another enabled mod ships animations, the player is asked first and the repair keeps
//!   their overrides (`animation_overrides`).
//! - After a repair the slot's file has a new size, so the library and the notes are moved to
//!   it (`mod_extras::Extras::rekey`), and the backup and the repaired hash are kept in
//!   `<data>/compat-repairs.json` for Revert.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use deadlock_core::compat_repair::{self, AppliedRepair, RepairFinding, RepairKind};
use deadlock_core::{import, mod_extras};
use rune::{El, Tx, col, row, text};
use serde_json::{Value, json};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY_MED, MICRO, SMALL, Ui};

use super::display_name;

/// Both repair families, in the order Grimoire runs them.
pub const KINDS: [RepairKind; 2] = [RepairKind::Data, RepairKind::Cameras];

/// One repair Revert can undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub slot: u16,
    pub file: String,
    pub backup: PathBuf,
    pub new_sha256: String,
    pub old_size: u64,
    pub new_size: u64,
    pub changes: usize,
    /// The repair rules' revision that made this file. An older one is rebuilt from its backup
    /// when the app updates, as Deadlock Mod Manager does with its repair revision.
    pub rule: u32,
}

/// An archive as the dry run read it. The review for a slot is kept against this stamp: a
/// different size or hash means the archive changed, so its findings and its approval are
/// reset (DMM's saved review is keyed the same way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub sha256: String,
    pub size: u64,
}

/// What the review's dry run found.
#[derive(Default)]
pub struct Checked {
    /// Repairable findings by slot.
    pub findings: BTreeMap<u16, Vec<RepairFinding>>,
    /// Enabled slots that ship animation files.
    pub animations: BTreeSet<u16>,
    /// Slots the dry run could not read (a split archive, a broken file), and why.
    pub errors: BTreeMap<u16, String>,
    /// The archive each slot had when the dry run read it, by SHA-256 and size.
    pub stamps: BTreeMap<u16, Stamp>,
}

/// One result of a repair batch.
pub enum Done {
    Repaired { slot: u16, file: String, applied: AppliedRepair },
    Reverted(Record),
    Nothing(u16),
    Failed(u16, String),
}

#[derive(Default)]
pub struct Repairs {
    pub records: Vec<Record>,
    pub loaded: bool,
    pub checked: Option<Checked>,
    pub check: Job<Checked>,
    pub work: Job<Vec<Done>>,
    /// Slots waiting for the animation question.
    pub ask: Option<Vec<u16>>,
    /// A health review is open and should be read again when the running rebuild finishes.
    pub refresh_health: bool,
}

impl Record {
    fn to_json(&self) -> Value {
        json!({
            "slot": self.slot,
            "file": self.file,
            "backup": self.backup,
            "newSha256": self.new_sha256,
            "oldSize": self.old_size,
            "newSize": self.new_size,
            "changes": self.changes,
            "rule": self.rule,
        })
    }

    fn from_json(v: &Value) -> Option<Record> {
        Some(Record {
            slot: u16::try_from(v.get("slot")?.as_u64()?).ok()?,
            file: v.get("file")?.as_str()?.to_string(),
            // Rebased: a record from before the data folder's rename names the old folder.
            backup: deadlock_core::app_data::rebase(Path::new(v.get("backup")?.as_str()?)),
            new_sha256: v.get("newSha256")?.as_str()?.to_string(),
            old_size: v.get("oldSize")?.as_u64()?,
            new_size: v.get("newSize")?.as_u64()?,
            changes: v.get("changes").and_then(Value::as_u64).unwrap_or(0) as usize,
            // Records written before the revision existed were made by revision 1.
            rule: v
                .get("rule")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(1),
        })
    }
}

fn records_path() -> PathBuf {
    crate::data::data_dir().join("compat-repairs.json")
}

fn backup_root() -> PathBuf {
    crate::data::data_dir().join("compat-backups")
}

/// The repaired-to-original map the core keeps beside the backups (C3), read when the update
/// check loads and after each repair.
pub fn aliases() -> Vec<deadlock_core::compat_repair::RepairAlias> {
    if rune::is_headless() || cfg!(test) {
        return Vec::new();
    }
    deadlock_core::compat_repair::read_repair_aliases(&backup_root())
}

fn staging() -> PathBuf {
    crate::data::data_dir().join("staging").join("compat")
}

fn load_records() -> Vec<Record> {
    std::fs::read_to_string(records_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
        .map(|rows| rows.iter().filter_map(Record::from_json).collect())
        .unwrap_or_default()
}

fn save_records(records: &[Record]) {
    if rune::is_headless() || cfg!(test) {
        return;
    }
    let path = records_path();
    let partial = path.with_extension("json.tmp");
    let rows: Vec<Value> = records.iter().map(Record::to_json).collect();
    let text = serde_json::to_string_pretty(&rows).unwrap_or_default();
    if std::fs::write(&partial, text).is_ok() && std::fs::rename(&partial, &path).is_err() {
        let _ = std::fs::remove_file(&partial);
    }
}

/// Whether repairing `slot` meets another enabled mod's animations.
pub fn meets_animations(animations: &BTreeSet<u16>, slot: u16) -> bool {
    animations.iter().any(|other| *other != slot)
}

/// Enabled slot files, as `(slot, file, path)`.
fn enabled_files(st: &State) -> Vec<(u16, String, PathBuf)> {
    let dir = super::addons_folder(st);
    st.addons
        .addons
        .iter()
        .filter(|a| a.enabled)
        .map(|a| (a.slot, a.file.clone(), a.path(&dir)))
        .collect()
}

/// The dry run, started with the review.
pub fn start_check(st: &mut State) {
    if !st.installed.repairs.loaded && !st.demo {
        st.installed.repairs.records = load_records();
        st.installed.repairs.loaded = true;
    }
    if st.demo || rune::is_headless() || st.installed.repairs.check.is_running() || st.installed.repairs.work.is_running() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let files = enabled_files(st);
    let wake = st.wake.clone();
    st.installed.repairs.check.start(wake, move || Ok(dry_run(&game_root, &files)));
}

/// Read-only analysis workers for the full recheck (DMM PR #1386): every mod's animations are
/// learned first, then two workers check the mods side by side.
pub const CHECK_WORKERS: usize = 2;

fn dry_run(game_root: &Path, files: &[(u16, String, PathBuf)]) -> Checked {
    let base = deadlock_core::game_index::game_vpk_path(game_root);
    let mut out = Checked::default();
    for (slot, _, path) in files {
        if compat_repair::has_animation_overrides(std::slice::from_ref(path)) {
            out.animations.insert(*slot);
        }
    }
    let animations = out.animations.clone();
    let results = analyse_parallel(files, CHECK_WORKERS, |slot, path| check_one(&base, slot, path, &animations));
    for (slot, result) in results {
        match result {
            Ok((found, stamp)) => {
                out.stamps.insert(slot, stamp);
                if !found.is_empty() {
                    out.findings.insert(slot, found);
                }
            }
            Err((error, stamp)) => {
                if let Some(stamp) = stamp {
                    out.stamps.insert(slot, stamp);
                }
                out.errors.insert(slot, error);
            }
        }
    }
    out
}

/// Both repair families' repairable findings for one mod, with the archive's stamp, or the
/// first error and the stamp when the file was readable.
fn check_one(
    base: &Path,
    slot: u16,
    path: &Path,
    animations: &BTreeSet<u16>,
) -> std::result::Result<(Vec<RepairFinding>, Stamp), (String, Option<Stamp>)> {
    let stamp = std::fs::metadata(path)
        .map_err(|error| (error.to_string(), None))
        .and_then(|meta| {
            deadlock_core::identify::sha256_file(path)
                .map(|sha256| Stamp { sha256, size: meta.len() })
                .map_err(|error| (error.to_string(), None))
        })?;
    let overrides = meets_animations(animations, slot);
    let mut found = Vec::new();
    for kind in KINDS {
        let findings = compat_repair::check(base, path, kind, overrides)
            .map_err(|error| (error.to_string(), Some(stamp.clone())))?;
        found.extend(findings.into_iter().filter(|f| f.repairable));
    }
    Ok((found, stamp))
}

/// Run `check` over the files on `workers` threads, each taking the next file from a shared
/// cursor, and return the results in slot order.
pub fn analyse_parallel<T, F>(files: &[(u16, String, PathBuf)], workers: usize, check: F) -> Vec<(u16, T)>
where
    T: Send,
    F: Fn(u16, &Path) -> T + Sync,
{
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::with_capacity(files.len()));
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, files.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((slot, _, path)) = files.get(index) else { break };
                    let result = check(*slot, path);
                    if let Ok(mut results) = results.lock() {
                        results.push((*slot, result));
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_default();
    results.sort_by_key(|(slot, _)| *slot);
    results
}

/// Repair these slots, asking first when another enabled mod ships animations.
pub fn ask_repair(st: &mut State, slots: Vec<u16>) {
    if slots.is_empty() {
        return;
    }
    let animations = st.installed.repairs.checked.as_ref().map(|c| c.animations.clone()).unwrap_or_default();
    if slots.iter().any(|slot| meets_animations(&animations, *slot)) {
        st.installed.repairs.ask = Some(slots);
        return;
    }
    run_repair(st, slots);
}

/// The approved repair: one worker pass over the slots.
pub fn run_repair(st: &mut State, slots: Vec<u16>) {
    if crate::screens::installed::waiting::refuse_held(st, crate::screens::installed::waiting::HELD_REWRITE) {
        return;
    }
    st.installed.repairs.ask = None;
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) || st.installed.repairs.work.is_running() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        st.status = tr("Set the game folder in Settings first.").to_string();
        return;
    };
    let animations = st.installed.repairs.checked.as_ref().map(|c| c.animations.clone()).unwrap_or_default();
    let files: Vec<(u16, String, PathBuf)> = enabled_files(st).into_iter().filter(|(slot, _, _)| slots.contains(slot)).collect();
    let wake = st.wake.clone();
    st.installed.repairs.work.start(wake, move || Ok(repair_all(&game_root, &files, &animations)));
}

fn repair_all(game_root: &Path, files: &[(u16, String, PathBuf)], animations: &BTreeSet<u16>) -> Vec<Done> {
    let base = deadlock_core::game_index::game_vpk_path(game_root);
    files
        .iter()
        .map(|(slot, file, path)| repair_slot(&base, *slot, file, path, meets_animations(animations, *slot)))
        .collect()
}

/// Repair one slot's file. `overrides` asks the data heal to keep another mod's animations.
fn repair_slot(base: &Path, slot: u16, file: &str, path: &Path, overrides: bool) -> Done {
    match compat_repair::apply_repair(base, path, &KINDS, overrides, &backup_root(), &staging(), true) {
        Ok(Some(applied)) => Done::Repaired { slot, file: file.to_string(), applied },
        Ok(None) => Done::Nothing(slot),
        Err(error) => Done::Failed(slot, error.to_string()),
    }
}

/// Rebuild repairs whose archive changed or whose rules are older than this build's.
///
/// `tasks` is `(slot, revert_first)`. A changed archive (an update, a reinstall, the player
/// replacing the file while Deadlock ran) is repaired in place: the standing repair is the
/// approval. A stale rule revision goes back to its backup first, so the new rule sees the
/// original mod rather than the old repair's output.
fn start_rebuild(st: &mut State, tasks: Vec<(u16, bool)>, records: BTreeMap<u16, Record>) {
    let Some(game_root) = st.game_root.clone() else { return };
    let dir = super::addons_folder(st);
    let files: Vec<(u16, String, PathBuf)> = st
        .addons
        .addons
        .iter()
        .map(|addon| (addon.slot, addon.file.clone(), addon.path(&dir)))
        .collect();
    let active: BTreeSet<u16> = st.addons.addons.iter().filter(|addon| addon.enabled).map(|addon| addon.slot).collect();
    let animations: BTreeSet<u16> = files
        .iter()
        .filter(|(slot, _, path)| active.contains(slot) && compat_repair::has_animation_overrides(std::slice::from_ref(path)))
        .map(|(slot, _, _)| *slot)
        .collect();
    let wake = st.wake.clone();
    st.installed.repairs.work.start(wake, move || {
        let base = deadlock_core::game_index::game_vpk_path(&game_root);
        Ok(tasks
            .into_iter()
            .map(|(slot, revert_first)| {
                let Some((_, file, path)) = files.iter().find(|(own, _, _)| *own == slot).cloned() else {
                    return Done::Failed(slot, "The mod's file is gone.".to_string());
                };
                if revert_first {
                    let Some(record) = records.get(&slot) else {
                        return Done::Failed(slot, "The repair's backup is not on record.".to_string());
                    };
                    if let Err(error) = compat_repair::revert_repair(&record.backup, &path, &record.new_sha256, true) {
                        return Done::Failed(slot, error.to_string());
                    }
                }
                repair_slot(&base, slot, &file, &path, meets_animations(&animations, slot))
            })
            .collect())
    });
}

/// Revert one repair from its backup.
pub fn revert(st: &mut State, record: Record) {
    if crate::screens::installed::waiting::refuse_held(st, crate::screens::installed::waiting::HELD_REWRITE) {
        return;
    }
    if st.demo {
        st.status = tr("Demo library: nothing was changed.").to_string();
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) || st.installed.repairs.work.is_running() {
        return;
    }
    let path = deadlock_core::addons::slot_folder(&super::addons_folder(st), record.slot).join(&record.file);
    let wake = st.wake.clone();
    st.installed.repairs.work.start(wake, move || {
        Ok(vec![match compat_repair::revert_repair(&record.backup, &path, &record.new_sha256, true) {
            Ok(()) => Done::Reverted(record),
            Err(error) => Done::Failed(record.slot, error.to_string()),
        }])
    });
}

/// Move the library entry and the notes of a slot from one file size to another.
fn rekey(st: &mut State, slot: u16, old_size: u64, new_size: u64) {
    let library = super::library_file();
    if let Ok(mut lib) = import::read_library(&library) {
        if let Some(meta) = lib.mods.iter_mut().rev().find(|m| m.slot == slot && m.size_bytes == old_size) {
            meta.size_bytes = new_size;
        }
        let _ = import::write_library(&library, &lib);
    }
    let _ = mod_extras::edit_extras(&super::extras_path(), |x| x.rekey(slot, old_size, new_size));
    st.installed.notes.extras.rekey(slot, old_size, new_size);
}

/// Apply a batch's results: records, sizes, one recap line. Shared with the launch step.
pub fn finish(st: &mut State, done: Vec<Done>) {
    st.installed.updates.aliases = aliases();
    let (mut fixed, mut reverted, mut failed) = (0usize, 0usize, Vec::new());
    for item in done {
        match item {
            Done::Repaired { slot, file, applied } => {
                crate::screens::settings::product::repair(st, slot, true);
                rekey(st, slot, applied.old_size, applied.new_size);
                let changes = applied.findings.iter().map(|f| f.changes.max(1)).sum();
                st.installed.repairs.records.retain(|r| r.slot != slot);
                st.installed.repairs.records.push(Record {
                    slot,
                    file,
                    backup: applied.backup,
                    new_sha256: applied.new_sha256,
                    old_size: applied.old_size,
                    new_size: applied.new_size,
                    changes,
                    rule: compat_repair::REPAIR_RULE,
                });
                if let Some(c) = st.installed.repairs.checked.as_mut() {
                    c.findings.remove(&slot);
                }
                fixed += 1;
            }
            Done::Reverted(record) => {
                rekey(st, record.slot, record.new_size, record.old_size);
                st.installed.repairs.records.retain(|r| r != &record);
                reverted += 1;
            }
            Done::Nothing(slot) => {
                if let Some(c) = st.installed.repairs.checked.as_mut() {
                    c.findings.remove(&slot);
                }
            }
            Done::Failed(slot, error) => {
                crate::screens::settings::product::repair(st, slot, false);
                failed.push(format!("{slot:02}: {error}"));
            }
        }
    }
    save_records(&st.installed.repairs.records);
    if fixed > 0 {
        crate::screens::settings::telemetry::event(st, "repair:apply", Some("page:installed"));
    }
    let mut line = Vec::new();
    if fixed > 0 {
        line.push(if fixed == 1 { tr("Repaired 1 mod.").to_string() } else { fill(tr("Repaired {} mods."), &[&fixed.to_string()]) });
    }
    if reverted > 0 {
        line.push(tr("Put the original file back.").to_string());
    }
    if !failed.is_empty() {
        // One toast: the failure, with what did work in front of the reasons (issue #4).
        line.push(failed.join("\n"));
        crate::notify::push(
            st,
            crate::notify::Notice::error(fill(tr("{} mods could not be repaired"), &[&failed.len().to_string()]))
                .detail(line.join(" ")),
        );
    } else if !line.is_empty() {
        st.status = line.join(" ");
    }
    super::rescan(st);
}

/// Take finished work, and reconcile repairs with the files on disk. Called from Installed's
/// poll.
///
/// Two things bring a repair back after its file changed: the app's rule revision is newer
/// than the one a record was made under (an app update; the record goes back to its backup and
/// is repaired again), or the slot's archive is no longer the file the record's SHA-256 names
/// (a mod update, a reinstall, Steam verifying, or a file the player changed while Deadlock
/// ran). The second waits until the game is closed, so a repair never writes while it reads.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if !st.installed.repairs.loaded && !st.demo {
        st.installed.repairs.records = load_records();
        st.installed.repairs.loaded = true;
    }
    if let Some(result) = st.installed.repairs.check.try_take() {
        st.installed.repairs.checked = result.ok();
        changed = true;
    }
    if let Some(result) = st.installed.repairs.work.try_take() {
        match result {
            Ok(done) => finish(st, done),
            Err(error) => st.status = error,
        }
        if std::mem::take(&mut st.installed.repairs.refresh_health) && st.installed.tools.health.is_some() {
            super::tools::open_health(st);
        }
        changed = true;
    }
    changed |= reconcile(st);
    changed
}

/// Reset review findings for archives that changed, and rebuild repairs whose archive changed
/// or whose rule revision is old. Returns whether anything was started.
fn reconcile(st: &mut State) -> bool {
    let enabled: BTreeSet<u16> = st.addons.addons.iter().filter(|addon| addon.enabled).map(|addon| addon.slot).collect();
    let sizes: BTreeMap<u16, u64> = st.addons.addons.iter().map(|addon| (addon.slot, addon.size_bytes)).collect();
    let mut recheck = false;
    if let Some(checked) = st.installed.repairs.checked.as_mut() {
        let stale: Vec<u16> = checked
            .stamps
            .iter()
            .filter(|(slot, stamp)| !enabled.contains(*slot) || sizes.get(*slot) != Some(&stamp.size))
            .map(|(slot, _)| *slot)
            .collect();
        if !stale.is_empty() {
            for slot in stale {
                checked.stamps.remove(&slot);
                checked.findings.remove(&slot);
                checked.errors.remove(&slot);
            }
            recheck = true;
        }
    }

    let running = st.settings.game_running || st.launch.monitor.running();
    let can_rebuild = !st.demo
        && !rune::is_headless()
        && !running
        && !st.installed.repairs.work.is_running()
        && st.game_root.is_some();
    let mut tasks: Vec<(u16, bool)> = Vec::new();
    if can_rebuild {
        for record in &st.installed.repairs.records {
            match sizes.get(&record.slot) {
                None => {}
                Some(size) if *size != record.new_size => tasks.push((record.slot, false)),
                Some(_) if record.rule < compat_repair::REPAIR_RULE => tasks.push((record.slot, true)),
                Some(_) => {}
            }
        }
    }
    if !tasks.is_empty() {
        let records: BTreeMap<u16, Record> = st.installed.repairs.records.iter().map(|record| (record.slot, record.clone())).collect();
        st.installed.repairs.records.retain(|record| {
            let current = sizes.get(&record.slot).copied();
            current == Some(record.new_size) && record.rule >= compat_repair::REPAIR_RULE
        });
        save_records(&st.installed.repairs.records);
        if st.installed.tools.health.is_some() {
            st.installed.repairs.refresh_health = true;
        }
        start_rebuild(st, tasks, records);
        return true;
    }
    if recheck && !running && !st.demo && !rune::is_headless() {
        start_check(st);
        return true;
    }
    false
}

/// A mod was updated or reinstalled: its repair record names the file that was there before, so
/// drop it, recheck the new file, and rebuild the repair when the mod had one. The health review
/// is refreshed right away when it is open and nothing is being rebuilt (DMM rechecks after an
/// installed file changes).
pub fn after_update(st: &mut State, slot: u16) {
    if st.demo || rune::is_headless() {
        return;
    }
    let records: BTreeMap<u16, Record> = st.installed.repairs.records.iter().map(|record| (record.slot, record.clone())).collect();
    let repaired = records.contains_key(&slot);
    st.installed.repairs.records.retain(|record| record.slot != slot);
    save_records(&st.installed.repairs.records);
    if let Some(checked) = st.installed.repairs.checked.as_mut() {
        checked.stamps.remove(&slot);
        checked.findings.remove(&slot);
        checked.errors.remove(&slot);
    }
    let running = st.settings.game_running || st.launch.monitor.running();
    let rebuild = repaired && !running && !st.installed.repairs.work.is_running() && st.game_root.is_some();
    if rebuild {
        if st.installed.tools.health.is_some() {
            st.installed.repairs.refresh_health = true;
        }
        start_rebuild(st, vec![(slot, false)], records);
    } else {
        start_check(st);
        if st.installed.tools.health.is_some() {
            super::tools::open_health(st);
        }
    }
}

/// The launch step for "Fix mods automatically before launch": repair every enabled mod that
/// needs it, on a worker. The option is the player's standing approval.
pub fn start_before_launch(st: &mut State) -> bool {
    if !crate::screens::settings::options::get().repair_before_launch || st.demo || rune::is_headless() {
        return false;
    }
    if st.installed.repairs.work.is_running() {
        return true;
    }
    let Some(game_root) = st.game_root.clone() else { return false };
    let files = enabled_files(st);
    let wake = st.wake.clone();
    st.installed.repairs.work.start(wake, move || {
        let animations: BTreeSet<u16> = files
            .iter()
            .filter(|(_, _, path)| compat_repair::has_animation_overrides(std::slice::from_ref(path)))
            .map(|(slot, _, _)| *slot)
            .collect();
        Ok(repair_all(&game_root, &files, &animations))
    });
    true
}

// --- copyable diagnostics ----------------------------------------------------------------

/// One mod's part of the diagnostics: its path findings, what a repair would change, why it
/// could not be read, and whether it was repaired. English on purpose: the text goes to mod
/// authors and bug reports, like Send diagnostics.
pub fn mod_text(
    name: &str,
    slot: u16,
    review: Option<&deadlock_core::mod_health::SlotReview>,
    checked: Option<&Checked>,
    record: Option<&Record>,
) -> String {
    let mut out = format!("[{}] {name}\n", deadlock_core::addons::slot_label(slot));
    if let Some(review) = review {
        if let Some(error) = &review.error {
            out.push_str(&format!("  unreadable: {error}\n"));
        }
        if !review.issues.is_empty() {
            out.push_str(&format!("  {} of {} paths not in the game\n", review.missing, review.entries));
        }
        for issue in &review.issues {
            match &issue.now_at {
                Some(now) => out.push_str(&format!("  {}: {} (now {now})\n", issue.kind.label(), issue.path)),
                None => out.push_str(&format!("  {}: {}\n", issue.kind.label(), issue.path)),
            }
        }
        if review.more > 0 {
            out.push_str(&format!("  and {} more\n", review.more));
        }
    }
    if let Some(checked) = checked {
        for finding in checked.findings.get(&slot).into_iter().flatten() {
            out.push_str(&format!("  repair {}: {} ({} changes)", finding.kind, finding.entry, finding.changes.max(1)));
            if !finding.detail.is_empty() {
                out.push_str(&format!(": {}", finding.detail));
            }
            out.push('\n');
        }
        if let Some(error) = checked.errors.get(&slot) {
            out.push_str(&format!("  repair check failed: {error}\n"));
        }
        if checked.animations.contains(&slot) {
            out.push_str("  ships animation files\n");
        }
    }
    if let Some(record) = record {
        out.push_str(&format!(
            "  repaired: {} changes, {} -> {} bytes, sha256 {}\n",
            record.changes, record.old_size, record.new_size, record.new_sha256
        ));
    }
    out
}

/// `ClientVersion` from the game's `steam.inf`, read when the player presses Copy.
fn client_version(game_root: Option<&Path>) -> Option<String> {
    let text = std::fs::read_to_string(game_root?.join("game").join("citadel").join("steam.inf")).ok()?;
    text.lines()
        .find_map(|line| line.trim().strip_prefix("ClientVersion=").map(|v| v.trim().to_string()))
}

/// The whole review as text (DMM 2.4.0's copyable compatibility diagnostics): app and game
/// build, then every mod with a finding, a repair or an error.
pub fn review_text(s: &State) -> String {
    let reviews: &[deadlock_core::mod_health::SlotReview] =
        s.installed.tools.health.as_ref().map(|h| h.reviews.as_slice()).unwrap_or(&[]);
    let checked = s.installed.repairs.checked.as_ref();
    let mut slots: BTreeSet<u16> = reviews.iter().filter(|r| !r.is_clear()).map(|r| r.slot).collect();
    if let Some(c) = checked {
        slots.extend(c.findings.keys().copied());
        slots.extend(c.errors.keys().copied());
    }
    slots.extend(s.installed.repairs.records.iter().map(|r| r.slot));
    let version = env!("CARGO_PKG_VERSION");
    let build = client_version(s.game_root.as_deref()).unwrap_or_else(|| "unknown".to_string());
    let count = s.installed.tools.health.as_ref().map(|h| h.checked).unwrap_or(0);
    let mut out = format!("Sigil {version} compatibility review\nGame build: {build}\nMods checked: {count}\n");
    if slots.is_empty() {
        out.push_str("(nothing to report)\n");
    }
    for slot in slots {
        out.push('\n');
        out.push_str(&slot_text(s, slot));
    }
    out
}

/// [`mod_text`] for one slot of the live state.
pub fn slot_text(s: &State, slot: u16) -> String {
    let name = s
        .addons
        .addons
        .iter()
        .find(|a| a.slot == slot)
        .map(|a| display_name(s, a))
        .unwrap_or_else(|| {
            let label = deadlock_core::addons::slot_label(slot);
            format!("Slot {label}")
        });
    let review = s.installed.tools.health.as_ref().and_then(|h| h.reviews.iter().find(|r| r.slot == slot));
    let record = s.installed.repairs.records.iter().find(|r| r.slot == slot);
    mod_text(&name, slot, review, s.installed.repairs.checked.as_ref(), record)
}

/// Put `text` on the clipboard and say so.
pub fn copy(st: &mut State, tx: &mut Tx, text: String) {
    tx.copy(text);
    crate::notify::push(st, crate::notify::Notice::info(tr("Copied the details to the clipboard")));
}

/// The Copy details button for the whole review, in the review's footer.
pub fn copy_all(u: &Ui) -> El {
    u.icon_action(&icons::COPY, tr("Copy details"), ActionKind::Ghost)
        .key("health-copy")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|st: &mut State, tx: &mut Tx| {
            let text = review_text(st);
            copy(st, tx, text);
        })
}

/// A small copy button for one mod's row.
fn copy_one(u: &Ui, slot: u16, key: &str) -> El {
    u.icon_button(&icons::COPY, tr("Copy details"))
        .key(format!("{key}-{slot:02}"))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
            let text = slot_text(st, slot);
            copy(st, tx, text);
        })
}

// --- view --------------------------------------------------------------------------------

/// The repairs block in the review: each fixable mod with Repair, each repaired mod with Revert,
/// the animation question, and Fix all.
pub fn block(s: &State, u: &Ui) -> Option<El> {
    let r = &s.installed.repairs;
    let busy = r.work.is_running();
    let mut list = col().w_full().gap(4.0);
    let mut fixable = Vec::new();
    if let Some(checked) = &r.checked {
        for (slot, findings) in &checked.findings {
            let Some(addon) = s.addons.addons.iter().find(|a| a.slot == *slot) else { continue };
            fixable.push(*slot);
            let changes: usize = findings.iter().map(|f| f.changes.max(1)).sum();
            let families: BTreeSet<&str> = findings.iter().map(|f| f.kind.as_str()).collect();
            let slot = *slot;
            list = list.child(
                row()
                    .key(format!("repair-row-{slot:02}"))
                    .w_full()
                    .h(36.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(text(deadlock_core::addons::slot_label(slot)).font(MICRO).color(u.faint).w(28.0))
                    .child(text(display_name(s, addon)).font(BODY_MED).color(u.ink).truncate().grow(1.0).min_w(0.0))
                    .child(
                        text({
                            let kinds = families.into_iter().collect::<Vec<_>>().join(", ");
                            if changes == 1 {
                                fill(tr("1 fix: {}"), &[&kinds])
                            } else {
                                fill(tr("{} fixes: {}"), &[&changes.to_string(), &kinds])
                            }
                        })
                            .font(SMALL)
                            .color(u.amber),
                    )
                    .child(copy_one(u, slot, "repair-copy"))
                    .child(
                        u.action(tr("Repair"), ActionKind::Secondary)
                            .key(format!("repair-one-{slot:02}"))
                            .disabled(busy)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| ask_repair(st, vec![slot])),
                    ),
            );
        }
        for (slot, error) in &checked.errors {
            list = list.child(
                row()
                    .w_full()
                    .py(4.0)
                    .gap(8.0)
                    .child(text(deadlock_core::addons::slot_label(*slot)).font(MICRO).color(u.faint).w(28.0))
                    .child(text(error.clone()).font(SMALL).color(u.faint).wrap().grow(1.0).min_w(0.0))
                    .child(copy_one(u, *slot, "repair-error-copy")),
            );
        }
    }
    for record in &r.records {
        let Some(addon) = s.addons.addons.iter().find(|a| a.slot == record.slot && a.size_bytes == record.new_size) else { continue };
        let rec = record.clone();
        list = list.child(
            row()
                .key(format!("repair-done-{:02}", record.slot))
                .w_full()
                .h(36.0)
                .gap(8.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(deadlock_core::addons::slot_label(record.slot)).font(MICRO).color(u.faint).w(28.0))
                .child(text(display_name(s, addon)).font(BODY_MED).color(u.ink).truncate().grow(1.0).min_w(0.0))
                .child(text(tr("Repaired")).font(SMALL).color(u.teal))
                .child(copy_one(u, record.slot, "repair-done-copy"))
                .child(
                    u.action(tr("Revert"), ActionKind::Ghost)
                        .key(format!("repair-revert-{:02}", record.slot))
                        .disabled(busy)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| revert(st, rec.clone())),
                ),
        );
    }
    let checking = r.check.is_running();
    if !checking && fixable.is_empty() && r.records.is_empty() && r.checked.as_ref().is_none_or(|c| c.errors.is_empty()) {
        return None;
    }
    let mut c = col()
        .key("repair-block")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(tr("Repairs")).font(BODY_MED).color(u.ink))
                        .child(
                    text(if checking {
                        tr("Checking each mod against the game's files")
                    } else if busy {
                        tr("Repairing")
                    } else {
                        tr("Old game data, sound bindings and cameras. The original file is kept so you can revert.")
                    })
                    .font(SMALL)
                    .color(u.faint)
                    .wrap(),
                        ),
                )
                .child(if fixable.len() > 1 {
                    let all = fixable.clone();
                    u.icon_action(&icons::WRENCH, &fill(tr("Fix all {}"), &[&fixable.len().to_string()]), ActionKind::Secondary)
                        .key("repair-all")
                        .disabled(busy)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| ask_repair(st, all.clone()))
                } else {
                    rune::el()
                }),
        )
        .child(list);
    if let Some(slots) = r.ask.clone() {
        c = c.child(
            row()
                .key("repair-ask")
                .w_full()
                .padding(10.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .border(1.0, u.amber.alpha(0.45))
                .child(
                    text(tr("Another mod you have on changes animations. The repair keeps its animations where it can, but a hero can still move oddly. Repair anyway?"))
                        .font(SMALL)
                        .color(u.prose)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key("repair-ask-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.installed.repairs.ask = None),
                )
                .child(
                    u.action(tr("Repair"), ActionKind::Primary)
                        .key("repair-ask-yes")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| run_repair(st, slots.clone())),
                ),
        );
    }
    Some(c)
}

/// Scenes: two fixable mods, one with a camera fix, one repaired mod, and another mod's
/// animations so Fix all asks first.
pub fn stage_scene(st: &mut State, asking: bool) {
    let finding = |entry: &str, kind: &str, changes: usize| RepairFinding {
        entry: entry.to_string(),
        kind: kind.to_string(),
        repairable: true,
        changes,
        detail: String::new(),
    };
    let mut checked = Checked::default();
    checked.findings.insert(2, vec![finding("scripts/heroes.vdata_c", "data", 4), finding("sounds/footsteps.vsndevts_c", "sounds", 2)]);
    checked.findings.insert(9, vec![finding("models/heroes_wip/drifter/drifter.vmdl_c", "cameras", 1)]);
    checked.animations.insert(5);
    checked.errors.insert(7, tr("pak07_dir.vpk could not be read: the file ends early.").to_string());
    st.installed.repairs.checked = Some(checked);
    st.installed.repairs.loaded = true;
    if let Some(addon) = st.addons.addons.iter().find(|a| a.slot == 3) {
        st.installed.repairs.records.push(Record {
            slot: 3,
            file: addon.file.clone(),
            backup: PathBuf::from("compat-backups/scene.vpk"),
            new_sha256: String::new(),
            old_size: addon.size_bytes + 512,
            new_size: addon.size_bytes,
            changes: 3,
            rule: compat_repair::REPAIR_RULE,
        });
    }
    if asking {
        st.installed.repairs.ask = Some(vec![2, 9]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_mods_animations_raise_the_question_but_its_own_do_not() {
        let animations: BTreeSet<u16> = [5].into_iter().collect();
        assert!(meets_animations(&animations, 2));
        assert!(!meets_animations(&animations, 5), "a mod's own animations are not another mod's");
        assert!(!meets_animations(&BTreeSet::new(), 2));
    }

    #[test]
    fn the_recheck_runs_on_two_workers_and_keeps_slot_order() {
        let files: Vec<(u16, String, PathBuf)> =
            [7u16, 2, 9, 4, 1].iter().map(|slot| (*slot, format!("pak{slot:02}_dir.vpk"), PathBuf::from(format!("pak{slot:02}_dir.vpk")))).collect();
        let threads = std::sync::Mutex::new(std::collections::HashSet::new());
        let at_once = std::sync::atomic::AtomicUsize::new(0);
        let peak = std::sync::atomic::AtomicUsize::new(0);
        let results = analyse_parallel(&files, CHECK_WORKERS, |slot, path| {
            use std::sync::atomic::Ordering;
            let now = at_once.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            threads.lock().unwrap().insert(std::thread::current().id());
            std::thread::sleep(std::time::Duration::from_millis(20));
            at_once.fetch_sub(1, Ordering::SeqCst);
            (slot, path.to_path_buf())
        });
        let slots: Vec<u16> = results.iter().map(|(slot, _)| *slot).collect();
        assert_eq!(slots, vec![1, 2, 4, 7, 9]);
        assert!(results.iter().all(|(slot, (same, path))| slot == same && path.to_string_lossy().contains(&format!("{slot:02}"))));
        assert!(peak.load(std::sync::atomic::Ordering::SeqCst) <= CHECK_WORKERS);
        assert_eq!(threads.lock().unwrap().len(), CHECK_WORKERS);
    }

    #[test]
    fn one_mods_text_names_every_finding_and_the_repair() {
        let mut checked = Checked::default();
        checked.findings.insert(
            4,
            vec![RepairFinding {
                entry: "models/heroes_wip/drifter/drifter.vmdl_c".into(),
                kind: "cameras".into(),
                repairable: true,
                changes: 2,
                detail: "Stale camera inputs: standing_pivot".into(),
            }],
        );
        checked.errors.insert(4, "the file ends early".into());
        let review = deadlock_core::mod_health::SlotReview {
            slot: 4,
            issues: vec![deadlock_core::mod_health::Issue {
                kind: deadlock_core::mod_health::IssueKind::Moved,
                path: "sounds/a.vsnd_c".into(),
                now_at: Some("sounds/b.vsnd_c".into()),
            }],
            missing: 1,
            entries: 10,
            ..Default::default()
        };
        let text = mod_text("Drifter skin", 4, Some(&review), Some(&checked), None);
        assert!(text.starts_with("[04] Drifter skin"), "{text}");
        assert!(text.contains("Moved: sounds/a.vsnd_c (now sounds/b.vsnd_c)"), "{text}");
        assert!(text.contains("repair cameras: models/heroes_wip/drifter/drifter.vmdl_c (2 changes): Stale camera inputs"), "{text}");
        assert!(text.contains("repair check failed: the file ends early"), "{text}");
    }

    #[test]
    fn records_round_trip_as_json() {
        let record = Record {
            slot: 4,
            file: "pak04_dir.vpk".into(),
            backup: PathBuf::from("/data/compat-backups/abc.vpk"),
            new_sha256: "abc".into(),
            old_size: 10,
            new_size: 12,
            changes: 3,
            rule: 2,
        };
        assert_eq!(Record::from_json(&record.to_json()), Some(record));
        let older = json!({
            "slot": 4, "file": "pak04_dir.vpk", "backup": "/data/compat-backups/abc.vpk",
            "newSha256": "abc", "oldSize": 10, "newSize": 12, "changes": 3,
        });
        assert_eq!(Record::from_json(&older).map(|record| record.rule), Some(1), "records before the revision were rule 1");
        assert_eq!(Record::from_json(&json!({"slot": 400})), None);
    }
}
