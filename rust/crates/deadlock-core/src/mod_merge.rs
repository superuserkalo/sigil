//! Merge several mods into one VPK, and take that apart again.
//!
//! Two mods that each replace a different hero can ship as one file, which frees a slot and
//! keeps them together. The merge copies each source's entries into one staging tree in priority
//! order (the highest slot first), so the lowest slot's files are written last and win exactly as
//! the game would load them; the result takes the lowest free slot and every source is switched
//! off, not deleted. Strict mode refuses instead of letting one source overwrite another.
//!
//! [`merge_mods`] and [`unmerge`] are the game-folder writes here. Both take `approve`, write
//! through temp files, never overwrite a file, and keep a sidecar record ([`MergedMod`]) so
//! [`unmerge`] can put the sources back on and delete the merged slot. The sources themselves
//! are only renamed to `.disabled`, so nothing a player had is lost.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::addons::{AddonToggleResult, DISABLED_SUFFIX, set_enabled};
use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::install::occupied_slots;
use crate::util::{now_iso, write_atomic};
use crate::vpk::{ReadVpkOptions, is_safe_vpk_path, read_vpk_directory_with, read_vpk_entry_data};
use crate::vpk_write::{PackOptions, pack_vpk};

pub const MERGED_FILE_NAME: &str = "merged-mods.json";
pub const MERGED_SCHEMA: u32 = 1;
/// Sources one merge may hold. A merge is a convenience, not a library.
pub const MAX_SOURCES: usize = 32;
/// Name length kept in a record.
pub const MAX_NAME: usize = 80;

/// What one source was when it went into a merge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeSource {
    pub slot: u16,
    /// File names as they were in the addon folder (the `.disabled` one included).
    pub files: Vec<String>,
    /// The name shown for the mod at the time.
    pub name: String,
    pub gamebanana_id: Option<u64>,
    pub sha256: Option<String>,
    /// Whether it was on when the merge ran.
    pub enabled: bool,
}

/// One merged slot on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedMod {
    pub id: String,
    pub created_at: String,
    pub name: String,
    /// The folder it was written into.
    pub addons_dir: PathBuf,
    /// The slot it took.
    pub slot: u16,
    pub files: Vec<String>,
    pub entries: usize,
    pub bytes: u64,
    pub strict: bool,
    pub sources: Vec<MergeSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedStore {
    pub schema: u32,
    pub merged: Vec<MergedMod>,
}

impl Default for MergedStore {
    fn default() -> Self {
        MergedStore {
            schema: MERGED_SCHEMA,
            merged: Vec::new(),
        }
    }
}

pub fn merged_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MERGED_FILE_NAME)
}

pub fn read_merged(path: &Path) -> MergedStore {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write_merged(path: &Path, store: &MergedStore) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(store)?)
}

impl MergedStore {
    pub fn for_dir(&self, addons: &Path) -> Vec<&MergedMod> {
        self.merged
            .iter()
            .filter(|merged| merged.addons_dir == addons)
            .collect()
    }

    pub fn find_slot(&self, addons: &Path, slot: u16) -> Option<&MergedMod> {
        self.for_dir(addons)
            .into_iter()
            .find(|merged| merged.slot == slot)
    }

    pub fn remove(&mut self, id: &str) {
        self.merged.retain(|merged| merged.id != id);
    }
}

/// A slot and the label the player sees, for a merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeInput {
    pub slot: u16,
    pub name: String,
}

/// What the merge dialog shows before anything is written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergePreview {
    /// Sources in the order they are applied: the lowest slot last, which is the winner.
    pub sources: Vec<MergeInput>,
    /// Distinct paths across the sources.
    pub paths: usize,
    /// Paths more than one source ships, with the slots in priority order.
    pub collisions: Vec<(String, Vec<u16>)>,
    pub bytes: u64,
    /// A source whose file could not be read.
    pub error: Option<String>,
}

