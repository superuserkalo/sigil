//! `dah publish prepare`: stage the latest release, its metadata and install
//! instructions in a folder. It never uploads anything.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

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
                "Project {} has no release to publish. Run `Sigil cli mod release` first.",
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
                "Release {} is missing its VPK at {}. Run `Sigil cli mod release` again.",
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
                    "Release {} lists {name} but {} is missing. Run `Sigil cli mod release` again.",
                    manifest.version,
                    source.display()
                ),
            ));
        }

        let target = destination.join(name);
        copy_file(&source, &target)?;
        files.push(lossy(&target));
    }

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

    let archive = create_archive(&destination, &manifest, &names, &readme_path)?;
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
        "archive": {
            "file": archive.file,
            "bytes": archive.bytes,
            "sha256": archive.sha256,
        },
        "install": {
            "addonsFolder": "game/citadel/addons",
            "requiresGameinfoAddonsLine": true,
        },
    });

    let metadata_path = destination.join("metadata.json");
    write_json(&metadata_path, &metadata)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "out": lossy(&destination),
            "files": files,
            "metadata": lossy(&metadata_path),
            "readme": lossy(&readme_path),
            "archive": lossy(&destination.join(&archive.file)),
            "archiveSha256": archive.sha256,
            "archiveBytes": archive.bytes,
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
    println!("  Archive:  {}", destination.join(&archive.file).display());
    println!("  SHA-256:  {}", archive.sha256);
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
            "A Deadlock cosmetic mod built with Sigil. It ships {} file(s) for {} target game path(s) as {}.",
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
    pub approve: bool,
    pub open: bool,
    pub json: bool,
}

pub struct ConfirmArgs {
    pub workspace: Option<PathBuf>,
    pub url: Option<String>,
    pub skip_verify: bool,
    pub allow_http: bool,
    pub json: bool,
}

