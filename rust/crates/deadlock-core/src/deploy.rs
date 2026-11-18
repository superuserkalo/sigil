//! Deploy planning and the approved copy into `game/citadel/addons`.
//!
//! Planning is read only. `apply_deploy` is the only function in the core that
//! writes into a game install and it refuses unless the caller passes an
//! approval flag and a plan that still matches addons on disk. The only game
//! files touched are `pakNN_dir.vpk`, its chunk files and the `SearchPaths`
//! block of `gameinfo.gi`.
//!
//! Lower slot wins when two mods ship the same path, so planning reports the
//! lower slots that already contain a release entry.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::discovery::{
    format_slot_file_name, read_game_info_status, read_addons_status, GameInfoStatus,
};
use crate::error::{Error, Result};
use crate::release::{ReleaseManifest, validate_release_file_name};
use crate::util::now_iso;
use crate::vpk::{ReadVpkOptions, VpkDirectory, read_vpk_directory_with};

pub const GAMEINFO_ADDONS_LINE: &str = "Game citadel/addons";
pub const GAMEINFO_CITADEL_LINE: &str = "Game citadel";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeployConflict {
    /// Existing slot, always lower than the target slot.
    pub slot: u8,
    /// Existing addon file name.
    pub file: String,
    /// Release entries that the existing slot already ships.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeployPlan {
    pub slot: u8,
    pub target: PathBuf,
    pub chunk_targets: Vec<PathBuf>,
    pub replace_required: bool,
    pub conflicts: Vec<DeployConflict>,
    pub gameinfo_repair: bool,
    pub warnings: Vec<String>,
    /// Game paths the release VPK ships, sorted.
    pub entries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeployResult {
    pub deployed: bool,
    pub target: PathBuf,
    /// Backups written before replacing an existing file or gameinfo.gi.
    pub backups: Vec<PathBuf>,
    pub gameinfo_repaired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameinfoRepair {
    pub changed: bool,
    pub backup: Option<PathBuf>,
    /// One based line number of the addons SearchPath after the repair.
    pub line: Option<usize>,
}

pub fn addons_dir(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("addons")
}

pub fn gameinfo_path(game_root: &Path) -> PathBuf {
    game_root
        .join("game")
        .join("citadel")
        .join("gameinfo.gi")
}

/// Plan a deploy. Reads addons and the release VPK, writes nothing.
///
/// `release_dir` is the packed release folder (`<workspace>/builds/<version>`).
/// The release VPK is needed here: conflicts are computed from its entry list,
/// and a missing VPK becomes a warning instead of a false "all clear".
pub fn plan_deploy(
    game_root: &Path,
    release_dir: &Path,
    manifest: &ReleaseManifest,
    slot: Option<u8>,
) -> Result<DeployPlan> {
    let mut warnings = Vec::new();

    let slot = match slot {
        Some(requested) if (1..=99).contains(&requested) => requested,
        Some(requested) => {
            warnings.push(format!(
                "Requested slot {requested} is outside 1..=99. Using the manifest slot {}.",
                manifest.slot
            ));
            manifest.slot
        }
        None => manifest.slot,
    };

    if !(1..=99).contains(&slot) {
        return Err(Error::msg(format!(
            "Deploy slot must be from 1 to 99, received {slot}."
        )));
    }

    if slot != manifest.slot {
        warnings.push(format!(
            "Deploying release {} to slot {slot} instead of its recorded slot {}.",
            manifest.version, manifest.slot
        ));

        if !manifest.chunk_files.is_empty() {
            warnings.push(format!(
                "Chunk files are named for slot {:02}. Pack the release for slot {slot:02} before deploying.",
                manifest.slot
            ));
        }
    }

    let addons = addons_dir(game_root);
    let target = addons.join(format_slot_file_name(slot)?);

    let mut chunk_targets = Vec::with_capacity(manifest.chunk_files.len());
    for chunk in &manifest.chunk_files {
        validate_release_file_name(chunk)?;
        chunk_targets.push(addons.join(chunk));
    }

    if !addons.is_dir() {
        warnings.push(format!(
            "Addon folder {} does not exist yet. Deploy creates it.",
            addons.display()
        ));
    }

    let replace_required = target.is_file() || chunk_targets.iter().any(|path| path.is_file());

    let release_vpk = manifest.vpk_path(release_dir);
    let release_present = release_vpk.is_file();

    if !release_present {
        warnings.push(format!(
            "Release VPK {} is missing. Deploy refuses until it exists.",
            release_vpk.display()
        ));
    }

    for chunk_path in manifest.chunk_paths(release_dir) {
        if !chunk_path.is_file() {
            warnings.push(format!(
                "Release chunk {} is missing. Deploy refuses until it exists.",
                chunk_path.display()
            ));
        }
    }

    let mut entries = Vec::new();
    if release_present {
        let directory = read_vpk_directory_with(&release_vpk, &ReadVpkOptions {
            keep_preload: false,
        })?;
        entries = directory.by_path.keys().cloned().collect();
        entries.sort();
    } else {
        warnings.push(
            "Release entries are unknown because the release VPK is missing.".to_string(),
        );
    }

    let conflicts = scan_conflicts(&addons, slot, &entries, &mut warnings)?;

    let gameinfo_file = gameinfo_path(game_root);
    let gameinfo = read_game_info_status(&gameinfo_file)?;
    let gameinfo_repair = !(gameinfo.present && gameinfo.addons_search_path);

    if !gameinfo.present {
        warnings.push(format!(
            "{} is missing. SearchPaths cannot be repaired until Steam restores it.",
            gameinfo_file.display()
        ));
    }

    Ok(DeployPlan {
        slot,
        target,
        chunk_targets,
        replace_required,
        conflicts,
        gameinfo_repair,
        warnings,
        entries,
    })
}

/// Copy an approved release into addons and repair gameinfo when planned.
///
/// Refuses when `approve` is false, when the plan no longer matches addons on
/// disk, or when a packed release file is missing.
pub fn apply_deploy(
    game_root: &Path,
    release_dir: &Path,
    manifest: &ReleaseManifest,
    plan: &DeployPlan,
    approve: bool,
    backup_root: &Path,
) -> Result<DeployResult> {
    let addons = addons_dir(game_root);

    if !approve {
        return Err(Error::msg(format!(
            "Deploy to {} was not approved. Nothing was written.",
            plan.target.display()
        )));
    }

    if plan.slot != manifest.slot && !manifest.chunk_files.is_empty() {
        return Err(Error::msg(format!(
            "Release {} was packed for slot {:02} and its chunk files will not load in slot {:02}. Pack the release for the requested slot.",
            manifest.version, manifest.slot, plan.slot
        )));
    }

    if plan.gameinfo_repair {
        let status = read_game_info_status(&gameinfo_path(game_root))?;
        if !status.present {
            return Err(Error::msg(format!(
                "{} is missing. Restore it through Steam before deploying. Nothing was written.",
                gameinfo_path(game_root).display()
            )));
        }
    }

    if !(1..=99).contains(&plan.slot) {
        return Err(Error::msg(format!(
            "Deploy plan slot must be from 1 to 99, received {}.",
            plan.slot
        )));
    }

    let expected_target = addons.join(format_slot_file_name(plan.slot)?);
    if plan.target != expected_target {
        return Err(Error::msg(format!(
            "Deploy plan target {} does not match slot {} in {}.",
            plan.target.display(),
            plan.slot,
            addons.display()
        )));
    }

    for chunk_target in &plan.chunk_targets {
        if chunk_target.parent() != Some(addons.as_path()) {
            return Err(Error::msg(format!(
                "Deploy plan chunk {} is outside {}.",
                chunk_target.display(),
                addons.display()
            )));
        }
    }

    let target_exists = plan.target.is_file();
    let chunk_exists = plan.chunk_targets.iter().any(|path| path.is_file());

    if plan.replace_required != (target_exists || chunk_exists) {
        return Err(Error::msg(
            "Deploy plan is stale: addons changed since planning. Plan the deploy again.",
        ));
    }

    let release_vpk = manifest.vpk_path(release_dir);
    if !release_vpk.is_file() {
        return Err(Error::msg(format!(
            "Release VPK {} does not exist. Pack the release first.",
            release_vpk.display()
        )));
    }

    for chunk_path in manifest.chunk_paths(release_dir) {
        if !chunk_path.is_file() {
            return Err(Error::msg(format!(
                "Release chunk {} does not exist. Pack the release first.",
                chunk_path.display()
            )));
        }
    }

    fs::create_dir_all(&addons)?;

    let mut backups = Vec::new();
    let mut existing = Vec::new();
    if target_exists {
        existing.push(plan.target.clone());
    }
    for chunk_target in &plan.chunk_targets {
        if chunk_target.is_file() {
            existing.push(chunk_target.clone());
        }
    }

    if !existing.is_empty() {
        let backup_dir = backup_root.join(backup_stamp());
        fs::create_dir_all(&backup_dir)?;

        for source in existing {
            let name = source.file_name().ok_or_else(|| {
                Error::msg(format!(
                    "Cannot back up {}: it has no file name.",
                    source.display()
                ))
            })?;
            let destination = backup_dir.join(name);
            fs::copy(&source, &destination)?;
            backups.push(destination);
        }
    }

    copy_into_place(&release_vpk, &plan.target)?;

    for (chunk_file, chunk_target) in manifest.chunk_files.iter().zip(&plan.chunk_targets) {
        copy_into_place(&release_dir.join(chunk_file), chunk_target)?;
    }

    let mut gameinfo_repaired = false;

    if plan.gameinfo_repair {
        let repair = repair_gameinfo(&gameinfo_path(game_root), true)?;
        gameinfo_repaired = repair.changed;
        if let Some(backup) = repair.backup {
            backups.push(backup);
        }
    }

    Ok(DeployResult {
        deployed: true,
        target: plan.target.clone(),
        backups,
        gameinfo_repaired,
    })
}

/// Restore the addons SearchPath above `Game citadel`.
///
/// Only that one line is inserted. Every other byte of gameinfo.gi is kept and
/// the original is copied aside before the write.
pub fn repair_gameinfo(gameinfo_path: &Path, approve: bool) -> Result<GameinfoRepair> {
    if !approve {
        return Err(Error::msg(format!(
            "Repair of {} was not approved. Nothing was written.",
            gameinfo_path.display()
        )));
    }

    let status: GameInfoStatus = read_game_info_status(gameinfo_path)?;

    if !status.present {
        return Err(Error::msg(format!(
            "{} does not exist. Restore it through Steam before repairing SearchPaths.",
            gameinfo_path.display()
        )));
    }

    if status.addons_search_path && status.missing_paths.is_empty() {
        return Ok(GameinfoRepair {
            changed: false,
            backup: None,
            line: status.line,
        });
    }

    let text = fs::read_to_string(gameinfo_path)?;
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let citadel_pattern =
        Regex::new(r"(?i)^\s*Game\s+citadel\s*$").expect("gameinfo citadel pattern is valid");
    let code = |raw: &str| -> String {
        let line = raw.trim_end_matches(['\r', '\n']);
        line.find("//").map_or(line, |at| &line[..at]).to_string()
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();

    let Some(citadel_at) = lines.iter().position(|raw| citadel_pattern.is_match(&code(raw))) else {
        return Err(Error::msg(format!(
            "{} has no `{GAMEINFO_CITADEL_LINE}` SearchPath line to insert above. Refusing to guess.",
            gameinfo_path.display()
        )));
    };
    let citadel_line = lines[citadel_at].trim_end_matches(['\r', '\n']);
    let indent = &citadel_line[..citadel_line.len() - citadel_line.trim_start().len()];

    // Above `Game citadel`: the addons line, then the language mounts (they must precede the
    // base game). Before the block's closing brace: the Mod and Write paths.
    let missing = |key: &str, path: &str| status.missing_paths.iter().any(|m| m == &format!("{key} {path}"));
    let mut above: Vec<String> = Vec::new();
    if !status.addons_search_path {
        above.push(GAMEINFO_ADDONS_LINE.to_string());
    }
    let mut closing: Vec<String> = Vec::new();
    for (key, path) in crate::discovery::REQUIRED_SEARCH_PATHS {
        if !missing(key, path) {
            continue;
        }
        let line = format!("{key}\t\t{path}");
        if key.starts_with("Game_") {
            above.push(line);
        } else {
            closing.push(line);
        }
    }
    let block_end = crate::discovery::search_paths_block(&text).map(|(_, end)| end);
    if !closing.is_empty() && block_end.is_none_or(|end| end <= citadel_at) {
        return Err(Error::msg(format!(
            "{} has no SearchPaths block Sigil can read to add the Mod and Write paths. Refusing to guess.",
            gameinfo_path.display()
        )));
    }

    let mut output = String::with_capacity(text.len() + 256);
    for (index, raw_line) in lines.iter().enumerate() {
        if index == citadel_at {
            for line in &above {
                output.push_str(indent);
                output.push_str(line);
                output.push_str(ending);
            }
        }
        if Some(index) == block_end {
            for line in &closing {
                output.push_str(indent);
                output.push_str(line);
                output.push_str(ending);
            }
        }
        output.push_str(raw_line);
    }

    let line = if status.addons_search_path { status.line } else { Some(citadel_at + 1) };
    let backup_path = backup_path_for(gameinfo_path)?;
    fs::copy(gameinfo_path, &backup_path)?;
    crate::util::write_atomic(gameinfo_path, output.as_bytes())?;

    Ok(GameinfoRepair {
        changed: true,
        backup: Some(backup_path),
        line,
    })
}

fn scan_conflicts(
    addons: &Path,
    target_slot: u8,
    entries: &[String],
    warnings: &mut Vec<String>,
) -> Result<Vec<DeployConflict>> {
    if !addons.is_dir() || entries.is_empty() {
        return Ok(Vec::new());
    }

    let addons_status = read_addons_status(addons)?;
    let mut grouped: BTreeMap<(u8, String), Vec<String>> = BTreeMap::new();

    for addon in &addons_status.slots {
        if addon.slot >= target_slot {
            continue;
        }

        let path = addons.join(&addon.file_name);
        let directory: VpkDirectory =
            match read_vpk_directory_with(&path, &ReadVpkOptions { keep_preload: false }) {
                Ok(directory) => directory,
                Err(error) => {
                    warnings.push(format!(
                        "Existing addon {} could not be read, so its conflicts are unknown: {error}",
                        addon.file_name
                    ));
                    continue;
                }
            };

        for entry in entries {
            if directory.by_path.contains_key(entry) {
                grouped
                    .entry((addon.slot, addon.file_name.clone()))
                    .or_default()
                    .push(entry.clone());
            }
        }
    }

    Ok(grouped
        .into_iter()
        .map(|((slot, file), paths)| DeployConflict { slot, file, paths })
        .collect())
}

fn copy_into_place(source: &Path, destination: &Path) -> Result<()> {
    let name = destination
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| {
            Error::msg(format!(
                "Cannot copy to {}: it has no file name.",
                destination.display()
            ))
        })?;
    let temporary = destination.with_file_name(format!("{name}.dah-tmp"));

    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }

    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }

    Ok(())
}

