//! Whole-library tools behind Settings: a full backup of the addons folder (snapshot) and its
//! restore, clearing every mod (to Sigil's trash, with a recovery report), resetting Sigil's
//! own records of mods, wiping Sigil's mod data, copying the mods to another (developer) game
//! folder, and the list of files in the addons folder the game does not read.
//!
//! GAME WRITES here: [`restore_snapshot`], [`clear_all_mods`] and [`copy_mods_to`] change a
//! `game/citadel/addons` folder. Each takes `approve`, writes nothing without it, and keeps a
//! way back: a restore that replaces moves the current slots to the trash first, clearing
//! moves to the trash, and a copy into a folder that has mods moves them to the trash too.
//! [`create_snapshot`] only reads the game folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::deploy::addons_dir;
use crate::error::{Error, Result};
use crate::load_order::{read_slot_files, slot_of};

pub const SNAPSHOTS_DIR: &str = "snapshots";
pub const SNAPSHOT_MANIFEST: &str = "snapshot.json";
pub const SNAPSHOT_SCHEMA: u32 = 1;

/// What a full addons backup holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotManifest {
    pub schema: u32,
    pub created_at: String,
    /// The addons folder it was taken from.
    pub addons_dir: PathBuf,
    /// File names, as they were (`.disabled` kept).
    pub files: Vec<String>,
    pub bytes: u64,
    /// Sigil's library.json at the time, when there was one, so names come back too.
    #[serde(default)]
    pub library: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub dir: PathBuf,
    pub manifest: SnapshotManifest,
}

/// `<workspace>/.dah/install/snapshots`.
pub fn snapshots_root(workspace: &Path) -> PathBuf {
    crate::install::install_state_dir(workspace).join(SNAPSHOTS_DIR)
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string()
}

/// Copy every slot file of the addons folder (and Sigil's library.json when given) into a new
/// snapshot folder. Reads the game, writes only under `root`.
pub fn create_snapshot(game_root: &Path, root: &Path, library: Option<&Path>) -> Result<Snapshot> {
    let addons = addons_dir(game_root);
    let slots = read_slot_files(&addons)?;
    let files: Vec<String> = slots.into_values().flatten().collect();
    if files.is_empty() {
        return Err(Error::msg("The addons folder has no mods to back up."));
    }
    let mut id = stamp();
    let mut n = 1;
    while root.join(&id).exists() {
        id = format!("{}-{n}", stamp());
        n += 1;
    }
    let dir = root.join(&id);
    let partial = root.join(format!(".{id}.partial"));
    fs::create_dir_all(&partial)?;
    let mut bytes = 0;
    let result = (|| -> Result<()> {
        for name in &files {
            bytes += fs::copy(addons.join(name), partial.join(name))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&partial);
        return Err(error);
    }
    let with_library = match library {
        Some(path) if path.is_file() => fs::copy(path, partial.join(crate::import::LIBRARY_FILE_NAME)).is_ok(),
        _ => false,
    };
    let manifest = SnapshotManifest {
        schema: SNAPSHOT_SCHEMA,
        created_at: crate::util::now_iso(),
        addons_dir: addons,
        files,
        bytes,
        library: with_library,
    };
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| Error::msg(e.to_string()))?;
    crate::util::write_atomic(&partial.join(SNAPSHOT_MANIFEST), text.as_bytes())?;
    fs::rename(&partial, &dir)?;
    Ok(Snapshot { dir, manifest })
}

/// Read one snapshot folder.
pub fn read_snapshot(dir: &Path) -> Result<Snapshot> {
    let text = fs::read_to_string(dir.join(SNAPSHOT_MANIFEST))?;
    let manifest: SnapshotManifest =
        serde_json::from_str(&text).map_err(|e| Error::msg(format!("Not a Sigil backup: {e}")))?;
    Ok(Snapshot {
        dir: dir.to_path_buf(),
        manifest,
    })
}

