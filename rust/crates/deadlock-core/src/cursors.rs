//! Cursor packs: replacement cursor images for Deadlock's cursor folder.
//!
//! Deadlock does not load its cursors from a VPK. SDL (through `inputsystem.dll`) reads loose
//! BMP files from `game/citadel/resource/cursors`, so a cursor mod is a set of files copied into
//! that folder: `cursor.bmp`, `cursor_ping.bmp`, `cursor_shop.bmp`, `cursor_commend.bmp`,
//! optional size variants such as `_vsz<N>`, and an optional `cursor.res` holding the hotspots.
//! Ported from Grimoire's cursor packs (MIT, `electron/main/services/cursorPacks.ts` and
//! `cursorFiles.ts`).
//!
//! Packs live in the app data folder the caller gives, under `cursor-packs/`: `state.json`, one
//! folder per pack under `packs/<id>/`, and a `stock/` copy of every stock cursor Sigil replaced.
//! Imports, [`list`] and [`delete`] touch that store only.
//!
//! [`apply_plan`] and [`restore_plan`] read the game cursor folder and describe the copy the app
//! must perform. Nothing here writes to a game install: the plans carry the source, the
//! destination, the order and the hash of every file.
//!
//! `state.json` keeps the sha256 of each file Sigil last wrote into the cursor folder. A cursor
//! file whose bytes hash differently was put back by Steam (a game update or a file verify), so
//! it is the current stock copy and is backed up again before the next write. Without that
//! record the next write would back the applied pack up as if it were stock.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::install::{ExtractLimits, ToolPaths, extract_archive};
use crate::mod_extras::new_uid;
use crate::util::{now_iso, write_atomic};

/// Folder under the caller's app data folder that holds every cursor pack.
pub const CURSOR_PACKS_DIR: &str = "cursor-packs";
pub const STATE_FILE_NAME: &str = "state.json";
/// A cursor image larger than this is not a cursor, it is a hostile or broken file.
pub const MAX_CURSOR_FILE_BYTES: u64 = 4 * 1024 * 1024;

const PACKS_DIR_NAME: &str = "packs";
const STOCK_DIR_NAME: &str = "stock";
const BMP_FILE_HEADER: usize = 14;
const BMP_INFO_HEADER: usize = 40;
const BMP_V5_HEADER: usize = 124;

/// One imported set of cursor files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorPack {
    pub id: String,
    pub name: String,
    /// Archive folder the set came from, when one archive shipped several sets. The
    /// display name already carries it, so the UI can use this to group variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Lowercased file names the pack installs, sorted. The game folder gets these names.
    pub files: Vec<String>,
    pub installed_at: String,
}

/// The stored packs plus the record of what Sigil last wrote into the game folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorPacksState {
    pub packs: Vec<CursorPack>,
    /// Pack currently applied to the game folder, when one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_id: Option<String>,
    /// sha256 of every file Sigil last wrote into the game cursor folder, keyed by
    /// lowercased file name. A file there with another hash was put back by Steam.
    pub applied: BTreeMap<String, String>,
    /// Where each pack came from, keyed by pack id. Only packs installed from GameBanana have
    /// an entry; reinstalling the same mod replaces its pack in place.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, PackSource>,
}

/// The GameBanana mod and file a cursor pack was installed from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PackSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gamebanana_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gamebanana_file_id: Option<u64>,
}

impl PackSource {
    /// Same GameBanana file (Grimoire's rule), or the same mod (a newer file of it: an update).
    pub fn same_origin(&self, other: &PackSource) -> bool {
        let same = |a: Option<u64>, b: Option<u64>| a.is_some() && a == b;
        same(self.gamebanana_file_id, other.gamebanana_file_id)
            || same(self.gamebanana_id, other.gamebanana_id)
    }
}

/// Why the active pack must be written again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reapply {
    pub pack_id: String,
    pub pack_name: String,
    /// Cursor files in the game folder that no longer hold what Sigil wrote: Steam put them
    /// back to stock (a game update or a file verify), or they are gone.
    pub restored_by_steam: Vec<String>,
    /// The pack was reinstalled from its GameBanana source since it was applied, so the store
    /// holds newer files than the game folder.
    pub pack_updated: bool,
}

/// One stock cursor file to copy into the store before a pack replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockBackup {
    /// Lowercased cursor file name.
    pub name: String,
    /// The game file whose current bytes are the stock copy.
    pub source: PathBuf,
    /// Where the stock copy goes under the pack store.
    pub destination: PathBuf,
    pub sha256: String,
}

/// A stock file to put back, or a file to remove when no stock copy exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRestore {
    pub name: String,
    /// `None` means the file did not exist before the first pack was applied, so the app
    /// removes `to` instead of copying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<PathBuf>,
    pub to: PathBuf,
}

/// One pack file to write into the game cursor folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileWrite {
    pub name: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub sha256: String,
}

/// What the app must do to apply a pack, in the order listed below.
///
/// 1. Copy every [`StockBackup`] into the store.
/// 2. Run every [`FileRestore`].
/// 3. Save [`ApplyPlan::state`].
/// 4. Write every [`FileWrite`].
///
/// The state must be saved before the pack files land: if the app crashes mid write, the
/// saved `applied` map lets the next run notice that the game folder no longer matches and
/// reapply the pack instead of backing it up as stock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPlan {
    pub pack_id: String,
    pub pack_name: String,
    pub cursor_folder: PathBuf,
    /// Stock files to back up first. Their `source` is the game file, so the app verifies
    /// the hash before copying to catch a game write between planning and applying.
    pub stock_backups: Vec<StockBackup>,
    /// Files from the previously applied pack to put back, or remove.
    pub restores: Vec<FileRestore>,
    pub writes: Vec<FileWrite>,
    /// The state to save, with this pack active and the new `applied` map.
    pub state: CursorPacksState,
    pub warnings: Vec<String>,
}

impl ApplyPlan {
    /// The `applied` map the store must hold once the writes land.
    pub fn applied(&self) -> &BTreeMap<String, String> {
        &self.state.applied
    }
}

/// What the app must do to put Deadlock's own cursors back.
///
/// The order is the same as [`ApplyPlan`]: stock backups, restores, then save `state`. The
/// saved state has no active pack and an empty `applied` map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePlan {
    pub cursor_folder: PathBuf,
    pub stock_backups: Vec<StockBackup>,
    pub restores: Vec<FileRestore>,
    pub state: CursorPacksState,
    pub warnings: Vec<String>,
}

/// Where Deadlock reads its cursors: `game/citadel/resource/cursors` under the game root.
pub fn cursor_folder(game_root: &Path) -> PathBuf {
    game_root
        .join("game")
        .join("citadel")
        .join("resource")
        .join("cursors")
}

