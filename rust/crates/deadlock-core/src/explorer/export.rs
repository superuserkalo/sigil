//! Export files from any source into a mod project: the original bytes into `staging/` (the
//! payload, so the mod replaces that exact game path) or an editable text copy into `sources/`.
//! The destination must be a mod project (a folder with `project.json`); a game install is
//! never one.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{Listing, extension, preview};
use crate::error::{Error, Result};
use crate::project::{ProjectHistoryEntry, read_project, write_project};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportMode {
    /// The file as it is, into `staging/<game path>`. Compiled files become project targets.
    Original,
    /// Decoded text into `sources/<game path>` with a source extension.
    Editable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub path: String,
    pub written: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub written: Vec<Exported>,
    /// Path and reason for each file left out.
    pub skipped: Vec<(String, String)>,
    /// Game paths newly added to the project's targets.
    pub new_targets: Vec<String>,
}

/// The editable name for a game path: `x.vdata_c` to `x.vdata`, `x.vxml_c` to `x.vxml`, other
/// compiled data to `x.<ext>.kv3`. `None` for files with no text form (textures, sounds).
pub fn editable_path(path: &str) -> Option<String> {
    let ext = extension(path);
    if !ext.ends_with("_c") {
        return Some(path.to_string());
    }
    if preview::NO_TEXT_COMPILED.contains(&ext.as_str()) {
        return None;
    }
    let stem = &path[..path.len() - 2];
    Some(match ext.as_str() {
        "vdata_c" | "vxml_c" | "vcss_c" | "vjs_c" | "vts_c" | "vsndevts_c" => stem.to_string(),
        _ => format!("{stem}.kv3"),
    })
}

/// The mod project a folder belongs to: the nearest ancestor with `project.json`.
pub fn project_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join("project.json").is_file())
        .map(Path::to_path_buf)
}

/// Export `paths` (files, or folders meaning everything below them) into a project.
pub fn export(
    listing: &Listing,
    paths: &[String],
    project: &Path,
    mode: ExportMode,
    overwrite: bool,
) -> Result<ExportReport> {
    if !project.join("project.json").is_file() {
        return Err(Error::msg(format!(
            "{} is not a mod project. Export writes into a project only.",
            project.display()
        )));
    }
    let base = project.join(match mode {
        ExportMode::Original => "staging",
        ExportMode::Editable => "sources",
    });
    let mut report = ExportReport::default();
    let mut files: Vec<String> = paths.iter().flat_map(|p| listing.files_under(p)).collect();
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(Error::msg("Nothing to export: pick a file or a folder."));
    }
    for path in files {
        if !crate::vpk::is_safe_vpk_path(&path) {
            report.skipped.push((path, "not a safe game path".to_string()));
            continue;
        }
        let (relative, bytes) = match mode {
            ExportMode::Original => match listing.read(&path) {
                Ok(bytes) => (path.clone(), bytes),
                Err(error) => {
                    report.skipped.push((path, error.to_string()));
                    continue;
                }
            },
            ExportMode::Editable => {
                let Some(relative) = editable_path(&path) else {
                    report.skipped.push((path, "has no text form; use Decompile in Toolchain".to_string()));
                    continue;
                };
                let bytes = match listing.read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        report.skipped.push((path, error.to_string()));
                        continue;
                    }
                };
                if !extension(&path).ends_with("_c") {
                    (relative, bytes)
                } else {
                    match preview::decoded_text(&path, &bytes) {
                        Some(decoded) => (relative, decoded.text.into_bytes()),
                        None => {
                            report.skipped.push((path, "did not decode to text".to_string()));
                            continue;
                        }
                    }
                }
            }
        };
        let target = base.join(&relative);
        if !target.starts_with(&base) {
            report.skipped.push((path, "leaves the project".to_string()));
            continue;
        }
        if target.exists() && !overwrite {
            report.skipped.push((path, "already in the project".to_string()));
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &bytes)?;
        report.written.push(Exported {
            path: path.clone(),
            written: target,
            bytes: bytes.len() as u64,
        });
        if mode == ExportMode::Original && path.ends_with("_c") {
            report.new_targets.push(path);
        }
    }
    if !report.new_targets.is_empty()
        && let Ok(mut manifest) = read_project(project)
    {
        let mut added = Vec::new();
        for target in &report.new_targets {
            if !manifest.targets.iter().any(|t| t.eq_ignore_ascii_case(target)) {
                manifest.targets.push(target.clone());
                added.push(target.clone());
            }
        }
        report.new_targets = added;
        if !report.new_targets.is_empty() {
            let mut entry = ProjectHistoryEntry::new("export");
            entry.note = Some(format!(
                "Exported {} file(s) from {}",
                report.new_targets.len(),
                listing.label
            ));
            manifest.history.push(entry);
            write_project(project, &manifest)?;
        }
    }
    Ok(report)
}
