//! Load order: which addon slot number each mod sits in, and the renames that change it.
//!
//! Deadlock loads `pak01_dir.vpk` before `pak02_dir.vpk` and the lower slot wins a path two
//! mods both ship. Changing the order is renaming slot files between numbers. A slot is up to
//! three kinds of file, and every one of them moves with the slot:
//!
//! - `pakNN_dir.vpk` (on) or `pakNN_dir.vpk.disabled` (off), the directory file;
//! - `pakNN_MMM.vpk`, the chunk files the directory points into by number.
//!
//! [`plan_reorder`] is pure: it turns slot moves into two passes of renames that never
//! overwrite a file. Pass one moves every file of every moving slot to a hidden temporary
//! name in the same folder, pass two moves each temporary name to its final name. A cycle
//! (01 to 02 and 02 to 01) therefore never needs a free slot. [`apply_reorder`] is the game
//! install write: it checks the plan against the folder, renames, and puts every file back
//! where it was when any rename fails. [`reverse_plan`] is the undo.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::addons::{DISABLED_SUFFIX, MAX_SLOT, global_slot, is_slot, local_slot, slot_folder};
use crate::error::{Error, Result};
use crate::import::Library;

/// Prefix of the hidden temporary names a reorder uses between its two passes.
pub const TEMP_PREFIX: &str = ".sigil-reorder-";

/// One slot changing number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotMove {
    pub from: u16,
    pub to: u16,
}

/// One rename, by path relative to the `citadel` folder (`addons/pak05_dir.vpk`,
/// `sigil-addons2/pak01_dir.vpk`), so a move can cross into an extra addon folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameStep {
    pub from: String,
    pub to: String,
}

/// The renames that carry out a set of slot moves. Built by [`plan_reorder`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPlan {
    pub addons_dir: PathBuf,
    /// The moves, sorted by `from`. Moves to the same number are dropped.
    pub moves: Vec<SlotMove>,
    /// Pass one: every file of a moving slot to its temporary name.
    pub stage: Vec<RenameStep>,
    /// Pass two: every temporary name to its final name.
    pub finish: Vec<RenameStep>,
}

impl ReorderPlan {
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// `(original name, final name)` for every file the plan moves.
    pub fn renames(&self) -> Vec<(String, String)> {
        self.stage
            .iter()
            .zip(self.finish.iter())
            .map(|(stage, finish)| (stage.from.clone(), finish.to.clone()))
            .collect()
    }
}

/// What [`apply_reorder`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderReport {
    pub moves: Vec<SlotMove>,
    /// `(from, to)` file names, in the order they reached their final name.
    pub renamed: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Slot file names (pure)
// ---------------------------------------------------------------------------

fn slot_pattern() -> Regex {
    Regex::new(r"(?i)^pak(\d{2})_(dir\.vpk(\.disabled)?|\d{3}\.vpk)$").expect("slot file pattern is valid")
}

/// The `pakNN` number a file name carries, 01 to 99, inside its own folder: `pakNN_dir.vpk`,
/// `pakNN_dir.vpk.disabled` or a `pakNN_MMM.vpk` chunk. Anything else is `None`. A slot in an
/// extra folder is [`crate::addons::global_slot`] of its folder and this number.
pub fn slot_of(name: &str) -> Option<u16> {
    let captures = slot_pattern().captures(name)?;
    let slot = captures.get(1)?.as_str().parse::<u16>().ok()?;
    (1..=99).contains(&slot).then_some(slot)
}

/// `name` moved from slot `from` to slot `to` (both global), keeping its kind and suffix: the
/// new name carries `to`'s number inside its folder. `None` when the name is not a file of slot
/// `from` or `to` is not a slot.
pub fn rename_for_slot(name: &str, from: u16, to: u16) -> Option<String> {
    if slot_of(name) != Some(local_slot(from)) || !is_slot(to) {
        return None;
    }
    // `pak` plus two digits; the rest of the name stays as it is (case included).
    Some(format!("{}{:02}{}", &name[..3], local_slot(to), &name[5..]))
}

