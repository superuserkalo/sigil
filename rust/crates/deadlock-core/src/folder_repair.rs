//! Repair the mods folder: find broken or stray slot files and fix them after a review.
//!
//! Deadlock Mod Manager's unmerged repair work (a "Repair mod folder" action, forgetting mods
//! whose VPKs were deleted outside the app, a ledger that follows moved files) and Grimoire's
//! orphan cleanup describe one need: the addon folders drift. A player deletes a file in
//! Explorer, a download stops halfway, another tool leaves a `.disabled` copy beside an enabled
//! one, a guide says to drop `mymod.vpk` straight into `addons`, a game update resets
//! `gameinfo.gi` so Sigil's extra folders stop loading. [`review`] finds each case and says how
//! it would fix it; [`apply`] fixes the ones the player ticked.
//!
//! What it finds, across `citadel/addons` and Sigil's `sigil-addons2` to `sigil-addons10`:
//!
//! - [`Kind::MissingChunks`]: a directory VPK whose `pakNN_MMM.vpk` parts are gone. The game
//!   loads it partly or not at all. Fix: the slot to the trash (or Reinstall, the app's).
//! - [`Kind::Unreadable`]: a directory VPK that does not parse (a truncated download, an empty
//!   file). Fix: the slot to the trash.
//! - [`Kind::OrphanChunks`]: chunk files with no directory file. The game never opens them.
//!   Fix: those files to the trash.
//! - [`Kind::Doubled`]: `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled` side by side. The same mod
//!   twice: the `.disabled` leftover goes to the trash. Two different mods: the off one moves to
//!   a free slot of its own, still off.
//! - [`Kind::LibraryGone`]: a library entry whose slot is empty (deleted outside Sigil). Fix:
//!   forget the entry. App data only.
//! - [`Kind::Misplaced`]: a VPK the game ignores because of where it is: a wrong name directly
//!   in an addon folder (`mymod.vpk`, `pak1_dir.vpk`, `mymod_dir.vpk` with its parts), a
//!   subfolder of `addons`, or a `sigil-addons` folder outside 2 to 10. Fix: move it into the
//!   lowest free slot, switched off, so the player turns it on. Subfolders are left unticked by
//!   default: another manager may own them, and a folder holding `.dmm.json` is never offered.
//! - [`Kind::Unmounted`]: a Sigil extra folder holds mods but `gameinfo.gi` does not mount it.
//!   Fix: [`crate::addons::sync_overflow_mount`].
//! - [`Kind::StoppedReorder`]: `.sigil-reorder-*` files a stopped reorder left. Fix:
//!   [`crate::load_order::recover_temp_files`].
//! - [`Kind::Duplicate`]: the same mod installed twice ([`crate::duplicates`]). Fix: the extra
//!   copy to the trash.
//!
//! Every removal goes to Sigil's trash ([`crate::trash`]), so [`undo`] can put it back. Moves
//! never overwrite. [`apply`] reads the folders again first and fixes only findings that are
//! still exactly as reviewed. GAME WRITE: [`apply`] and [`undo`] refuse without `approve`; the
//! app calls them on the player's press in the review dialog, never from an agent tool.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::addons::{self, DISABLED_SUFFIX};
use crate::error::{Error, Result};
use crate::import::{Library, ModMeta};
use crate::trash::{self, TrashEntry};
use crate::vpk::{ReadVpkOptions, VPK_DIRECTORY_ARCHIVE, read_vpk_directory_with};

/// What is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    MissingChunks { chunks: Vec<String> },
    Unreadable { error: String },
    OrphanChunks { files: Vec<String> },
    /// `same` when both files are the same mod.
    Doubled { same: bool },
    LibraryGone { name: String },
    /// `files` are paths from the `citadel` folder; `subfolder` when they sit in a subfolder.
    Misplaced { files: Vec<String>, subfolder: bool },
    Unmounted { folder: String },
    StoppedReorder { files: Vec<String> },
    /// The copy kept and its name in the library, when it has one.
    Duplicate { keep: u16 },
}

