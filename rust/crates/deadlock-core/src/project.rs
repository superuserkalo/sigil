//! Mod project manifests, baselines and change impact checks.
//!
//! A project lives in a normal workspace directory. The manifest is
//! `<workspace>/project.json`; the captured path baseline is
//! `<workspace>/.dah/baseline.json`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::discovery::{AddonSlot, DEADLOCK_APP_ID, GameInfoStatus, GameStatus};
use crate::error::{Error, Result};
use crate::game_catalog::GameCatalog;
use crate::util::{basename, now_iso, write_atomic};
use crate::vpk::is_safe_vpk_path;

pub const PROJECT_SCHEMA: u32 = 1;
pub const BASELINE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHistoryEntry {
    pub timestamp: String,
    pub action: String,
    pub note: Option<String>,
}

impl ProjectHistoryEntry {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            timestamp: now_iso(),
            action: action.into(),
            note: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModProject {
    pub schema: u32,
    pub slug: String,
    pub name: String,
    pub version: String,
    pub created_at: String,
    pub updated_at: String,
    pub game_app_id: String,
    /// Exact game paths the mod intends to replace.
    pub targets: Vec<String>,
    pub slot: Option<u8>,
    pub references: Vec<String>,
    pub history: Vec<ProjectHistoryEntry>,
}

impl ModProject {
    pub fn new(name: impl Into<String>, slug: impl Into<String>) -> Self {
        let timestamp = now_iso();

        Self {
            schema: PROJECT_SCHEMA,
            slug: slug.into(),
            name: name.into(),
            version: "0.1.0".to_string(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            game_app_id: DEADLOCK_APP_ID.to_string(),
            targets: Vec::new(),
            slot: None,
            references: Vec::new(),
            history: vec![ProjectHistoryEntry::new("created")],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineEntry {
    pub path: String,
    pub crc32: u32,
    pub vpk: String,
    pub length: u32,
    pub preload_bytes: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBaseline {
    pub schema: u32,
    pub project_slug: String,
    pub captured_at: String,
    pub game_app_id: String,
    pub slot: Option<u8>,
    pub gameinfo: Option<GameInfoStatus>,
    pub entries: Vec<BaselineEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetStatus {
    Unchanged,
    Changed,
    Missing,
    Moved,
    Unbaselined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TargetCheck {
    pub path: String,
    pub status: TargetStatus,
    pub crc32: Option<u32>,
    pub previous_crc32: Option<u32>,
    /// Current path when the target moved.
    pub current_path: Option<String>,
    pub vpk: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCheck {
    pub schema: u32,
    pub project_slug: String,
    pub checked_at: String,
    pub targets: Vec<TargetCheck>,
    pub unchanged: usize,
    pub changed: usize,
    pub missing: usize,
    pub moved: usize,
    pub unbaselined: usize,
    pub gameinfo: Option<GameInfoStatus>,
    pub gameinfo_ok: bool,
    pub slot: Option<u8>,
    /// Installed addon occupying the project's preferred slot, when any.
    pub slot_conflict: Option<AddonSlot>,
    pub next_free_slot: Option<u8>,
}

pub fn project_path(workspace: &Path) -> PathBuf {
    workspace.join("project.json")
}

pub fn baseline_path(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join("baseline.json")
}

pub fn create_project(workspace: &Path, name: &str, slug: &str) -> Result<ModProject> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::msg("Project name must not be empty."));
    }

    let slug = if slug.trim().is_empty() {
        slugify(name)
    } else {
        slug.trim().to_string()
    };

    if slug.is_empty() {
        return Err(Error::msg(
            "Project slug must contain at least one letter or digit.",
        ));
    }

    validate_slug(&slug)?;

    fs::create_dir_all(workspace)?;
    let path = project_path(workspace);

    if path.exists() {
        return Err(Error::msg(format!(
            "{} already exists. Read it instead of creating over it.",
            path.display()
        )));
    }

    let project = ModProject::new(name, slug);
    write_project(workspace, &project)?;

    Ok(project)
}

pub fn read_project(workspace: &Path) -> Result<ModProject> {
    let path = project_path(workspace);
    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "{} does not exist. Create the project first.",
                path.display()
            ))
        } else {
            error.into()
        }
    })?;

    let project: ModProject = serde_json::from_str(&text)?;
    validate_project(&project)?;