/// Group addon folder file names by slot. Names that are not slot files are left out.
pub fn group_slot_files<S: AsRef<str>>(names: &[S]) -> BTreeMap<u16, Vec<String>> {
    let mut slots: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for name in names {
        let name = name.as_ref();
        if let Some(slot) = slot_of(name) {
            slots.entry(slot).or_default().push(name.to_string());
        }
    }
    for files in slots.values_mut() {
        files.sort();
    }
    slots
}

/// The slot files in `addons_dir` and its extra folders, grouped by global slot. Names are bare
/// file names; [`crate::addons::slot_folder`] says which folder each slot's files are in. Read
/// only. Only regular files count: a symlink or folder with a slot name is left out, so no
/// operation follows it.
pub fn read_slot_files(addons_dir: &Path) -> Result<BTreeMap<u16, Vec<String>>> {
    let mut slots: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for (index, folder) in crate::addons::existing_folders(addons_dir) {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let mut names = Vec::new();
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_file() {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        for (local, files) in group_slot_files(&names) {
            slots.insert(global_slot(index, local), files);
        }
    }
    Ok(slots)
}

/// `name` in `slot`'s folder, relative to the `citadel` folder: `addons/pak05_dir.vpk`.
pub fn relative_to_citadel(addons_dir: &Path, slot: u16, name: &str) -> String {
    let folder = slot_folder(addons_dir, slot);
    let folder = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{folder}/{name}")
}

/// Whether a slot file name is a directory file that is switched off.
pub fn is_disabled_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(DISABLED_SUFFIX)
}

// ---------------------------------------------------------------------------
// Orders (pure)
// ---------------------------------------------------------------------------

/// The moves that give the slot numbers in `current` (ascending) to the same mods listed in
/// `wanted` order: the first mod of `wanted` takes the lowest number, and so on. The set of
/// numbers in use does not change. Mods that keep their number are left out.
///
/// `wanted` must hold exactly the slots of `current`; otherwise there is no move.
pub fn moves_for_order(current: &[u16], wanted: &[u16]) -> Vec<SlotMove> {
    let mut numbers: Vec<u16> = current.to_vec();
    numbers.sort_unstable();
    numbers.dedup();
    let mut check: Vec<u16> = wanted.to_vec();
    check.sort_unstable();
    if check != numbers {
        return Vec::new();
    }
    wanted
        .iter()
        .zip(numbers.iter())
        .filter(|(from, to)| from != to)
        .map(|(&from, &to)| SlotMove { from, to })
        .collect()
}

/// `slots` (in load order) with `slot` taken out and put back at `index`, clamped to the end.
/// A slot that is not in the list leaves the order as it was.
pub fn move_to_index(slots: &[u16], slot: u16, index: usize) -> Vec<u16> {
    let mut order: Vec<u16> = slots.to_vec();
    let Some(at) = order.iter().position(|&s| s == slot) else {
        return order;
    };
    order.remove(at);
    let index = index.min(order.len());
    order.insert(index, slot);
    order
}

/// The moves that swap two slots' numbers.
pub fn swap(a: u16, b: u16) -> Vec<SlotMove> {
    if a == b {
        return Vec::new();
    }
    vec![SlotMove { from: a, to: b }, SlotMove { from: b, to: a }]
}

// ---------------------------------------------------------------------------
// Plan (pure)
// ---------------------------------------------------------------------------

