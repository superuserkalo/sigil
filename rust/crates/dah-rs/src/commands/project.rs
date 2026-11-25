//! `dah mod ...`: create a project, record a baseline, check compatibility and
//! bump the version.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::domain::{self, TargetStatus};
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, now_rfc3339, print_json, validate_slug, validate_version};

pub struct NewArgs {
    pub slug: String,
    pub name: Option<String>,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub fn new_project(args: &NewArgs) -> Result<()> {
    validate_slug(&args.slug)?;

    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let name = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&args.slug)
        .to_string();
    let project_dir = workspace.join("mods").join(&args.slug);

    if project_dir.exists() {
        return Err(CliError::new(
            "project_exists",
            format!(
                "{} already exists. Pick another slug or remove it first.",
                project_dir.display()
            ),
        ));
    }

    let manifest = domain::create_project(&project_dir, &name, &args.slug)?;

    for directory in ["sources", "staging", "builds", "notes"] {
        std::fs::create_dir_all(project_dir.join(directory))?;
    }

    if args.json {
        print_json(&json!({
            "ok": true,
            "slug": manifest.slug,
            "name": manifest.name,
            "version": manifest.version,
            "path": project_dir.to_string_lossy(),
            "manifestPath": project_dir.join("project.json").to_string_lossy(),
        }));
        return Ok(());
    }

    println!(
        "Created mod project {} at {}",
        manifest.name,
        project_dir.display()
    );
    println!("  Manifest: {}", project_dir.join("project.json").display());
    println!("  Version:  {}", manifest.version);
    println!("Add target game paths to the manifest, then run `Sigil cli mod baseline`.");

    Ok(())
}

pub struct ProjectArgs {
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub fn baseline(args: &ProjectArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;

    if project.targets.is_empty() {
        return Err(CliError::new(
            "no_targets",
            format!(
                "Project {} has no target paths. Add the exact game paths it replaces to {} first.",
                project.slug,
                project_dir.join("project.json").display()
            ),
        ));
    }

    let load = domain::load_catalog(&project_dir, false)?;
    let baseline = domain::write_baseline(
        &project_dir,
        &project,
        &load.loaded.catalog,
        &load.status,
    )?;

    let baseline_paths: std::collections::BTreeSet<String> = baseline
        .entries
        .iter()
        .map(|entry| entry.path.to_lowercase())
        .collect();
    let not_found: Vec<&String> = project
        .targets
        .iter()
        .filter(|target| !baseline_paths.contains(&target.to_lowercase()))
        .collect();

    if args.json {
        print_json(&render_baseline_json(&project_dir, &baseline, &not_found, &load));
        return Ok(());
    }

    println!(
        "Baseline: {} of {} target(s) captured at {}",
        baseline.entries.len(),
        project.targets.len(),
        baseline.captured_at
    );
    println!("  Project: {}", project_dir.display());
    println!("  Game:    {}", load.install.game_root.display());

    for entry in &baseline.entries {
        println!(
            "  {}: {} (crc {:08x}, {})",
            entry.vpk,
            entry.path,
            entry.crc32,
            format_bytes(u64::from(entry.length) + u64::from(entry.preload_bytes))
        );
    }

    for target in &not_found {
        println!("WARNING target not found in the game files: {target}");
    }

    println!("Run `Sigil cli mod check` after a game update to see what moved.");

    Ok(())
}

pub fn check(args: &ProjectArgs) -> Result<bool> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;

    if project.targets.is_empty() {
        return Err(CliError::new(
            "no_targets",
            format!(
                "Project {} has no target paths to check. Add the exact game paths it replaces to {} first.",
                project.slug,
                project_dir.join("project.json").display()
            ),
        ));
    }

    let load = domain::load_catalog(&project_dir, false)?;
    let check = domain::check_project(&project_dir, &load.loaded.catalog, &load.status)?;

    if check.targets.is_empty() {
        return Err(CliError::new(
            "no_targets",
            format!(
                "Project {} has no target paths to check. Add the exact game paths it replaces to {} first.",
                check.project_slug,
                project_dir.join("project.json").display()
            ),
        ));
    }

    let ok = check.changed == 0
        && check.missing == 0
        && check.moved == 0
        && check.unbaselined == 0
        && check.gameinfo_ok
        && check.slot_conflict.is_none();

    if args.json {
        print_json(&render_check_json(&project_dir, &check, ok));
        return Ok(ok);
    }

    println!(
        "Project {}: {} unchanged, {} changed, {} missing, {} moved, {} unbaselined",
        check.project_slug,
        check.unchanged,
        check.changed,
        check.missing,
        check.moved,
        check.unbaselined
    );

    for target in &check.targets {
        match target.status {
            TargetStatus::Unchanged => println!("  OK          {}", target.path),
            TargetStatus::Changed => println!(
                "  CHANGED    {} (crc {} -> {})",
                target.path,
                hex(target.previous_crc32),
                hex(target.crc32)
            ),
            TargetStatus::Missing => println!("  MISSING    {}", target.path),
            TargetStatus::Moved => match &target.current_path {
                Some(current) => println!("  MOVED      {} -> {current}", target.path),
                None => println!("  MOVED      {}", target.path),
            },
            TargetStatus::Unbaselined => println!("  UNBASELINED {}", target.path),
        }
    }

