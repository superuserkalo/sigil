//! `dah watch`: one read-only pass over every mod project in the workspace,
//! reported as a review queue after a game update.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::{self, Migration, ProjectCheck};
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{lossy, print_json};

pub struct WatchArgs {
    pub limit: usize,
    pub json: bool,
}

struct WatchedProject {
    path: PathBuf,
    slug: String,
    name: String,
    version: String,
    check: Option<ProjectCheck>,
    migrations: Vec<Migration>,
    error: Option<String>,
}

impl WatchedProject {
    fn counts(&self) -> (usize, usize, usize, usize) {
        match &self.check {
            Some(check) => (
                check.changed,
                check.missing,
                check.moved,
                check.unbaselined,
            ),
            None => (0, 0, 0, 0),
        }
    }

    fn needs_review(&self) -> bool {
        if self.error.is_some() || !self.migrations.is_empty() {
            return true;
        }

        let check = match &self.check {
            Some(check) => check,
            None => return true,
        };
        let (changed, missing, moved, unbaselined) = self.counts();

        changed > 0
            || missing > 0
            || moved > 0
            || unbaselined > 0
            || !check.gameinfo_ok
            || check.slot_conflict.is_some()
    }

    fn warnings(&self) -> Vec<String> {
        let check = match &self.check {
            Some(check) => check,
            None => return Vec::new(),
        };
        let mut warnings = Vec::new();

        if !check.gameinfo_ok {
            warnings.push(
                "gameinfo.gi has no `Game citadel/addons` SearchPaths line, mods will not load"
                    .to_string(),
            );
        }

        if let Some(conflict) = &check.slot_conflict
            && let Some(slot) = check.slot
        {
            let file = domain::slot_file_name(slot);
            warnings.push(if conflict.file_name == file {
                format!("slot {file} already exists in the addon folder")
            } else {
                format!("slot {file} is already used by {}", conflict.file_name)
            });
        }

        warnings
    }
}

pub fn run(args: &WatchArgs) -> Result<()> {
    let paths = paths::resolve_paths(None)?;
    let workspace = &paths.workspace_dir;
    let project_dirs = discover_projects(workspace);

    if project_dirs.is_empty() {
        return Err(CliError::new(
            "no_projects",
            format!(
                "No mod projects found under {} or {}. Run `dah mod new <slug>` first.",
                lossy(workspace),
                workspace.join("mods").display()
            ),
        ));
    }

    let status = domain::read_status()?;
    let install = domain::require_install(&status)?.clone();
    let loaded = domain::load_catalog_for(&install.game_root, workspace, false)?;
    let catalog = &loaded.catalog;

    let mut projects = Vec::with_capacity(project_dirs.len());

    for project_dir in &project_dirs {
        let project = match domain::read_project(project_dir) {
            Ok(project) => project,
            Err(error) => {
                projects.push(WatchedProject {
                    path: project_dir.clone(),
                    slug: project_name(project_dir),
                    name: String::new(),
                    version: String::new(),
                    check: None,
                    migrations: Vec::new(),
                    error: Some(error.to_string()),
                });
                continue;
            }
        };

        let migrations = domain::migration_plan(&project, catalog);
        let check = domain::check_project(project_dir, catalog, &status);
        let (check, error) = match check {
            Ok(check) => (Some(check), None),
            Err(error) => (None, Some(error.to_string())),
        };

        projects.push(WatchedProject {
            path: project_dir.clone(),
            slug: project.slug,
            name: project.name,
            version: project.version,
            check,
            migrations,
            error,
        });
    }

    let total = projects.len();
    let review = projects
        .iter()
        .filter(|project| project.needs_review())
        .count();
    let shown = projects.iter().take(args.limit).collect::<Vec<_>>();

    if args.json {
        print_json(&json!({
            "ok": true,
            "workspace": lossy(workspace),
            "gameRoot": lossy(&install.game_root),
            "total": total,
            "needsReview": review,
            "limit": args.limit,
            "truncated": args.limit < total,
            "projects": shown
                .iter()
                .map(|project| project_json(project))
                .collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!(
        "Watch: {} project(s) under {}",
        total,
        workspace.display()
    );
    println!("  Game:   {}", install.game_root.display());
    println!("  Review: {review} project(s) need attention");
    println!();

    for project in &shown {
        print_project(project);
    }

    if args.limit < total {
        println!(
            "Showing {} of {total} projects. Raise --limit to see the rest.",
            args.limit
        );
    }

    println!("Read-only pass: no project files were written.");

    Ok(())
}

fn print_project(project: &WatchedProject) {
    let flag = if project.needs_review() {
        " REVIEW"
    } else {
        ""
    };
    println!("{}{flag}", project.slug);

    if let Some(error) = &project.error {
        println!("  WARNING {error}");
        println!("  Path: {}", project.path.display());
        println!();
        return;
    }

    let (changed, missing, moved, unbaselined) = project.counts();
    println!("  version {}", project.version);
    println!(
        "  changed {changed}, missing {missing}, moved {moved}, unbaselined {unbaselined}"
    );

    for migration in &project.migrations {
        println!("  MOVE {} -> {}", migration.from, migration.to);
    }

    for warning in project.warnings() {
        println!("  WARNING {warning}");
    }

    println!();
}

fn project_json(project: &WatchedProject) -> Value {
    let check = project.check.as_ref();
    let (changed, missing, moved, unbaselined) = project.counts();

    json!({
        "path": lossy(&project.path),
        "slug": project.slug,
        "name": project.name,
        "version": project.version,
        "needsReview": project.needs_review(),
        "error": project.error,
        "counts": {
            "changed": changed,
            "missing": missing,
            "moved": moved,
            "unbaselined": unbaselined,
        },
        "migrations": project
            .migrations
            .iter()
            .map(|migration| json!({
                "from": migration.from,
                "to": migration.to,
            }))
            .collect::<Vec<_>>(),
        "gameinfo": check.map(|check| json!({
            "ok": check.gameinfo_ok,
            "present": check.gameinfo.as_ref().map(|gameinfo| gameinfo.present),
            "addonsSearchPath": check
                .gameinfo
                .as_ref()
                .map(|gameinfo| gameinfo.addons_search_path),
        })),
        "slot": check.map(|check| json!({
            "slot": check.slot,
            "file": check.slot.map(domain::slot_file_name),
            "conflict": check.slot_conflict.as_ref().map(|conflict| json!({
                "fileName": conflict.file_name,
                "sizeBytes": conflict.size_bytes,
            })),
        })),
        "warnings": project.warnings(),
    })
}

fn discover_projects(workspace: &Path) -> Vec<PathBuf> {
    let mut projects = Vec::new();

    if workspace.join("project.json").is_file() {
        projects.push(workspace.to_path_buf());
    }

    let mods = workspace.join("mods");
    if let Ok(entries) = std::fs::read_dir(&mods) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("project.json").is_file() {
                projects.push(path);
            }
        }
    }

    projects.sort();
    projects
}

fn project_name(project_dir: &Path) -> String {
    project_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string())
}
