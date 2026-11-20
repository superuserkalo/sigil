//! Cache folders: content keys, atomic publish, last-use tracking and pruning.
//!
//! Layout under the cache dir the caller passes (`<data>/viewer-cache` in the app):
//!
//! ```text
//! heroes/<key>/<codename>.glb, <codename>.json, <codename>.cloth.json
//! hideout/<key>/scene.json, scene-<hero key>.json, room.json, models/*.glb, ...
//! ```
//!
//! An entry is built in `.tmp-*` beside its final name and renamed into place when complete,
//! so a reader never sees half an entry and two exports of the same key cannot interleave.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Result;

const USED_MARKER: &str = ".used";
const STAGING_PREFIX: &str = ".tmp-";
/// An entry read this recently is in use: prune leaves it alone.
const RECENT: Duration = Duration::from_secs(5 * 60);
/// A staging folder this old belongs to an export that died.
const ABANDONED: Duration = Duration::from_secs(60 * 60);

/// A 24 character hex key over the parts, in order.
pub(crate) fn key(parts: &[String]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0u8]);
    }
    let digest = hash.finalize();
    digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A folder being written. Dropped without [`Staging::commit`], it is removed.
pub(crate) struct Staging {
    pub dir: PathBuf,
    target: PathBuf,
    done: bool,
}

impl Staging {
    pub fn begin(parent: &Path, key: &str) -> Result<Staging> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(parent)?;
        let dir = parent.join(format!(
            "{STAGING_PREFIX}{key}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(Staging {
            dir,
            target: parent.join(key),
            done: false,
        })
    }

    /// Move the folder into place. When another export finished the same key first, its copy
    /// is kept and this one discarded: both hold the same content.
    pub fn commit(mut self) -> Result<PathBuf> {
        std::fs::write(self.dir.join(USED_MARKER), b"")?;
        match std::fs::rename(&self.dir, &self.target) {
            Ok(()) => {}
            Err(error) => {
                if !self.target.is_dir() {
                    return Err(error.into());
                }
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
        self.done = true;
        Ok(self.target.clone())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.done {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Write a file inside a committed entry atomically: a temp file renamed over the name.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(
        "{STAGING_PREFIX}{file_name}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temporary, bytes)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        // Windows will not rename over a file another process holds open.
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

/// Record a cache hit, so prune keeps entries in use.
pub(crate) fn touch(entry: &Path) {
    let _ = std::fs::write(entry.join(USED_MARKER), b"");
}

/// What [`prune_cache`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    pub removed: usize,
    pub freed_bytes: u64,
    pub kept_bytes: u64,
}

/// Remove the least recently used hero and Hideout entries until the cache holds at most
/// `keep_bytes`. Entries used in the last five minutes stay even past the budget, and
/// staging folders left by a crashed export are cleared once an hour old.
pub fn prune_cache(cache_dir: &Path, keep_bytes: u64) -> Result<PruneReport> {
    let mut report = PruneReport::default();
    let mut entries: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let now = SystemTime::now();

    for group in ["heroes", "hideout"] {
        let Ok(children) = std::fs::read_dir(cache_dir.join(group)) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            if !path.is_dir() {
                continue;
            }
            let name = child.file_name().to_string_lossy().into_owned();
            let size = dir_size(&path);
            if name.starts_with(STAGING_PREFIX) {
                let age = modified(&path).and_then(|time| now.duration_since(time).ok());
                if age.is_some_and(|age| age > ABANDONED) && std::fs::remove_dir_all(&path).is_ok()
                {
                    report.removed += 1;
                    report.freed_bytes += size;
                }
                continue;
            }
            let used = modified(&path.join(USED_MARKER))
                .or_else(|| modified(&path))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((used, size, path));
        }
    }

    let mut total: u64 = entries.iter().map(|(_, size, _)| size).sum();
    entries.sort_by_key(|(used, _, _)| *used);
    for (used, size, path) in entries {
        if total <= keep_bytes {
            break;
        }
        if now.duration_since(used).is_ok_and(|age| age < RECENT) {
            continue;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            total = total.saturating_sub(size);
            report.removed += 1;
            report.freed_bytes += size;
        }
    }
    report.kept_bytes = total;
    Ok(report)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

pub(crate) fn dir_size(path: &Path) -> u64 {
    let Ok(children) = std::fs::read_dir(path) else {
        return 0;
    };
    children
        .flatten()
        .map(|child| match child.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&child.path()),
            Ok(_) => child.metadata().map(|meta| meta.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}
