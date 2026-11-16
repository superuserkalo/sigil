//! `dah doctor`: install, gameinfo and addon slot status.

use serde_json::{json, Value};

use deadlock_core::{demo, install, locker, toolchain};

use crate::domain;
use crate::error::Result;
use crate::paths::CliPaths;
use crate::util::{format_bytes, lossy, print_json};

pub fn run(paths: &CliPaths, json_output: bool) -> Result<()> {
    let status = domain::read_status()?;
    let warnings = warnings_for(&status);

    if json_output {
        print_json(&render_json(paths, &status, &warnings));
        return Ok(());
    }

    println!("Deadlock Agent Harness");
    println!("  Data dir:  {}", lossy(&paths.data_dir));
    println!("  Workspace: {}", lossy(&paths.workspace_dir));

    match &status.install {
        Some(install) => {
            println!("Deadlock install: {}", lossy(&install.game_root));
        }
        None => {
            println!("Deadlock install: not found");
            if !status.searched_roots.is_empty() {
                println!("Searched Steam roots:");
                for root in &status.searched_roots {
                    println!("  {}", lossy(root));
                }
            }
            println!("Set DAH_GAME_DIR to the Deadlock folder, its `game` folder or `game/citadel`.");
        }
    }

    if let Some(install) = &status.install {
        match &status.gameinfo {
            None => println!("gameinfo.gi: not read"),
            Some(gameinfo) if !gameinfo.present => println!("gameinfo.gi: missing"),
            Some(gameinfo) if gameinfo.addons_search_path => {
                match gameinfo.line {
                    Some(line) => println!(
                        "gameinfo.gi: addons SearchPaths present at line {line}"
                    ),
                    None => println!("gameinfo.gi: addons SearchPaths present"),
                }
            }
            Some(_) => {
                println!("gameinfo.gi: addons SearchPaths missing, mods will not load");
                println!("  {}", lossy(&install.gameinfo_path));
            }
        }
    }

    match &status.addons {
        None => {
            if status.install.is_some() {
                println!("Addon folder: not read");
            }
        }
        Some(addons) if !addons.present => {
            println!("Addon folder: missing ({})", lossy(&addons.directory));
        }
        Some(addons) => {
            if addons.slots.is_empty() {
                println!("Addon folder: {} (no slots used)", lossy(&addons.directory));
            } else {
                println!("Addon folder: {}", lossy(&addons.directory));
                for slot in &addons.slots {
                    println!(
                        "  slot {:02}: {} ({})",
                        slot.slot,
                        slot.file_name,
                        format_bytes(slot.size_bytes)
                    );
                }
            }

            match addons.next_free_slot {
                Some(slot) => println!("Next free slot: pak{slot:02}_dir.vpk"),
                None => println!("Next free slot: none, all 99 slots are used"),
            }

            if !addons.ignored_files.is_empty() {
                println!("Ignored files: {}", addons.ignored_files.join(", "));
            }
            if !addons.chunk_files.is_empty() {
                println!("Chunk files: {}", addons.chunk_files.join(", "));
            }
        }
    }

    println!();
    println!("Capabilities");
    for (label, value) in capabilities() {
        println!("  {label}: {value}");
    }

    for warning in &warnings {
        println!("WARNING {warning}");
    }

    Ok(())
}