/// Whether a file name is a cursor file Deadlock's input layer reads.
///
/// `cursor.res` holds the hotspots. Images are `cursor.bmp` plus underscore suffixes of
/// lowercase letters and digits, which covers `cursor_ping.bmp` and `cursor_vsz2.bmp`.
/// Matches Grimoire's `^cursor(?:_[a-z0-9]+)*\.bmp$` rule, case insensitively.
pub fn is_cursor_file_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();

    if name == "cursor.res" {
        return true;
    }

    let Some(stem) = name.strip_suffix(".bmp") else {
        return false;
    };
    let Some(suffix) = stem.strip_prefix("cursor") else {
        return false;
    };
    if suffix.is_empty() {
        return true;
    }

    let Some(suffix) = suffix.strip_prefix('_') else {
        return false;
    };

    !suffix.is_empty()
        && suffix.split('_').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

/// Re-header a 32-bit `BI_RGB` BMP so its reserved fourth byte is read as alpha.
///
/// Deadlock's stock cursors are 32-bit `BI_RGB` files whose fourth pixel byte is alpha. SDL
/// reads it, but Chromium treats it as padding and paints the transparent pixels (stored
/// white) opaque, so a preview would not match the game. The rewrite turns the header into
/// `BITMAPV5HEADER` with an explicit alpha mask and keeps the pixels untouched.
///
/// Anything else, including a 32-bit file whose alpha channel is all zero, comes back
/// unchanged, so a preview never fails on a file it cannot improve.
pub fn bmp_with_explicit_alpha(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < BMP_FILE_HEADER + BMP_INFO_HEADER {
        return Ok(bytes.to_vec());
    }
    if u32_at(bytes, 14) != BMP_INFO_HEADER as u32
        || u16_at(bytes, 28) != 32
        || u32_at(bytes, 30) != 0
    {
        return Ok(bytes.to_vec());
    }

    let offset = u32_at(bytes, 10) as usize;
    if offset >= bytes.len() {
        return Ok(bytes.to_vec());
    }
    let pixels = &bytes[offset..];

    // SDL loads an all-zero alpha channel as opaque, which is how the file already previews.
    if !pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0) {
        return Ok(bytes.to_vec());
    }

    let start = BMP_FILE_HEADER + BMP_V5_HEADER;
    let mut output = vec![0u8; start + pixels.len()];
    output[..BMP_FILE_HEADER + BMP_INFO_HEADER]
        .copy_from_slice(&bytes[..BMP_FILE_HEADER + BMP_INFO_HEADER]);

    let length = output.len() as u32;
    output[2..6].copy_from_slice(&length.to_le_bytes());
    output[10..14].copy_from_slice(&(start as u32).to_le_bytes());
    output[14..18].copy_from_slice(&(BMP_V5_HEADER as u32).to_le_bytes());
    output[30..34].copy_from_slice(&3_u32.to_le_bytes()); // BI_BITFIELDS
    output[54..58].copy_from_slice(&0x00ff_0000_u32.to_le_bytes());
    output[58..62].copy_from_slice(&0x0000_ff00_u32.to_le_bytes());
    output[62..66].copy_from_slice(&0x0000_00ff_u32.to_le_bytes());
    output[66..70].copy_from_slice(&0xff00_0000_u32.to_le_bytes());
    output[70..74].copy_from_slice(b"BGRs"); // LCS_sRGB, little endian
    output[start..].copy_from_slice(pixels);

    Ok(output)
}

/// Import loose cursor files from disk as one pack. Names that are not cursor files are
/// skipped, and unusable images are dropped. At least one BMP must remain.
pub fn import_files(store: &Path, name: &str, files: &[PathBuf]) -> Result<CursorPack> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::msg("A cursor pack needs a name."));
    }

    let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for path in files {
        let file_name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !is_cursor_file_name(&file_name) {
            continue;
        }
        let bytes = fs::read(path).map_err(|error| {
            Error::msg(format!(
                "Cursor file {} could not be read: {error}",
                path.display()
            ))
        })?;
        if is_usable_cursor_file(&file_name, &bytes) {
            contents.insert(file_name, bytes);
        }
    }

    if !contents.keys().any(|name| name.ends_with(".bmp")) {
        return Err(Error::msg(
            "No cursor images were found. Cursor mods are BMP files named like cursor.bmp.",
        ));
    }

    let mut state = load_state(store)?;
    let pack = install_pack(store, &mut state, name, None, contents)?;

    if let Err(error) = save_state(store, &state) {
        let _ = fs::remove_dir_all(pack_dir(store, &pack.id));
        return Err(error);
    }

    Ok(pack)
}

/// Import every cursor set inside an archive. Reuses the install pipeline's extractor, so
/// `.zip`, `.7z` and `.rar` all open. One archive holding sibling folders that each carry
/// their own cursor image becomes one pack per folder, named after the folder.
///
/// Resolves to an empty list when the archive holds no cursor images. The caller decides
/// whether that is an error.
pub fn import_archive(store: &Path, archive: &Path) -> Result<Vec<CursorPack>> {
    import_archive_from(store, archive, None, &PackSource::default())
}

/// [`import_archive`] for an archive downloaded from GameBanana. `origin` is recorded on every
/// pack it yields, and a pack already installed from the same file or the same mod with the
/// same variant is replaced in place: it keeps its id (so an active pack stays active) and gets
/// the new files. Call [`needs_reapply`] afterwards; an active pack that was replaced reports
/// `pack_updated`. `name` overrides the archive's file name as the pack name (the mod's title).
pub fn import_archive_from(
    store: &Path,
    archive: &Path,
    name: Option<&str>,
    origin: &PackSource,
) -> Result<Vec<CursorPack>> {
    let source = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| archive_stem_name(archive));
    import_extracted_archive(store, archive, &source, origin)
}

fn archive_stem_name(archive: &Path) -> String {
    archive
        .file_stem()
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "cursor pack".to_string())
}

fn import_extracted_archive(
    store: &Path,
    archive: &Path,
    source: &str,
    origin: &PackSource,
) -> Result<Vec<CursorPack>> {
    let extract_root = packs_root(store).join(format!(".extract-{}", new_uid()));
    let report = match extract_archive(
        archive,
        &extract_root,
        &ExtractLimits::default(),
        &ToolPaths::from_environment(),
    ) {
        Ok(report) => report,
        Err(error) => {
            let _ = fs::remove_dir_all(&extract_root);
            return Err(error);
        }
    };

    let result = install_extracted(store, source, &report.directory, &report.files, origin);
    let _ = fs::remove_dir_all(&extract_root);
    result
}

/// Every stored pack and the applied record. A missing state file is an empty store. A state
/// file that cannot be read or parsed is an error: treating it as empty would forget which
/// files Sigil wrote and the next apply would back up the applied pack as stock.
pub fn list(store: &Path) -> Result<CursorPacksState> {
    load_state(store)
}

/// Folder holding one pack's files, the place a preview reads from. Join the pack's `files`
/// names to it and pass a BMP through [`bmp_with_explicit_alpha`] before showing it.
pub fn pack_folder(store: &Path, id: &str) -> PathBuf {
    pack_dir(store, id)
}

