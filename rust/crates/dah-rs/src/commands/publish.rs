//! `dah publish prepare`: stage the latest release, its metadata and install
//! instructions in a folder. It never uploads anything.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain;
use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, lossy, now_rfc3339, print_json};

pub struct PrepareArgs {
    pub workspace: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub json: bool,
}

pub fn prepare(args: &PrepareArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;

    let manifest = domain::latest_release_manifest(&project_dir)?.ok_or_else(|| {
        CliError::new(
            "no_release",
            format!(
                "Project {} has no release to publish. Run `dah mod release` first.",
                project.slug
            ),
        )
    })?;

    let release_dir = domain::release_directory(&project_dir, &manifest.version);
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

    let destination = match &args.out {
        Some(out) => paths::absolute(out)?,
        None => domain::publish_dir(&project_dir, &manifest.version),
    };

    if destination == release_dir {
        return Err(CliError::new(
            "publish_out",
            format!(
                "{} is the release folder itself. Pass --out with a different directory.",
                destination.display()
            ),
        ));
    }

    std::fs::create_dir_all(&destination).map_err(|error| {
        CliError::new(
            "publish_out",
            format!("Could not create {}: {error}", destination.display()),
        )
    })?;

    let mut files = Vec::new();
    let mut names = Vec::new();
    names.push(manifest.vpk_file.clone());
    names.extend(manifest.chunk_files.iter().cloned());

    for name in &names {
        let source = release_dir.join(name);
        if !source.is_file() {
            return Err(CliError::new(
                "release_incomplete",
                format!(
                    "Release {} lists {name} but {} is missing. Run `dah mod release` again.",
                    manifest.version,
                    source.display()
                ),
            ));
        }

        let target = destination.join(name);
        copy_file(&source, &target)?;
        files.push(lossy(&target));
    }

    let created = now_rfc3339();
    let metadata = json!({
        "name": manifest.project_name,
        "slug": manifest.project_slug,
        "version": manifest.version,
        "slot": manifest.slot,
        "slotFile": domain::slot_file_name(manifest.slot),
        "entries": manifest.entries,
        "totalBytes": manifest.total_bytes,
        "targets": project.targets,
        "created": created,
        "files": names,
        "install": {
            "addonsFolder": "game/citadel/addons",
            "requiresGameinfoAddonsLine": true,
        },
    });

    let metadata_path = destination.join("metadata.json");
    write_json(&metadata_path, &metadata)?;

    let readme_path = destination.join("README.md");
    std::fs::write(
        &readme_path,
        render_readme(&project.name, &manifest, project.targets.len()),
    )
    .map_err(|error| {
        CliError::new(
            "publish_readme",
            format!("Could not write {}: {error}", readme_path.display()),
        )
    })?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "out": lossy(&destination),
            "files": files,
            "metadata": lossy(&metadata_path),
            "readme": lossy(&readme_path),
            "uploaded": false,
        }));
        return Ok(());
    }

    println!(
        "Publish folder for {} {}: {}",
        manifest.project_name,
        manifest.version,
        destination.display()
    );
    for file in &files {
        println!("  File:     {file}");
    }
    println!("  Metadata: {}", metadata_path.display());
    println!("  README:   {}", readme_path.display());
    println!(
        "  Size:     {} across {} entr{}",
        format_bytes(manifest.total_bytes),
        manifest.entries,
        if manifest.entries == 1 { "y" } else { "ies" }
    );
    println!("Nothing was uploaded. Review the folder, then upload it yourself.");

    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> Result<()> {
    std::fs::copy(source, target).map_err(|error| {
        CliError::new(
            "publish_copy",
            format!(
                "Could not copy {} to {}: {error}",
                source.display(),
                target.display()
            ),
        )
    })?;

    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text).map_err(|error| {
        CliError::new(
            "publish_metadata",
            format!("Could not write {}: {error}", path.display()),
        )
    })?;

    Ok(())
}