pub struct StatusArgs {
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

const GAMEBANANA_SUBMIT_URL: &str = "https://gamebanana.com/mods/submit";
const UPLOAD_RECORD: &str = "upload.json";

struct ArchiveInfo {
    file: String,
    bytes: u64,
    sha256: String,
}

/// Create a GameBanana-ready archive next to the prepared files.
fn create_archive(
    folder: &Path,
    manifest: &domain::ReleaseManifest,
    names: &[String],
    readme: &Path,
) -> Result<ArchiveInfo> {
    let file_name = format!("{}-{}.zip", manifest.project_slug, manifest.version);
    let path = folder.join(&file_name);
    let file = fs::File::create(&path).map_err(|error| {
        CliError::new(
            "publish_archive",
            format!("Could not create {}: {error}", path.display()),
        )
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for name in names {
        let source = folder.join(name);
        let bytes = fs::read(&source).map_err(|error| {
            CliError::new(
                "publish_archive",
                format!("Could not read {}: {error}", source.display()),
            )
        })?;
        zip.start_file(name, options).map_err(|error| {
            CliError::new("publish_archive", format!("Could not add {name}: {error}"))
        })?;
        zip.write_all(&bytes).map_err(|error| {
            CliError::new("publish_archive", format!("Could not write {name}: {error}"))
        })?;
    }

    let readme_bytes = fs::read(readme).map_err(|error| {
        CliError::new(
            "publish_archive",
            format!("Could not read {}: {error}", readme.display()),
        )
    })?;
    zip.start_file("README.md", options).map_err(|error| {
        CliError::new("publish_archive", format!("Could not add README.md: {error}"))
    })?;
    zip.write_all(&readme_bytes).map_err(|error| {
        CliError::new("publish_archive", format!("Could not write README.md: {error}"))
    })?;
    zip.finish().map_err(|error| {
        CliError::new("publish_archive", format!("Could not finish archive: {error}"))
    })?;

    let bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let sha256 = sha256_file(&path)?;

    Ok(ArchiveInfo {
        file: file_name,
        bytes,
        sha256,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|error| {
        CliError::new(
            "publish_hash",
            format!("Could not read {}: {error}", path.display()),
        )
    })?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

fn upload_record_path(folder: &Path) -> PathBuf {
    folder.join(UPLOAD_RECORD)
}

fn read_upload_record(folder: &Path) -> Result<Option<Value>> {
    let path = upload_record_path(folder);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CliError::new(
                "publish_state",
                format!("Could not read {}: {error}", path.display()),
            ))
        }
    };
    Ok(Some(serde_json::from_str(&text).map_err(|error| {
        CliError::new(
            "publish_state",
            format!("{} is not valid JSON: {error}", path.display()),
        )
    })?))
}

fn write_upload_record(folder: &Path, record: &Value) -> Result<PathBuf> {
    let path = upload_record_path(folder);
    write_json(&path, record)?;
    Ok(path)
}

/// Plan or start an approval-gated GameBanana upload. With `approve`, the
/// archive and a durable upload record are prepared and the submission page is
/// opened. The user still signs in and submits; no API credential is read.
pub fn upload(args: &UploadArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;
    let manifest = domain::latest_release_manifest(&project_dir)?.ok_or_else(|| {
        CliError::new(
            "no_release",
            format!(
                "Project {} has no release to publish. Run `Sigil cli mod release` first.",
                project.slug
            ),
        )
    })?;
    let folder = domain::publish_dir(&project_dir, &manifest.version);
    let metadata_path = folder.join("metadata.json");

    if !metadata_path.is_file() {
        return Err(CliError::new(
            "publish_not_prepared",
            format!(
                "{} has no metadata.json. Run `Sigil cli publish prepare` first.",
                folder.display()
            ),
        ));
    }

    let metadata: Value = serde_json::from_str(&fs::read_to_string(&metadata_path).map_err(
        |error| CliError::new("publish_metadata", format!("Could not read metadata: {error}")),
    )?)
    .map_err(|error| CliError::new("publish_metadata", format!("Invalid metadata: {error}")))?;

    let archive_file = metadata
        .get("archive")
        .and_then(|archive| archive.get("file"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::new(
                "publish_archive",
                "metadata.json has no archive. Run `Sigil cli publish prepare` again.",
            )
        })?;
    let archive_path = folder.join(archive_file);
    if !archive_path.is_file() {
        return Err(CliError::new(
            "publish_archive",
            format!("Archive {} is missing. Run `Sigil cli publish prepare` again.", archive_path.display()),
        ));
    }
    let sha256 = sha256_file(&archive_path)?;
    let bytes = fs::metadata(&archive_path)
        .map(|meta| meta.len())
        .unwrap_or(0);

    if !args.approve {
        if args.json {
            print_json(&json!({
                "ok": true,
                "approved": false,
                "project": lossy(&project_dir),
                "slug": project.slug,
                "version": manifest.version,
                "folder": lossy(&folder),
                "archive": lossy(&archive_path),
                "archiveSha256": sha256,
                "archiveBytes": bytes,
                "url": GAMEBANANA_SUBMIT_URL,
                "uploaded": false,
            }));
            return Ok(());
        }

        println!(
            "GameBanana upload plan for {} {}",
            project.name, manifest.version
        );
        println!("  Folder:  {}", folder.display());
        println!("  Archive: {}", archive_path.display());
        println!("  SHA-256: {sha256}");
        println!("  Page:    {GAMEBANANA_SUBMIT_URL}");
        println!("Nothing was opened or recorded. Re-run with `--approve` to start the handoff.");
        return Ok(());
    }

    let now = now_rfc3339();
    let record = json!({
        "schema": 1,
        "state": "awaiting_user",
        "projectSlug": project.slug,
        "projectName": project.name,
        "version": manifest.version,
        "folder": lossy(&folder),
        "archiveFile": archive_file,
        "archiveSha256": sha256,
        "archiveBytes": bytes,
        "submitUrl": GAMEBANANA_SUBMIT_URL,
        "submissionUrl": null,
        "verified": false,
        "createdAt": now,
        "updatedAt": now,
    });
    let record_path = write_upload_record(&folder, &record)?;

    if args.open {
        open_browser(GAMEBANANA_SUBMIT_URL)?;
    }

    if args.json {
        print_json(&json!({
            "ok": true,
            "approved": true,
            "state": "awaiting_user",
            "project": lossy(&project_dir),
            "slug": project.slug,
            "version": manifest.version,
            "folder": lossy(&folder),
            "archive": lossy(&archive_path),
            "archiveSha256": sha256,
            "archiveBytes": bytes,
            "url": GAMEBANANA_SUBMIT_URL,
            "record": lossy(&record_path),
            "opened": args.open,
            "uploaded": false,
        }));
        return Ok(());
    }

    println!(
        "GameBanana handoff approved for {} {}",
        project.name, manifest.version
    );
    println!("  Folder:  {}", folder.display());
    println!("  Archive: {}", archive_path.display());
    println!("  SHA-256: {sha256}");
    println!("  Page:    {GAMEBANANA_SUBMIT_URL}");
    println!("  Record:  {}", record_path.display());
    if args.open {
        println!("  Opened the page in your browser.");
    }
    println!("Sign in, choose the Deadlock category, upload the archive, then run:");
    println!(
        "  dah publish confirm --url https://gamebanana.com/mods/<id>"
    );
    println!("Nothing was uploaded by this command.");

    Ok(())
}

/// Record and verify the submission URL after the user uploads in the browser.
pub fn confirm(args: &ConfirmArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;
    let manifest = domain::latest_release_manifest(&project_dir)?.ok_or_else(|| {
        CliError::new(
            "no_release",
            format!(
                "Project {} has no release. Run `Sigil cli mod release` first.",
                project.slug
            ),
        )
    })?;
    let folder = domain::publish_dir(&project_dir, &manifest.version);
    let mut record = read_upload_record(&folder)?.ok_or_else(|| {
        CliError::new(
            "publish_state",
            "No upload record. Run `Sigil cli publish upload --approve` first.",
        )
    })?;

    if record.get("state").and_then(Value::as_str) != Some("awaiting_user") {
        return Err(CliError::new(
            "publish_state",
            format!(
                "Upload record is {:?}, not awaiting_user.",
                record.get("state").and_then(Value::as_str)
            ),
        ));
    }

    let archive_file = record
        .get("archiveFile")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new("publish_state", "Upload record has no archiveFile."))?;
    let archive_path = folder.join(archive_file);
    let sha256 = sha256_file(&archive_path)?;
    let expected = record
        .get("archiveSha256")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if sha256 != expected {
        return Err(CliError::new(
            "publish_archive",
            format!(
                "Archive hash changed: expected {expected}, found {sha256}. Re-run `Sigil cli publish prepare` and upload again."
            ),
        ));
    }

    let url = args
        .url
        .clone()
        .or_else(|| std::env::var("GAMEBANANA_SUBMISSION_URL").ok())
        .ok_or_else(|| {
            CliError::new(
                "publish_confirm",
                "Pass --url https://gamebanana.com/mods/<id> after uploading, or set GAMEBANANA_SUBMISSION_URL.",
            )
        })?;

    let local_test = args.allow_http
        && (url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:"));

    if !url.starts_with("https://gamebanana.com/") && !local_test {
        return Err(CliError::new(
            "publish_confirm",
            "Submission URL must be https://gamebanana.com/mods/<id>. Pass --allow-http only for a local test server.",
        ));
    }

    let verified = if args.skip_verify {
        false
    } else {
        let mut response = deadlock_core::net::get(&url)
            .header("User-Agent", concat!("Sigil/", env!("CARGO_PKG_VERSION")))
            .call()
            .map_err(|error| {
                CliError::new("publish_verify", format!("Could not fetch {url}: {error}"))
            })?;
        let status = response.status();
        let body = response.body_mut().read_to_string().map_err(|error| {
            CliError::new("publish_verify", format!("Could not read {url}: {error}"))
        })?;
        let name = project.name.to_lowercase();
        let slug = project.slug.to_lowercase();
        let body_lower = body.to_lowercase();

        if status.as_u16() != 200 || (!body_lower.contains(&name) && !body_lower.contains(&slug)) {
            return Err(CliError::new(
                "publish_verify",
                format!(
                    "The submission page at {url} did not confirm this project. Check the URL or pass --skip-verify to record it unverified."
                ),
            ));
        }
        true
    };

    let now = now_rfc3339();
    if let Some(object) = record.as_object_mut() {
        object.insert("state".into(), Value::String(if verified { "verified" } else { "recorded" }.into()));
        object.insert("submissionUrl".into(), Value::String(url.clone()));
        object.insert("verified".into(), Value::Bool(verified));
        object.insert("confirmedAt".into(), Value::String(now.clone()));
        object.insert("updatedAt".into(), Value::String(now));
    }
    let record_path = write_upload_record(&folder, &record)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "state": record.get("state"),
            "verified": verified,
            "project": lossy(&project_dir),
            "slug": project.slug,
            "version": manifest.version,
            "submissionUrl": url,
            "record": lossy(&record_path),
        }));
    } else {
        println!(
            "Recorded GameBanana submission for {} {}: {url}",
            project.name, manifest.version
        );
        println!(
            "  State:  {}",
            if verified { "verified" } else { "recorded (not checked)" }
        );
        println!("  Record: {}", record_path.display());
    }

    Ok(())
}

