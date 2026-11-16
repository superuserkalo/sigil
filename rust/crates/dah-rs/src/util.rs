//! Small shared helpers: timestamps, sizes and argument validation.

use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;

use crate::error::{CliError, Result};

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let value = bytes as f64;

    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn print_json(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(error) => eprintln!("dah: could not serialize JSON output: {error}"),
    }
}

/// A short, stable string for display when no simpler value exists.
pub fn lossy(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Matches the core project rule: lowercase letters, digits and hyphens.
pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(CliError::new("invalid_slug", "Mod slug must not be empty."));
    }

    if slug.len() > 64 {
        return Err(CliError::new(
            "invalid_slug",
            "Mod slug must be 64 characters or fewer.",
        ));
    }

    let mut characters = slug.chars();
    let first = characters.next().unwrap_or(' ');

    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(CliError::new(
            "invalid_slug",
            "Mod slug must start with a lowercase letter or a digit.",
        ));
    }

    for character in characters {
        let allowed =
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-';
        if !allowed {
            return Err(CliError::new(
                "invalid_slug",
                format!(
                    "Mod slug contains {character:?}. Use lowercase letters, digits and hyphens."
                ),
            ));
        }
    }

    Ok(())
}

pub fn validate_version(version: &str) -> Result<()> {
    let trimmed = version.trim();

    if trimmed.is_empty() {
        return Err(CliError::new(
            "invalid_version",
            "Version must not be empty.",
        ));
    }

    if trimmed != version {
        return Err(CliError::new(
            "invalid_version",
            "Version must not have leading or trailing whitespace.",
        ));
    }

    let core = trimmed
        .split_once(['-', '+'])
        .map(|(core, _)| core)
        .unwrap_or(trimmed);
    let parts: Vec<&str> = core.split('.').collect();

    if parts.len() != 3 {
        return Err(CliError::new(
            "invalid_version",
            format!("Version {trimmed:?} must be three dot-separated numbers, for example 1.2.0."),
        ));
    }

    for part in parts {
        if part.is_empty() || !part.chars().all(|character| character.is_ascii_digit()) {
            return Err(CliError::new(
                "invalid_version",
                format!("Version {trimmed:?} must use numeric components."),
            ));
        }
    }

    Ok(())
}
