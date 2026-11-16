//! `dah addons`: installed addon slots, conflicts between enabled slots, and
//! one approval-gated enable or disable.
//!
//! `list` and `conflicts` only inspect `game/citadel/addons`. `enable` and
//! `disable` rename a single slot directory file and refuse without
//! `--approve`. Nothing here deletes, renumbers or touches chunk files.

use serde_json::json;

use crate::domain;
use crate::error::{core, Result};
use crate::util::{format_bytes, lossy, print_json};

use deadlock_core::addons::{
    ConflictReport, InstalledAddon, list_installed, read_conflict_report, set_enabled,
};
use deadlock_core::discovery::DeadlockInstall;

pub fn list(json_output: bool) -> Result<()> {
    let install = install()?;
    let addons =
        list_installed(&install.game_root).map_err(|error| core("installed addons", error))?;

    if json_output {
        print_json(&json!({
            "ok": true,
            "addonsDir": lossy(&install.addons_dir),
            "count": addons.len(),
            "enabled": addons.iter().filter(|addon| addon.enabled).count(),
            "addons": addons.iter().map(addon_json).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!("Addon folder: {}", lossy(&install.addons_dir));

    if addons.is_empty() {
        println!("No installed addons. Slots are named pakNN_dir.vpk, NN from 01 to 99.");
        return Ok(());
    }

    let enabled = addons.iter().filter(|addon| addon.enabled).count();
    println!(
        "Slots: {} ({} enabled, {} disabled)",
        addons.len(),
        enabled,
        addons.len() - enabled
    );

    for addon in &addons {
        println!(
            "  slot {:02}  {:<8}  {:<26}  {:>9}  {}{}",
            addon.slot,
            if addon.enabled { "enabled" } else { "disabled" },
            addon.file,
            format_bytes(addon.size_bytes),
            entry_label(addon),
            chunk_label(addon),
        );
    }

    Ok(())
}

pub fn enable(slot: u8, approve: bool, json_output: bool) -> Result<()> {
    set_state(slot, true, approve, json_output)
}

pub fn disable(slot: u8, approve: bool, json_output: bool) -> Result<()> {
    set_state(slot, false, approve, json_output)
}

fn set_state(slot: u8, enabled: bool, approve: bool, json_output: bool) -> Result<()> {
    let install = install()?;
    let result = set_enabled(&install.game_root, slot, enabled, approve)
        .map_err(|error| core(if enabled { "addon enable" } else { "addon disable" }, error))?;

    if json_output {
        print_json(&json!({
            "ok": true,
            "addonsDir": lossy(&install.addons_dir),
            "slot": result.slot,
            "enabled": enabled,
            "from": result.from,
            "to": result.to,
        }));
        return Ok(());
    }

    println!(
        "Slot {:02} {}: {} -> {}",
        result.slot,
        if enabled { "enabled" } else { "disabled" },
        result.from,
        result.to
    );
    println!("Addon folder: {}", lossy(&install.addons_dir));

    Ok(())
}

pub fn conflicts(json_output: bool) -> Result<()> {
    let install = install()?;
    let report = read_conflict_report(&install.game_root)
        .map_err(|error| core("addon conflicts", error))?;

    if json_output {
        print_json(&json!({
            "ok": true,
            "addonsDir": lossy(&install.addons_dir),
            "enabled": report.addons.iter().filter(|addon| addon.enabled).count(),
            "addons": report.addons.iter().map(addon_json).collect::<Vec<_>>(),
            "conflicts": report.conflicts.iter().map(|conflict| json!({
                "path": conflict.path,
                "slots": conflict.slots,
                "winner": conflict.winner,
            })).collect::<Vec<_>>(),
            "warnings": report.warnings,
        }));
        return Ok(());
    }

    print_conflicts(&install, &report);
    Ok(())
}

fn print_conflicts(install: &DeadlockInstall, report: &ConflictReport) {
    println!("Addon folder: {}", lossy(&install.addons_dir));
    println!(
        "Enabled addons: {} of {} installed",
        report.addons.iter().filter(|addon| addon.enabled).count(),
        report.addons.len()
    );

    if report.conflicts.is_empty() {
        println!("Conflicts: none. No game path is shipped by more than one enabled addon.");
    } else {
        println!("Conflicts: {}", report.conflicts.len());
        for conflict in &report.conflicts {
            println!("  {}", conflict.path);
            println!(
                "    slots {} (winner slot {:02})",
                slot_list(&conflict.slots),
                conflict.winner
            );
        }
    }

    for warning in &report.warnings {
        println!("WARNING {warning}");
    }
}

fn install() -> Result<DeadlockInstall> {
    let status = domain::read_status()?;
    Ok(domain::require_install(&status)?.clone())
}

fn addon_json(addon: &InstalledAddon) -> serde_json::Value {
    json!({
        "slot": addon.slot,
        "file": addon.file,
        "sizeBytes": addon.size_bytes,
        "chunkFiles": addon.chunk_files,
        "entryCount": addon.entry_count,
        "enabled": addon.enabled,
    })
}

fn entry_label(addon: &InstalledAddon) -> String {
    match addon.entry_count {
        Some(1) => "1 entry".to_string(),
        Some(count) => format!("{count} entries"),
        None => "entries unknown".to_string(),
    }
}

fn chunk_label(addon: &InstalledAddon) -> String {
    if addon.chunk_files.is_empty() {
        String::new()
    } else {
        format!("  chunks: {}", addon.chunk_files.join(", "))
    }
}

fn slot_list(slots: &[u8]) -> String {
    slots
        .iter()
        .map(|slot| format!("{slot:02}"))
        .collect::<Vec<_>>()
        .join(", ")
}
