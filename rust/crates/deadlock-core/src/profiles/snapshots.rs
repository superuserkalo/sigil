//! Snapshots: small recovery points of a mod setup, taken before a change that could go wrong.
//!
//! A snapshot is a JSON manifest of the mods that were on, with their slots, their GameBanana
//! ids and their fingerprints. It never holds VPK bytes, so one file is a few kilobytes and a
//! hundred of them cost nothing. Restoring a snapshot resolves it against the install the same
//! way a profile is applied: mods that are gone are named, never guessed.
//!
//! Snapshots live in `<workspace>/.dah/snapshots`, next to `profiles/`, both inside the app data
//! directory. They are written by [`capture`] (the "Snapshot now" button and the automatic
//! points) and read by the Profiles page. Nothing here touches a game install.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{CurrentSetup, Profile, ProfileSlot, PROFILE_SCHEMA};
use crate::crosshair::Crosshair;
use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

/// Schema written by this build.
pub const SNAPSHOT_SCHEMA: u32 = 1;
pub const SNAPSHOT_DIR_NAME: &str = "snapshots";
pub const SNAPSHOT_FILE_SUFFIX: &str = ".json";
const MAX_SNAPSHOTS_LISTED: usize = 500;

/// Why a recovery point was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotTrigger {
    /// Before a mod update replaced an installed file.
    PreUpdate,
    /// Before a profile apply switched mods around.
    PreApplyProfile,
    /// Before an import downloaded and installed mods.
    PreImport,
    /// The "Snapshot now" button.
    Manual,
}

impl SnapshotTrigger {
    /// A stable id for files and tests.
    pub fn id(self) -> &'static str {
        match self {
            SnapshotTrigger::PreUpdate => "pre-update",
            SnapshotTrigger::PreApplyProfile => "pre-apply",
            SnapshotTrigger::PreImport => "pre-import",
            SnapshotTrigger::Manual => "manual",
        }
    }

    pub fn from_id(id: &str) -> Option<SnapshotTrigger> {
        match id {
            "pre-update" => Some(SnapshotTrigger::PreUpdate),
            "pre-apply" => Some(SnapshotTrigger::PreApplyProfile),
            "pre-import" => Some(SnapshotTrigger::PreImport),
            "manual" => Some(SnapshotTrigger::Manual),
            _ => None,
        }
    }

    /// What the list says when the caller gave no label.
    pub fn default_label(self) -> &'static str {
        match self {
            SnapshotTrigger::PreUpdate => "Before a mod update",
            SnapshotTrigger::PreApplyProfile => "Before a profile switch",
            SnapshotTrigger::PreImport => "Before an import",
            SnapshotTrigger::Manual => "Snapshot",
        }
    }
}

/// One recovery point as stored on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema: u32,
    pub id: String,
    pub created_at: String,
    pub trigger: SnapshotTrigger,
    /// What this point was taken for, as shown in the list.
    pub label: String,
    pub slots: Vec<ProfileSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoexec: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosshair: Option<Crosshair>,
}

impl Snapshot {
    pub fn mod_count(&self) -> usize {
        self.slots.len()
    }

    /// The snapshot as an in-memory profile, so one resolver handles both.
    pub fn as_profile(&self) -> Profile {
        Profile {
            schema: PROFILE_SCHEMA,
            name: self.label.clone(),
            slug: "snapshot".to_string(),
            created_at: self.created_at.clone(),
            updated_at: self.created_at.clone(),
            slots: self.slots.clone(),
            crosshair: self.crosshair.clone(),
            autoexec: self.autoexec.clone(),
            author: None,
            extensions: Default::default(),
            isolated: false,
        }
    }
}

/// The list row: enough to draw a snapshot without loading its slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSummary {
    pub id: String,
    pub created_at: String,
    pub trigger: SnapshotTrigger,
    pub label: String,
    pub mods: usize,
}

pub fn snapshot_dir(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join(SNAPSHOT_DIR_NAME)
}

/// Take a recovery point of the setup on now. Workspace write only.
///
/// A missing game install is an error: there is nothing to record.
pub fn capture(
    game_root: &Path,
    workspace: &Path,
    trigger: SnapshotTrigger,
    label: Option<&str>,
) -> Result<Snapshot> {
    let current = super::read_current_setup(game_root, workspace)?;
    capture_setup(workspace, trigger, label, current)
}