/// Every snapshot under `root`, newest first.
pub fn list_snapshots(root: &Path) -> Vec<Snapshot> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Snapshot> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| read_snapshot(&e.path()).ok())
        .collect();
    out.sort_by(|a, b| b.manifest.created_at.cmp(&a.manifest.created_at));
    out
}

/// How a snapshot comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    /// The addons folder ends up exactly as the snapshot: current mods go to the trash first.
    Replace,
    /// Snapshot mods go into slots that are free; slots in use keep what they have.
    Merge,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotRestore {
    pub copied: Vec<String>,
    /// Snapshot files skipped because their slot was in use (merge).
    pub skipped: Vec<String>,
    /// Trash entries made for the slots a replace cleared.
    pub trashed: Vec<PathBuf>,
}

/// GAME WRITE. Put a snapshot back into the game's addons folder. Refuses without `approve`.
pub fn restore_snapshot(
    snapshot: &Path,
    game_root: &Path,
    trash_root: &Path,
    mode: RestoreMode,
    approve: bool,
) -> Result<SnapshotRestore> {
    if !approve {
        return Err(Error::msg("Not approved: restore the addons backup. Nothing was changed."));
    }
    let snap = read_snapshot(snapshot)?;
    let addons = addons_dir(game_root);
    fs::create_dir_all(&addons)?;
    let mut report = SnapshotRestore::default();
    let current = read_slot_files(&addons)?;
    if mode == RestoreMode::Replace {
        let slots: Vec<(u8, Option<crate::import::ModMeta>)> = current.keys().map(|s| (*s, None)).collect();
        let (entries, errors) = crate::trash::trash_slots(game_root, trash_root, &slots, true);
        if let Some((slot, error)) = errors.first() {
            return Err(Error::msg(format!(
                "Slot {slot:02} could not be cleared ({error}). {} slots went to the trash; nothing was copied.",
                entries.len()
            )));
        }
        report.trashed = entries.into_iter().map(|e| e.dir).collect();
    }
    let used: std::collections::BTreeSet<u8> = if mode == RestoreMode::Merge {
        current.keys().copied().collect()
    } else {
        Default::default()
    };
    for name in &snap.manifest.files {
        let Some(slot) = slot_of(name) else { continue };
        if name.contains(['/', '\\']) {
            continue;
        }
        if used.contains(&slot) {
            report.skipped.push(name.clone());
            continue;
        }
        let target = addons.join(name);
        let partial = addons.join(format!("{name}.dah-tmp"));
        fs::copy(snapshot.join(name), &partial)?;
        fs::rename(&partial, &target)?;
        report.copied.push(name.clone());
    }
    Ok(report)
}

/// What clearing every mod did, and how to get it back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// (slot, trash entry folder) for each slot moved out.
    pub moved: Vec<(u8, PathBuf)>,
    pub failed: Vec<(u8, String)>,
    pub bytes: u64,
}

impl RecoveryReport {
    /// Plain text for the clipboard or a file: what moved where.
    pub fn text(&self) -> String {
        let mut lines = vec![format!(
            "Sigil mod recovery report: {} slots moved to the trash, {} failed.",
            self.moved.len(),
            self.failed.len()
        )];
        for (slot, dir) in &self.moved {
            lines.push(format!("pak{slot:02}: {}", dir.display()));
        }
        for (slot, error) in &self.failed {
            lines.push(format!("pak{slot:02} stayed: {error}"));
        }
        lines.push("Restore any of them from Installed > Recently deleted.".into());
        lines.join("\n")
    }
}

/// GAME WRITE. Move every mod in the addons folder to Sigil's trash. Refuses without `approve`.
pub fn clear_all_mods(game_root: &Path, trash_root: &Path, approve: bool) -> Result<RecoveryReport> {
    if !approve {
        return Err(Error::msg("Not approved: clear all mods. Nothing was changed."));
    }
    let slots = read_slot_files(&addons_dir(game_root))?;
    let list: Vec<(u8, Option<crate::import::ModMeta>)> = slots.keys().map(|s| (*s, None)).collect();
    let (entries, failed) = crate::trash::trash_slots(game_root, trash_root, &list, true);
    Ok(RecoveryReport {
        bytes: entries.iter().map(|e| e.manifest.bytes).sum(),
        moved: entries.into_iter().map(|e| (e.manifest.slot, e.dir)).collect(),
        failed,
    })
}

