//! The look inside an archive before it is installed: entry names from a ZIP's central
//! directory or a 7z header, or from an unpack with the install's own limits. Pure apart from
//! the readers.

use std::fs::{self, File};
use std::path::Path;

use chrono::Utc;

use crate::error::{Error, Result};
use crate::install::{self, ArchiveFormat, ExtractLimits, ToolPaths};

use super::{size, ScanReport, ScanWarning, EXECUTABLE, SCAN_MAX_BYTES, SHORTCUT};

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// Read an archive's file names and say which ones a player should look at.
///
/// Pure: the caller has already listed the entries. `bytes` is the uncompressed total when the
/// listing gave one.
pub fn scan_entries(files: &[String], bytes: u64) -> ScanReport {
    let mut warnings = Vec::new();
    let mut total = bytes;
    if total == 0 {
        total = 0;
    }
    for name in files {
        let lower = name.to_ascii_lowercase();
        let extension = lower.rsplit('.').next().unwrap_or("").to_string();
        if EXECUTABLE.contains(&extension.as_str()) {
            warnings.push(ScanWarning {
                path: name.clone(),
                message: format!("{name} is a program. A mod should only bring game files."),
            });
        } else if SHORTCUT.contains(&extension.as_str()) {
            warnings.push(ScanWarning {
                path: name.clone(),
                message: format!("{name} is a desktop shortcut or settings file."),
            });
        } else if lower.ends_with("gameinfo.gi") {
            warnings.push(ScanWarning {
                path: name.clone(),
                message: "The archive carries gameinfo.gi, which can change which folders the game reads."
                    .to_string(),
            });
        } else if lower.ends_with(".vpk") {
            let file = lower.rsplit('/').next().unwrap_or(&lower);
            let looks_like_pak = file.len() >= 7
                && file.starts_with("pak")
                && file.as_bytes().get(5) == Some(&b'_');
            if !looks_like_pak {
                warnings.push(ScanWarning {
                    path: name.clone(),
                    message: format!("{file} is not named pakNN_dir.vpk, so the game ignores it."),
                });
            }
        }
    }
    // A mod that ships thousands of files is unusual but not harmful; one huge file is worth
    // a mention because the download may not be what the player expected.
    ScanReport {
        checked: true,
        files: files.len(),
        bytes: total,
        warnings,
        error: None,
    }
}

/// List a ZIP's entries without unpacking it.
pub fn list_zip(archive: &Path) -> Result<(Vec<String>, u64)> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| Error::msg(format!("This is not a readable ZIP: {error}")))?;
    let mut files = Vec::with_capacity(zip.len());
    let mut bytes = 0;
    for index in 0..zip.len() {
        let entry = zip
            .by_index_raw(index)
            .map_err(|error| Error::msg(format!("ZIP entry {index} could not be read: {error}")))?;
        if entry.is_dir() {
            continue;
        }
        bytes += entry.size();
        files.push(entry.name().to_string());
    }
    Ok((files, bytes))
}

/// List a 7z's entries from its header without unpacking it (DMM 2.4.0 reads the file list the
/// same way). Fails on a header encrypted with a password or a damaged header; the caller then
/// falls back to the unpack, which reports the reason in the player's words.
pub fn list_7z(archive: &Path) -> Result<(Vec<String>, u64)> {
    let header = sevenz_rust2::Archive::open(archive)
        .map_err(|error| Error::msg(format!("The 7z header could not be read: {error}")))?;
    let mut files = Vec::with_capacity(header.files.len());
    let mut bytes = 0;
    for entry in &header.files {
        if entry.is_directory() {
            continue;
        }
        bytes += entry.size();
        files.push(entry.name().replace('\\', "/"));
    }
    Ok((files, bytes))
}

/// Look inside a downloaded archive. Never writes to the game install.
///
/// A ZIP is listed from its central directory and a `.7z` from its header, at any size. A
/// `.rar`, or a 7z whose header is encrypted or unreadable, is unpacked into
/// `scratch_root` with the install's own limits, read, and removed. A format that cannot be
/// read says so.
pub fn scan_archive(archive: &Path, scratch_root: &Path) -> ScanReport {
    let size_bytes = fs::metadata(archive).map(|meta| meta.len()).unwrap_or(0);
    let format = install::detect_archive_format(archive);
    match format {
        ArchiveFormat::Zip => match list_zip(archive) {
            Ok((files, bytes)) => scan_entries(&files, bytes),
            Err(error) => ScanReport {
                checked: false,
                error: Some(error.to_string()),
                ..ScanReport::default()
            },
        },
        ArchiveFormat::SevenZip | ArchiveFormat::Rar => {
            if format == ArchiveFormat::SevenZip {
                if let Ok((files, bytes)) = list_7z(archive) {
                    return scan_entries(&files, bytes);
                }
            }
            if size_bytes > SCAN_MAX_BYTES {
                return ScanReport {
                    checked: false,
                    error: Some(format!(
                        "The archive is {}, too large to unpack just to look inside.",
                        size(size_bytes)
                    )),
                    ..ScanReport::default()
                };
            }
            let destination = scratch_root.join(format!(
                "scan-{}-{}",
                std::process::id(),
                Utc::now().timestamp_millis()
            ));
            let limits = ExtractLimits::default();
            let tools = ToolPaths::from_environment();
            let result = install::extract_archive(archive, &destination, &limits, &tools);
            let report = match result {
                Ok(extract) => scan_entries(&extract.files, extract.bytes),
                Err(error) => ScanReport {
                    checked: false,
                    error: Some(error.to_string()),
                    ..ScanReport::default()
                },
            };
            let _ = fs::remove_dir_all(&destination);
            report
        }
        ArchiveFormat::Vpk | ArchiveFormat::Unknown => {
            // Not an archive at all: the install pipeline will say so. A single file still
            // gets a name check.
            let name = archive
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            scan_entries(&[name], size_bytes)
        }
    }
}


#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives").join(name)
    }

    #[test]
    fn a_7z_is_listed_from_its_header_without_a_scratch_folder() {
        let archive = fixture("solid.7z");
        let (files, bytes) = list_7z(&archive).expect("header lists");
        assert!(!files.is_empty());
        assert!(bytes > 0);
        let scratch = std::env::temp_dir().join(format!("sigil-scan-7z-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let report = scan_archive(&archive, &scratch);
        assert!(report.checked, "{:?}", report.error);
        assert_eq!(report.files, files.len());
        assert!(!scratch.exists(), "the header path never unpacks");
    }

    #[test]
    fn a_damaged_7z_header_is_an_error_not_an_empty_list() {
        assert!(list_7z(&fixture("solid.rar")).is_err());
    }
}