/// How a finding is fixed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Move these files of `slot` to the trash (every file of the slot when `names` is empty).
    Trash { slot: u16, names: Vec<String> },
    /// Move files into a free slot, switched off: `(from, to-suffix)` pairs, `from` absolute,
    /// `suffix` `_dir.vpk.disabled` or `_MMM.vpk`.
    Adopt { files: Vec<(PathBuf, String)> },
    /// Drop the library entry of `slot`.
    Forget { slot: u16 },
    Mount,
    FinishReorder,
    /// Nothing Sigil can do here; the line explains.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Stable for the same folder state, so the dialog's ticks survive a second read.
    pub id: String,
    pub slot: Option<u16>,
    pub kind: Kind,
    pub fix: Fix,
    /// Ticked when the dialog opens.
    pub suggested: bool,
}

/// [`review`]'s answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    pub findings: Vec<Finding>,
}

impl Review {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// What [`apply`] did, and what [`undo`] needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub fixed: Vec<String>,
    pub trashed: Vec<TrashEntry>,
    /// `(from, to)` of every file moved, in order.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Slots files were moved into.
    pub adopted: Vec<u16>,
    pub forgotten: Vec<ModMeta>,
    pub mounted: bool,
    pub reorder_files: usize,
    /// `(finding id, why)` for each fix that failed.
    pub errors: Vec<(String, String)>,
}

fn chunk_re() -> Regex {
    Regex::new(r"(?i)^(.+)_(\d{3})\.vpk$").expect("chunk pattern is valid")
}

fn dir_re() -> Regex {
    Regex::new(r"(?i)^(.+)_dir\.vpk$").expect("dir pattern is valid")
}

fn lower(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// The chunk indices a directory VPK points at, or the parse error.
fn needed_chunks(path: &Path) -> std::result::Result<BTreeSet<u16>, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        return Err("the file is empty".to_string());
    }
    let directory = read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: false }).map_err(|e| e.to_string())?;
    Ok(directory
        .entries
        .iter()
        .filter(|e| e.length > 0 && e.archive_index != VPK_DIRECTORY_ARCHIVE)
        .map(|e| e.archive_index)
        .collect())
}

