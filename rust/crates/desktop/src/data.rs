//! Real reads from `deadlock-core` at startup.
//!
//! Only reads happen here: game status, the cached Steam news feed, an optional workspace
//! project and the latest release for that project. A missing install, cache or release leaves
//! the matching screen empty instead of inventing data. The review queue loads the game catalog,
//! so it only runs from a local scene or an explicit action, never on every frame.

use std::fs;
use std::path::{Path, PathBuf};

use deadlock_core::{deploy, discovery, game_catalog, migrate, patch_notes, project, release};

use crate::app::State;
use crate::model::{
    self, Compat, DeployConflictView, DeployPlanView, HistoryEntry, PathMove, Project, ProjectState,
    PublishInfo, Reference, ReleaseInfo, ReviewItem, ReviewQueue, Target,
};

pub fn init_state() -> State {
    let mut state = State::default();
    state.paths.data = data_dir().display().to_string();
    let game_root = apply_game_status(&mut state);
    load_cached_patches(&mut state);
    load_workspace_project(&mut state);
    refresh_release_with(&mut state, game_root.as_deref());
    state
}

/// The app data directory. `DAH_DATA_DIR` overrides the platform default.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DAH_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/DeadlockAgentHarness"))
            .unwrap_or_else(|| PathBuf::from(".dah"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(|h| PathBuf::from(h).join("DeadlockAgentHarness"))
            .unwrap_or_else(|| PathBuf::from(".dah"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(|h| PathBuf::from(h).join("DeadlockAgentHarness"))
            .or_else(|| home.map(|h| h.join(".local/share/deadlock-harness")))
            .unwrap_or_else(|| PathBuf::from(".dah"))
    }
}

pub fn workspace_dir() -> PathBuf {
    if let Some(dir) =
        std::env::var_os("DAH_WORKSPACE_DIR").or_else(|| std::env::var_os("DAH_WORKSPACE"))
    {
        return PathBuf::from(dir);
    }
    data_dir().join("workspace")
}

fn apply_game_status(state: &mut State) -> Option<PathBuf> {
    match discovery::read_game_status(None) {
        Ok(status) => {
            let Some(install) = &status.install else {
                state.game_detected = false;
                state.game_label = "Game not detected".to_string();
                state.status =
                    "No Deadlock install found in the Steam roots. Open Settings to see the paths searched."
                        .to_string();
                return None;
            };
            state.game_detected = true;
            state.paths.game = install.game_root.display().to_string();
            state.game_label = match &status.addons {
                Some(addons) => match addons.next_free_slot {
                    Some(slot) => format!("citadel · slot {slot:02} free"),
                    None => "citadel · no free slot".to_string(),
                },
                None => "citadel · addons missing".to_string(),
            };
            if let Some(info) = &status.gameinfo
                && !info.addons_search_path
            {
                state.status = "Game found, but gameinfo.gi does not list citadel/addons. The game will ignore every addon until SearchPaths is repaired."
                    .to_string();
            }
            Some(install.game_root.clone())
        }
        Err(error) => {
            state.game_detected = false;
            state.game_label = "Game not detected".to_string();
            state.status = format!("Game discovery failed: {error}");
            None
        }
    }
}

fn load_cached_patches(state: &mut State) {
    let workspace = workspace_dir();
    let path = patch_notes::patch_note_cache_path(&workspace);
    let Ok(cache) = patch_notes::load_patch_note_cache(&path) else {
        return;
    };
    let notes: Vec<model::PatchNote> = cache
        .notes
        .iter()
        .filter(|note| note.is_patch)
        .take(40)
        .map(|note| {
            let body: Vec<String> = patch_notes::strip_patch_note_html(&note.contents)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(14)
                .map(str::to_string)
                .collect();
            let date: String = note.published_at.chars().take(10).collect();
            model::PatchNote {
                title: note.title.trim().to_string(),
                date,
                build: String::new(),
                body,
                changed: Vec::new(),
                targets: Vec::new(),
                is_new: false,
                source: "Steam news · cached".to_string(),
            }
        })
        .collect();
    if !notes.is_empty() {
        state.patches = notes;
        state.selected_patch = 0;
    }
}

fn load_workspace_project(state: &mut State) {
    let root = workspace_dir();
    let mut dirs = Vec::new();

    if root.join("project.json").is_file() {
        dirs.push(root.clone());
    }

    let mods = root.join("mods");
    if let Ok(read_dir) = fs::read_dir(&mods) {
        let mut children: Vec<PathBuf> = read_dir
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("project.json").is_file())
            .collect();
        children.sort();
        dirs.extend(children);
    }

    let mut projects = Vec::new();
    let mut first_dir: Option<PathBuf> = None;

    for dir in dirs {
        let Ok(project) = project::read_project(&dir) else {
            continue;
        };
        if first_dir.is_none() {
            first_dir = Some(dir.clone());
        }
        projects.push(map_project(&project));
    }

    state.projects = projects;
    state.selected_project = 0;

    if let Some(dir) = first_dir {
        state.paths.workspace = dir.display().to_string();
        state.paths.staging = dir.join("staging").display().to_string();
        if state.status.is_empty()
            && let Some(project) = state.projects.first()
        {
            state.status = format!("Opened {} from {}.", project.name, dir.display());
        }
    }
}

fn map_project(project: &project::ModProject) -> Project {
    let history = project
        .history
        .iter()
        .rev()
        .take(12)
        .map(|entry| HistoryEntry {
            version: entry.action.clone(),
            date: entry.timestamp.chars().take(10).collect(),
            note: entry.note.clone().unwrap_or_else(|| entry.action.clone()),
        })
        .collect();
    Project {
        name: project.name.clone(),
        version: project.version.clone(),
        slot: project.slot.unwrap_or(0),
        state: ProjectState::Draft,
        updated: project
            .updated_at
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
        summary: format!(
            "Read from {}. {} target{}, {} reference{}.",
            project.slug,
            project.targets.len(),
            if project.targets.len() == 1 { "" } else { "s" },
            project.references.len(),
            if project.references.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        targets: project
            .targets
            .iter()
            .map(|path| Target {
                path: path.clone(),
                kind: kind_for(path),
                status: Compat::Unchanged,
            })
            .collect(),
        references: project
            .references
            .iter()
            .map(|vpk| Reference {
                vpk: vpk.clone(),
                mode: "read".to_string(),
            })
            .collect(),
        history,
    }
}

fn kind_for(path: &str) -> String {
    if path.ends_with(".vmat_c") {
        "material"
    } else if path.ends_with(".vtex_c") {
        "texture"
    } else if path.ends_with(".vmdl_c") {
        "model"
    } else if path.ends_with(".vpcf_c") {
        "particle"
    } else if path.ends_with(".vsnd_c") {
        "sound"
    } else if path.ends_with(".vxml_c") {
        "layout"
    } else if path.ends_with(".vcss_c") {
        "style"
    } else {
        "file"
    }
    .to_string()
}

/// Start the Release scene with the real workspace: project, game status, latest
/// release and deploy plan. The review queue is loaded here too because a scene
/// load is an explicit moment, not a render frame.
pub fn init_release_state() -> State {
    let mut state = init_state();
    state.screen = model::Screen::Release;
    load_review_queue(&mut state);
    state
}

/// Re-read the latest release and its deploy plan. Small files only: the release
/// manifest, its VPK directory tree and the addon VPK trees in a game install.
pub fn refresh_release(state: &mut State) {
    let root = if state.paths.workspace.is_empty() {
        workspace_dir()
    } else {
        PathBuf::from(&state.paths.workspace)
    };
    if !root.join("project.json").is_file() {
        refresh_release_with(state, None);
        return;
    }
    let game_root = discovery::read_game_status(None)
        .ok()
        .and_then(|status| status.install.map(|install| install.game_root));
    refresh_release_with(state, game_root.as_deref());
}

fn refresh_release_with(state: &mut State, game_root: Option<&Path>) {
    let root = if state.paths.workspace.is_empty() {
        workspace_dir()
    } else {
        PathBuf::from(&state.paths.workspace)
    };
    state.release.info = None;
    state.release.plan = None;
    state.release.plan_error = None;
    state.release.publish = None;
    state.release.lookup_error = None;

    let manifest_path = root.join("project.json");
    if !manifest_path.is_file() {
        state.release.lookup_error = Some(format!(
            "No project manifest at {}. A release is read from the open workspace.",
            manifest_path.display()
        ));
        return;
    }

    let manifest = match release::latest_release(&root) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return,
        Err(error) => {
            state.release.lookup_error =
                Some(format!("Could not read builds/ in {}: {error}", root.display()));
            return;
        }
    };

    let release_dir = release::release_dir(&root, &manifest.version);
    let vpk_present = release_dir.join(&manifest.vpk_file).is_file();

    state.release.info = Some(ReleaseInfo {
        version: manifest.version.clone(),
        slot: manifest.slot,
        vpk_file: manifest.vpk_file.clone(),
        chunk_files: manifest.chunk_files.clone(),
        entries: manifest.entries,
        total_bytes: manifest.total_bytes,
        created_at: manifest.created_at.clone(),
        baseline_captured_at: manifest.baseline_captured_at.clone(),
        notes: manifest.notes.clone(),
        dir: release_dir.display().to_string(),
        vpk_present,
    });

    state.release.publish = Some(read_publish(&root, &manifest.version));

    let Some(game_root) = game_root else {
        return;
    };

    match deploy::plan_deploy(game_root, &release_dir, &manifest, None) {
        Ok(plan) => {
            state.release.plan = Some(DeployPlanView {
                slot: plan.slot,
                target: plan.target.display().to_string(),
                chunk_targets: plan
                    .chunk_targets
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                replace_required: plan.replace_required,
                conflicts: plan
                    .conflicts
                    .into_iter()
                    .map(|conflict| DeployConflictView {
                        slot: conflict.slot,
                        file: conflict.file,
                        paths: conflict.paths,
                    })
                    .collect(),
                gameinfo_repair: plan.gameinfo_repair,
                warnings: plan.warnings,
                entry_count: plan.entries.len(),
                entry_preview: plan.entries.into_iter().take(8).collect(),
            });
        }
        Err(error) => state.release.plan_error = Some(error.to_string()),
    }
}

fn read_publish(project_dir: &Path, version: &str) -> PublishInfo {
    let dir = project_dir.join("publish").join(version);
    let metadata_path = dir.join("metadata.json");
    let dir_present = dir.is_dir();
    let metadata_present = metadata_path.is_file();
    let mut created_at = None;
    let mut files = Vec::new();

    if metadata_present
        && let Ok(text) = fs::read_to_string(&metadata_path)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
    {
        created_at = value
            .get("created")
            .and_then(|created| created.as_str())
            .map(short_stamp);
        files = value
            .get("files")
            .and_then(|files| files.as_array())
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| file.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
    }

    PublishInfo {
        dir: dir.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        dir_present,
        metadata_present,
        created_at,
        files,
    }
}

/// Run the review queue: one read-only pass over every project in the workspace.
/// This loads the game catalog, so call it from a local scene or a press action.
pub fn load_review_queue(state: &mut State) {
    let root = workspace_dir();
    let mut queue = ReviewQueue {
        workspace: root.display().to_string(),
        catalog_label: "no catalog loaded".to_string(),
        items: Vec::new(),
        error: None,
    };

    let mut loaded: Option<(discovery::GameStatus, game_catalog::GameCatalog)> = None;

    match discovery::read_game_status(None) {
        Ok(status) => match status.install.clone() {
            Some(install) => {
                let options = game_catalog::LoadGameCatalogOptions {
                    game_root: install.game_root,
                    cache_dir: root.join(".dah").join("index"),
                    force: false,
                };
                match game_catalog::load_game_catalog(options) {
                    Ok(result) => {
                        queue.catalog_label = format!(
                            "{} paths from {} game source{}",
                            result.catalog.entries.len(),
                            result.catalog.sources.len(),
                            if result.catalog.sources.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        );
                        loaded = Some((status, result.catalog));
                    }
                    Err(error) => {
                        queue.error = Some(format!("The game catalog could not be loaded: {error}"));
                    }
                }
            }
            None => {
                queue.error =
                    Some("No Deadlock install was found, so project checks did not run.".to_string());
            }
        },
        Err(error) => {
            queue.error = Some(format!("Game discovery failed: {error}"));
        }
    }

    for dir in collect_project_dirs(&root) {
        queue.items.push(review_item(&dir, &loaded));
    }

    state.release.review = Some(queue);
}

fn collect_project_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if root.join("project.json").is_file() {
        dirs.push(root.to_path_buf());
    }

    if let Ok(entries) = fs::read_dir(root.join("mods")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("project.json").is_file() {
                dirs.push(path);
            }
        }
    }

    dirs.sort();
    dirs
}

