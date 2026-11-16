//! Extract exact entries from a VPK into a workspace destination.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util::{absolute, write_atomic};
use crate::vpk::{crc32, is_safe_vpk_path, read_vpk_directory, read_vpk_entry_data};

#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    pub dir_vpk_path: PathBuf,
    pub entry_paths: Vec<String>,
    pub destination: PathBuf,
    /// Overwrite files that already exist. Default false.
    pub overwrite: bool,
    /// Verify the VPK CRC32 before writing. `None` means true.
    pub verify_crc: Option<bool>,
    /// Hard cap for one call. Default 500.
    pub max_files: Option<usize>,
}

impl ExtractOptions {
    pub fn new(dir_vpk_path: PathBuf, entry_paths: Vec<String>, destination: PathBuf) -> Self {
        Self {
            dir_vpk_path,
            entry_paths,
            destination,
            overwrite: false,
            verify_crc: None,
            max_files: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedFile {
    pub path: String,
    pub target: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractResult {
    pub destination: PathBuf,
    pub files: Vec<ExtractedFile>,
    pub skipped: Vec<ExtractIssue>,
    pub errors: Vec<ExtractIssue>,
    pub total_bytes: u64,
}

pub fn extract_from_vpk(options: ExtractOptions) -> Result<ExtractResult> {
    let max_files = options.max_files.unwrap_or(500);
    let overwrite = options.overwrite;
    let verify_crc = options.verify_crc.unwrap_or(true);
    let destination = absolute(&options.destination);

    let mut result = ExtractResult {
        destination: destination.clone(),
        files: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
        total_bytes: 0,
    };

    let mut seen = HashSet::new();
    let mut requested = Vec::new();

    for path in &options.entry_paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            requested.push(trimmed.to_string());
        }
    }

    if requested.is_empty() {
        result.errors.push(ExtractIssue {
            path: String::new(),
            message: "No entry paths were requested.".to_string(),
        });
        return Ok(result);
    }

    if requested.len() > max_files {
        result.errors.push(ExtractIssue {
            path: String::new(),
            message: format!(
                "Requested {} files but one call is limited to {}.",
                requested.len(),
                max_files
            ),
        });
        return Ok(result);
    }

    let directory = read_vpk_directory(&options.dir_vpk_path)?;

    for requested_path in requested {
        if !is_safe_vpk_path(&requested_path) {
            result.errors.push(ExtractIssue {
                path: requested_path,
                message: "Entry path is absolute, uses backslashes, or contains a parent segment."
                    .to_string(),
            });
            continue;
        }

        let Some(entry) = directory.by_path.get(&requested_path.to_lowercase()) else {
            result.errors.push(ExtractIssue {
                path: requested_path,
                message: "Path is not in the game index.".to_string(),
            });
            continue;
        };

        let Some(target) = safe_target(&destination, &entry.path) else {
            result.errors.push(ExtractIssue {
                path: requested_path,
                message: "Path resolves outside the destination.".to_string(),
            });
            continue;
        };

        if !overwrite && target.exists() {
            result.skipped.push(ExtractIssue {
                path: requested_path,
                message: "File already exists. Pass overwrite to replace it.".to_string(),
            });
            continue;
        }

        let data = match read_vpk_entry_data(&directory, entry) {
            Ok(data) => data,
            Err(error) => {
                result.errors.push(ExtractIssue {
                    path: requested_path,
                    message: error.to_string(),
                });
                continue;
            }
        };

        let actual_crc = crc32(&data);

        if verify_crc && actual_crc != entry.crc32 {
            result.errors.push(ExtractIssue {
                path: requested_path,
                message: format!(
                    "CRC32 mismatch: expected 0x{:x}, got 0x{:x}.",
                    entry.crc32, actual_crc
                ),
            });
            continue;
        }

        if let Err(error) = write_atomic(&target, &data) {
            result.errors.push(ExtractIssue {
                path: requested_path,
                message: error.to_string(),
            });
            continue;
        }

        result.total_bytes += data.len() as u64;
        result.files.push(ExtractedFile {
            path: entry.path.clone(),
            target,
            bytes: data.len() as u64,
        });
    }

    Ok(result)
}

pub fn format_extract_result(result: &ExtractResult) -> String {
    let mut lines = Vec::new();

    if result.files.is_empty() && result.errors.is_empty() {
        lines.push(format!(
            "Nothing extracted. {} file(s) already existed.",
            result.skipped.len()
        ));
        return lines.join("\n");
    }

    if !result.files.is_empty() {
        lines.push(format!(
            "Extracted {} file(s), {} bytes, into {}",
            result.files.len(),
            result.total_bytes,
            result.destination.display()
        ));

        for file in &result.files {
            lines.push(format!("  {} -> {}", file.path, file.target.display()));
        }
    }

    for skipped in &result.skipped {
        lines.push(format!("SKIPPED {}: {}", skipped.path, skipped.message));
    }

    for issue in &result.errors {
        if issue.path.is_empty() {
            lines.push(format!("ERROR {}", issue.message));
        } else {
            lines.push(format!("ERROR {}: {}", issue.path, issue.message));
        }
    }

    lines.join("\n")
}

fn safe_target(destination: &Path, entry_path: &str) -> Option<PathBuf> {
    let relative = Path::new(entry_path);

    if relative.is_absolute() {
        return None;
    }

    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return None;
        }
    }

    let target = destination.join(relative);

    if !target.starts_with(destination) {
        return None;
    }

    Some(target)
}