/// A recovery point of a saved profile's own mods, for a change that overwrites the profile
/// before the install changes. Workspace write only.
pub fn capture_from_profile(
    workspace: &Path,
    trigger: SnapshotTrigger,
    label: Option<&str>,
    profile: &Profile,
) -> Result<Snapshot> {
    let created_at = now_iso();
    let label = label
        .map(|line| super::plain_text(line, super::MAX_HINT_CHARS))
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| trigger.default_label().to_string());
    let snapshot = Snapshot {
        schema: SNAPSHOT_SCHEMA,
        id: unique_id(workspace, &created_at, trigger)?,
        created_at,
        trigger,
        label,
        slots: profile.slots.clone(),
        autoexec: profile.autoexec.clone(),
        crosshair: profile.crosshair.clone(),
    };
    write(workspace, &snapshot)?;
    Ok(snapshot)
}

/// The same as [`capture`], from a setup already read by the caller.
pub(crate) fn capture_setup(
    workspace: &Path,
    trigger: SnapshotTrigger,
    label: Option<&str>,
    current: CurrentSetup,
) -> Result<Snapshot> {
    let created_at = now_iso();
    let label = label
        .map(|line| super::plain_text(line, super::MAX_HINT_CHARS))
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| trigger.default_label().to_string());
    let snapshot = Snapshot {
        schema: SNAPSHOT_SCHEMA,
        id: unique_id(workspace, &created_at, trigger)?,
        created_at,
        trigger,
        label,
        slots: current.slots,
        autoexec: current.autoexec,
        crosshair: current.crosshair,
    };
    write(workspace, &snapshot)?;
    Ok(snapshot)
}