fn review_item(
    dir: &Path,
    loaded: &Option<(discovery::GameStatus, game_catalog::GameCatalog)>,
) -> ReviewItem {
    let mut item = ReviewItem {
        slug: dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string()),
        name: String::new(),
        version: String::new(),
        path: dir.display().to_string(),
        slot: None,
        unchanged: 0,
        changed: 0,
        missing: 0,
        moved: 0,
        unbaselined: 0,
        migrations: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };

    let project = match project::read_project(dir) {
        Ok(project) => project,
        Err(error) => {
            item.error = Some(error.to_string());
            return item;
        }
    };

    item.slug = project.slug.clone();
    item.name = project.name.clone();
    item.version = project.version.clone();
    item.slot = project.slot;

    let Some((status, catalog)) = loaded else {
        item.error = Some("No game catalog loaded, so the target check did not run.".to_string());
        return item;
    };

    item.migrations = migrate::plan_migrations(&project, catalog)
        .into_iter()
        .map(|migration| PathMove {
            from: migration.from,
            to: migration.to,
        })
        .collect();

    match project::check_project(dir, catalog, status) {
        Ok(check) => {
            item.unchanged = check.unchanged;
            item.changed = check.changed;
            item.missing = check.missing;
            item.moved = check.moved;
            item.unbaselined = check.unbaselined;
            item.warnings = review_warnings(&check);
        }
        Err(error) => item.error = Some(error.to_string()),
    }

    item
}

fn review_warnings(check: &project::ProjectCheck) -> Vec<String> {
    let mut warnings = Vec::new();

    if !check.gameinfo_ok {
        warnings.push(
            "gameinfo.gi has no `Game citadel/addons` SearchPaths line, mods will not load"
                .to_string(),
        );
    }

    if let Some(conflict) = &check.slot_conflict
        && let Some(slot) = check.slot
    {
        let file = discovery::format_slot_file_name(slot)
            .unwrap_or_else(|_| format!("pak{slot:02}_dir.vpk"));
        warnings.push(if conflict.file_name == file {
            format!("slot {file} already exists in the addon folder")
        } else {
            format!("slot {file} is already used by {}", conflict.file_name)
        });
    }

    warnings
}

fn short_stamp(value: &str) -> String {
    value.chars().take(16).collect::<String>().replace('T', " ")
}
