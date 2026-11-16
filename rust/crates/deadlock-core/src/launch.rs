//! Launch Deadlock through Steam with or without addons, and move addon files
//! aside while vanilla is running.
//!
//! Deadlock loads `game/citadel/addons` only when `gameinfo.gi` lists the folder
//! in `SearchPaths`. Launching is therefore just `steam://rungameid/1422450`;
//! this module never edits `gameinfo.gi` and never writes inside the game.
//!
//! A vanilla run needs the addon folder emptied. Rather than deleting anything,
//! [`stash_vanilla`] moves every `pakNN_dir.vpk` and `pakNN_MMM.vpk` into
//! `<data_dir>/vanilla-stash/<timestamp>/` and records the move in `stash.json`.
//! [`restore_vanilla`] moves them back and refuses when a destination already
//! exists. [`launch_vanilla`] stashes, starts Steam and returns without
//! restoring: the game keeps running in vanilla after the process exits, so the
//! restore belongs to a later step, either when the app sees the game exit or on
//! the next launch. Until then [`stash_status`] reports the pending stash.
//!
//! Nothing here deletes a VPK. The only removals are the empty stash folder and
//! `stash.json` after a complete restore.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::deploy::addons_dir;
use crate::discovery::{
    DEADLOCK_APP_ID, DeadlockInstall, InstallDiscoveryOptions, discover_deadlock_install,
};
use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

/// Steam hands the URL to the installed Deadlock client.
pub const STEAM_RUN_URL: &str = "steam://rungameid/1422450";

/// Schema version written to `stash.json`.
pub const STASH_SCHEMA: u32 = 1;

/// Folder under the app data directory that holds every stash and the manifest.
pub const STASH_DIR_NAME: &str = "vanilla-stash";

/// Manifest file name, directly under the stash root.
pub const STASH_MANIFEST_NAME: &str = "stash.json";

/// Launch mode recorded in [`LaunchResult`].
pub const MODE_MODDED: &str = "modded";

/// Launch mode recorded in [`LaunchResult`].
pub const MODE_VANILLA: &str = "vanilla";

/// One addon file that the stash moved.
///
/// `from` is the original absolute path inside `game/citadel/addons` and `to`
/// is the absolute path inside the stash folder. `crc32` is the standard
/// IEEE CRC-32 of the file contents at stash time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StashFile {
    pub from: PathBuf,
    pub to: PathBuf,
    pub bytes: u64,
    pub crc32: u32,
}

/// The `stash.json` document. It is written before the first move, so a crash
/// mid-move still leaves a manifest that [`restore_vanilla`] can finish.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StashManifest {
    pub version: u32,
    pub created_at: String,
    pub game_root: PathBuf,
    pub files: Vec<StashFile>,
    pub addons_dir: PathBuf,
}

/// What an approved stash moved and what it left in place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StashResult {
    pub addons_dir: PathBuf,
    /// Timestamp folder that holds the moved files.
    pub stash_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub files: Vec<StashFile>,
    pub total_bytes: u64,
    /// Addon files the game does not load with these names, left untouched.
    pub ignored: Vec<String>,
}

/// What an approved restore moved back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub addons_dir: PathBuf,
    pub stash_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub restored: Vec<StashFile>,
    /// Files already back in the addon folder, from a crash after a partial
    /// restore or a previous restore that did not clean up. Nothing to do.
    pub already_present: Vec<String>,
    pub removed_manifest: bool,
    /// A stash folder that could not be removed because it is not empty.
    pub warnings: Vec<String>,
}

/// Read-only view of a pending stash. `problem` is set when `stash.json` exists
/// but cannot be read, so status never pretends there is no stash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StashStatus {
    pub stash_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub created_at: String,
    pub game_root: PathBuf,
    pub addons_dir: PathBuf,
    pub files: Vec<StashFile>,
    pub present: usize,
    pub missing: usize,
    pub total_bytes: u64,
    pub problem: Option<String>,
}

/// A Steam launch that was handed off to the platform opener.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResult {
    pub mode: String,
    pub app_id: String,
    pub url: String,
    pub game_root: PathBuf,
    pub addons_dir: PathBuf,
    /// Opener program that received the URL.
    pub opener: String,
    /// Pending stash when this was a vanilla launch.
    pub stash_dir: Option<PathBuf>,
}

/// Root of every stash under the app data directory.
pub fn vanilla_stash_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(STASH_DIR_NAME)
}