/// The folder holding Deadlock's own cursors for a "stock" preview, as Grimoire picks it: the
/// game's cursor folder while no pack is applied, else Sigil's stock copy in the store. List
/// cursor names there with [`is_cursor_file_name`] and pass BMPs through
/// [`bmp_with_explicit_alpha`].
pub fn stock_preview_folder(store: &Path, game_root: &Path) -> Result<PathBuf> {
    let state = load_state(store)?;
    Ok(if state.active_id.is_none() {
        cursor_folder(game_root)
    } else {
        stock_dir(store)
    })
}

/// Write the state file. The app calls this with the `state` a plan returned, after running
/// that plan's stock backups and restores.
pub fn save_state(store: &Path, state: &CursorPacksState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state)?;
    write_atomic(&state_path(store), &bytes)
}

/// Remove a pack from the store. The pack must not be the applied one: its files are still in
/// the game folder, so put the stock cursors back first (run [`restore_plan`]) and then
/// delete it. Nothing outside the store is touched.
pub fn delete(store: &Path, id: &str) -> Result<()> {
    let mut state = load_state(store)?;
    let Some(position) = state.packs.iter().position(|pack| pack.id == id) else {
        return Err(Error::msg("That cursor pack is no longer installed."));
    };
    if state.active_id.as_deref() == Some(id) {
        return Err(Error::msg(
            "The active cursor pack is still applied to the game. Restore the stock cursors first.",
        ));
    }
    if !is_safe_id(id) {
        return Err(Error::msg(
            "That cursor pack has an id the store cannot own.",
        ));
    }

    state.packs.remove(position);
    state.sources.remove(id);
    save_state(store, &state)?;

    let directory = pack_dir(store, id);
    if directory.exists() {
        fs::remove_dir_all(&directory).map_err(|error| {
            Error::msg(format!("Could not delete {}: {error}", directory.display()))
        })?;
    }

    Ok(())
}

/// Plan applying a pack to the game cursor folder. Reads the pack, the store and the game
/// folder; writes nothing.
///
/// Stock files are those whose bytes do not match the `applied` record, including a stock
/// file Steam put back after an update or a verify. Restores put the previous pack's files
/// back before the new ones land. The returned `state` has this pack active and the new
/// `applied` map; save it before the writes.
pub fn apply_plan(store: &Path, id: &str, game_root: &Path) -> Result<ApplyPlan> {
    let state = load_state(store)?;
    let pack = state
        .packs
        .iter()
        .find(|pack| pack.id == id)
        .cloned()
        .ok_or_else(|| Error::msg("That cursor pack is no longer installed."))?;
    if !is_safe_id(&pack.id) {
        return Err(Error::msg(
            "That cursor pack has an id the store cannot own.",
        ));
    }

    let directory = pack_dir(store, &pack.id);
    let mut writes = Vec::with_capacity(pack.files.len());
    for name in &pack.files {
        if !is_cursor_file_name(name) {
            return Err(Error::msg(format!(
                "Cursor pack {} lists {name}, which is not a cursor file name. Delete the pack and import it again.",
                pack.name
            )));
        }
        let path = directory.join(name);
        let bytes = fs::read(&path).map_err(|error| {
            Error::msg(format!(
                "Cursor pack file {} could not be read: {error}. Delete the pack and import it again.",
                path.display()
            ))
        })?;
        writes.push(FileWrite {
            name: name.clone(),
            from: path,
            to: PathBuf::new(),
            sha256: sha256_hex(&bytes),
        });
    }

    let folder = cursor_folder(game_root);
    if !folder.is_dir() {
        return Err(Error::msg(format!(
            "Deadlock's cursor folder was not found at {}. Check the game path in Settings.",
            folder.display()
        )));
    }

    let (current, warnings) = scan_cursor_folder(&folder)?;
    let stock_backups = plan_stock_backups(store, &state, &current);
    let incoming: BTreeSet<String> = writes.iter().map(|write| write.name.clone()).collect();
    let restores = restore_actions(
        &state,
        &folder,
        &stock_dir(store),
        &incoming,
        &stock_backups,
    );
    for write in &mut writes {
        write.to = folder.join(&write.name);
    }

    let mut next = state;
    next.active_id = Some(pack.id.clone());
    next.applied = writes
        .iter()
        .map(|write| (write.name.clone(), write.sha256.clone()))
        .collect();

    Ok(ApplyPlan {
        pack_id: pack.id,
        pack_name: pack.name,
        cursor_folder: folder,
        stock_backups,
        restores,
        writes,
        state: next,
        warnings,
    })
}

/// Plan putting Deadlock's own cursors back. Reads the store and the game folder; writes
/// nothing.
///
/// A stock file Steam put back is backed up first, so the restore uses the current stock
/// bytes rather than the copy taken before the first apply. A missing cursor folder (Deadlock
/// moved or uninstalled) still clears the record, with a warning, so a stale active pack does
/// not block the UI.
pub fn restore_plan(store: &Path, game_root: &Path) -> Result<RestorePlan> {
    let state = load_state(store)?;
    let folder = cursor_folder(game_root);

    let mut warnings = Vec::new();
    let mut stock_backups = Vec::new();
    let mut restores = Vec::new();

    if folder.is_dir() {
        let (current, case_warnings) = scan_cursor_folder(&folder)?;
        warnings.extend(case_warnings);
        stock_backups = plan_stock_backups(store, &state, &current);
        let incoming = BTreeSet::new();
        restores = restore_actions(
            &state,
            &folder,
            &stock_dir(store),
            &incoming,
            &stock_backups,
        );
    } else {
        warnings.push(format!(
            "Deadlock's cursor folder was not found at {}. The pack record was cleared but nothing was restored on disk.",
            folder.display()
        ));
    }

    let mut next = state;
    next.active_id = None;
    next.applied.clear();

    Ok(RestorePlan {
        cursor_folder: folder,
        stock_backups,
        restores,
        state: next,
        warnings,
    })
}

fn packs_root(store: &Path) -> PathBuf {
    store.join(CURSOR_PACKS_DIR)
}

fn state_path(store: &Path) -> PathBuf {
    packs_root(store).join(STATE_FILE_NAME)
}

fn stock_dir(store: &Path) -> PathBuf {
    packs_root(store).join(STOCK_DIR_NAME)
}

fn pack_dir(store: &Path, id: &str) -> PathBuf {
    packs_root(store).join(PACKS_DIR_NAME).join(id)
}

/// Pack ids never contain a path separator; a hand edited id could otherwise escape the store.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

fn is_usable_cursor_file(name: &str, bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() as u64 > MAX_CURSOR_FILE_BYTES {
        return false;
    }
    if name.ends_with(".res") {
        return true;
    }
    bytes.len() >= 2 && bytes[0] == b'B' && bytes[1] == b'M'
}

