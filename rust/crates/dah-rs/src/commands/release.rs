//! `dah mod release` and `dah mod deploy`: freeze a validated pack into
//! `builds/<version>/release.json` and install that release when approved.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::{self, DeployReport, ProjectHistoryEntry, ReleaseManifest};
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, lossy, now_rfc3339, print_json};

pub struct ReleaseArgs {
    pub workspace: Option<PathBuf>,
    pub slot: Option<u8>,
    pub note: Option<String>,
    pub json: bool,
}

pub struct DeployArgs {
    pub workspace: Option<PathBuf>,
    pub slot: Option<u8>,
    pub approve: bool,
    pub json: bool,
}

pub fn release(args: &ReleaseArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let mut project = domain::read_project(&project_dir)?;
    let slot = domain::resolve_slot(args.slot, &project)?;

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

    let staging = domain::staging_dir(&project_dir);
    if !staging.is_dir() {
        return Err(CliError::new(
            "no_staging",
            format!(
                "{} does not exist. Compile the mod sources into the staging folder first.",
                staging.display()
            ),
        ));
    }

    let entries = domain::staging_entries(&staging)?;
    let issues = domain::validate_staging_entries(&entries);

    if domain::has_errors(&issues) {
        return Err(CliError::new(
            "payload_invalid",
            format!(
                "Payload validation failed, nothing was released.\n{}",
                domain::format_payload_issues(&issues)
            ),
        ));
    }

    let warnings = domain::payload_warnings(&issues);

    let baseline_path = domain::baseline_path(&project_dir);
    if !baseline_path.is_file() {
        return Err(CliError::new(
            "no_baseline",
            format!(
                "{} does not exist. Run `dah mod baseline` before releasing so the release records which game files it was built against.",
                baseline_path.display()
            ),
        ));
    }
    let baseline = domain::read_baseline(&project_dir)?;

    let release_dir = domain::release_directory(&project_dir, &project.version);
    if directory_has_entries(&release_dir)? {
        return Err(CliError::new(
            "release_exists",
            format!(
                "{} already holds a release for version {}. Bump the version with `dah mod version <VERSION>` or remove that folder first.",
                release_dir.display(),
                project.version
            ),
        ));
    }

    std::fs::create_dir_all(&release_dir).map_err(|error| {
        CliError::new(
            "release_dir",
            format!("Could not create {}: {error}", release_dir.display()),
        )
    })?;

    let packed = domain::pack_staging(&staging, &release_dir, slot)?;
    let note = args
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(str::to_string);
    let manifest = domain::build_release_manifest(
        &project,
        slot,
        &packed,
        &baseline.captured_at,
        note.clone(),
    );

    // The folder was empty and created by this run, so a failed manifest write
    // must not leave a release that looks complete but has no metadata.
    if let Err(error) = domain::write_manifest(&release_dir, &manifest) {
        let _ = std::fs::remove_dir_all(&release_dir);
        return Err(error);
    }

    project.updated_at = now_rfc3339();
    project.history.push(ProjectHistoryEntry {
        timestamp: project.updated_at.clone(),
        action: format!("release {}", project.version),
        note: note.clone(),
    });
    domain::write_project(&project_dir, &project)?;

    let manifest_path = domain::release_manifest_file(&release_dir);

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "slug": project.slug,
            "version": project.version,
            "slot": slot,
            "releaseDir": lossy(&release_dir),
            "manifest": lossy(&manifest_path),
            "vpk": lossy(&packed.dir_path),
            "chunks": packed.chunk_paths.iter().map(|path| lossy(path)).collect::<Vec<_>>(),
            "entries": packed.entry_count,
            "totalBytes": packed.total_bytes,
            "baselineCapturedAt": manifest.baseline_captured_at,
            "note": note,
            "warnings": warnings,
        }));
        return Ok(());
    }

    println!(
        "Released {} {} as {}",
        project.name,
        project.version,
        manifest.vpk_file
    );
    println!("  Release:  {}", release_dir.display());
    println!("  Manifest: {}", manifest_path.display());
    println!(
        "  Entries:  {} ({})",
        packed.entry_count,
        format_bytes(packed.total_bytes)
    );
    println!(
        "  Baseline: {}",
        manifest.baseline_captured_at.as_deref().unwrap_or("unknown")
    );
    if let Some(note) = &note {
        println!("  Note:     {note}");
    }
    for warning in &warnings {
        println!("WARNING {warning}");
    }
    println!("  History:  project.json records release {}", project.version);
    println!("Next: `dah mod deploy` to plan an install for the game.");

    Ok(())
}

