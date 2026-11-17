//! What a recent change left behind: the mod a crash most likely came from, and the overlap
//! a player asked Sigil to stop mentioning.
//!
//! Both records belong to the app, not the game install: `overlap-ignores.json` sits under the
//! app data directory, and the suspects are worked out from the install journal that
//! `crate::install` already writes. Nothing here writes to the game folder.
//!
//! The crash side is a heuristic, and the UI says so: the dump's own error text is matched
//! against the files each enabled mod ships, and when the dump names nothing, the newest
//! install from before the crash is named instead. A wrong guess only costs the player one
//! toggle, which is why the action offered next to it is "turn it off", never "delete it".

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::game_session::Dump;
use crate::install::read_journal;
use crate::util::write_atomic;

// ---------------------------------------------------------------------------
// Recent changes
// ---------------------------------------------------------------------------

/// One install the journal recorded, as the crash advisory reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub slot: u8,
    /// The mod name the plan carried (`sourceLabel`), or the file name without it.
    pub name: String,
    pub at: String,
    /// Unix seconds, for comparing against a dump's modified time.
    pub at_secs: i64,
    /// `pakNN_dir.vpk` the install wrote.
    pub file: String,
}

impl Change {
    /// `2 hours ago`, `yesterday`, `5 days ago`. English, matching the rest of the UI.
    pub fn ago(&self, now: i64) -> String {
        let seconds = (now - self.at_secs).max(0);
        match seconds {
            0..=90 => "just now".to_string(),
            91..=3_599 => format!("{} minutes ago", (seconds + 30) / 60),
            3_600..=86_399 => {
                let hours = (seconds + 1_800) / 3_600;
                format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
            }
            86_400..=172_800 => "yesterday".to_string(),
            _ => {
                let days = (seconds + 43_200) / 86_400;
                format!("{days} days ago")
            }
        }
    }
}

/// Journal installs newer than `within`, newest first. Read only.
pub fn recent_changes(workspace: &Path, within: Duration, limit: usize) -> Vec<Change> {
    let Ok(entries) = read_journal(workspace) else {
        return Vec::new();
    };
    let now = Utc::now().timestamp();
    let mut changes: Vec<Change> = entries
        .into_iter()
        .filter_map(|entry| {
            let at_secs = iso_seconds(&entry.installed_at)?;
            if now - at_secs > within.as_secs() as i64 {
                return None;
            }
            let name = if entry.source_label.trim().is_empty() {
                entry.file.clone()
            } else {
                entry.source_label.clone()
            };
            Some(Change {
                slot: entry.slot,
                name,
                at: entry.installed_at,
                at_secs,
                file: entry.file,
            })
        })
        .collect();
    changes.sort_by(|a, b| b.at_secs.cmp(&a.at_secs).then(a.slot.cmp(&b.slot)));
    changes.dedup_by(|a, b| a.slot == b.slot && a.at_secs == b.at_secs);
    changes.truncate(limit);
    changes
}

fn iso_seconds(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|value| value.timestamp())
}

/// Unix seconds of a dump's modified time.
pub fn dump_seconds(dump: &Dump) -> Option<i64> {
    dump.modified
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs() as i64)
}

/// The newest dump that a launch marker has not seen yet.
///
/// `started_secs` is when Sigil asked Steam to start the game; a dump written before that
/// belongs to an earlier session and is not an advisory.
pub fn unacknowledged_dump(
    dumps: &[Dump],
    started_secs: i64,
    acknowledged_secs: Option<i64>,
) -> Option<&Dump> {
    dumps
        .iter()
        .filter_map(|dump| dump_seconds(dump).map(|seconds| (seconds, dump)))
        .filter(|(seconds, _)| *seconds >= started_secs)
        .filter(|(seconds, _)| acknowledged_secs.is_none_or(|seen| *seconds > seen))
        .max_by_key(|(seconds, _)| *seconds)
        .map(|(_, dump)| dump)
}

// ---------------------------------------------------------------------------
// The likely culprit
// ---------------------------------------------------------------------------

