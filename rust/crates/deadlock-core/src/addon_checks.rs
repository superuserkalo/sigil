//! Checks on the addon folders that Deadlock Mod Manager runs and Sigil lacked: missing files
//! before a launch, read-only files, a summary of what the folders hold ("analyze addons"),
//! and the HUD layout check.
//!
//! - Missing files: a mod the library remembers whose slot file is gone (deleted outside the
//!   app), or a directory VPK that points at a `pakNN_MMM.vpk` chunk that is not there. The game
//!   either skips the mod or fails to load parts of it. DMM checks its manifest the same way
//!   before a launch.
//! - Read-only files: a slot file Windows or a sync tool marked read-only makes the next
//!   toggle, reorder or install fail halfway. DMM detects it and offers to clear the flag; Sigil
//!   detects it here and leaves the clear to the owner's app action (it changes game files).
//! - Analyze addons: per addon folder, the mods on and off, chunk files, chunks with no
//!   directory file, files the game ignores (wrong names, subfolders), temporary files a stopped
//!   reorder left, and slots the library does not know (candidates for identify).
//! - HUD check, after DMM's `hud_compatibility.rs` (behaviour only): for every Panorama layout
//!   (`panorama/layout/*.vxml_c`) an enabled mod wins, compare the panel ids in the mod's layout
//!   with the game's own layout. A panel the game added since the mod was made is missing from
//!   the mod's copy, which is how a HUD mod breaks after an update. The layouts are decompiled to
//!   XML with vpkmerge (already linked, ADR 0013) into a scratch folder the caller gives. Lines
//!   in `console.log` that name a missing panel or the layout are attached as evidence.
//!
//! Everything here reads. Nothing writes to the game install.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::vpk::{ReadVpkOptions, VPK_DIRECTORY_ARCHIVE, read_vpk_directory_with};

// ---------------------------------------------------------------------------
// Missing files
// ---------------------------------------------------------------------------

/// One problem found before a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingFile {
    /// The library remembers a mod in `slot` but no slot file is there.
    SlotGone { slot: u16, name: String },
    /// The slot's directory VPK needs chunk files that are not there.
    ChunksGone { slot: u16, file: String, chunks: Vec<String> },
}

impl MissingFile {
    pub fn slot(&self) -> u16 {
        match self {
            MissingFile::SlotGone { slot, .. } | MissingFile::ChunksGone { slot, .. } => *slot,
        }
    }
}

/// The chunk files a directory VPK references, by archive index, named for `dir_name`
/// (`pak05_dir.vpk` needs `pak05_000.vpk` and so on).
pub fn referenced_chunks(dir_vpk: &Path) -> Result<Vec<String>> {
    let directory = read_vpk_directory_with(dir_vpk, &ReadVpkOptions { keep_preload: false })?;
    let name = dir_vpk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let cut = lower.find("_dir.vpk").ok_or_else(|| Error::msg(format!("{name} is not a directory VPK.")))?;
    let stem = &name[..cut];
    let indices: BTreeSet<u16> = directory
        .entries
        .iter()
        .filter(|entry| entry.length > 0 && entry.archive_index != VPK_DIRECTORY_ARCHIVE)
        .map(|entry| entry.archive_index)
        .collect();
    Ok(indices.into_iter().map(|index| format!("{stem}_{index:03}.vpk")).collect())
}