/// Read the sources and work out what a merge would do. Read only, safe to run on a worker.
pub fn preview(sources: &[MergeInput], addons: &Path) -> MergePreview {
    let mut preview = MergePreview {
        sources: sources.to_vec(),
        ..MergePreview::default()
    };
    preview
        .sources
        .sort_by_key(|source| std::cmp::Reverse(source.slot));
    let mut owners: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    for source in &preview.sources {
        let file = crate::addons::slot_path(addons, source.slot, "_dir.vpk");
        let file = if file.is_file() {
            file
        } else {
            crate::addons::slot_path(addons, source.slot, &format!("_dir.vpk{DISABLED_SUFFIX}"))
        };
        let directory = match read_vpk_directory_with(&file, &ReadVpkOptions { keep_preload: false })
        {
            Ok(directory) => directory,
            Err(error) => {
                preview.error = Some(format!("Slot {:02}: {error}", source.slot));
                return preview;
            }
        };
        for entry in directory.entries {
            preview.bytes += u64::from(entry.total_length);
            owners
                .entry(entry.path)
                .or_default()
                .push(source.slot);
        }
    }
    preview.paths = owners.len();
    preview.collisions = owners
        .into_iter()
        .filter(|(_, slots)| slots.len() > 1)
        .collect();
    preview
}

/// The record id for a merge: the time and the slot, so two merges never share one.
fn new_id(slot: u16) -> String {
    let stamp = now_iso()
        .replace(['-', ':', '.', 'T', 'Z'], "")
        .chars()
        .take(14)
        .collect::<String>();
    format!("merge-{stamp}-{slot:02}")
}

/// A name a merge can carry: one line, at most [`MAX_NAME`] characters.
pub fn clean_name(name: &str) -> Result<String> {
    let name = name.trim().replace(['\r', '\n'], " ");
    let name: String = name.chars().take(MAX_NAME).collect();
    if name.trim().is_empty() {
        return Err(Error::msg("Give the merged mod a name first."));
    }
    Ok(name.trim().to_string())
}

