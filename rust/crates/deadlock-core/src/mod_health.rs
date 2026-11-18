//! Compatibility review: does what a mod ships still exist in the game?
//!
//! Deadlock names every asset by path; a mod replaces a file by that exact path. A game update
//! that moves a folder (hero folders move most often) leaves a mod that ships the old path doing
//! nothing, silently. This compares each slot's VPK entries with the game's own `pak01` listing
//! and reports three things:
//!
//! - [`IssueKind::Moved`]: the game has the same file name under another path, so the mod is one
//!   update behind a folder move;
//! - [`IssueKind::Gone`]: no file with that name is in the game any more (a removed asset);
//! - [`IssueKind::Skeleton`]: a hero model or a game-data file whose path is gone, the pattern a
//!   stale skeleton leaves behind;
//! - [`IssueKind::Unreadable`]: the slot file could not be read at all.
//!
//! Read only. The game listing comes from the game index (`pak01_dir.vpk`), cached beside it.
//! A mod that ships only textures or sounds for paths the game keeps reports nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::game_index::{LoadGameIndexOptions, default_game_index_path, game_vpk_path, load_game_index};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Paths listed per mod in a report.
pub const MAX_ISSUES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueKind {
    /// The file name exists at another path in the game.
    Moved,
    /// The path is gone and nothing in the game has that name.
    Gone,
    /// A hero model or game data file whose path is gone: the stale skeleton case.
    Skeleton,
    /// The slot VPK could not be read.
    Unreadable,
}

impl IssueKind {
    pub fn label(self) -> &'static str {
        match self {
            IssueKind::Moved => "Moved",
            IssueKind::Gone => "Gone",
            IssueKind::Skeleton => "Old model",
            IssueKind::Unreadable => "Unreadable",
        }
    }

    pub fn line(self) -> &'static str {
        match self {
            IssueKind::Moved => "The game moved this file. The mod is one update behind.",
            IssueKind::Gone => "The game no longer has this file. The mod part is unused.",
            IssueKind::Skeleton => "An old hero model or game data file. The hero can look wrong.",
            IssueKind::Unreadable => "Sigil could not read this mod's files.",
        }
    }
}

/// One problem with one shipped path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub kind: IssueKind,
    /// Path inside the mod.
    pub path: String,
    /// Where the game has it now, for [`IssueKind::Moved`].
    pub now_at: Option<String>,
}

/// The review of one slot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotReview {
    pub slot: u16,
    pub issues: Vec<Issue>,
    /// Issues beyond [`MAX_ISSUES`].
    pub more: usize,
    /// Entries checked.
    pub entries: usize,
    /// Entries that are not in the game.
    pub missing: usize,
    /// The file could not be read.
    pub error: Option<String>,
}

impl SlotReview {
    pub fn is_clear(&self) -> bool {
        self.issues.is_empty() && self.error.is_none()
    }

    /// The worst kind found, for the order of the list.
    pub fn worst(&self) -> Option<IssueKind> {
        self.issues.iter().map(|issue| issue.kind).min()
    }

    /// Paths worth naming on the card, at most `count`.
    pub fn examples(&self, count: usize) -> Vec<&str> {
        self.issues
            .iter()
            .map(|issue| issue.path.as_str())
            .take(count)
            .collect()
    }
}

/// A slot and the VPK file to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotFile {
    pub slot: u16,
    pub path: PathBuf,
}

/// The game's own paths, from the game index. Builds (and caches) the index on first use.
pub fn game_paths(game_root: &Path, cache_dir: &Path) -> Result<BTreeSet<String>> {
    let vpk = game_vpk_path(game_root);
    let loaded = load_game_index(LoadGameIndexOptions {
        dir_vpk_path: vpk,
        cache_path: Some(default_game_index_path(cache_dir)),
        force: false,
    })?;
    Ok(loaded
        .index
        .entries
        .iter()
        .map(|entry| entry.path.to_ascii_lowercase())
        .collect())
}

