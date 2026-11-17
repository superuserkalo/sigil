//! Update checks for installed GameBanana mods.
//!
//! A mod Sigil installed from GameBanana has a library entry with its mod id, and the install
//! journal remembers which file it downloaded (`https://gamebanana.com/dl/<file id>`) and
//! when. A check reads the mod's current file list from the public API
//! (`apiv11/Mod/<id>?_csvProperties=_idRow,_aFiles`, the same host as Browse) and compares:
//!
//! - the installed file is known: an update is a newer, non-archived file than the installed
//!   one, uploaded after the install;
//! - only the install time is known (mods imported from another manager): an update is a
//!   non-archived file uploaded after that time;
//! - neither: unknown, never flagged.
//!
//! Results are cached in `<data_dir>/mod-updates.json` and refreshed at most every six hours.
//! Nothing here writes to the game install; an update itself is the normal install pipeline.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::install::InstallJournalEntry;

pub const UPDATE_CACHE_FILE: &str = "mod-updates.json";
pub const UPDATE_CACHE_SCHEMA: u32 = 1;
/// Six hours between automatic checks.
pub const CHECK_INTERVAL_SECONDS: i64 = 6 * 60 * 60;

const API_BASE: &str = "https://gamebanana.com/apiv11/Mod";
const USER_AGENT: &str = "dah/0.0.0";
const TIMEOUT_SECONDS: u64 = 20;

/// One file on a GameBanana mod page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    pub file_id: u64,
    pub file_name: String,
    /// `_tsDateAdded`, unix seconds.
    pub added: i64,
    #[serde(default)]
    pub archived: bool,
}

/// The file list of one mod, as the last check saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMod {
    pub files: Vec<RemoteFile>,
}

impl RemoteMod {
    /// The newest file that is not archived. A tie keeps the first listed.
    pub fn newest(&self) -> Option<&RemoteFile> {
        let mut best: Option<&RemoteFile> = None;
        for file in self.files.iter().filter(|f| !f.archived) {
            if best.is_none_or(|b| file.added > b.added) {
                best = Some(file);
            }
        }
        best
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCache {
    pub schema: u32,
    /// When the last full check finished, unix seconds. 0 means never.
    pub checked_at: i64,
    /// By GameBanana mod id.
    pub mods: BTreeMap<u64, RemoteMod>,
    /// Mods the last check could not read, with the reason.
    #[serde(default)]
    pub errors: BTreeMap<u64, String>,
}

impl Default for UpdateCache {
    fn default() -> Self {
        UpdateCache {
            schema: UPDATE_CACHE_SCHEMA,
            checked_at: 0,
            mods: BTreeMap::new(),
            errors: BTreeMap::new(),
        }
    }
}

/// What Sigil knows about the file that sits in a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Baseline {
    pub file_id: Option<u64>,
    /// Install time, unix seconds.
    pub installed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Current,
    /// A newer file is on GameBanana.
    Available(RemoteFile),
    /// Not enough is known to tell.
    Unknown,
}

pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(UPDATE_CACHE_FILE)
}

/// Read the cache. A missing or unreadable file is an empty cache.
pub fn read_cache(path: &Path) -> UpdateCache {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write the cache atomically. App data, not the game install.
pub fn write_cache(path: &Path, cache: &UpdateCache) -> Result<()> {
    let text = serde_json::to_string_pretty(cache)
        .map_err(|error| Error::msg(format!("Could not encode the update cache: {error}")))?;
    crate::util::write_atomic(path, text.as_bytes())
}

/// Whether a check should run: six hours since the last one, or a mod id the cache has never
/// seen.
pub fn is_due(cache: &UpdateCache, ids: &[u64], now: i64) -> bool {
    if ids.is_empty() {
        return false;
    }
    now - cache.checked_at >= CHECK_INTERVAL_SECONDS
        || ids
            .iter()
            .any(|id| !cache.mods.contains_key(id) && !cache.errors.contains_key(id))
}

/// Check every id with `fetch` and return the new cache. A failed read keeps the mod's last
/// known files, so one bad request does not hide an update the cache already knew.
pub fn check_all(
    previous: &UpdateCache,
    ids: &[u64],
    now: i64,
    mut fetch: impl FnMut(u64) -> Result<RemoteMod>,
) -> UpdateCache {
    let mut cache = UpdateCache {
        checked_at: now,
        ..UpdateCache::default()
    };
    let mut unique: Vec<u64> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    for id in unique {
        match fetch(id) {
            Ok(remote) => {
                cache.mods.insert(id, remote);
            }
            Err(error) => {
                if let Some(old) = previous.mods.get(&id) {
                    cache.mods.insert(id, old.clone());
                }
                cache.errors.insert(id, error.to_string());
            }
        }
    }
    cache
}

/// Read a mod's file list from the public API.
pub fn fetch_remote_mod(mod_id: u64) -> Result<RemoteMod> {
    let url = format!("{API_BASE}/{mod_id}?_csvProperties=_idRow,_aFiles");
    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(TIMEOUT_SECONDS)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|error| Error::Network(format!("GameBanana request for mod {mod_id} failed: {error}")))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::Network(format!("GameBanana response for mod {mod_id} failed: {error}")))?;
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("GameBanana returned HTTP {status} for mod {mod_id}.")));
    }
    parse_remote_mod(&body)
}

