//! `dah install plan|apply|list|cache prune`: the Browse install pipeline.
//!
//! `plan` downloads into the workspace, extracts, validates the payload and
//! writes a plan. It never touches the game install.
//!
//! `apply` loads that plan and writes it into `game/citadel/addons` only with
//! `--approve`. Without the flag it prints the plan again.
//!
//! Wiring lives in `main.rs`; every entry point here is a plain run function.

use std::path::PathBuf;

use serde_json::json;

use deadlock_core::install::{
    DownloadPolicy, InstallRequest, PlanOptions, apply_install, format_cache_prune,
    format_install_plan, format_install_status, install_status, journal_path, load_plan,
    plan_install, prune_download_cache,
};

use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, lossy, print_json};

pub struct PlanArgs {
    /// GameBanana mod id, mod URL, archive URL or local archive.
    pub target: String,
    /// Requested addon slot, 1 to 99. Defaults to the lowest free slot.
    pub slot: Option<u16>,
    /// Replace an occupied slot after backing it up.
    pub force: bool,
    /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
    pub workspace: Option<PathBuf>,
    /// Extra download origin, for example a local mirror. Defaults to DAH_INSTALL_MIRROR.
    pub mirror: Option<String>,
    pub json: bool,
}

pub struct ApplyArgs {
    /// Plan id printed by `dah install plan`.
    pub plan_id: String,
    /// Write the payload into the game addons folder.
    pub approve: bool,
    /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct ListArgs {
    /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct PruneArgs {
    /// Workspace directory. Defaults to DAH_WORKSPACE_DIR or <data>/workspace.
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

/// Plan an install from a GameBanana id, a mod URL or an archive.
pub fn plan(args: &PlanArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;

    let request = InstallRequest::new(args.target.clone(), workspace)
        .map_err(|error| CliError::new("install_source", error.to_string()))?
        .with_slot(args.slot)
        .with_force(args.force);

    let mut policy = DownloadPolicy::from_environment()
        .map_err(|error| CliError::new("install_allowlist", error.to_string()))?;

    if let Some(mirror) = args.mirror.as_deref() {
        policy = policy
            .with_mirror(mirror)
            .map_err(|error| CliError::new("install_allowlist", error.to_string()))?;
    }

    let options = PlanOptions {
        policy,
        ..PlanOptions::default()
    };

    let plan = plan_install(&request, &options)
        .map_err(|error| CliError::new("install_plan", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "applied": false,
            "plan": serde_json::to_value(&plan)?,
        }));
        return Ok(());
    }

    println!("{}", format_install_plan(&plan));
    println!();
    println!(
        "Nothing was written to the game install. Re-run with `dah install apply {} --approve`.",
        plan.id
    );

    Ok(())
}

/// Load a stored plan and install it with approval.
pub fn apply(args: &ApplyArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;

    let plan = load_plan(&workspace, &args.plan_id)
        .map_err(|error| CliError::new("install_plan", error.to_string()))?;

    if !args.approve {
        if args.json {
            print_json(&json!({
                "ok": true,
                "applied": false,
                "plan": serde_json::to_value(&plan)?,
            }));
            return Ok(());
        }

        println!("{}", format_install_plan(&plan));
        println!();
        println!("Dry run: nothing was written. Re-run with `--approve` to install.");
        return Ok(());
    }

    let record = apply_install(&plan, true)
        .map_err(|error| CliError::new("install_apply", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "applied": true,
            "installed": record.entry,
            "backups": record.backups.iter().map(|path| lossy(path)).collect::<Vec<_>>(),
            "gameinfoRepaired": record.gameinfo_repaired,
        }));
        return Ok(());
    }

    println!(
        "Installed slot {:02}: {}",
        record.entry.slot, record.entry.file
    );
    println!("  Target:  {}", record.target.display());
    println!(
        "  Files:   {} ({})",
        record.entry.entries,
        format_bytes(record.entry.bytes)
    );
    println!(
        "  VPK:     {} (sha256 {})",
        format_bytes(record.entry.vpk_bytes),
        record.entry.vpk_sha256.chars().take(16).collect::<String>()
    );

    if record.entry.chunk_files.is_empty() {
        println!("  Chunks:  none");
    } else {
        println!("  Chunks:  {}", record.entry.chunk_files.join(", "));
    }

    if record.backups.is_empty() {
        println!("  Backups: none, the slot was free");
    } else {
        println!("  Backups: {}", record.backups.len());
        for backup in &record.backups {
            println!("    {}", backup.display());
        }
    }

    if !record.entry.removed_chunks.is_empty() {
        println!(
            "  Removed stale chunks: {}",
            record.entry.removed_chunks.join(", ")
        );
    }

    if !record.entry.removed_files.is_empty() {
        println!(
            "  Removed slot files: {}",
            record.entry.removed_files.join(", ")
        );
    }

    println!(
        "  gameinfo.gi: {}",
        if record.gameinfo_repaired {
            "addons SearchPath repaired"
        } else {
            "unchanged"
        }
    );
    println!("  Journal: {}", journal_path(&workspace).display());
    println!("Run `dah install list` to confirm the file is still in place.");

    Ok(())
}

/// List install journal entries and whether each file is still present.
pub fn list(args: &ListArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let status = install_status(&workspace)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "workspace": lossy(&workspace),
            "journal": lossy(&journal_path(&workspace)),
            "count": status.len(),
            "installs": status,
        }));
        return Ok(());
    }

    println!("{}", format_install_status(&status));

    if !status.is_empty() {
        println!();
        println!("Journal: {}", journal_path(&workspace).display());
    }

    Ok(())
}

/// Delete downloaded archives and extracted trees from the workspace.
pub fn cache_prune(args: &PruneArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let report = prune_download_cache(&workspace)
        .map_err(|error| CliError::new("install_cache", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "workspace": lossy(&workspace),
            "removed": report.removed,
            "totalBytes": report.total_bytes,
        }));
        return Ok(());
    }

    println!("{}", format_cache_prune(&report));
    println!();
    println!(
        "An unapplied plan whose archive was pruned cannot be applied. Run `dah install plan` again for it."
    );

    Ok(())
}