/// Turn slot moves into the two rename passes.
///
/// `files` is the addon folder grouped by slot ([`read_slot_files`]). Refuses a move from a
/// slot with no files, a slot that moves twice, two moves to one number, a number outside 01
/// to 99, and a move onto a slot that is taken and not itself moving away. `token` makes the
/// temporary names unique to this plan; it must be letters, digits or dashes.
pub fn plan_reorder(
    addons_dir: &Path,
    files: &BTreeMap<u16, Vec<String>>,
    moves: &[SlotMove],
    token: &str,
) -> Result<ReorderPlan> {
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::msg(format!("Reorder token {token:?} must be letters, digits or dashes.")));
    }
    let mut moves: Vec<SlotMove> = moves.iter().copied().filter(|m| m.from != m.to).collect();
    moves.sort();

    let mut froms = BTreeSet::new();
    let mut tos = BTreeSet::new();
    for m in &moves {
        for slot in [m.from, m.to] {
            if !is_slot(slot) {
                return Err(Error::msg(format!(
                    "Slot {slot} is outside 1 to {MAX_SLOT}. Nothing was renamed."
                )));
            }
        }
        if !froms.insert(m.from) {
            return Err(Error::msg(format!(
                "Slot {:02} is moved twice. Nothing was renamed.",
                m.from
            )));
        }
        if !tos.insert(m.to) {
            return Err(Error::msg(format!(
                "Two mods are moved to slot {:02}. Nothing was renamed.",
                m.to
            )));
        }
        if files.get(&m.from).is_none_or(Vec::is_empty) {
            return Err(Error::msg(format!(
                "Slot {:02} has no files to move. Nothing was renamed.",
                m.from
            )));
        }
    }
    for m in &moves {
        if files.get(&m.to).is_some_and(|names| !names.is_empty()) && !froms.contains(&m.to) {
            return Err(Error::msg(format!(
                "Slot {:02} is taken and is not moving. Nothing was renamed.",
                m.to
            )));
        }
    }

    let mut stage = Vec::new();
    let mut finish = Vec::new();
    for m in &moves {
        for name in &files[&m.from] {
            let target = rename_for_slot(name, m.from, m.to).ok_or_else(|| {
                Error::msg(format!("{name} is not a file of slot {:02}.", m.from))
            })?;
            let temporary = format!("{TEMP_PREFIX}{token}-{target}");
            let temporary = relative_to_citadel(addons_dir, m.to, &temporary);
            stage.push(RenameStep {
                from: relative_to_citadel(addons_dir, m.from, name),
                to: temporary.clone(),
            });
            finish.push(RenameStep {
                from: temporary,
                to: relative_to_citadel(addons_dir, m.to, &target),
            });
        }
    }

    Ok(ReorderPlan {
        addons_dir: addons_dir.to_path_buf(),
        moves,
        stage,
        finish,
    })
}

/// The plan that puts a finished reorder back: the same files, from their final names
/// through the temporary names to where they started. Pure; apply it with
/// [`apply_reorder`].
pub fn reverse_plan(plan: &ReorderPlan) -> ReorderPlan {
    let mut moves: Vec<SlotMove> = plan
        .moves
        .iter()
        .map(|m| SlotMove {
            from: m.to,
            to: m.from,
        })
        .collect();
    moves.sort();
    ReorderPlan {
        addons_dir: plan.addons_dir.clone(),
        moves,
        stage: plan
            .finish
            .iter()
            .map(|step| RenameStep {
                from: step.to.clone(),
                to: step.from.clone(),
            })
            .collect(),
        finish: plan
            .stage
            .iter()
            .map(|step| RenameStep {
                from: step.to.clone(),
                to: step.from.clone(),
            })
            .collect(),
    }
}

/// Library entries follow their slot to its new number. App data, not the game install.
pub fn remap_library(library: &mut Library, moves: &[SlotMove]) {
    let map: BTreeMap<u16, u16> = moves.iter().map(|m| (m.from, m.to)).collect();
    for meta in &mut library.mods {
        if let Some(&to) = map.get(&meta.slot) {
            meta.slot = to;
        }
    }
    library.mods.sort_by_key(|meta| meta.slot);
}

// ---------------------------------------------------------------------------
// Apply (game install write)
// ---------------------------------------------------------------------------

