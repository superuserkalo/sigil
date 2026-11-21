//! Custom performance presets from public GitHub repositories (Grimoire #572, issue #568).
//!
//! The player types a repository; Sigil reads it at its newest commit (`api.github.com`: the
//! commit and its file tree), lists the gameinfo and video files in it, and downloads only the
//! files the player picks from `raw.githubusercontent.com` at that commit, each checked against
//! the git blob hash the tree records. Nothing is read until the player presses Look up, and
//! nothing about the player is sent: anonymous GETs of public files. `docs/privacy.md` lists it.
//!
//! What is downloaded is kept (a snapshot at that commit), so a custom preset keeps working
//! offline and never changes under the player. A repository is not a curated source, so a
//! custom preset contributes plain ConVars and video values only: no engine section edits, no
//! console commands, no value that is not a number or true/false. What is left out is listed.
//!
//! The player can turn some of its settings into switches with their own labels, hide the
//! built-in presets, delete a custom preset, and import a VPK the repository ships through the
//! normal import (switched off, then the mod safety review when it is turned on). Imports are
//! remembered per preset.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::catalog::{ConfigDef, SourceDef, Tier, Versioning};
use super::fetch::{self, CachedConfig, Remote, git_blob_sha};
use super::generate::{self, Held, OptIn};
use super::{Config, GameConvars, classify::Group};
use crate::error::{Error, Result};
use crate::util::write_atomic;

/// The store of custom presets, under the performance cache.
pub const STORE_FILE: &str = "custom.json";
/// Custom presets kept at once (Grimoire's limit).
pub const LIMIT: usize = 50;
/// The largest gameinfo or video file read.
pub const FILE_MAX: u64 = 1_000_000;
/// The largest VPK a repository may ship for import.
pub const ADDON_MAX: u64 = 128 * 1024 * 1024;
/// Switches per preset.
pub const TOGGLES_MAX: usize = 100;
/// Every custom preset id starts with this.
pub const ID_PREFIX: &str = "custom-";

/// One file in the repository tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFile {
    pub path: String,
    pub size: u64,
    /// Git blob hash at the inspected commit.
    pub sha: String,
}

/// A setting the player made into a switch, with its label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toggle {
    pub key: String,
    pub label: String,
}

/// One custom preset as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPreset {
    pub id: String,
    pub name: String,
    /// `owner/repository`.
    pub repo: String,
    pub commit: String,
    /// The commit's date, yyyy-mm-dd.
    pub date: String,
    /// The gameinfo file read, None for a video-only preset.
    pub config_path: Option<String>,
    pub video_path: Option<String>,
    #[serde(default)]
    pub toggles: Vec<Toggle>,
    /// The VPKs the repository ships at that commit.
    #[serde(default)]
    pub addons: Vec<RepoFile>,
    /// Import history: VPK paths the player imported, not a claim they are still installed.
    #[serde(default)]
    pub imported: Vec<String>,
    /// When it was added, ISO 8601.
    pub added_at: String,
}

impl CustomPreset {
    pub fn url(&self) -> String {
        format!("https://github.com/{}/tree/{}", self.repo, self.commit)
    }

    pub fn toggle_label(&self, key: &str) -> Option<&str> {
        self.toggles.iter().find(|t| t.key == key).map(|t| t.label.as_str())
    }
}

/// Everything the store holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    pub presets: Vec<CustomPreset>,
    /// Hide the built-in presets in the list (the custom ones stay).
    pub hide_builtins: bool,
}

impl Store {
    pub fn get(&self, id: &str) -> Option<&CustomPreset> {
        self.presets.iter().find(|p| p.id == id)
    }
}

fn store_path(data_dir: &Path) -> PathBuf {
    fetch::cache_root(data_dir).join(STORE_FILE)
}