/// Whether the active pack must be written again, Grimoire's startup reconcile as a check: a
/// cursor file Sigil wrote was put back to stock by Steam (or removed), or the pack was
/// reinstalled from its source since it was applied. None when no pack is active, nothing
/// changed, or the cursor folder is missing. Reads only; the fix is the usual
/// [`apply_plan`] for `pack_id`.
pub fn needs_reapply(store: &Path, game_root: &Path) -> Result<Option<Reapply>> {
    let state = load_state(store)?;
    let Some(active) = state.active_id.clone() else {
        return Ok(None);
    };
    let Some(pack) = state.packs.iter().find(|pack| pack.id == active) else {
        return Ok(None);
    };
    let folder = cursor_folder(game_root);
    if !folder.is_dir() {
        return Ok(None);
    }
    let restored_by_steam: Vec<String> = state
        .applied
        .iter()
        .filter(|(name, hash)| {
            fs::read(folder.join(name.as_str()))
                .map(|bytes| sha256_hex(&bytes) != **hash)
                .unwrap_or(true)
        })
        .map(|(name, _)| name.clone())
        .collect();
    let directory = pack_dir(store, &pack.id);
    let stored: BTreeMap<String, String> = pack
        .files
        .iter()
        .filter_map(|name| fs::read(directory.join(name)).ok().map(|bytes| (name.clone(), sha256_hex(&bytes))))
        .collect();
    let pack_updated = stored != state.applied;
    if restored_by_steam.is_empty() && !pack_updated {
        return Ok(None);
    }
    Ok(Some(Reapply {
        pack_id: pack.id.clone(),
        pack_name: pack.name.clone(),
        restored_by_steam,
        pack_updated,
    }))
}

fn load_state(store: &Path) -> Result<CursorPacksState> {
    let path = state_path(store);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| {
            Error::msg(format!(
                "Could not parse {}: {error}. Fix or remove the file; cursor packs cannot be read while it is broken.",
                path.display()
            ))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(CursorPacksState::default())
        }
        Err(error) => Err(Error::msg(format!(
            "Could not read {}: {error}",
            path.display()
        ))),
    }
}

fn install_extracted(
    store: &Path,
    source: &str,
    directory: &Path,
    files: &[String],
    origin: &PackSource,
) -> Result<Vec<CursorPack>> {
    let mut groups: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    for relative in files {
        let (folder, base) = relative
            .rsplit_once('/')
            .map_or(("", relative.as_str()), |(folder, base)| (folder, base));
        let name = base.to_ascii_lowercase();
        if !is_cursor_file_name(&name) {
            continue;
        }
        let path = relative
            .split('/')
            .fold(directory.to_path_buf(), |path, segment| path.join(segment));
        groups
            .entry(folder.to_string())
            .or_default()
            .insert(name, path);
    }

    let sets: Vec<(String, BTreeMap<String, PathBuf>)> = groups
        .into_iter()
        .filter(|(_, files)| files.keys().any(|name| name.ends_with(".bmp")))
        .collect();
    let variant_sets = sets.len() > 1;

    let mut state = load_state(store)?;
    let mut installed: Vec<CursorPack> = Vec::new();

    for (folder, paths) in sets {
        let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (name, path) in paths {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    remove_pack_dirs(store, &installed);
                    return Err(Error::msg(format!(
                        "Cursor file {} could not be read: {error}",
                        path.display()
                    )));
                }
            };
            if is_usable_cursor_file(&name, &bytes) {
                contents.insert(name, bytes);
            }
        }
        if !contents.keys().any(|name| name.ends_with(".bmp")) {
            continue;
        }

        let variant = (variant_sets && !folder.is_empty()).then(|| folder.clone());
        let display = variant
            .as_deref()
            .map(variant_display)
            .filter(|value| !value.is_empty());
        let name = match display {
            Some(value) => format!("{source} ({value})"),
            None => source.to_string(),
        };

        // Reinstalling from the same GameBanana source replaces that pack in place.
        let replaces = (*origin != PackSource::default())
            .then(|| {
                state.packs.iter().find(|pack| {
                    pack.variant == variant
                        && state.sources.get(&pack.id).is_some_and(|known| known.same_origin(origin))
                        && !installed.iter().any(|done| done.id == pack.id)
                })
            })
            .flatten()
            .map(|pack| pack.id.clone());
        let result = install_pack_as(store, &mut state, replaces, &name, variant, contents);
        match result {
            Ok(pack) => {
                if *origin != PackSource::default() {
                    state.sources.insert(pack.id.clone(), origin.clone());
                }
                installed.push(pack)
            }
            Err(error) => {
                remove_pack_dirs(store, &installed);
                return Err(error);
            }
        }
    }

    if !installed.is_empty()
        && let Err(error) = save_state(store, &state)
    {
        remove_pack_dirs(store, &installed);
        return Err(error);
    }

    Ok(installed)
}

fn install_pack(
    store: &Path,
    state: &mut CursorPacksState,
    name: &str,
    variant: Option<String>,
    contents: BTreeMap<String, Vec<u8>>,
) -> Result<CursorPack> {
    install_pack_as(store, state, None, name, variant, contents)
}

/// [`install_pack`], reusing `existing` as the id when it replaces a pack in place. The new
/// files go to a fresh folder first and replace the old folder only once all are written.
fn install_pack_as(
    store: &Path,
    state: &mut CursorPacksState,
    existing: Option<String>,
    name: &str,
    variant: Option<String>,
    contents: BTreeMap<String, Vec<u8>>,
) -> Result<CursorPack> {
    let id = existing.clone().filter(|id| is_safe_id(id)).unwrap_or_else(new_uid);
    let final_dir = pack_dir(store, &id);
    let directory = if existing.is_some() {
        packs_root(store).join(format!(".replace-{}", new_uid()))
    } else {
        final_dir.clone()
    };
    let pack = CursorPack {
        id: id.clone(),
        name: name.to_string(),
        variant,
        files: contents.keys().cloned().collect(),
        installed_at: now_iso(),
    };

    fs::create_dir_all(&directory).map_err(|error| {
        Error::msg(format!("Could not create {}: {error}", directory.display()))
    })?;
    for (file, bytes) in &contents {
        let path = directory.join(file);
        if let Err(error) = fs::write(&path, bytes) {
            let _ = fs::remove_dir_all(&directory);
            return Err(Error::msg(format!(
                "Cursor file {} could not be written: {error}",
                path.display()
            )));
        }
    }
    if directory != final_dir {
        let _ = fs::remove_dir_all(&final_dir);
        if let Err(error) = fs::rename(&directory, &final_dir) {
            let _ = fs::remove_dir_all(&directory);
            return Err(Error::msg(format!(
                "Could not replace {}: {error}",
                final_dir.display()
            )));
        }
    }

    state.packs.retain(|existing| existing.id != pack.id);
    state.packs.push(pack.clone());
    Ok(pack)
}

fn remove_pack_dirs(store: &Path, packs: &[CursorPack]) {
    for pack in packs {
        let _ = fs::remove_dir_all(pack_dir(store, &pack.id));
    }
}

