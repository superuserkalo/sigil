//! Custom maps installed as addon mods: which map a mod carries, the console command that loads
//! it, and the connect code friends paste to join.
//!
//! A custom map ships as a normal addon (`pakNN_dir.vpk`) whose payload holds the compiled map
//! as a nested VPK at `maps/<name>.vpk`. The game loads it with the console command
//! `map <name>`. Deadlock Mod Manager reads the name from the GameBanana description; Sigil reads
//! it from the VPK first (exact, and works offline) and falls back to the description through
//! [`crate::servers::console_map_name`].
//!
//! Starting the game straight into a map uses the managed `map <name>` block in `autoexec.cfg`
//! ([`crate::servers::write_map_launch`]), the same way Deadlock Mod Manager does it. A
//! `steam://run/1422450//+map <name>` URL also carries the command, but Steam asks the player to
//! confirm launch arguments from a URL, so it is not used.
//!
//! When the map server is up, the game writes `ServerSteamID=[A:1:...]` to `console.log` (only
//! with `-condebug`). [`ConnectWatch`] follows the log for it, and [`connect_command`] is the line
//! friends paste into their console (F7).
//!
//! Read only. Nothing here writes to a game install or contacts the network.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::install::InstallJournalEntry;
use crate::servers::{console_map_name, valid_map_name};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// One map carried by an enabled addon slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMap {
    pub slot: u16,
    /// The console name: `map <name>` loads it.
    pub name: String,
    /// The mod's title from Sigil's install journal, when Sigil installed it.
    pub title: Option<String>,
    /// The slot's directory VPK.
    pub vpk: PathBuf,
}

/// The map names in these game paths: every `maps/<name>.vpk` directly under `maps/`, never a
/// hero-select scene under `maps/ui/`. Case is kept as shipped, either slash works, and each
/// name is listed once, in path order.
pub fn map_names_in_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for path in paths {
        let path = path.replace('\\', "/");
        let Some(rest) = path
            .get(..5)
            .filter(|head| head.eq_ignore_ascii_case("maps/"))
            .map(|_| &path[5..])
        else {
            continue;
        };
        if rest.contains('/') {
            continue;
        }
        let Some(stem) = rest
            .len()
            .checked_sub(4)
            .filter(|at| rest.is_char_boundary(*at) && rest[*at..].eq_ignore_ascii_case(".vpk"))
            .map(|at| &rest[..at])
        else {
            continue;
        };
        // `maps/foo_dir.vpk` style chunk names never appear inside a payload, but a stem must
        // still be a name the console accepts.
        if valid_map_name(stem) && !names.iter().any(|have| have.eq_ignore_ascii_case(stem)) {
            names.push(stem.to_string());
        }
    }
    names
}

/// [`map_names_in_paths`] over every file in the VPK at `path`. Reads the directory tree only.
pub fn map_names_in_vpk(path: &Path) -> Result<Vec<String>> {
    let directory = read_vpk_directory_with(
        path,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;
    Ok(map_names_in_paths(
        directory.entries.iter().map(|entry| entry.path.as_str()),
    ))
}

/// The map name to use: the first one the VPK carries, else the one the description names.
pub fn pick_map_name(vpk_names: &[String], description: Option<&str>) -> Option<String> {
    vpk_names
        .first()
        .cloned()
        .or_else(|| description.and_then(console_map_name))
}

/// Every map carried by an enabled addon slot, in `addons_dir` and its extra folders, by slot.
/// A switched-off slot is left out: the game does not load it, so its map cannot start. A slot
/// whose VPK cannot be read is skipped. Titles are None; [`attach_titles`] fills them.
pub fn installed_maps(addons_dir: &Path) -> Vec<InstalledMap> {
    let Ok(slots) = crate::load_order::read_slot_files(addons_dir) else {
        return Vec::new();
    };
    let mut maps = Vec::new();
    for (slot, files) in slots {
        let Some(dir_file) = files
            .iter()
            .find(|name| name.to_ascii_lowercase().ends_with("_dir.vpk"))
        else {
            continue;
        };
        let vpk = crate::addons::slot_folder(addons_dir, slot).join(dir_file);
        let Ok(names) = map_names_in_vpk(&vpk) else {
            continue;
        };
        for name in names {
            maps.push(InstalledMap {
                slot,
                name,
                title: None,
                vpk: vpk.clone(),
            });
        }
    }
    maps
}

/// Fill each map's title from the install journal: the newest entry for its slot whose VPK has
/// the same size as the file on disk. A map installed by hand or by another manager keeps None.
pub fn attach_titles(maps: &mut [InstalledMap], journal: &[InstallJournalEntry]) {
    for map in maps {
        let size = fs::metadata(&map.vpk).map(|meta| meta.len()).ok();
        map.title = journal
            .iter()
            .rev()
            .find(|entry| entry.slot == map.slot && Some(entry.vpk_bytes) == size)
            .map(|entry| entry.source_label.trim().to_string())
            .filter(|label| !label.is_empty());
    }
}

/// The console command that loads a map, or None for a name the console would not accept.
pub fn map_command(name: &str) -> Option<String> {
    valid_map_name(name).then(|| format!("map {name}"))
}

/// The line friends paste into their console (F7) to join.
pub fn connect_command(code: &str) -> String {
    format!("connect {}", code.trim())
}

/// The connect code in console output: the `[A:1:...:...]` after `ServerSteamID=`.
pub fn connect_code_in(text: &str) -> Option<String> {
    let pattern = Regex::new(r"ServerSteamID=(\[A:\d+:\d+:\d+\])").ok()?;
    pattern
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|code| code.as_str().to_string())
}