/// Missing files across every addon folder: library mods whose slot is empty, and enabled or
/// disabled directory VPKs whose chunks are gone. `library` is `(slot, name)` for each mod Sigil
/// recorded. Sorted by slot.
pub fn missing_files(addons_dir: &Path, library: &[(u16, String)]) -> Result<Vec<MissingFile>> {
    let slots = crate::load_order::read_slot_files(addons_dir)?;
    let mut out = Vec::new();
    let mut named: BTreeSet<u16> = BTreeSet::new();
    for (slot, name) in library {
        let has_dir = slots
            .get(slot)
            .is_some_and(|files| files.iter().any(|f| f.to_ascii_lowercase().contains("_dir.vpk")));
        if !has_dir && named.insert(*slot) {
            out.push(MissingFile::SlotGone { slot: *slot, name: name.clone() });
        }
    }
    for (slot, files) in &slots {
        let folder = crate::addons::slot_folder(addons_dir, *slot);
        for file in files.iter().filter(|f| f.to_ascii_lowercase().contains("_dir.vpk")) {
            let Ok(needed) = referenced_chunks(&folder.join(file)) else {
                continue;
            };
            let gone: Vec<String> = needed
                .into_iter()
                .filter(|chunk| !files.iter().any(|f| f.eq_ignore_ascii_case(chunk)))
                .collect();
            if !gone.is_empty() {
                out.push(MissingFile::ChunksGone { slot: *slot, file: file.clone(), chunks: gone });
            }
        }
    }
    out.sort_by_key(MissingFile::slot);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Read-only files
// ---------------------------------------------------------------------------

fn is_read_only(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o200 == 0
    }
    #[cfg(not(unix))]
    {
        meta.permissions().readonly()
    }
}

/// Slot files (and the addon folders themselves) that are read-only, in every addon folder.
/// A toggle, reorder or install that renames or replaces one of these fails. Clearing the flag
/// is a game write the app performs after asking.
pub fn read_only_files(addons_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for (_, folder) in crate::addons::existing_folders(addons_dir) {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if crate::load_order::slot_of(&name).is_none() {
                continue;
            }
            if let Ok(meta) = entry.metadata()
                && meta.is_file()
                && is_read_only(&meta)
            {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------------
// Analyze addons
// ---------------------------------------------------------------------------

/// What one addon folder holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderSummary {
    pub index: usize,
    pub folder: PathBuf,
    pub enabled: usize,
    pub disabled: usize,
    pub chunks: usize,
    /// `pakNN_MMM.vpk` files whose slot has no directory file: dead weight the game never opens.
    pub orphan_chunks: Vec<String>,
    /// Files and folders the game ignores: wrong names, subfolders, archives left behind.
    pub ignored: Vec<String>,
    /// Temporary names a stopped reorder left (see [`crate::load_order::recover_temp_files`]).
    pub temp_files: Vec<String>,
    pub bytes: u64,
}

/// A file in an addon folder that no mod uses: an orphan chunk, a file the game never reads,
/// or a temporary name a stopped reorder left. Analyze offers to move these to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leftover {
    pub folder: PathBuf,
    pub file: String,
    pub kind: LeftoverKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LeftoverKind {
    /// `pakNN_MMM.vpk` whose slot has no directory file.
    OrphanChunk,
    /// A file the game ignores: a wrong name, a subfolder's contents, an archive left behind.
    Ignored,
    /// A temporary name a stopped reorder left.
    TempFile,
}

impl LeftoverKind {
    pub fn label(self) -> &'static str {
        match self {
            LeftoverKind::OrphanChunk => "chunk",
            LeftoverKind::Ignored => "ignored",
            LeftoverKind::TempFile => "temporary",
        }
    }
}

/// The whole picture across folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddonsAnalysis {
    pub folders: Vec<FolderSummary>,
    /// Slots with a directory file that the library has no record of.
    pub unknown_slots: Vec<u16>,
    /// Slots holding both `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled`.
    pub doubled_slots: Vec<u16>,
    pub read_only: Vec<PathBuf>,
    pub missing: Vec<MissingFile>,
    /// Files no mod uses, sorted by folder and name. Removing one goes through
    /// [`crate::trash::trash_loose`], so it can be undone.
    pub leftovers: Vec<Leftover>,
}

impl AddonsAnalysis {
    pub fn total_mods(&self) -> usize {
        self.folders.iter().map(|f| f.enabled + f.disabled).sum()
    }

    pub fn total_bytes(&self) -> u64 {
        self.folders.iter().map(|f| f.bytes).sum()
    }

    /// True when nothing needs the player's attention.
    pub fn is_clean(&self) -> bool {
        self.unknown_slots.is_empty()
            && self.doubled_slots.is_empty()
            && self.read_only.is_empty()
            && self.missing.is_empty()
            && self.leftovers.is_empty()
    }
}

