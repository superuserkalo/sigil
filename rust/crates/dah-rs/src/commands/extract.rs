//! `dah extract`: copy exact game paths out of a game VPK into the workspace.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain;
use crate::error::{CliError, Result};
use crate::paths::{self, CliPaths};
use crate::util::{format_bytes, lossy, print_json};

pub struct ExtractArgs {
    pub paths: Vec<String>,
    pub vpk: String,
    pub out: Option<PathBuf>,
    pub overwrite: bool,
    pub json: bool,
}

/// Returns false when some entries failed, so the process can exit non-zero
/// after the report is printed.
pub fn run(cli_paths: &CliPaths, args: &ExtractArgs) -> Result<bool> {
    let requested: Vec<String> = args
        .paths
        .iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();

    if requested.is_empty() {
        return Err(CliError::new(
            "no_paths",
            "Pass at least one exact game path, for example models/heroes_wip/billy/billy.vmdl_c.",
        ));
    }

    let status = domain::read_status()?;
    let install = domain::require_install(&status)?.clone();
    let dir_vpk_path = domain::vpk_path(&install.game_root, &args.vpk)?;
    let destination = resolve_destination(&cli_paths.workspace_dir, args.out.as_deref())?;

    let options = domain::ExtractOptions {
        dir_vpk_path,
        entry_paths: requested,
        destination: destination.clone(),
        overwrite: args.overwrite,
        verify_crc: Some(true),
        max_files: Some(500),
    };

    let result = domain::extract_from_vpk(options)?;
    let ok = result.errors.is_empty();

    if args.json {
        print_json(&render_json(&result, ok));
        return Ok(ok);
    }

    if result.files.is_empty() && result.errors.is_empty() {
        println!(
            "Nothing extracted. {} file(s) already existed. Pass --overwrite to replace them.",
            result.skipped.len()
        );
        return Ok(ok);
    }

    if !result.files.is_empty() {
        println!(
            "Extracted {} file(s), {}, into {}",
            result.files.len(),
            format_bytes(result.total_bytes),
            lossy(&result.destination)
        );
        for file in &result.files {
            println!("  {} -> {}", file.path, lossy(&file.target));
        }
    }

    for skipped in &result.skipped {
        println!("SKIPPED {}: {}", skipped.path, skipped.message);
    }

    for issue in &result.errors {
        if issue.path.is_empty() {
            println!("ERROR {}", issue.message);
        } else {
            println!("ERROR {}: {}", issue.path, issue.message);
        }
    }

    Ok(ok)
}

fn resolve_destination(workspace: &Path, out: Option<&Path>) -> Result<PathBuf> {
    let candidate = match out {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => workspace.join(path),
        None => workspace.join("extracted"),
    };

    let resolved = paths::absolute(&candidate)?;
    let workspace_root = paths::absolute(workspace)?;

    if !paths::is_inside(&workspace_root, &resolved) {
        return Err(CliError::new(
            "outside_workspace",
            format!(
                "Destination {} is outside the workspace {}. Extraction writes inside the mod workspace only.",
                resolved.display(),
                workspace_root.display()
            ),
        ));
    }

    Ok(resolved)
}

fn render_json(result: &domain::ExtractResult, ok: bool) -> Value {
    json!({
        "ok": ok,
        "destination": lossy(&result.destination),
        "totalBytes": result.total_bytes,
        "files": result
            .files
            .iter()
            .map(|file| json!({
                "path": file.path,
                "target": lossy(&file.target),
                "bytes": file.bytes,
            }))
            .collect::<Vec<_>>(),
        "skipped": result
            .skipped
            .iter()
            .map(|issue| json!({
                "path": issue.path,
                "message": issue.message,
            }))
            .collect::<Vec<_>>(),
        "errors": result
            .errors
            .iter()
            .map(|issue| json!({
                "path": issue.path,
                "message": issue.message,
            }))
            .collect::<Vec<_>>(),
    })
}