    Ok(project)
}

pub fn write_project(workspace: &Path, project: &ModProject) -> Result<()> {
    validate_project(project)?;

    let mut text = serde_json::to_string_pretty(project)?;
    text.push('\n');
    write_atomic(&project_path(workspace), text.as_bytes())
}

pub fn write_baseline(
    workspace: &Path,
    project: &ModProject,
    catalog: &GameCatalog,
    status: &GameStatus,
) -> Result<ProjectBaseline> {
    validate_project(project)?;

    let by_path: HashMap<String, &crate::game_catalog::CatalogEntry> = catalog
        .entries
        .iter()
        .map(|entry| (entry.path.to_lowercase(), entry))
        .collect();

    let mut entries = Vec::new();

    for target in &project.targets {
        if let Some(entry) = by_path.get(&target.to_lowercase()) {
            entries.push(BaselineEntry {
                path: entry.path.clone(),
                crc32: entry.crc32,
                vpk: entry.vpk.clone(),
                length: entry.length,
                preload_bytes: entry.preload_bytes,
            });
        }
    }

    let baseline = ProjectBaseline {
        schema: BASELINE_SCHEMA,
        project_slug: project.slug.clone(),
        captured_at: now_iso(),
        game_app_id: project.game_app_id.clone(),
        slot: project.slot,
        gameinfo: status.gameinfo.clone(),
        entries,
    };

    let mut text = serde_json::to_string_pretty(&baseline)?;
    text.push('\n');
    write_atomic(&baseline_path(workspace), text.as_bytes())?;

    Ok(baseline)
}

pub fn check_project(
    workspace: &Path,
    catalog: &GameCatalog,
    status: &GameStatus,
) -> Result<ProjectCheck> {
    let project = read_project(workspace)?;
    let baseline = read_baseline(workspace)?;

    if baseline.project_slug != project.slug {
        return Err(Error::msg(format!(
            "Baseline belongs to project {} but the workspace project is {}.",
            baseline.project_slug, project.slug
        )));
    }

    let baseline_by_path: HashMap<String, &BaselineEntry> = baseline
        .entries
        .iter()
        .map(|entry| (entry.path.to_lowercase(), entry))
        .collect();
    let catalog_by_path: HashMap<String, &crate::game_catalog::CatalogEntry> = catalog
        .entries
        .iter()
        .map(|entry| (entry.path.to_lowercase(), entry))
        .collect();
    let catalog_by_name: HashMap<String, Vec<&crate::game_catalog::CatalogEntry>> = {
        let mut map: HashMap<String, Vec<&crate::game_catalog::CatalogEntry>> = HashMap::new();
        for entry in &catalog.entries {
            map.entry(basename(&entry.path).to_lowercase())
                .or_default()
                .push(entry);
        }
        map
    };

    let mut targets = Vec::new();
    let mut counts = [0_usize; 5];

    for target in &project.targets {
        let key = target.to_lowercase();
        let baseline_entry = baseline_by_path.get(&key);
        let current = catalog_by_path.get(&key);

        let check = match (baseline_entry, current) {
            (None, _) => TargetCheck {
                path: target.clone(),
                status: TargetStatus::Unbaselined,
                crc32: current.map(|entry| entry.crc32),
                previous_crc32: None,
                current_path: current.map(|entry| entry.path.clone()),
                vpk: current.map(|entry| entry.vpk.clone()),
            },
            (Some(previous), Some(entry)) => {
                let changed = previous.crc32 != entry.crc32
                    || previous.length != entry.length
                    || previous.preload_bytes != entry.preload_bytes;

                TargetCheck {
                    path: target.clone(),
                    status: if changed {
                        TargetStatus::Changed
                    } else {
                        TargetStatus::Unchanged
                    },
                    crc32: Some(entry.crc32),
                    previous_crc32: Some(previous.crc32),
                    current_path: Some(entry.path.clone()),
                    vpk: Some(entry.vpk.clone()),
                }
            }
            (Some(previous), None) => {
                let moved = find_moved(&previous.path, &catalog_by_name);

                match moved {
                    Some(entry) => TargetCheck {
                        path: target.clone(),
                        status: TargetStatus::Moved,
                        crc32: Some(entry.crc32),
                        previous_crc32: Some(previous.crc32),
                        current_path: Some(entry.path.clone()),
                        vpk: Some(entry.vpk.clone()),
                    },
                    None => TargetCheck {
                        path: target.clone(),
                        status: TargetStatus::Missing,
                        crc32: None,
                        previous_crc32: Some(previous.crc32),
                        current_path: None,
                        vpk: None,
                    },
                }
            }
        };

        counts[match check.status {
            TargetStatus::Unchanged => 0,
            TargetStatus::Changed => 1,
            TargetStatus::Missing => 2,
            TargetStatus::Moved => 3,
            TargetStatus::Unbaselined => 4,
        }] += 1;

        targets.push(check);
    }

    let slot_conflict = project.slot.and_then(|slot| {
        status
            .addons
            .as_ref()
            .and_then(|addons| addons.slots.iter().find(|used| used.slot == slot).cloned())
    });

    let gameinfo = status.gameinfo.clone();
    let gameinfo_ok = gameinfo
        .as_ref()
        .map(|value| value.present && value.addons_search_path)
        .unwrap_or(false);

    Ok(ProjectCheck {
        schema: 1,
        project_slug: project.slug.clone(),
        checked_at: now_iso(),
        targets,
        unchanged: counts[0],
        changed: counts[1],
        missing: counts[2],
        moved: counts[3],
        unbaselined: counts[4],
        gameinfo,
        gameinfo_ok,
        slot: project.slot,
        slot_conflict,
        next_free_slot: status
            .addons
            .as_ref()
            .and_then(|addons| addons.next_free_slot),
    })
}

pub fn format_project_check(check: &ProjectCheck) -> String {
    let mut lines = vec![format!(
        "Project {}: {} unchanged, {} changed, {} missing, {} moved, {} unbaselined",
        check.project_slug,
        check.unchanged,
        check.changed,
        check.missing,
        check.moved,
        check.unbaselined
    )];

    for target in &check.targets {
        let status = match target.status {
            TargetStatus::Unchanged => "ok",
            TargetStatus::Changed => "CHANGED",
            TargetStatus::Missing => "MISSING",
            TargetStatus::Moved => "MOVED",
            TargetStatus::Unbaselined => "UNBASELINED",
        };

        match (&target.status, &target.current_path) {
            (TargetStatus::Moved, Some(path)) => {
                lines.push(format!("{status} {} -> {path}", target.path));
            }
            _ => lines.push(format!("{status} {}", target.path)),
        }
    }

    lines.push(match &check.gameinfo {
        Some(gameinfo) if gameinfo.present && gameinfo.addons_search_path => {
            "gameinfo.gi: addons SearchPaths present".to_string()
        }
        Some(gameinfo) if gameinfo.present => "gameinfo.gi: addons SearchPaths missing".to_string(),
        _ => "gameinfo.gi: missing".to_string(),
    });

    if let Some(slot) = check.slot {
        match &check.slot_conflict {
            Some(conflict) => lines.push(format!(
                "Slot {} is already used by {}.",
                slot, conflict.file_name
            )),
            None => lines.push(format!("Slot {slot:02} is free for this project.")),
        }
    }

    lines.join("\n")
}

pub fn read_baseline(workspace: &Path) -> Result<ProjectBaseline> {
    let path = baseline_path(workspace);
    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "{} does not exist. Capture a baseline first.",
                path.display()
            ))
        } else {
            error.into()
        }
    })?;

    let baseline: ProjectBaseline = serde_json::from_str(&text)?;

    if baseline.schema != BASELINE_SCHEMA {
        return Err(Error::msg(format!(
            "Baseline uses unsupported schema {}.",
            baseline.schema
        )));
    }

    Ok(baseline)
}