/// What a reset of Sigil's records moved aside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetReport {
    /// Where the old records went, when there were any.
    pub kept_in: Option<PathBuf>,
    pub moved: Vec<String>,
    /// Folders deleted for good (download cache, install backups, trash) by a full wipe.
    pub deleted: Vec<PathBuf>,
    pub freed: u64,
}

/// Sigil's mod records: names and previews (`library.json`), the install journal and saved
/// install plans. They are moved into `<data>/reset/<stamp>/`, not deleted. Mods in the game
/// stay and show as unknown until matched again. Writes only Sigil's own folders.
pub fn reset_mod_records(data_dir: &Path, workspace: &Path) -> Result<ResetReport> {
    let candidates = [
        crate::import::library_path(data_dir),
        crate::install::journal_path(workspace),
        crate::install::plans_dir(workspace),
    ];
    let present: Vec<&PathBuf> = candidates.iter().filter(|p| p.exists()).collect();
    let mut report = ResetReport::default();
    if present.is_empty() {
        return Ok(report);
    }
    let keep = data_dir.join("reset").join(stamp());
    fs::create_dir_all(&keep)?;
    for path in present {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let to = keep.join(&name);
        if fs::rename(path, &to).is_err() {
            copy_tree(path, &to)?;
            remove_any(path)?;
        }
        report.moved.push(name);
    }
    report.kept_in = Some(keep);
    Ok(report)
}

/// [`reset_mod_records`], then delete the download cache, the install backups and the trash.
/// The game's addons folder is not touched. Writes only Sigil's own folders.
pub fn wipe_mod_data(data_dir: &Path, workspace: &Path, trash_root: &Path) -> Result<ResetReport> {
    let mut report = reset_mod_records(data_dir, workspace)?;
    for dir in [
        crate::install::downloads_dir(workspace),
        crate::install::backups_dir(workspace),
        trash_root.to_path_buf(),
    ] {
        if dir.exists() {
            report.freed += tree_size(&dir);
            remove_any(&dir)?;
            report.deleted.push(dir);
        }
    }
    Ok(report)
}

/// Files in the addons folder the game does not load: anything that is not a slot file.
/// Folders count too. Read only.
pub fn unknown_files(game_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(addons_dir(game_root)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| slot_of(name).is_none() && !name.starts_with('.') && !name.ends_with(".dah-tmp"))
        .collect();
    out.sort();
    out
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub copied: usize,
    pub bytes: u64,
    pub trashed: usize,
}

/// GAME WRITE (to `target_root`, a developer copy of the game). Make the target's addons
/// folder hold the same mods as the real one: its own mods go to the trash first, then every
/// slot file is copied. `target_root` must have `game/citadel` and must not be `game_root`.
pub fn copy_mods_to(game_root: &Path, target_root: &Path, trash_root: &Path, approve: bool) -> Result<CopyReport> {
    if !approve {
        return Err(Error::msg("Not approved: copy mods. Nothing was changed."));
    }
    let same = match (fs::canonicalize(game_root), fs::canonicalize(target_root)) {
        (Ok(a), Ok(b)) => a == b,
        _ => game_root == target_root,
    };
    if same {
        return Err(Error::msg("The copy needs a different folder than the game Sigil manages."));
    }
    if !target_root.join("game").join("citadel").is_dir() {
        return Err(Error::msg(format!(
            "{} has no game/citadel folder.",
            target_root.display()
        )));
    }
    let from = addons_dir(game_root);
    let to = addons_dir(target_root);
    fs::create_dir_all(&to)?;
    let existing = read_slot_files(&to)?;
    let list: Vec<(u8, Option<crate::import::ModMeta>)> = existing.keys().map(|s| (*s, None)).collect();
    let (trashed, errors) = crate::trash::trash_slots(target_root, trash_root, &list, true);
    if let Some((slot, error)) = errors.first() {
        return Err(Error::msg(format!("Slot {slot:02} in the copy could not be cleared: {error}")));
    }
    let mut report = CopyReport {
        trashed: trashed.len(),
        ..Default::default()
    };
    for name in read_slot_files(&from)?.into_values().flatten() {
        let partial = to.join(format!("{name}.dah-tmp"));
        report.bytes += fs::copy(from.join(&name), &partial)?;
        fs::rename(&partial, to.join(&name))?;
        report.copied += 1;
    }
    Ok(report)
}