fn backup_path_for(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| {
            Error::msg(format!(
                "Cannot back up {}: it has no file name.",
                path.display()
            ))
        })?;

    Ok(path.with_file_name(format!("{name}.dah-backup-{}", backup_stamp())))
}

fn backup_stamp() -> String {
    format!("{}-{}", now_iso().replace([':', '.'], "-"), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::{
        release_dir, release_manifest_path, latest_release, read_release_manifest,
        write_release_manifest, ReleaseManifest,
    };
    use crate::vpk::VPK_DIRECTORY_ARCHIVE;

    fn cstring(value: &str, output: &mut Vec<u8>) {
        output.extend_from_slice(value.as_bytes());
        output.push(0);
    }

    fn write_directory_vpk(path: &Path, type_name: &str, directory: &str, stems: &[&str]) {
        let mut tree = Vec::new();
        cstring(type_name, &mut tree);
        cstring(directory, &mut tree);

        for stem in stems {
            cstring(stem, &mut tree);
            let start = tree.len();
            tree.extend_from_slice(&[0u8; 18]);
            tree[start + 6..start + 8]
                .copy_from_slice(&VPK_DIRECTORY_ARCHIVE.to_le_bytes());
            tree[start + 16..start + 18].copy_from_slice(&0xffff_u16.to_le_bytes());
        }

        cstring("", &mut tree);
        cstring("", &mut tree);
        cstring("", &mut tree);

        let mut header = Vec::new();
        header.extend_from_slice(&crate::vpk::VPK_MAGIC.to_le_bytes());
        header.extend_from_slice(&2_u32.to_le_bytes());
        header.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, [header, tree].concat()).unwrap();
    }

    #[test]
    fn plans_and_applies_an_approved_deploy_with_conflict_and_gameinfo_repair() {
        let root = std::env::temp_dir().join(format!("dah-deploy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let game_root = root.join("game-root");
        let addons = addons_dir(&game_root);
        let gameinfo = gameinfo_path(&game_root);

        let hero_path = "materials/heroes/billy/billy_color";
        let type_name = "vmat_c";
        let shipped = format!("{hero_path}.{type_name}");

        write_directory_vpk(
            &addons.join("pak05_dir.vpk"),
            type_name,
            "materials/heroes/billy",
            &["billy_color"],
        );

        fs::create_dir_all(gameinfo.parent().unwrap()).unwrap();
        fs::write(
            &gameinfo,
            "// Valve gameinfo\nSearchPaths\n{\n\tGame citadel\n\tGame core\n}\n",
        )
        .unwrap();

        let workspace = root.join("workspace");
        let version = "1.2.0";
        let release = release_dir(&workspace, version);
        write_directory_vpk(
            &release.join("pak07_dir.vpk"),
            type_name,
            "materials/heroes/billy",
            &["billy_color"],
        );

        let mut manifest = ReleaseManifest::new("billy", "Billy Recolor", version, 7, "pak07_dir.vpk");
        manifest.entries = 1;
        manifest.total_bytes = 0;
        manifest.notes = Some("test release".to_string());

        let manifest_path = release_manifest_path(&workspace, version);
        write_release_manifest(&manifest_path, &manifest).unwrap();
        assert_eq!(read_release_manifest(&manifest_path).unwrap(), manifest);
        assert_eq!(latest_release(&workspace).unwrap(), Some(manifest.clone()));

        let plan = plan_deploy(&game_root, &release, &manifest, None).unwrap();
        assert_eq!(plan.slot, 7);
        assert_eq!(plan.target, addons.join("pak07_dir.vpk"));
        assert!(!plan.replace_required);
        assert!(plan.gameinfo_repair);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].slot, 5);
        assert_eq!(plan.conflicts[0].file, "pak05_dir.vpk");
        assert_eq!(plan.conflicts[0].paths, vec![shipped.clone()]);
        assert_eq!(plan.entries, vec![shipped]);

        let backup_root = root.join("backups");
        let refused = apply_deploy(&game_root, &release, &manifest, &plan, false, &backup_root);
        assert!(refused.is_err());
        assert!(!plan.target.exists());

        let result = apply_deploy(&game_root, &release, &manifest, &plan, true, &backup_root).unwrap();
        assert!(result.deployed);
        assert!(result.gameinfo_repaired);
        assert!(plan.target.is_file());
        assert_eq!(result.backups.len(), 1);
        assert!(result.backups[0].is_file());

        let repaired = fs::read_to_string(&gameinfo).unwrap();
        let addons_line = repaired.find(GAMEINFO_ADDONS_LINE).unwrap();
        let citadel_line = repaired.find("\tGame citadel\n").unwrap();
        assert!(addons_line < citadel_line);
        assert!(repaired.contains("\tGame core\n"));

        let second_plan = plan_deploy(&game_root, &release, &manifest, Some(7)).unwrap();
        assert!(second_plan.replace_required);

        let second = apply_deploy(
            &game_root,
            &release,
            &manifest,
            &second_plan,
            true,
            &backup_root,
        )
        .unwrap();
        assert!(second.deployed);
        assert!(!second.gameinfo_repaired);
        assert_eq!(second.backups.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod gameinfo_paths_tests {
    use super::*;
    use crate::discovery::read_game_info_status;

    const STOCK: &str = "\"GameInfo\"\r\n{\r\n\tFileSystem\r\n\t{\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tGame\t\t\t\tcitadel\r\n\t\t\tGame\t\t\t\tcore\r\n\t\t}\r\n\t}\r\n}\r\n";

    fn write(text: &str, tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-gameinfo-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gameinfo.gi");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_stock_file_gets_every_required_line_in_order() {
        let path = write(STOCK, "stock");
        let before = read_game_info_status(&path).unwrap();
        assert!(!before.addons_search_path);
        assert_eq!(before.missing_paths.len(), 6, "{:?}", before.missing_paths);

        let repair = repair_gameinfo(&path, true).unwrap();
        assert!(repair.changed);
        assert!(repair.backup.is_some());
        let text = fs::read_to_string(&path).unwrap();
        let order: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1))
            .filter(|w| !w.is_empty())
            .collect();
        let at = |needle: &str| order.iter().position(|w| *w == needle).unwrap_or_else(|| panic!("{needle} missing in {text}"));
        // Addons and the language mounts come before the base game; Mod and Write close the block.
        assert!(at("citadel/addons") < at("citadel_*LANGUAGE*"));
        assert!(at("citadel_lv") < at("citadel"));
        assert!(text.contains("Write\t\tcore"));
        assert!(text.contains("\r\n"), "the file's line endings are kept");
        let after = read_game_info_status(&path).unwrap();
        assert!(after.addons_search_path && after.missing_paths.is_empty(), "{:?}", after.missing_paths);

        // A second repair finds nothing to do.
        let again = repair_gameinfo(&path, true).unwrap();
        assert!(!again.changed && again.backup.is_none());
    }

    #[test]
    fn only_the_missing_lines_are_added() {
        let partial = STOCK.replace(
            "\t\t\tGame\t\t\t\tcitadel\r\n",
            "\t\t\tGame\t\tcitadel/addons\r\n\t\t\tGame_UILanguage\tcitadel_*LANGUAGE*\r\n\t\t\tGame_LowViolence\tcitadel_lv\r\n\t\t\tGame\t\t\t\tcitadel\r\n",
        );
        let path = write(&partial, "partial");
        let status = read_game_info_status(&path).unwrap();
        assert!(status.addons_search_path);
        assert_eq!(status.missing_paths, ["Mod citadel", "Write citadel", "Mod core", "Write core"]);
        assert!(repair_gameinfo(&path, true).unwrap().changed);
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("citadel/addons").count(), 1);
        assert_eq!(text.matches("citadel_lv").count(), 1);
        assert!(read_game_info_status(&path).unwrap().missing_paths.is_empty());
    }

    #[test]
    fn nothing_is_written_without_approval() {
        let path = write(STOCK, "unapproved");
        assert!(repair_gameinfo(&path, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), STOCK);
    }
}
