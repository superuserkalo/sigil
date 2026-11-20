//! Payload path rules for a mod staging list.
//!
//! Ported from `packages/deadlock-tools/src/validate.ts`. The rules, messages,
//! issue order and stable issue codes must stay in step with the TypeScript
//! implementation. No file access: this checks an entry list before packing or
//! deploying.

use std::collections::HashMap;
use std::fmt;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadIssue {
    pub severity: Severity,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    pub message: String,
}

impl PayloadIssue {
    fn error(code: &'static str, entry: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            entry: Some(entry.into()),
            message: message.into(),
        }
    }
}

/// Settings > Mods > Allow script mods: gameplay data (`scripts/*.vdata_c`) and skeletons
/// (`*.vnmskel_c`) become warnings the install review shows instead of errors. Off by default.
/// Only installs (no staging folder) read it; the agent's staging checks stay strict.
#[cfg_attr(test, allow(dead_code))]
static ALLOW_SCRIPTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    static TEST_ALLOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_allow_scripts(allow: bool) {
    #[cfg(test)]
    TEST_ALLOW.with(|c| c.set(allow));
    #[cfg(not(test))]
    ALLOW_SCRIPTS.store(allow, std::sync::atomic::Ordering::SeqCst);
}

pub fn allow_scripts() -> bool {
    #[cfg(test)]
    return TEST_ALLOW.with(std::cell::Cell::get);
    #[cfg(not(test))]
    ALLOW_SCRIPTS.load(std::sync::atomic::Ordering::SeqCst)
}

/// What makes a file a script mod, in the sense of Settings > Allow script mods. The install
/// check and the safety scan (`mod_safety`) both read this, so they agree on what the setting
/// covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptModKind {
    /// `*.vnmskel_c`: replaces a skeleton, which can break hero animations.
    Skeleton,
    /// `scripts/**/*.vdata_c`: gameplay data, which changes gameplay and goes stale.
    GameplayData,
}

impl ScriptModKind {
    /// The install review's line for this file.
    pub fn message(self) -> &'static str {
        match self {
            ScriptModKind::Skeleton => "Script mod: replaces a skeleton, which can break hero animations.",
            ScriptModKind::GameplayData => "Script mod: changes gameplay data, which goes stale after game updates.",
        }
    }
}

/// Whether `entry` (a VPK path with forward slashes) makes a mod a script mod.
pub fn script_mod_kind(entry: &str) -> Option<ScriptModKind> {
    let lowered = entry.trim().to_lowercase();
    if lowered.ends_with(".vnmskel_c") {
        return Some(ScriptModKind::Skeleton);
    }
    if lowered.split('/').next() == Some("scripts") && lowered.ends_with(".vdata_c") {
        return Some(ScriptModKind::GameplayData);
    }
    None
}

#[derive(Debug, Clone, Default)]
pub struct PayloadOptions {
    /// Staging folder name that must not appear as the first path segment.
    pub staging_folder: Option<String>,
}

