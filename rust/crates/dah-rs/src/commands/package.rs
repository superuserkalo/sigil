//! `dah mod validate` and `dah mod pack`: check the staging tree and turn it
//! into `pakNN_dir.vpk` inside the workspace.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::{self, PayloadIssue};
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, lossy, print_json};

pub struct ValidateArgs {
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct PackArgs {
    pub workspace: Option<PathBuf>,
    pub slot: Option<u16>,
    pub out: Option<PathBuf>,
    pub json: bool,
}

/// A project staging folder that exists and belongs to a readable project.
struct Staged {
    project_dir: PathBuf,
    staging: PathBuf,
    entries: Vec<String>,
    issues: Vec<PayloadIssue>,
}

fn staged(workspace: Option<&Path>) -> Result<Staged> {
    let project_dir = paths::project_dir(workspace)?;
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

    Ok(Staged {
        project_dir,
        staging,
        entries,
        issues,
    })
}

pub fn validate(args: &ValidateArgs) -> Result<bool> {
    let staged = staged(args.workspace.as_deref())?;
    let ok = !domain::has_errors(&staged.issues);

    if args.json {
        print_json(&json!({
            "ok": ok,
            "project": lossy(&staged.project_dir),
            "staging": lossy(&staged.staging),
            "entries": staged.entries.len(),
            "issues": staged.issues.iter().map(issue_json).collect::<Vec<_>>(),
        }));
        return Ok(ok);
    }

    println!(
        "Payload: {} entr{} in {}",
        staged.entries.len(),
        if staged.entries.len() == 1 { "y" } else { "ies" },
        staged.staging.display()
    );
    println!("{}", domain::format_payload_issues(&staged.issues));

    if ok {
        println!("Run `dah mod pack` to build the VPK.");
    } else {
        println!("Fix the errors above before packing.");
    }

    Ok(ok)
}

pub fn pack(args: &PackArgs) -> Result<i32> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;
    let slot = domain::resolve_slot(args.slot, &project)?;
    let staged = staged(Some(&project_dir))?;
    let warnings = domain::payload_warnings(&staged.issues);

    if domain::has_errors(&staged.issues) {
        return Err(CliError::new(
            "payload_invalid",
            format!(
                "Payload validation failed, nothing was packed.\n{}",
                domain::format_payload_issues(&staged.issues)
            ),
        ));
    }

    let output = match &args.out {
        Some(path) => paths::absolute(path)?,
        None => domain::builds_dir(&project_dir),
    };

    std::fs::create_dir_all(&output).map_err(|error| {
        CliError::new(
            "pack_output",
            format!("Could not create {}: {error}", output.display()),
        )
    })?;

    let packed = domain::pack_staging(&staged.staging, &output, slot)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "slug": project.slug,
            "version": project.version,
            "slot": slot,
            "slotFile": domain::slot_file_name(slot),
            "outputDir": lossy(&output),
            "vpk": lossy(&packed.dir_path),
            "chunks": packed.chunk_paths.iter().map(|path| lossy(path)).collect::<Vec<_>>(),
            "entries": packed.entry_count,
            "totalBytes": packed.total_bytes,
            "warnings": warnings,
        }));
        return Ok(0);
    }

    println!(
        "Packed {} entr{} as {}",
        packed.entry_count,
        if packed.entry_count == 1 { "y" } else { "ies" },
        domain::slot_file_name(slot)
    );
    println!("  VPK:     {}", packed.dir_path.display());
    for chunk in &packed.chunk_paths {
        println!("  Chunk:   {}", chunk.display());
    }
    println!("  Size:    {}", format_bytes(packed.total_bytes));
    println!("  Output:  {}", output.display());

    for warning in &warnings {
        println!("WARNING {warning}");
    }

    println!("Next: `dah mod release` records this build as a release.");

    Ok(0)
}

fn issue_json(issue: &PayloadIssue) -> Value {
    json!({
        "severity": issue.severity.as_str(),
        "code": issue.code,
        "entry": issue.entry,
        "message": issue.message,
    })
}