// --- switch variants ---------------------------------------------------------------------

/// Other slots that hold a file of the same GameBanana mod as `new_slot`, from the library's
/// newest entry per slot.
pub fn sibling_slots(library: &crate::import::Library, mod_id: u64, new_slot: u8) -> Vec<u8> {
    let mut latest: std::collections::BTreeMap<u8, Option<u64>> = Default::default();
    for meta in &library.mods {
        latest.insert(meta.slot, meta.gamebanana_id);
    }
    latest
        .into_iter()
        .filter(|(slot, id)| *slot != new_slot && *id == Some(mod_id))
        .map(|(slot, _)| slot)
        .collect()
}

/// GAME WRITE. Grimoire's "Switch variants": after a new file of a mod lands in `new_slot`,
/// turn off the other files of the same mod that are on, and turn the new one on when any
/// were. Returns the slots turned off.
pub fn switch_variants(
    game_root: &Path,
    library: &crate::import::Library,
    mod_id: u64,
    new_slot: u8,
    approve: bool,
) -> Result<Vec<u8>> {
    if !approve {
        return Err(Error::msg("Not approved: switch variants. Nothing was changed."));
    }
    let files = read_slot_files(&addons_dir(game_root))?;
    let mut off = Vec::new();
    for slot in sibling_slots(library, mod_id, new_slot) {
        let on = files
            .get(&slot)
            .is_some_and(|names| names.iter().any(|n| n.eq_ignore_ascii_case(&format!("pak{slot:02}_dir.vpk"))));
        if on && crate::addons::set_enabled(game_root, slot, false, true).is_ok() {
            off.push(slot);
        }
    }
    if !off.is_empty() {
        let new_on = files
            .get(&new_slot)
            .is_some_and(|names| names.iter().any(|n| n.eq_ignore_ascii_case(&format!("pak{new_slot:02}_dir.vpk"))));
        if !new_on {
            let _ = crate::addons::set_enabled(game_root, new_slot, true, true);
        }
    }
    Ok(off)
}

// --- moving a folder -----------------------------------------------------------------------

/// Why `to` cannot take `from`'s contents: inside it, containing it, or not writable.
pub fn check_move_target(from: &Path, to: &Path) -> Result<()> {
    let canon = |p: &Path| {
        let mut rest = Vec::new();
        let mut cur = p.to_path_buf();
        loop {
            if let Ok(real) = fs::canonicalize(&cur) {
                return rest.iter().rev().fold(real, |acc: PathBuf, part| acc.join(part));
            }
            match (cur.file_name().map(|n| n.to_os_string()), cur.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name);
                    cur = parent.to_path_buf();
                }
                _ => return p.to_path_buf(),
            }
        }
    };
    let (a, b) = (canon(from), canon(to));
    if b.starts_with(&a) || a.starts_with(&b) {
        return Err(Error::msg("Pick a folder outside the current download cache."));
    }
    if b.components().any(|c| c.as_os_str().eq_ignore_ascii_case("addons")) {
        return Err(Error::msg("Pick a folder outside the game's addons folder."));
    }
    fs::create_dir_all(to)?;
    let probe = to.join(".sigil-write-test");
    fs::write(&probe, b"ok").map_err(|e| Error::msg(format!("{} is not writable: {e}", to.display())))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Move every entry of `from` into `to` (a rename, or a copy and delete across drives). On a