/// Check an entry list against the Deadlock payload rules.
///
/// The returned issues keep the order the TypeScript validator emits them:
/// one issue per rule for each entry, in entry order, with `empty_payload`
/// first when the list is empty.
pub fn validate_payload(entries: &[String], options: &PayloadOptions) -> Vec<PayloadIssue> {
    let mut issues = Vec::new();

    if entries.is_empty() {
        issues.push(PayloadIssue {
            severity: Severity::Error,
            code: "empty_payload",
            entry: None,
            message: "Payload is empty. A mod must contain at least one compiled file.".to_string(),
        });
    }

    let mut seen: HashMap<String, String> = HashMap::new();

    for raw in entries {
        let entry = raw.trim();

        if entry.is_empty() {
            issues.push(PayloadIssue::error(
                "empty_entry",
                raw.clone(),
                "Entry path is empty.",
            ));
            continue;
        }

        let mut rejected = false;

        if entry.contains('\\') {
            issues.push(PayloadIssue::error(
                "backslash_path",
                entry,
                "Use forward slashes in VPK paths.",
            ));
            rejected = true;
        }

        if entry.starts_with('/') || is_drive_absolute(entry) {
            issues.push(PayloadIssue::error(
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
            issues.push(PayloadIssue::error(
                "invalid_segment",
                entry,
                "Path has an empty, current or parent segment.",
            ));
            rejected = true;
        }

        if let Some(staging) = options.staging_folder.as_deref()
            && segments.first() == Some(&staging)
        {
            issues.push(PayloadIssue::error(
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
            issues.push(PayloadIssue::error(
                "tool_cache",
                entry,
                "Tool cache files are not part of a mod.",
            ));
            rejected = true;
        }

        let scripts_allowed = options.staging_folder.is_none() && allow_scripts();

        let script_mod = script_mod_kind(entry);

        if script_mod == Some(ScriptModKind::Skeleton) {
            if scripts_allowed {
                issues.push(PayloadIssue {
                    severity: Severity::Warning,
                    code: "skeleton",
                    entry: Some(entry.to_string()),
                    message: ScriptModKind::Skeleton.message().to_string(),
                });
            } else {
                issues.push(PayloadIssue::error(
                    "skeleton",
                    entry,
                    "Skeletons break hero animations and are not allowed in a mod.",
                ));
            }
            rejected = true;
        }

        if script_mod == Some(ScriptModKind::GameplayData) {
            if scripts_allowed {
                issues.push(PayloadIssue {
                    severity: Severity::Warning,
                    code: "gameplay_data",
                    entry: Some(entry.to_string()),
                    message: ScriptModKind::GameplayData.message().to_string(),
                });
            } else {
                issues.push(PayloadIssue::error(
                    "gameplay_data",
                    entry,
                    "Gameplay data is not cosmetic, goes stale after updates and is not allowed in a mod.",
                ));
            }
            rejected = true;
        }

        if EXECUTABLE_EXTENSIONS.contains(&extension) {
            issues.push(PayloadIssue::error(
                "executable",
                entry,
                "Executables are never allowed in a mod payload.",
            ));
            rejected = true;
        }

        if ARCHIVE_EXTENSIONS.contains(&extension) {
            issues.push(PayloadIssue::error(
                "archive",
                entry,
                "Archives belong outside the VPK, not inside it.",
            ));
            rejected = true;
        }

        if !lowered.ends_with("_c") && !rejected {
            if SOURCE_EXTENSIONS.contains(&extension) {
                issues.push(PayloadIssue::error(
                    "source_file",
                    entry,
                    "Source assets are ignored by the game and must not ship in the payload.",
                ));
            } else {
                issues.push(PayloadIssue {
                    severity: Severity::Warning,
                    code: "not_compiled",
                    entry: Some(entry.to_string()),
                    message: "Entry does not end in _c. The game reads compiled files only."
                        .to_string(),
                });
            }
        }

        let key = entry.to_lowercase();
        match seen.get(&key) {
            Some(previous) => issues.push(PayloadIssue::error(
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

pub fn has_errors(issues: &[PayloadIssue]) -> bool {
    issues.iter().any(|issue| issue.severity == Severity::Error)
}

pub fn format_payload_issues(issues: &[PayloadIssue]) -> String {
    if issues.is_empty() {
        return "Payload is valid. Every entry is a compiled file at a safe game path.".to_string();
    }

    issues
        .iter()
        .map(|issue| {
            let location = match &issue.entry {
                Some(entry) => format!("{entry}: "),
                None => String::new(),
            };
            format!(
                "{} {}: {}{}",
                issue.severity.as_str().to_uppercase(),
                issue.code,
                location,
                issue.message
            )
        })
        .collect::<Vec<String>>()
        .join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn reports_every_rule_in_typescript_order() {
        let entries = paths(&[
            "materials/hero.vmat_c",
            "",
            "bad\\path.vmat_c",
            "/absolute/file.vmdl_c",
            "models/hero.vnmskel_c",
            "scripts/hero.vdata_c",
            "staging/materials/hero.vmat_c",
            "citadel/tools_thumbnail_cache.bin",
            "models/hero.png",
            "tools/readme.txt",
            "bin/tool.exe",
            "docs/pack.zip",
            "materials/hero.vmat_c",
        ]);
        let options = PayloadOptions {
            staging_folder: Some("staging".to_string()),
        };

        let issues = validate_payload(&entries, &options);
        let actual: Vec<(&str, &str)> = issues
            .iter()
            .map(|issue| (issue.severity.as_str(), issue.code))
            .collect();

        assert_eq!(
            actual,
            vec![
                ("error", "empty_entry"),
                ("error", "backslash_path"),
                ("error", "absolute_path"),
                ("error", "invalid_segment"),
                ("error", "skeleton"),
                ("error", "gameplay_data"),
                ("error", "staging_folder_prefix"),
                ("error", "tool_cache"),
                ("error", "source_file"),
                ("warning", "not_compiled"),
                ("error", "executable"),
                ("error", "archive"),
                ("error", "duplicate_path"),
            ]
        );
        assert!(has_errors(&issues));
        assert_eq!(
            issues[0].entry.as_deref(),
            Some(""),
            "the empty entry keeps the raw path"
        );
        assert!(issues[12].message.contains("materials/hero.vmat_c"));

        let text = format_payload_issues(&issues);
        assert_eq!(text.lines().count(), issues.len());
        assert!(text.starts_with("ERROR empty_entry: : Entry path is empty."));
        assert!(text.contains(
            "WARNING not_compiled: tools/readme.txt: Entry does not end in _c. The game reads compiled files only."
        ));

        let empty = validate_payload(&[], &PayloadOptions::default());
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].code, "empty_payload");
        assert!(empty[0].entry.is_none());
        assert!(has_errors(&empty));

        let clean = validate_payload(&paths(&["materials/hero.vmat_c"]), &options);
        assert!(clean.is_empty());
        assert!(!has_errors(&clean));
        assert_eq!(
            format_payload_issues(&clean),
            "Payload is valid. Every entry is a compiled file at a safe game path."
        );
    }

    #[test]
    fn script_mods_turn_into_warnings_only_when_allowed() {
        let entries = vec!["scripts/heroes.vdata_c".to_string(), "models/a.vnmskel_c".to_string()];
        assert!(has_errors(&validate_payload(&entries, &PayloadOptions::default())));
        set_allow_scripts(true);
        let issues = validate_payload(&entries, &PayloadOptions::default());
        let staged = validate_payload(&entries, &PayloadOptions { staging_folder: Some("staging".into()) });
        set_allow_scripts(false);
        assert!(!has_errors(&issues), "{issues:?}");
        assert_eq!(issues.iter().filter(|i| i.severity == Severity::Warning).count(), 2);
        assert!(has_errors(&staged), "agent staging stays strict");
    }
}
