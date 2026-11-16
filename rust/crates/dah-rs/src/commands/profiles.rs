//! `dah profiles`: capture installed addon slots as a named set and apply it.
//!
//! `list` and `save` only touch the workspace. `apply` renames slot files inside
//! `game/citadel/addons` and refuses without `--approve`.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::domain;
use crate::error::{Result, core};
use crate::paths;
use crate::util::{lossy, print_json};

use deadlock_core::profiles::{
    Profile, apply_profile, capture_profile, find_profile, list_profiles, profile_dir, profile_path,
};

pub fn list(json_output: bool, workspace: Option<PathBuf>) -> Result<()> {
    let workspace = paths::workspace_root(workspace.as_deref())?;
    let profiles = list_profiles(&workspace).map_err(|error| core("profiles", error))?;
    let directory = profile_dir(&workspace);

    if json_output {
        print_json(&json!({
            "ok": true,
            "profilesDir": lossy(&directory),
            "count": profiles.len(),
            "profiles": profiles.iter().map(profile_json).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!("Profiles: {} in {}", profiles.len(), directory.display());

    if profiles.is_empty() {
        println!("No saved profiles. Run `dah profiles save <name>` to capture the current slots.");
        return Ok(());
    }

    for profile in &profiles {
        println!(
            "  {:<28}  [{}]  {}",
            profile.name,
            profile.slug,
            slot_summary(profile)
        );
    }

    Ok(())
}

pub fn save(name: &str, json_output: bool, workspace: Option<PathBuf>) -> Result<()> {
    let workspace = paths::workspace_root(workspace.as_deref())?;
    let install = install()?;

    let profile = capture_profile(&install.game_root, &workspace, name)
        .map_err(|error| core("profile save", error))?;
    let path = profile_path(&workspace, &profile.slug);

    if json_output {
        print_json(&json!({
            "ok": true,
            "path": lossy(&path),
            "profile": profile_json(&profile),
        }));
        return Ok(());
    }

    println!("Saved profile {} [{}]: {}", profile.name, profile.slug, slot_summary(&profile));
    println!("File: {}", path.display());

    Ok(())
}

pub fn apply(
    name: &str,
    approve: bool,
    json_output: bool,
    workspace: Option<PathBuf>,
) -> Result<()> {
    let workspace = paths::workspace_root(workspace.as_deref())?;
    let install = install()?;

    let profile = find_profile(&workspace, name).map_err(|error| core("profile lookup", error))?;
    let result = apply_profile(&install.game_root, &workspace, &profile.slug, approve)
        .map_err(|error| core("profile apply", error))?;

    if json_output {
        print_json(&json!({
            "ok": true,
            "profile": {
                "name": profile.name,
                "slug": profile.slug,
            },
            "addonsDir": lossy(&install.addons_dir),
            "changed": result.changed,
            "warnings": result.warnings,
        }));
        return Ok(());
    }

    println!(
        "Profile {} [{}] applied to {}",
        profile.name,
        profile.slug,
        install.addons_dir.display()
    );

    if result.changed.is_empty() {
        println!("No changes: the install already matches this profile.");
    } else {
        for change in &result.changed {
            println!("  {change}");
        }
    }

    for warning in &result.warnings {
        println!("WARNING {warning}");
    }

    Ok(())
}

fn install() -> Result<domain::DeadlockInstall> {
    let status = domain::read_status()?;
    Ok(domain::require_install(&status)?.clone())
}

fn slot_summary(profile: &Profile) -> String {
    let enabled = profile.enabled_slots();
    let total = profile.slots.len();

    match total {
        1 => format!("1 slot, {enabled} enabled"),
        _ => format!("{total} slots, {enabled} enabled"),
    }
}

fn profile_json(profile: &Profile) -> Value {
    json!({
        "name": profile.name,
        "slug": profile.slug,
        "createdAt": profile.created_at,
        "updatedAt": profile.updated_at,
        "slots": profile.slots.iter().map(|slot| json!({
            "slot": slot.slot,
            "file": slot.file,
            "enabled": slot.enabled,
        })).collect::<Vec<_>>(),
    })
}