/// Parse `_aFiles` from a `Mod/<id>` or `Mod/<id>/ProfilePage` body.
pub fn parse_remote_mod(body: &str) -> Result<RemoteMod> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| Error::Network(format!("GameBanana response was not JSON ({error}).")))?;
    let files = value
        .get("_aFiles")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Network("GameBanana response had no _aFiles list.".to_string()))?;
    Ok(RemoteMod {
        files: files
            .iter()
            .filter_map(|file| {
                Some(RemoteFile {
                    file_id: file.get("_idRow").and_then(Value::as_u64)?,
                    file_name: file
                        .get("_sFile")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    added: file.get("_tsDateAdded").and_then(Value::as_i64).unwrap_or(0),
                    archived: file.get("_bIsArchived").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect(),
    })
}

/// The GameBanana file id in a download URL: `https://gamebanana.com/dl/1234567`.
pub fn file_id_from_url(url: &str) -> Option<u64> {
    let rest = url.split("/dl/").nth(1)?;
    rest.split(['/', '?', '#']).next()?.parse().ok()
}

/// The GameBanana mod id a journal entry installed, from its source label
/// (`gamebanana:715100`) or what the user typed (`715100`, a mod URL).
pub fn journal_mod_id(entry: &InstallJournalEntry) -> Option<u64> {
    if let Some(id) = entry.source_label.strip_prefix("gamebanana:") {
        return id.trim().parse().ok();
    }
    let source = entry.source.trim();
    if !source.is_empty() && source.chars().all(|c| c.is_ascii_digit()) {
        return source.parse().ok();
    }
    let rest = source.split("gamebanana.com/mods/").nth(1)?;
    rest.split(['/', '?', '#']).next()?.parse().ok()
}

/// What is known about the file installed for `mod_id` whose directory VPK has `vpk_bytes`.
/// The newest journal entry for the mod with that size wins; otherwise `recorded_at` (the
/// library entry's time, RFC 3339) stands in for the install time.
pub fn baseline_for(
    journal: &[InstallJournalEntry],
    mod_id: u64,
    vpk_bytes: u64,
    recorded_at: &str,
) -> Baseline {
    let entry = journal
        .iter()
        .rev()
        .find(|entry| entry.vpk_bytes == vpk_bytes && journal_mod_id(entry) == Some(mod_id));
    match entry {
        Some(entry) => Baseline {
            file_id: entry.download_url.as_deref().and_then(file_id_from_url),
            installed_at: parse_time(&entry.installed_at),
        },
        None => Baseline {
            file_id: None,
            installed_at: parse_time(recorded_at),
        },
    }
}

/// Compare what is installed with the mod's current files.
pub fn compare(baseline: Baseline, remote: &RemoteMod) -> UpdateState {
    let Some(newest) = remote.newest() else {
        return UpdateState::Unknown;
    };
    match (baseline.file_id, baseline.installed_at) {
        (Some(id), installed_at) => {
            if newest.file_id == id {
                return UpdateState::Current;
            }
            let installed_added = remote.files.iter().find(|f| f.file_id == id).map(|f| f.added);
            let newer_than_file = installed_added.is_none_or(|added| newest.added > added);
            let after_install = installed_at.is_none_or(|at| newest.added > at);
            if newer_than_file && after_install {
                UpdateState::Available(newest.clone())
            } else {
                UpdateState::Current
            }
        }
        (None, Some(at)) => {
            if newest.added > at {
                UpdateState::Available(newest.clone())
            } else {
                UpdateState::Current
            }
        }
        (None, None) => UpdateState::Unknown,
    }
}

fn parse_time(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|time| time.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, added: i64) -> RemoteFile {
        RemoteFile {
            file_id: id,
            file_name: format!("f{id}.zip"),
            added,
            archived: false,
        }
    }

    fn entry(source_label: &str, url: Option<&str>, bytes: u64, at: &str) -> InstallJournalEntry {
        serde_json::from_value(serde_json::json!({
            "schema": 1, "planId": "p", "installedAt": at, "source": "x",
            "sourceLabel": source_label, "downloadUrl": url, "archive": "/a",
            "archiveSha256": "", "slot": 3, "file": "pak03_dir.vpk", "target": "/t",
            "chunkFiles": [], "entries": 1, "bytes": 1, "vpkBytes": bytes, "vpkSha256": "",
            "backups": [], "removedChunks": [], "removedFiles": [], "gameinfoRepaired": false,
            "warnings": []
        }))
        .unwrap()
    }

    #[test]
    fn parses_the_file_list() {
        let body = r#"{"_idRow":1,"_aFiles":[
            {"_idRow":11,"_sFile":"old.zip","_tsDateAdded":100,"_bIsArchived":true},
            {"_idRow":12,"_sFile":"new.zip","_tsDateAdded":200},
            {"_sFile":"no id"}]}"#;
        let remote = parse_remote_mod(body).unwrap();
        assert_eq!(remote.files.len(), 2);
        assert!(remote.files[0].archived);
        assert_eq!(remote.newest().unwrap().file_id, 12);
        assert!(parse_remote_mod("{}").is_err());
        assert!(parse_remote_mod("nope").is_err());
    }

    #[test]
    fn reads_ids_from_urls_and_journal_entries() {
        assert_eq!(file_id_from_url("https://gamebanana.com/dl/1414905"), Some(1414905));
        assert_eq!(file_id_from_url("https://gamebanana.com/dl/77?x=1"), Some(77));
        assert_eq!(file_id_from_url("https://files.gamebanana.com/mods/x.zip"), None);
        assert_eq!(journal_mod_id(&entry("gamebanana:715100", None, 1, "")), Some(715100));
        let mut by_url = entry("x", None, 1, "");
        by_url.source = "https://gamebanana.com/mods/42".to_string();
        assert_eq!(journal_mod_id(&by_url), Some(42));
    }

    #[test]
    fn compares_by_file_then_by_time() {
        let remote = RemoteMod {
            files: vec![file(1, 1_000), file(2, 2_000)],
        };
        // Installed the newest file: current.
        let b = Baseline { file_id: Some(2), installed_at: Some(2_100) };
        assert_eq!(compare(b, &remote), UpdateState::Current);
        // Installed file 1, file 2 came after the install: update.
        let b = Baseline { file_id: Some(1), installed_at: Some(1_500) };
        assert_eq!(compare(b, &remote), UpdateState::Available(file(2, 2_000)));
        // File 2 was already there when file 1 was picked: not an update.
        let b = Baseline { file_id: Some(1), installed_at: Some(2_500) };
        assert_eq!(compare(b, &remote), UpdateState::Current);
        // Only a time: anything newer counts.
        let b = Baseline { file_id: None, installed_at: Some(1_500) };
        assert!(matches!(compare(b, &remote), UpdateState::Available(_)));
        let b = Baseline { file_id: None, installed_at: Some(3_000) };
        assert_eq!(compare(b, &remote), UpdateState::Current);
        assert_eq!(compare(Baseline::default(), &remote), UpdateState::Unknown);
        // An archived newest file never counts.
        let archived = RemoteMod {
            files: vec![file(1, 1_000), RemoteFile { archived: true, ..file(2, 2_000) }],
        };
        let b = Baseline { file_id: Some(1), installed_at: Some(1_500) };
        assert_eq!(compare(b, &archived), UpdateState::Current);
    }

    #[test]
    fn baseline_prefers_the_matching_journal_entry() {
        let journal = vec![
            entry("gamebanana:9", Some("https://gamebanana.com/dl/5"), 100, "2026-01-01T00:00:00Z"),
            entry("gamebanana:9", Some("https://gamebanana.com/dl/6"), 200, "2026-02-01T00:00:00Z"),
            entry("gamebanana:8", Some("https://gamebanana.com/dl/7"), 100, "2026-03-01T00:00:00Z"),
        ];
        let b = baseline_for(&journal, 9, 100, "");
        assert_eq!(b.file_id, Some(5));
        assert_eq!(b.installed_at, Some(1_767_225_600));
        let b = baseline_for(&journal, 9, 999, "2026-05-01T00:00:00.000Z");
        assert_eq!(b.file_id, None);
        assert!(b.installed_at.is_some());
        assert_eq!(baseline_for(&journal, 9, 999, ""), Baseline::default());
    }

    #[test]
    fn checks_are_due_every_six_hours_or_for_new_ids() {
        let mut cache = UpdateCache { checked_at: 1_000, ..Default::default() };
        cache.mods.insert(1, RemoteMod::default());
        assert!(!is_due(&cache, &[1], 1_000 + 60));
        assert!(is_due(&cache, &[1, 2], 1_000 + 60));
        assert!(is_due(&cache, &[1], 1_000 + CHECK_INTERVAL_SECONDS));
        assert!(!is_due(&cache, &[], 1_000_000));
    }

    #[test]
    fn a_failed_read_keeps_the_last_known_files() {
        let mut old = UpdateCache::default();
        old.mods.insert(1, RemoteMod { files: vec![file(3, 30)] });
        let cache = check_all(&old, &[1, 2, 2], 50, |id| {
            if id == 1 {
                Err(Error::Network("down".to_string()))
            } else {
                Ok(RemoteMod { files: vec![file(4, 40)] })
            }
        });
        assert_eq!(cache.checked_at, 50);
        assert_eq!(cache.mods[&1].files[0].file_id, 3);
        assert_eq!(cache.mods[&2].files[0].file_id, 4);
        assert!(cache.errors.contains_key(&1));
    }

    #[test]
    fn cache_round_trips() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/mod-updates-test")
            .join(format!("cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = cache_path(&dir);
        assert_eq!(read_cache(&path), UpdateCache::default());
        let mut cache = UpdateCache { checked_at: 7, ..Default::default() };
        cache.mods.insert(5, RemoteMod { files: vec![file(1, 2)] });
        write_cache(&path, &cache).unwrap();
        assert_eq!(read_cache(&path), cache);
    }
}
