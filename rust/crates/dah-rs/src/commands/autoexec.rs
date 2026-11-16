//! `dah autoexec`: read, replace and clear `game/citadel/cfg/autoexec.cfg`.
//!
//! `show` is read only. `set` and `clear` write into the game install and
//! refuse without `--approve`; an existing file is backed up first.

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::domain;
use crate::error::{CliError, Result, core};
use crate::util::{lossy, print_json};

use deadlock_core::autoexec::{
    AutoexecWriteResult, autoexec_path, list_lines, normalize_trailing_newline, read_autoexec,
    write_autoexec,
};

pub fn show(json_output: bool) -> Result<()> {
    let install = install()?;
    let path = autoexec_path(&install.game_root);
    let text = read_autoexec(&install.game_root).map_err(|error| core("autoexec", error))?;
    let line_count = text.as_deref().map(|value| list_lines(value).len());

    if json_output {
        print_json(&json!({
            "ok": true,
            "path": lossy(&path),
            "exists": text.is_some(),
            "lineCount": line_count,
            "text": text.unwrap_or_default(),
        }));
        return Ok(());
    }

    match text {
        None => {
            println!("autoexec.cfg: missing at {}", path.display());
            println!("Write one with `dah autoexec set --file <path> --approve`.");
        }
        Some(text) => {
            println!(
                "autoexec.cfg: {} ({} lines)",
                path.display(),
                list_lines(&text).len()
            );
            println!();
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
    }

    Ok(())
}

pub fn set(file: &Path, approve: bool, json_output: bool) -> Result<()> {
    let text = fs::read_to_string(file).map_err(|error| {
        CliError::new(
            "io",
            format!("Could not read {}: {error}", lossy(file)),
        )
    })?;

    let install = install()?;
    let result = write_autoexec(&install.game_root, &text, approve)
        .map_err(|error| core("autoexec write", error))?;

    report_write(&result, &text, "Wrote", json_output)
}

pub fn clear(approve: bool, json_output: bool) -> Result<()> {
    let install = install()?;
    let result = write_autoexec(&install.game_root, "", approve)
        .map_err(|error| core("autoexec clear", error))?;

    report_write(&result, "", "Cleared", json_output)
}

fn install() -> Result<domain::DeadlockInstall> {
    let status = domain::read_status()?;
    Ok(domain::require_install(&status)?.clone())
}

fn report_write(
    result: &AutoexecWriteResult,
    text: &str,
    action: &str,
    json_output: bool,
) -> Result<()> {
    let normalized = normalize_trailing_newline(text);
    let line_count = list_lines(&normalized).len();

    if json_output {
        print_json(&json!({
            "ok": true,
            "action": action,
            "path": lossy(&result.path),
            "backup": result.backup.as_ref().map(|path| lossy(path)),
            "lineCount": line_count,
            "bytes": normalized.len(),
        }));
        return Ok(());
    }

    println!("{action} {} ({line_count} lines)", result.path.display());

    match &result.backup {
        Some(backup) => println!("Backup: {}", backup.display()),
        None => println!("Backup: none (the file did not exist)"),
    }

    Ok(())
}