/// Merge the slots into one new slot. Game install write; see the module docs.
///
/// `sources` must hold two to [`MAX_SOURCES`] installed slots. The merged VPK lands switched off
/// in the lowest free slot, its file name in `store` records what it was built from, then every
/// source that is on is switched off with [`set_enabled`]. All or nothing: when a source cannot
/// be switched off (Deadlock holds its file open on Windows), the sources already switched off
/// go back on and the merged files Sigil just wrote are removed again, so the sources and the
/// merge never both load.
#[allow(clippy::too_many_arguments)]
pub fn merge_mods(
    game_root: &Path,
    workspace: &Path,
    store_path: &Path,
    sources: &[MergeInput],
    name: &str,
    strict: bool,
    approve: bool,
) -> Result<MergedMod> {
    if !approve {
        return Err(Error::msg(
            "Not approved: merge these mods. Nothing was written.",
        ));
    }
    let name = clean_name(name)?;
    if sources.len() < 2 {
        return Err(Error::msg(
            "Pick at least two mods to merge. Nothing was written.",
        ));
    }
    if sources.len() > MAX_SOURCES {
        return Err(Error::msg(format!(
            "A merge holds at most {MAX_SOURCES} mods. Nothing was written."
        )));
    }
    let mut slots: BTreeSet<u16> = BTreeSet::new();
    for source in sources {
        if !(1..=crate::addons::MAX_SLOT).contains(&source.slot) || !slots.insert(source.slot) {
            return Err(Error::msg(format!(
                "Slot {:02} is named twice or is not an addon slot. Nothing was written.",
                source.slot
            )));
        }
    }

    let addons = addons_dir(game_root);
    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "The addon folder {} does not exist. Nothing was written.",
            addons.display()
        )));
    }
    let preview = preview(sources, &addons);
    if let Some(error) = &preview.error {
        return Err(Error::msg(format!("{error} Nothing was written.")));
    }
    if strict && !preview.collisions.is_empty() {
        let first = &preview.collisions[0];
        return Err(Error::msg(format!(
            "{} mods replace {}. Turn off Strict or pick other mods. Nothing was written.",
            first.1.len(),
            first.0
        )));
    }

    let used = occupied_slots(&addons)?;
    let Some(slot) = (1..=crate::addons::MAX_SLOT).find(|slot| !used.contains(slot)) else {
        return Err(Error::msg(
            "Every addon slot is taken. Nothing was written.",
        ));
    };

    let token = crate::load_order::new_token();
    let work = workspace.join(format!("merge-{token}"));
    let staging = work.join("staging");
    let out = work.join("out");
    let outcome = (|| -> Result<(Vec<String>, usize, u64)> {
        fs::create_dir_all(&staging)?;
        // Highest slot first: it is applied first, so a lower slot replaces its files.
        for source in &preview.sources {
            let file = slot_dir_vpk(&addons, source.slot)?;
            let directory =
                read_vpk_directory_with(&file, &ReadVpkOptions { keep_preload: false })?;
            for entry in &directory.entries {
                if !is_safe_vpk_path(&entry.path) {
                    return Err(Error::msg(format!(
                        "{} holds the unsafe path {}. Nothing was written.",
                        file.display(),
                        entry.path
                    )));
                }
                let target = staging.join(&entry.path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let data = read_vpk_entry_data(&directory, entry)?;
                write_atomic(&target, &data)?;
            }
        }
        let packed = pack_vpk(&PackOptions::new(staging.clone(), out.clone(), "merged"))?;
        let mut files = Vec::new();
        for chunk in &packed.chunk_paths {
            let index = chunk
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.rsplit('_').next())
                .and_then(|tail| tail.strip_suffix(".vpk"))
                .unwrap_or("000");
            let target = crate::addons::slot_path(&addons, slot, &format!("_{index}.vpk"));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_new(chunk, &target)?;
            files.push(file_name(&target));
        }
        let target = crate::addons::slot_path(&addons, slot, &format!("_dir.vpk{DISABLED_SUFFIX}"));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = target.with_file_name(format!("{}.sigil-tmp", file_name(&target)));
        if let Err(error) = fs::copy(&packed.dir_path, &temporary) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        if target.exists() {
            let _ = fs::remove_file(&temporary);
            return Err(Error::msg(format!(
                "{} appeared while merging. Nothing was overwritten.",
                target.display()
            )));
        }
        fs::rename(&temporary, &target)?;
        files.push(file_name(&target));
        Ok((files, packed.entry_count, packed.total_bytes))
    })();

    let (files, entries, bytes) = match outcome {
        Ok(done) => done,
        Err(error) => {
            let _ = fs::remove_dir_all(&work);
            return Err(error);
        }
    };
    let _ = fs::remove_dir_all(&work);

    // Every source that is on is switched off, keeping its files. A failure takes the merge
    // back: the switched sources go on again and the new merged files (never loaded, they
    // landed switched off) are removed.
    let mut switched = Vec::new();
    for source in &preview.sources {
        if !crate::addons::slot_path(&addons, source.slot, "_dir.vpk").is_file() {
            continue;
        }
        match set_enabled(game_root, source.slot, false, true) {
            Ok(result) => switched.push(result),
            Err(error) => {
                let reverted = crate::addons::revert_toggles(game_root, &switched, true).is_ok();
                let folder = crate::addons::slot_folder(&addons, slot);
                for name in &files {
                    let _ = fs::remove_file(folder.join(name));
                }
                return Err(Error::msg(if reverted {
                    format!("{error} The merge was taken back: every mod is as it was.")
                } else {
                    format!("{error} The merge was taken back, but some mods stayed off: turn them on again.")
                }));
            }
        }
    }

    let sources_record: Vec<MergeSource> = preview
        .sources
        .iter()
        .map(|source| MergeSource {
            slot: source.slot,
            files: slot_file_names(&addons, source.slot),
            name: source.name.clone(),
            gamebanana_id: None,
            sha256: None,
            enabled: true,
        })
        .collect();
    let merged = MergedMod {
        id: new_id(slot),
        created_at: now_iso(),
        name,
        addons_dir: addons,
        slot,
        files,
        entries,
        bytes,
        strict,
        sources: sources_record,
    };
    let mut store = read_merged(store_path);
    store.merged.retain(|other| other.slot != slot || other.addons_dir != merged.addons_dir);
    store.merged.push(merged.clone());
    write_merged(store_path, &store)?;
    // A merged mod past slot 99 lands in an extra folder that gameinfo.gi has to mount.
    if crate::addons::folder_index(slot) > 0 {
        crate::addons::sync_overflow_mount(&merged.addons_dir)?;
    }
    Ok(merged)
}

