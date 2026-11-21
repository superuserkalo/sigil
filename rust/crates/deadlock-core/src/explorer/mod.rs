//! The file explorer behind the Game index's Files tab and the agent's browse tools: any
//! directory VPK (the game's own `pak01_dir.vpk`, an installed addon) or loose folder (a mod
//! project's staging or sources) as a tree, with previews, content search, comparison and
//! export into a mod project.
//!
//! Reads only. The one write is [`export::export`], which refuses a destination outside a mod
//! project, so nothing here can touch a game install.

pub mod compare;
pub mod dmx;
pub mod export;
pub mod kv3_text;
pub mod preview;
pub mod search;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::vpk::{VpkDirectory, read_vpk_directory, read_vpk_entry_data};

/// A folder walk stops here, so a mistaken pick (a drive root) cannot hang a worker.
pub const MAX_FOLDER_FILES: usize = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Vpk,
    Folder,
}

/// One file in a source, with its path in game form (forward slashes, no leading slash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub path: String,
    pub size: u64,
    /// The VPK tree's CRC. A folder file has none until a comparison hashes it.
    pub crc32: Option<u32>,
}

/// A row of a tree level: a folder with its file count and total size, or one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub name: String,
    pub path: String,
    pub folder: bool,
    pub size: u64,
    pub files: usize,
}

/// A source the explorer can open, as offered to the player: the game's VPKs and the installed
/// addons. Projects and picked files are added by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub label: String,
    pub path: PathBuf,
}

/// An opened source: every file it holds, sorted by path.
#[derive(Debug, Clone)]
pub struct Listing {
    pub label: String,
    pub root: PathBuf,
    pub kind: SourceKind,
    pub files: Vec<SourceFile>,
    vpk: Option<VpkDirectory>,
    by_path: HashMap<String, usize>,
}

/// Open a directory VPK (a path ending in `_dir.vpk` or `.vpk`) or a folder.
pub fn open(path: &Path) -> Result<Listing> {
    let label = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    if path.is_dir() {
        return open_folder(path, label);
    }
    if !path.is_file() {
        return Err(Error::msg(format!("{} does not exist.", path.display())));
    }
    let directory = read_vpk_directory(path)?;
    let mut files: Vec<SourceFile> = directory
        .entries
        .iter()
        .map(|entry| SourceFile {
            path: entry.path.clone(),
            size: u64::from(entry.total_length),
            crc32: Some(entry.crc32),
        })
        .collect();
    sort_files(&mut files);
    let by_path = index_of(&files);
    Ok(Listing {
        label,
        root: path.to_path_buf(),
        kind: SourceKind::Vpk,
        files,
        vpk: Some(directory),
        by_path,
    })
}

fn open_folder(root: &Path, label: String) -> Result<Listing> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(reader) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in reader.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let game_path = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            files.push(SourceFile {
                path: game_path,
                size,
                crc32: None,
            });
            if files.len() >= MAX_FOLDER_FILES {
                return Err(Error::msg(format!(
                    "{} holds more than {MAX_FOLDER_FILES} files. Pick a mod folder, not a drive.",
                    root.display()
                )));
            }
        }
    }
    sort_files(&mut files);
    let by_path = index_of(&files);
    Ok(Listing {
        label,
        root: root.to_path_buf(),
        kind: SourceKind::Folder,
        files,
        vpk: None,
        by_path,
    })
}