fn find_moved<'a>(
    previous_path: &str,
    catalog_by_name: &'a HashMap<String, Vec<&crate::game_catalog::CatalogEntry>>,
) -> Option<&'a crate::game_catalog::CatalogEntry> {
    let name = basename(previous_path).to_lowercase();
    let candidates = catalog_by_name.get(&name)?;

    if candidates.len() != 1 {
        return None;
    }

    let candidate = candidates[0];
    (candidate.path.to_lowercase() != previous_path.to_lowercase()).then_some(candidate)
}

fn validate_project(project: &ModProject) -> Result<()> {
    if project.schema != PROJECT_SCHEMA {
        return Err(Error::msg(format!(
            "Project uses unsupported schema {}.",
            project.schema
        )));
    }

    if project.name.trim().is_empty() {
        return Err(Error::msg("Project name must not be empty."));
    }

    validate_slug(&project.slug)?;

    if project.game_app_id != DEADLOCK_APP_ID {
        return Err(Error::msg(format!(
            "Project targets app {} but this build only supports {DEADLOCK_APP_ID}.",
            project.game_app_id
        )));
    }

    if let Some(slot) = project.slot {
        if !(1..=99).contains(&slot) {
            return Err(Error::msg(format!(
                "Project slot must be from 1 to 99, received {slot}."
            )));
        }
    }

    for target in &project.targets {
        if target.trim() != target {
            return Err(Error::msg(format!(
                "Target path {target:?} has leading or trailing whitespace."
            )));
        }

        if !is_safe_vpk_path(target) {
            return Err(Error::msg(format!(
                "Target path {target:?} is absolute, uses backslashes, or contains a parent segment."
            )));
        }
    }

    Ok(())
}

fn validate_slug(slug: &str) -> Result<()> {
    let pattern = Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("slug pattern is valid");

    if slug.len() > 64 {
        return Err(Error::msg("Project slug must be 64 characters or fewer."));
    }

    if !pattern.is_match(slug) {
        return Err(Error::msg(format!(
            "Project slug {slug:?} must use lowercase letters, digits and hyphens."
        )));
    }

    Ok(())
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('-');
            last_was_separator = true;
        }
    }

    slug.trim_matches('-').to_string()
}