/// Read every addon folder and say what needs fixing. `library` is Sigil's library store, so a
/// gone mod is named and the copy the library knows is kept. Reads only.
pub fn review(addons_dir: &Path, library: &Library) -> Result<Review> {
    let mut findings = Vec::new();
    let slots = crate::load_order::read_slot_files(addons_dir)?;
    let temp: BTreeSet<String> = crate::load_order::leftover_temp_paths(addons_dir).into_iter().map(|(_, n)| n).collect();

    // Slot problems.
    for (slot, files) in &slots {
        let slot = *slot;
        let folder = addons::slot_folder(addons_dir, slot);
        let dir_name = lower(&addons::slot_name(slot, "_dir.vpk"));
        let on = files.iter().find(|f| lower(f) == dir_name).cloned();
        let off = files.iter().find(|f| lower(f) == format!("{dir_name}{DISABLED_SUFFIX}")).cloned();
        let chunks: Vec<String> = files.iter().filter(|f| Some(*f) != on.as_ref() && Some(*f) != off.as_ref()).cloned().collect();
        let Some(main) = on.clone().or(off.clone()) else {
            if !chunks.is_empty() {
                findings.push(Finding {
                    id: format!("orphan:{slot}:{}", chunks.join(",")),
                    slot: Some(slot),
                    kind: Kind::OrphanChunks { files: chunks.clone() },
                    fix: Fix::Trash { slot, names: chunks },
                    suggested: true,
                });
            }
            continue;
        };
        match needed_chunks(&folder.join(&main)) {
            Err(error) => findings.push(Finding {
                id: format!("unreadable:{slot}:{main}"),
                slot: Some(slot),
                kind: Kind::Unreadable { error },
                fix: Fix::Trash { slot, names: Vec::new() },
                suggested: false,
            }),
            Ok(needed) => {
                let stem = format!("pak{:02}", addons::local_slot(slot));
                let gone: Vec<String> = needed
                    .iter()
                    .map(|i| format!("{stem}_{i:03}.vpk"))
                    .filter(|name| !chunks.iter().any(|c| c.eq_ignore_ascii_case(name)))
                    .collect();
                if !gone.is_empty() {
                    findings.push(Finding {
                        id: format!("chunks:{slot}:{}", gone.join(",")),
                        slot: Some(slot),
                        kind: Kind::MissingChunks { chunks: gone },
                        fix: Fix::Trash { slot, names: Vec::new() },
                        suggested: false,
                    });
                }
            }
        }
        if let (Some(on), Some(off)) = (&on, &off) {
            let a = crate::duplicates::content_print(&folder.join(on)).ok().flatten();
            let b = crate::duplicates::content_print(&folder.join(off)).ok().flatten();
            let same = a.is_some() && a == b;
            let id = format!("doubled:{slot}:{}", u8::from(same));
            if same {
                findings.push(Finding {
                    id,
                    slot: Some(slot),
                    kind: Kind::Doubled { same },
                    fix: Fix::Trash { slot, names: vec![off.clone()] },
                    suggested: true,
                });
            } else {
                // A self-contained off copy can move; one that needs parts shares them with the
                // on copy and cannot be told apart, so it goes to the trash.
                let alone = needed_chunks(&folder.join(off)).map(|n| n.is_empty()).unwrap_or(false);
                findings.push(Finding {
                    id,
                    slot: Some(slot),
                    kind: Kind::Doubled { same },
                    fix: if alone {
                        Fix::Adopt { files: vec![(folder.join(off), format!("_dir.vpk{DISABLED_SUFFIX}"))] }
                    } else {
                        Fix::Trash { slot, names: vec![off.clone()] }
                    },
                    suggested: alone,
                });
            }
        }
    }

    // Library entries whose slot is empty.
    let mut named = BTreeSet::new();
    for meta in &library.mods {
        let has_dir = slots
            .get(&meta.slot)
            .is_some_and(|files| files.iter().any(|f| lower(f).contains("_dir.vpk")));
        if !has_dir && named.insert(meta.slot) {
            findings.push(Finding {
                id: format!("gone:{}:{}", meta.slot, meta.size_bytes),
                slot: Some(meta.slot),
                kind: Kind::LibraryGone { name: meta.name.clone() },
                fix: Fix::Forget { slot: meta.slot },
                suggested: true,
            });
        }
    }

    // Wrong names in each addon folder, and subfolders of the main one.
    for (index, folder) in addons::existing_folders(addons_dir) {
        stray_vpks(addons_dir, &folder, false, &temp, &mut findings);
        if index == 0
            && let Ok(entries) = fs::read_dir(&folder)
        {
            let mut subs: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .filter(|p| !p.join(".dmm.json").exists())
                .collect();
            subs.sort();
            for sub in subs {
                stray_vpks(addons_dir, &sub, true, &BTreeSet::new(), &mut findings);
            }
        }
    }
    // Sigil-named folders outside 2 to 10 hold slot files the game never mounts.
    if let Some(citadel) = addons_dir.parent()
        && let Ok(entries) = fs::read_dir(citadel)
    {
        let mut odd: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter(|e| {
                let name = lower(&e.file_name().to_string_lossy());
                name.strip_prefix(crate::overflow::FOLDER_PREFIX)
                    .is_some_and(|n| n.parse::<usize>().map(|n| !(2..=crate::overflow::MAX_FOLDERS).contains(&n)).unwrap_or(false))
            })
            .map(|e| e.path())
            .collect();
        odd.sort();
        for folder in odd {
            slot_units_in(addons_dir, &folder, &mut findings);
        }
    }

    // Extra folders that hold mods but are not mounted.
    if let Some(citadel) = addons_dir.parent()
        && let Ok(text) = fs::read_to_string(citadel.join("gameinfo.gi"))
        && !text.lines().any(|l| l.trim() == "// sigil:profile-begin")
    {
        let mounted = crate::overflow::overflow_folders_in(&text);
        let used: BTreeSet<usize> = slots
            .iter()
            .filter(|(_, files)| files.iter().any(|f| lower(f).ends_with("_dir.vpk")))
            .map(|(slot, _)| addons::folder_index(*slot))
            .filter(|i| *i > 0)
            .collect();
        for index in used {
            let Ok(name) = crate::overflow::folder_name(index + 1) else { continue };
            if !mounted.iter().any(|m| m.eq_ignore_ascii_case(&name) || m.ends_with(&format!("/{name}"))) {
                findings.push(Finding {
                    id: format!("unmounted:{name}"),
                    slot: None,
                    kind: Kind::Unmounted { folder: name },
                    fix: Fix::Mount,
                    suggested: true,
                });
            }
        }
    }

    if !temp.is_empty() {
        let files: Vec<String> = temp.into_iter().collect();
        findings.push(Finding {
            id: format!("reorder:{}", files.join(",")),
            slot: None,
            kind: Kind::StoppedReorder { files },
            fix: Fix::FinishReorder,
            suggested: true,
        });
    }

    // The same mod installed twice.
    let prints = crate::duplicates::scan(addons_dir).unwrap_or_default();
    let copies: Vec<crate::duplicates::SlotCopy> = prints
        .iter()
        .map(|(slot, enabled, print, bytes)| crate::duplicates::SlotCopy {
            slot: *slot,
            enabled: *enabled,
            known: library.mods.iter().any(|m| m.slot == *slot),
            print: print.hash.clone(),
            disk_bytes: *bytes,
        })
        .collect();
    let broken: BTreeSet<u16> = findings
        .iter()
        .filter(|f| matches!(f.kind, Kind::MissingChunks { .. } | Kind::Unreadable { .. }))
        .filter_map(|f| f.slot)
        .collect();
    for set in crate::duplicates::find_duplicates(&copies) {
        for extra in set.extra.iter().filter(|s| !broken.contains(s)) {
            findings.push(Finding {
                id: format!("duplicate:{extra}:{}", set.print),
                slot: Some(*extra),
                kind: Kind::Duplicate { keep: set.keep },
                fix: Fix::Trash { slot: *extra, names: Vec::new() },
                suggested: true,
            });
        }
    }

    Ok(Review { findings })
}

