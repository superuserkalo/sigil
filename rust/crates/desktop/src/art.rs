//! The Locker art built into the executable (see `build.rs`).
//!
//! Sigil ships as one file. The first launch of a build writes its art once to
//! `<app data>/art/<id>` and points the art lookup there, so the Locker, Stats chips and launch
//! backdrops draw wherever the executable sits: the Desktop, Downloads, or inside a zip.

use std::fs;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/embedded_art.rs"));

/// Marks a folder whose files were all written.
const COMPLETE: &str = ".complete";

/// Unpack the built-in art (once per build) and point `DAH_LOCKER_ASSETS` at it. An explicit
/// `DAH_LOCKER_ASSETS` wins. Call before any thread starts: it sets a process variable, which
/// the CLI and agent host children inherit.
pub fn install() {
    if ART.is_empty() || std::env::var_os(deadlock_core::locker::LOCKER_ASSETS_ENV).is_some() {
        return;
    }
    let base = crate::data::data_dir().join("art");
    let Some(root) = unpack(&base) else { return };
    // SAFETY: called first thing in `main`, before Rune, the tray or any worker thread exists.
    unsafe { std::env::set_var(deadlock_core::locker::LOCKER_ASSETS_ENV, &root) };
}

/// The folder holding this build's art, written when missing. `None` when the disk refuses.
fn unpack(base: &Path) -> Option<PathBuf> {
    let root = base.join(ART_ID);
    if root.join(COMPLETE).is_file() {
        return Some(root);
    }
    // Write beside the final folder, then rename, so a crash or a second Sigil starting at the
    // same moment never sees half the files.
    let staging = base.join(format!("{ART_ID}.tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let written = (|| -> std::io::Result<()> {
        for (relative, bytes) in ART {
            let path = staging.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)?;
        }
        fs::write(staging.join(COMPLETE), ART_ID)
    })();
    if written.is_err() {
        let _ = fs::remove_dir_all(&staging);
        return None;
    }
    if fs::rename(&staging, &root).is_err() {
        // Another launch finished first, or an old partial folder is in the way.
        let _ = fs::remove_dir_all(&staging);
        if !root.join(COMPLETE).is_file() {
            let _ = fs::remove_dir_all(&root);
            return None;
        }
    }
    prune(base);
    Some(root)
}

/// Remove art from older builds.
fn prune(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // A staging folder may belong to a second Sigil unpacking right now.
        if name != ART_ID && !name.contains(".tmp-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_once_and_prunes_old_builds() {
        let base = std::env::temp_dir().join(format!("sigil-art-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("old-build")).unwrap();
        let root = unpack(&base).expect("unpacked");
        assert!(root.join(COMPLETE).is_file());
        for (relative, bytes) in ART {
            assert_eq!(fs::read(root.join(relative)).unwrap(), *bytes);
        }
        assert!(!base.join("old-build").exists());
        // A second call reuses the folder.
        assert_eq!(unpack(&base), Some(root));
        let _ = fs::remove_dir_all(&base);
    }
}
