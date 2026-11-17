//! Leftover archives in the addons folder.
//!
//! Some guides tell players to unpack a mod inside `game/citadel/addons`, which leaves the
//! `.zip`, `.7z` or `.rar` behind. The game ignores them, but they hide real problems in a
//! folder listing and use disk. This lists them and, on the user's confirm, deletes them. Only
//! regular files directly in the addons folder with an archive extension are touched; VPKs,
//! folders and anything else stay.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const ARCHIVE_EXTENSIONS: [&str; 6] = ["zip", "7z", "rar", "tar", "gz", "bz2"];

fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ARCHIVE_EXTENSIONS.iter().any(|a| e.eq_ignore_ascii_case(a)))
}

/// Archives sitting directly in the addons folder, with their sizes.
pub fn leftover_archives(addons: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(addons) else {
        return Vec::new();
    };
    let mut out: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            (meta.is_file() && is_archive(&entry.path())).then(|| (entry.path(), meta.len()))
        })
        .collect();
    out.sort();
    out
}

/// Delete the leftover archives. `approve` must be true: the caller passes the user's confirm.
/// Returns what was removed and the bytes freed.
///
/// Only the archives the user was shown (`shown`) go, and only while each is still a regular
/// archive file directly in `addons` with the size it was listed at. A file that appeared or
/// changed after the list was drawn is left alone.
pub fn remove_leftover_archives(
    addons: &Path,
    shown: &[(PathBuf, u64)],
    approve: bool,
) -> Result<(Vec<PathBuf>, u64)> {
    if !approve {
        return Err(Error::Message("Removing archives needs the user's confirm".into()));
    }
    let current = leftover_archives(addons);
    let mut removed = Vec::new();
    let mut freed = 0;
    for (path, bytes) in current.into_iter().filter(|item| shown.contains(item)) {
        fs::remove_file(&path).map_err(|e| Error::Message(format!("{}: {e}", path.display())))?;
        freed += bytes;
        removed.push(path);
    }
    Ok((removed, freed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_archives_directly_in_addons() {
        let addons = std::env::temp_dir().join(format!("sigil-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&addons);
        fs::create_dir_all(addons.join("sub")).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), b"vpk").unwrap();
        fs::write(addons.join("Skin.ZIP"), b"12345").unwrap();
        fs::write(addons.join("sound.7z"), b"1").unwrap();
        fs::write(addons.join("sub").join("deep.zip"), b"1").unwrap();
        fs::write(addons.join("readme.txt"), b"1").unwrap();
        assert_eq!(leftover_archives(&addons).len(), 2);
        let shown = leftover_archives(&addons);
        assert!(remove_leftover_archives(&addons, &shown, false).is_err());
        assert!(addons.join("Skin.ZIP").exists(), "nothing goes without approval");
        // Appeared after the list was drawn: not the user's choice, stays.
        fs::write(addons.join("late.zip"), b"1").unwrap();
        let (removed, freed) = remove_leftover_archives(&addons, &shown, true).unwrap();
        assert_eq!((removed.len(), freed), (2, 6));
        assert!(addons.join("late.zip").exists());
        assert!(addons.join("pak01_dir.vpk").exists());
        assert!(addons.join("sub").join("deep.zip").exists());
        assert!(addons.join("readme.txt").exists());
        let _ = fs::remove_dir_all(&addons);
    }
}
