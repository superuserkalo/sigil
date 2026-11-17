//! Deadlock install discovery, gameinfo SearchPaths and addon slot status.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::absolute;

pub const DEADLOCK_APP_ID: &str = "1422450";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeadlockInstall {
    pub steam_root: PathBuf,
    pub game_root: PathBuf,
    pub citadel_dir: PathBuf,
    pub gameinfo_path: PathBuf,
    pub addons_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameInfoStatus {
    pub present: bool,
    pub addons_search_path: bool,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonSlot {
    pub slot: u8,
    pub file_name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddonsStatus {
    pub directory: PathBuf,
    pub present: bool,
    pub slots: Vec<AddonSlot>,
    pub chunk_files: Vec<String>,
    pub ignored_files: Vec<String>,
    pub next_free_slot: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallDiscovery {
    pub install: Option<DeadlockInstall>,
    pub searched_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    pub install: Option<DeadlockInstall>,
    pub searched_roots: Vec<PathBuf>,
    pub gameinfo: Option<GameInfoStatus>,
    pub addons: Option<AddonsStatus>,
}

#[derive(Debug, Clone, Default)]
pub struct InstallDiscoveryOptions {
    pub steam_roots: Option<Vec<PathBuf>>,
    /// Deadlock game folder, its `game` folder, or `game/citadel`. Overrides Steam discovery.
    pub game_dir: Option<PathBuf>,
    pub platform: Option<String>,
    pub home: Option<PathBuf>,
    /// When set, replaces the process environment lookup of `DAH_GAME_DIR`.
    pub env_game_dir: Option<PathBuf>,
}

pub fn parse_library_folders(vdf: &str) -> Vec<String> {
    let pattern = Regex::new(r#""path"\s+"((?:[^"\\]|\\.)*)""#).expect("vdf pattern is valid");
    let mut libraries = Vec::new();

    for captures in pattern.captures_iter(vdf) {
        let Some(raw) = captures.get(1) else {
            continue;
        };
        if raw.as_str().is_empty() {
            continue;
        }
        libraries.push(unescape_vdf(raw.as_str()));
    }

    libraries
}

pub fn default_steam_roots() -> Vec<PathBuf> {
    default_steam_roots_for(std::env::consts::OS, home_dir().as_deref(), |name| {
        std::env::var(name).ok()
    })
}

pub fn default_steam_roots_for(
    platform: &str,
    home: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<PathBuf> {
    match platform {
        "macos" => home
            .map(|home| {
                vec![
                    home.join("Library")
                        .join("Application Support")
                        .join("Steam"),
                ]
            })
            .unwrap_or_default(),
        "windows" => {
            let program_files_x86 =
                env("ProgramFiles(x86)").unwrap_or_else(|| "C:\\Program Files (x86)".to_string());
            let program_files =
                env("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".to_string());
            vec![
                PathBuf::from(program_files_x86).join("Steam"),
                PathBuf::from(program_files).join("Steam"),
            ]
        }
        _ => home
            .map(|home| {
                vec![
                    home.join(".steam").join("steam"),
                    home.join(".steam").join("root"),
                    home.join(".local").join("share").join("Steam"),
                ]
            })
            .unwrap_or_default(),
    }
}

pub fn format_slot_file_name(slot: u8) -> Result<String> {
    if !(1..=99).contains(&slot) {
        return Err(Error::msg(format!(
            "Addon slot must be an integer from 1 to 99, received {slot}"
        )));
    }

    Ok(format!("pak{slot:02}_dir.vpk"))
}

pub fn discover_deadlock_install(options: InstallDiscoveryOptions) -> Result<InstallDiscovery> {
    let configured_game_dir = options
        .game_dir
        .clone()
        .or(options.env_game_dir.clone())
        .or_else(|| std::env::var_os("DAH_GAME_DIR").map(PathBuf::from))
        .filter(|path| !path.as_os_str().is_empty());

    if let Some(configured) = configured_game_dir {
        let configured = absolute(&configured);
        let parent = configured.parent().map(Path::to_path_buf);
        let grandparent = parent
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf);

        let candidates = [
            (configured.clone(), configured.join("game").join("citadel")),
            match parent {
                Some(parent) => (parent, configured.join("citadel")),
                None => (configured.clone(), configured.join("citadel")),
            },
            match grandparent {
                Some(grandparent) => (grandparent, configured.clone()),
                None => (configured.clone(), configured.clone()),
            },
        ];

        for (game_root, citadel_dir) in candidates {
            if is_directory(&citadel_dir)? {
                return Ok(InstallDiscovery {
                    install: Some(install_at(game_root, citadel_dir)),
                    searched_roots: vec![configured.clone()],
                });
            }
        }

        return Ok(InstallDiscovery {
            install: None,
            searched_roots: vec![configured],
        });
    }

    let seed_roots = match options.steam_roots {
        Some(roots) => roots,
        None => {
            let fallback_home = home_dir();
            let home = options.home.as_deref().or(fallback_home.as_deref());
            default_steam_roots_for(
                options.platform.as_deref().unwrap_or(std::env::consts::OS),
                home,
                |name| std::env::var(name).ok(),
            )
        }
    };

    let mut searched_roots: Vec<PathBuf> = Vec::new();

    for root in seed_roots {
        for candidate in expand_steam_root(&root)? {
            if !searched_roots.contains(&candidate) {
                searched_roots.push(candidate);
            }
        }
    }

    for steam_root in &searched_roots {
        let game_root = steam_root.join("steamapps").join("common").join("Deadlock");
        let citadel_dir = game_root.join("game").join("citadel");

        if is_directory(&citadel_dir)? {
            return Ok(InstallDiscovery {
                install: Some(install_at(game_root, citadel_dir)),
                searched_roots,
            });
        }
    }

    Ok(InstallDiscovery {
        install: None,
        searched_roots,
    })
}

pub fn read_game_info_status(gameinfo_path: &Path) -> Result<GameInfoStatus> {
    let text = match fs::read_to_string(gameinfo_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GameInfoStatus {
                present: false,
                addons_search_path: false,
                line: None,
            });
        }
        Err(error) => return Err(error.into()),
    };

    let pattern =
        Regex::new(r"(?i)^\s*Game\s+citadel[\\/]addons\s*$").expect("gameinfo pattern is valid");

    for (index, line) in text.lines().enumerate() {
        let without_comment = match line.find("//") {
            Some(position) => &line[..position],
            None => line,
        };

        if pattern.is_match(without_comment) {
            return Ok(GameInfoStatus {
                present: true,
                addons_search_path: true,
                line: Some(index + 1),
            });
        }
    }

    // A Sigil profile with its own folder replaces the plain line with a marked block that
    // mounts `citadel/addons/sigil-<slug>`. Mods load from there, so the search path is set.
    if crate::profiles::profile_block_folder(&text).is_some() {
        let line = text
            .lines()
            .position(|line| line.trim() == crate::profiles::PROFILE_BLOCK_BEGIN)
            .map(|index| index + 1);
        return Ok(GameInfoStatus {
            present: true,
            addons_search_path: true,
            line,
        });
    }

    Ok(GameInfoStatus {
        present: true,
        addons_search_path: false,
        line: None,
    })
}

pub fn read_addons_status(addons_dir: &Path) -> Result<AddonsStatus> {
    let read_dir = match fs::read_dir(addons_dir) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AddonsStatus {
                directory: addons_dir.to_path_buf(),
                present: false,
                slots: Vec::new(),
                chunk_files: Vec::new(),
                ignored_files: Vec::new(),
                next_free_slot: Some(1),
            });
        }
        Err(error) => return Err(error.into()),
    };

    let slot_pattern = Regex::new(r"(?i)^pak(\d{2})_dir\.vpk$").expect("slot pattern is valid");
    let chunk_pattern = Regex::new(r"(?i)^pak\d{2}_\d{3}\.vpk$").expect("chunk pattern is valid");
    let mut slots = Vec::new();
    let mut chunk_files = Vec::new();
    let mut ignored_files = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if !path.is_file() {
            ignored_files.push(format!("{name}/"));
            continue;
        }

        if let Some(captures) = slot_pattern.captures(&name) {
            if let Some(value) = captures.get(1) {
                if let Ok(slot) = value.as_str().parse::<u8>() {
                    if (1..=99).contains(&slot) {
                        let size_bytes = fs::metadata(&path).map(|data| data.len()).unwrap_or(0);
                        slots.push(AddonSlot {
                            slot,
                            file_name: name,
                            size_bytes,
                        });
                        continue;
                    }
                }
            }
        }

        if chunk_pattern.is_match(&name) {
            chunk_files.push(name);
            continue;
        }

        ignored_files.push(name);
    }

    slots.sort_by_key(|slot| slot.slot);
    chunk_files.sort();
    ignored_files.sort();

    let used: Vec<u8> = slots.iter().map(|slot| slot.slot).collect();
    let next_free_slot = (1..=99).find(|slot| !used.contains(slot));

    Ok(AddonsStatus {
        directory: addons_dir.to_path_buf(),
        present: true,
        slots,
        chunk_files,
        ignored_files,
        next_free_slot,
    })
}