/// The single manifest that describes the pending stash.
pub fn stash_manifest_path(data_dir: &Path) -> PathBuf {
    vanilla_stash_dir(data_dir).join(STASH_MANIFEST_NAME)
}

/// Launch Deadlock with addons. Requires `approve` and an installed game.
///
/// Nothing is written: `gameinfo.gi` is left as it is, so whether mods load
/// depends on the SearchPaths line the install already has.
pub fn launch_modded(approve: bool) -> Result<LaunchResult> {
    if !approve {
        return Err(Error::msg(
            "Launching Deadlock was not approved. Nothing was started. Re-run with --approve.",
        ));
    }

    let install = require_install()?;
    let opener = open_steam()?;

    Ok(LaunchResult {
        mode: MODE_MODDED.to_string(),
        app_id: DEADLOCK_APP_ID.to_string(),
        url: STEAM_RUN_URL.to_string(),
        game_root: install.game_root.clone(),
        addons_dir: install.addons_dir.clone(),
        opener,
        stash_dir: None,
    })
}

/// Move every addon file into a timestamped stash folder. Requires `approve`.
///
/// Only `pakNN_dir.vpk` and `pakNN_MMM.vpk` with a slot from 01 to 99 move.
/// Other files, including `pakNN_dir.vpk.disabled`, are reported in
/// `ignored` and stay where they are. Files are moved, never copied or deleted,
/// and a second stash is refused while a manifest exists.
pub fn stash_vanilla(game_root: &Path, data_dir: &Path, approve: bool) -> Result<StashResult> {
    let addons = addons_dir(game_root);
    let manifest_path = stash_manifest_path(data_dir);

    if !approve {
        return Err(Error::msg(format!(
            "Stashing addons from {} was not approved. Nothing was moved. Re-run with --approve.",
            addons.display()
        )));
    }

    if manifest_path.is_file() {
        return Err(Error::msg(format!(
            "A vanilla stash already exists: {}. Restore it with `dah launch restore --approve` before stashing again. Nothing was moved.",
            manifest_path.display()
        )));
    }

    if !addons.is_dir() {
        return Err(Error::msg(format!(
            "Addon folder {} does not exist. Nothing was moved.",
            addons.display()
        )));
    }

    let (names, ignored) = collect_addon_files(&addons)?;
    let stash_dir = unique_stash_dir(data_dir);
    fs::create_dir_all(&stash_dir)?;

    let mut files = Vec::with_capacity(names.len());

    for (name, bytes) in names {
        let from = addons.join(&name);
        files.push(StashFile {
            to: stash_dir.join(&name),
            crc32: file_crc32(&from)?,
            from,
            bytes,
        });
    }

    let manifest = StashManifest {
        version: STASH_SCHEMA,
        created_at: now_iso(),
        game_root: game_root.to_path_buf(),
        files: files.clone(),
        addons_dir: addons.clone(),
    };

    // Manifest first: a crash after this point leaves a recoverable record.
    if let Err(error) = write_manifest(data_dir, &manifest) {
        let _ = fs::remove_dir(&stash_dir);
        return Err(error);
    }

    let total_bytes = files.iter().map(|file| file.bytes).sum();

    for (index, file) in files.iter().enumerate() {
        if let Err(error) = fs::rename(&file.from, &file.to) {
            let rolled_back = rollback_moves(&files[..index]);
            let suffix = if rolled_back {
                format!("Nothing was moved. {error}")
            } else {
                format!(
                    "Some files could not be moved back. Run `dah launch restore --approve` to finish. {error}"
                )
            };

            if rolled_back {
                let _ = fs::remove_file(&manifest_path);
                let _ = fs::remove_dir(&stash_dir);
            }

            return Err(Error::msg(format!(
                "Could not stash {} into {}: {suffix}",
                file.from.display(),
                file.to.display()
            )));
        }
    }

    Ok(StashResult {
        addons_dir: addons,
        stash_dir,
        manifest_path,
        files,
        total_bytes,
        ignored,
    })
}

