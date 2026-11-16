//! `dah watch`: one read-only pass over every mod project in the workspace,
//! reported as a review queue after a game update.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::{self, Migration, PatchNote, ProjectCheck};
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{lossy, print_json};

pub struct WatchArgs {
    pub limit: usize,
    pub json: bool,
    pub refresh: bool,
    pub watch: bool,
    pub interval: u64,
}

struct WatchedProject {
    path: PathBuf,
    slug: String,
    name: String,
    version: String,
    check: Option<ProjectCheck>,
    migrations: Vec<Migration>,
    baseline_at: Option<String>,
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

    fn baseline_stale(&self, newest_patch: Option<&PatchNote>) -> bool {
        match (self.baseline_at.as_deref(), newest_patch) {
            (Some(baseline), Some(patch)) => baseline < patch.published_at.as_str(),
            _ => false,
        }
    }
}

pub fn run(args: &WatchArgs) -> Result<()> {
    if !args.watch {
        watch_once(args)?;
        return Ok(());
    }

    let interval = args.interval.max(5);
    loop {
        let summary = watch_once(args)?;
        write_watch_state(&summary, interval)?;
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

struct WatchSummary {
    total: usize,
    review: usize,
    newest_patch: Option<String>,
    game_root: String,
}

fn watch_once(args: &WatchArgs) -> Result<WatchSummary> {
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

    let patch_cache_path = domain::patch_note_cache_path(workspace);
    let mut patch_notes = domain::load_patch_note_cache(&patch_cache_path)
        .map(|cache| cache.notes)
        .unwrap_or_default();
    let mut patch_error = None;

    if args.refresh {
        match domain::fetch_patch_notes(20) {
            Ok(notes) => {
                if let Err(error) = domain::save_patch_note_cache(
                    &patch_cache_path,
                    &domain::cache_from_notes(notes.clone()),
                ) {
                    patch_error = Some(format!("Could not write the patch cache: {error}"));
                }
                patch_notes = notes;
            }
            Err(error) => patch_error = Some(format!("Could not fetch patch notes: {error}")),
        }
    }

    patch_notes.sort_by(|left, right| right.published_at.cmp(&left.published_at));
    let newest_patch = patch_notes.first().cloned();

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
                    baseline_at: None,
                    error: Some(error.to_string()),
                });
                continue;
            }
        };

        let migrations = domain::migration_plan(&project, catalog);
        let baseline_at = domain::read_baseline(project_dir)
            .ok()
            .map(|baseline| baseline.captured_at);
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
            baseline_at,
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
        let summary = WatchSummary {
            total,
            review,
            newest_patch: newest_patch.as_ref().map(|patch| patch.title.clone()),
            game_root: lossy(&install.game_root),
        };
        print_json(&json!({
            "ok": true,
            "workspace": lossy(workspace),
            "gameRoot": lossy(&install.game_root),
            "total": total,
            "needsReview": review,
            "limit": args.limit,
            "truncated": args.limit < total,
            "patches": {
                "newest": newest_patch.as_ref().map(|patch| json!({
                    "id": patch.id,
                    "title": patch.title,
                    "publishedAt": patch.published_at,
                })),
                "error": patch_error,
            },
            "projects": shown
                .iter()
                .map(|project| project_json(project, newest_patch.as_ref()))
                .collect::<Vec<_>>(),
        }));
        return Ok(summary);
    }

    println!(
        "Watch: {} project(s) under {}",
        total,
        workspace.display()
    );
    println!("  Game:   {}", install.game_root.display());
    if let Some(patch) = &newest_patch {
        println!(
            "  Patch:  {} ({})",
            patch.title,
            patch.published_at.chars().take(10).collect::<String>()
        );
    } else {
        println!("  Patch:  no cached patch notes (run `dah patches`)");
    }
    if let Some(error) = &patch_error {
        println!("  Patch:  WARNING {error}");
    }
    println!("  Review: {review} project(s) need attention");
    println!();

    for project in &shown {
        print_project(project, newest_patch.as_ref());
    }

    if args.limit < total {
        println!(
            "Showing {} of {total} projects. Raise --limit to see the rest.",
            args.limit
        );
    }

    println!("Read-only pass: no project files were written.");

    Ok(WatchSummary {
        total,
        review,
        newest_patch: newest_patch.as_ref().map(|patch| patch.title.clone()),
        game_root: lossy(&install.game_root),
    })
}

fn write_watch_state(summary: &WatchSummary, interval: u64) -> Result<()> {
    let paths = paths::resolve_paths(None)?;
    let path = paths.workspace_dir.join(".dah").join("watch-state.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let state = json!({
        "schema": 1,
        "updatedAt": crate::util::now_rfc3339(),
        "intervalSeconds": interval,
        "gameRoot": summary.game_root,
        "newestPatch": summary.newest_patch,
        "totalProjects": summary.total,
        "needsReview": summary.review,
    });
    let mut text = serde_json::to_string_pretty(&state)?;
    text.push('\n');
    std::fs::write(&path, text)?;
    eprintln!("[watch] state written to {}", path.display());
    Ok(())
}

fn print_project(project: &WatchedProject, newest_patch: Option<&PatchNote>) {
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

    if project.baseline_stale(newest_patch)
        && let Some(patch) = newest_patch
    {
        println!(
            "  WARNING baseline predates the newest patch ({} on {})",
            patch.title,
            patch.published_at.chars().take(10).collect::<String>()
        );
    }

    println!();
}

fn project_json(project: &WatchedProject, newest_patch: Option<&PatchNote>) -> Value {
    let check = project.check.as_ref();
    let (changed, missing, moved, unbaselined) = project.counts();

    json!({
        "path": lossy(&project.path),
        "slug": project.slug,
        "name": project.name,
        "version": project.version,
        "needsReview": project.needs_review(),
        "error": project.error,
        "baselineAt": project.baseline_at,
        "staleBaseline": project.baseline_stale(newest_patch),
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