fn render_readme(
    name: &str,
    manifest: &domain::ReleaseManifest,
    targets: usize,
) -> String {
    let mut lines = vec![
        format!("# {name} {}", manifest.version),
        String::new(),
        format!(
            "A Deadlock cosmetic mod built with the Deadlock Agent Harness. It ships {} file(s) for {} target game path(s) as {}.",
            manifest.entries, targets, manifest.vpk_file
        ),
        String::new(),
        "## Install".to_string(),
        String::new(),
        "1. Close Deadlock.".to_string(),
        format!(
            "2. Copy `{}` into `<Deadlock>/game/citadel/addons/`.",
            manifest.vpk_file
        ),
    ];

    if manifest.chunk_files.is_empty() {
        lines.push("3. No chunk files are needed for this release.".to_string());
    } else {
        lines.push(format!(
            "3. Copy the chunk files next to it: {}.",
            manifest
                .chunk_files
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    lines.push(
        "4. Open `<Deadlock>/game/citadel/gameinfo.gi` and make sure the line `Game citadel/addons` comes before `Game citadel` in SearchPaths.".to_string(),
    );
    lines.push("5. Launch Deadlock. The mod loads from the addon slot.".to_string());
    lines.push(String::new());
    lines.push("## Uninstall".to_string());
    lines.push(String::new());
    lines.push(format!(
        "Remove `{}` and its chunk files from `game/citadel/addons/`.",
        manifest.vpk_file
    ));
    lines.push(String::new());
    lines.push("## Notes".to_string());
    lines.push(String::new());
    lines.push(format!("- Slot: `{}`", domain::slot_file_name(manifest.slot)));
    lines.push(format!(
        "- Compiled against game files captured at {}",
        manifest.baseline_captured_at.as_deref().unwrap_or("unknown")
    ));
    lines.push("- Cosmetic only. No gameplay data, skeletons or executables.".to_string());
    lines.push(
        "- A game update can move target paths. Rebuild from a fresh release after a patch."
            .to_string(),
    );
    lines.push(String::new());

    lines.join("\n")
}

pub struct UploadArgs {
    pub workspace: Option<PathBuf>,
    pub open: bool,
    pub json: bool,
}

const GAMEBANANA_SUBMIT_URL: &str = "https://gamebanana.com/mods/submit";

/// Open the GameBanana submission page. The upload, login and category choice
/// stay with the user; this command never sends files anywhere.
pub fn upload(args: &UploadArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;
    let manifest = domain::latest_release_manifest(&project_dir)?.ok_or_else(|| {
        CliError::new(
            "no_release",
            format!(
                "Project {} has no release to publish. Run `dah mod release` first.",
                project.slug
            ),
        )
    })?;
    let folder = domain::publish_dir(&project_dir, &manifest.version);
    let metadata = folder.join("metadata.json");

    if !metadata.is_file() {
        return Err(CliError::new(
            "publish_not_prepared",
            format!(
                "{} has no metadata.json. Run `dah publish prepare` first.",
                folder.display()
            ),
        ));
    }

    if args.open {
        open_browser(GAMEBANANA_SUBMIT_URL)?;
    }

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "slug": project.slug,
            "version": manifest.version,
            "folder": lossy(&folder),
            "url": GAMEBANANA_SUBMIT_URL,
            "opened": args.open,
            "uploaded": false,
        }));
        return Ok(());
    }

    println!(
        "GameBanana handoff for {} {}",
        project.name, manifest.version
    );
    println!("  Folder: {}", folder.display());
    println!("  Page:   {GAMEBANANA_SUBMIT_URL}");
    if args.open {
        println!("  Opened the page in your browser.");
    }
    println!("Sign in, choose the Deadlock category, and upload the files from the folder.");
    println!("Nothing was uploaded by this command.");

    Ok(())
}

fn open_browser(url: &str) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };

    command.spawn().map(|_| ()).map_err(|error| {
        CliError::new(
            "publish_open",
            format!("Could not open {url}: {error}. Open it manually."),
        )
    })
}