/// Follows `console.log` for the connect code. Starts at the end of the file as it is now, so
/// a code from an earlier session is never reported. A file that got shorter (the game started
/// again and truncated it) is read from the start.
#[derive(Debug, Clone, Default)]
pub struct ConnectWatch {
    pub offset: u64,
}

impl ConnectWatch {
    /// Start after whatever the log holds now.
    pub fn from_end(path: &Path) -> Self {
        Self {
            offset: fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
        }
    }

    /// Read what is new; the connect code when it appeared.
    pub fn poll(&mut self, path: &Path) -> Option<String> {
        let mut file = fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        if len < self.offset {
            self.offset = 0;
        }
        if len == self.offset {
            return None;
        }
        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut bytes = Vec::new();
        file.take(len - self.offset).read_to_end(&mut bytes).ok()?;
        // Only whole lines; a half-written last line waits for the next poll.
        let end = bytes.iter().rposition(|byte| *byte == b'\n')?;
        self.offset += end as u64 + 1;
        connect_code_in(&String::from_utf8_lossy(&bytes[..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dah-maps-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Pack `files` into `<dir>/<name>_dir.vpk` through a staging folder under `root`.
    fn vpk(root: &Path, dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let staging = root.join(format!("staging-{name}-{}", dir.display().to_string().len()));
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        fs::create_dir_all(dir).unwrap();
        pack_vpk(&PackOptions::new(staging, dir.to_path_buf(), name))
            .unwrap()
            .dir_path
    }

    #[test]
    fn map_names_are_vpks_directly_under_maps() {
        assert_eq!(
            map_names_in_paths([
                "maps/street_test.vpk",
                "maps/street_test/entities/lights.vmdl_c",
                "MAPS\\Arena_2.VPK",
            ]),
            vec!["street_test".to_string(), "Arena_2".to_string()]
        );
    }

    #[test]
    fn scenes_lookalikes_and_bad_names_are_not_maps() {
        assert!(map_names_in_paths(["maps/ui/hero_prefabs/necro.vpk"]).is_empty());
        assert!(map_names_in_paths(["panorama/maps/street.vpk"]).is_empty());
        assert!(map_names_in_paths(["maps/street.vmap_c"]).is_empty());
        assert!(map_names_in_paths(["maps/bad name.vpk"]).is_empty());
        assert!(map_names_in_paths(["maps/.vpk", "maps/"]).is_empty());
        assert!(map_names_in_paths([]).is_empty());
    }

    #[test]
    fn a_name_is_listed_once() {
        assert_eq!(
            map_names_in_paths(["maps/duel.vpk", "maps/DUEL.vpk"]),
            vec!["duel".to_string()]
        );
    }

    #[test]
    fn the_vpk_name_wins_over_the_description() {
        let names = vec!["harbor_night".to_string()];
        assert_eq!(
            pick_map_name(&names, Some("Open the console and type map other_name")),
            Some("harbor_night".to_string())
        );
        assert_eq!(
            pick_map_name(&[], Some("Open the console and type map other_name")),
            Some("other_name".to_string())
        );
        assert_eq!(pick_map_name(&[], Some("A lovely map.")), None);
        assert_eq!(pick_map_name(&[], None), None);
    }

    #[test]
    fn commands_are_built_only_for_valid_names() {
        assert_eq!(map_command("streetball").as_deref(), Some("map streetball"));
        assert_eq!(map_command("street ball; quit"), None);
        assert_eq!(map_command(""), None);
        assert_eq!(connect_command(" [A:1:3695290372:49365] "), "connect [A:1:3695290372:49365]");
    }

    #[test]
    fn the_connect_code_is_read_from_console_output() {
        let log = "Host_NewGame on map streetball\nSV: ServerSteamID=[A:1:3695290372:49365] ready\n";
        assert_eq!(connect_code_in(log).as_deref(), Some("[A:1:3695290372:49365]"));
        assert_eq!(connect_code_in("ServerSteamID=[G:1:2]"), None);
        assert_eq!(connect_code_in(""), None);
    }

    #[test]
    fn the_watch_reports_only_new_whole_lines() {
        let root = temp_root("watch");
        let log = root.join("console.log");
        fs::write(&log, "old ServerSteamID=[A:1:1:1]\n").unwrap();
        let mut watch = ConnectWatch::from_end(&log);
        assert_eq!(watch.poll(&log), None, "an old code is not reported");

        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        std::io::Write::write_all(&mut file, b"loading\nServerSteamID=[A:1:22:33]").unwrap();
        assert_eq!(watch.poll(&log), None, "the half-written line waits");
        std::io::Write::write_all(&mut file, b"\n").unwrap();
        assert_eq!(watch.poll(&log).as_deref(), Some("[A:1:22:33]"));

        // The game started again and truncated the log.
        fs::write(&log, "ServerSteamID=[A:1:44:55]\n").unwrap();
        assert_eq!(watch.poll(&log).as_deref(), Some("[A:1:44:55]"));

        let missing = root.join("nope.log");
        assert_eq!(ConnectWatch::from_end(&missing).poll(&missing), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installed_maps_scan_enabled_slots_in_every_folder() {
        let root = temp_root("installed");
        let addons = root.join("game/citadel/addons");
        fs::create_dir_all(&addons).unwrap();
        vpk(&root, &addons, "pak01", &[("maps/street_test.vpk", b"map")]);
        vpk(&root, &addons, "pak02", &[("models/hero.vmdl_c", b"skin")]);
        let off = vpk(&root, &addons, "pak03", &[("maps/off_map.vpk", b"map")]);
        fs::rename(&off, addons.join("pak03_dir.vpk.disabled")).unwrap();
        let extra = crate::addons::slot_folder(&addons, 100);
        vpk(&root, &extra, "pak01", &[("maps/far_map.vpk", b"map"), ("maps/far_two.vpk", b"m")]);

        let maps = installed_maps(&addons);
        let found: Vec<(u16, &str)> = maps.iter().map(|map| (map.slot, map.name.as_str())).collect();
        assert_eq!(found, vec![(1, "street_test"), (100, "far_map"), (100, "far_two")]);
        assert_eq!(maps[0].vpk, addons.join("pak01_dir.vpk"));
        assert_eq!(maps[1].vpk, extra.join("pak01_dir.vpk"));
        assert!(maps.iter().all(|map| map.title.is_none()));

        assert!(installed_maps(&root.join("missing")).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn titles_come_from_the_newest_matching_journal_entry() {
        let root = temp_root("titles");
        let addons = root.join("addons");
        let path = vpk(&root, &addons, "pak04", &[("maps/duel.vpk", b"map")]);
        let size = fs::metadata(&path).unwrap().len();
        let entry = |slot: u16, bytes: u64, label: &str| -> InstallJournalEntry {
            serde_json::from_value(serde_json::json!({
                "schema": 1, "planId": "p", "installedAt": "2026-11-01T00:00:00Z", "source": "x",
                "sourceLabel": label, "downloadUrl": null, "archive": "/a",
                "archiveSha256": "", "slot": slot, "file": "pak04_dir.vpk", "target": "/t",
                "chunkFiles": [], "entries": 1, "bytes": 1, "vpkBytes": bytes, "vpkSha256": "",
                "backups": [], "removedChunks": [], "removedFiles": [], "gameinfoRepaired": false,
                "warnings": []
            }))
            .unwrap()
        };
        let journal = vec![
            entry(4, size, "Old Duel"),
            entry(4, size, "Duel Arena"),
            entry(4, size + 1, "Other size"),
            entry(5, size, "Other slot"),
        ];
        let mut maps = installed_maps(&addons);
        attach_titles(&mut maps, &journal);
        assert_eq!(maps[0].title.as_deref(), Some("Duel Arena"));
        attach_titles(&mut maps, &journal[2..]);
        assert_eq!(maps[0].title, None, "hand-installed or replaced mods keep no title");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_vpk_is_an_error() {
        let root = temp_root("missing-vpk");
        assert!(map_names_in_vpk(&root.join("pak09_dir.vpk")).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