/// The dump's facts, as the advisory shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpFacts {
    pub path: PathBuf,
    pub modified_secs: Option<i64>,
    /// Minidump exception code, when the header had one.
    pub exception: Option<u32>,
    /// The game's own error text from the dump comment stream.
    pub comment: Option<String>,
}

impl DumpFacts {
    pub fn from_dump(dump: &Dump) -> Self {
        DumpFacts {
            path: dump.path.clone(),
            modified_secs: dump_seconds(dump),
            exception: None,
            comment: None,
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Why a mod was named. Shown next to it so the player can judge the guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// The dump's error text names a file the mod ships.
    DumpNamesFile,
    /// The mod is the newest install from before the crash.
    NewestChange,
}

impl Reason {
    /// The line under the mod name. `file` is the shipped path when there is one.
    pub fn line(self, file: Option<&str>, change: Option<&Change>, now: i64) -> String {
        match (self, file, change) {
            (Reason::DumpNamesFile, Some(file), _) => {
                format!("The crash report names {file}, which this mod replaces.")
            }
            (Reason::DumpNamesFile, None, _) => {
                "The crash report names a file this mod replaces.".to_string()
            }
            (Reason::NewestChange, _, Some(change)) => {
                format!("Installed {} and not changed since.", change.ago(now))
            }
            (Reason::NewestChange, _, None) => "Installed shortly before the crash.".to_string(),
        }
    }
}

/// The mod the advisory names, and the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suspect {
    pub slot: u8,
    pub name: String,
    pub reason: Reason,
    /// The shipped path the dump named, when the match came from the dump.
    pub file: Option<String>,
    /// The change the guess came from, when it came from the journal.
    pub change: Option<Change>,
}

/// The advice for a dump: the mod to try turning off, or none when the evidence is too thin.
///
/// `names` maps a slot to the name the library has for it; `shipped` maps a slot file to the
/// game paths it replaces. Both cover enabled slots only, so the advice never names a mod that
/// is already off. Pure: all reading happens before the call.
pub fn suspect(
    facts: &DumpFacts,
    changes: &[Change],
    names: &BTreeMap<u8, String>,
    shipped: &BTreeMap<u8, Vec<String>>,
) -> Option<Suspect> {
    if let Some(comment) = facts.comment.as_deref()
        && let Some((slot, file)) = match_comment(comment, shipped)
    {
        return Some(Suspect {
            slot,
            name: name_for(names, slot),
            reason: Reason::DumpNamesFile,
            file: Some(file),
            change: changes.iter().find(|change| change.slot == slot).cloned(),
        });
    }
    let newest = changes.iter().filter(|change| names.contains_key(&change.slot)).max_by_key(|change| change.at_secs)?;
    Some(Suspect {
        slot: newest.slot,
        name: name_for(names, newest.slot),
        reason: Reason::NewestChange,
        file: None,
        change: Some(newest.clone()),
    })
}

fn name_for(names: &BTreeMap<u8, String>, slot: u8) -> String {
    names
        .get(&slot)
        .cloned()
        .unwrap_or_else(|| format!("slot {slot:02}"))
}

/// The lowest slot that ships a path the error text names.
///
/// The comment streams in a Deadlock dump read like `FATAL ERROR: panorama/layout/hud.xml
/// failed to load`, so a full path or its last segment is enough. Matching is case
/// insensitive and needs at least four characters, which keeps `hud` from matching everything.
fn match_comment(comment: &str, shipped: &BTreeMap<u8, Vec<String>>) -> Option<(u8, String)> {
    let text = comment.to_lowercase();
    for (slot, paths) in shipped {
        for path in paths {
            let lower = path.to_lowercase();
            if text.contains(&lower) {
                return Some((*slot, path.clone()));
            }
            if let Some(name) = lower.rsplit('/').next()
                && name.len() >= 4
                && text.contains(name)
            {
                return Some((*slot, path.clone()));
            }
        }
    }
    None
}

/// Group shipped game paths by slot, from `(slot file, paths)` pairs the caller read.
pub fn shipped_by_slot(pairs: impl IntoIterator<Item = (u8, Vec<String>)>) -> BTreeMap<u8, Vec<String>> {
    let mut out: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    for (slot, mut paths) in pairs {
        paths.sort();
        out.insert(slot, paths);
    }
    out
}

// ---------------------------------------------------------------------------
// Overlap ignores
// ---------------------------------------------------------------------------

/// Ignore list under the app data directory.
pub const IGNORES_NAME: &str = "overlap-ignores.json";

/// Schema version written to the ignore list.
pub const IGNORES_SCHEMA: u32 = 1;

/// Two slots whose overlap the player does not want to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnorePair {
    pub a: u8,
    pub b: u8,
}