/// Carry out a reorder plan in the addon folder. Game install write; refuses without
/// `approve`.
///
/// Before the first rename it checks that every source is a regular file, that no temporary
/// name exists, and that every final name is free or belongs to a file the plan moves away.
/// No rename ever replaces a file. When any rename fails, the ones already done are undone in
/// reverse order, so the folder ends as it started; the error says so, or names the files it
/// could not put back.
pub fn apply_reorder(plan: &ReorderPlan, approve: bool) -> Result<ReorderReport> {
    if !approve {
        return Err(Error::msg(
            "Not approved: change the load order. Nothing was renamed.",
        ));
    }
    if plan.stage.len() != plan.finish.len() {
        return Err(Error::msg("The reorder plan is malformed. Nothing was renamed."));
    }
    let addons = &plan.addons_dir;
    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was renamed.",
            addons.display()
        )));
    }
    // Steps are relative to the citadel folder so they can cross into an extra addon folder.
    let citadel = addons.parent().unwrap_or(addons.as_path()).to_path_buf();
    let dir = &citadel;
    let addon_folders: BTreeSet<String> = (0..crate::overflow::MAX_FOLDERS)
        .filter_map(|index| crate::addons::folder_at(addons, index).file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();

    let leaving: BTreeSet<String> = plan.stage.iter().map(|s| s.from.to_ascii_lowercase()).collect();
    for step in &plan.stage {
        check_step_path(&step.from, &addon_folders)?;
        check_step_path(&step.to, &addon_folders)?;
        let source = dir.join(&step.from);
        let regular = fs::symlink_metadata(&source).map(|m| m.file_type().is_file()).unwrap_or(false);
        if !regular {
            return Err(Error::msg(format!(
                "{} is missing or not a regular file. The folder changed since the plan was made. Nothing was renamed.",
                step.from
            )));
        }
        if fs::symlink_metadata(dir.join(&step.to)).is_ok() {
            return Err(Error::msg(format!(
                "{} already exists. Nothing was renamed.",
                step.to
            )));
        }
    }
    for step in &plan.finish {
        check_step_path(&step.to, &addon_folders)?;
        if slot_of(file_part(&step.to)).is_none() {
            return Err(Error::msg(format!("{} is not a slot file name. Nothing was renamed.", step.to)));
        }
        if fs::symlink_metadata(dir.join(&step.to)).is_ok()
            && !leaving.contains(&step.to.to_ascii_lowercase())
        {
            return Err(Error::msg(format!(
                "{} already exists and is not moving. Nothing was renamed.",
                step.to
            )));
        }
    }

    // A move into an extra folder needs the folder; it is mounted afterwards.
    for step in &plan.finish {
        if let Some(parent) = dir.join(&step.to).parent() {
            fs::create_dir_all(parent)?;
        }
    }

    // Pass one, then pass two. `done` holds every rename made so far, for the rollback.
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for step in plan.stage.iter().chain(plan.finish.iter()) {
        let from = dir.join(&step.from);
        let to = dir.join(&step.to);
        let result = if fs::symlink_metadata(&to).is_ok() {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} appeared during the reorder", step.to),
            ))
        } else {
            fs::rename(&from, &to)
        };
        if let Err(error) = result {
            let stuck = roll_back(&done);
            return Err(if stuck.is_empty() {
                Error::msg(format!(
                    "Renaming {} to {} failed ({error}). Every file was put back where it was.",
                    step.from, step.to
                ))
            } else {
                Error::msg(format!(
                    "Renaming {} to {} failed ({error}), and these could not be put back: {}. Rename them by hand in {}.",
                    step.from,
                    step.to,
                    stuck.join(", "),
                    dir.display()
                ))
            });
        }
        done.push((from, to));
    }

    // A move into or out of an extra folder changes which folders gameinfo.gi must mount. The
    // files are in place first; a failed mount leaves mods unmounted, never half moved.
    if plan.moves.iter().any(|m| crate::addons::folder_index(m.from) > 0 || crate::addons::folder_index(m.to) > 0) {
        crate::addons::sync_overflow_mount(addons)?;
    }

    Ok(ReorderReport {
        moves: plan.moves.clone(),
        renamed: plan.renames(),
    })
}

