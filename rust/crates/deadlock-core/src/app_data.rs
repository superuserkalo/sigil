//! Sigil's data folder: where it is, and the one-time move from the project's old name.
//!
//! Sigil keeps its settings, profiles, gameinfo.gi backups, the vanilla stash, the trash, caches,
//! update staging and the agent workspace in one folder under the platform's data directory:
//!
//! - Windows: `%APPDATA%\Sigil`
//! - macOS: `~/Library/Application Support/Sigil`
//! - Linux: `$XDG_DATA_HOME/Sigil`, else `~/.local/share/Sigil`
//!
//! Builds up to 0.1.42 called it `DeadlockAgentHarness` (and the standalone CLI and agent host
//! used `deadlock-agent-harness` on Linux). The first start of a newer build moves the old folder
//! to the new name with one rename in the same parent, which is atomic on one volume:
//!
//! - new folder missing, old present: rename. When the rename fails (another Sigil holds the
//!   folder, a file is open, permissions), this run keeps using the old folder and the next start
//!   tries again. Nothing is ever copied, so nothing is ever half moved.
//! - both present: the new one wins and the old one is left untouched. No merge.
//! - neither present: the new one is created on first write, as before.
//!
//! `SIGIL_DATA_DIR` (or the older `DAH_DATA_DIR`) overrides the folder and skips the move. The
//! desktop app sets `SIGIL_DATA_DIR` to the folder it chose, so the CLI and agent host it starts
//! use the same one and never move it under the running app.
//!
//! Records written before the move may hold absolute paths inside the old folder; [`rebase`]
//! maps those onto the folder in use.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The data folder's name under the platform data directory.
pub const LEAF: &str = "Sigil";
/// Names older builds used, most important first: the desktop app's own folder, then the
/// standalone CLI's Linux folder.
pub const LEGACY_LEAVES: [&str; 2] = ["DeadlockAgentHarness", "deadlock-agent-harness"];
/// Overrides the data folder.
pub const ENV: &str = "SIGIL_DATA_DIR";
/// The older override, still read when [`ENV`] is unset.
pub const LEGACY_ENV: &str = "DAH_DATA_DIR";
/// The lock file the macOS single-instance claim holds while a Sigil runs
/// (`desktop/src/single_instance.rs`).
const INSTANCE_LOCK: &str = "sigil.lock";
/// Written into the new folder once the old one was noted as left behind, so the log line
/// appears once and not on every start.
const NOTED: &str = ".legacy-folder-noted";
/// Set by Rune on a process that relaunches another (an update or a restart): that process is
/// still quitting when this one starts.
const RELAUNCH_AFTER: &str = "RUNE_RELAUNCH_AFTER";
/// The longest a relaunch waits for the previous process to let go of the old folder.
const RELAUNCH_WAIT: Duration = Duration::from_secs(10);

/// What the start did with the data folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `SIGIL_DATA_DIR` or `DAH_DATA_DIR` names the folder; nothing was looked at.
    Override,
    /// The new folder already exists and no old one is beside it.
    Current,
    /// Neither exists: the new folder is created on first write.
    Fresh,
    /// The old folder was renamed to the new name.
    Moved { from: PathBuf },
    /// Both exist: the new one is used, the old one left alone. `first` is true the first time
    /// this was seen (the start logs it then).
    BothKept { old: PathBuf, first: bool },
    /// The rename failed: this run uses the old folder and the next start tries again.
    Deferred { old: PathBuf, error: String },
}

impl Outcome {
    /// One line for the log, or `None` when there is nothing to say.
    pub fn log_line(&self) -> Option<String> {
        match self {
            Outcome::Override | Outcome::Current | Outcome::Fresh => None,
            Outcome::Moved { from } => Some(format!(
                "Moved Sigil's data folder from {} to its new name {LEAF}",
                from.display()
            )),
            Outcome::BothKept { old, first: true } => Some(format!(
                "An old data folder is still at {}; Sigil uses {LEAF} and leaves the old one untouched",
                old.display()
            )),
            Outcome::BothKept { first: false, .. } => None,
            Outcome::Deferred { old, error } => Some(format!(
                "Could not rename the data folder {} to {LEAF} ({error}); using it as it is and trying again next start",
                old.display()
            )),
        }
    }
}

/// The platform data directory the folder lives in, from the environment alone.
pub fn platform_base() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").filter(|h| !h.is_empty()).map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")))
    }
}

