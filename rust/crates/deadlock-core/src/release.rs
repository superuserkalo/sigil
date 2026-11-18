//! Versioned release manifests for packed build output.
//!
//! A release lives in `<workspace>/builds/<version>/` next to the packed VPK
//! and its chunk files. `release.json` records the slot, the packed file names
//! and the entry count so deploy and publish can read one file instead of
//! walking the workspace again.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

pub const RELEASE_SCHEMA: u32 = 1;
pub const RELEASE_MANIFEST_FILE: &str = "release.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub schema: u32,
    pub project_slug: String,
    pub project_name: String,
    pub version: String,
    pub slot: u16,
    pub created_at: String,
    /// Packed directory VPK name inside the release folder, for example `pak07_dir.vpk`.
    pub vpk_file: String,
    #[serde(default)]
    pub chunk_files: Vec<String>,
    pub entries: usize,
    pub total_bytes: u64,
    #[serde(default)]
    pub baseline_captured_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl ReleaseManifest {
    /// Build a manifest for a freshly packed release.
    pub fn new(
        project_slug: impl Into<String>,
        project_name: impl Into<String>,
        version: impl Into<String>,
        slot: u16,
        vpk_file: impl Into<String>,
    ) -> Self {
        Self {
            schema: RELEASE_SCHEMA,
            project_slug: project_slug.into(),
            project_name: project_name.into(),
            version: version.into(),
            slot,
            created_at: now_iso(),
            vpk_file: vpk_file.into(),
            chunk_files: Vec::new(),
            entries: 0,
            total_bytes: 0,
            baseline_captured_at: None,
            notes: None,
        }
    }

    /// Packed VPK path inside the release folder.
    pub fn vpk_path(&self, release_dir: &Path) -> PathBuf {
        release_dir.join(&self.vpk_file)
    }

    /// Packed chunk paths inside the release folder.
    pub fn chunk_paths(&self, release_dir: &Path) -> Vec<PathBuf> {
        self.chunk_files
            .iter()
            .map(|file| release_dir.join(file))
            .collect()
    }
}

pub fn builds_dir(workspace: &Path) -> PathBuf {
    workspace.join("builds")
}

/// Release folder for one version: `<workspace>/builds/<version>`.
pub fn release_dir(workspace: &Path, version: &str) -> PathBuf {
    builds_dir(workspace).join(version)
}

pub fn release_manifest_path(workspace: &Path, version: &str) -> PathBuf {
    release_dir(workspace, version).join(RELEASE_MANIFEST_FILE)
}

pub fn write_release_manifest(path: &Path, manifest: &ReleaseManifest) -> Result<()> {
    validate_release_manifest(manifest)?;

    let mut text = serde_json::to_string_pretty(manifest)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

pub fn read_release_manifest(path: &Path) -> Result<ReleaseManifest> {
    let text = fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "{} does not exist. Pack the release first.",
                path.display()
            ))
        } else {
            error.into()
        }
    })?;

    let manifest: ReleaseManifest = serde_json::from_str(&text)?;
    validate_release_manifest(&manifest)?;

    Ok(manifest)
}

/// Newest release in `<workspace>/builds`, comparing versions first and the
/// manifest mtime only when two folders share a version.
pub fn latest_release(workspace: &Path) -> Result<Option<ReleaseManifest>> {
    let builds = builds_dir(workspace);

    let entries = match fs::read_dir(&builds) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };

    let mut latest: Option<(ReleaseManifest, SystemTime)> = None;

    for entry in entries {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join(RELEASE_MANIFEST_FILE);
        if !manifest_path.is_file() {
            continue;
        }

        let manifest = read_release_manifest(&manifest_path)?;
        let mtime = fs::metadata(&manifest_path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let replace = match &latest {
            None => true,
            Some((current, current_mtime)) => {
                match compare_versions(&manifest.version, &current.version) {
                    Ordering::Greater => true,
                    Ordering::Less => false,
                    Ordering::Equal => mtime > *current_mtime,
                }
            }
        };

        if replace {
            latest = Some((manifest, mtime));
        }
    }

    Ok(latest.map(|(manifest, _)| manifest))
}

fn validate_release_manifest(manifest: &ReleaseManifest) -> Result<()> {
    if manifest.schema != RELEASE_SCHEMA {
        return Err(Error::msg(format!(
            "Release manifest uses unsupported schema {}.",
            manifest.schema
        )));
    }

    if manifest.project_slug.trim().is_empty() {
        return Err(Error::msg("Release manifest project slug must not be empty."));
    }

    if manifest.project_name.trim().is_empty() {
        return Err(Error::msg("Release manifest project name must not be empty."));
    }

    if manifest.version.trim().is_empty() {
        return Err(Error::msg("Release manifest version must not be empty."));
    }

    if !(1..=99).contains(&manifest.slot) {
        return Err(Error::msg(format!(
            "Release slot must be from 1 to 99, received {}.",
            manifest.slot
        )));
    }

    validate_release_file_name(&manifest.vpk_file)?;

    for chunk in &manifest.chunk_files {
        validate_release_file_name(chunk)?;
    }

    Ok(())
}

/// Release file names are plain file names in the release folder. Path
/// separators would escape the folder and a non VPK would break deploy.
pub(crate) fn validate_release_file_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::msg("Release file name must not be empty."));
    }

    if name.trim() != name {
        return Err(Error::msg(format!(
            "Release file name {name:?} has leading or trailing whitespace."
        )));
    }

    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(Error::msg(format!(
            "Release file name {name:?} must be a plain file name without a folder."
        )));
    }

    if name.chars().any(char::is_control) {
        return Err(Error::msg(format!(
            "Release file name {name:?} contains a control character."
        )));
    }

    if !name.to_ascii_lowercase().ends_with(".vpk") {
        return Err(Error::msg(format!(
            "Release file name {name:?} must end in .vpk."
        )));
    }

    Ok(())
}

/// Compare dotted versions numerically, with a leading `v` ignored. Segments
/// that are not numbers compare as lowercase text.
fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = left.trim().trim_start_matches(['v', 'V']);
    let right = right.trim().trim_start_matches(['v', 'V']);

    let left_parts: Vec<&str> = left.split('.').collect();
    let right_parts: Vec<&str> = right.split('.').collect();

    for (left_part, right_part) in left_parts.iter().zip(right_parts.iter()) {
        let ordering = match (left_part.parse::<u64>(), right_part.parse::<u64>()) {
            (Ok(left_number), Ok(right_number)) => left_number.cmp(&right_number),
            _ => left_part
                .to_ascii_lowercase()
                .cmp(&right_part.to_ascii_lowercase()),
        };

        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left_parts.len().cmp(&right_parts.len())
}