fn unique_id(workspace: &Path, created_at: &str, trigger: SnapshotTrigger) -> Result<String> {
    let stamp: String = created_at
        .chars()
        .map(|character| match character {
            '0'..='9' | 'a'..='z' => character,
            'A'..='Z' => character.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    for counter in 1..1000 {
        let id = format!(
            "{}-{}-{}-{counter}",
            stamp,
            trigger.id(),
            std::process::id()
        );
        if !snapshot_path(workspace, &id).exists() {
            return Ok(id);
        }
    }
    Err(Error::msg("Could not name a new snapshot."))
}

pub fn snapshot_path(workspace: &Path, id: &str) -> PathBuf {
    snapshot_dir(workspace).join(format!("{id}{SNAPSHOT_FILE_SUFFIX}"))
}

fn validate_id(id: &str) -> Result<()> {
    let valid = (1..=160).contains(&id.len())
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if !valid {
        return Err(Error::msg(format!("Snapshot id {id:?} is not valid.")));
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Snapshot> {
    let text = fs::read_to_string(path)
        .map_err(|error| Error::msg(format!("Could not read {}: {error}", path.display())))?;
    let snapshot: Snapshot = serde_json::from_str(&text).map_err(|error| {
        Error::msg(format!("{} is not a snapshot: {error}", path.display()))
    })?;
    if snapshot.schema > SNAPSHOT_SCHEMA {
        return Err(Error::msg(format!(
            "Snapshot {:?} uses schema {}, which this build does not read.",
            snapshot.label, snapshot.schema
        )));
    }
    validate_id(&snapshot.id)?;
    if snapshot.slots.iter().any(|slot| !(1..=99).contains(&slot.slot)) {
        return Err(Error::msg(format!(
            "Snapshot {:?} records a slot outside 1 to 99.",
            snapshot.label
        )));
    }
    Ok(snapshot)
}

/// Write one snapshot under its id. The file is replaced atomically.
pub fn write(workspace: &Path, snapshot: &Snapshot) -> Result<()> {
    validate_id(&snapshot.id)?;
    let mut text = serde_json::to_string_pretty(snapshot)?;
    text.push('\n');
    write_atomic(&snapshot_path(workspace, &snapshot.id), text.as_bytes())
}

/// Every snapshot, newest first. A broken file is skipped: it must not hide the rest.
pub fn list(workspace: &Path) -> Vec<SnapshotSummary> {
    let Ok(entries) = fs::read_dir(snapshot_dir(workspace)) else {
        return Vec::new();
    };
    let mut out: Vec<SnapshotSummary> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some("json")
        })
        .filter_map(|path| read_file(&path).ok())
        .map(|snapshot| SnapshotSummary {
            id: snapshot.id,
            created_at: snapshot.created_at,
            trigger: snapshot.trigger,
            mods: snapshot.slots.len(),
            label: snapshot.label,
        })
        .collect();
    out.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    out.truncate(MAX_SNAPSHOTS_LISTED);
    out
}

pub fn read(workspace: &Path, id: &str) -> Result<Snapshot> {
    validate_id(id)?;
    read_file(&snapshot_path(workspace, id))
}

pub fn delete(workspace: &Path, id: &str) -> Result<()> {
    validate_id(id)?;
    let path = snapshot_path(workspace, id);
    fs::remove_file(&path)
        .map_err(|error| Error::msg(format!("Could not delete {}: {error}", path.display())))
}

/// Delete several snapshots. Returns how many went away; a missing one is not an error.
pub fn delete_many(workspace: &Path, ids: &[String]) -> Result<usize> {
    let mut removed = 0;
    for id in ids {
        validate_id(id)?;
        match fs::remove_file(snapshot_path(workspace, id)) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::msg(format!("Could not delete snapshot {id}: {error}")));
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::addons_dir;

    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "dah-snapshots-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(addons_dir(&root.join("game"))).unwrap();
            fs::create_dir_all(root.join("workspace")).unwrap();
            Scratch { root }
        }
        fn game(&self) -> PathBuf {
            self.root.join("game")
        }
        fn workspace(&self) -> PathBuf {
            self.root.join("workspace")
        }
        fn put(&self, name: &str, body: &[u8]) {
            fs::write(addons_dir(&self.game()).join(name), body).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn slot(n: u16, name: &str) -> ProfileSlot {
        ProfileSlot {
            slot: n,
            file: format!("pak{n:02}_dir.vpk"),
            enabled: true,
            name: Some(name.to_string()),
            gamebanana_id: Some(1000 + n as u64),
            gamebanana_file_id: Some(2000 + n as u64),
            fingerprint: Some(format!("q1:{n}")),
        }
    }

    #[test]
    fn capture_records_the_mods_on_without_any_vpk_bytes() {
        let scratch = Scratch::new("capture");
        scratch.put("pak01_dir.vpk", b"one");
        scratch.put("pak02_dir.vpk.disabled", b"two");
        let snapshot = capture(
            &scratch.game(),
            &scratch.workspace(),
            SnapshotTrigger::Manual,
            None,
        )
        .unwrap();
        assert_eq!(snapshot.slots.len(), 1);
        assert_eq!(snapshot.slots[0].slot, 1);
        assert_eq!(snapshot.label, "Snapshot");
        let path = snapshot_path(&scratch.workspace(), &snapshot.id);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"slots\""));
        assert!(!text.contains("one"));
        assert!(text.len() < 4096);
        let listed = list(&scratch.workspace());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].mods, 1);
    }

    #[test]
    fn list_is_newest_first_and_skips_a_broken_file() {
        let scratch = Scratch::new("list");
        scratch.put("pak01_dir.vpk", b"one");
        let first = capture(
            &scratch.game(),
            &scratch.workspace(),
            SnapshotTrigger::PreApplyProfile,
            Some("Before applying Ranked"),
        )
        .unwrap();
        let mut second = first.clone();
        second.id = "z-later".to_string();
        second.created_at = "2999-01-01T00:00:00.000Z".to_string();
        write(&scratch.workspace(), &second).unwrap();
        fs::write(snapshot_path(&scratch.workspace(), "broken"), "{not json").unwrap();
        let listed = list(&scratch.workspace());
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "z-later");
        assert_eq!(listed[1].label, "Before applying Ranked");
    }

    #[test]
    fn read_delete_and_bulk_delete_never_escape_the_folder() {
        let scratch = Scratch::new("delete");
        scratch.put("pak01_dir.vpk", b"one");
        let snapshot = capture(
            &scratch.game(),
            &scratch.workspace(),
            SnapshotTrigger::Manual,
            None,
        )
        .unwrap();
        assert!(read(&scratch.workspace(), &snapshot.id).is_ok());
        assert!(read(&scratch.workspace(), "../escape").is_err());
        assert!(delete(&scratch.workspace(), "../escape").is_err());
        let mut second = snapshot.clone();
        second.id = "second".to_string();
        write(&scratch.workspace(), &second).unwrap();
        assert_eq!(
            delete_many(&scratch.workspace(), &[snapshot.id.clone(), "second".into(), "gone".into()])
                .unwrap(),
            2
        );
        assert!(list(&scratch.workspace()).is_empty());
    }

    #[test]
    fn a_snapshot_reads_back_as_a_profile_for_the_resolver() {
        let scratch = Scratch::new("profile");
        scratch.put("pak01_dir.vpk", b"one");
        let snapshot = capture(
            &scratch.game(),
            &scratch.workspace(),
            SnapshotTrigger::Manual,
            Some("These mods"),
        )
        .unwrap();
        let profile = snapshot.as_profile();
        assert_eq!(profile.slots, snapshot.slots);
        assert_eq!(profile.name, "These mods");
        assert_eq!(profile.slug, "snapshot");
    }

    #[test]
    fn trigger_ids_round_trip() {
        for trigger in [
            SnapshotTrigger::PreUpdate,
            SnapshotTrigger::PreApplyProfile,
            SnapshotTrigger::PreImport,
            SnapshotTrigger::Manual,
        ] {
            assert_eq!(SnapshotTrigger::from_id(trigger.id()), Some(trigger));
        }
        assert_eq!(SnapshotTrigger::from_id("other"), None);
    }
}