/// Put a merge back: the merged slot goes to Sigil's trash, its sources are switched on again,
/// and the record is dropped. Game install write; refuses without `approve`.
pub fn unmerge(
    game_root: &Path,
    trash_root: &Path,
    store_path: &Path,
    merged: &MergedMod,
    approve: bool,
) -> Result<Vec<AddonToggleResult>> {
    if !approve {
        return Err(Error::msg(
            "Not approved: take this merge apart. Nothing was written.",
        ));
    }
    let addons = addons_dir(game_root);
    if addons != merged.addons_dir {
        return Err(Error::msg(format!(
            "This merge was made in {}, not {}. Nothing was written.",
            merged.addons_dir.display(),
            addons.display()
        )));
    }
    // The record holds the name from when the merge landed switched off; it may be on by now.
    let folder = crate::addons::slot_folder(&addons, merged.slot);
    let files: Vec<String> = merged
        .files
        .iter()
        .map(|name| {
            let toggled = match name.strip_suffix(DISABLED_SUFFIX) {
                Some(on) => on.to_string(),
                None => format!("{name}{DISABLED_SUFFIX}"),
            };
            if !folder.join(name).exists() && folder.join(&toggled).exists() { toggled } else { name.clone() }
        })
        .collect();
    crate::trash::trash_named(game_root, trash_root, merged.slot, &files, true)?;

    let mut back = Vec::new();
    for source in &merged.sources {
        // The source files moved with their slot file names; find the slot by name.
        // The source kept its slot; a name only carries the number inside its folder.
        let slot = source.slot;
        if let Ok(result) = set_enabled(game_root, slot, true, true) {
            back.push(result);
        }
    }
    let mut store = read_merged(store_path);
    store.remove(&merged.id);
    write_merged(store_path, &store)?;
    Ok(back)
}

fn slot_dir_vpk(addons: &Path, slot: u16) -> Result<PathBuf> {
    let enabled = crate::addons::slot_path(addons, slot, "_dir.vpk");
    if enabled.is_file() {
        return Ok(enabled);
    }
    let disabled = crate::addons::slot_path(addons, slot, &format!("_dir.vpk{DISABLED_SUFFIX}"));
    if disabled.is_file() {
        return Ok(disabled);
    }
    Err(Error::msg(format!(
        "Slot {slot:02} has no directory VPK in {}. Scan again.",
        addons.display()
    )))
}