/// Move stashed addon files back. Requires `approve`.
///
/// The call checks every file before moving any: a destination that already
/// exists refuses the whole restore. That covers a mod deployed while the game
/// ran vanilla, which must not be overwritten. A file already back at its
/// original path is skipped, so a restore after a crash finishes the job. On
/// success the empty stash folder and `stash.json` are removed.
pub fn restore_vanilla(data_dir: &Path, approve: bool) -> Result<RestoreResult> {
    let manifest_path = stash_manifest_path(data_dir);

    if !approve {
        return Err(Error::msg(format!(
            "Restoring the vanilla stash at {} was not approved. Nothing was moved. Re-run with --approve.",
            manifest_path.display()
        )));
    }

    let manifest = read_manifest(data_dir)?;
    let entries = validated_entries(&manifest, data_dir)?;
    let addons = manifest.addons_dir.clone();
    let stash_dir = entries
        .first()
        .and_then(|(_, to, _)| to.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| vanilla_stash_dir(data_dir));

    let mut already_present = Vec::new();

    // Pre-flight the whole set so a refusal leaves addons exactly as they are.
    for (name, to, _) in &entries {
        let from = addons.join(name);
        let from_exists = from.is_file();
        let to_exists = to.is_file();

        match (from_exists, to_exists) {
            (true, true) => {
                return Err(Error::msg(format!(
                    "Restore refused: {name} is both in {} and in the stash at {}. Remove or rename one first. Nothing was moved.",
                    from.display(),
                    to.display()
                )));
            }
            (false, true) => {}
            (true, false) => already_present.push(name.clone()),
            (false, false) => {
                return Err(Error::msg(format!(
                    "Restore refused: {name} is neither in {} nor in the stash at {}. The stash is incomplete. Nothing was moved.",
                    from.display(),
                    to.display()
                )));
            }
        }
    }

    if !entries.is_empty() {
        let citadel = addons.parent().map(Path::to_path_buf);
        let citadel_present = citadel.as_deref().is_some_and(Path::is_dir);

        if !citadel_present {
            return Err(Error::msg(format!(
                "Restore refused: {} no longer exists, so the Deadlock install looks removed. The stash stays at {}. Nothing was moved.",
                addons.display(),
                stash_dir.display()
            )));
        }

        if !addons.is_dir() {
            fs::create_dir_all(&addons)?;
        }
    }

    let mut restored = Vec::with_capacity(entries.len());

    for (name, to, file) in &entries {
        let from = addons.join(name);

        if !to.is_file() {
            continue;
        }

        fs::rename(to, &from).map_err(|error| {
            Error::msg(format!(
                "Could not restore {name}: rename {} to {} failed: {error}. The stash stays at {}. Run restore again after fixing this file.",
                to.display(),
                from.display(),
                stash_dir.display()
            ))
        })?;

        restored.push(file.clone());
    }

    let mut warnings = Vec::new();

    prune_empty_dirs(&vanilla_stash_dir(data_dir), &mut warnings);

    fs::remove_file(&manifest_path).map_err(|error| {
        Error::msg(format!(
            "Files were restored but {} could not be removed: {error}. Delete it before the next stash.",
            manifest_path.display()
        ))
    })?;

    // The manifest was the last entry, so the root is empty now.
    let _ = fs::remove_dir(&vanilla_stash_dir(data_dir));

    Ok(RestoreResult {
        addons_dir: addons,
        stash_dir,
        manifest_path,
        restored,
        already_present,
        removed_manifest: true,
        warnings,
    })
}

/// Read the pending stash without writing anything.
///
/// `None` means no `stash.json` exists. If the file exists but cannot be read,
/// the status carries a `problem` string instead of hiding it.
pub fn stash_status(data_dir: &Path) -> Option<StashStatus> {
    let manifest_path = stash_manifest_path(data_dir);

    if !manifest_path.is_file() {
        return None;
    }

    match read_manifest(data_dir) {
        Ok(manifest) => {
            let stash_dir = manifest
                .files
                .first()
                .and_then(|file| file.to.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| vanilla_stash_dir(data_dir));
            let present = manifest
                .files
                .iter()
                .filter(|file| file.to.is_file())
                .count();
            let total_bytes = manifest
                .files
                .iter()
                .filter(|file| file.to.is_file())
                .map(|file| file.bytes)
                .sum();

            Some(StashStatus {
                stash_dir,
                manifest_path,
                created_at: manifest.created_at,
                game_root: manifest.game_root,
                addons_dir: manifest.addons_dir,
                missing: manifest.files.len() - present,
                files: manifest.files,
                present,
                total_bytes,
                problem: None,
            })
        }
        Err(error) => Some(StashStatus {
            stash_dir: vanilla_stash_dir(data_dir),
            manifest_path,
            created_at: String::new(),
            game_root: PathBuf::new(),
            addons_dir: PathBuf::new(),
            files: Vec::new(),
            present: 0,
            missing: 0,
            total_bytes: 0,
            problem: Some(error.to_string()),
        }),
    }
}

