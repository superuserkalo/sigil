//! The single seam between the CLI and `deadlock-core`.
//!
//! Every domain call lives here so a core change touches one file. The expected
//! surface is documented in `rust/crates/dah-rs/CORE_API.md`.

use std::path::{Path, PathBuf};

use crate::error::{core, CliError, Result};

pub use deadlock_core::discovery::{AddonsStatus, DeadlockInstall, GameInfoStatus, GameStatus};
pub use deadlock_core::extract::{extract_from_vpk, ExtractOptions, ExtractResult};
pub use deadlock_core::game_catalog::{
    discover_game_vpks, load_game_catalog, search_game_catalog, GameCatalog,
    LoadGameCatalogOptions, LoadGameCatalogResult, SearchGameCatalogOptions,
    SearchGameCatalogResult,
};
pub use deadlock_core::deploy::{
    apply_deploy, plan_deploy, DeployConflict, DeployPlan, DeployResult,
};
pub use deadlock_core::game_index::{diff_game_index, IndexDiff};
pub use deadlock_core::migrate::{apply_migrations, plan_migrations, Migration};
pub use deadlock_core::patch_notes::{
    cache_from_notes, fetch_patch_notes, load_patch_note_cache, patch_note_cache_path,
    save_patch_note_cache, strip_patch_note_html, PatchNote,
};
pub use deadlock_core::payload::{
    format_payload_issues, has_errors, validate_payload, PayloadIssue, PayloadOptions, Severity,
};
pub use deadlock_core::project::{
    baseline_path, check_project, create_project, read_baseline, read_project, write_baseline,
    write_project, ModProject, ProjectBaseline, ProjectCheck, ProjectHistoryEntry, TargetStatus,
};
pub use deadlock_core::release::{
    builds_dir, latest_release, release_dir, write_release_manifest, ReleaseManifest,
    RELEASE_MANIFEST_FILE, RELEASE_SCHEMA,
};
pub use deadlock_core::vpk_write::{
    collect_staging_entries, pack_vpk, PackOptions, PackedVpk,
};

/// Read the install, `gameinfo.gi` and addon slot status. `DAH_GAME_DIR` is
/// honoured inside the core.
pub fn read_status() -> Result<GameStatus> {
    deadlock_core::discovery::read_game_status(None).map_err(|error| core("game status", error))
}

pub fn require_install(status: &GameStatus) -> Result<&DeadlockInstall> {
    status.install.as_ref().ok_or_else(|| {
        CliError::new(
            "game_not_found",
            missing_install_message(&status.searched_roots),
        )
    })
}

pub fn missing_install_message(searched_roots: &[PathBuf]) -> String {
    let mut message = String::from("Deadlock install not found.");
    if !searched_roots.is_empty() {
        let roots: Vec<String> = searched_roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect();
        message.push_str(&format!(" Searched: {}.", roots.join(", ")));
    }
    message.push_str(" Install Deadlock through Steam or set DAH_GAME_DIR.");
    message
}

/// Cache directory for the catalog, inside the workspace.
pub fn cache_dir(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join("index")
}

pub struct CatalogLoad {
    pub status: GameStatus,
    pub install: DeadlockInstall,
    pub loaded: LoadGameCatalogResult,
}

pub fn load_catalog(workspace: &Path, refresh: bool) -> Result<CatalogLoad> {
    let status = read_status()?;
    let install = require_install(&status)?.clone();
    let loaded = load_catalog_for(&install.game_root, workspace, refresh)?;

    Ok(CatalogLoad {
        status,
        install,
        loaded,
    })
}

pub fn load_catalog_for(
    game_root: &Path,
    workspace: &Path,
    refresh: bool,
) -> Result<LoadGameCatalogResult> {
    let options = LoadGameCatalogOptions {
        game_root: game_root.to_path_buf(),
        cache_dir: cache_dir(workspace),
        force: refresh,
    };

    load_game_catalog(options).map_err(|error| core("game catalog", error))
}

/// Locate the directory VPK for a catalog source id such as `citadel`,
/// `core` or `maps/city`.
pub fn vpk_path(game_root: &Path, id: &str) -> Result<PathBuf> {
    let descriptors =
        discover_game_vpks(game_root).map_err(|error| core("VPK discovery", error))?;

    if let Some(found) = descriptors.iter().find(|candidate| candidate.id == id) {
        return Ok(found.path.clone());
    }

    let available: Vec<String> = descriptors.iter().map(|item| item.id.clone()).collect();

    Err(CliError::new(
        "unknown_vpk",
        format!(
            "No VPK named {id:?}. Available: {}.",
            if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            }
        ),
    ))
}

// ---------------------------------------------------------------------------
// Packaging, releases, deploy plans and path migrations.
//
// These wrappers keep every core type and field name in this file. If a core
// shape changes, adjust the adapter here and no command changes.
// ---------------------------------------------------------------------------

/// Compiled output folder inside a project.
pub const STAGING_FOLDER: &str = "staging";

/// The product refuses a single VPK above 2 GiB, so packing chunks beyond it.
pub const VPK_CHUNK_SIZE: u64 = 2 * 1024 * 1024 * 1024;

pub fn staging_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(STAGING_FOLDER)
}

pub fn publish_dir(project_dir: &Path, version: &str) -> PathBuf {
    project_dir.join("publish").join(version)
}

pub fn backup_root(project_dir: &Path) -> PathBuf {
    project_dir.join(".dah").join("backups")
}

pub fn staging_payload_options() -> PayloadOptions {
    PayloadOptions {
        staging_folder: Some(STAGING_FOLDER.to_string()),
    }
}