/// Review the slots against a set of lowercased game paths. Pure, so the screen can call it on
/// a worker with the paths it already loaded.
pub fn review_against(slots: &[SlotFile], game_paths: &BTreeSet<String>) -> Vec<SlotReview> {
    // File name -> the one path the game has it at. A name the game has twice is no clue.
    let mut by_name: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    for path in game_paths {
        let name = path.rsplit('/').next().unwrap_or(path);
        by_name
            .entry(name)
            .and_modify(|slot| *slot = None)
            .or_insert(Some(path.as_str()));
    }

    slots
        .iter()
        .map(|slot| {
            let directory = match read_vpk_directory_with(&slot.path, &ReadVpkOptions {
                keep_preload: false,
            }) {
                Ok(directory) => directory,
                Err(error) => {
                    return SlotReview {
                        slot: slot.slot,
                        error: Some(error.to_string()),
                        ..SlotReview::default()
                    };
                }
            };
            let mut review = SlotReview {
                slot: slot.slot,
                entries: directory.entries.len(),
                ..SlotReview::default()
            };
            for entry in &directory.entries {
                let path = entry.path.to_ascii_lowercase();
                if game_paths.contains(&path) {
                    continue;
                }
                review.missing += 1;
                let name = path.rsplit('/').next().unwrap_or(&path);
                let kind = if path.contains("models/heroes")
                    || path.ends_with(".vmdl_c")
                    || path.ends_with(".vdata_c")
                    || path.ends_with(".vdata")
                {
                    IssueKind::Skeleton
                } else {
                    IssueKind::Gone
                };
                let now_at = by_name
                    .get(name)
                    .and_then(|found| *found)
                    .map(str::to_string);
                let kind = if now_at.is_some() {
                    IssueKind::Moved
                } else {
                    kind
                };
                if review.issues.len() < MAX_ISSUES {
                    review.issues.push(Issue {
                        kind,
                        path: entry.path.clone(),
                        now_at,
                    });
                } else {
                    review.more += 1;
                }
            }
            review.issues.sort_by_key(|issue| issue.kind);
            review
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};
    use std::fs;

    fn root(tag: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/health-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let staging = root.join(format!("staging-{name}"));
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        pack_vpk(&PackOptions::new(staging, root.join("out"), name))
            .unwrap()
            .dir_path
    }

    #[test]
    fn finds_moved_gone_and_stale_paths_and_leaves_current_ones_alone() {
        let root = root("review");
        let file = vpk(&root, "pak01", &[
            ("materials/models/heroes/atlas/atlas_body.vmat_c", b"still here"),
            ("materials/models/heroes/old/atlas_body2.vmat_c", b"moved"),
            ("models/heroes_wip/billy/billy.vmdl_c", b"old model"),
            ("scripts/unused.vdata_c", b"data"),
        ]);
        let game: BTreeSet<String> = [
            "materials/models/heroes/atlas/atlas_body.vmat_c",
            "materials/models/heroes/new/atlas_body2.vmat_c",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let review = review_against(&[SlotFile { slot: 3, path: file }], &game);
        assert_eq!(review.len(), 1);
        let one = &review[0];
        assert_eq!(one.entries, 4);
        assert_eq!(one.missing, 3);
        assert_eq!(one.more, 0);
        assert!(one.issues.iter().any(|issue| {
            issue.kind == IssueKind::Moved
                && issue.path.ends_with("atlas_body2.vmat_c")
                && issue.now_at.as_deref() == Some("materials/models/heroes/new/atlas_body2.vmat_c")
        }));
        assert!(one.issues.iter().any(|issue| {
            issue.kind == IssueKind::Skeleton && issue.path.ends_with("billy.vmdl_c")
        }));
        assert!(one.issues.iter().any(|issue| {
            issue.kind == IssueKind::Skeleton && issue.path.ends_with("unused.vdata_c")
        }));
        assert!(!one.issues.iter().any(|issue| issue.path.ends_with("atlas_body.vmat_c")));
        assert!(!one.is_clear());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_current_mod_is_clear_and_a_broken_file_reports() {
        let root = root("clear");
        let file = vpk(&root, "pak02", &[("sound/hero/atlas.vsnd_c", b"s")]);
        let game: BTreeSet<String> = ["sound/hero/atlas.vsnd_c".to_string()].into();
        let review = review_against(&[SlotFile { slot: 1, path: file }], &game);
        assert!(review[0].is_clear());
        assert_eq!(review[0].worst(), None);

        let broken = review_against(
            &[SlotFile { slot: 2, path: root.join("missing_dir.vpk") }],
            &game,
        );
        assert!(broken[0].error.is_some());
        assert!(!broken[0].is_clear());
        let _ = fs::remove_dir_all(&root);
    }
}
