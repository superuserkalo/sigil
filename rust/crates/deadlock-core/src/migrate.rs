//! Path migration after a game update.
//!
//! Hero folders move between patches. A migration is a proposal until the user
//! approves it; applying one rewrites the project manifest and records the
//! change in project history. Nothing here touches the game install.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::game_catalog::GameCatalog;
use crate::project::{ModProject, ProjectHistoryEntry, write_project};
use crate::util::{basename, now_iso};
use crate::vpk::is_safe_vpk_path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Migration {
    pub from: String,
    pub to: String,
}

/// Propose a new path for every target the catalog no longer has.
///
/// A target is only proposed when its basename matches exactly one catalog
/// entry, so two heroes shipping `body.vmdl_c` never produce a guess.
pub fn plan_migrations(project: &ModProject, catalog: &GameCatalog) -> Vec<Migration> {
    let known: HashSet<String> = catalog
        .entries
        .iter()
        .map(|entry| entry.path.to_lowercase())
        .collect();

    let mut by_name: HashMap<String, Vec<&str>> = HashMap::new();
    for entry in &catalog.entries {
        by_name
            .entry(basename(&entry.path).to_lowercase())
            .or_default()
            .push(entry.path.as_str());
    }

    let mut migrations = Vec::new();
    let mut claimed: HashSet<String> = HashSet::new();

    for target in &project.targets {
        if known.contains(&target.to_lowercase()) {
            continue;
        }

        let name = basename(target).to_lowercase();
        let Some(candidates) = by_name.get(&name) else {
            continue;
        };

        let mut unique: Vec<&str> = Vec::new();
        for candidate in candidates {
            if !unique
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(candidate))
            {
                unique.push(candidate);
            }
        }

        if unique.len() != 1 {
            continue;
        }

        let candidate = unique[0];

        if claimed.contains(&candidate.to_lowercase()) {
            continue;
        }

        if project
            .targets
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(candidate))
        {
            continue;
        }

        claimed.insert(candidate.to_lowercase());
        migrations.push(Migration {
            from: target.clone(),
            to: candidate.to_string(),
        });
    }

    migrations
}

/// Apply approved migrations, update the timestamp and history, write
/// `project.json`. An empty migration list returns the project unchanged.
pub fn apply_migrations(
    workspace: &Path,
    project: &ModProject,
    migrations: &[Migration],
) -> Result<ModProject> {
    if migrations.is_empty() {
        return Ok(project.clone());
    }

    let mut replacements: HashMap<String, String> = HashMap::new();

    for migration in migrations {
        if migration.from.trim() != migration.from || migration.to.trim() != migration.to {
            return Err(Error::msg("Migration paths must not have leading or trailing whitespace."));
        }

        if !is_safe_vpk_path(&migration.from) || !is_safe_vpk_path(&migration.to) {
            return Err(Error::msg(format!(
                "Migration {:?} -> {:?} is not a safe VPK path.",
                migration.from, migration.to
            )));
        }

        if migration.from.eq_ignore_ascii_case(&migration.to) {
            return Err(Error::msg(format!(
                "Migration for {} does not change the path.",
                migration.from
            )));
        }

        if !project
            .targets
            .iter()
            .any(|target| target.eq_ignore_ascii_case(&migration.from))
        {
            return Err(Error::msg(format!(
                "Migration source {} is not a project target.",
                migration.from
            )));
        }

        if replacements
            .insert(migration.from.to_lowercase(), migration.to.clone())
            .is_some()
        {
            return Err(Error::msg(format!(
                "Migration source {} appears more than once.",
                migration.from
            )));
        }
    }

    let mut updated = project.clone();
    let mut targets = Vec::with_capacity(updated.targets.len());

    for target in &updated.targets {
        match replacements.get(&target.to_lowercase()) {
            Some(next) => targets.push(next.clone()),
            None => targets.push(target.clone()),
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    for target in &targets {
        if !seen.insert(target.to_lowercase()) {
            return Err(Error::msg(format!(
                "Migration would create a duplicate target {target}."
            )));
        }
    }

    // Keep the staged payload on the new game path too. A manifest-only move
    // would pack the old path on the next release.
    let staging = workspace.join("staging");
    for migration in migrations {
        let source = staging.join(&migration.from);
        let destination = staging.join(&migration.to);

        if !source.is_file() {
            continue;
        }

        if destination.exists() {
            return Err(Error::msg(format!(
                "Migration destination {} already exists in staging.",
                destination.display()
            )));
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::rename(&source, &destination)?;
    }

    updated.targets = targets;
    updated.updated_at = now_iso();
    updated
        .history
        .push(ProjectHistoryEntry::new(format!(
            "migrated {} path(s)",
            migrations.len()
        )));

    write_project(workspace, &updated)?;

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_catalog::{CatalogEntry, GameCatalog};
    use crate::project::{create_project, read_project};

    fn catalog_entry(path: &str) -> CatalogEntry {
        CatalogEntry {
            path: path.to_string(),
            crc32: 0,
            preload_bytes: 0,
            archive_index: 0,
            offset: 0,
            length: 0,
            vpk: "citadel".to_string(),
        }
    }

    fn catalog(paths: &[&str]) -> GameCatalog {
        GameCatalog {
            schema: 1,
            built_at: "2026-01-01T00:00:00.000Z".to_string(),
            sources: Vec::new(),
            entries: paths.iter().map(|path| catalog_entry(path)).collect(),
        }
    }

    #[test]
    fn migrates_one_moved_path_and_ignores_ambiguous_basenames() {
        let root = std::env::temp_dir().join(format!("dah-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let workspace = root.join("workspace");
        let mut project = create_project(&workspace, "Billy Recolor", "billy-recolor").unwrap();
        project.targets = vec![
            "models/heroes_wip/billy/billy.vmdl_c".to_string(),
            "sound/abilities/unique.vsnd_c".to_string(),
            "materials/unchanged.vmat_c".to_string(),
        ];
        project.updated_at = "2020-01-01T00:00:00.000Z".to_string();
        write_project(&workspace, &project).unwrap();

        let catalog = catalog(&[
            "models/heroes/billy/billy.vmdl_c",
            "materials/unchanged.vmat_c",
            "sound/hero_a/unique.vsnd_c",
            "sound/hero_b/unique.vsnd_c",
        ]);

        let migrations = plan_migrations(&project, &catalog);
        assert_eq!(
            migrations,
            vec![Migration {
                from: "models/heroes_wip/billy/billy.vmdl_c".to_string(),
                to: "models/heroes/billy/billy.vmdl_c".to_string(),
            }]
        );

        let updated = apply_migrations(&workspace, &project, &migrations).unwrap();
        assert_eq!(
            updated.targets,
            vec![
                "models/heroes/billy/billy.vmdl_c".to_string(),
                "sound/abilities/unique.vsnd_c".to_string(),
                "materials/unchanged.vmat_c".to_string(),
            ]
        );
        assert_ne!(updated.updated_at, project.updated_at);
        assert_eq!(
            updated.history.last().map(|entry| entry.action.as_str()),
            Some("migrated 1 path(s)")
        );

        let written = read_project(&workspace).unwrap();
        assert_eq!(written, updated);

        let _ = std::fs::remove_dir_all(&root);
    }
}