/// Analyze every addon folder. `library` is `(slot, name)` for each mod Sigil recorded (pass
/// every library entry's slot; unknown slots are the rest).
pub fn analyze_addons(addons_dir: &Path, library: &[(u16, String)]) -> Result<AddonsAnalysis> {
    let mut analysis = AddonsAnalysis::default();
    let known: BTreeSet<u16> = library.iter().map(|(slot, _)| *slot).collect();
    for (index, folder) in crate::addons::existing_folders(addons_dir) {
        let mut summary = FolderSummary { index, folder: folder.clone(), ..FolderSummary::default() };
        let Ok(entries) = fs::read_dir(&folder) else {
            if index == 0 {
                analysis.folders.push(summary);
            }
            continue;
        };
        let temp: BTreeSet<String> = crate::load_order::leftover_temp_files(&folder).into_iter().collect();
        let mut dirs: BTreeMap<u16, (bool, bool)> = BTreeMap::new();
        let mut chunks: Vec<(u16, String)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata().ok();
            summary.bytes += meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()).unwrap_or(0);
            if temp.contains(&name) {
                analysis.leftovers.push(Leftover {
                    folder: folder.clone(),
                    file: name.clone(),
                    kind: LeftoverKind::TempFile,
                });
                summary.temp_files.push(name);
                continue;
            }
            let is_file = meta.as_ref().is_some_and(|m| m.is_file());
            match crate::load_order::slot_of(&name) {
                Some(local) if is_file => {
                    let slot = crate::addons::global_slot(index, local);
                    let lower = name.to_ascii_lowercase();
                    if lower.ends_with("_dir.vpk") {
                        summary.enabled += 1;
                        dirs.entry(slot).or_default().0 = true;
                    } else if lower.ends_with(".disabled") {
                        summary.disabled += 1;
                        dirs.entry(slot).or_default().1 = true;
                    } else {
                        summary.chunks += 1;
                        chunks.push((slot, name));
                    }
                }
                _ => {
                    // A folder is reported but never offered for removal: only plain files
                    // the game skips become leftovers.
                    if is_file {
                        analysis.leftovers.push(Leftover {
                            folder: folder.clone(),
                            file: name.clone(),
                            kind: LeftoverKind::Ignored,
                        });
                    }
                    summary.ignored.push(name)
                }
            }
        }
        summary.orphan_chunks = chunks
            .into_iter()
            .filter(|(slot, _)| !dirs.contains_key(slot))
            .map(|(_, name)| name)
            .collect();
        for file in &summary.orphan_chunks {
            analysis.leftovers.push(Leftover {
                folder: folder.clone(),
                file: file.clone(),
                kind: LeftoverKind::OrphanChunk,
            });
        }
        for (slot, (on, off)) in &dirs {
            if *on && *off {
                analysis.doubled_slots.push(*slot);
            }
            if !known.contains(slot) {
                analysis.unknown_slots.push(*slot);
            }
        }
        summary.orphan_chunks.sort();
        summary.ignored.sort();
        summary.temp_files.sort();
        analysis.folders.push(summary);
    }
    analysis.read_only = read_only_files(addons_dir)?;
    analysis.missing = missing_files(addons_dir, library)?;
    analysis
        .leftovers
        .sort_by(|a, b| a.folder.cmp(&b.folder).then_with(|| a.file.cmp(&b.file)));
    Ok(analysis)
}

// ---------------------------------------------------------------------------
// HUD check
// ---------------------------------------------------------------------------

/// How one layout compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudStatus {
    Compared,
    /// The game has no layout at this path: the mod adds its own, nothing to compare.
    NoBaseline,
    /// A layout could not be decompiled.
    Unsupported,
}

/// The result for one layout an enabled mod wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudCheck {
    pub slot: u16,
    /// Game path, `panorama/layout/....vxml_c`.
    pub layout: String,
    pub status: HudStatus,
    /// Panel ids the game's layout has and the mod's copy lacks.
    pub missing_panels: Vec<String>,
    pub detail: Option<String>,
    /// `console.log` lines that name a missing panel or this layout, newest first.
    pub console: Vec<String>,
}