/// Undo renames newest first. Returns the names that could not be put back.
fn roll_back(done: &[(PathBuf, PathBuf)]) -> Vec<String> {
    let mut stuck = Vec::new();
    for (from, to) in done.iter().rev() {
        if fs::symlink_metadata(from).is_ok() || fs::rename(to, from).is_err() {
            stuck.push(
                to.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
    }
    stuck
}

/// The file name part of a step path (`addons/pak05_dir.vpk` gives `pak05_dir.vpk`).
pub(crate) fn file_part(step: &str) -> &str {
    step.rsplit('/').next().unwrap_or(step)
}

/// A step path is one addon folder this build knows and one plain file name.
fn check_step_path(step: &str, folders: &BTreeSet<String>) -> Result<()> {
    let Some((folder, name)) = step.split_once('/') else {
        return Err(Error::msg(format!("{step:?} has no addon folder. Nothing was changed.")));
    };
    if !folders.contains(folder) {
        return Err(Error::msg(format!("{folder:?} is not an addon folder. Nothing was changed.")));
    }
    check_plain_name(name)
}

/// A bare file name: no separators, no `..`, not empty.
pub(crate) fn check_plain_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
    {
        return Err(Error::msg(format!("{name:?} is not a plain file name. Nothing was changed.")));
    }
    Ok(())
}

/// A token for temporary names: the time and the process, letters, digits and dashes.
pub fn new_token() -> String {
    format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S%3f"),
        std::process::id()
    )
}

/// The final slot file name a `.sigil-reorder-<token>-` temporary name was heading for, or
/// `None` when the name is not one of the reorder's temporary names.
pub fn target_of_temp(name: &str) -> Option<String> {
    let rest = name.strip_prefix(TEMP_PREFIX)?;
    let lower = rest.to_ascii_lowercase();
    let at = lower.rmatch_indices("-pak").next().map(|(index, _)| index)?;
    let target = &rest[at + 1..];
    slot_of(target).map(|_| target.to_string())
}

/// The `.sigil-reorder-*` files an interrupted reorder left behind, in the main addon folder.
/// Read only. [`leftover_temp_paths`] covers the extra folders too.
pub fn leftover_temp_files(addons_dir: &Path) -> Vec<String> {
    leftover_in(addons_dir)
}

fn leftover_in(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| target_of_temp(name).is_some())
        .collect();
    names.sort();
    names
}

/// Every leftover temporary file across the addon folders, as `(folder, name)`. Read only.
pub fn leftover_temp_paths(addons_dir: &Path) -> Vec<(PathBuf, String)> {
    crate::addons::existing_folders(addons_dir)
        .into_iter()
        .flat_map(|(_, folder)| leftover_in(&folder).into_iter().map(move |name| (folder.clone(), name)))
        .collect()
}

/// What a recovery pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    /// `(temporary name, final name)` for every file put back.
    pub restored: Vec<(String, String)>,
    /// Temporary files whose final name is taken, so they were left alone.
    pub left: Vec<String>,
}

/// Put back the files of a reorder that stopped between its two passes. Game install write;
/// refuses without `approve`.
///
/// Each `.sigil-reorder-<token>-pakNN...` file is renamed to the slot name it was heading for.
/// A name that is taken again (another mod went into the slot) leaves its temporary file in
/// place and is named in [`RecoveryReport::left`], so nothing is ever overwritten.
pub fn recover_temp_files(addons_dir: &Path, approve: bool) -> Result<RecoveryReport> {
    if !approve {
        return Err(Error::msg(
            "Not approved: put back the files of an unfinished reorder. Nothing was renamed.",
        ));
    }
    if !addons_dir.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was renamed.",
            addons_dir.display()
        )));
    }
    let mut report = RecoveryReport::default();
    for (folder, name) in leftover_temp_paths(addons_dir) {
        let Some(target) = target_of_temp(&name) else {
            continue;
        };
        let to = folder.join(&target);
        if fs::symlink_metadata(&to).is_ok() {
            report.left.push(name);
            continue;
        }
        match fs::rename(folder.join(&name), &to) {
            Ok(()) => report.restored.push((name, target)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                report.left.push(name);
            }
            Err(error) => {
                return Err(Error::msg(format!(
                    "Putting {name} back failed: {error}. The other files were put back."
                )));
            }
        }
    }
    Ok(report)
}