/// failure the entries already moved go back. Sigil's own folders only.
pub fn move_contents(from: &Path, to: &Path) -> Result<u64> {
    fs::create_dir_all(to)?;
    let entries: Vec<PathBuf> = match fs::read_dir(from) {
        Ok(e) => e.flatten().map(|e| e.path()).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut bytes = 0;
    for entry in entries {
        let target = to.join(entry.file_name().unwrap_or_default());
        if target.exists() {
            undo_moves(&moved);
            return Err(Error::msg(format!("{} is already in the new folder.", target.display())));
        }
        let size = tree_size(&entry);
        let result = fs::rename(&entry, &target).or_else(|_| {
            copy_tree(&entry, &target).and_then(|_| remove_any(&entry)).map_err(|e| {
                let _ = remove_any(&target);
                std::io::Error::other(e.to_string())
            })
        });
        if let Err(error) = result {
            undo_moves(&moved);
            return Err(Error::msg(format!("Moving {} failed: {error}", entry.display())));
        }
        bytes += size;
        moved.push((entry, target));
    }
    Ok(bytes)
}

fn undo_moves(moved: &[(PathBuf, PathBuf)]) {
    for (from, to) in moved.iter().rev() {
        if fs::rename(to, from).is_err() && copy_tree(to, from).is_ok() {
            let _ = remove_any(to);
        }
    }
}

/// Point `link` at `target`: a symlink on Unix, a junction on Windows (no admin needed).
pub fn link_dir(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .creation_flags(0x0800_0000)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::msg("Windows could not link the folder (mklink /J)."))
        }
    }
}

/// Whether `path` is a link (symlink or junction) rather than a real folder.
pub fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
        || (cfg!(windows) && fs::read_link(path).is_ok())
}

/// Move the folder at `dir` to `to` and leave a link at `dir`, so every reader of the old path
/// keeps working. Sigil's own folders only (the download cache).
pub fn relocate(dir: &Path, to: &Path) -> Result<u64> {
    if is_link(dir) {
        return Err(Error::msg("The folder was already moved. Move it back first."));
    }
    check_move_target(dir, to)?;
    let bytes = move_contents(dir, to)?;
    let _ = fs::remove_dir(dir);
    if let Err(error) = link_dir(to, dir) {
        let _ = fs::create_dir_all(dir);
        let _ = move_contents(to, dir);
        return Err(error);
    }
    Ok(bytes)
}

/// Undo [`relocate`]: remove the link at `dir` and move the contents back from `from`.
pub fn relocate_back(dir: &Path, from: &Path) -> Result<u64> {
    if is_link(dir) {
        // A junction or symlink to a folder is removed without touching its target.
        if fs::remove_file(dir).is_err() {
            fs::remove_dir(dir)?;
        }
    }
    fs::create_dir_all(dir)?;
    move_contents(from, dir)
}


// --- imprint ---------------------------------------------------------------------------------

/// Who a mod is, written into its VPK so it can be recognised after it is copied or renamed.
/// Grimoire's "Imprint installed mods" (MIT) writes the same two root entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imprint {
    pub title: String,
    pub author: String,
    pub gamebanana_id: Option<u64>,
    pub source_url: String,
}

fn kv_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], " ")
}

/// `addoninfo.txt` (KeyValues) and `modinfo.json` for an imprint, given the original bytes'
/// sha256 and size.
pub fn imprint_files(info: &Imprint, sha256: &str, size: u64) -> (String, String) {
    let id = info.gamebanana_id.map(|i| i.to_string()).unwrap_or_default();
    let now = crate::util::now_iso();
    let addoninfo = format!(
        "\"AddonInfo\"\n{{\n    addonversion \"1.0\"\n    addontitle \"{}\"\n    addonauthor \"{}\"\n    gamebananaId \"{}\"\n    sourceUrl \"{}\"\n    buildDate \"{}\"\n    originalSha256 \"{}\"\n    originalSize \"{}\"\n    modinfoVersion \"1\"\n}}\n",
        kv_escape(&info.title),
        kv_escape(&info.author),
        id,
        kv_escape(&info.source_url),
        now,
        sha256,
        size
    );
    let modinfo = serde_json::json!({
        "format": "vpk-modinfo",
        "schemaVersion": 1,
        "kind": "mod",
        "writtenBy": {"tool": "sigil", "version": env!("CARGO_PKG_VERSION")},
        "writtenAt": now,
        "game": {"name": "Deadlock", "steamAppId": 1422450, "gameBananaGameId": 20948},
        "identity": {"sha256": sha256, "size": size},
        "title": info.title,
        "author": info.author,
        "source": {"gamebananaId": info.gamebanana_id, "url": info.source_url},
    });
    (addoninfo, serde_json::to_string_pretty(&modinfo).unwrap_or_default() + "\n")
}

