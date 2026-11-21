//! Duplicate mods: the same mod installed twice, in another slot or under another name.
//!
//! Deadlock loads every `pakNN_dir.vpk`, so a mod installed twice (once from GameBanana, once
//! dragged in by hand, or imported from another manager) costs disk and load time, and a toggle
//! of one copy leaves the other on. A file name says nothing: the copy may sit in another slot,
//! in another addon folder, or carry another manager's stamp.
//!
//! Deadlock Mod Manager's upcoming VPK fingerprinting hashes a mod by its directory tree and
//! leaves its own identity stamp out of the hash. Sigil does the same with its own rules:
//!
//! - [`content_print`] hashes the sorted list of `(path, CRC32, size)` the directory tree holds.
//!   Every entry already carries the CRC of its bytes, so reading the tree (a few KiB at the
//!   start of the file) is enough; no file data is read. Paths are lower case with forward
//!   slashes, so a repack with other casing or another entry order is still the same mod.
//! - Files a manager writes to say who a mod is are left out, so a mod Sigil, Grimoire or DMM
//!   stamped matches its unstamped copy: the imprint at the root ([`IMPRINT_FILES`],
//!   `addoninfo.txt` and `modinfo.json`, written by Sigil and Grimoire) and DMM's stamp (an
//!   entry with the `dmm` extension directly in [`DMM_STAMP_DIR`], `.dmm/fingerprint.dmm` on
//!   DMM's `feat/vpk-fingerprinting`, the same rule its own content hash uses). The junk an OS
//!   file browser drops ([`JUNK_FILES`]) is left out too.
//! - A tree whose CRCs are all zero (some packers skip them) cannot be told apart by its tree,
//!   so its print is the sha256 of the directory file's bytes instead (`f1:`), the hash the
//!   unknown-mod lookup already uses ([`crate::identify::sha256_file`]).
//! - [`find_duplicates`] groups copies by print and picks the one to keep: the copy that is on,
//!   then the one the library knows (it keeps its name, picture and update check), then the
//!   lowest slot (it wins every shared path today). The rest are the extras the player can send
//!   to Sigil's trash (`trash::trash_slots`, an app action with Undo).
//!
//! Everything here reads. The removal is the app's, through the trash.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// The prefix of a print made from the directory tree.
pub const TREE_PREFIX: &str = "c1:";
/// The prefix of a print made from the directory file's bytes.
pub const FILE_PREFIX: &str = "f1:";

/// Root entries an imprint writes (Sigil's `mod_data::imprint_vpk`, Grimoire's "Imprint
/// installed mods"). Only at the root: a mod may ship its own `addoninfo.txt` deeper down.
pub const IMPRINT_FILES: [&str; 2] = ["addoninfo.txt", "modinfo.json"];
/// The folder DMM writes its identity stamp into. An entry there with the `dmm` extension is
/// the stamp; anything else in it, or a `.dmm` file elsewhere, is counted like any other file.
pub const DMM_STAMP_DIR: &str = ".dmm";
/// What an OS file browser drops into a folder before it is packed. Anywhere in the tree.
pub const JUNK_FILES: [&str; 3] = ["thumbs.db", ".ds_store", "desktop.ini"];

/// What the directory tree of one slot hashes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentPrint {
    /// `c1:<sha256>` from the tree, or `f1:<sha256>` from the file bytes.
    pub hash: String,
    /// Entries counted, stamps left out.
    pub entries: usize,
    /// Bytes the counted entries hold.
    pub bytes: u64,
}

/// One entry as the print reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintEntry {
    pub path: String,
    pub crc32: u32,
    pub size: u64,
}

/// Whether a normalised entry path (lower case, forward slashes) says who a mod is rather than
/// what it changes.
fn is_stamp(path: &str) -> bool {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    (dir.is_empty() && IMPRINT_FILES.contains(&name))
        || JUNK_FILES.contains(&name)
        || (dir == DMM_STAMP_DIR && name.ends_with(".dmm"))
}