/// Read install status. `game_dir` overrides Steam discovery, then `DAH_GAME_DIR`.
pub fn read_game_status(game_dir: Option<PathBuf>) -> Result<GameStatus> {
    let discovery = discover_deadlock_install(InstallDiscoveryOptions {
        game_dir,
        ..InstallDiscoveryOptions::default()
    })?;

    let Some(install) = discovery.install else {
        return Ok(GameStatus {
            install: None,
            searched_roots: discovery.searched_roots,
            gameinfo: None,
            addons: None,
        });
    };

    let gameinfo = read_game_info_status(&install.gameinfo_path)?;
    let addons = read_addons_status(&install.addons_dir)?;

    Ok(GameStatus {
        install: Some(install),
        searched_roots: discovery.searched_roots,
        gameinfo: Some(gameinfo),
        addons: Some(addons),
    })
}

pub fn format_game_status(status: &GameStatus) -> String {
    let mut lines = Vec::new();

    let Some(install) = &status.install else {
        lines.push("Deadlock install: not found".to_string());
        lines.push("Searched Steam roots:".to_string());
        for root in &status.searched_roots {
            lines.push(format!("  {}", root.display()));
        }
        lines.push("Install Deadlock through Steam or pass a steam root explicitly.".to_string());
        return lines.join("\n");
    };

    lines.push(format!("Deadlock install: {}", install.game_root.display()));

    match &status.gameinfo {
        None => lines.push("gameinfo.gi: missing".to_string()),
        Some(gameinfo) if !gameinfo.present => lines.push("gameinfo.gi: missing".to_string()),
        Some(gameinfo) if gameinfo.addons_search_path => lines.push(format!(
            "gameinfo.gi: addons SearchPaths present at line {}",
            gameinfo
                .line
                .map(|line| line.to_string())
                .unwrap_or_else(|| "?".to_string())
        )),
        Some(_) => lines.push(
            "gameinfo.gi: addons SearchPaths missing, mods will not load until repaired"
                .to_string(),
        ),
    }

    match &status.addons {
        None => lines.push("Addon folder: missing".to_string()),
        Some(addons) if !addons.present => lines.push("Addon folder: missing".to_string()),
        Some(addons) if addons.slots.is_empty() => lines.push(format!(
            "Addon folder: {} (no slots used)",
            addons.directory.display()
        )),
        Some(addons) => {
            let used: Vec<String> = addons
                .slots
                .iter()
                .map(|slot| format!("{:02}", slot.slot))
                .collect();
            lines.push(format!("Addon slots in use: {}", used.join(", ")));
        }
    }

    if let Some(addons) = &status.addons {
        match addons.next_free_slot {
            Some(slot) => lines.push(format!(
                "Next free slot: {}",
                format_slot_file_name(slot).unwrap_or_default()
            )),
            None => lines.push("Next free slot: none, all 99 slots are used".to_string()),
        }

        if !addons.ignored_files.is_empty() {
            lines.push(format!(
                "Ignored files: {}",
                addons.ignored_files.join(", ")
            ));
        }
        if !addons.chunk_files.is_empty() {
            lines.push(format!("Chunk files: {}", addons.chunk_files.join(", ")));
        }
    }

    lines.join("\n")
}