/// The folder name as the UI shows it: underscores and hyphens become one space, the same
/// substitution Grimoire makes for a variant name.
fn variant_display(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

struct CurrentFile {
    path: PathBuf,
    sha256: String,
}

/// Every cursor file currently in the game folder, keyed by lowercased name.
fn scan_cursor_folder(folder: &Path) -> Result<(BTreeMap<String, CurrentFile>, Vec<String>)> {
    let entries = fs::read_dir(folder)
        .map_err(|error| Error::msg(format!("Could not read {}: {error}", folder.display())))?;

    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !is_cursor_file_name(&name) {
            continue;
        }
        names.push((name, path));
    }
    names.sort();

    let mut found = BTreeMap::new();
    let mut warnings = Vec::new();
    for (name, path) in names {
        let bytes = fs::read(&path).map_err(|error| {
            Error::msg(format!(
                "Cursor file {} could not be read: {error}",
                path.display()
            ))
        })?;
        if let Some(other) = found.get(&name) {
            let other: &CurrentFile = other;
            warnings.push(format!(
                "The cursor folder holds {} and {}, which differ only by case. The plan uses {}.",
                other.path.display(),
                path.display(),
                other.path.display()
            ));
            continue;
        }
        found.insert(
            name,
            CurrentFile {
                path,
                sha256: sha256_hex(&bytes),
            },
        );
    }

    Ok((found, warnings))
}

fn plan_stock_backups(
    store: &Path,
    state: &CursorPacksState,
    current: &BTreeMap<String, CurrentFile>,
) -> Vec<StockBackup> {
    let mut backups = Vec::new();
    for (name, file) in current {
        if state.applied.get(name) == Some(&file.sha256) {
            continue;
        }
        backups.push(StockBackup {
            name: name.clone(),
            source: file.path.clone(),
            destination: stock_dir(store).join(name),
            sha256: file.sha256.clone(),
        });
    }
    backups
}

fn restore_actions(
    state: &CursorPacksState,
    folder: &Path,
    stock: &Path,
    incoming: &BTreeSet<String>,
    stock_backups: &[StockBackup],
) -> Vec<FileRestore> {
    let mut actions = Vec::new();
    for name in state.applied.keys() {
        // A name the store could not have written is ignored; the restored state clears the
        // whole map, so a hand edited entry cannot steer a copy outside the cursor folder.
        if !is_cursor_file_name(name) || incoming.contains(name) {
            continue;
        }
        let stock_file = stock.join(name);
        let available =
            stock_file.is_file() || stock_backups.iter().any(|backup| &backup.name == name);
        actions.push(FileRestore {
            name: name.clone(),
            from: available.then_some(stock_file),
            to: folder.join(name),
        });
    }
    actions
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounds checked by the caller"),
    )
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounds checked by the caller"),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Apply a pack to the game's cursor folder: the app action behind the player's Apply.
///
/// Refuses when `approve` is false. Plans, then checks that the game files to back up still
/// match the plan and that every pack file is intact, and only then writes: stock backups into
/// the store, restores of the previous pack, the new state, then the pack files (each through a
/// temporary file and a rename). The state is saved before the pack files land so a crash in
/// between is noticed and repaired by applying again.
pub fn apply(store: &Path, id: &str, game_root: &Path, approve: bool) -> Result<ApplyPlan> {
    if !approve {
        return Err(Error::msg("Applying the cursor pack was not approved. Nothing was written."));
    }
    let plan = apply_plan(store, id, game_root)?;
    check_backups(&plan.stock_backups)?;
    for write in &plan.writes {
        let bytes = fs::read(&write.from)?;
        if sha256_hex(&bytes) != write.sha256 {
            return Err(Error::msg(format!(
                "The cursor pack file {} changed while Sigil was applying it. Try again. Nothing was written.",
                write.name
            )));
        }
    }
    run_backups_and_restores(&plan.stock_backups, &plan.restores)?;
    save_state(store, &plan.state)?;
    for write in &plan.writes {
        write_atomic(&write.to, &fs::read(&write.from)?)?;
    }
    Ok(plan)
}

/// Put Deadlock's own cursors back: the app action behind Restore. Refuses when `approve` is
/// false; otherwise the same order as [`apply`] without the pack files.
pub fn restore(store: &Path, game_root: &Path, approve: bool) -> Result<RestorePlan> {
    if !approve {
        return Err(Error::msg("Restoring Deadlock's cursors was not approved. Nothing was written."));
    }
    let plan = restore_plan(store, game_root)?;
    check_backups(&plan.stock_backups)?;
    run_backups_and_restores(&plan.stock_backups, &plan.restores)?;
    save_state(store, &plan.state)?;
    Ok(plan)
}

/// A game file that changed after planning means the plan is stale.
fn check_backups(backups: &[StockBackup]) -> Result<()> {
    for backup in backups {
        let bytes = fs::read(&backup.source)?;
        if sha256_hex(&bytes) != backup.sha256 {
            return Err(Error::msg(format!(
                "{} changed while Sigil was planning. Try again. Nothing was written.",
                backup.source.display()
            )));
        }
    }
    Ok(())
}