/// Read the store. Missing is empty; a damaged store is an error, so an import never
/// overwrites it silently.
pub fn read_store(data_dir: &Path) -> Result<Store> {
    match std::fs::read_to_string(store_path(data_dir)) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| Error::msg(format!("The custom preset list is not readable: {e}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(error.into()),
    }
}

fn write_store(data_dir: &Path, store: &Store) -> Result<()> {
    write_atomic(&store_path(data_dir), &serde_json::to_vec_pretty(store)?)
}

// --- input checks ------------------------------------------------------------------------

/// `owner/repository` from a GitHub URL or the short form.
pub fn parse_repository(input: &str) -> Result<String> {
    let mut repo = input.trim();
    for prefix in ["https://github.com/", "http://github.com/", "github.com/"] {
        if repo.len() >= prefix.len() && repo[..prefix.len()].eq_ignore_ascii_case(prefix) {
            repo = &repo[prefix.len()..];
        }
    }
    let repo = repo.trim_end_matches('/');
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let bad = || Error::msg("Use a GitHub repository URL or owner/repository.");
    let (owner, name) = repo.split_once('/').ok_or_else(bad)?;
    let owner_ok = !owner.is_empty()
        && owner.len() <= 39
        && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-');
    let name_ok = !name.is_empty()
        && name.len() <= 100
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && name != "."
        && name != "..";
    if owner_ok && name_ok {
        Ok(format!("{owner}/{name}"))
    } else {
        Err(bad())
    }
}

/// A relative path inside the repository, with no `.`, `..`, empty parts or control bytes.
pub fn validate_path(path: &str) -> Result<String> {
    let bad = path.is_empty()
        || path.len() > 500
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || path.contains(['\\', '?', '#'])
        || path.chars().any(|c| (c as u32) < 32);
    if bad {
        Err(Error::msg("Use a relative file path within the repository."))
    } else {
        Ok(path.to_string())
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn folder(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..=at])
}

/// A gameinfo file: `*.gi` or `gameinfo*.txt` (some authors publish the ConVars block as text).
pub fn is_config_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = file_name(&lower);
    lower.ends_with(".gi") || (name.starts_with("gameinfo") && name.ends_with(".txt"))
}

/// A video settings file: `video*.txt`.
pub fn is_video_path(path: &str) -> bool {
    let name = file_name(path).to_ascii_lowercase();
    name.starts_with("video") && name.ends_with(".txt")
}

/// The video file that goes with `config`: one in the same folder (or its `cfg/`), the
/// conventional `video.txt` first. `Ok(None)` when there is none, `Err` when several fit and
/// the player has to choose.
#[allow(clippy::result_unit_err)]
pub fn suggested_video_path(config: Option<&str>, videos: &[String]) -> std::result::Result<Option<String>, ()> {
    let candidates: Vec<&String> = match config {
        None => videos.iter().collect(),
        Some(config) => {
            let dir = folder(config);
            videos
                .iter()
                .filter(|path| {
                    let parent = folder(path);
                    parent == dir || parent == format!("{dir}cfg/")
                })
                .collect()
        }
    };
    let conventional: Vec<&&String> =
        candidates.iter().filter(|path| file_name(path).eq_ignore_ascii_case("video.txt")).collect();
    if conventional.len() == 1 {
        return Ok(Some((*conventional[0]).clone()));
    }
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(Some(candidates[0].clone())),
        _ => Err(()),
    }
}

/// VPKs that import on their own: not a numbered chunk, and not a `_dir.vpk` that needs them.
pub fn standalone_addons(files: &[RepoFile]) -> Vec<RepoFile> {
    let chunk = |path: &str| {
        let lower = path.to_ascii_lowercase();
        lower.len() > 8
            && lower.ends_with(".vpk")
            && lower.as_bytes()[lower.len() - 8] == b'_'
            && lower[lower.len() - 7..lower.len() - 4].bytes().all(|b| b.is_ascii_digit())
    };
    let split_bases: Vec<String> = files
        .iter()
        .filter(|f| chunk(&f.path))
        .map(|f| f.path.to_ascii_lowercase()[..f.path.len() - 8].to_string())
        .collect();
    files
        .iter()
        .filter(|f| {
            let lower = f.path.to_ascii_lowercase();
            lower.ends_with(".vpk")
                && !chunk(&f.path)
                && !lower
                    .strip_suffix("_dir.vpk")
                    .is_some_and(|base| split_bases.iter().any(|b| b == base))
        })
        .cloned()
        .collect()
}

/// A Valve config written as a bare `ConVars { ... }` block gets a root, so the same reader and
/// gates apply (Grimoire's `normalizeCustomGameinfo`).
pub fn normalize_gameinfo(text: &str) -> String {
    let code: String = text
        .lines()
        .map(|line| line.find("//").map_or(line, |at| &line[..at]))
        .collect::<Vec<_>>()
        .join("\n");
    let code = code.trim_start();
    let bare = code.strip_prefix('"').unwrap_or(code);
    let is_block = bare
        .strip_prefix("ConVars")
        .map(|rest| rest.trim_start_matches('"').trim_start().starts_with('{'))
        .unwrap_or(false);
    if is_block {
        format!("\"GameInfo\"\n{{\n{text}\n}}\n")
    } else {
        text.to_string()
    }
}