impl HudCheck {
    /// True when the mod's layout is behind the game's.
    pub fn is_outdated(&self) -> bool {
        !self.missing_panels.is_empty()
    }
}

/// Panel ids in a Panorama layout's XML: every `id="..."` attribute outside `<snippets>`
/// (snippet ids are templates, not panels the game builds; DMM skips them too).
pub fn panel_ids(xml: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut rest = xml;
    let mut in_snippets = 0usize;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("/snippets") {
            in_snippets = in_snippets.saturating_sub(1);
        } else if lower.starts_with("snippets") && !tag.trim_end().ends_with('/') {
            in_snippets += 1;
        } else if in_snippets == 0
            && !lower.starts_with('/')
            && !lower.starts_with('!')
            && !lower.starts_with('?')
            && let Some(id) = attribute(tag, "id")
        {
            ids.insert(id);
        }
        rest = &rest[end.min(rest.len())..];
    }
    ids
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut search = tag;
    while let Some(at) = search.find(name) {
        let before = search[..at].chars().last();
        let after = &search[at + name.len()..];
        let after_trim = after.trim_start();
        if before.is_some_and(char::is_whitespace) && after_trim.starts_with('=') {
            let value = after_trim[1..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let body = &value[1..];
                let close = body.find(quote)?;
                let id = body[..close].trim();
                return (!id.is_empty()).then(|| id.to_string());
            }
        }
        search = &search[at + name.len()..];
    }
    None
}

/// Bytes of `console.log` read from its end.
pub const CONSOLE_LIMIT: u64 = 256 * 1024;

/// HUD-related lines from the end of `console.log`, newest first, at most 40: "unable to find
/// child", "unable to load layout file", and layout errors.
pub fn console_layout_lines(console_log: &Path) -> Vec<String> {
    let Ok(mut file) = fs::File::open(console_log) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file.seek(SeekFrom::Start(len.saturating_sub(CONSOLE_LIMIT))).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(CONSOLE_LIMIT).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if len > CONSOLE_LIMIT && !lines.is_empty() {
        lines.remove(0);
    }
    lines
        .into_iter()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("unable to find child")
                || lower.contains("unable to load layout file")
                || (lower.contains("layout") && (lower.contains("failed") || lower.contains("error")))
        })
        .rev()
        .take(40)
        .map(|line| line.chars().take(500).collect())
        .collect()
}

/// Keep the console lines that name one of the missing panels or the layout itself.
pub fn correlate(lines: &[String], layout: &str, missing: &[String]) -> Vec<String> {
    let source = layout.trim_end_matches("_c").to_ascii_lowercase();
    lines
        .iter()
        .filter(|line| {
            missing.iter().any(|id| line.contains(&format!("'{id}'")) || line.contains(&format!("\"{id}\"")))
                || line.replace('\\', "/").to_ascii_lowercase().contains(&source)
        })
        .cloned()
        .collect()
}

/// Decompile `paths` from `vpk` into `out` and read each layout's XML. A path that could not be
/// decompiled is absent from the map.
fn layouts_xml(vpk: &Path, paths: &[String], out: &Path) -> Result<BTreeMap<String, String>> {
    let options = vpkmerge_core::PanoramaDumpOptions {
        include_all: false,
        include_raw: false,
        mips: false,
        prefixes: paths.to_vec(),
    };
    let report = vpkmerge_core::dump_panorama_workspace(vpk, out, &options)
        .map_err(|error| Error::msg(format!("Could not read the layouts in {}: {error:#}", vpk.display())))?;
    let mut xml = BTreeMap::new();
    for entry in report.entries {
        if entry.mode != vpkmerge_core::panorama::PanoramaDumpMode::LayoutXml {
            continue;
        }
        if let Some(source) = entry.source_path
            && let Ok(text) = fs::read_to_string(out.join(source))
        {
            xml.insert(entry.entry, text);
        }
    }
    Ok(xml)
}