fn run_backups_and_restores(backups: &[StockBackup], restores: &[FileRestore]) -> Result<()> {
    let mut sorted: Vec<&StockBackup> = backups.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for backup in sorted {
        write_atomic(&backup.destination, &fs::read(&backup.source)?)?;
    }
    for restore in restores {
        match &restore.from {
            Some(from) => write_atomic(&restore.to, &fs::read(from)?)?,
            None => match fs::remove_file(&restore.to) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dah-cursors-{label}-{}-{stamp}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// A BMP with a 40 byte info header, so the alpha fix has the layout it expects.
    fn bmp(width: u32, height: u32, bits: u16, pixels: &[u8]) -> Vec<u8> {
        let offset = 54usize;
        let mut bytes = vec![0u8; offset];
        bytes[0] = b'B';
        bytes[1] = b'M';
        bytes[2..6].copy_from_slice(&(offset as u32 + pixels.len() as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&width.to_le_bytes());
        bytes[22..26].copy_from_slice(&height.to_le_bytes());
        bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(pixels);
        bytes
    }

    fn bmp_32(alpha: u8) -> Vec<u8> {
        bmp(1, 1, 32, &[0x10, 0x20, 0x30, alpha])
    }

    fn fake_game(root: &Path) -> PathBuf {
        let game_root = root.join("Deadlock");
        fs::create_dir_all(cursor_folder(&game_root)).unwrap();
        game_root
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn run_actions(
        store: &Path,
        stock_backups: &[StockBackup],
        restores: &[FileRestore],
        state: &CursorPacksState,
    ) {
        for backup in stock_backups {
            if let Some(parent) = backup.destination.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(&backup.source, &backup.destination).unwrap();
        }
        for restore in restores {
            match &restore.from {
                Some(from) => {
                    fs::copy(from, &restore.to).unwrap();
                }
                None => {
                    let _ = fs::remove_file(&restore.to);
                }
            }
        }
        save_state(store, state).unwrap();
    }

    #[test]
    fn cursor_file_names_follow_the_game_rules() {
        for name in [
            "cursor.bmp",
            "CURSOR.BMP",
            "cursor.res",
            "CURSOR.RES",
            "cursor_ping.bmp",
            "cursor_shop.bmp",
            "cursor_commend.bmp",
            "cursor_vsz2.bmp",
            "cursor_shop_2.bmp",
        ] {
            assert!(is_cursor_file_name(name), "{name} should be a cursor file");
        }

        for name in [
            "mycursor.bmp",
            "cursors.bmp",
            "cursor.png",
            "cursor.bmp.bak",
            "cursor_.bmp",
            "cursor__a.bmp",
            "cursor-a.bmp",
            "cursor_shop.BMP.tmp",
            "cursor/shop.bmp",
            "",
        ] {
            assert!(
                !is_cursor_file_name(name),
                "{name} should not be a cursor file"
            );
        }
    }

    #[test]
    fn alpha_fix_rewrites_a_32_bit_bmp_as_v5() {
        let source = bmp(2, 1, 32, &[0x10, 0x20, 0x30, 0x80, 0x40, 0x50, 0x60, 0x00]);
        let fixed = bmp_with_explicit_alpha(&source).unwrap();

        assert_eq!(fixed.len(), 138 + 8);
        assert_eq!(u32_at(&fixed, 2) as usize, fixed.len());
        assert_eq!(u32_at(&fixed, 10) as usize, 138);
        assert_eq!(u32_at(&fixed, 14) as usize, BMP_V5_HEADER);
        assert_eq!(u32_at(&fixed, 18), 2);
        assert_eq!(u32_at(&fixed, 22), 1);
        assert_eq!(u16_at(&fixed, 28), 32);
        assert_eq!(u32_at(&fixed, 30), 3);
        assert_eq!(u32_at(&fixed, 54), 0x00ff_0000);
        assert_eq!(u32_at(&fixed, 58), 0x0000_ff00);
        assert_eq!(u32_at(&fixed, 62), 0x0000_00ff);
        assert_eq!(u32_at(&fixed, 66), 0xff00_0000);
        assert_eq!(&fixed[70..74], b"BGRs");
        assert_eq!(&fixed[138..], &source[54..]);
    }

    #[test]
    fn alpha_fix_leaves_other_bmps_alone() {
        let opaque = bmp_32(0);
        assert_eq!(bmp_with_explicit_alpha(&opaque).unwrap(), opaque);

        let source = bmp(1, 1, 24, &[0x10, 0x20, 0x30, 0x00]);
        assert_eq!(bmp_with_explicit_alpha(&source).unwrap(), source);

        let mut compressed = bmp_32(0x80);
        compressed[30..34].copy_from_slice(&3_u32.to_le_bytes());
        assert_eq!(bmp_with_explicit_alpha(&compressed).unwrap(), compressed);

        let short = b"\x42\x4d\x00";
        assert_eq!(bmp_with_explicit_alpha(short).unwrap(), short);

        let mut broken_offset = bmp_32(0x80);
        broken_offset[10..14].copy_from_slice(&9999_u32.to_le_bytes());
        assert_eq!(
            bmp_with_explicit_alpha(&broken_offset).unwrap(),
            broken_offset
        );
    }

    #[test]
    fn imports_loose_files_and_skips_names_the_game_ignores() {
        let temp = TempDir::new("files");
        let store = temp.join("data");
        fs::create_dir_all(temp.join("source")).unwrap();
        let main = temp.join("source").join("cursor.bmp");
        let ping = temp.join("source").join("CURSOR_PING.BMP");
        let notes = temp.join("source").join("notes.txt");
        fs::write(&main, bmp_32(0x80)).unwrap();
        fs::write(&ping, bmp_32(0x40)).unwrap();
        fs::write(&notes, b"not a cursor").unwrap();

        let pack = import_files(&store, "Test pack", &[main, ping, notes]).unwrap();

        assert_eq!(pack.name, "Test pack");
        assert_eq!(pack.variant, None);
        assert_eq!(pack.files, vec!["cursor.bmp", "cursor_ping.bmp"]);
        assert!(!pack.installed_at.is_empty());
        assert_eq!(
            fs::read(pack_folder(&store, &pack.id).join("cursor_ping.bmp")).unwrap(),
            bmp_32(0x40)
        );

        let state = list(&store).unwrap();
        assert_eq!(state.packs.len(), 1);
        assert_eq!(state.active_id, None);
        assert!(state.applied.is_empty());
    }

    #[test]
    fn import_files_needs_an_image_and_a_name() {
        let temp = TempDir::new("files-empty");
        let store = temp.join("data");
        let notes = temp.join("notes.txt");
        fs::write(&notes, b"not a cursor").unwrap();

        let error = import_files(&store, "Pack", std::slice::from_ref(&notes)).unwrap_err();
        assert!(error.to_string().contains("No cursor images were found"));

        let error = import_files(&store, "  ", &[notes]).unwrap_err();
        assert!(error.to_string().contains("needs a name"));
    }

    #[test]
    fn imports_one_pack_from_a_flat_zip() {
        let temp = TempDir::new("zip-flat");
        let store = temp.join("data");
        let archive = temp.join("neon-cursors.zip");
        write_zip(
            &archive,
            &[
                ("cursor.bmp", &bmp_32(0x80)),
                ("nested/notes.txt", b"ignored"),
            ],
        );

        let packs = import_archive(&store, &archive).unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].name, "neon-cursors");
        assert_eq!(packs[0].variant, None);
        assert_eq!(packs[0].files, vec!["cursor.bmp"]);
    }

    #[test]
    fn imports_one_pack_per_archive_folder() {
        let temp = TempDir::new("zip-variants");
        let store = temp.join("data");
        let archive = temp.join("pixel-pack.zip");
        write_zip(
            &archive,
            &[
                ("Large/cursor.bmp", &bmp_32(0x80)),
                ("Small/cursor_ping.bmp", &bmp_32(0x40)),
                ("Small/cursor.res", b"hotspots"),
            ],
        );

        let packs = import_archive(&store, &archive).unwrap();
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0].name, "pixel-pack (Large)");
        assert_eq!(packs[0].variant.as_deref(), Some("Large"));
        assert_eq!(packs[0].files, vec!["cursor.bmp"]);
        assert_eq!(packs[1].name, "pixel-pack (Small)");
        assert_eq!(packs[1].variant.as_deref(), Some("Small"));
        assert_eq!(packs[1].files, vec!["cursor.res", "cursor_ping.bmp"]);

        let state = list(&store).unwrap();
        assert_eq!(state.packs.len(), 2);
        // The temporary extract folder is cleaned up.
        let extracted: Vec<PathBuf> = fs::read_dir(packs_root(&store))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".extract-"))
            .map(|entry| entry.path())
            .collect();
        assert!(extracted.is_empty(), "{extracted:?}");
    }

    #[test]
    fn an_archive_without_cursor_images_installs_nothing() {
        let temp = TempDir::new("zip-empty");
        let store = temp.join("data");
        let archive = temp.join("sounds.zip");
        write_zip(&archive, &[("sound.wav", b"RIFF")]);

        assert!(import_archive(&store, &archive).unwrap().is_empty());
        assert!(list(&store).unwrap().packs.is_empty());
    }

    #[test]
    fn variant_folder_names_lose_separators_in_the_display_name() {
        assert_eq!(variant_display("Large_Pack"), "Large Pack");
        assert_eq!(variant_display("small--set"), "small set");
        assert_eq!(variant_display("_"), "");
    }

    #[test]
    fn apply_plan_backs_up_stock_and_writes_the_pack() {
        let temp = TempDir::new("apply");
        let store = temp.join("data");
        let game = fake_game(temp.path());
        let folder = cursor_folder(&game);
        let stock_main = bmp_32(0x00);
        let stock_ping = bmp(1, 1, 24, &[1, 2, 3, 0]);
        fs::write(folder.join("cursor.bmp"), &stock_main).unwrap();
        fs::write(folder.join("cursor_ping.bmp"), &stock_ping).unwrap();

        let source = temp.join("cursor.bmp");
        let pack_bytes = bmp_32(0x80);
        fs::write(&source, &pack_bytes).unwrap();
        let pack = import_files(&store, "Neon", &[source]).unwrap();

        let plan = apply_plan(&store, &pack.id, &game).unwrap();
        let pack_sha = sha256_hex(&pack_bytes);

        assert_eq!(plan.pack_id, pack.id);
        assert_eq!(plan.cursor_folder, folder);
        assert_eq!(
            plan.stock_backups
                .iter()
                .map(|backup| backup.name.as_str())
                .collect::<Vec<_>>(),
            vec!["cursor.bmp", "cursor_ping.bmp"]
        );
        assert_eq!(plan.stock_backups[0].sha256, sha256_hex(&stock_main));
        assert!(plan.restores.is_empty());
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.writes[0].to, folder.join("cursor.bmp"));
        assert_eq!(plan.writes[0].sha256, sha256_hex(&pack_bytes));
        assert_eq!(plan.applied().len(), 1);
        assert_eq!(plan.state.active_id.as_deref(), Some(pack.id.as_str()));
        assert_eq!(
            plan.applied().get("cursor.bmp").map(String::as_str),
            Some(pack_sha.as_str())
        );
    }

    #[test]
    fn a_second_apply_notices_a_stock_file_steam_put_back() {
        let temp = TempDir::new("apply-steam");
        let store = temp.join("data");
        let game = fake_game(temp.path());
        let folder = cursor_folder(&game);
        let stock_main = bmp_32(0x00);
        fs::write(folder.join("cursor.bmp"), &stock_main).unwrap();
        fs::write(folder.join("cursor_ping.bmp"), bmp_32(0x11)).unwrap();

        let main = temp.join("cursor.bmp");
        let ping = temp.join("cursor_ping.bmp");
        fs::write(&main, bmp_32(0x80)).unwrap();
        fs::write(&ping, bmp_32(0x40)).unwrap();
        let first = import_files(&store, "Neon", &[main, ping]).unwrap();
        let second_source = temp.join("cursor.bmp");
        fs::write(&second_source, bmp_32(0x22)).unwrap();
        let second = import_files(&store, "Other", &[second_source]).unwrap();

        let applied = apply_plan(&store, &first.id, &game).unwrap();
        run_actions(
            &store,
            &applied.stock_backups,
            &applied.restores,
            &applied.state,
        );
        for write in &applied.writes {
            fs::copy(&write.from, &write.to).unwrap();
        }

        // Steam puts the stock cursor back and drops the file Sigil wrote.
        fs::write(folder.join("cursor.bmp"), &stock_main).unwrap();
        fs::remove_file(folder.join("cursor_ping.bmp")).unwrap();

        let plan = apply_plan(&store, &second.id, &game).unwrap();

        // The applied record no longer matches cursor.bmp, so it is stock again.
        assert_eq!(plan.stock_backups.len(), 1);
        assert_eq!(plan.stock_backups[0].name, "cursor.bmp");
        assert_eq!(plan.stock_backups[0].sha256, sha256_hex(&stock_main));
        // cursor_ping.bmp belonged to the previous pack, so its stock copy goes back.
        assert_eq!(plan.restores.len(), 1);
        assert_eq!(plan.restores[0].name, "cursor_ping.bmp");
        assert_eq!(
            plan.restores[0].from,
            Some(stock_dir(&store).join("cursor_ping.bmp"))
        );
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.writes[0].name, "cursor.bmp");
    }

    #[test]
    fn restore_plan_puts_stock_back_and_removes_files_with_no_stock_copy() {
        let temp = TempDir::new("restore");
        let store = temp.join("data");
        let game = fake_game(temp.path());
        let folder = cursor_folder(&game);
        let stock_main = bmp_32(0x00);
        fs::write(folder.join("cursor.bmp"), &stock_main).unwrap();

        let main = temp.join("cursor.bmp");
        let shop = temp.join("cursor_shop.bmp");
        fs::write(&main, bmp_32(0x80)).unwrap();
        fs::write(&shop, bmp_32(0x40)).unwrap();
        let pack = import_files(&store, "Neon", &[main, shop]).unwrap();

        let applied = apply_plan(&store, &pack.id, &game).unwrap();
        run_actions(
            &store,
            &applied.stock_backups,
            &applied.restores,
            &applied.state,
        );
        for write in &applied.writes {
            fs::copy(&write.from, &write.to).unwrap();
        }

        // Steam restores the stock cursor; cursor_shop.bmp has no stock copy at all.
        fs::write(folder.join("cursor.bmp"), &stock_main).unwrap();

        let plan = restore_plan(&store, &game).unwrap();

        assert_eq!(plan.stock_backups.len(), 1);
        assert_eq!(plan.stock_backups[0].name, "cursor.bmp");
        assert_eq!(plan.stock_backups[0].sha256, sha256_hex(&stock_main));
        assert_eq!(
            plan.restores
                .iter()
                .map(|restore| (restore.name.as_str(), restore.from.is_some()))
                .collect::<Vec<_>>(),
            vec![("cursor.bmp", true), ("cursor_shop.bmp", false)]
        );
        assert_eq!(plan.state.active_id, None);
        assert!(plan.state.applied.is_empty());

        run_actions(&store, &plan.stock_backups, &plan.restores, &plan.state);
        assert_eq!(fs::read(folder.join("cursor.bmp")).unwrap(), stock_main);
        assert!(!folder.join("cursor_shop.bmp").exists());

        let state = list(&store).unwrap();
        assert_eq!(state.active_id, None);
        assert!(state.applied.is_empty());
        // The pack itself is still installed and can be applied again.
        assert_eq!(state.packs.len(), 1);
    }

    #[test]
    fn restore_plan_without_a_cursor_folder_still_clears_the_record() {
        let temp = TempDir::new("restore-missing");
        let store = temp.join("data");
        let game = temp.join("Deadlock");

        let plan = restore_plan(&store, &game).unwrap();

        assert!(plan.stock_backups.is_empty());
        assert!(plan.restores.is_empty());
        assert_eq!(plan.state.active_id, None);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("was not found"));
    }

    #[test]
    fn delete_refuses_the_active_pack_and_removes_the_rest() {
        let temp = TempDir::new("delete");
        let store = temp.join("data");
        let game = fake_game(temp.path());
        let folder = cursor_folder(&game);
        fs::write(folder.join("cursor.bmp"), bmp_32(0x00)).unwrap();

        let first_source = temp.join("cursor.bmp");
        let second_source = temp.join("cursor_ping.bmp");
        fs::write(&first_source, bmp_32(0x80)).unwrap();
        fs::write(&second_source, bmp_32(0x40)).unwrap();
        let first = import_files(&store, "First", &[first_source]).unwrap();
        let second = import_files(&store, "Second", &[second_source]).unwrap();

        let plan = apply_plan(&store, &first.id, &game).unwrap();
        run_actions(&store, &plan.stock_backups, &plan.restores, &plan.state);

        let error = delete(&store, &first.id).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Restore the stock cursors first")
        );

        delete(&store, &second.id).unwrap();
        assert!(!pack_dir(&store, &second.id).exists());
        assert_eq!(list(&store).unwrap().packs.len(), 1);

        let error = delete(&store, "missing").unwrap_err();
        assert!(error.to_string().contains("no longer installed"));
    }

    #[test]
    fn a_broken_state_file_is_an_error_not_an_empty_store() {
        let temp = TempDir::new("state");
        let store = temp.join("data");
        fs::create_dir_all(packs_root(&store)).unwrap();
        fs::write(state_path(&store), "{").unwrap();

        let error = list(&store).unwrap_err();
        assert!(error.to_string().contains("Could not parse"));

        fs::write(
            state_path(&store),
            r#"{"packs":[{"id":"abc","name":"Old","files":["cursor.bmp"],"installedAt":"now"}],"activeId":"abc","applied":{"cursor.bmp":"00"}}"#,
        )
        .unwrap();
        let state = list(&store).unwrap();
        assert_eq!(state.packs.len(), 1);
        assert_eq!(state.active_id.as_deref(), Some("abc"));
        assert_eq!(
            state.applied.get("cursor.bmp").map(String::as_str),
            Some("00")
        );
    }

    #[test]
    fn a_hand_edited_pack_file_name_cannot_escape_the_cursor_folder() {
        let temp = TempDir::new("traversal");
        let store = temp.join("data");
        let game = fake_game(temp.path());
        fs::create_dir_all(pack_dir(&store, "abc")).unwrap();
        fs::write(
            state_path(&store),
            r#"{"packs":[{"id":"abc","name":"Bad","files":["../evil.bmp"],"installedAt":"now"}]}"#,
        )
        .unwrap();

        let error = apply_plan(&store, "abc", &game).unwrap_err();
        assert!(error.to_string().contains("not a cursor file name"));
    }

    #[test]
    fn apply_writes_the_pack_and_restore_puts_stock_back() {
        let temp = TempDir::new("apply-real");
        let store = temp.join("data");
        let game = fake_game(temp.path());
        let folder = cursor_folder(&game);
        let stock = bmp_32(0x00);
        fs::write(folder.join("cursor.bmp"), &stock).unwrap();
        let source = temp.join("cursor.bmp");
        fs::write(&source, bmp_32(0x80)).unwrap();
        let pack = import_files(&store, "Neon", &[source]).unwrap();

        let error = apply(&store, &pack.id, &game, false).unwrap_err();
        assert!(error.to_string().contains("not approved"), "{error}");
        assert_eq!(fs::read(folder.join("cursor.bmp")).unwrap(), stock);

        let plan = apply(&store, &pack.id, &game, true).unwrap();
        let written = fs::read(folder.join("cursor.bmp")).unwrap();
        assert_eq!(written, fs::read(&plan.writes[0].from).unwrap());
        assert_ne!(written, stock);
        assert_eq!(list(&store).unwrap().active_id.as_deref(), Some(pack.id.as_str()));

        restore(&store, &game, true).unwrap();
        assert_eq!(fs::read(folder.join("cursor.bmp")).unwrap(), stock);
        assert!(list(&store).unwrap().active_id.is_none());
    }

    #[test]
    fn sources_replace_in_place_and_reapply_is_detected() {
        let temp = TempDir::new("source");
        let store = temp.join("store");
        let game_root = fake_game(temp.path());
        let folder = cursor_folder(&game_root);
        fs::write(folder.join("cursor.bmp"), bmp_32(1)).unwrap();

        let first = temp.join("first.zip");
        write_zip(&first, &[("cursor.bmp", &bmp_32(200))]);
        let origin = PackSource { gamebanana_id: Some(77), gamebanana_file_id: Some(1) };
        let packs = import_archive_from(&store, &first, Some("Neon"), &origin).unwrap();
        assert_eq!(packs.len(), 1);
        let id = packs[0].id.clone();
        assert_eq!(packs[0].name, "Neon");
        assert_eq!(list(&store).unwrap().sources.get(&id), Some(&origin));
        assert_eq!(needs_reapply(&store, &game_root).unwrap(), None, "nothing is applied yet");
        assert_eq!(stock_preview_folder(&store, &game_root).unwrap(), folder);

        let plan = apply_plan(&store, &id, &game_root).unwrap();
        run_actions(&store, &plan.stock_backups, &plan.restores, &plan.state);
        for write in &plan.writes {
            fs::copy(&write.from, &write.to).unwrap();
        }
        assert_eq!(needs_reapply(&store, &game_root).unwrap(), None);
        assert_eq!(stock_preview_folder(&store, &game_root).unwrap(), stock_dir(&store));

        // Steam verifies the files: the stock cursor is back.
        fs::write(folder.join("cursor.bmp"), bmp_32(1)).unwrap();
        let reapply = needs_reapply(&store, &game_root).unwrap().unwrap();
        assert_eq!(reapply.pack_id, id);
        assert_eq!(reapply.restored_by_steam, vec!["cursor.bmp".to_string()]);
        assert!(!reapply.pack_updated);
        for write in &plan.writes {
            fs::copy(&write.from, &write.to).unwrap();
        }

        // A newer file of the same mod replaces the pack in place and keeps it active.
        let second = temp.join("second.zip");
        write_zip(&second, &[("cursor.bmp", &bmp_32(100))]);
        let update = PackSource { gamebanana_id: Some(77), gamebanana_file_id: Some(2) };
        let packs = import_archive_from(&store, &second, Some("Neon"), &update).unwrap();
        assert_eq!(packs[0].id, id);
        let state = list(&store).unwrap();
        assert_eq!(state.packs.len(), 1);
        assert_eq!(state.active_id.as_deref(), Some(id.as_str()));
        assert_eq!(state.sources.get(&id), Some(&update));
        assert_eq!(fs::read(pack_folder(&store, &id).join("cursor.bmp")).unwrap(), bmp_32(100));
        let reapply = needs_reapply(&store, &game_root).unwrap().unwrap();
        assert!(reapply.pack_updated);
        assert!(reapply.restored_by_steam.is_empty());
        assert!(
            fs::read_dir(packs_root(&store)).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with('.')),
            "no staging folder is left"
        );

        // Another mod is another pack; a plain import never replaces.
        let other = PackSource { gamebanana_id: Some(78), gamebanana_file_id: Some(9) };
        import_archive_from(&store, &second, None, &other).unwrap();
        import_archive(&store, &second).unwrap();
        assert_eq!(list(&store).unwrap().packs.len(), 3);
    }
}