// --- the repository ------------------------------------------------------------------------

/// What a repository has at one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub repository: String,
    pub commit: String,
    pub date: String,
    pub config_paths: Vec<String>,
    pub video_paths: Vec<String>,
    pub addons: Vec<RepoFile>,
    /// Files the import cannot use (`.cfg`, archives), shown so the player knows why.
    pub unsupported: Vec<String>,
    /// Every usable file, for the downloads.
    pub files: BTreeMap<String, RepoFile>,
}

fn encode(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|byte| {
                    if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                        (byte as char).to_string()
                    } else {
                        format!("%{byte:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Read the repository at its newest commit, or at `commit`. Network (`api.github.com`); run
/// off the UI thread, and only from the player's Look up.
pub fn inspect(remote: &dyn Remote, repository: &str, commit: Option<&str>) -> Result<Inspection> {
    let repo = parse_repository(repository)?;
    if let Some(commit) = commit
        && !is_sha(commit)
    {
        return Err(Error::msg("That is not a commit id."));
    }
    let api = format!("https://api.github.com/repos/{repo}");
    let head: Value = match commit {
        Some(commit) => remote.get_json(&format!("{api}/commits/{commit}"))?,
        None => remote.get_json(&format!("{api}/commits?per_page=1"))?[0].clone(),
    };
    let sha = head["sha"].as_str().unwrap_or_default().to_string();
    let date: String = head["commit"]["committer"]["date"].as_str().unwrap_or_default().chars().take(10).collect();
    if !is_sha(&sha) || date.is_empty() {
        return Err(Error::msg("Could not read the repository's newest commit. Check the name."));
    }
    if commit.is_some_and(|c| c != sha) {
        return Err(Error::msg("The repository answered with a different commit."));
    }
    let listing = remote.get_json(&format!("{api}/git/trees/{sha}?recursive=1"))?;
    if listing["truncated"].as_bool() == Some(true) {
        return Err(Error::msg("The repository is too large to read."));
    }
    let mut files = BTreeMap::new();
    let mut unsupported = Vec::new();
    for item in listing["tree"].as_array().into_iter().flatten() {
        let mode = item["mode"].as_str().unwrap_or_default();
        if item["type"].as_str() != Some("blob") || !matches!(mode, "100644" | "100755") {
            continue;
        }
        let (Some(path), Some(blob), Some(size)) = (item["path"].as_str(), item["sha"].as_str(), item["size"].as_u64())
        else {
            continue;
        };
        // An odd file name elsewhere in the repository must not hide usable presets.
        let Ok(path) = validate_path(path) else {
            continue;
        };
        let lower = path.to_ascii_lowercase();
        if [".cfg", ".zip", ".rar", ".7z"].iter().any(|ext| lower.ends_with(ext)) {
            unsupported.push(path.clone());
        }
        files.insert(path.clone(), RepoFile { path, size, sha: blob.to_string() });
    }
    let all: Vec<RepoFile> = files.values().cloned().collect();
    Ok(Inspection {
        repository: repo,
        commit: sha,
        date,
        config_paths: all.iter().filter(|f| is_config_path(&f.path)).map(|f| f.path.clone()).collect(),
        video_paths: all.iter().filter(|f| is_video_path(&f.path)).map(|f| f.path.clone()).collect(),
        addons: standalone_addons(&all).into_iter().filter(|f| f.size <= ADDON_MAX).collect(),
        unsupported,
        files,
    })
}

fn raw_url(repo: &str, commit: &str, path: &str) -> String {
    format!("https://raw.githubusercontent.com/{repo}/{commit}/{}", encode(path))
}

/// Download one picked file at the inspected commit and check it against the tree.
fn download(remote: &dyn Remote, inspection: &Inspection, path: &str) -> Result<String> {
    let file = inspection
        .files
        .get(path)
        .ok_or_else(|| Error::msg(format!("{path} is not in the repository.")))?;
    if file.size > FILE_MAX {
        return Err(Error::msg(format!("{path} is too large for a config file.")));
    }
    let bytes = remote
        .get_bytes(&raw_url(&inspection.repository, &inspection.commit, path))?
        .ok_or_else(|| Error::msg(format!("{path} is missing from the repository.")))?;
    if git_blob_sha(&bytes) != file.sha {
        return Err(Error::msg(format!("{path} does not match the commit it came from, so it was not used.")));
    }
    String::from_utf8(bytes).map_err(|_| Error::msg(format!("{path} is not a text file.")))
}

/// What the player picked in the import form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportChoice {
    pub name: String,
    /// The gameinfo file, None to import video settings only.
    pub config_path: Option<String>,
    /// `None` takes the suggested video file; `Some(None)` skips video.
    pub video_path: Option<Option<String>>,
}

/// The time an import records, ISO 8601.
pub fn now() -> String {
    crate::util::now_iso()
}

/// Download the picked files and keep them as a new custom preset. Network; run off the UI
/// thread from the player's Import. Writes only the app data directory. Returns the new id.
pub fn import(
    remote: &dyn Remote,
    data_dir: &Path,
    inspection: &Inspection,
    choice: &ImportChoice,
    now: &str,
) -> Result<String> {
    let name = choice.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(Error::msg("Enter a preset name (up to 100 characters)."));
    }
    let config_path = choice.config_path.as_deref().map(validate_path).transpose()?;
    if let Some(path) = &config_path
        && (!is_config_path(path) || !inspection.config_paths.contains(path))
    {
        return Err(Error::msg("Choose a gameinfo.gi or gameinfo.txt file from the repository."));
    }
    let video_path = match &choice.video_path {
        Some(path) => path.as_deref().map(validate_path).transpose()?,
        None => suggested_video_path(config_path.as_deref(), &inspection.video_paths)
            .map_err(|()| Error::msg("Several video files match this config. Choose one."))?,
    };
    if let Some(path) = &video_path
        && !inspection.video_paths.contains(path)
    {
        return Err(Error::msg("That video file is not in the repository."));
    }
    if config_path.is_none() && video_path.is_none() {
        return Err(Error::msg("Choose a gameinfo or video file to import."));
    }
    let mut store = read_store(data_dir)?;
    if store.presets.len() >= LIMIT {
        return Err(Error::msg(format!("Remove a custom preset before adding another (limit {LIMIT}).")));
    }
    // Every config is a diff against the stock file; fetch it once if this is the first use.
    if !fetch::file_path(data_dir, super::catalog::BASELINE_ID, "gameinfo.gi").exists() {
        fetch::refresh(remote, data_dir, now)?;
    }
    let config = config_path.as_deref().map(|p| download(remote, inspection, p)).transpose()?;
    let video = video_path.as_deref().map(|p| download(remote, inspection, p)).transpose()?;

    let id = new_id(&inspection.repository, now, &store);
    if let Some(text) = &config {
        write_atomic(&fetch::file_path(data_dir, &id, "gameinfo.gi"), normalize_gameinfo(text).as_bytes())?;
    }
    if let Some(text) = &video {
        write_atomic(&fetch::file_path(data_dir, &id, "video.txt"), text.as_bytes())?;
    }
    let preset = CustomPreset {
        id: id.clone(),
        name: name.to_string(),
        repo: inspection.repository.clone(),
        commit: inspection.commit.clone(),
        date: inspection.date.clone(),
        config_path,
        video_path,
        toggles: Vec::new(),
        addons: inspection.addons.clone(),
        imported: Vec::new(),
        added_at: now.to_string(),
    };
    // It must build before it is kept.
    if let Some(baseline) = baseline(data_dir) {
        build(&preset, data_dir, &baseline, None)?;
    }
    store.presets.push(preset);
    write_store(data_dir, &store)?;
    Ok(id)
}

fn new_id(repo: &str, now: &str, store: &Store) -> String {
    let seed = git_blob_sha(format!("{repo}\n{now}\n{}", store.presets.len()).as_bytes());
    let mut id = format!("{ID_PREFIX}{}", &seed[..12]);
    let mut n = 1;
    while store.get(&id).is_some() {
        id = format!("{ID_PREFIX}{}-{n}", &seed[..12]);
        n += 1;
    }
    id
}

// --- managing ------------------------------------------------------------------------------

/// Forget a custom preset and its snapshot. A gameinfo.gi it was applied to still turns off as
/// usual, since the markers carry what removal needs. App data only.
pub fn delete(data_dir: &Path, id: &str) -> Result<()> {
    let mut store = read_store(data_dir)?;
    let before = store.presets.len();
    store.presets.retain(|p| p.id != id);
    if store.presets.len() == before {
        return Err(Error::msg("That custom preset is gone already."));
    }
    write_store(data_dir, &store)?;
    let dir = fetch::cache_root(data_dir).join("files").join(id);
    if id.starts_with(ID_PREFIX) {
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(addon_dir(data_dir, id));
    }
    Ok(())
}

/// Show or hide the built-in presets. App data only.
pub fn set_hide_builtins(data_dir: &Path, hide: bool) -> Result<()> {
    let mut store = read_store(data_dir)?;
    store.hide_builtins = hide;
    write_store(data_dir, &store)
}

/// Check switches against the settings the preset can turn into one.
pub fn validate_toggles(toggles: &[Toggle], available: &[(String, String)]) -> Result<Vec<Toggle>> {
    if toggles.len() > TOGGLES_MAX {
        return Err(Error::msg("Too many switches."));
    }
    let mut seen = std::collections::BTreeSet::new();
    toggles
        .iter()
        .map(|t| {
            let label = t.label.trim();
            if !available.iter().any(|(k, _)| k == &t.key)
                || !seen.insert(t.key.clone())
                || label.is_empty()
                || label.chars().count() > 100
            {
                return Err(Error::msg("Each switch needs a setting from this preset and a label."));
            }
            Ok(Toggle { key: t.key.clone(), label: label.to_string() })
        })
        .collect()
}

/// Save the preset's switches. App data only.
pub fn set_toggles(data_dir: &Path, id: &str, toggles: &[Toggle]) -> Result<()> {
    let mut store = read_store(data_dir)?;
    let base = baseline(data_dir).ok_or_else(|| Error::msg("The stock gameinfo.gi is not cached yet."))?;
    let preset = store.presets.iter_mut().find(|p| p.id == id).ok_or_else(|| Error::msg("That custom preset is gone."))?;
    let built = build(preset, data_dir, &base, None)?;
    preset.toggles = validate_toggles(toggles, &built.available)?;
    write_store(data_dir, &store)
}

fn addon_dir(data_dir: &Path, id: &str) -> PathBuf {
    fetch::cache_root(data_dir).join("addons").join(id)
}

/// Download a VPK the preset's repository ships, check its size and blob hash, and keep it
/// under the app data directory for the normal import. Network; run off the UI thread from the
/// player's Import. `get` downloads a URL with a byte limit.
pub fn download_addon(
    get: &dyn Fn(&str, u64) -> Result<Vec<u8>>,
    data_dir: &Path,
    id: &str,
    path: &str,
) -> Result<PathBuf> {
    let store = read_store(data_dir)?;
    let preset = store.get(id).ok_or_else(|| Error::msg("That custom preset is gone."))?;
    let addon = preset
        .addons
        .iter()
        .find(|a| a.path == path)
        .filter(|a| a.size <= ADDON_MAX)
        .ok_or_else(|| Error::msg("That VPK is not in the repository or is larger than 128 MiB."))?;
    let bytes = get(&raw_url(&preset.repo, &preset.commit, &addon.path), ADDON_MAX)?;
    if bytes.len() as u64 != addon.size || git_blob_sha(&bytes) != addon.sha {
        return Err(Error::msg("The VPK does not match the repository, so it was not used."));
    }
    let name = file_name(&addon.path);
    if !name.to_ascii_lowercase().ends_with(".vpk") || name.contains(['/', '\\']) {
        return Err(Error::msg("That is not a VPK."));
    }
    let out = addon_dir(data_dir, id).join(name);
    write_atomic(&out, &bytes)?;
    Ok(out)
}

/// Remember that the player imported `path` from this preset. App data only.
pub fn mark_imported(data_dir: &Path, id: &str, path: &str) -> Result<()> {
    let mut store = read_store(data_dir)?;
    if let Some(preset) = store.presets.iter_mut().find(|p| p.id == id)
        && preset.addons.iter().any(|a| a.path == path)
        && !preset.imported.iter().any(|p| p == path)
    {
        preset.imported.push(path.to_string());
        write_store(data_dir, &store)?;
    }
    Ok(())
}

/// A download with a byte limit, for `download_addon`. Network (`raw.githubusercontent.com`).
pub fn http_get(url: &str, limit: u64) -> Result<Vec<u8>> {
    let agent = crate::net::agent(std::time::Duration::from_secs(15 * 60));
    let mut response = agent
        .get(url)
        .header("User-Agent", "sigil-mod-manager")
        .call()
        .map_err(|e| Error::Network(format!("raw.githubusercontent.com: {e}")))?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| Error::Network(format!("raw.githubusercontent.com: {e}")))
}