/// Move one slot to the front of its group (the mods that are on, or the ones that are off).
/// Game install write; refuses without `approve`. This is how an "always load first" mod wins
/// every overlap: the lowest slot number in the addon folder.
pub fn move_slot_to_front(game_root: &Path, slot: u16, approve: bool) -> Result<ReorderReport> {
    let addons = crate::deploy::addons_dir(game_root);
    let files = read_slot_files(&addons)?;
    let plan = plan_slot_to_front(&addons, &files, slot, &new_token())?;
    apply_reorder(&plan, approve)
}

/// The plan that moves one slot to the front of its group. Pure; apply it with
/// [`apply_reorder`] so the caller can keep it for Undo.
pub fn plan_slot_to_front(
    addons_dir: &Path,
    files: &BTreeMap<u16, Vec<String>>,
    slot: u16,
    token: &str,
) -> Result<ReorderPlan> {
    let Some(names) = files.get(&slot) else {
        return Err(Error::msg(format!(
            "Slot {slot:02} is not installed. Nothing was renamed."
        )));
    };
    let enabled = !names.iter().any(|name| is_disabled_name(name));
    let group: Vec<u16> = files
        .iter()
        .filter(|(_, names)| names.iter().any(|name| is_disabled_name(name)) != enabled)
        .map(|(&slot, _)| slot)
        .collect();
    let wanted = move_to_index(&group, slot, 0);
    let moves = moves_for_order(&group, &wanted);
    plan_reorder(addons_dir, files, &moves, token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ModMeta;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/load-order-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
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
    fn slot_names_are_recognised_and_renamed() {
        assert_eq!(slot_of("pak01_dir.vpk"), Some(1));
        assert_eq!(slot_of("PAK07_DIR.VPK.disabled"), Some(7));
        assert_eq!(slot_of("pak12_003.vpk"), Some(12));
        assert_eq!(slot_of("pak00_dir.vpk"), None);
        assert_eq!(slot_of("pak1_dir.vpk"), None);
        assert_eq!(slot_of("pak01_dir.vpk.bak"), None);
        assert_eq!(slot_of("readme.txt"), None);
        assert_eq!(slot_of("pak01_dir.vpk/../x"), None);
        assert_eq!(
            rename_for_slot("pak05_dir.vpk.disabled", 5, 1).as_deref(),
            Some("pak01_dir.vpk.disabled")
        );
        assert_eq!(rename_for_slot("pak05_002.vpk", 5, 42).as_deref(), Some("pak42_002.vpk"));
        assert_eq!(rename_for_slot("pak05_dir.vpk", 4, 1), None);
        // Slot 100 is pak01 in the first extra folder; past the last slot there is none.
        assert_eq!(rename_for_slot("pak05_dir.vpk", 5, 100).as_deref(), Some("pak01_dir.vpk"));
        assert_eq!(rename_for_slot("pak01_dir.vpk", 100, 5).as_deref(), Some("pak05_dir.vpk"));
        assert_eq!(rename_for_slot("pak05_dir.vpk", 5, crate::addons::MAX_SLOT + 1), None);
    }

    #[test]
    fn orders_become_moves() {
        let slots = [1, 2, 3, 5];
        let wanted = move_to_index(&slots, 5, 0);
        assert_eq!(wanted, vec![5, 1, 2, 3]);
        assert_eq!(
            moves_for_order(&slots, &wanted),
            vec![
                SlotMove { from: 5, to: 1 },
                SlotMove { from: 1, to: 2 },
                SlotMove { from: 2, to: 3 },
                SlotMove { from: 3, to: 5 },
            ]
        );
        assert_eq!(move_to_index(&slots, 1, 99), vec![2, 3, 5, 1]);
        assert_eq!(move_to_index(&slots, 9, 0), slots.to_vec());
        assert!(moves_for_order(&slots, &[1, 2, 3]).is_empty());
        assert!(moves_for_order(&slots, &slots).is_empty());
        assert_eq!(swap(2, 7).len(), 2);
        assert!(swap(3, 3).is_empty());
    }

    #[test]
    fn plan_uses_temporary_names_and_never_targets_a_taken_slot() {
        let files = group_slot_files(&[
            "pak01_dir.vpk",
            "pak02_dir.vpk.disabled",
            "pak02_000.vpk",
            "pak03_dir.vpk",
            "notes.txt",
        ]);
        let plan = plan_reorder(Path::new("/x"), &files, &swap(1, 2), "t1").unwrap();
        assert_eq!(plan.stage.len(), 3);
        // Steps name their folder (relative to citadel), so a move can cross folders.
        for step in &plan.stage {
            assert!(step.to.starts_with("x/.sigil-reorder-t1-"), "{}", step.to);
        }
        let renames = plan.renames();
        assert!(renames.contains(&("x/pak02_000.vpk".into(), "x/pak01_000.vpk".into())));
        assert!(renames.contains(&("x/pak02_dir.vpk.disabled".into(), "x/pak01_dir.vpk.disabled".into())));
        assert!(renames.contains(&("x/pak01_dir.vpk".into(), "x/pak02_dir.vpk".into())));

        // Onto slot 3, which stays: refused.
        let error = plan_reorder(Path::new("/x"), &files, &[SlotMove { from: 1, to: 3 }], "t")
            .unwrap_err()
            .to_string();
        assert!(error.contains("taken"), "{error}");
        // Twice to one number, an empty slot, a bad token.
        assert!(plan_reorder(Path::new("/x"), &files, &[SlotMove { from: 1, to: 9 }, SlotMove { from: 3, to: 9 }], "t").is_err());
        assert!(plan_reorder(Path::new("/x"), &files, &[SlotMove { from: 8, to: 9 }], "t").is_err());
        assert!(plan_reorder(Path::new("/x"), &files, &[SlotMove { from: 1, to: 9 }], "../t").is_err());
        // A move to a free number is fine.
        assert!(plan_reorder(Path::new("/x"), &files, &[SlotMove { from: 3, to: 9 }], "t").is_ok());
    }

    #[test]
    fn apply_swaps_slots_with_chunks_and_disabled_files_and_reverse_undoes_it() {
        let dir = scratch("swap");
        write(&dir, "pak01_dir.vpk", "one");
        write(&dir, "pak01_000.vpk", "one-chunk");
        write(&dir, "pak02_dir.vpk.disabled", "two");
        write(&dir, "pak03_dir.vpk", "three");
        write(&dir, "readme.txt", "keep");

        let files = read_slot_files(&dir).unwrap();
        let wanted = move_to_index(&[1, 2, 3], 3, 0);
        let plan = plan_reorder(&dir, &files, &moves_for_order(&[1, 2, 3], &wanted), &new_token()).unwrap();
        let report = apply_reorder(&plan, true).unwrap();
        assert_eq!(report.moves.len(), 3);
        assert_eq!(
            names(&dir),
            vec!["pak01_dir.vpk", "pak02_000.vpk", "pak02_dir.vpk", "pak03_dir.vpk.disabled", "readme.txt"]
        );
        assert_eq!(read(&dir, "pak01_dir.vpk"), "three");
        assert_eq!(read(&dir, "pak02_dir.vpk"), "one");
        assert_eq!(read(&dir, "pak02_000.vpk"), "one-chunk");
        assert_eq!(read(&dir, "pak03_dir.vpk.disabled"), "two");

        apply_reorder(&reverse_plan(&plan), true).unwrap();
        assert_eq!(
            names(&dir),
            vec!["pak01_000.vpk", "pak01_dir.vpk", "pak02_dir.vpk.disabled", "pak03_dir.vpk", "readme.txt"]
        );
        assert_eq!(read(&dir, "pak01_dir.vpk"), "one");
        assert_eq!(read(&dir, "pak03_dir.vpk"), "three");
    }

    #[test]
    fn apply_refuses_without_approval_and_on_a_stale_folder() {
        let dir = scratch("stale");
        write(&dir, "pak01_dir.vpk", "one");
        write(&dir, "pak02_dir.vpk", "two");
        let files = read_slot_files(&dir).unwrap();
        let plan = plan_reorder(&dir, &files, &[SlotMove { from: 2, to: 5 }], "t").unwrap();
        assert!(apply_reorder(&plan, false).is_err());

        // Slot 05 appeared after planning: refused, nothing moved.
        write(&dir, "pak05_dir.vpk", "intruder");
        let error = apply_reorder(&plan, true).unwrap_err().to_string();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(read(&dir, "pak02_dir.vpk"), "two");
        assert_eq!(read(&dir, "pak05_dir.vpk"), "intruder");
    }

    #[test]
    fn a_failed_rename_rolls_everything_back() {
        let dir = scratch("rollback");
        write(&dir, "pak01_dir.vpk", "one");
        write(&dir, "pak02_dir.vpk", "two");
        let files = read_slot_files(&dir).unwrap();
        let mut plan = plan_reorder(&dir, &files, &swap(1, 2), "t").unwrap();
        // Sabotage pass two: its last step reads a temporary name that never exists.
        let last = plan.finish.len() - 1;
        plan.finish[last].from = ".sigil-reorder-t-missing".to_string();
        let error = apply_reorder(&plan, true).unwrap_err().to_string();
        assert!(error.contains("put back"), "{error}");
        assert_eq!(names(&dir), vec!["pak01_dir.vpk", "pak02_dir.vpk"]);
        assert_eq!(read(&dir, "pak01_dir.vpk"), "one");
        assert_eq!(read(&dir, "pak02_dir.vpk"), "two");
    }

    #[test]
    fn library_entries_follow_their_slot() {
        let mut library = Library::default();
        for (slot, name) in [(1, "a"), (2, "b"), (4, "c")] {
            library.mods.push(ModMeta {
                slot,
                name: name.to_string(),
                ..Default::default()
            });
        }
        remap_library(&mut library, &swap(1, 2));
        let got: Vec<(u16, &str)> = library.mods.iter().map(|m| (m.slot, m.name.as_str())).collect();
        assert_eq!(got, vec![(1, "b"), (2, "a"), (4, "c")]);
    }

    #[test]
    fn leftover_reorder_files_are_recognised_and_put_back() {
        let dir = scratch("recover");
        write(&dir, "pak01_dir.vpk", "one");
        let temp = format!("{TEMP_PREFIX}20261117120000000-999-pak02_dir.vpk");
        write(&dir, &temp, "two");
        assert_eq!(target_of_temp(&temp).as_deref(), Some("pak02_dir.vpk"));
        assert_eq!(target_of_temp("pak01_dir.vpk"), None);
        assert_eq!(leftover_temp_files(&dir), vec![temp.clone()]);

        // A target that is taken again stays as a temporary file.
        write(&dir, "pak03_dir.vpk", "three");
        let taken = format!("{TEMP_PREFIX}token-pak03_dir.vpk");
        write(&dir, &taken, "old three");

        assert!(recover_temp_files(&dir, false).is_err());
        let report = recover_temp_files(&dir, true).unwrap();
        assert_eq!(report.restored, vec![(temp, "pak02_dir.vpk".to_string())]);
        assert_eq!(report.left, vec![taken.clone()]);
        assert_eq!(read(&dir, "pak02_dir.vpk"), "two");
        assert_eq!(read(&dir, "pak03_dir.vpk"), "three");
        assert_eq!(read(&dir, &taken), "old three");
    }

    #[test]
    fn a_slot_moves_to_the_front_of_its_own_group() {
        let root = scratch("front");
        let dir = crate::deploy::addons_dir(&root);
        fs::create_dir_all(&dir).unwrap();
        write(&dir, "pak01_dir.vpk", "one");
        write(&dir, "pak02_dir.vpk", "two");
        write(&dir, "pak05_dir.vpk", "five");
        write(&dir, "pak07_dir.vpk.disabled", "seven");
        assert!(move_slot_to_front(&root, 5, false).is_err());
        let report = move_slot_to_front(&root, 5, true).unwrap();
        assert_eq!(report.moves.len(), 3);
        assert_eq!(read(&dir, "pak01_dir.vpk"), "five");
        assert_eq!(read(&dir, "pak02_dir.vpk"), "one");
        assert_eq!(read(&dir, "pak05_dir.vpk"), "two");
        // The off slot keeps its own group's order.
        assert_eq!(read(&dir, "pak07_dir.vpk.disabled"), "seven");
        assert!(move_slot_to_front(&root, 9, true).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