fn install_at(game_root: PathBuf, citadel_dir: PathBuf) -> DeadlockInstall {
    DeadlockInstall {
        steam_root: game_root.clone(),
        gameinfo_path: game_root.join("game").join("citadel").join("gameinfo.gi"),
        addons_dir: game_root.join("game").join("citadel").join("addons"),
        game_root,
        citadel_dir,
    }
}

fn expand_steam_root(root: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = vec![root.to_path_buf()];
    let library_file = root.join("steamapps").join("libraryfolders.vdf");

    match fs::read_to_string(&library_file) {
        Ok(vdf) => {
            for library in parse_library_folders(&vdf) {
                let library = PathBuf::from(library);
                if !roots.contains(&library) {
                    roots.push(library);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    Ok(roots)
}

fn unescape_vdf(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();

    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some(next @ ('"' | '\\')) => output.push(next),
                Some(next) => {
                    output.push('\\');
                    output.push(next);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(character);
        }
    }

    output
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn is_directory(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod profile_block_tests {
    use super::read_game_info_status;

    #[test]
    fn a_mounted_profile_block_counts_as_the_addons_search_path() {
        let dir = std::env::temp_dir().join(format!("dah-gi-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gameinfo.gi");
        std::fs::write(
            &path,
            "SearchPaths\n{\n\t// sigil:profile-begin\n\tGame\t\tcitadel/addons/sigil-ranked\n\t// sigil:profile-end\n\tGame\t\tcitadel\n}\n",
        )
        .unwrap();
        let status = read_game_info_status(&path).unwrap();
        assert!(status.addons_search_path);
        assert_eq!(status.line, Some(3));
        std::fs::write(&path, "SearchPaths\n{\n\tGame\t\tcitadel\n}\n").unwrap();
        assert!(!read_game_info_status(&path).unwrap().addons_search_path);
        std::fs::remove_dir_all(&dir).ok();
    }
}