/// DMM's HUD check. `enabled` is every enabled mod's directory VPK with its slot, in any order;
/// the lowest slot wins a layout two mods ship, and only the winner is compared. `game_pak01`
/// is the game's `pak01_dir.vpk`, `console_log` the game's `citadel/console.log` (may be
/// missing), `scratch` an empty app-data folder for the decompiled XML (removed afterwards).
pub fn hud_check(
    enabled: &[(u16, PathBuf)],
    game_pak01: &Path,
    console_log: &Path,
    scratch: &Path,
) -> Result<Vec<HudCheck>> {
    let mut winners: BTreeMap<String, (u16, PathBuf)> = BTreeMap::new();
    let mut sorted = enabled.to_vec();
    sorted.sort_by_key(|(slot, _)| *slot);
    for (slot, vpk) in &sorted {
        let Ok(directory) = read_vpk_directory_with(vpk, &ReadVpkOptions { keep_preload: false }) else {
            continue;
        };
        for entry in directory.entries {
            let path = entry.path.to_ascii_lowercase();
            if path.starts_with("panorama/layout/") && path.ends_with(".vxml_c") {
                winners.entry(entry.path.clone()).or_insert_with(|| (*slot, vpk.clone()));
            }
        }
    }
    if winners.is_empty() {
        return Ok(Vec::new());
    }
    let console = console_layout_lines(console_log);
    let run = scratch.join(format!("hud-{}", std::process::id()));
    let _ = fs::remove_dir_all(&run);
    let result = (|| -> Result<Vec<HudCheck>> {
        let all: Vec<String> = winners.keys().cloned().collect();
        let game = layouts_xml(game_pak01, &all, &run.join("game"))?;
        let game_has: BTreeSet<String> = {
            let directory = read_vpk_directory_with(game_pak01, &ReadVpkOptions { keep_preload: false })?;
            directory.entries.into_iter().map(|e| e.path).filter(|p| winners.contains_key(p)).collect()
        };
        let mut by_vpk: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        for (path, (_, vpk)) in &winners {
            by_vpk.entry(vpk.clone()).or_default().push(path.clone());
        }
        let mut mods: BTreeMap<String, String> = BTreeMap::new();
        for (index, (vpk, paths)) in by_vpk.iter().enumerate() {
            if let Ok(found) = layouts_xml(vpk, paths, &run.join(format!("mod{index}"))) {
                mods.extend(found);
            }
        }
        let mut checks = Vec::new();
        for (path, (slot, _)) in &winners {
            let mut check = HudCheck {
                slot: *slot,
                layout: path.clone(),
                status: HudStatus::Compared,
                missing_panels: Vec::new(),
                detail: None,
                console: Vec::new(),
            };
            if !game_has.contains(path) {
                check.status = HudStatus::NoBaseline;
                check.detail = Some("The game has no layout here; the mod adds its own.".to_string());
            } else {
                match (game.get(path), mods.get(path)) {
                    (Some(base), Some(ours)) => {
                        let have = panel_ids(ours);
                        check.missing_panels = panel_ids(base).difference(&have).cloned().collect();
                    }
                    _ => {
                        check.status = HudStatus::Unsupported;
                        check.detail = Some("The layout could not be decompiled.".to_string());
                    }
                }
            }
            check.console = correlate(&console, path, &check.missing_panels);
            checks.push(check);
        }
        Ok(checks)
    })();
    let _ = fs::remove_dir_all(&run);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-addon-checks-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pack(root: &Path, name: &str, files: &[(&str, &[u8])], chunk_size: u64) -> (PathBuf, Vec<PathBuf>) {
        let staging = root.join(format!(".staging-{name}"));
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        let mut options = PackOptions::new(staging.clone(), root.to_path_buf(), name);
        options.chunk_size = chunk_size;
        let packed = pack_vpk(&options).unwrap();
        let _ = fs::remove_dir_all(staging);
        (packed.dir_path, packed.chunk_paths)
    }

    #[test]
    fn finds_missing_slots_and_chunks() {
        let root = scratch("missing");
        let addons = root.join("addons");
        fs::create_dir_all(&addons).unwrap();
        let (_, chunks) = pack(&addons, "pak02", &[("a/a.txt", b"aaaa"), ("b/b.txt", b"bbbb")], 4);
        assert_eq!(chunks.len(), 2);
        fs::remove_file(&chunks[1]).unwrap();
        pack(&addons, "pak03", &[("c/c.txt", b"c")], 0);
        let library = vec![(1, "Gone mod".to_string()), (3, "Fine".to_string())];
        let missing = missing_files(&addons, &library).unwrap();
        assert_eq!(
            missing,
            vec![
                MissingFile::SlotGone { slot: 1, name: "Gone mod".into() },
                MissingFile::ChunksGone { slot: 2, file: "pak02_dir.vpk".into(), chunks: vec!["pak02_001.vpk".into()] },
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn analyzes_folders_and_read_only_files() {
        let root = scratch("analyze");
        let addons = root.join("addons");
        fs::create_dir_all(&addons).unwrap();
        pack(&addons, "pak01", &[("a/a.txt", b"a")], 0);
        fs::write(addons.join("pak02_dir.vpk.disabled"), b"x").unwrap();
        fs::write(addons.join("pak02_dir.vpk"), b"x").unwrap();
        fs::write(addons.join("pak07_000.vpk"), b"orphan").unwrap();
        fs::write(addons.join("my_mod.vpk"), b"ignored").unwrap();
        fs::create_dir_all(addons.join("subfolder")).unwrap();
        let readonly = addons.join("pak02_dir.vpk");
        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&readonly, permissions).unwrap();

        let analysis = analyze_addons(&addons, &[(1, "Known".into())]).unwrap();
        let main = &analysis.folders[0];
        assert_eq!((main.enabled, main.disabled), (2, 1));
        assert_eq!(main.orphan_chunks, vec!["pak07_000.vpk".to_string()]);
        assert_eq!(main.ignored, vec!["my_mod.vpk".to_string(), "subfolder".to_string()]);
        assert_eq!(analysis.unknown_slots, vec![2]);
        assert_eq!(analysis.doubled_slots, vec![2]);
        assert_eq!(analysis.read_only, vec![readonly.clone()]);
        assert_eq!(
            analysis
                .leftovers
                .iter()
                .map(|leftover| (leftover.file.as_str(), leftover.kind))
                .collect::<Vec<_>>(),
            vec![
                ("my_mod.vpk", LeftoverKind::Ignored),
                ("pak07_000.vpk", LeftoverKind::OrphanChunk),
            ],
            "a folder is reported but is not a leftover"
        );
        assert!(analysis.leftovers.iter().all(|leftover| leftover.folder == addons));
        assert!(!analysis.is_clean());
        assert_eq!(analysis.total_mods(), 3);

        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&readonly, permissions).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn panel_ids_skip_snippets_and_closing_tags() {
        let xml = r#"<root>
            <snippets><snippet name="row"><Panel id="TemplateOnly"/></snippet></snippets>
            <Panel id="Hud" class="a">
              <Label id='Health' text="x"/>
              <Panel class="noid" data-id="Fake"/>
            </Panel>
            <!-- <Panel id="Commented"/> -->
          </root>"#;
        let ids = panel_ids(xml);
        assert_eq!(ids, BTreeSet::from(["Hud".to_string(), "Health".to_string()]));
    }

    #[test]
    fn console_lines_are_filtered_and_correlated() {
        let root = scratch("console");
        let log = root.join("console.log");
        fs::write(
            &log,
            "boot ok\nUnable to find child 'NewPanel'\nUnable to load layout file 'panorama/layout/hud.vxml'\nother\n",
        )
        .unwrap();
        let lines = console_layout_lines(&log);
        assert_eq!(lines.len(), 2);
        assert_eq!(correlate(&lines, "panorama/layout/hud.vxml_c", &["NewPanel".into()]).len(), 2);
        assert_eq!(correlate(&lines, "panorama/layout/other.vxml_c", &[]).len(), 0);
        assert!(console_layout_lines(&root.join("missing.log")).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mod_without_layouts_needs_no_hud_check() {
        let root = scratch("hud");
        let (vpk, _) = pack(&root, "pak01", &[("materials/a.vmat_c", b"a")], 0);
        let checks = hud_check(&[(1, vpk.clone())], &vpk, &root.join("console.log"), &root.join("s")).unwrap();
        assert!(checks.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
