//! `dah crosshair`: read, edit and apply the crosshair convars.
//!
//! Reading is free. Applying writes the managed block into
//! `game/citadel/cfg/autoexec.cfg` through the same approved autoexec path as the
//! Autoexec screen: the old file is copied beside itself first.

use std::path::PathBuf;

use serde_json::json;

use deadlock_core::autoexec;
use deadlock_core::crosshair::{self, Crosshair};

use crate::error::{CliError, Result};
use crate::paths::CliPaths;
use crate::util::print_json;

pub struct ShowArgs {
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct ApplyArgs {
    /// Write the managed block into autoexec.cfg.
    pub approve: bool,
    pub json: bool,
}

pub struct ClearArgs {
    pub approve: bool,
    pub json: bool,
}

pub struct ImportArgs {
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

/// `dah crosshair show`: the editor defaults and the commands they emit.
pub fn show(args: &ShowArgs) -> Result<()> {
    let _ = &args.workspace;
    let crosshair = Crosshair::default();
    crosshair
        .validate()
        .map_err(|error| CliError::new("crosshair_settings", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "settings": crosshair,
            "commands": crosshair.commands(),
            "note": "These are the editor defaults. Apply writes them to autoexec.cfg, or import reads the in-game values.",
        }));
        return Ok(());
    }

    println!("Crosshair (editor defaults)");
    println!("  {crosshair}");
    println!("  hero-specific crosshairs {}", if crosshair.disable_hero_specific_crosshairs { "disabled" } else { "enabled" });
    println!();
    println!("{}", crosshair.command_text());
    Ok(())
}

/// `dah crosshair import`: read the player's in-game crosshair.
pub fn import(paths: &CliPaths, args: &ImportArgs) -> Result<()> {
    let _ = &args.workspace;
    let game_root = game_root(paths)?;
    let vcfg = game_root
        .join("game")
        .join("citadel")
        .join("cfg")
        .join("machine_convars.vcfg");
    let text = std::fs::read_to_string(&vcfg).map_err(|error| {
        CliError::new(
            "crosshair_import",
            format!("Could not read {}: {error}", vcfg.display()),
        )
    })?;
    let crosshair = crosshair::parse_machine_convars(&text).ok_or_else(|| {
        CliError::new(
            "crosshair_import",
            format!(
                "{} has no citadel_crosshair_ convars. The game writes them after the first crosshair change.",
                vcfg.display()
            ),
        )
    })?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "settings": crosshair,
            "commands": crosshair.commands(),
            "source": vcfg.display().to_string(),
        }));
        return Ok(());
    }

    println!("Imported the in-game crosshair from {}", vcfg.display());
    println!("  {crosshair}");
    println!();
    println!("{}", crosshair.command_text());
    Ok(())
}

/// `dah crosshair apply <file|--defaults>`: write the managed block.
pub fn apply(paths: &CliPaths, args: &ApplyArgs) -> Result<()> {
    let game_root = game_root(paths)?;
    let existing = autoexec::read_autoexec(&game_root)
        .map_err(|error| CliError::new("crosshair_autoexec", error.to_string()))?
        .unwrap_or_default();
    let merged = crosshair::apply_to_autoexec(&existing, &Crosshair::default())
        .map_err(|error| CliError::new("crosshair_autoexec", error.to_string()))?;

    if !args.approve {
        if args.json {
            print_json(&json!({
                "ok": true,
                "approved": false,
                "wouldWrite": autoexec::autoexec_path(&game_root).display().to_string(),
                "block": crosshair::Crosshair::default().commands(),
            }));
        } else {
            println!("Plan: merge the crosshair block into {}", autoexec::autoexec_path(&game_root).display());
            println!("Nothing was written. Re-run with --approve.");
        }
        return Ok(());
    }

    let result = autoexec::write_autoexec(&game_root, &merged, true)
        .map_err(|error| CliError::new("crosshair_autoexec", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "approved": true,
            "path": result.path.display().to_string(),
            "backup": result.backup.as_ref().map(|path| path.display().to_string()),
        }));
    } else {
        println!("Wrote the crosshair block into {}", result.path.display());
        match result.backup {
            Some(backup) => println!("  previous file copied to {}", backup.display()),
            None => println!("  there was no previous file to back up"),
        }
    }
    Ok(())
}

/// `dah crosshair clear`: remove the managed block.
pub fn clear(paths: &CliPaths, args: &ClearArgs) -> Result<()> {
    let game_root = game_root(paths)?;
    let existing = autoexec::read_autoexec(&game_root)
        .map_err(|error| CliError::new("crosshair_autoexec", error.to_string()))?
        .unwrap_or_default();
    if !existing.contains(crosshair::AUTOEXEC_BEGIN) {
        println!("autoexec.cfg has no managed crosshair block. Nothing to clear.");
        return Ok(());
    }
    let merged = crosshair::clear_from_autoexec(&existing)
        .map_err(|error| CliError::new("crosshair_autoexec", error.to_string()))?;
    if !args.approve {
        println!("Plan: remove the managed crosshair block. Nothing was written.");
        println!("Re-run with --approve.");
        return Ok(());
    }
    let result = autoexec::write_autoexec(&game_root, &merged, true)
        .map_err(|error| CliError::new("crosshair_autoexec", error.to_string()))?;
    if args.json {
        print_json(&json!({
            "ok": true,
            "path": result.path.display().to_string(),
            "backup": result.backup.as_ref().map(|path| path.display().to_string()),
        }));
    } else {
        println!("Removed the managed crosshair block from {}", result.path.display());
    }
    Ok(())
}

fn game_root(paths: &CliPaths) -> Result<PathBuf> {
    let _ = paths;
    let status = deadlock_core::discovery::read_game_status(None)
        .map_err(|error| CliError::new("crosshair_game", error.to_string()))?;
    let install = status.install.ok_or_else(|| {
        CliError::new(
            "crosshair_game",
            "No Deadlock install was found. Set DAH_GAME_DIR to the Deadlock folder.",
        )
    })?;
    Ok(install.game_root)
}