fn slot_file_names(addons: &Path, slot: u16) -> Vec<String> {
    crate::load_order::read_slot_files(addons)
        .ok()
        .and_then(|mut files| files.remove(&slot))
        .unwrap_or_default()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn copy_new(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        return Err(Error::msg(format!(
            "{} already exists. Nothing was overwritten.",
            target.display()
        )));
    }
    let temporary = target.with_file_name(format!("{}.sigil-tmp", file_name(target)));
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

/// The file a merge slot has, so a card can say what it was built from.
pub fn source_slots(merged: &MergedMod) -> Vec<u16> {
    merged.sources.iter().map(|source| source.slot).collect()
}

/// A slot number as the merge list shows it.
pub fn slots_label(slots: &[u16]) -> String {
    slots
        .iter()
        .map(|slot| format!("{slot:02}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The most packs "Pack all enabled" makes (DMM's mod compression offers 2 to 8).
pub const MAX_PACKS: usize = 8;

/// Split `mods` (enabled, sorted by slot) into at most `packs` runs of neighbouring slots, so
/// each pack keeps its mods' load order and the packs keep theirs. Runs differ by at most one
/// mod and none is a lone mod: one left outside would keep its low slot and beat the packs,
/// which land in later free slots. Pure.
pub fn pack_groups(mods: &[MergeInput], packs: usize) -> Vec<Vec<MergeInput>> {
    let packs = packs.clamp(1, MAX_PACKS);
    if mods.len() < 2 {
        return Vec::new();
    }
    let count = packs.min(mods.len() / 2);
    let (size, extra) = (mods.len() / count, mods.len() % count);
    let mut start = 0;
    (0..count)
        .map(|index| {
            let end = start + size + usize::from(index < extra);
            let run = mods[start..end].to_vec();
            start = end;
            run
        })
        .collect()
}

/// Pack every enabled mod into `packs` merged mods (DMM's unreleased mod compression, behaviour
/// only): [`pack_groups`] in slot order, each merged with [`merge_mods`] and switched on. Every
/// enabled slot goes in, past 99 too: one left out could sit below a pack and start beating
/// mods it used to lose to. The packs are made lowest slots first, so each takes a lower free
/// slot than the next and the load order holds. Every source stays on disk switched off;
/// [`unmerge`] takes a pack apart. Game install write.
///
/// All or nothing: a half-done run would leave finished packs in high slots under sources
/// still on in low ones, so on a failure the packs made so far are taken apart again. A pack
/// that cannot be taken apart is returned with the error.
pub fn pack_all(
    game_root: &Path,
    workspace: &Path,
    trash_root: &Path,
    store_path: &Path,
    enabled: &[MergeInput],
    packs: usize,
    approve: bool,
) -> (Vec<MergedMod>, Option<Error>) {
    if !approve {
        return (Vec::new(), Some(Error::msg("Not approved: pack the enabled mods. Nothing was written.")));
    }
    let mut mods: Vec<MergeInput> = enabled.to_vec();
    mods.sort_by_key(|m| m.slot);
    let installed = match crate::addons::list_installed(game_root) {
        Ok(installed) => installed,
        Err(error) => return (Vec::new(), Some(error)),
    };
    let on_disk: BTreeSet<u16> = installed.iter().filter(|addon| addon.enabled).map(|addon| addon.slot).collect();
    let asked: BTreeSet<u16> = mods.iter().map(|m| m.slot).collect();
    if on_disk != asked {
        return (
            Vec::new(),
            Some(Error::msg("The enabled mods changed. Scan again, then pack. Nothing was written.")),
        );
    }
    if mods.len() > packs.clamp(1, MAX_PACKS) * MAX_SOURCES {
        return (
            Vec::new(),
            Some(Error::msg(format!(
                "{} mods do not fit in {} packs of {MAX_SOURCES}. Nothing was written.",
                mods.len(),
                packs
            ))),
        );
    }
    let groups = pack_groups(&mods, packs);
    let free = (crate::addons::MAX_SLOT as usize).saturating_sub(installed.len());
    if free < groups.len() {
        return (
            Vec::new(),
            Some(Error::msg(format!(
                "{} packs need {} free slots and {free} are left. Nothing was written.",
                groups.len(),
                groups.len()
            ))),
        );
    }
    let total = groups.len();
    let mut made: Vec<MergedMod> = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let name = format!("Pack {} of {total}", index + 1);
        let failure = match merge_mods(game_root, workspace, store_path, group, &name, false, true) {
            Ok(merged) => {
                let switched = set_enabled(game_root, merged.slot, true, true);
                made.push(merged);
                switched.err()
            }
            Err(error) => Some(error),
        };
        if let Some(error) = failure {
            return roll_back(game_root, trash_root, store_path, made, error);
        }
    }
    (made, None)
}

/// Take the packs of a failed run apart, newest first, so every source is back on.
fn roll_back(
    game_root: &Path,
    trash_root: &Path,
    store_path: &Path,
    made: Vec<MergedMod>,
    error: Error,
) -> (Vec<MergedMod>, Option<Error>) {
    let mut stuck = Vec::new();
    for merged in made.into_iter().rev() {
        if unmerge(game_root, trash_root, store_path, &merged, true).is_err() {
            stuck.push(merged);
        }
    }
    if stuck.is_empty() {
        return (Vec::new(), Some(Error::msg(format!("{error} Your mods are back as they were."))));
    }
    stuck.reverse();
    let message = format!(
        "{error} {} pack(s) could not be taken apart: use Unmerge in the merge list to restore the load order.",
        stuck.len()
    );
    (stuck, Some(Error::msg(message)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk::read_vpk_directory;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn root(tag: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/merge-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn game(root: &Path) -> PathBuf {
        let game = root.join("Deadlock");
        fs::create_dir_all(game.join("game/citadel/addons")).unwrap();
        game
    }

    fn vpk_into(addons: &Path, name: &str, files: &[(&str, &[u8])]) {
        let stage = addons.join(format!(".work-{name}"));
        for (path, data) in files {
            let target = stage.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        let out = addons.join(format!(".out-{name}"));
        let packed = pack_vpk(&PackOptions::new(stage, out.clone(), name)).unwrap();
        fs::rename(&packed.dir_path, addons.join(format!("{name}_dir.vpk"))).unwrap();
        for chunk in &packed.chunk_paths {
            let index = chunk.file_name().unwrap().to_str().unwrap().rsplit('_').next().unwrap();
            fs::copy(chunk, addons.join(format!("{name}_{index}"))).unwrap();
        }
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(addons.join(format!(".work-{name}")));
    }

    fn inputs() -> Vec<MergeInput> {
        vec![
            MergeInput { slot: 1, name: "Atlas".into() },
            MergeInput { slot: 2, name: "Billy".into() },
        ]
    }

    #[test]
    fn merges_into_a_free_slot_and_lower_slot_wins_a_shared_path() {
        let root = root("merge");
        let game = game(&root);
        let addons = addons_dir(&game);
        vpk_into(&addons, "pak01", &[("materials/a.vmat_c", b"first"), ("materials/b.vmat_c", b"one")]);
        vpk_into(&addons, "pak02", &[("materials/a.vmat_c", b"second"), ("materials/c.vmat_c", b"two")]);

        let store = merged_path(&root.join("app"));
        let preview = preview(&inputs(), &addons);
        assert_eq!(preview.paths, 3);
        assert_eq!(preview.collisions.len(), 1);
        assert_eq!(preview.sources[0].slot, 2, "highest slot first");
        assert!(preview.error.is_none());

        // Strict refuses the shared path.
        let refused = merge_mods(&game, &root.join("work"), &store, &inputs(), "Both", true, true);
        assert!(refused.unwrap_err().to_string().contains("Strict"));
        assert!(merge_mods(&game, &root.join("work"), &store, &inputs(), "Both", false, false).is_err());

        let merged = merge_mods(&game, &root.join("work"), &store, &inputs(), "Both", false, true).unwrap();
        assert_eq!(merged.slot, 3, "the lowest free slot");
        assert_eq!(merged.sources.len(), 2);
        assert_eq!(merged.entries, 3);
        assert!(addons.join("pak03_dir.vpk.disabled").is_file());
        assert!(!addons.join("pak03_dir.vpk").exists(), "it lands switched off");
        // Both sources went off, and their files stayed.
        assert!(addons.join("pak01_dir.vpk.disabled").is_file());
        assert!(addons.join("pak02_dir.vpk.disabled").is_file());

        // The merged file holds the lower slot's copy of the shared path.
        let directory = read_vpk_directory(&addons.join("pak03_dir.vpk.disabled")).unwrap();
        let entry = directory.by_path.get("materials/a.vmat_c").unwrap();
        assert_eq!(read_vpk_entry_data(&directory, entry).unwrap(), b"first");

        let record = read_merged(&store);
        assert_eq!(record.for_dir(&addons).len(), 1);

        // Unmerge puts the sources back on and removes the merged slot to the trash.
        let trash = root.join("trash");
        let back = unmerge(&game, &trash, &store, &merged, true).unwrap();
        assert_eq!(back.len(), 2);
        assert!(addons.join("pak01_dir.vpk").is_file());
        assert!(addons.join("pak02_dir.vpk").is_file());
        assert!(!addons.join("pak03_dir.vpk.disabled").exists());
        assert!(read_merged(&store).for_dir(&addons).is_empty());
        assert_eq!(crate::trash::list_trash(&trash).len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pack_groups_keep_neighbours_together_and_leave_no_lone_mod() {
        let mods: Vec<MergeInput> =
            (1..=7).map(|slot| MergeInput { slot, name: format!("M{slot}") }).collect();
        let slots = |groups: Vec<Vec<MergeInput>>| -> Vec<Vec<u16>> {
            groups.iter().map(|g| g.iter().map(|m| m.slot).collect()).collect()
        };
        assert_eq!(slots(pack_groups(&mods, 2)), vec![vec![1, 2, 3, 4], vec![5, 6, 7]]);
        assert_eq!(slots(pack_groups(&mods, 3)), vec![vec![1, 2, 3], vec![4, 5], vec![6, 7]]);
        assert_eq!(slots(pack_groups(&mods[..3], 4)), vec![vec![1, 2, 3]], "more packs than pairs");
        assert_eq!(slots(pack_groups(&mods, 1)), vec![(1..=7).collect::<Vec<u16>>()]);
        assert!(pack_groups(&mods[..1], 4).is_empty());
    }

    #[test]
    fn pack_all_merges_in_order_and_turns_the_packs_on() {
        let root = root("pack-all");
        let game = game(&root);
        let addons = addons_dir(&game);
        for (slot, data) in [(1, b"a" as &[u8]), (2, b"b"), (3, b"c"), (4, b"d")] {
            vpk_into(&addons, &format!("pak0{slot}"), &[("materials/shared.vmat_c", data)]);
        }
        let store = merged_path(&root.join("app"));
        let enabled: Vec<MergeInput> =
            (1..=4).map(|slot| MergeInput { slot, name: format!("M{slot}") }).collect();
        let trash = root.join("trash");
        assert!(pack_all(&game, &root.join("w"), &trash, &store, &enabled, 2, false).1.is_some());
        let (made, error) = pack_all(&game, &root.join("w"), &trash, &store, &enabled, 2, true);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(made.iter().map(|m| m.slot).collect::<Vec<_>>(), vec![5, 6]);
        assert!(addons.join("pak05_dir.vpk").is_file(), "packs are on");
        assert!(addons.join("pak01_dir.vpk.disabled").is_file(), "sources are off");
        // Pack 1 holds slot 1's copy: the lowest slot still wins.
        let directory = read_vpk_directory(&addons.join("pak05_dir.vpk")).unwrap();
        let entry = directory.by_path.get("materials/shared.vmat_c").unwrap();
        assert_eq!(read_vpk_entry_data(&directory, entry).unwrap(), b"a");
        // A pack that is on comes apart too: its record holds the switched-off name.
        for merged in made.iter().rev() {
            unmerge(&game, &trash, &store, merged, true).unwrap();
        }
        for slot in 1..=4 {
            assert!(addons.join(format!("pak0{slot}_dir.vpk")).is_file(), "slot {slot} is back on");
        }
        assert!(!addons.join("pak05_dir.vpk").exists() && !addons.join("pak06_dir.vpk").exists());
        let _ = fs::remove_dir_all(&root);
    }

    /// The bug this guards: a mod past slot 99 stayed out of the packs, and a pack landing after
    /// it (slots 1 to 99 full) lost to it, so a mod that used to lose started to win.
    #[test]
    fn pack_all_takes_mods_past_slot_99_so_the_order_holds() {
        let root = root("pack-past-99");
        let game = game(&root);
        let addons = addons_dir(&game);
        vpk_into(&addons, "pak01", &[("materials/shared.vmat_c", b"first")]);
        vpk_into(&addons, "pak02", &[("materials/two.vmat_c", b"two")]);
        for slot in 3..=99 {
            fs::write(addons.join(format!("pak{slot:02}_dir.vpk.disabled")), b"off").unwrap();
        }
        let hundred = crate::addons::slot_path(&addons, 100, "_dir.vpk");
        let folder = hundred.parent().unwrap().to_path_buf();
        fs::create_dir_all(&folder).unwrap();
        let name = hundred.file_name().unwrap().to_str().unwrap().strip_suffix("_dir.vpk").unwrap().to_string();
        vpk_into(&folder, &name, &[("materials/shared.vmat_c", b"late")]);

        let store = merged_path(&root.join("app"));
        let enabled: Vec<MergeInput> =
            [1, 2, 100].into_iter().map(|slot| MergeInput { slot, name: format!("M{slot}") }).collect();
        let (made, error) = pack_all(&game, &root.join("w"), &root.join("trash"), &store, &enabled, 1, true);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].slot, 101, "1 to 100 are taken");
        assert!(!hundred.is_file(), "slot 100 went into the pack and is off");
        let pack = crate::addons::slot_path(&addons, 101, "_dir.vpk");
        let directory = read_vpk_directory(&pack).unwrap();
        let entry = directory.by_path.get("materials/shared.vmat_c").unwrap();
        assert_eq!(read_vpk_entry_data(&directory, entry).unwrap(), b"first", "slot 1 still wins");
        let _ = fs::remove_dir_all(&root);
    }

    /// A source that will not switch off (the game holds it on Windows) takes the merge back.
    #[test]
    fn a_source_that_will_not_switch_off_takes_the_merge_back() {
        let root = root("merge-held");
        let game = game(&root);
        let addons = addons_dir(&game);
        vpk_into(&addons, "pak01", &[("materials/a.vmat_c", b"first")]);
        vpk_into(&addons, "pak02", &[("materials/c.vmat_c", b"two")]);
        // Slot 01 has both names, so its switch off is refused after slot 02 went off.
        fs::write(addons.join("pak01_dir.vpk.disabled"), b"old copy").unwrap();
        let store = merged_path(&root.join("app"));
        let error = merge_mods(&game, &root.join("work"), &store, &inputs(), "Both", false, true).unwrap_err();
        assert!(error.to_string().contains("every mod is as it was"), "{error}");
        assert!(addons.join("pak01_dir.vpk").is_file());
        assert!(addons.join("pak02_dir.vpk").is_file(), "slot 02 is on again");
        assert!(!addons.join("pak02_dir.vpk.disabled").exists());
        assert!(!addons.join("pak03_dir.vpk.disabled").exists(), "the merged file is gone");
        assert!(read_merged(&store).merged.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_pack_all_puts_every_mod_back() {
        let root = root("pack-rollback");
        let game = game(&root);
        let addons = addons_dir(&game);
        for slot in 1..=3 {
            vpk_into(&addons, &format!("pak0{slot}"), &[("materials/shared.vmat_c", b"x")]);
        }
        fs::write(addons.join("pak04_dir.vpk"), b"not a vpk").unwrap();
        let store = merged_path(&root.join("app"));
        let enabled: Vec<MergeInput> =
            (1..=4).map(|slot| MergeInput { slot, name: format!("M{slot}") }).collect();
        let (made, error) = pack_all(&game, &root.join("w"), &root.join("trash"), &store, &enabled, 2, true);
        assert!(made.is_empty());
        assert!(error.unwrap().to_string().contains("back as they were"));
        for slot in 1..=3 {
            assert!(addons.join(format!("pak0{slot}_dir.vpk")).is_file(), "slot {slot} is on again");
        }
        assert!(!addons.join("pak05_dir.vpk").exists() && !addons.join("pak05_dir.vpk.disabled").exists());
        assert!(read_merged(&store).merged.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pack_all_refuses_a_stale_list() {
        let root = root("pack-stale");
        let game = game(&root);
        let addons = addons_dir(&game);
        for slot in 1..=3 {
            vpk_into(&addons, &format!("pak0{slot}"), &[("materials/a.vmat_c", b"x")]);
        }
        let store = merged_path(&root.join("app"));
        let enabled: Vec<MergeInput> =
            (1..=2).map(|slot| MergeInput { slot, name: format!("M{slot}") }).collect();
        let (made, error) = pack_all(&game, &root.join("w"), &root.join("trash"), &store, &enabled, 1, true);
        assert!(made.is_empty() && error.unwrap().to_string().contains("Scan again"));
        assert!(addons.join("pak01_dir.vpk").is_file(), "nothing changed");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_a_single_source_an_empty_name_and_a_full_folder() {
        let root = root("refuse");
        let game = game(&root);
        let addons = addons_dir(&game);
        vpk_into(&addons, "pak01", &[("materials/a.vmat_c", b"a")]);
        let store = merged_path(&root.join("app"));
        let one = vec![MergeInput { slot: 1, name: "One".into() }];
        assert!(merge_mods(&game, &root.join("w"), &store, &one, "One", false, true).is_err());
        assert!(
            merge_mods(&game, &root.join("w"), &store, &inputs(), "  ", false, true)
                .unwrap_err()
                .to_string()
                .contains("name")
        );
        // Slot 02 has no file.
        assert!(merge_mods(&game, &root.join("w"), &store, &inputs(), "Both", false, true).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_preview_reports_an_unreadable_source() {
        let root = root("broken");
        let addons = root.join("addons");
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), b"not a vpk").unwrap();
        let preview = preview(&[MergeInput { slot: 1, name: "A".into() }], &addons);
        assert!(preview.error.is_some());
        let _ = fs::remove_dir_all(&root);
    }
}