pub fn status(args: &StatusArgs) -> Result<()> {
    let project_dir = paths::project_dir(args.workspace.as_deref())?;
    let project = domain::read_project(&project_dir)?;
    let manifest = domain::latest_release_manifest(&project_dir)?.ok_or_else(|| {
        CliError::new(
            "no_release",
            format!("Project {} has no release.", project.slug),
        )
    })?;
    let folder = domain::publish_dir(&project_dir, &manifest.version);
    let record = read_upload_record(&folder)?.ok_or_else(|| {
        CliError::new(
            "publish_state",
            format!("No upload record for {}. Run `Sigil cli publish upload` first.", folder.display()),
        )
    })?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "project": lossy(&project_dir),
            "slug": project.slug,
            "version": manifest.version,
            "folder": lossy(&folder),
            "record": record,
        }));
    } else {
        println!(
            "GameBanana upload state for {} {}",
            project.name, manifest.version
        );
        println!("  Folder: {}", folder.display());
        println!(
            "  State:  {}",
            record.get("state").and_then(Value::as_str).unwrap_or("unknown")
        );
        println!(
            "  URL:    {}",
            record
                .get("submissionUrl")
                .and_then(Value::as_str)
                .unwrap_or("not recorded")
        );
        println!(
            "  SHA:    {}",
            record
                .get("archiveSha256")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        );
    }

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
