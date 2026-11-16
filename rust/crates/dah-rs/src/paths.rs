//! Data directory and workspace resolution shared by every command.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{CliError, Result};

/// Resolved application paths. The workspace defaults under the data directory
/// and can be overridden per command.
#[derive(Debug, Clone)]
pub struct CliPaths {
    pub data_dir: PathBuf,
    pub workspace_dir: PathBuf,
}

pub fn resolve_paths(workspace_override: Option<&Path>) -> Result<CliPaths> {
    let data_dir = data_dir()?;
    let workspace_dir = match workspace_override {
        Some(path) => absolute(path)?,
        None => match env_value("DAH_WORKSPACE_DIR") {
            Some(value) => absolute(Path::new(&value))?,
            None => data_dir.join("workspace"),
        },
    };

    Ok(CliPaths {
        data_dir,
        workspace_dir,
    })
}

/// The data directory follows the platform convention and `DAH_DATA_DIR`.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(value) = env_value("DAH_DATA_DIR") {
        return absolute(Path::new(&value));
    }

    let base = dirs::data_dir().ok_or_else(|| {
        CliError::new(
            "data_dir",
            "Could not resolve the platform data directory. Set DAH_DATA_DIR.",
        )
    })?;

    let leaf = if cfg!(target_os = "linux") {
        "deadlock-agent-harness"
    } else {
        "DeadlockAgentHarness"
    };

    Ok(base.join(leaf))
}

/// Resolve the parent workspace for `dah mod new`.
pub fn workspace_root(workspace_override: Option<&Path>) -> Result<PathBuf> {
    resolve_paths(workspace_override).map(|paths| paths.workspace_dir)
}

/// Resolve the mod project directory for `check`, `baseline` and `version`.
///
/// A directory that directly holds `project.json` is the project. Otherwise the
/// workspace root is searched for `mods/<slug>/project.json`.
pub fn project_dir(workspace_override: Option<&Path>) -> Result<PathBuf> {
    let workspace = workspace_root(workspace_override)?;
    resolve_project_in(&workspace)
}

fn resolve_project_in(workspace: &Path) -> Result<PathBuf> {
    if workspace.join("project.json").is_file() {
        return Ok(workspace.to_path_buf());
    }

    let mods_dir = workspace.join("mods");
    let mut found: Vec<PathBuf> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&mods_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("project.json").is_file() {
                found.push(path);
            }
        }
    }

    found.sort();

    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(CliError::new(
            "no_project",
            format!(
                "No mod project found under {}. Run `dah mod new <slug>` first, or pass --workspace with the project directory.",
                workspace.display()
            ),
        )),
        _ => {
            let slugs: Vec<String> = found
                .iter()
                .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
                .collect();
            Err(CliError::new(
                "ambiguous_project",
                format!(
                    "More than one mod project is under {}. Pass --workspace with one of: {}.",
                    workspace.display(),
                    slugs.join(", ")
                ),
            ))
        }
    }
}

pub fn absolute(path: &Path) -> Result<PathBuf> {
    let expanded = match path.to_str() {
        Some(text) if text == "~" || text.starts_with("~/") => {
            PathBuf::from(expand_tilde(text))
        }
        _ => path.to_path_buf(),
    };

    let joined = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|error| {
                CliError::new(
                    "path",
                    format!("Could not read the current directory: {error}"),
                )
            })?
            .join(expanded)
    };

    Ok(normalize_lexical(&joined))
}

/// Resolve `.` and `..` without touching the filesystem, so containment checks
/// cannot be bypassed by a path that only lexically looks inside a directory.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut output = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }

    output
}

/// True when `candidate` is inside `root` (or equal to it).
pub fn is_inside(root: &Path, candidate: &Path) -> bool {
    candidate == root || candidate.starts_with(root)
}

fn env_value(name: &str) -> Option<String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(expand_tilde(value.trim())),
        _ => None,
    }
}

pub fn expand_tilde(value: &str) -> String {
    if value == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
        return value.to_string();
    }

    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into_owned();
    }

    value.to_string()
}