impl IgnorePair {
    pub fn new(a: u8, b: u8) -> Self {
        if a <= b {
            IgnorePair { a, b }
        } else {
            IgnorePair { a: b, b: a }
        }
    }
}

/// One path a player does not want to be told about for one slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreFile {
    pub slot: u8,
    pub path: String,
}

/// The whole ignore list. Every entry is something the player turned off in Overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ignores {
    pub schema: u32,
    /// Sorted, one entry per pair.
    pub pairs: Vec<IgnorePair>,
    /// Sorted by slot then path.
    pub files: Vec<IgnoreFile>,
    /// Sorted slots whose overlaps are all ignored.
    pub mods: Vec<u8>,
}

impl Default for Ignores {
    fn default() -> Self {
        Ignores {
            schema: IGNORES_SCHEMA,
            pairs: Vec::new(),
            files: Vec::new(),
            mods: Vec::new(),
        }
    }
}

impl Ignores {
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty() && self.files.is_empty() && self.mods.is_empty()
    }

    /// Every entry, as one count for the restore list.
    pub fn len(&self) -> usize {
        self.pairs.len() + self.files.len() + self.mods.len()
    }

    pub fn pair_ignored(&self, a: u8, b: u8) -> bool {
        let wanted = IgnorePair::new(a, b);
        self.pairs.contains(&wanted)
    }

    pub fn mod_ignored(&self, slot: u8) -> bool {
        self.mods.contains(&slot)
    }

    pub fn file_ignored(&self, slot: u8, path: &str) -> bool {
        self.files
            .iter()
            .any(|entry| entry.slot == slot && entry.path.eq_ignore_ascii_case(path))
    }

    /// Whether an overlap should stay hidden: `slots` ships `path`, `winner` is the slot the
    /// game loads, so the others are the ones that lose.
    ///
    /// The row hides when every losing slot is ignored for this path, or the mod itself is
    /// ignored, or the pair with the winner is ignored. One remaining un-ignored loser keeps
    /// the row, which is why a new mod brings its overlaps back.
    pub fn hides(&self, path: &str, slots: &[u8], winner: u8) -> bool {
        let losers: Vec<u8> = slots.iter().copied().filter(|slot| *slot != winner).collect();
        if losers.is_empty() {
            return false;
        }
        losers.iter().all(|slot| {
            self.mod_ignored(*slot)
                || self.file_ignored(*slot, path)
                || self.pair_ignored(winner, *slot)
        })
    }

    /// Add a pair. True when the list changed.
    pub fn add_pair(&mut self, a: u8, b: u8) -> bool {
        if a == b {
            return false;
        }
        let pair = IgnorePair::new(a, b);
        if self.pairs.contains(&pair) {
            return false;
        }
        self.pairs.push(pair);
        self.pairs.sort();
        true
    }

    /// Add one path for one slot. True when the list changed.
    pub fn add_file(&mut self, slot: u8, path: &str) -> bool {
        let entry = IgnoreFile {
            slot,
            path: path.to_string(),
        };
        if self.files.contains(&entry) {
            return false;
        }
        self.files.push(entry);
        self.files.sort();
        true
    }

    /// Add one slot. True when the list changed.
    pub fn add_mod(&mut self, slot: u8) -> bool {
        if self.mods.contains(&slot) {
            return false;
        }
        self.mods.push(slot);
        self.mods.sort_unstable();
        true
    }

    /// Take one pair back off the list.
    pub fn remove_pair(&mut self, a: u8, b: u8) -> bool {
        let pair = IgnorePair::new(a, b);
        let before = self.pairs.len();
        self.pairs.retain(|entry| *entry != pair);
        self.pairs.len() != before
    }

    /// Take one file back off the list.
    pub fn remove_file(&mut self, slot: u8, path: &str) -> bool {
        let before = self.files.len();
        self.files
            .retain(|entry| !(entry.slot == slot && entry.path.eq_ignore_ascii_case(path)));
        self.files.len() != before
    }

    /// Turn one mod back on in the report.
    pub fn remove_mod(&mut self, slot: u8) -> bool {
        let before = self.mods.len();
        self.mods.retain(|entry| *entry != slot);
        self.mods.len() != before
    }

    /// Every slot the list mentions, for the restore list.
    pub fn slots(&self) -> BTreeSet<u8> {
        let mut slots: BTreeSet<u8> = self.mods.iter().copied().collect();
        for pair in &self.pairs {
            slots.insert(pair.a);
            slots.insert(pair.b);
        }
        for file in &self.files {
            slots.insert(file.slot);
        }
        slots
    }
}