fn citadel_relative(addons_dir: &Path, path: &Path) -> String {
    let citadel = addons_dir.parent().unwrap_or(addons_dir);
    path.strip_prefix(citadel).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// `.vpk` files directly in `folder` that are not slot names: a `<stem>_dir.vpk` with its
/// `<stem>_MMM.vpk` parts, or a plain `<name>.vpk` that is a whole VPK on its own.
fn stray_vpks(addons_dir: &Path, folder: &Path, subfolder: bool, temp: &BTreeSet<String>, findings: &mut Vec<Finding>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| lower(n).ends_with(".vpk"))
        .filter(|n| crate::load_order::slot_of(n).is_none() && !temp.contains(n))
        .collect();
    names.sort();
    let claimed = dmm_claimed(folder);
    let (dirs, chunks) = (dir_re(), chunk_re());
    let mut parts: BTreeMap<String, Vec<(u16, String)>> = BTreeMap::new();
    for name in &names {
        if let Some(c) = chunks.captures(name)
            && let Ok(index) = c[2].parse::<u16>()
        {
            parts.entry(lower(&c[1])).or_default().push((index, name.clone()));
        }
    }
    for name in &names {
        let path = folder.join(name);
        let (stem, is_dir) = match dirs.captures(name) {
            Some(c) => (lower(&c[1]), true),
            None if chunks.is_match(name) => continue,
            None => (lower(name.trim_end_matches(".vpk").trim_end_matches(".VPK")), false),
        };
        let Ok(needed) = needed_chunks(&path) else { continue };
        let have: Vec<(u16, String)> = if is_dir { parts.get(&stem).cloned().unwrap_or_default() } else { Vec::new() };
        let mut files = vec![(path.clone(), format!("_dir.vpk{DISABLED_SUFFIX}"))];
        let mut complete = true;
        for index in &needed {
            match have.iter().find(|(i, _)| i == index) {
                Some((_, chunk)) => files.push((folder.join(chunk), format!("_{index:03}.vpk"))),
                None => complete = false,
            }
        }
        let shown: Vec<String> = files.iter().map(|(p, _)| citadel_relative(addons_dir, p)).collect();
        findings.push(Finding {
            id: format!("misplaced:{}", shown.join(",")),
            slot: None,
            kind: Kind::Misplaced { files: shown, subfolder },
            fix: if complete { Fix::Adopt { files } } else { Fix::None },
            // A mod Deadlock Mod Manager parked here: its Import keeps the name, a plain move
            // would not, so it is offered unticked.
            suggested: complete && !subfolder && !claimed.contains(&lower(name)),
        });
    }
}

