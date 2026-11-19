//! Deadworks plugins on and off: DLLs move between `managed/plugins` and `managed/plugins-off`.
//! Deadworks loads every DLL in `plugins/` and hot-reloads one replaced while the server runs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::require_approval;
use crate::error::{Error, Result};

/// `<server_dir>/game/bin/win64/managed/plugins`.
pub fn plugins_dir(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("bin").join("win64").join("managed").join("plugins")
}

/// `<server_dir>/game/bin/win64/managed/plugins-off`, where Sigil parks switched-off plugins.
pub fn plugins_off_dir(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("bin").join("win64").join("managed").join("plugins-off")
}

/// One plugin DLL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// File name without `.dll`.
    pub name: String,
    pub file_name: String,
    pub enabled: bool,
    pub size: u64,
}

fn is_dll(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"))
}

fn scan(dir: &Path, enabled: bool, out: &mut Vec<Plugin>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || !is_dll(&path) {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(Plugin { name, file_name, enabled, size: meta.len() });
    }
    Ok(())
}

/// Every plugin DLL, on and off, sorted by name.
pub fn list_plugins(server_dir: &Path) -> Result<Vec<Plugin>> {
    let mut plugins = Vec::new();
    scan(&plugins_dir(server_dir), true, &mut plugins)?;
    scan(&plugins_off_dir(server_dir), false, &mut plugins)?;
    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(b.enabled.cmp(&a.enabled)));
    Ok(plugins)
}

/// Switch a plugin on (move into `plugins/`) or off (into `plugins-off/`). `name` is the file
/// name with or without `.dll`; it may not hold a path.
pub fn set_plugin(server_dir: &Path, name: &str, on: bool, approve: bool) -> Result<Plugin> {
    require_approval(approve, if on { "Turning the plugin on" } else { "Turning the plugin off" })?;
    let name = name.trim();
    let bad = name.is_empty()
        || name.contains(['/', '\\', ':'])
        || name == "."
        || name == ".."
        || name.chars().any(char::is_control);
    if bad {
        return Err(Error::msg(format!("{name:?} is not a plugin file name.")));
    }
    let file_name = if is_dll(Path::new(name)) { name.to_string() } else { format!("{name}.dll") };
    let (from_dir, to_dir) = if on {
        (plugins_off_dir(server_dir), plugins_dir(server_dir))
    } else {
        (plugins_dir(server_dir), plugins_off_dir(server_dir))
    };
    let from = from_dir.join(&file_name);
    let to = to_dir.join(&file_name);
    let already = to.is_file();
    if !from.is_file() {
        if already {
            let size = fs::metadata(&to)?.len();
            let stem = Path::new(&file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            return Ok(Plugin { name: stem, file_name, enabled: on, size });
        }
        return Err(Error::msg(format!("No plugin named {file_name} in the server folder.")));
    }
    if already {
        return Err(Error::msg(format!(
            "{file_name} is in both plugins and plugins-off. Remove one copy first."
        )));
    }
    fs::create_dir_all(&to_dir)?;
    fs::rename(&from, &to)?;
    let size = fs::metadata(&to)?.len();
    let stem = Path::new(&file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Plugin { name: stem, file_name, enabled: on, size })
}