/// Where the ignore list lives.
pub fn ignores_path(data_dir: &Path) -> PathBuf {
    data_dir.join(IGNORES_NAME)
}

/// Read the ignore list. A missing or unreadable list is empty, never an error the UI shows.
pub fn load_ignores(data_dir: &Path) -> Ignores {
    let Ok(text) = fs::read_to_string(ignores_path(data_dir)) else {
        return Ignores::default();
    };
    match serde_json::from_str::<Ignores>(&text) {
        Ok(list) if list.schema == IGNORES_SCHEMA => list,
        _ => Ignores::default(),
    }
}

/// Write the ignore list. App data write, atomic, no game folder involvement.
pub fn store_ignores(data_dir: &Path, list: &Ignores) -> Result<()> {
    let mut text = serde_json::to_string_pretty(list)?;
    text.push('\n');
    write_atomic(&ignores_path(data_dir), text.as_bytes())
}

/// Read, edit and write the list in one step. The edit returns `true` when it changed
/// something; an unchanged list is not written.
pub fn edit_ignores(
    data_dir: &Path,
    edit: impl FnOnce(&mut Ignores) -> bool,
) -> Result<Ignores> {
    let mut list = load_ignores(data_dir);
    if edit(&mut list) {
        if list.is_empty() {
            // Nothing left: remove the file rather than keep an empty document.
            match fs::remove_file(ignores_path(data_dir)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        } else {
            store_ignores(data_dir, &list)?;
        }
    }
    Ok(list)
}

/// A restore-list entry, already worded for the Overlap page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreEntry {
    pub label: String,
    /// Exactly what `restore` takes back.
    pub kind: RestoreKind,
}

/// What one restore entry removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreKind {
    Pair(u8, u8),
    File(u8, String),
    Mod(u8),
}

/// The ignore list as rows for the restore list, one line each.
pub fn restore_entries(list: &Ignores, names: &BTreeMap<u8, String>) -> Vec<RestoreEntry> {
    let name = |slot: u8| {
        names
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| format!("slot {slot:02}"))
    };
    let mut entries = Vec::new();
    for slot in &list.mods {
        entries.push(RestoreEntry {
            label: format!("{} is ignored", name(*slot)),
            kind: RestoreKind::Mod(*slot),
        });
    }
    for pair in &list.pairs {
        entries.push(RestoreEntry {
            label: format!("{} and {} together are ignored", name(pair.a), name(pair.b)),
            kind: RestoreKind::Pair(pair.a, pair.b),
        });
    }
    for file in &list.files {
        entries.push(RestoreEntry {
            label: format!("{} is ignored for {}", name(file.slot), file.path),
            kind: RestoreKind::File(file.slot, file.path.clone()),
        });
    }
    entries
}