/// File names (lower case) Deadlock Mod Manager's `.dmm.json` in `folder` lists for its mods.
fn dmm_claimed(folder: &Path) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(folder.join(".dmm.json")) else { return BTreeSet::new() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return BTreeSet::new() };
    let Some(mods) = value.get("mods").and_then(|m| m.as_object()) else { return BTreeSet::new() };
    mods.values()
        .flat_map(|entry| ["currentVpks", "disabledVpks"].map(|key| entry.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default()))
        .flatten()
        .filter_map(|name| name.as_str().map(lower))
        .collect()
}

/// Slot-named units (`pakNN_dir.vpk` and its parts) in a folder the game does not mount.
fn slot_units_in(addons_dir: &Path, folder: &Path, findings: &mut Vec<Finding>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    let names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for (local, files) in crate::load_order::group_slot_files(&names) {
        let stem = format!("pak{local:02}");
        let Some(main) = files.iter().find(|f| lower(f).starts_with(&format!("{stem}_dir.vpk"))).cloned() else { continue };
        let mut out = vec![(folder.join(&main), format!("_dir.vpk{DISABLED_SUFFIX}"))];
        for chunk in files.iter().filter(|f| **f != main && !lower(f).contains("_dir.vpk")) {
            let suffix = chunk[stem.len()..].to_string();
            out.push((folder.join(chunk), suffix));
        }
        let shown: Vec<String> = out.iter().map(|(p, _)| citadel_relative(addons_dir, p)).collect();
        findings.push(Finding {
            id: format!("misplaced:{}", shown.join(",")),
            slot: None,
            kind: Kind::Misplaced { files: shown, subfolder: false },
            fix: Fix::Adopt { files: out },
            suggested: true,
        });
    }
}

/// The lowest slot of the main folder with no file at all, past `taken`.
fn free_slot(addons_dir: &Path, taken: &BTreeSet<u16>) -> Result<Option<u16>> {
    let slots = crate::load_order::read_slot_files(addons_dir)?;
    Ok((1..=addons::SLOTS_PER_FOLDER).find(|s| !slots.contains_key(s) && !taken.contains(s)))
}