// --- building ------------------------------------------------------------------------------

/// A custom preset as a catalog config, and what it held back.
#[derive(Debug, Clone)]
pub struct Built {
    pub config: Config,
    /// ConVars the player can turn into switches: `(key, value)`.
    pub available: Vec<(String, String)>,
    /// Settings left out because a repository is not a curated source.
    pub withheld: Vec<String>,
}

fn baseline(data_dir: &Path) -> Option<generate::Parsed> {
    let text = std::fs::read_to_string(fetch::file_path(data_dir, super::catalog::BASELINE_ID, "gameinfo.gi")).ok()?;
    Some(generate::parse(&super::remove_text(&text)))
}

type Defs = (&'static ConfigDef, &'static SourceDef);

fn registry() -> &'static Mutex<HashMap<String, Defs>> {
    static REG: OnceLock<Mutex<HashMap<String, Defs>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

/// The catalog definitions of a custom preset. They live for the process (the catalog's types
/// are `'static`), made once per preset id and name.
fn defs(preset: &CustomPreset) -> Defs {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = reg.get(&preset.id)
        && found.0.name == preset.name
    {
        return *found;
    }
    let leak = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
    let id = leak(&preset.id);
    let source: &'static SourceDef = Box::leak(Box::new(SourceDef {
        id,
        repo: leak(&preset.repo),
        name: leak(&preset.repo),
        license: "See the repository",
        versioning: Versioning::Commit,
        verbatim_video: false,
    }));
    let config: &'static ConfigDef = Box::leak(Box::new(ConfigDef {
        id,
        name: leak(&preset.name),
        author: leak(preset.repo.split('/').next().unwrap_or_default()),
        tier: Tier::Balanced,
        source: id,
        folders: &[],
        file: "gameinfo.gi",
        videos: &[],
        testing: false,
        note: "A custom preset from a GitHub repository you added.",
        gamebanana: None,
    }));
    reg.insert(preset.id.clone(), (config, source));
    (config, source)
}