pub fn deploy(args: &DeployArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let mut project = domain::read_project(&project_dir)?;

    let manifest = domain::latest_release_manifest(&project_dir)?.ok_or_else(|| {
        CliError::new(
            "no_release",
            format!(
                "Project {} has no release yet. Run `dah mod release` first.",
                project.slug
            ),
        )
    })?;

    let release_dir = domain::release_directory(&project_dir, &manifest.version);
    if !release_dir.is_dir() {
        return Err(CliError::new(
            "release_incomplete",
            format!(
                "Release {} is recorded but {} is missing. Run `dah mod release` again.",
                manifest.version,
                release_dir.display()
            ),
        ));
    }

    let vpk_path = release_dir.join(&manifest.vpk_file);
    if !vpk_path.is_file() {
        return Err(CliError::new(
            "release_incomplete",
            format!(
                "Release {} is missing its VPK at {}. Run `dah mod release` again.",
                manifest.version,
                vpk_path.display()
            ),
        ));
    }

    let status = domain::read_status()?;
    let install = domain::require_install(&status)?.clone();
    let plan = domain::deploy_plan(&install.game_root, &release_dir, &manifest, args.slot)?;
    let report = domain::describe_deploy_plan(&plan);

    if !args.approve {
        if args.json {
            print_json(&json!({
                "ok": true,
                "applied": false,
                "project": lossy(&project_dir),
                "slug": manifest.project_slug,
                "version": manifest.version,
                "gameRoot": lossy(&install.game_root),
                "plan": plan_json(&manifest, &report),
            }));
            return Ok(());
        }

        print_plan(&manifest, &report);
        println!("Dry run: no files were written. Re-run with `--approve` to install.");
        return Ok(());
    }

    let backups = domain::backup_root(&project_dir);
    let result = domain::deploy_apply(
        &install.game_root,
        &release_dir,
        &manifest,
        &plan,
        &backups,
    )?;

    let deployed_at = now_rfc3339();
    project.deployed_slot = Some(report.slot);
    project.deployed_at = Some(deployed_at.clone());
    project.updated_at = deployed_at;
    project.history.push(ProjectHistoryEntry::new(format!(
        "deployed to pak{:02}_dir.vpk",
        report.slot
    )));
    domain::write_project(&project_dir, &project)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "applied": result.deployed,
            "project": lossy(&project_dir),
            "slug": manifest.project_slug,
            "version": manifest.version,
            "gameRoot": lossy(&install.game_root),
            "backups": result.backups.iter().map(|path| lossy(path)).collect::<Vec<_>>(),
            "gameinfoRepaired": result.gameinfo_repaired,
            "plan": plan_json(&manifest, &report),
        }));
        return Ok(());
    }

    print_plan(&manifest, &report);
    println!("Installed {} into {}", manifest.vpk_file, report.addons_dir.display());
    if result.backups.is_empty() {
        println!("  Backups: none, no file was replaced");
    } else {
        println!("  Backups: {}", backups.display());
        for backup in &result.backups {
            println!("    {}", backup.display());
        }
    }
    if result.gameinfo_repaired {
        println!("  gameinfo.gi: addons SearchPaths line repaired");
    }
    println!("Run `dah doctor` to confirm the addon slot and gameinfo.gi.");

    Ok(())
}

fn print_plan(manifest: &ReleaseManifest, report: &DeployReport) {
    println!(
        "Deploy plan for {} {}",
        manifest.project_name, manifest.version
    );
    println!("  Addons: {}", report.addons_dir.display());
    println!("  Slot:   {}", domain::slot_file_name(report.slot));
    println!("  VPK:    {}", manifest.vpk_file);
    if !manifest.chunk_files.is_empty() {
        println!("  Chunks: {}", manifest.chunk_files.join(", "));
    }
    println!(
        "  Entries: {} ({})",
        manifest.entries,
        format_bytes(manifest.total_bytes)
    );

    if report.conflicts.is_empty() {
        println!("  Conflicts: none in the target addon folder");
    } else {
        println!("  Conflicts:");
        for conflict in &report.conflicts {
            println!(
                "    slot {:02}: {} ({} overlapping path(s))",
                conflict.slot, conflict.file, conflict.paths
            );
        }
    }

    if report.gameinfo_needs_repair {
        println!("  gameinfo.gi: WARNING the `Game citadel/addons` line is missing");
    } else {
        println!("  gameinfo.gi: addons SearchPaths present");
    }

    if report.replace_required {
        println!("  Replace:  required, the target files above would be overwritten");
    } else {
        println!("  Replace:  not required");
    }

    for warning in &report.warnings {
        println!("  WARNING {warning}");
    }
}

fn plan_json(manifest: &ReleaseManifest, report: &DeployReport) -> Value {
    json!({
        "addonsDir": lossy(&report.addons_dir),
        "slot": report.slot,
        "slotFile": domain::slot_file_name(report.slot),
        "vpk": manifest.vpk_file,
        "chunks": manifest.chunk_files,
        "entries": report.entries,
        "totalBytes": manifest.total_bytes,
        "conflicts": report
            .conflicts
            .iter()
            .map(|conflict| json!({
                "slot": conflict.slot,
                "file": conflict.file,
                "paths": conflict.paths,
            }))
            .collect::<Vec<_>>(),
        "gameinfoNeedsRepair": report.gameinfo_needs_repair,
        "requiresReplace": report.replace_required,
        "warnings": report.warnings,
    })
}

fn directory_has_entries(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let mut entries = std::fs::read_dir(path).map_err(|error| {
        CliError::new(
            "release_dir",
            format!("Could not read {}: {error}", path.display()),
        )
    })?;

    Ok(entries.next().is_some())
}
