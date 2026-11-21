//! Deadlock Mod Manager's identity stamp, read only.
//!
//! DMM's VPK fingerprinting work (branch `feat/vpk-fingerprinting`, unreleased) writes a small
//! JSON document into every VPK it manages, at `.dmm/fingerprint.dmm`, so the file can be
//! matched to its mod after a rename or a move. The engine ignores the entry. Sigil keeps its
//! own sidecar model (`identify.rs`, the install journal) and never writes this stamp; it only
//! reads it so a mod imported from DMM, or a renamed DMM file found in the addons folder, is
//! still tied to its GameBanana page. Behaviour only: the format below is described from the
//! branch, no DMM code is used.
//!
//! The document: `{version, id, modId, originalName, issuedAt, contentHash}`. `modId` is
//! DMM's remote id, the GameBanana submission id for GameBanana mods.

use std::path::Path;

use serde::Deserialize;

use crate::vpk::{ReadVpkOptions, read_vpk_directory_with, read_vpk_entry_data};

/// Where DMM puts the stamp inside a VPK.
pub const STAMP_PATH: &str = ".dmm/fingerprint.dmm";

/// The newest stamp format this reader understands. A newer one is ignored.
pub const STAMP_VERSION: u32 = 1;

/// A stamp is a few hundred bytes; anything larger is not one.
const MAX_STAMP_BYTES: u32 = 16 * 1024;

/// What DMM stamped into a VPK.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmmStamp {
    pub version: u32,
    /// Unique per physical file.
    #[serde(default)]
    pub id: String,
    /// DMM's id for the mod: the GameBanana submission id for GameBanana mods.
    pub mod_id: String,
    /// The file name the mod shipped with, before DMM renamed it to a slot.
    #[serde(default)]
    pub original_name: String,
    #[serde(default)]
    pub issued_at: u64,
    /// SHA-256 of the VPK without the stamp.
    #[serde(default)]
    pub content_hash: String,
}

impl DmmStamp {
    /// The GameBanana submission id, when `mod_id` is one (a plain positive number).
    pub fn gamebanana_id(&self) -> Option<u64> {
        let id = self.mod_id.trim();
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        id.parse::<u64>().ok().filter(|id| *id > 0)
    }

    /// A readable name from the shipped file name (`neon_haze_dir.vpk` is `neon_haze`).
    pub fn original_stem(&self) -> Option<String> {
        let name = self.original_name.trim();
        if name.is_empty() {
            return None;
        }
        let lower = name.to_ascii_lowercase();
        let mut end = name.len();
        for suffix in ["_dir.vpk", ".vpk"] {
            if lower.ends_with(suffix) {
                end = name.len() - suffix.len();
                break;
            }
        }
        let stem = name[..end].trim();
        (!stem.is_empty()).then(|| stem.to_string())
    }
}

/// Parse a stamp document. None for anything unreadable or newer than this reader.
pub fn parse(payload: &[u8]) -> Option<DmmStamp> {
    let stamp: DmmStamp = serde_json::from_slice(payload).ok()?;
    (stamp.version <= STAMP_VERSION && !stamp.mod_id.trim().is_empty()).then_some(stamp)
}

/// Read DMM's stamp from a directory VPK. Reads only the tree and the stamp entry. Read only.
pub fn read(path: &Path) -> Option<DmmStamp> {
    let directory = read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: true }).ok()?;
    let entry = directory.by_path.get(STAMP_PATH)?;
    if entry.total_length > MAX_STAMP_BYTES {
        return None;
    }
    parse(&read_vpk_entry_data(&directory, entry).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};
    use std::fs;
    use std::path::PathBuf;

    fn root(tag: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/dmm-stamp-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    pub(crate) fn stamped_vpk(root: &Path, name: &str, stamp: Option<&str>) -> PathBuf {
        let staging = root.join(format!("staging-{name}"));
        let model = staging.join("models/heroes/haze/haze.vmdl_c");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(model, b"model").unwrap();
        if let Some(stamp) = stamp {
            fs::create_dir_all(staging.join(".dmm")).unwrap();
            fs::write(staging.join(STAMP_PATH), stamp).unwrap();
        }
        pack_vpk(&PackOptions::new(staging, root.join("out"), name)).unwrap().dir_path
    }

    const STAMP: &str = r#"{"version":1,"id":"0f1e2d3c4b5a69788796a5b4c3d2e1f0","modId":"512345","originalName":"neon_haze_dir.vpk","issuedAt":1763000000,"contentHash":"ab"}"#;

    #[test]
    fn a_stamped_vpk_names_its_gamebanana_mod() {
        let root = root("read");
        let stamped = stamped_vpk(&root, "pak07", Some(STAMP));
        let stamp = read(&stamped).expect("the stamp is read");
        assert_eq!(stamp.gamebanana_id(), Some(512_345));
        assert_eq!(stamp.original_stem().as_deref(), Some("neon_haze"));
        let plain = stamped_vpk(&root, "pak08", None);
        assert_eq!(read(&plain), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn newer_or_local_stamps_give_no_gamebanana_id() {
        assert_eq!(parse(STAMP.replace("\"version\":1", "\"version\":2").as_bytes()), None);
        assert_eq!(parse(b"not json"), None);
        let local = parse(STAMP.replace("512345", "local-abc").as_bytes()).unwrap();
        assert_eq!(local.gamebanana_id(), None);
        assert_eq!(parse(STAMP.replace("\"512345\"", "\"\"").as_bytes()), None);
    }
}