fn normal(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

/// The tree print of a set of entries: `None` when nothing but stamps is left, or when every
/// entry with bytes has a zero CRC (the caller then hashes the file). Pure, so a test can pin it.
pub fn tree_print(entries: &[PrintEntry]) -> Option<ContentPrint> {
    let mut rows: Vec<(String, u32, u64)> = entries
        .iter()
        .map(|e| (normal(&e.path), e.crc32, e.size))
        .filter(|(path, _, _)| !path.is_empty() && !is_stamp(path))
        .collect();
    if rows.is_empty() {
        return None;
    }
    if rows.iter().filter(|(_, _, size)| *size > 0).all(|(_, crc, _)| *crc == 0)
        && rows.iter().any(|(_, _, size)| *size > 0)
    {
        return None;
    }
    rows.sort();
    rows.dedup();
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    for (path, crc, size) in &rows {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(crc.to_le_bytes());
        hasher.update(size.to_le_bytes());
        bytes += size;
    }
    Some(ContentPrint {
        hash: format!("{TREE_PREFIX}{}", hex(&hasher.finalize())),
        entries: rows.len(),
        bytes,
    })
}

/// The print of one directory VPK. Reads the tree only, or the whole directory file when the
/// tree carries no CRCs. `Ok(None)` for a VPK with nothing in it but stamps.
pub fn content_print(dir_vpk: &Path) -> Result<Option<ContentPrint>> {
    let directory = read_vpk_directory_with(dir_vpk, &ReadVpkOptions { keep_preload: false })?;
    let entries: Vec<PrintEntry> = directory
        .entries
        .iter()
        .map(|e| PrintEntry {
            path: e.path.clone(),
            crc32: e.crc32,
            size: u64::from(e.total_length).max(u64::from(e.preload_bytes) + u64::from(e.length)),
        })
        .collect();
    if let Some(print) = tree_print(&entries) {
        return Ok(Some(print));
    }
    let counted: Vec<&PrintEntry> = entries.iter().filter(|e| !is_stamp(&normal(&e.path))).collect();
    if counted.is_empty() {
        return Ok(None);
    }
    let sha = crate::identify::sha256_file(dir_vpk)?;
    Ok(Some(ContentPrint {
        hash: format!("{FILE_PREFIX}{sha}"),
        entries: counted.len(),
        bytes: counted.iter().map(|e| e.size).sum(),
    }))
}

/// One installed copy, as the duplicate check sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotCopy {
    pub slot: u16,
    pub enabled: bool,
    /// The library has a record of it (a name, a GameBanana id).
    pub known: bool,
    pub print: String,
    /// Bytes on disk, directory file and chunks.
    pub disk_bytes: u64,
}

/// Copies of one mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateSet {
    pub print: String,
    /// The copy to keep.
    pub keep: u16,
    /// The other copies, lowest slot first.
    pub extra: Vec<u16>,
    /// Bytes the extras take on disk.
    pub extra_bytes: u64,
}

/// Group copies by print. A print held once is no duplicate. Sets are sorted by the slot kept.
pub fn find_duplicates(copies: &[SlotCopy]) -> Vec<DuplicateSet> {
    let mut by_print: BTreeMap<&str, Vec<&SlotCopy>> = BTreeMap::new();
    for copy in copies {
        by_print.entry(copy.print.as_str()).or_default().push(copy);
    }
    let mut out: Vec<DuplicateSet> = by_print
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(print, mut members)| {
            // On first, then known, then the lowest slot.
            members.sort_by_key(|c| (!c.enabled, !c.known, c.slot));
            let keep = members[0].slot;
            let mut extra: Vec<&SlotCopy> = members[1..].to_vec();
            extra.sort_by_key(|c| c.slot);
            DuplicateSet {
                print: print.to_string(),
                keep,
                extra_bytes: extra.iter().map(|c| c.disk_bytes).sum(),
                extra: extra.iter().map(|c| c.slot).collect(),
            }
        })
        .collect();
    out.sort_by_key(|set| set.keep);
    out
}