    match &check.gameinfo {
        Some(gameinfo) if gameinfo.present && gameinfo.addons_search_path => {
            println!("gameinfo.gi: OK")
        }
        Some(gameinfo) if gameinfo.present => {
            println!("gameinfo.gi: WARNING addons SearchPaths line is missing")
        }
        Some(_) => println!("gameinfo.gi: WARNING the file is missing"),
        None => println!("gameinfo.gi: WARNING the file could not be read"),
    }

    if check.slot.is_some() && check.slot == check.deployed_slot && check.slot_conflict.is_none() {
        println!(
            "Slot: deployed to {} by this project",
            format!("pak{:02}_dir.vpk", check.slot.unwrap_or(0))
        );
    } else {
        match (check.slot, &check.slot_conflict) {
        (Some(slot), Some(conflict)) => {
            if conflict.file_name == format!("pak{slot:02}_dir.vpk") {
                println!(
                    "Slot: COLLISION pak{slot:02}_dir.vpk already exists ({})",
                    format_bytes(conflict.size_bytes)
                );
            } else {
                println!(
                    "Slot: COLLISION pak{slot:02}_dir.vpk is used by {} ({})",
                    conflict.file_name,
                    format_bytes(conflict.size_bytes)
                );
            }
        }
        (Some(slot), None) => println!("Slot: pak{slot:02}_dir.vpk is free"),
        (None, _) => println!("Slot: none assigned in the manifest"),
        }
    }

    if ok {
        println!("Project: compatible with the current game files.");
    } else {
        println!("Project: needs attention before packing.");
    }

    Ok(ok)
}

pub struct VersionArgs {
    pub version: String,
    pub note: Option<String>,
    pub workspace: Option<PathBuf>,
}

pub fn version(args: &VersionArgs) -> Result<()> {
    validate_version(&args.version)?;

    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let mut project = domain::read_project(&project_dir)?;

    if project.version == args.version {
        return Err(CliError::new(
            "same_version",
            format!(
                "Project {} is already at version {}. Pass a new version.",
                project.slug, args.version
            ),
        ));
    }

    let previous = project.version.clone();
    let note = args
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(str::to_string);
    let timestamp = now_rfc3339();

    project.version = args.version.clone();
    project.updated_at = timestamp.clone();
    project.history.push(domain::ProjectHistoryEntry {
        timestamp,
        action: format!("version {}", args.version),
        note: note.clone(),
    });

    domain::write_project(&project_dir, &project)?;

    println!(
        "Project {}: version {} -> {}",
        project.slug, previous, project.version
    );
    if let Some(note) = &note {
        println!("  Note: {note}");
    }
    println!("  Manifest: {}", project_dir.join("project.json").display());

    Ok(())
}

fn hex(value: Option<u32>) -> String {
    value
        .map(|value| format!("{value:08x}"))
        .unwrap_or_else(|| "????????".to_string())
}

fn render_baseline_json(
    project_dir: &std::path::Path,
    baseline: &domain::ProjectBaseline,
    not_found: &[&String],
    load: &domain::CatalogLoad,
) -> Value {
    json!({
        "ok": true,
        "project": project_dir.to_string_lossy(),
        "slug": baseline.project_slug,
        "capturedAt": baseline.captured_at,
        "gameRoot": load.install.game_root.to_string_lossy(),
        "slot": baseline.slot,
        "targets": baseline
            .entries
            .iter()
            .map(|entry| json!({
                "path": entry.path,
                "vpk": entry.vpk,
                "crc32": format!("{:08x}", entry.crc32),
                "length": entry.length,
                "preloadBytes": entry.preload_bytes,
            }))
            .collect::<Vec<_>>(),
        "notFound": not_found,
    })
}

fn render_check_json(
    project_dir: &std::path::Path,
    check: &domain::ProjectCheck,
    ok: bool,
) -> Value {
    json!({
        "ok": ok,
        "project": project_dir.to_string_lossy(),
        "slug": check.project_slug,
        "checkedAt": check.checked_at,
        "counts": {
            "unchanged": check.unchanged,
            "changed": check.changed,
            "missing": check.missing,
            "moved": check.moved,
            "unbaselined": check.unbaselined,
        },
        "targets": check
            .targets
            .iter()
            .map(|target| json!({
                "path": target.path,
                "status": target.status,
                "crc32": target.crc32.map(|value| format!("{value:08x}")),
                "previousCrc32": target.previous_crc32.map(|value| format!("{value:08x}")),
                "currentPath": target.current_path,
                "vpk": target.vpk,
            }))
            .collect::<Vec<_>>(),
        "gameinfo": {
            "ok": check.gameinfo_ok,
            "present": check.gameinfo.as_ref().map(|gameinfo| gameinfo.present),
            "addonsSearchPath": check
                .gameinfo
                .as_ref()
                .map(|gameinfo| gameinfo.addons_search_path),
            "line": check.gameinfo.as_ref().and_then(|gameinfo| gameinfo.line),
        },
        "slot": {
            "slot": check.slot,
            "file": check.slot.map(|slot| format!("pak{slot:02}_dir.vpk")),
            "conflict": check.slot_conflict.as_ref().map(|conflict| json!({
                "fileName": conflict.file_name,
                "sizeBytes": conflict.size_bytes,
            })),
            "nextFreeSlot": check.next_free_slot,
        },
    })
}