/// The override from the environment: `SIGIL_DATA_DIR`, else `DAH_DATA_DIR`. Empty values do
/// not count.
pub fn override_dir() -> Option<PathBuf> {
    [ENV, LEGACY_ENV]
        .iter()
        .filter_map(|name| std::env::var_os(name))
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

static CHOSEN: OnceLock<(PathBuf, Outcome)> = OnceLock::new();

/// The data folder. Without an override and before [`init`], it picks the folder that exists
/// (the new name, else the old one) without moving anything, so tests and tools that never call
/// [`init`] leave the player's folder where it is.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = override_dir() {
        return dir;
    }
    CHOSEN
        .get_or_init(|| match platform_base() {
            Some(base) => (pick_in(&base), Outcome::Current),
            None => (PathBuf::from(".dah"), Outcome::Fresh),
        })
        .0
        .clone()
}

/// Run the move, once per process, and say what it did. Every entry point calls this before
/// anything reads or writes the folder: the desktop app first thing in `main`, the CLI in
/// `dah_rs::run_from` and the agent host in `dah_agent::run`. A later call returns the first
/// answer.
pub fn init() -> Outcome {
    if override_dir().is_some() {
        return Outcome::Override;
    }
    CHOSEN
        .get_or_init(|| match platform_base() {
            Some(base) => {
                let wait = std::env::var_os(RELAUNCH_AFTER).is_some();
                migrate_in(&base, &|old| wait_until_free(old, wait))
            }
            // No home or AppData: the folder beside the working directory, as before.
            None => (PathBuf::from(".dah"), Outcome::Fresh),
        })
        .1
        .clone()
}

/// The folder to use in `base` without moving anything: the new name when it exists, else an
/// old one that does, else the new name.
pub fn pick_in(base: &Path) -> PathBuf {
    let new = base.join(LEAF);
    if new.is_dir() {
        return new;
    }
    LEGACY_LEAVES.iter().map(|leaf| base.join(leaf)).find(|p| p.is_dir()).unwrap_or(new)
}

/// Decide and, when it applies, perform the move inside `base`. `free` answers whether no other
/// Sigil holds the old folder; it is asked only when a move would happen. Returns the folder to
/// use for this run and what happened. Pure over paths apart from the one rename, so tests run it
/// on any OS against a temporary `base`.
pub fn migrate_in(base: &Path, free: &dyn Fn(&Path) -> bool) -> (PathBuf, Outcome) {
    let new = base.join(LEAF);
    let old = LEGACY_LEAVES.iter().map(|leaf| base.join(leaf)).find(|p| p.is_dir());
    // Case-insensitive file systems (Windows, macOS by default) see one folder for two names
    // that differ in case only; none of the old names do, but never rename a folder onto itself.
    let old = old.filter(|o| !same_dir(o, &new));
    match (new.is_dir(), old) {
        (true, None) => (new, Outcome::Current),
        (false, None) => (new, Outcome::Fresh),
        (true, Some(old)) => {
            let marker = new.join(NOTED);
            let first = !marker.exists();
            if first {
                let _ = std::fs::write(&marker, old.display().to_string());
            }
            (new, Outcome::BothKept { old, first })
        }
        (false, Some(old)) => {
            if !free(&old) {
                let error = "another Sigil is using it".to_string();
                return (old.clone(), Outcome::Deferred { old, error });
            }
            match rename_with_retry(&old, &new) {
                Ok(()) => (new, Outcome::Moved { from: old }),
                Err(error) => {
                    // A rename either happened or did not. If the new name appeared anyway (a
                    // second Sigil starting at the same moment won), use it.
                    if new.is_dir() && !old.exists() {
                        return (new, Outcome::Moved { from: old });
                    }
                    (old.clone(), Outcome::Deferred { old, error: error.to_string() })
                }
            }
        }
    }
}