/// GAME WRITE. Fix the findings whose ids are in `chosen`, after reading the folders again:
/// only a finding that is still exactly as reviewed is fixed. Order: a stopped reorder first,
/// then removals to the trash, then moves into free slots, then the library, then the mounts.
/// A failure stops only its own finding. `library_path` is the library store the forgotten
/// entries leave. Refuses without `approve`.
pub fn apply(
    game_root: &Path,
    trash_root: &Path,
    library_path: &Path,
    reviewed: &[Finding],
    chosen: &BTreeSet<String>,
    approve: bool,
) -> Result<Report> {
    if !approve {
        return Err(Error::msg("Not approved: repair the mods folder. Nothing was changed."));
    }
    let addons_dir = crate::deploy::addons_dir(game_root);
    let mut library = crate::import::read_library(library_path).unwrap_or_default();
    let fresh = review(&addons_dir, &library)?;
    let wanted: Vec<Finding> = fresh
        .findings
        .into_iter()
        .filter(|f| chosen.contains(&f.id) && reviewed.contains(f))
        .collect();
    let mut report = Report::default();
    let rank = |f: &Finding| match f.fix {
        Fix::FinishReorder => 0,
        Fix::Trash { .. } => 1,
        Fix::Adopt { .. } => 2,
        Fix::Forget { .. } => 3,
        Fix::Mount => 4,
        Fix::None => 5,
    };
    let mut ordered = wanted;
    ordered.sort_by_key(rank);
    let mut whole_trashed = BTreeSet::new();
    let mut taken = BTreeSet::new();
    let mut mount = false;
    for finding in ordered {
        let result: Result<()> = match &finding.fix {
            Fix::FinishReorder => crate::load_order::recover_temp_files(&addons_dir, true).map(|r| {
                report.reorder_files += r.restored.len();
            }),
            Fix::Trash { slot, names } => {
                if whole_trashed.contains(slot) {
                    Ok(())
                } else if names.is_empty() {
                    let meta = library.mods.iter().rev().find(|m| m.slot == *slot).cloned();
                    trash::trash_slot(game_root, trash_root, *slot, meta, true).map(|entry| {
                        whole_trashed.insert(*slot);
                        library.mods.retain(|m| m.slot != *slot);
                        report.trashed.push(entry);
                    })
                } else {
                    trash::trash_named(game_root, trash_root, *slot, names, true).map(|entry| report.trashed.push(entry))
                }
            }
            Fix::Adopt { files } => adopt(&addons_dir, files, &mut taken, &mut report).map(|moved_overflow| {
                mount |= moved_overflow;
            }),
            Fix::Forget { slot } => {
                let gone: Vec<ModMeta> = library.mods.iter().filter(|m| m.slot == *slot).cloned().collect();
                library.mods.retain(|m| m.slot != *slot);
                report.forgotten.extend(gone);
                Ok(())
            }
            Fix::Mount => {
                mount = true;
                Ok(())
            }
            Fix::None => continue,
        };
        match result {
            Ok(()) => report.fixed.push(finding.id.clone()),
            Err(error) => report.errors.push((finding.id.clone(), error.to_string())),
        }
    }
    if !report.forgotten.is_empty() || !whole_trashed.is_empty() {
        crate::import::write_library(library_path, &library)?;
    }
    if mount || !report.moved.is_empty() || !report.trashed.is_empty() {
        match addons::sync_overflow_mount(&addons_dir) {
            Ok(changed) => report.mounted = changed,
            Err(error) if mount => report.errors.push(("mount".to_string(), error.to_string())),
            Err(_) => {}
        }
    }
    Ok(report)
}

/// Move one unit into the lowest free slot of the main folder, switched off. Returns whether a
/// file left an extra folder (the mounts then need a sync).
fn adopt(addons_dir: &Path, files: &[(PathBuf, String)], taken: &mut BTreeSet<u16>, report: &mut Report) -> Result<bool> {
    let slot = free_slot(addons_dir, taken)?.ok_or_else(|| Error::msg("No free slot in the addons folder. Delete or merge a mod first."))?;
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, suffix) in files {
        let to = addons::slot_path(addons_dir, slot, suffix);
        if fs::symlink_metadata(&to).is_ok() {
            rollback(&done);
            return Err(Error::msg(format!("{} is taken. Nothing was moved.", to.display())));
        }
        if let Err(error) = fs::rename(from, &to) {
            rollback(&done);
            return Err(Error::msg(format!("Moving {} failed ({error}). Nothing was moved.", from.display())));
        }
        done.push((from.clone(), to));
    }
    taken.insert(slot);
    let left_extra = files.iter().any(|(from, _)| from.parent() != Some(addons_dir));
    report.moved.extend(done);
    report.adopted.push(slot);
    Ok(left_extra)
}

fn rollback(done: &[(PathBuf, PathBuf)]) {
    for (from, to) in done.iter().rev() {
        let _ = fs::rename(to, from);
    }
}

/// What [`undo`] could not put back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoReport {
    pub restored: usize,
    pub skipped: Vec<String>,
}