/// GAME WRITE. Repack a mod's `pakNN_dir.vpk` (and its chunk files) with `addoninfo.txt` and
/// `modinfo.json` at its root. Every other entry keeps its bytes. A VPK already imprinted is
/// left alone (false). Each new file replaces the old one atomically; chunk files the new pack
/// no longer needs are removed. `path` may end in `.disabled`; the name is kept.
pub fn imprint_vpk(path: &Path, info: &Imprint, approve: bool) -> Result<bool> {
    if !approve {
        return Err(Error::msg("Not approved: imprint the mod. Nothing was changed."));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(base) = name.find("_dir.vpk").map(|at| name[..at].to_string()) else {
        return Err(Error::msg(format!("{name} is not a _dir.vpk file.")));
    };
    let suffix = name[base.len() + "_dir.vpk".len()..].to_string();
    let dir = crate::vpk::read_vpk_directory(path)?;
    if dir.by_path.contains_key("modinfo.json") {
        return Ok(false);
    }
    let parent = path.parent().ok_or_else(|| Error::msg("The VPK has no folder."))?.to_path_buf();
    let (sha, size) = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        let mut file = fs::File::open(path)?;
        let size = std::io::copy(&mut file, &mut hasher)?;
        (hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>(), size)
    };
    let work = parent.join(format!(".sigil-imprint-{}", std::process::id()));
    let staging = work.join("staging");
    let out = work.join("out");
    let result = (|| -> Result<crate::vpk_write::PackedVpk> {
        for entry in &dir.entries {
            let target = staging.join(&entry.path);
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            fs::write(&target, crate::vpk::read_vpk_entry_data(&dir, entry)?)?;
        }
        let (addoninfo, modinfo) = imprint_files(info, &sha, size);
        fs::create_dir_all(&staging)?;
        fs::write(staging.join("addoninfo.txt"), addoninfo)?;
        fs::write(staging.join("modinfo.json"), modinfo)?;
        crate::vpk_write::pack_vpk(&crate::vpk_write::PackOptions::new(staging.clone(), out.clone(), base.clone()))
    })();
    let outcome = result.and_then(|packed| {
        let old_chunks: Vec<PathBuf> = fs::read_dir(&parent)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                n.starts_with(&format!("{base}_")) && !n.contains("_dir.vpk") && n.ends_with(".vpk")
            })
            .collect();
        let mut kept = Vec::new();
        for chunk in &packed.chunk_paths {
            let target = parent.join(chunk.file_name().unwrap_or_default());
            let partial = parent.join(format!("{}.dah-tmp", target.file_name().unwrap_or_default().to_string_lossy()));
            fs::copy(chunk, &partial)?;
            fs::rename(&partial, &target)?;
            kept.push(target);
        }
        let partial = parent.join(format!("{name}.dah-tmp"));
        fs::copy(&packed.dir_path, &partial)?;
        fs::rename(&partial, path)?;
        for old in old_chunks {
            if !kept.contains(&old) {
                let _ = fs::remove_file(old);
            }
        }
        let _ = suffix;
        Ok(true)
    });
    let _ = fs::remove_dir_all(&work);
    outcome
}