/// A custom preset's definition, when one was built in this process.
pub fn lookup(id: &str) -> Option<Defs> {
    if !id.starts_with(ID_PREFIX) {
        return None;
    }
    registry().lock().unwrap_or_else(|e| e.into_inner()).get(id).copied()
}

fn scalar(value: &str) -> bool {
    let v = value.trim();
    if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false") {
        return true;
    }
    let v = v.strip_prefix(['+', '-']).unwrap_or(v);
    let (mantissa, exponent) = match v.find(['e', 'E']) {
        Some(at) => (&v[..at], Some(&v[at + 1..])),
        None => (v, None),
    };
    let digits = mantissa.replacen('.', "", 1);
    let mantissa_ok = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && mantissa.matches('.').count() <= 1;
    let exponent_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
    });
    mantissa_ok && exponent_ok
}

fn command(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["exec", "bind", "unbind", "write"].iter().any(|p| k.starts_with(p))
        || matches!(k.as_str(), "alias" | "connect" | "disconnect" | "quit" | "exit" | "log" | "toggle" | "incrementvar")
}

/// Whether a repository may set `key` to `value`: a lowercase convar name, a number or
/// true/false, and not a console command.
pub fn allowed(key: &str, value: &str) -> bool {
    !key.is_empty()
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
        && value.len() <= 512
        && scalar(value)
        && !command(key)
}