/// GAME WRITE. Put a repair back: moved files go back where they were (when that name is free),
/// trash entries are restored, forgotten library entries come back. Refuses without `approve`.
pub fn undo(game_root: &Path, library_path: &Path, report: &Report, approve: bool) -> Result<UndoReport> {
    if !approve {
        return Err(Error::msg("Not approved: undo the repair. Nothing was changed."));
    }
    let mut out = UndoReport::default();
    for (from, to) in report.moved.iter().rev() {
        if fs::symlink_metadata(from).is_ok() || !to.is_file() {
            out.skipped.push(from.display().to_string());
            continue;
        }
        if let Some(parent) = from.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::rename(to, from) {
            Ok(()) => out.restored += 1,
            Err(_) => out.skipped.push(from.display().to_string()),
        }
    }
    let mut library = crate::import::read_library(library_path).unwrap_or_default();
    for entry in &report.trashed {
        match trash::restore(&entry.dir, true) {
            Ok(restored) => {
                out.restored += 1;
                if let Some(meta) = restored.meta {
                    library.mods.retain(|m| m.slot != meta.slot);
                    library.mods.push(meta);
                }
            }
            Err(error) => out.skipped.push(error.to_string()),
        }
    }
    for meta in &report.forgotten {
        if !library.mods.iter().any(|m| m.slot == meta.slot) {
            library.mods.push(meta.clone());
            out.restored += 1;
        }
    }
    library.mods.sort_by_key(|m| m.slot);
    crate::import::write_library(library_path, &library)?;
    let _ = addons::sync_overflow_mount(&crate::deploy::addons_dir(game_root));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::tests::{pack, pack_split};

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("sigil-folder-repair-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let addons = root.join("game/citadel/addons");
        fs::create_dir_all(&addons).unwrap();
        (root, addons)
    }

    fn split(addons: &Path, name: &str, files: &[(&str, &[u8])]) -> Vec<PathBuf> {
        pack_split(addons, name, files, 4)
    }

    fn meta(slot: u16, name: &str) -> ModMeta {
        ModMeta { slot, size_bytes: 1, name: name.to_string(), author: None, thumbnail_url: None, gamebanana_id: None, source: "local".into(), recorded_at: String::new() }
    }

    fn kinds(review: &Review) -> Vec<String> {
        review.findings.iter().map(|f| f.id.split(':').next().unwrap().to_string()).collect()
    }

    #[test]
    fn finds_each_broken_state() {
        let (root, addons) = scratch("finds");
        // 02: a split mod with a missing part.
        let parts = split(&addons, "pak02", &[("a/a.txt", b"aaaa"), ("b/b.txt", b"bbbb")]);
        fs::remove_file(&parts[1]).unwrap();
        // 03: chunks with no directory file.
        fs::write(addons.join("pak03_000.vpk"), b"x").unwrap();
        // 04: the same mod on and off.
        pack(&addons, "pak04", &[("c.txt", b"c")]);
        fs::copy(addons.join("pak04_dir.vpk"), addons.join("pak04_dir.vpk.disabled")).unwrap();
        // 05: an empty directory file.
        fs::write(addons.join("pak05_dir.vpk"), b"").unwrap();
        // A wrong name, and a copy of 04 under 06.
        pack(&addons, "mymod", &[("d.txt", b"d")]);
        fs::rename(addons.join("mymod_dir.vpk"), addons.join("mymod.vpk")).unwrap();
        pack(&addons, "pak06", &[("c.txt", b"c")]);
        let library = Library { schema: 1, mods: vec![meta(9, "Gone"), meta(4, "Kept")] };
        let review = review(&addons, &library).unwrap();
        let found = kinds(&review);
        for want in ["chunks", "orphan", "doubled", "unreadable", "gone", "misplaced", "duplicate"] {
            assert!(found.contains(&want.to_string()), "{want} in {found:?}");
        }
        let doubled = review.findings.iter().find(|f| f.id.starts_with("doubled")).unwrap();
        assert_eq!(doubled.kind, Kind::Doubled { same: true });
        let dup = review.findings.iter().find(|f| f.id.starts_with("duplicate")).unwrap();
        assert_eq!((dup.slot, &dup.kind), (Some(6), &Kind::Duplicate { keep: 4 }), "the library's copy is kept");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_fixes_the_ticked_ones_and_undo_puts_them_back() {
        let (root, addons) = scratch("apply");
        let game = root.clone();
        fs::write(addons.join("pak03_000.vpk"), b"x").unwrap();
        pack(&addons, "mymod", &[("d.txt", b"d")]);
        fs::rename(addons.join("mymod_dir.vpk"), addons.join("mymod.vpk")).unwrap();
        pack(&addons, "pak01", &[("e.txt", b"e")]);
        let library_path = root.join("data/library.json");
        crate::import::write_library(&library_path, &Library { schema: 1, mods: vec![meta(9, "Gone")] }).unwrap();
        let library = crate::import::read_library(&library_path).unwrap();
        let reviewed = review(&addons, &library).unwrap().findings;
        let chosen: BTreeSet<String> = reviewed.iter().filter(|f| f.suggested).map(|f| f.id.clone()).collect();
        assert!(apply(&game, &root.join("trash"), &library_path, &reviewed, &chosen, false).is_err());
        let report = apply(&game, &root.join("trash"), &library_path, &reviewed, &chosen, true).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.fixed.len(), 3);
        assert!(!addons.join("pak03_000.vpk").exists());
        assert_eq!(report.adopted, vec![2], "the lowest free slot");
        assert!(addons.join("pak02_dir.vpk.disabled").is_file());
        assert!(crate::import::read_library(&library_path).unwrap().mods.is_empty());
        assert!(review(&addons, &crate::import::read_library(&library_path).unwrap()).unwrap().is_clean());

        let back = undo(&game, &library_path, &report, true).unwrap();
        assert!(back.skipped.is_empty(), "{:?}", back.skipped);
        assert!(addons.join("pak03_000.vpk").is_file());
        assert!(addons.join("mymod.vpk").is_file());
        assert_eq!(crate::import::read_library(&library_path).unwrap().mods.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_changed_finding_is_not_applied() {
        let (root, addons) = scratch("changed");
        fs::write(addons.join("pak03_000.vpk"), b"x").unwrap();
        let library_path = root.join("library.json");
        let reviewed = review(&addons, &Library::default()).unwrap().findings;
        fs::write(addons.join("pak03_001.vpk"), b"y").unwrap();
        let chosen: BTreeSet<String> = reviewed.iter().map(|f| f.id.clone()).collect();
        let report = apply(&root, &root.join("trash"), &library_path, &reviewed, &chosen, true).unwrap();
        assert!(report.fixed.is_empty());
        assert!(addons.join("pak03_000.vpk").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn subfolders_are_offered_unticked_and_dmm_folders_never() {
        let (root, addons) = scratch("subfolders");
        let sub = addons.join("downloaded");
        fs::create_dir_all(&sub).unwrap();
        pack(&sub, "pak01", &[("f.txt", b"f")]);
        fs::rename(sub.join("pak01_dir.vpk"), sub.join("cool_dir.vpk")).unwrap();
        let dmm = addons.join("default");
        fs::create_dir_all(&dmm).unwrap();
        fs::write(dmm.join(".dmm.json"), b"{}").unwrap();
        pack(&dmm, "pak01", &[("g.txt", b"g")]);
        fs::rename(dmm.join("pak01_dir.vpk"), dmm.join("other_dir.vpk")).unwrap();
        let review = review(&addons, &Library::default()).unwrap();
        assert_eq!(review.findings.len(), 1, "{:?}", review.findings);
        let finding = &review.findings[0];
        assert_eq!(finding.kind, Kind::Misplaced { files: vec!["addons/downloaded/cool_dir.vpk".into()], subfolder: true });
        assert!(!finding.suggested);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_mod_dmm_parked_in_addons_is_offered_unticked() {
        let (root, addons) = scratch("dmm-parked");
        pack(&addons, "pak01", &[("p.txt", b"p")]);
        fs::rename(addons.join("pak01_dir.vpk"), addons.join("504_pak01_dir.vpk")).unwrap();
        pack(&addons, "pak02", &[("q.txt", b"q")]);
        fs::rename(addons.join("pak02_dir.vpk"), addons.join("hand_dir.vpk")).unwrap();
        fs::write(
            addons.join(".dmm.json"),
            r#"{"version":3,"mods":{"504":{"enabled":false,"currentVpks":[],"disabledVpks":["504_pak01_dir.vpk"]}}}"#,
        )
        .unwrap();
        let review = review(&addons, &Library::default()).unwrap();
        let ticked: Vec<(String, bool)> = review
            .findings
            .iter()
            .filter_map(|f| match &f.kind {
                Kind::Misplaced { files, .. } => Some((files[0].clone(), f.suggested)),
                _ => None,
            })
            .collect();
        assert_eq!(
            ticked,
            vec![("addons/504_pak01_dir.vpk".to_string(), false), ("addons/hand_dir.vpk".to_string(), true)]
        );
        let _ = fs::remove_dir_all(root);
    }
}
