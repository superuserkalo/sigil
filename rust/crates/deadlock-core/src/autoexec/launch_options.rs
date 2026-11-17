//! Steam launch options Sigil keeps for Deadlock and writes into Steam's `localconfig.vdf`
//! right before it launches the game (Grimoire's behaviour, MIT).
//!
//! The player's value lives in `<data>/launch-options.json`. It reaches Steam in one of two
//! ways, both user actions: the launch buttons call [`sync_at_launch`] before they hand off to
//! Steam, and the Autoexec page has a "Write now" button. Steam rewrites `localconfig.vdf`
//! when it exits, so a write is skipped while Steam runs. The file edit itself is
//! `steam_launch::write_file`, which copies the file aside first and writes atomically.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::steam_launch;
use crate::util::write_atomic;

const FILE: &str = "launch-options.json";

/// What the player saved on the Autoexec page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub value: String,
    /// The player saved a value at least once, so an empty value means "clear Steam's".
    pub managed: bool,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// The saved value; the default when nothing was saved or the file is unreadable.
pub fn load(data_dir: &Path) -> Saved {
    fs::read_to_string(path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Save the player's value (app data only; Steam is written at launch). The value is cleaned
/// to one line, and refused when it is too long.
pub fn save(data_dir: &Path, value: &str) -> Result<Saved> {
    let value = steam_launch::clean_value(value)?;
    let saved = Saved {
        value,
        managed: true,
    };
    fs::create_dir_all(data_dir)?;
    write_atomic(&path(data_dir), serde_json::to_string_pretty(&saved)?.as_bytes())?;
    Ok(saved)
}

/// What to write to Steam, if anything: nothing when the player never saved a value, or when
/// Steam already holds it.
pub fn resolve_write(desired: &str, managed: bool, on_disk: Option<&str>) -> Option<String> {
    if desired.is_empty() && !managed {
        return None;
    }
    if on_disk.unwrap_or("") == desired {
        return None;
    }
    Some(desired.to_string())
}

/// How Steam's copy compares, for the Autoexec page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamState {
    /// The `localconfig.vdf` Sigil would write, the most recently used account's.
    pub path: Option<PathBuf>,
    /// The launch options Steam holds now. `None` when unset or unreadable.
    pub current: Option<String>,
    pub steam_running: bool,
}

/// Read Steam's launch options for Deadlock. Read only.
pub fn steam_state(game_root: Option<&Path>) -> SteamState {
    let path = steam_launch::find_localconfigs(game_root).into_iter().next();
    let current = path
        .as_deref()
        .and_then(|path| steam_launch::read_file(path).ok().flatten());
    SteamState {
        path,
        current,
        steam_running: steam_launch::steam_running(),
    }
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchSync {
    /// The player never saved launch options in Sigil.
    NotManaged,
    /// Steam already holds the saved value.
    InSync,
    /// No `localconfig.vdf` was found: Steam has not run Deadlock for this account.
    NoSteamConfig,
    /// Steam is running and would overwrite the change when it exits. Nothing was written.
    SteamRunning,
    /// The value was written; the old file was copied to `backup`.
    Written { path: PathBuf, backup: PathBuf },
}

/// GAME-ADJACENT WRITE. Write the saved launch options into Steam's `localconfig.vdf` when
/// they differ. Called from the launch buttons right before Steam starts the game, and from
/// the "Write now" button. Skips while Steam runs; `steam_launch::write_file` backs the file
/// up and writes atomically.
pub fn sync_at_launch(data_dir: &Path, game_root: Option<&Path>) -> Result<LaunchSync> {
    let configs = steam_launch::find_localconfigs(game_root);
    sync_with(&load(data_dir), configs.first().map(PathBuf::as_path), steam_launch::steam_running())
}

/// [`sync_at_launch`] with its inputs passed in, for tests.
pub fn sync_with(saved: &Saved, localconfig: Option<&Path>, steam_running: bool) -> Result<LaunchSync> {
    if !saved.managed && saved.value.is_empty() {
        return Ok(LaunchSync::NotManaged);
    }
    let Some(path) = localconfig else {
        return Ok(LaunchSync::NoSteamConfig);
    };
    let on_disk = steam_launch::read_file(path)?;
    let Some(value) = resolve_write(&saved.value, saved.managed, on_disk.as_deref()) else {
        return Ok(LaunchSync::InSync);
    };
    if steam_running {
        return Ok(LaunchSync::SteamRunning);
    }
    let write = steam_launch::write_file_with(path, &value, true, false)?;
    Ok(LaunchSync::Written {
        path: write.path,
        backup: write.backup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const VDF: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1422450\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    fn temp(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "dah-launch-options-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_matches_grimoire() {
        assert_eq!(resolve_write("", false, Some("-novid")), None);
        assert_eq!(resolve_write("-high", true, Some("-high")), None);
        assert_eq!(resolve_write("-high", true, Some("-novid")), Some("-high".into()));
        assert_eq!(resolve_write("", true, Some("-novid")), Some(String::new()));
        assert_eq!(resolve_write("", true, None), None);
        assert_eq!(resolve_write("-high", false, None), Some("-high".into()));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = temp("save");
        assert_eq!(load(&dir), Saved::default());
        let saved = save(&dir, "  -high -novid  ").unwrap();
        assert!(saved.managed);
        assert_eq!(load(&dir), saved);
        assert_eq!(save(&dir, "-a\n-b").unwrap().value, "-a -b");
        assert!(save(&dir, &"x".repeat(2000)).is_err());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sync_writes_with_a_backup_and_skips_when_it_must() {
        let dir = temp("sync");
        let vdf = dir.join("localconfig.vdf");
        fs::write(&vdf, VDF).unwrap();
        let saved = Saved {
            value: "-high -novid".into(),
            managed: true,
        };

        assert_eq!(sync_with(&Saved::default(), Some(&vdf), false).unwrap(), LaunchSync::NotManaged);
        assert_eq!(sync_with(&saved, None, false).unwrap(), LaunchSync::NoSteamConfig);
        assert_eq!(sync_with(&saved, Some(&vdf), true).unwrap(), LaunchSync::SteamRunning);
        assert_eq!(fs::read_to_string(&vdf).unwrap(), VDF);

        match sync_with(&saved, Some(&vdf), false).unwrap() {
            LaunchSync::Written { backup, .. } => {
                assert_eq!(fs::read_to_string(&backup).unwrap(), VDF);
            }
            other => panic!("expected a write, got {other:?}"),
        }
        assert_eq!(
            steam_launch::read_file(&vdf).unwrap().as_deref(),
            Some("-high -novid")
        );
        assert_eq!(sync_with(&saved, Some(&vdf), false).unwrap(), LaunchSync::InSync);
        fs::remove_dir_all(dir).ok();
    }
}