/// Build a custom preset from its snapshot under today's rules. Disk only.
pub fn build(preset: &CustomPreset, data_dir: &Path, baseline: &generate::Parsed, game: Option<&GameConvars>) -> Result<Built> {
    let (def, source) = defs(preset);
    let gameinfo = match std::fs::read_to_string(fetch::file_path(data_dir, &preset.id, "gameinfo.gi")) {
        Ok(text) => text,
        Err(_) if preset.config_path.is_none() => String::new(),
        Err(error) => return Err(error.into()),
    };
    let video = std::fs::read_to_string(fetch::file_path(data_dir, &preset.id, "video.txt")).ok();
    let videos: Vec<(String, String)> = video.map(|t| ("video.txt".to_string(), t)).into_iter().collect();
    let meta = CachedConfig {
        id: preset.id.clone(),
        version: preset.commit.chars().take(7).collect(),
        commit: preset.commit.clone(),
        date: preset.date.clone(),
        path: preset.config_path.clone().or_else(|| preset.video_path.clone()).unwrap_or_default(),
        blob: String::new(),
        videos: Vec::new(),
    };
    let mut config = if gameinfo.is_empty() {
        // Video only: nothing differs from stock.
        let mut c = Config::from_texts(def, &meta, baseline, "", &videos, game);
        c.body = generate::Body::default();
        for set in &mut c.videos {
            set.label = "Default".into();
        }
        c
    } else {
        Config::from_texts(def, &meta, baseline, &gameinfo, &videos, game)
    };
    config.source = source;

    let mut held = std::mem::take(&mut config.body.held);
    // What the generator already held back (commands, text values) is withheld too.
    let mut withheld: Vec<String> = held.iter().map(|h| h.key.clone()).collect();
    let mut hold = |key: &str, place: &str, held: &mut Vec<Held>| {
        withheld.push(key.to_string());
        held.push(Held {
            key: key.to_string(),
            place: place.to_string(),
            reason: "A custom preset sets plain values only: no engine edits, commands or text values.".into(),
        });
    };
    for op in std::mem::take(&mut config.body.section_ops) {
        hold(&op.key, &op.path.join("/"), &mut held);
    }
    let convars = std::mem::take(&mut config.body.convars);
    for (key, value) in convars {
        if allowed(&key, &value) {
            config.body.convars.push((key, value));
        } else {
            hold(&key, "ConVars", &mut held);
        }
    }
    let opt_ins = std::mem::take(&mut config.body.opt_ins);
    for opt in opt_ins {
        if allowed(&opt.key, &opt.value) {
            config.body.opt_ins.push(opt);
        } else {
            hold(&opt.key, "ConVars", &mut held);
        }
    }
    for set in &mut config.videos {
        let settings = std::mem::take(&mut set.settings);
        for (key, value) in settings {
            if allowed(&key, &value) {
                set.settings.push((key, value));
            } else {
                hold(&key, "video.txt", &mut held);
            }
        }
    }
    config.videos.retain(|set| !set.settings.is_empty());
    config.body.held = held;

    // A switch cannot control a value the same preset sets again through video.txt.
    let video_keys: std::collections::BTreeSet<&String> =
        config.videos.iter().flat_map(|set| set.settings.iter().map(|(k, _)| k)).collect();
    let available: Vec<(String, String)> =
        config.body.convars.iter().filter(|(k, _)| !video_keys.contains(k)).cloned().collect();
    // A later rule change may withhold a key a switch named; that switch drops.
    let toggles: Vec<&Toggle> = preset.toggles.iter().filter(|t| available.iter().any(|(k, _)| k == &t.key)).collect();
    let (switched, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut config.body.convars)
        .into_iter()
        .partition(|(k, _)| toggles.iter().any(|t| &t.key == k));
    config.body.convars = kept;
    for (key, value) in switched {
        config.body.opt_ins.push(OptIn { key, value, group: Group::Visibility, stock: None });
    }
    // A switch edit changes what the preset writes although the commit stays.
    if !preset.toggles.is_empty() {
        let joined: String = preset.toggles.iter().map(|t| format!("{}={}\n", t.key, t.label)).collect();
        config.version = format!("{}.{}", config.version, &git_blob_sha(joined.as_bytes())[..8]);
    }
    Ok(Built { config, available, withheld })
}

/// Every custom preset that builds, for `performance::load`. Disk only.
pub fn configs(data_dir: &Path, baseline: &generate::Parsed, game: Option<&GameConvars>) -> Vec<Config> {
    let Ok(store) = read_store(data_dir) else {
        return Vec::new();
    };
    store
        .presets
        .iter()
        .filter_map(|preset| build(preset, data_dir, baseline, game).ok().map(|b| b.config))
        .collect()
}

#[cfg(test)]
mod tests;
