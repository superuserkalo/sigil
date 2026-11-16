//! Pure Deadlock payload path rules, ported from
//! `packages/deadlock-tools/src/validate.ts` and the TypeScript host copy.
//! Keep the three in step until the Rust domain core owns the rule.
//!
//! No file access: this checks an entry list before packing or deploying.

use std::collections::HashMap;

use serde::Serialize;

const SOURCE_EXTENSIONS: [&str; 16] = [
    ".vmat",
    ".vmdl",
    ".vpcf",
    ".vsndevts",
    ".png",
    ".tga",
    ".psd",
    ".wav",
    ".mp3",
    ".dmx",
    ".xml",
    ".css",
    ".js",
    ".vxml",
    ".vcss",
    ".vjs",
];

const EXECUTABLE_EXTENSIONS: [&str; 13] = [
    ".exe", ".dll", ".bat", ".cmd", ".ps1", ".scr", ".sh", ".app", ".dylib", ".so", ".msi", ".vbs",
    ".com",
];

const ARCHIVE_EXTENSIONS: [&str; 5] = [".zip", ".7z", ".rar", ".tar", ".gz"];

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: &'static str,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    pub message: String,
}

impl Issue {
    fn error(code: &'static str, entry: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: "error",
            code,
            entry: Some(entry.into()),
            message: message.into(),
        }
    }
}

pub fn validate(entries: &[String], staging_folder: Option<&str>) -> Vec<Issue> {
    let mut issues = Vec::new();

    if entries.is_empty() {
        issues.push(Issue {
            severity: "error",
            code: "empty_payload",
            entry: None,
            message: "Payload is empty. A mod must contain at least one compiled file.".into(),
        });
    }

    let mut seen: HashMap<String, String> = HashMap::new();

    for raw in entries {
        let entry = raw.trim();

        if entry.is_empty() {
            issues.push(Issue::error(
                "empty_entry",
                raw.clone(),
                "Entry path is empty.",
            ));
            continue;
        }

        let mut rejected = false;

        if entry.contains('\\') {
            issues.push(Issue::error(
                "backslash_path",
                entry,
                "Use forward slashes in VPK paths.",
            ));
            rejected = true;
        }

        if entry.starts_with('/') || is_drive_absolute(entry) {
            issues.push(Issue::error(
                "absolute_path",
                entry,
                "VPK paths must be relative to the game root.",
            ));
            rejected = true;
        }

        let segments: Vec<&str> = entry.split('/').collect();
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
        {
            issues.push(Issue::error(
                "invalid_segment",
                entry,
                "Path has an empty, current or parent segment.",
            ));
            rejected = true;
        }

        if let Some(staging) = staging_folder
            && segments.first() == Some(&staging)
        {
            issues.push(Issue::error(
                "staging_folder_prefix",
                entry,
                format!(
                    "Path starts with the staging folder \"{staging}\". Paths must start at a game folder."
                ),
            ));
            rejected = true;
        }

        let name = segments.last().copied().unwrap_or(entry);
        let lowered = name.to_lowercase();
        let extension = extension_of(&lowered);

        if (lowered.starts_with("cache_") && lowered.ends_with(".soc"))
            || lowered == "tools_thumbnail_cache.bin"
        {
            issues.push(Issue::error(
                "tool_cache",
                entry,
                "Tool cache files are not part of a mod.",
            ));
            rejected = true;
        }

        if lowered.ends_with(".vnmskel_c") {
            issues.push(Issue::error(
                "skeleton",
                entry,
                "Skeletons break hero animations and are not allowed in a mod.",
            ));
            rejected = true;
        }

        if segments
            .first()
            .is_some_and(|segment| segment.eq_ignore_ascii_case("scripts"))
            && lowered.ends_with(".vdata_c")
        {
            issues.push(Issue::error(
                "gameplay_data",
                entry,
                "Gameplay data is not cosmetic, goes stale after updates and is not allowed in a mod.",
            ));
            rejected = true;
        }

        if EXECUTABLE_EXTENSIONS.contains(&extension) {
            issues.push(Issue::error(
                "executable",
                entry,
                "Executables are never allowed in a mod payload.",
            ));
            rejected = true;
        }

        if ARCHIVE_EXTENSIONS.contains(&extension) {
            issues.push(Issue::error(
                "archive",
                entry,
                "Archives belong outside the VPK, not inside it.",
            ));
            rejected = true;
        }

        if !lowered.ends_with("_c") && !rejected {
            if SOURCE_EXTENSIONS.contains(&extension) {
                issues.push(Issue::error(
                    "source_file",
                    entry,
                    "Source assets are ignored by the game and must not ship in the payload.",
                ));
            } else {
                issues.push(Issue {
                    severity: "warning",
                    code: "not_compiled",
                    entry: Some(entry.to_string()),
                    message: "Entry does not end in _c. The game reads compiled files only.".into(),
                });
            }
        }

        let key = entry.to_lowercase();
        match seen.get(&key) {
            Some(previous) => issues.push(Issue::error(
                "duplicate_path",
                entry,
                format!("Duplicate entry path, also listed as \"{previous}\"."),
            )),
            None => {
                seen.insert(key, entry.to_string());
            }
        }
    }

    issues
}

pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|issue| issue.severity == "error")
}

fn is_drive_absolute(entry: &str) -> bool {
    let bytes = entry.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn extension_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(index) => &name[index..],
        None => "",
    }
}