fn tree_size(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path)
            .map(|entries| entries.flatten().map(|e| tree_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn remove_any(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(_) => {}
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)?.flatten() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if meta.is_file() {
        fs::copy(from, to)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("sigil-moddata-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn game(dir: &Path, files: &[&str]) -> PathBuf {
        let root = dir.to_path_buf();
        let addons = addons_dir(&root);
        fs::create_dir_all(&addons).unwrap();
        for name in files {
            fs::write(addons.join(name), name.as_bytes()).unwrap();
        }
        root
    }

    fn names(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(addons_dir(root))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn snapshot_then_replace_and_merge() {
        let tmp = Scratch::new("snap");
        let root = game(&tmp.path().join("g"), &["pak01_dir.vpk", "pak02_dir.vpk.disabled", "readme.txt"]);
        let snaps = tmp.path().join("snaps");
        let trash = tmp.path().join("trash");
        let snap = create_snapshot(&root, &snaps, None).unwrap();
        assert_eq!(snap.manifest.files, vec!["pak01_dir.vpk", "pak02_dir.vpk.disabled"]);
        assert_eq!(list_snapshots(&snaps).len(), 1);

        // Change the folder: drop 01, add 03.
        fs::remove_file(addons_dir(&root).join("pak01_dir.vpk")).unwrap();
        fs::write(addons_dir(&root).join("pak03_dir.vpk"), b"new").unwrap();
        fs::write(addons_dir(&root).join("pak02_dir.vpk.disabled"), b"changed").unwrap();

        let merged = restore_snapshot(&snap.dir, &root, &trash, RestoreMode::Merge, true).unwrap();
        assert_eq!(merged.copied, vec!["pak01_dir.vpk"]);
        assert_eq!(merged.skipped, vec!["pak02_dir.vpk.disabled"]);

        let replaced = restore_snapshot(&snap.dir, &root, &trash, RestoreMode::Replace, true).unwrap();
        assert_eq!(replaced.trashed.len(), 3);
        assert_eq!(names(&root), vec!["pak01_dir.vpk", "pak02_dir.vpk.disabled", "readme.txt"]);
        assert_eq!(fs::read(addons_dir(&root).join("pak02_dir.vpk.disabled")).unwrap(), b"pak02_dir.vpk.disabled");
        assert!(restore_snapshot(&snap.dir, &root, &trash, RestoreMode::Replace, false).is_err());
    }

    #[test]
    fn clearing_moves_everything_to_the_trash() {
        let tmp = Scratch::new("clear");
        let root = game(&tmp.path().join("g"), &["pak01_dir.vpk", "pak01_000.vpk", "pak05_dir.vpk", "notes.txt"]);
        let trash = tmp.path().join("trash");
        assert!(clear_all_mods(&root, &trash, false).is_err());
        let report = clear_all_mods(&root, &trash, true).unwrap();
        assert_eq!(report.moved.len(), 2);
        assert!(report.failed.is_empty());
        assert_eq!(names(&root), vec!["notes.txt"]);
        assert!(report.text().contains("pak05"));
        assert_eq!(unknown_files(&root), vec!["notes.txt"]);
    }

    #[test]
    fn records_are_kept_aside_and_wipes_free_space() {
        let tmp = Scratch::new("reset");
        let data = tmp.path().join("data");
        let ws = tmp.path().join("ws");
        fs::create_dir_all(&data).unwrap();
        fs::write(crate::import::library_path(&data), b"{}").unwrap();
        fs::create_dir_all(crate::install::downloads_dir(&ws).join("x")).unwrap();
        fs::write(crate::install::downloads_dir(&ws).join("x/a.zip"), vec![0u8; 100]).unwrap();
        let trash = data.join("trash");
        fs::create_dir_all(&trash).unwrap();
        let report = wipe_mod_data(&data, &ws, &trash).unwrap();
        assert_eq!(report.moved, vec![crate::import::LIBRARY_FILE_NAME]);
        assert!(report.kept_in.unwrap().join(crate::import::LIBRARY_FILE_NAME).is_file());
        assert!(!crate::install::downloads_dir(&ws).exists());
        assert!(report.freed >= 100);
        assert!(!crate::import::library_path(&data).exists());
        // Nothing left: a second reset is a no-op.
        assert_eq!(reset_mod_records(&data, &ws).unwrap(), ResetReport::default());
    }

    #[test]
    fn copies_mods_into_another_game_folder() {
        let tmp = Scratch::new("copy");
        let root = game(&tmp.path().join("g"), &["pak01_dir.vpk", "pak02_dir.vpk"]);
        let dev = game(&tmp.path().join("dev"), &["pak07_dir.vpk"]);
        let trash = tmp.path().join("trash");
        assert!(copy_mods_to(&root, &root, &trash, true).is_err());
        let report = copy_mods_to(&root, &dev, &trash, true).unwrap();
        assert_eq!(report.copied, 2);
        assert_eq!(report.trashed, 1);
        assert_eq!(names(&dev), vec!["pak01_dir.vpk", "pak02_dir.vpk"]);
    }

    #[test]
    fn variants_and_relocation() {
        let tmp = Scratch::new("variants");
        let root = game(&tmp.path().join("g"), &["pak01_dir.vpk", "pak02_dir.vpk", "pak03_dir.vpk.disabled"]);
        let meta = |slot, id| crate::import::ModMeta { slot, gamebanana_id: Some(id), ..Default::default() };
        let lib = crate::import::Library { schema: 1, mods: vec![meta(1, 7), meta(2, 8), meta(3, 7)] };
        assert_eq!(sibling_slots(&lib, 7, 3), vec![1]);
        let off = switch_variants(&root, &lib, 7, 3, true).unwrap();
        assert_eq!(off, vec![1]);
        assert_eq!(names(&root), vec!["pak01_dir.vpk.disabled", "pak02_dir.vpk", "pak03_dir.vpk"]);

        let cache = tmp.path().join("ws").join("downloads");
        fs::create_dir_all(cache.join("gb-1")).unwrap();
        fs::write(cache.join("gb-1").join("a.zip"), b"zip").unwrap();
        let to = tmp.path().join("big");
        assert!(check_move_target(&cache, &cache.join("inner")).is_err());
        relocate(&cache, &to).unwrap();
        assert!(is_link(&cache));
        assert_eq!(fs::read(cache.join("gb-1").join("a.zip")).unwrap(), b"zip", "the old path still reads");
        assert!(relocate(&cache, &to).is_err());
        relocate_back(&cache, &to).unwrap();
        assert!(!is_link(&cache));
        assert!(cache.join("gb-1").join("a.zip").is_file());
    }

    #[test]
    fn imprints_add_two_root_entries_and_keep_the_rest() {
        let tmp = Scratch::new("imprint");
        let staging = tmp.path().join("st");
        fs::create_dir_all(staging.join("materials")).unwrap();
        fs::write(staging.join("materials").join("a.vmat_c"), b"material bytes").unwrap();
        let packed = crate::vpk_write::pack_vpk(&crate::vpk_write::PackOptions::new(staging, tmp.path().join("o"), "pak01")).unwrap();
        assert!(packed.dir_path.ends_with("pak01_dir.vpk"));
        let info = Imprint { title: "Blue \"Haze\"".into(), author: "Kalo".into(), gamebanana_id: Some(7), source_url: "https://gamebanana.com/mods/7".into() };
        assert!(imprint_vpk(&packed.dir_path, &info, true).unwrap());
        let dir = crate::vpk::read_vpk_directory(&packed.dir_path).unwrap();
        let get = |p: &str| crate::vpk::read_vpk_entry_data(&dir, dir.by_path.get(p).unwrap()).unwrap();
        assert_eq!(get("materials/a.vmat_c"), b"material bytes");
        let addoninfo = String::from_utf8(get("addoninfo.txt")).unwrap();
        assert!(addoninfo.contains("addontitle \"Blue \\\"Haze\\\"\""));
        assert!(String::from_utf8(get("modinfo.json")).unwrap().contains("\"gamebananaId\": 7"));
        assert!(!imprint_vpk(&packed.dir_path, &info, true).unwrap(), "a second imprint is skipped");
    }
}