/// Stash the addons, then start Deadlock through Steam in vanilla mode.
///
/// The stash is not restored here. The game outlives this call, so restoring
/// immediately would put the mods back while vanilla is still running. The app
/// restores after the game exits or on the next launch, and [`stash_status`]
/// tells it whether a stash is still pending.
pub fn launch_vanilla(game_root: &Path, data_dir: &Path, approve: bool) -> Result<LaunchResult> {
    let stash = stash_vanilla(game_root, data_dir, approve)?;

    match open_steam() {
        Ok(opener) => Ok(LaunchResult {
            mode: MODE_VANILLA.to_string(),
            app_id: DEADLOCK_APP_ID.to_string(),
            url: STEAM_RUN_URL.to_string(),
            game_root: game_root.to_path_buf(),
            addons_dir: stash.addons_dir.clone(),
            opener,
            stash_dir: Some(stash.stash_dir),
        }),
        Err(error) => Err(Error::msg(format!(
            "{error} Addons are stashed at {}. Run `dah launch restore --approve` to move them back.",
            stash.stash_dir.display()
        ))),
    }
}

fn require_install() -> Result<DeadlockInstall> {
    let discovery = discover_deadlock_install(InstallDiscoveryOptions::default())?;

    match discovery.install {
        Some(install) => Ok(install),
        None => {
            let searched: Vec<String> = discovery
                .searched_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect();
            let suffix = if searched.is_empty() {
                String::new()
            } else {
                format!(" Searched: {}.", searched.join(", "))
            };

            Err(Error::msg(format!(
                "Deadlock install not found.{suffix} Install Deadlock through Steam or set DAH_GAME_DIR. Nothing was started."
            )))
        }
    }
}

/// Hand the Steam URL to the platform opener and wait for it to exit.
fn open_steam() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(STEAM_RUN_URL);
        after_spawn(command, "open")
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", STEAM_RUN_URL]);
        after_spawn(command, "cmd /C start")
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(STEAM_RUN_URL);
        after_spawn(command, "xdg-open")
    }
}

fn after_spawn(mut command: Command, label: &str) -> Result<String> {
    let status = command.status().map_err(|error| {
        Error::msg(format!(
            "Could not start {label} for {STEAM_RUN_URL}: {error}."
        ))
    })?;

    if !status.success() {
        return Err(Error::msg(format!(
            "Could not hand {STEAM_RUN_URL} to {label}: opener exited with {status}. Is Steam installed?"
        )));
    }

    Ok(label.to_string())
}

/// Addon file names that the game loads and the stash may move, sorted, with
/// their sizes. Everything else is reported as ignored.
fn collect_addon_files(addons: &Path) -> Result<(Vec<(String, u64)>, Vec<String>)> {
    let read_dir = fs::read_dir(addons)?;
    let slot_pattern = Regex::new(r"(?i)^pak(\d{2})_dir\.vpk$").expect("slot pattern is valid");
    let chunk_pattern = Regex::new(r"(?i)^pak(\d{2})_\d{3}\.vpk$").expect("chunk pattern is valid");

    let mut files = Vec::new();
    let mut ignored = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_regular = entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false);

        if !is_regular {
            ignored.push(format!("{name}/"));
            continue;
        }

        let slot = slot_pattern
            .captures(&name)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<u8>().ok());
        let chunk = chunk_pattern
            .captures(&name)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<u8>().ok());

        let matched = slot.is_some() || chunk.is_some();

        if !matched || !(1..=99).contains(&slot.or(chunk).unwrap_or(0)) {
            ignored.push(name);
            continue;
        }

        let bytes = fs::metadata(&path).map(|data| data.len()).unwrap_or(0);
        files.push((name, bytes));
    }

    files.sort_by(|left, right| left.0.cmp(&right.0));
    ignored.sort();

    Ok((files, ignored))
}

fn unique_stash_dir(data_dir: &Path) -> PathBuf {
    let root = vanilla_stash_dir(data_dir);
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    let mut candidate = root.join(&stamp);
    let mut suffix = 1u32;

    while candidate.exists() {
        suffix += 1;
        candidate = root.join(format!("{stamp}-{suffix}"));
    }

    candidate
}

fn file_crc32(path: &Path) -> Result<u32> {
    let mut file = fs::File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let read = file.read(&mut buffer)?;

        if read == 0 {
            break;
        }

        hasher.update(&buffer[..read]);
    }

    Ok(hasher.finalize())
}

fn write_manifest(data_dir: &Path, manifest: &StashManifest) -> Result<()> {
    let path = stash_manifest_path(data_dir);
    let mut text = serde_json::to_string_pretty(manifest)?;
    text.push('\n');
    write_atomic(&path, text.as_bytes())
}