/// What this build can actually do on this machine. Every line is a measured
/// availability, not a feature claim.
fn capabilities() -> Vec<(String, String)> {
    let mut lines = Vec::new();

    lines.push((
        "Stats API".to_string(),
        "api.deadlock-api.com, keyless".to_string(),
    ));

    let demos = demo::backend_status();
    let positions = if demos.position_extraction_available {
        "positions and heatmaps available"
    } else {
        "container only, positions and heatmaps absent"
    };
    lines.push(("Demo backend".to_string(), positions.to_string()));

    let tools = install::ToolPaths::from_environment();
    let archives = format!(
        "zip in process, 7z {}, rar {}",
        if tools.seven_zip.is_some() {
            "available"
        } else {
            "needs DAH_7Z or 7z on PATH"
        },
        if tools.unrar.is_some() {
            "available"
        } else {
            "needs DAH_UNRAR or unrar on PATH"
        }
    );
    lines.push(("Install archives".to_string(), archives));

    let chain = toolchain::toolchain_status();
    lines.push((
        "Decompile".to_string(),
        if chain.decompile_available {
            "ready".to_string()
        } else {
            chain
                .decompile_reason
                .clone()
                .unwrap_or_else(|| "unavailable".to_string())
        },
    ));
    lines.push((
        "Compile".to_string(),
        if chain.compile_available {
            "ready".to_string()
        } else {
            chain
                .compile_reason
                .clone()
                .unwrap_or_else(|| "unavailable".to_string())
        },
    ));

    let assets = locker::LockerAssetPaths::discover();
    lines.push((
        "Locker art".to_string(),
        assets
            .heroes_dir
            .as_ref()
            .map(|path| lossy(path))
            .unwrap_or_else(|| {
                "no art root. Set DAH_LOCKER_ASSETS to a folder with heroes/ and names/".to_string()
            }),
    ));

    let mesh = if std::env::var_os("DAH_MESHGEN_COMMAND").is_some() {
        "local command configured"
    } else if std::env::var_os("DAH_MESHGEN_URL").is_some() {
        "local HTTP model server configured"
    } else if std::env::var_os("MESHY_API_KEY").is_some() {
        "Meshy adapter configured, local backend not configured"
    } else {
        "no generative backend. The deterministic character stages still run"
    };
    lines.push(("Character".to_string(), mesh.to_string()));

    lines
}

fn warnings_for(status: &domain::GameStatus) -> Vec<String> {
    let mut warnings = Vec::new();

    if status.install.is_none() {
        return warnings;
    }

    match &status.gameinfo {
        None => warnings.push("gameinfo.gi could not be read.".to_string()),
        Some(gameinfo) if !gameinfo.present => warnings.push(
            "gameinfo.gi is missing. Mods will not load until the file is restored.".to_string(),
        ),
        Some(gameinfo) if !gameinfo.addons_search_path => warnings.push(
            "gameinfo.gi has no `Game citadel/addons` SearchPaths line. Mods will not load."
                .to_string(),
        ),
        Some(_) => {}
    }

    if let Some(addons) = &status.addons
        && addons.present
        && addons.next_free_slot.is_none()
    {
        warnings.push("All 99 addon slots are used.".to_string());
    }

    warnings
}

fn render_json(paths: &CliPaths, status: &domain::GameStatus, warnings: &[String]) -> Value {
    json!({
        "ok": true,
        "dataDir": lossy(&paths.data_dir),
        "workspaceDir": lossy(&paths.workspace_dir),
        "install": status.install.as_ref().map(install_json),
        "searchedRoots": status
            .searched_roots
            .iter()
            .map(|root| lossy(root))
            .collect::<Vec<_>>(),
        "gameinfo": status.gameinfo.as_ref().map(gameinfo_json),
        "addons": status.addons.as_ref().map(addons_json),
        "warnings": warnings,
    })
}

fn install_json(install: &domain::DeadlockInstall) -> Value {
    json!({
        "steamRoot": lossy(&install.steam_root),
        "gameRoot": lossy(&install.game_root),
        "citadelDir": lossy(&install.citadel_dir),
        "gameinfoPath": lossy(&install.gameinfo_path),
        "addonsDir": lossy(&install.addons_dir),
    })
}

fn gameinfo_json(gameinfo: &domain::GameInfoStatus) -> Value {
    json!({
        "present": gameinfo.present,
        "addonsSearchPath": gameinfo.addons_search_path,
        "line": gameinfo.line,
    })
}

fn addons_json(addons: &domain::AddonsStatus) -> Value {
    json!({
        "directory": lossy(&addons.directory),
        "present": addons.present,
        "slots": addons
            .slots
            .iter()
            .map(|slot| json!({
                "slot": slot.slot,
                "fileName": slot.file_name,
                "sizeBytes": slot.size_bytes,
            }))
            .collect::<Vec<_>>(),
        "chunkFiles": addons.chunk_files,
        "ignoredFiles": addons.ignored_files,
        "nextFreeSlot": addons.next_free_slot,
    })
}