fn sort_files(files: &mut [SourceFile]) {
    files.sort_by(|a, b| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn index_of(files: &[SourceFile]) -> HashMap<String, usize> {
    files
        .iter()
        .enumerate()
        .map(|(i, file)| (file.path.to_ascii_lowercase(), i))
        .collect()
}

impl Listing {
    /// A listing built from memory, for tests and for callers that already hold the bytes'
    /// paths. Reads fail on it.
    pub fn from_paths(label: &str, paths: &[(&str, u64)]) -> Self {
        let mut files: Vec<SourceFile> = paths
            .iter()
            .map(|(path, size)| SourceFile {
                path: (*path).to_string(),
                size: *size,
                crc32: None,
            })
            .collect();
        sort_files(&mut files);
        let by_path = index_of(&files);
        Listing {
            label: label.to_string(),
            root: PathBuf::new(),
            kind: SourceKind::Folder,
            files,
            vpk: None,
            by_path,
        }
    }

    pub fn find(&self, path: &str) -> Option<&SourceFile> {
        self.by_path
            .get(&path.to_ascii_lowercase())
            .and_then(|&i| self.files.get(i))
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// The bytes of one file.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        if !crate::vpk::is_safe_vpk_path(path) {
            return Err(Error::msg(format!("{path} is not a safe game path.")));
        }
        match (&self.vpk, self.kind) {
            (Some(directory), _) => {
                let entry = directory
                    .by_path
                    .get(&path.to_ascii_lowercase())
                    .ok_or_else(|| Error::msg(format!("{path} is not in {}.", self.label)))?;
                read_vpk_entry_data(directory, entry)
            }
            (None, SourceKind::Folder) if !self.root.as_os_str().is_empty() => {
                if self.find(path).is_none() {
                    return Err(Error::msg(format!("{path} is not in {}.", self.label)));
                }
                Ok(fs::read(self.root.join(path))?)
            }
            _ => Err(Error::msg(format!("{} cannot be read.", self.label))),
        }
    }

    /// The direct children of a folder (`""` for the root): folders first, then files, each
    /// sorted by name. A folder row carries the count and size of everything below it.
    pub fn children(&self, folder: &str) -> Vec<Node> {
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{}/", folder.trim_end_matches('/'))
        };
        let lower_prefix = prefix.to_ascii_lowercase();
        let mut folders: Vec<Node> = Vec::new();
        let mut folder_at: HashMap<String, usize> = HashMap::new();
        let mut files = Vec::new();
        for file in &self.files {
            if !file.path.to_ascii_lowercase().starts_with(&lower_prefix) {
                continue;
            }
            let rest = &file.path[prefix.len()..];
            match rest.split_once('/') {
                Some((name, _)) => {
                    let key = name.to_ascii_lowercase();
                    let at = *folder_at.entry(key).or_insert_with(|| {
                        folders.push(Node {
                            name: name.to_string(),
                            path: format!("{prefix}{name}"),
                            folder: true,
                            size: 0,
                            files: 0,
                        });
                        folders.len() - 1
                    });
                    folders[at].size += file.size;
                    folders[at].files += 1;
                }
                None => files.push(Node {
                    name: rest.to_string(),
                    path: file.path.clone(),
                    folder: false,
                    size: file.size,
                    files: 1,
                }),
            }
        }
        folders.sort_by_key(|node| node.name.to_ascii_lowercase());
        files.sort_by_key(|node| node.name.to_ascii_lowercase());
        folders.extend(files);
        folders
    }

    /// Every file path below a folder, or the file itself.
    pub fn files_under(&self, path: &str) -> Vec<String> {
        if self.find(path).is_some() {
            return vec![path.to_string()];
        }
        let prefix = format!("{}/", path.trim_end_matches('/')).to_ascii_lowercase();
        self.files
            .iter()
            .filter(|file| path.is_empty() || file.path.to_ascii_lowercase().starts_with(&prefix))
            .map(|file| file.path.clone())
            .collect()
    }
}

/// The game's own VPKs and every installed addon, in the order the explorer lists them.
pub fn game_sources(game_root: &Path) -> Vec<SourceRef> {
    let mut sources = Vec::new();
    for (label, relative) in [
        ("Game files (citadel)", "game/citadel/pak01_dir.vpk"),
        ("Engine files (core)", "game/core/pak01_dir.vpk"),
    ] {
        let path = game_root.join(relative);
        if path.is_file() {
            sources.push(SourceRef {
                label: label.to_string(),
                path,
            });
        }
    }
    let addons = crate::deploy::addons_dir(game_root);
    if let Ok(installed) = crate::addons::list_installed(game_root) {
        for addon in installed {
            sources.push(SourceRef {
                label: format!("Addon {}", addon.file),
                path: addon.path(&addons),
            });
        }
    }
    sources
}

/// The lowercased extension after the last dot of a game path's file name.
pub fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_ascii_lowercase(),
        None => String::new(),
    }
}