/// Take one restore entry back off the list.
pub fn restore(data_dir: &Path, kind: &RestoreKind) -> Result<Ignores> {
    edit_ignores(data_dir, |list| match kind {
        RestoreKind::Pair(a, b) => list.remove_pair(*a, *b),
        RestoreKind::File(slot, path) => list.remove_file(*slot, path),
        RestoreKind::Mod(slot) => list.remove_mod(*slot),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/impact-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn names() -> BTreeMap<u8, String> {
        BTreeMap::from([(1, "Alpha".to_string()), (3, "Bravo".to_string())])
    }

    #[test]
    fn ignores_hide_a_row_only_when_every_loser_is_covered() {
        let mut list = Ignores::default();
        assert!(!list.hides("a/b.vmat_c", &[1, 3], 1));
        assert!(list.add_pair(1, 3));
        assert!(list.hides("a/b.vmat_c", &[1, 3], 1));
        // The pair is symmetric: an ignored pair hides the row from either side.
        assert!(list.hides("a/b.vmat_c", &[1, 3], 3));
        // A third, un-ignored loser keeps the row.
        assert!(!list.hides("a/b.vmat_c", &[1, 3, 5], 1));
        assert!(list.add_mod(5));
        assert!(list.hides("a/b.vmat_c", &[1, 3, 5], 1));
        assert_eq!(list.len(), 2);
        assert_eq!(list.slots(), BTreeSet::from([1, 3, 5]));

        // One file for one slot hides that path and nothing else.
        let mut files = Ignores::default();
        assert!(!files.hides("a/c.vmat_c", &[1, 3], 1));
        assert!(files.add_file(3, "a/c.vmat_c"));
        assert!(files.hides("a/c.vmat_c", &[1, 3], 1));
        assert!(files.hides("a/C.vmat_C", &[1, 3], 1), "paths compare without case");
        assert!(!files.hides("a/d.vmat_c", &[1, 3], 1));
        assert!(!files.hides("a/c.vmat_c", &[1, 3, 5], 1));
    }

    #[test]
    fn the_ignore_list_round_trips_and_restores_one_entry() {
        let dir = scratch("ignores");
        assert!(load_ignores(&dir).is_empty());
        let list = edit_ignores(&dir, |list| list.add_pair(3, 1)).unwrap();
        assert_eq!(list.pairs, vec![IgnorePair { a: 1, b: 3 }]);
        assert!(load_ignores(&dir).pair_ignored(1, 3));

        let list = edit_ignores(&dir, |list| list.add_mod(4)).unwrap();
        edit_ignores(&dir, |list| list.add_file(2, "materials/x.vmat_c")).unwrap();
        assert_eq!(list.len(), 2);

        let entries = restore_entries(&load_ignores(&dir), &names());
        assert_eq!(entries.len(), 3);
        assert!(entries[0].label.contains("Alpha") || entries[0].label.contains("slot 04"));

        let list = restore(&dir, &RestoreKind::Pair(1, 3)).unwrap();
        assert!(!list.pair_ignored(1, 3));
        assert_eq!(list.len(), 2);

        // Restoring everything removes the file itself.
        restore(&dir, &RestoreKind::Mod(4)).unwrap();
        restore(&dir, &RestoreKind::File(2, "materials/x.vmat_c".to_string())).unwrap();
        assert!(load_ignores(&dir).is_empty());
        assert!(!ignores_path(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dump_that_names_a_shipped_file_points_at_that_slot() {
        let shipped = shipped_by_slot([
            (1, vec!["materials/heroes/billy/body.vmat_c".to_string()]),
            (3, vec!["panorama/layout/hud.xml".to_string()]),
        ]);
        let facts = DumpFacts {
            path: PathBuf::from("/dumps/deadlock_1.mdmp"),
            modified_secs: Some(1_000),
            exception: Some(0xC000_0005),
            comment: Some("FATAL ERROR: panorama/layout/hud.xml failed to load".to_string()),
        };
        let found = suspect(&facts, &[], &names(), &shipped).unwrap();
        assert_eq!(found.slot, 3);
        assert_eq!(found.name, "Bravo");
        assert_eq!(found.reason, Reason::DumpNamesFile);
        assert_eq!(found.file.as_deref(), Some("panorama/layout/hud.xml"));
        assert!(found.reason.line(found.file.as_deref(), None, 0).contains("hud.xml"));
    }

    #[test]
    fn without_a_named_file_the_newest_change_is_the_guess() {
        let facts = DumpFacts {
            path: PathBuf::from("/dumps/x.mdmp"),
            modified_secs: Some(1_000),
            exception: None,
            comment: Some("Engine error".to_string()),
        };
        let changes = vec![
            Change {
                slot: 1,
                name: "Alpha".into(),
                at: String::new(),
                at_secs: 900,
                file: "pak01_dir.vpk".into(),
            },
            Change {
                slot: 3,
                name: "Bravo".into(),
                at: String::new(),
                at_secs: 950,
                file: "pak03_dir.vpk".into(),
            },
            Change {
                slot: 7,
                name: "Gone".into(),
                at: String::new(),
                at_secs: 980,
                file: "pak07_dir.vpk".into(),
            },
        ];
        let found = suspect(&facts, &changes, &names(), &BTreeMap::new()).unwrap();
        assert_eq!(found.slot, 3, "slot 07 is not installed any more");
        assert_eq!(found.reason, Reason::NewestChange);
        assert!(found.reason.line(None, found.change.as_ref(), 1_000).contains("just now"));
        assert!(suspect(&facts, &[], &names(), &BTreeMap::new()).is_none());
    }

    #[test]
    fn only_a_dump_from_the_last_launch_is_unacknowledged() {
        let dumps = vec![
            Dump {
                path: PathBuf::from("old.mdmp"),
                bytes: 1,
                modified: Some(std::time::UNIX_EPOCH + Duration::from_secs(500)),
                source: "game",
            },
            Dump {
                path: PathBuf::from("new.mdmp"),
                bytes: 1,
                modified: Some(std::time::UNIX_EPOCH + Duration::from_secs(1_500)),
                source: "game",
            },
        ];
        assert_eq!(
            unacknowledged_dump(&dumps, 1_000, None).map(|d| d.path.clone()),
            Some(PathBuf::from("new.mdmp"))
        );
        assert!(unacknowledged_dump(&dumps, 1_000, Some(1_500)).is_none());
        assert!(unacknowledged_dump(&dumps, 2_000, None).is_none());
    }

    #[test]
    fn ages_are_worded_for_the_advisory() {
        let change = Change {
            slot: 1,
            name: "Alpha".into(),
            at: String::new(),
            at_secs: 0,
            file: String::new(),
        };
        assert_eq!(change.ago(30), "just now");
        assert_eq!(change.ago(3_600), "1 hour ago");
        assert_eq!(change.ago(7_200), "2 hours ago");
        assert_eq!(change.ago(90_000), "yesterday");
        assert_eq!(change.ago(4 * 86_400), "4 days ago");
    }

    #[test]
    fn recent_changes_read_the_journal_newest_first() {
        let dir = scratch("journal");
        let now = Utc::now().timestamp();
        let entries = [
            (1u8, "Alpha", now - 7_200),
            (3, "Bravo", now - 60),
            (5, "Old", now - 30 * 86_400),
        ];
        let mut text = String::new();
        for (slot, name, at) in entries {
            let entry = serde_json::json!({
                "schema": 1,
                "planId": format!("p{slot}"),
                "installedAt": chrono::DateTime::from_timestamp(at, 0).unwrap().to_rfc3339(),
                "source": "gamebanana",
                "sourceLabel": name,
                "downloadUrl": null,
                "archive": "/tmp/a.zip",
                "archiveSha256": "x",
                "slot": slot,
                "file": format!("pak{slot:02}_dir.vpk"),
                "target": "/tmp/pak.vpk",
                "chunkFiles": [],
                "entries": 1,
                "bytes": 1,
                "vpkBytes": 1,
                "vpkSha256": "x",
                "backups": [],
                "removedChunks": [],
                "removedFiles": [],
                "gameinfoRepaired": false,
                "warnings": []
            });
            text.push_str(&serde_json::to_string(&entry).unwrap());
            text.push('\n');
        }
        let path = crate::install::journal_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();

        let changes = recent_changes(&dir, Duration::from_secs(7 * 86_400), 10);
        assert_eq!(changes.iter().map(|c| c.slot).collect::<Vec<_>>(), vec![3, 1]);
        assert_eq!(changes[0].name, "Bravo");
        assert!(recent_changes(&Path::new("/nonexistent"), Duration::from_secs(60), 10).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