/// Windows refuses a rename for a moment while a scanner or the indexer has a file open; a
/// few short tries cover that without holding the start up.
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(100));
        }
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("rename failed")))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether no Sigil holds the old folder's instance lock (macOS keeps `sigil.lock` locked while
/// it runs; on Windows and Linux the desktop app claims its single instance before it moves the
/// folder, and a file open inside makes the rename fail instead). A relaunch waits up to ten
/// seconds for the quitting process to let go.
fn wait_until_free(old: &Path, relaunch: bool) -> bool {
    let deadline = Instant::now() + if relaunch { RELAUNCH_WAIT } else { Duration::ZERO };
    loop {
        if lock_free(&old.join(INSTANCE_LOCK)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// True when `path` is missing or can be locked right now. The lock is let go at once.
fn lock_free(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let Ok(file) = OpenOptions::new().write(true).open(path).or_else(|_| File::open(path)) else {
        // A lock file nobody can open is not a running Sigil's.
        return true;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            true
        }
        Err(std::fs::TryLockError::WouldBlock) => false,
        Err(std::fs::TryLockError::Error(_)) => true,
    }
}

/// Map a stored absolute path inside an old data folder onto `data_dir`. Records written before
/// the move (the vanilla stash manifest, the trash, update staging) keep their old prefix on
/// disk; readers pass every stored path through this. Anything else comes back unchanged.
pub fn rebase_onto(path: &Path, data_dir: &Path) -> PathBuf {
    let Some(base) = data_dir.parent() else {
        return path.to_path_buf();
    };
    for leaf in LEGACY_LEAVES.iter().copied().chain(std::iter::once(LEAF)) {
        let root = base.join(leaf);
        if root == data_dir {
            continue;
        }
        if let Ok(rest) = path.strip_prefix(&root) {
            return data_dir.join(rest);
        }
    }
    path.to_path_buf()
}

/// [`rebase_onto`] for every string in a JSON record: a record from before the move keeps the
/// old prefix in any field that names a file inside the data folder. Strings that are not such a
/// path (names, URLs, game paths) are left alone. Returns whether anything changed.
pub fn rebase_json(value: &mut serde_json::Value, data_dir: &Path) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let path = Path::new(text.as_str());
            if !path.is_absolute() {
                return false;
            }
            let moved = rebase_onto(path, data_dir);
            if moved != path {
                *text = moved.to_string_lossy().into_owned();
                return true;
            }
            false
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().fold(false, |changed, item| rebase_json(item, data_dir) | changed)
        }
        serde_json::Value::Object(map) => {
            map.values_mut().fold(false, |changed, item| rebase_json(item, data_dir) | changed)
        }
        _ => false,
    }
}

/// The data folder a workspace belongs to: its parent when it is the default `<data>/workspace`.
/// A workspace placed elsewhere (`SIGIL_WORKSPACE_DIR`) has none, and nothing is rebased.
pub fn data_dir_of_workspace(workspace: &Path) -> Option<&Path> {
    (workspace.file_name()? == "workspace").then(|| workspace.parent()).flatten()
}

/// Parse a JSON record kept in the data folder, mapping old-folder paths onto `data_dir` first
/// ([`rebase_json`]).
pub fn parse_in_data<T: serde::de::DeserializeOwned>(text: &str, data_dir: &Path) -> serde_json::Result<T> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    rebase_json(&mut value, data_dir);
    serde_json::from_value(value)
}

/// Parse a JSON record that sits in the workspace, mapping old-folder paths onto the folder in
/// use first ([`rebase_json`]).
pub fn parse_in_workspace<T: serde::de::DeserializeOwned>(
    text: &str,
    workspace: &Path,
) -> serde_json::Result<T> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(data_dir) = data_dir_of_workspace(workspace) {
        rebase_json(&mut value, data_dir);
    }
    serde_json::from_value(value)
}

/// [`rebase_onto`] the data folder in use.
pub fn rebase(path: &Path) -> PathBuf {
    rebase_onto(path, &data_dir())
}

/// Undo the move before an update rolls back to a build older than the rename (0.1.42 and
/// earlier read only the old name): rename `data_dir` back to the old name when that is free.
/// Returns the old path when it renamed, for [`redo_after_failed_rollback`].
pub fn undo_for_older_build(data_dir: &Path) -> Option<PathBuf> {
    if data_dir.file_name()? != LEAF {
        return None;
    }
    let old = data_dir.parent()?.join(LEGACY_LEAVES[0]);
    if old.exists() || !data_dir.is_dir() {
        return None;
    }
    std::fs::rename(data_dir, &old).ok().map(|()| old)
}

/// Put the folder back under the new name when the rollback that moved it did not start the
/// older build after all.
pub fn redo_after_failed_rollback(old: &Path, data_dir: &Path) {
    if !data_dir.exists() {
        let _ = std::fs::rename(old, data_dir);
    }
}

#[cfg(test)]
#[path = "app_data_tests.rs"]
mod tests;