fn read_manifest(data_dir: &Path) -> Result<StashManifest> {
    let path = stash_manifest_path(data_dir);

    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "No vanilla stash at {}. Nothing was moved.",
                path.display()
            ))
        } else {
            Error::msg(format!("Could not read {}: {error}", path.display()))
        }
    })?;

    serde_json::from_str(&text).map_err(|error| {
        Error::msg(format!(
            "{} is not a valid stash manifest: {error}",
            path.display()
        ))
    })
}

/// Resolve `(file name, stash path, manifest entry)` for every recorded file.
///
/// `stash.json` is data on disk, so paths are rebuilt from the manifest's
/// `addonsDir` and the file name instead of trusting the stored absolute paths.
/// A stored path that does not match the rebuilt one is refused as traversal.
fn validated_entries(
    manifest: &StashManifest,
    data_dir: &Path,
) -> Result<Vec<(String, PathBuf, StashFile)>> {
    if manifest.version != STASH_SCHEMA {
        return Err(Error::msg(format!(
            "Stash manifest uses version {}. This build reads version {STASH_SCHEMA}. Nothing was moved.",
            manifest.version
        )));
    }

    let addons = normalize(&manifest.addons_dir);
    let is_addons = addons
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("addons"))
        && addons
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name.eq_ignore_ascii_case("citadel"));

    if !is_addons {
        return Err(Error::msg(format!(
            "Stash manifest points at {}, which is not game/citadel/addons. Nothing was moved.",
            addons.display()
        )));
    }

    let stash_root = vanilla_stash_dir(data_dir);
    let mut entries = Vec::with_capacity(manifest.files.len());
    let mut names: Vec<String> = Vec::with_capacity(manifest.files.len());

    for file in &manifest.files {
        let name = file
            .to
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| {
                Error::msg(format!(
                    "Stash manifest records {:?}, which has no file name. Nothing was moved.",
                    file.to.display()
                ))
            })?;

        if !is_stashable_name(&name) {
            return Err(Error::msg(format!(
                "Stash manifest records {name:?}, which is not pakNN_dir.vpk or pakNN_MMM.vpk. Nothing was moved."
            )));
        }

        if names.contains(&name) {
            return Err(Error::msg(format!(
                "Stash manifest records {name} more than once. Nothing was moved."
            )));
        }

        let to = normalize(&file.to);
        let from = normalize(&file.from);
        let expected_from = addons.join(&name);
        let expected_to = stash_root
            .join(
                to.parent()
                    .and_then(Path::file_name)
                    .unwrap_or_else(|| std::ffi::OsStr::new("")),
            )
            .join(&name);

        if from != expected_from || to != expected_to {
            return Err(Error::msg(format!(
                "Stash manifest records paths outside the addon folder or stash for {name}. Refusing path traversal. Nothing was moved."
            )));
        }

        names.push(name.clone());
        entries.push((name, to, file.clone()));
    }

    Ok(entries)
}

fn is_stashable_name(name: &str) -> bool {
    let slot_pattern = Regex::new(r"(?i)^pak(\d{2})_dir\.vpk$").expect("slot pattern is valid");
    let chunk_pattern = Regex::new(r"(?i)^pak(\d{2})_\d{3}\.vpk$").expect("chunk pattern is valid");

    let slot = slot_pattern
        .captures(name)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<u8>().ok())
        .or_else(|| {
            chunk_pattern
                .captures(name)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<u8>().ok())
        });

    slot.is_some_and(|value| (1..=99).contains(&value))
}

fn rollback_moves(moved: &[StashFile]) -> bool {
    for file in moved.iter().rev() {
        if fs::rename(&file.to, &file.from).is_err() {
            return false;
        }
    }

    true
}

/// Remove the stash folder and any other empty folder under the stash root.
/// `remove_dir` refuses a folder that still holds files, so nothing is lost.
fn prune_empty_dirs(root: &Path, warnings: &mut Vec<String>) {
    let mut directories = Vec::new();

    if let Ok(read_dir) = fs::read_dir(root) {
        for entry in read_dir.flatten() {
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                directories.push(entry.path());
            }
        }
    }

    directories.sort();

    for directory in directories {
        if let Err(error) = fs::remove_dir(&directory) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warnings.push(format!(
                    "Stash folder {} was not removed: {error}. It still holds files.",
                    directory.display()
                ));
            }
        }
    }

    let _ = fs::remove_dir(root);
}

fn normalize(path: &Path) -> PathBuf {
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