/// Every slot directory file in every addon folder with its print, enabled or not. A file that
/// cannot be read is left out (the folder check reports it); a stamp-only VPK too.
pub fn scan(addons_dir: &Path) -> Result<Vec<(u16, bool, ContentPrint, u64)>> {
    let slots = crate::load_order::read_slot_files(addons_dir)?;
    let mut out = Vec::new();
    for (slot, files) in slots {
        let folder = crate::addons::slot_folder(addons_dir, slot);
        let dir_name = crate::addons::slot_name(slot, "_dir.vpk");
        let on = files.iter().find(|f| f.eq_ignore_ascii_case(&dir_name));
        let off = files.iter().find(|f| f.eq_ignore_ascii_case(&format!("{dir_name}{}", crate::addons::DISABLED_SUFFIX)));
        let Some(file) = on.or(off) else { continue };
        let Ok(Some(print)) = content_print(&folder.join(file)) else { continue };
        let disk: u64 = files
            .iter()
            .filter(|f| !f.to_ascii_lowercase().ends_with(crate::addons::DISABLED_SUFFIX) || Some(*f) == off.filter(|_| on.is_none()))
            .map(|f| std::fs::metadata(folder.join(f)).map(|m| m.len()).unwrap_or(0))
            .sum();
        out.push((slot, on.is_some(), print, disk));
    }
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn entry(path: &str, crc32: u32, size: u64) -> PrintEntry {
        PrintEntry { path: path.to_string(), crc32, size }
    }

    #[test]
    fn order_case_and_stamps_do_not_change_the_print() {
        let a = tree_print(&[entry("models/haze/haze.vmdl_c", 7, 100), entry("sounds/x.vsnd_c", 9, 5)]).unwrap();
        let b = tree_print(&[
            entry("addoninfo.txt", 1, 40),
            entry(".dmm/fingerprint.dmm", 1, 40),
            entry("Sounds\\X.vsnd_c", 9, 5),
            entry("models/haze/haze.vmdl_c", 7, 100),
            entry("modinfo.json", 3, 12),
        ])
        .unwrap();
        assert_eq!(a, b);
        assert!(a.hash.starts_with(TREE_PREFIX));
        assert_eq!(a.entries, 2);
        assert_eq!(a.bytes, 105);
    }

    #[test]
    fn only_the_imprint_at_the_root_and_dmms_own_stamp_are_left_out() {
        let base = tree_print(&[entry("a.vtex_c", 7, 100)]).unwrap();
        // DMM's rule: a `dmm` entry directly in `.dmm/`.
        assert_eq!(base, tree_print(&[entry("a.vtex_c", 7, 100), entry(".DMM/Fingerprint.dmm", 2, 128)]).unwrap());
        // Anything else is content.
        for other in ["scripts/addoninfo.txt", ".dmm/notes.json", ".dmm/deeper/fingerprint.dmm", "models/x.dmm"] {
            assert_ne!(base, tree_print(&[entry("a.vtex_c", 7, 100), entry(other, 2, 10)]).unwrap(), "{other} is counted");
        }
        // OS junk is left out wherever it sits.
        assert_eq!(base, tree_print(&[entry("a.vtex_c", 7, 100), entry("materials/Thumbs.db", 2, 10)]).unwrap());
    }

    #[test]
    fn other_bytes_or_paths_change_the_print() {
        let base = tree_print(&[entry("a.vtex_c", 7, 100)]).unwrap();
        assert_ne!(base, tree_print(&[entry("a.vtex_c", 8, 100)]).unwrap());
        assert_ne!(base, tree_print(&[entry("a.vtex_c", 7, 101)]).unwrap());
        assert_ne!(base, tree_print(&[entry("b.vtex_c", 7, 100)]).unwrap());
    }

    #[test]
    fn stamps_alone_or_missing_crcs_have_no_tree_print() {
        assert!(tree_print(&[entry("addoninfo.txt", 1, 40)]).is_none());
        assert!(tree_print(&[]).is_none());
        assert!(tree_print(&[entry("a.vtex_c", 0, 100), entry("b.vtex_c", 0, 3)]).is_none());
        // An empty file with no CRC next to real ones is fine.
        assert!(tree_print(&[entry("a.vtex_c", 4, 100), entry("empty.txt", 0, 0)]).is_some());
    }

    fn copy(slot: u16, enabled: bool, known: bool, print: &str) -> SlotCopy {
        SlotCopy { slot, enabled, known, print: print.to_string(), disk_bytes: 10 }
    }

    #[test]
    fn the_copy_kept_is_on_then_known_then_lowest() {
        let sets = find_duplicates(&[
            copy(3, false, true, "p"),
            copy(7, true, false, "p"),
            copy(9, false, false, "p"),
            copy(4, true, false, "q"),
            copy(2, true, true, "q"),
            copy(5, true, true, "lonely"),
        ]);
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0], DuplicateSet { print: "q".into(), keep: 2, extra: vec![4], extra_bytes: 10 });
        assert_eq!(sets[1], DuplicateSet { print: "p".into(), keep: 7, extra: vec![3, 9], extra_bytes: 20 });
    }

    #[test]
    fn a_real_vpk_prints_from_its_tree() {
        let addons = std::env::temp_dir().join(format!("sigil-duplicates-{}", std::process::id())).join("addons");
        let _ = std::fs::remove_dir_all(&addons);
        std::fs::create_dir_all(&addons).unwrap();
        pack(&addons, "pak01", &[("materials/a.vtex_c", b"one"), ("addoninfo.txt", b"\"x\" \"1\""), (".dmm/fingerprint.dmm", b"{}")]);
        pack(&addons, "pak05", &[("Materials/A.vtex_c", b"one")]);
        std::fs::rename(addons.join("pak05_dir.vpk"), addons.join("pak05_dir.vpk.disabled")).unwrap();
        pack(&addons, "pak06", &[("materials/a.vtex_c", b"two")]);
        let scanned = scan(&addons).unwrap();
        assert_eq!(scanned.len(), 3);
        assert_eq!(scanned[0].2.hash, scanned[1].2.hash, "the stamp and the casing are left out");
        assert_ne!(scanned[0].2.hash, scanned[2].2.hash);
        assert!(scanned[0].1 && !scanned[1].1);
        let _ = std::fs::remove_dir_all(addons.parent().unwrap());
    }

    /// Test helper, shared with `folder_repair`: pack `files` into `<root>/<name>_dir.vpk`, one
    /// self-contained file. Staging sits outside `root`, so a failed pack never leaves a
    /// subfolder in the addon folder under test.
    pub(crate) fn pack(root: &Path, name: &str, files: &[(&str, &[u8])]) {
        pack_with(root, name, files, None);
    }

    /// Test helper: pack `files` split into `chunk_size` byte parts beside the directory file.
    /// Returns the part files, in order.
    pub(crate) fn pack_split(root: &Path, name: &str, files: &[(&str, &[u8])], chunk_size: u64) -> Vec<std::path::PathBuf> {
        pack_with(root, name, files, Some(chunk_size))
    }

    fn pack_with(root: &Path, name: &str, files: &[(&str, &[u8])], chunk_size: Option<u64>) -> Vec<std::path::PathBuf> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let staging = std::env::temp_dir().join(format!(
            "sigil-test-pack-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&staging);
        for (path, data) in files {
            let target = staging.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, data).unwrap();
        }
        let mut options = crate::vpk_write::PackOptions::new(staging.clone(), root.to_path_buf(), name);
        let parts = match chunk_size {
            None => {
                crate::vpk_write::pack_vpk_single(&options).unwrap();
                Vec::new()
            }
            Some(size) => {
                options.chunk_size = size;
                crate::vpk_write::pack_vpk(&options).unwrap().chunk_paths
            }
        };
        let _ = std::fs::remove_dir_all(staging);
        parts
    }
}
