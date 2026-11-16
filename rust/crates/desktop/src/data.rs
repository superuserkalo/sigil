//! Real reads from `deadlock-core` at startup.
//!
//! Only reads happen here: game status, the cached Steam news feed and an optional workspace
//! project named by `DAH_WORKSPACE`. A missing install or cache leaves the matching screen empty
//! instead of inventing data.

use std::path::PathBuf;

use deadlock_core::{discovery, patch_notes, project};

use crate::app::State;
use crate::model::{self, Compat, HistoryEntry, Project, ProjectState, Reference, Target};

pub fn init_state() -> State {
    let mut state = State::default();
    state.paths.data = data_dir().display().to_string();
    apply_game_status(&mut state);
    load_cached_patches(&mut state);
    load_workspace_project(&mut state);
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

fn apply_game_status(state: &mut State) {
    match discovery::read_game_status(None) {
        Ok(status) => {
            let Some(install) = &status.install else {
                state.game_detected = false;
                state.game_label = "Game not detected".to_string();
                state.status =
                    "No Deadlock install found in the Steam roots. Open Settings to see the paths searched."
                        .to_string();
                return;
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
        }
        Err(error) => {
            state.game_detected = false;
            state.game_label = "Game not detected".to_string();
            state.status = format!("Game discovery failed: {error}");
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
    if !root.join("project.json").is_file() {
        return;
    }
    let Ok(project) = project::read_project(&root) else {
        return;
    };
    state.paths.workspace = root.display().to_string();
    state.paths.staging = root.join(".dah/staging").display().to_string();
    let mapped = map_project(&project);
    let name = mapped.name.clone();
    state.projects = vec![mapped];
    state.selected_project = 0;
    if state.status.is_empty() {
        state.status = format!("Opened {name} from {}.", root.display());
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
