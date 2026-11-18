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
use rune::{El, col, row, text};
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
        })
    }

    fn from_json(v: &Value) -> Option<Record> {
        Some(Record {
            slot: u16::try_from(v.get("slot")?.as_u64()?).ok()?,
            file: v.get("file")?.as_str()?.to_string(),
            backup: PathBuf::from(v.get("backup")?.as_str()?),
            new_sha256: v.get("newSha256")?.as_str()?.to_string(),
            old_size: v.get("oldSize")?.as_u64()?,
            new_size: v.get("newSize")?.as_u64()?,
            changes: v.get("changes").and_then(Value::as_u64).unwrap_or(0) as usize,
        })
    }
}

fn records_path() -> PathBuf {
    crate::data::data_dir().join("compat-repairs.json")
}

fn backup_root() -> PathBuf {
    crate::data::data_dir().join("compat-backups")
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
    if st.demo || rune::is_headless() || st.installed.repairs.check.is_running() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let files = enabled_files(st);
    let wake = st.wake.clone();
    st.installed.repairs.check.start(wake, move || Ok(dry_run(&game_root, &files)));
}

fn dry_run(game_root: &Path, files: &[(u16, String, PathBuf)]) -> Checked {
    let base = deadlock_core::game_index::game_vpk_path(game_root);
    let mut out = Checked::default();
    for (slot, _, path) in files {
        if compat_repair::has_animation_overrides(std::slice::from_ref(path)) {
            out.animations.insert(*slot);
        }
    }
    for (slot, _, path) in files {
        let overrides = meets_animations(&out.animations, *slot);
        let mut found = Vec::new();
        for kind in KINDS {
            match compat_repair::check(&base, path, kind, overrides) {
                Ok(findings) => found.extend(findings.into_iter().filter(|f| f.repairable)),
                Err(error) => {
                    out.errors.insert(*slot, error.to_string());
                    break;
                }
            }
        }
        if !found.is_empty() {
            out.findings.insert(*slot, found);
        }
    }
    out
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
        .map(|(slot, file, path)| {
            match compat_repair::apply_repair(
                &base,
                path,
                &KINDS,
                meets_animations(animations, *slot),
                &backup_root(),
                &staging(),
                true,
            ) {
                Ok(Some(applied)) => Done::Repaired { slot: *slot, file: file.clone(), applied },
                Ok(None) => Done::Nothing(*slot),
                Err(error) => Done::Failed(*slot, error.to_string()),
            }
        })
        .collect()
}

/// Revert one repair from its backup.
pub fn revert(st: &mut State, record: Record) {
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
    let (mut fixed, mut reverted, mut failed) = (0usize, 0usize, Vec::new());
    for item in done {
        match item {
            Done::Repaired { slot, file, applied } => {
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
            Done::Failed(slot, error) => failed.push(format!("{slot:02}: {error}")),
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
        crate::notify::push(
            st,
            crate::notify::Notice::error(fill(tr("{} mods could not be repaired"), &[&failed.len().to_string()]))
                .detail(failed.join("\n")),
        );
    }
    if !line.is_empty() {
        st.status = line.join(" ");
    }
    super::rescan(st);
}

/// Take finished work. Called from Installed's poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.installed.repairs.check.try_take() {
        st.installed.repairs.checked = result.ok();
        changed = true;
    }
    if let Some(result) = st.installed.repairs.work.try_take() {
        match result {
            Ok(done) => finish(st, done),
            Err(error) => st.status = error,
        }
        changed = true;
    }
    changed
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
                    .child(text(format!("{slot:02}")).font(MICRO).color(u.faint).w(20.0))
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
                    .child(text(format!("{slot:02}")).font(MICRO).color(u.faint).w(20.0))
                    .child(text(error.clone()).font(SMALL).color(u.faint).wrap().grow(1.0).min_w(0.0)),
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
                .child(text(format!("{:02}", record.slot)).font(MICRO).color(u.faint).w(20.0))
                .child(text(display_name(s, addon)).font(BODY_MED).color(u.ink).truncate().grow(1.0).min_w(0.0))
                .child(text(tr("Repaired")).font(SMALL).color(u.teal))
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
    checked.errors.insert(7, tr("This mod is a split archive (pak07_000.vpk next to it). Sigil does not repair split archives.").to_string());
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
    fn records_round_trip_as_json() {
        let record = Record {
            slot: 4,
            file: "pak04_dir.vpk".into(),
            backup: PathBuf::from("/data/compat-backups/abc.vpk"),
            new_sha256: "abc".into(),
            old_size: 10,
            new_size: 12,
            changes: 3,
        };
        assert_eq!(Record::from_json(&record.to_json()), Some(record));
        assert_eq!(Record::from_json(&json!({"slot": 400})), None);
    }
}
