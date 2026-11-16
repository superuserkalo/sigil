//! `dah launch`: start Deadlock with addons, start it vanilla with the addons
//! stashed, and restore the stash afterwards.
//!
//! `modded` and `vanilla` refuse without `--approve` because they start Steam.
//! `vanilla` moves addon files into the app data directory and leaves them
//! there: the game outlives the command, so `restore` is a separate approved
//! step. `status` is read only.

use std::path::Path;

use serde_json::{Value, json};

use crate::domain;
use crate::error::{Result, core};
use crate::paths;
use crate::util::{format_bytes, lossy, print_json};

use deadlock_core::launch::{
    LaunchResult, RestoreResult, StashFile, StashStatus, launch_modded, launch_vanilla,
    restore_vanilla, stash_manifest_path, stash_status, vanilla_stash_dir,
};

pub fn modded(approve: bool, json_output: bool) -> Result<()> {
    let result = launch_modded(approve).map_err(|error| core("launch modded", error))?;

    if json_output {
        print_json(&launch_json(&result));
        return Ok(());
    }

    print_launch(&result);
    Ok(())
}

pub fn vanilla(approve: bool, json_output: bool) -> Result<()> {
    let install = install()?;
    let data_dir = paths::data_dir()?;

    let result = launch_vanilla(&install.game_root, &data_dir, approve)
        .map_err(|error| core("launch vanilla", error))?;
    let stash = stash_status(&data_dir);
    let stash_dir = result
        .stash_dir
        .clone()
        .unwrap_or_else(|| vanilla_stash_dir(&data_dir));

    if json_output {
        let entries: Vec<Value> = stash
            .as_ref()
            .map(|status| status.files.iter().map(file_json).collect())
            .unwrap_or_default();

        print_json(&json!({
            "ok": true,
            "mode": result.mode,
            "appId": result.app_id,
            "url": result.url,
            "gameRoot": lossy(&result.game_root),
            "addonsDir": lossy(&result.addons_dir),
            "opener": result.opener,
            "stashDir": lossy(&stash_dir),
            "manifest": lossy(&stash_manifest_path(&data_dir)),
            "files": entries.len(),
            "entries": entries,
            "restoreCommand": "dah launch restore --approve",
        }));
        return Ok(());
    }

    print_launch(&result);
    println!("Addons stashed at: {}", stash_dir.display());
    println!("Restore after the game exits with `dah launch restore --approve`.");

    Ok(())
}

pub fn restore(approve: bool, json_output: bool) -> Result<()> {
    let data_dir = paths::data_dir()?;
    let result =
        restore_vanilla(&data_dir, approve).map_err(|error| core("launch restore", error))?;

    if json_output {
        print_json(&restore_json(&result));
        return Ok(());
    }

    print_restore(&result);
    Ok(())
}

pub fn status(json_output: bool) -> Result<()> {
    let data_dir = paths::data_dir()?;
    let status = stash_status(&data_dir);

    if json_output {
        match &status {
            Some(status) => print_json(&status_json(status, &data_dir)),
            None => print_json(&json!({
                "ok": true,
                "present": false,
                "stashRoot": lossy(&vanilla_stash_dir(&data_dir)),
                "manifest": lossy(&stash_manifest_path(&data_dir)),
            })),
        }
        return Ok(());
    }

    print_status(&data_dir, status.as_ref());
    Ok(())
}

fn install() -> Result<domain::DeadlockInstall> {
    let status = domain::read_status()?;
    Ok(domain::require_install(&status)?.clone())
}

fn launch_json(result: &LaunchResult) -> Value {
    json!({
        "ok": true,
        "mode": result.mode,
        "appId": result.app_id,
        "url": result.url,
        "gameRoot": lossy(&result.game_root),
        "addonsDir": lossy(&result.addons_dir),
        "opener": result.opener,
    })
}

fn print_launch(result: &LaunchResult) {
    println!(
        "Launching Deadlock ({}) through Steam: {}",
        result.mode, result.url
    );
    println!("Install: {}", result.game_root.display());
    println!("Addon folder: {}", result.addons_dir.display());
    println!("Opener: {}", result.opener);

    if result.mode == deadlock_core::launch::MODE_MODDED {
        println!("Mods load only when gameinfo.gi lists citadel/addons in SearchPaths.");
    }
}

fn restore_json(result: &RestoreResult) -> Value {
    json!({
        "ok": true,
        "addonsDir": lossy(&result.addons_dir),
        "stashDir": lossy(&result.stash_dir),
        "manifest": lossy(&result.manifest_path),
        "restored": result.restored.iter().map(file_json).collect::<Vec<_>>(),
        "alreadyPresent": result.already_present,
        "removedManifest": result.removed_manifest,
        "warnings": result.warnings,
    })
}

fn print_restore(result: &RestoreResult) {
    let bytes: u64 = result.restored.iter().map(|file| file.bytes).sum();

    if result.restored.is_empty() {
        println!("Restored no files from {}.", result.stash_dir.display());
    } else {
        println!(
            "Restored {} files ({}) to {}",
            result.restored.len(),
            format_bytes(bytes),
            result.addons_dir.display()
        );

        for file in &result.restored {
            println!(
                "  {:<26}  {:>9}  crc32 {:08x}  {}",
                file_name(&file.to),
                format_bytes(file.bytes),
                file.crc32,
                file.from.display()
            );
        }
    }

    for name in &result.already_present {
        println!("Already in place: {name}");
    }

    if result.removed_manifest {
        println!("Removed {}", result.manifest_path.display());
    }

    for warning in &result.warnings {
        println!("WARNING {warning}");
    }
}

fn status_json(status: &StashStatus, data_dir: &Path) -> Value {
    json!({
        "ok": status.problem.is_none(),
        "present": true,
        "stashRoot": lossy(&vanilla_stash_dir(data_dir)),
        "manifest": lossy(&status.manifest_path),
        "stashDir": lossy(&status.stash_dir),
        "createdAt": status.created_at,
        "gameRoot": lossy(&status.game_root),
        "addonsDir": lossy(&status.addons_dir),
        "files": status.files.len(),
        "presentFiles": status.present,
        "missingFiles": status.missing,
        "totalBytes": status.total_bytes,
        "entries": status.files.iter().map(file_json).collect::<Vec<_>>(),
        "problem": status.problem,
    })
}

fn print_status(data_dir: &Path, status: Option<&StashStatus>) {
    match status {
        None => {
            println!(
                "Vanilla stash: none under {}",
                vanilla_stash_dir(data_dir).display()
            );
        }
        Some(status) if status.problem.is_some() => {
            println!(
                "Vanilla stash: unreadable at {}",
                status.manifest_path.display()
            );
            println!("Problem: {}", status.problem.clone().unwrap_or_default());
        }
        Some(status) => {
            println!("Vanilla stash: {}", status.stash_dir.display());
            println!("Created: {}", status.created_at);
            println!("Install: {}", status.game_root.display());
            println!("Addon folder: {}", status.addons_dir.display());
            println!(
                "Files: {} ({} in the stash, {} missing, {})",
                status.files.len(),
                status.present,
                status.missing,
                format_bytes(status.total_bytes)
            );

            for file in &status.files {
                println!(
                    "  {:<26}  {:>9}  crc32 {:08x}",
                    file_name(&file.to),
                    format_bytes(file.bytes),
                    file.crc32
                );
            }

            println!("Restore with `dah launch restore --approve`.");
        }
    }
}

fn file_json(file: &StashFile) -> Value {
    json!({
        "from": lossy(&file.from),
        "to": lossy(&file.to),
        "bytes": file.bytes,
        "crc32": file.crc32,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
