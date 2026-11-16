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
pub use deadlock_core::game_index::{diff_game_index, IndexDiff};
pub use deadlock_core::patch_notes::{
    cache_from_notes, fetch_patch_notes, load_patch_note_cache, patch_note_cache_path,
    save_patch_note_cache, strip_patch_note_html, PatchNote,
};
pub use deadlock_core::project::{
    check_project, create_project, read_project, write_baseline, write_project, ProjectBaseline,
    ProjectCheck, ProjectHistoryEntry, TargetStatus,
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