/// Read every regular file under the staging folder as a forward-slash game
/// path. Directories are not entries.
pub fn staging_entries(staging: &Path) -> Result<Vec<String>> {
    collect_staging_entries(staging).map_err(|error| core("staging entries", error))
}

pub fn validate_staging_entries(entries: &[String]) -> Vec<PayloadIssue> {
    validate_payload(entries, &staging_payload_options())
}

/// Warning-level issues as `entry: message` lines. Errors are excluded because
/// every caller refuses to continue on them.
pub fn payload_warnings(issues: &[PayloadIssue]) -> Vec<String> {
    issues
        .iter()
        .filter(|issue| issue.severity != Severity::Error)
        .map(|issue| match &issue.entry {
            Some(entry) => format!("{}: {}", entry, issue.message),
            None => issue.message.clone(),
        })
        .collect()
}

/// `NN` from the command line, else the project slot, else 01.
pub fn resolve_slot(explicit: Option<u16>, project: &ModProject) -> Result<u16> {
    let slot = explicit.or(project.slot).unwrap_or(1);

    if !(1..=99).contains(&slot) {
        return Err(CliError::new(
            "invalid_slot",
            format!("Addon slot must be from 1 to 99, received {slot}."),
        ));
    }

    Ok(slot)
}

pub fn slot_file_name(slot: u16) -> String {
    format!("pak{slot:02}_dir.vpk")
}

/// Pack the staging tree into `<output>/pakNN_dir.vpk` plus chunk files.
pub fn pack_staging(staging: &Path, output: &Path, slot: u16) -> Result<PackedVpk> {
    let options = PackOptions {
        staging_dir: staging.to_path_buf(),
        output_dir: output.to_path_buf(),
        base_name: format!("pak{slot:02}"),
        chunk_size: VPK_CHUNK_SIZE,
    };

    pack_vpk(&options).map_err(|error| core("pack VPK", error))
}

pub fn release_directory(project_dir: &Path, version: &str) -> PathBuf {
    release_dir(project_dir, version)
}

pub fn release_manifest_file(release: &Path) -> PathBuf {
    release.join(RELEASE_MANIFEST_FILE)
}

pub fn latest_release_manifest(project_dir: &Path) -> Result<Option<ReleaseManifest>> {
    latest_release(project_dir).map_err(|error| core("latest release", error))
}

/// Build the release manifest from a finished pack. A release always has a
/// concrete slot, so the manifest stores it directly.
pub fn build_release_manifest(
    project: &ModProject,
    slot: u16,
    packed: &PackedVpk,
    baseline_captured_at: &str,
    notes: Option<String>,
) -> ReleaseManifest {
    ReleaseManifest {
        schema: RELEASE_SCHEMA,
        project_slug: project.slug.clone(),
        project_name: project.name.clone(),
        version: project.version.clone(),
        slot,
        created_at: crate::util::now_rfc3339(),
        vpk_file: file_name(&packed.dir_path),
        chunk_files: packed.chunk_paths.iter().map(|path| file_name(path)).collect(),
        entries: packed.entry_count,
        total_bytes: packed.total_bytes,
        baseline_captured_at: Some(baseline_captured_at.to_string()),
        notes,
    }
}

pub fn write_manifest(release: &Path, manifest: &ReleaseManifest) -> Result<()> {
    write_release_manifest(&release_manifest_file(release), manifest)
        .map_err(|error| core("write release manifest", error))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub struct DeployConflictView {
    pub slot: u16,
    pub file: String,
    pub paths: usize,
}

/// Flattened deploy plan for printing and JSON output, so the command never
/// names a core struct field.
pub struct DeployReport {
    pub addons_dir: PathBuf,
    pub slot: u16,
    pub conflicts: Vec<DeployConflictView>,
    pub replace_required: bool,
    pub gameinfo_needs_repair: bool,
    pub warnings: Vec<String>,
    pub entries: usize,
}

pub fn describe_deploy_plan(plan: &DeployPlan) -> DeployReport {
    DeployReport {
        addons_dir: plan
            .target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| plan.target.clone()),
        slot: plan.slot,
        conflicts: plan
            .conflicts
            .iter()
            .map(conflict_view)
            .collect::<Vec<_>>(),
        replace_required: plan.replace_required,
        gameinfo_needs_repair: plan.gameinfo_repair,
        warnings: plan.warnings.clone(),
        entries: plan.entries.len(),
    }
}

fn conflict_view(conflict: &DeployConflict) -> DeployConflictView {
    DeployConflictView {
        slot: conflict.slot,
        file: conflict.file.clone(),
        paths: conflict.paths.len(),
    }
}

pub fn deploy_plan(
    game_root: &Path,
    release: &Path,
    manifest: &ReleaseManifest,
    slot: Option<u16>,
) -> Result<DeployPlan> {
    plan_deploy(game_root, release, manifest, slot).map_err(|error| core("deploy plan", error))
}

/// Apply a plan. `--approve` is the replacement approval, so the replace flag
/// is only ever true here, never on the dry run.
pub fn deploy_apply(
    game_root: &Path,
    release: &Path,
    manifest: &ReleaseManifest,
    plan: &DeployPlan,
    backups: &Path,
) -> Result<DeployResult> {
    apply_deploy(game_root, release, manifest, plan, true, backups)
        .map_err(|error| core("deploy", error))
}

pub fn migration_plan(project: &ModProject, catalog: &GameCatalog) -> Vec<Migration> {
    plan_migrations(project, catalog)
}

pub fn apply_migration_plan(
    project_dir: &Path,
    project: &ModProject,
    migrations: &[Migration],
) -> Result<()> {
    apply_migrations(project_dir, project, migrations)
        .map(|_| ())
        .map_err(|error| core("apply migrations", error))
}
