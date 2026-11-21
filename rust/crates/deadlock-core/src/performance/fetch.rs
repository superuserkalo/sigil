//! Fetch configs from their authors' GitHub repositories and keep them on disk.
//!
//! Hosts: `api.github.com` resolves a source's newest release (a tag, or the
//! newest commit for a source that does not tag) and lists the files at that
//! commit; `raw.githubusercontent.com` serves the files at that commit. Every
//! file is checked against the git blob hash the commit's tree records, so what
//! is cached is byte for byte what the author committed. Nothing about the
//! player is sent: these are anonymous GETs of public files.
//!
//! The cache lives under the app data directory. Raw upstream files are kept,
//! not generated bodies, so a newer Sigil reclassifies them without a fetch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};

use super::catalog::{self, ConfigDef, SourceDef, Versioning};
use super::generate;
use crate::error::{Error, Result};
use crate::util::write_atomic;

const USER_AGENT: &str = "sigil-mod-manager";
const TIMEOUT_SECONDS: u64 = 20;
const CACHE_DIR: &str = "performance";
const INDEX: &str = "index.json";

/// Valve's convar list, as SteamTracking dumps it from each game build.
pub const CONVARS_REPO: &str = "SteamTracking/GameTracking-Deadlock";
pub const CONVARS_PATH: &str = "DumpSource2/convars.txt";
pub const CONVARS_ID: &str = "game";
pub const CONVARS_FILE: &str = "convars.txt";

/// One fetched config as the cache records it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CachedConfig {
    pub id: String,
    /// The upstream version: a tag, or the version the file states.
    pub version: String,
    pub commit: String,
    /// When the author last changed this config, yyyy-mm-dd.
    pub date: String,
    pub path: String,
    /// Git blob hash of the cached gameinfo.gi.
    pub blob: String,
    /// The video.txt variants cached next to it: (file name, blob hash).
    #[serde(default)]
    pub videos: Vec<(String, String)>,
}

/// The stock file the configs are diffed against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CachedBaseline {
    pub commit: String,
    pub date: String,
    pub blob: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheIndex {
    #[serde(default)]
    pub checked_at: Option<String>,
    #[serde(default)]
    pub baseline: Option<CachedBaseline>,
    #[serde(default)]
    pub configs: BTreeMap<String, CachedConfig>,
    /// Per config: why the newest upstream file was refused, if it was.
    #[serde(default)]
    pub errors: BTreeMap<String, String>,
    /// Commit of the cached convar list.
    #[serde(default)]
    pub convars_commit: Option<String>,
}

pub fn cache_root(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_DIR)
}

fn index_path(data_dir: &Path) -> PathBuf {
    cache_root(data_dir).join(INDEX)
}

pub(crate) fn file_path(data_dir: &Path, id: &str, name: &str) -> PathBuf {
    cache_root(data_dir).join("files").join(id).join(name)
}

pub fn read_index(data_dir: &Path) -> CacheIndex {
    std::fs::read_to_string(index_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_index(data_dir: &Path, index: &CacheIndex) -> Result<()> {
    let text = serde_json::to_string_pretty(index)?;
    write_atomic(&index_path(data_dir), text.as_bytes())
}

/// Git's object id for a blob: sha1 of `blob <len>\0<bytes>`.
pub fn git_blob_sha(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What a refresh did, for the screen's one line.
#[derive(Debug, Clone, Default)]
pub struct RefreshReport {
    pub updated: Vec<String>,
    pub errors: Vec<String>,
}

/// The network half, behind a trait so tests can serve files from memory.
pub trait Remote {
    fn get_json(&self, url: &str) -> Result<Value>;
    /// `None` on 404.
    fn get_bytes(&self, url: &str) -> Result<Option<Vec<u8>>>;
}

pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        let agent = crate::net::builder()
            .middleware(crate::net::gate)
            .timeout_global(Some(Duration::from_secs(TIMEOUT_SECONDS)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Http { agent }
    }
}

impl Http {
    fn get(&self, url: &str, accept: &str) -> Result<(u16, Vec<u8>)> {
        let mut response = self
            .agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", accept)
            .call()
            .map_err(|error| Error::Network(format!("{}: {error}", host(url))))?;
        let status = response.status().as_u16();
        let bytes = response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_vec()
            .map_err(|error| Error::Network(format!("{}: {error}", host(url))))?;
        Ok((status, bytes))
    }
}

fn host(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
}

impl Remote for Http {
    fn get_json(&self, url: &str) -> Result<Value> {
        let (status, bytes) = self.get(url, "application/vnd.github+json")?;
        if status == 403 || status == 429 {
            return Err(Error::Network(
                "GitHub is rate limiting requests from this network. Try again in an hour.".into(),
            ));
        }
        if !(200..300).contains(&status) {
            return Err(Error::Network(format!("GitHub returned HTTP {status}")));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn get_bytes(&self, url: &str) -> Result<Option<Vec<u8>>> {
        let (status, bytes) = self.get(url, "text/plain")?;
        if status == 404 {
            return Ok(None);
        }
        if !(200..300).contains(&status) {
            return Err(Error::Network(format!(
                "GitHub returned HTTP {status} for a file"
            )));
        }
        Ok(Some(bytes))
    }
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            let mut out = String::new();
            for byte in segment.bytes() {
                if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                    out.push(byte as char);
                } else {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A resolved upstream release of one source.
struct Release {
    commit: String,
    /// The tag, for a tagged source.
    tag: Option<String>,
    date: String,
    /// path -> blob sha at that commit.
    tree: BTreeMap<String, String>,
}

fn date_of(value: &Value) -> String {
    value
        .as_str()
        .map(|date| date.chars().take(10).collect())
        .unwrap_or_default()
}

fn resolve(remote: &dyn Remote, source: &SourceDef) -> Result<Release> {
    let api = format!("https://api.github.com/repos/{}", source.repo);
    let (commit, tag, date) = match source.versioning {
        Versioning::Tag => {
            let release = remote.get_json(&format!("{api}/releases/latest"))?;
            let tag = release["tag_name"]
                .as_str()
                .ok_or_else(|| Error::Network(format!("{} has no published release", source.repo)))?
                .to_string();
            let date = date_of(&release["published_at"]);
            let reference =
                remote.get_json(&format!("{api}/git/ref/tags/{}", encode_path(&tag)))?;
            let mut sha = reference["object"]["sha"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if reference["object"]["type"].as_str() == Some("tag") {
                let annotated = remote.get_json(&format!("{api}/git/tags/{sha}"))?;
                sha = annotated["object"]["sha"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
            (sha, Some(tag), date)
        }
        Versioning::Commit => {
            let list = remote.get_json(&format!("{api}/commits?per_page=1"))?;
            let head = &list[0];
            let sha = head["sha"].as_str().unwrap_or_default().to_string();
            (sha, None, date_of(&head["commit"]["committer"]["date"]))
        }
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Network(format!(
            "{} did not resolve to a commit",
            source.repo
        )));
    }
    let listing = remote.get_json(&format!("{api}/git/trees/{commit}?recursive=1"))?;
    let mut tree = BTreeMap::new();
    for item in listing["tree"].as_array().into_iter().flatten() {
        if item["type"].as_str() == Some("blob")
            && let (Some(path), Some(sha)) = (item["path"].as_str(), item["sha"].as_str())
        {
            tree.insert(path.to_string(), sha.to_string());
        }
    }
    Ok(Release {
        commit,
        tag,
        date,
        tree,
    })
}

/// Download `path` at the release's commit and check it against the tree.
fn download(
    remote: &dyn Remote,
    source: &SourceDef,
    release: &Release,
    path: &str,
) -> Result<Vec<u8>> {
    let expected = release
        .tree
        .get(path)
        .ok_or_else(|| Error::Network(format!("{path} is not in {}", source.repo)))?;
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/{}",
        source.repo,
        release.commit,
        encode_path(path)
    );
    let bytes = remote
        .get_bytes(&url)?
        .ok_or_else(|| Error::Network(format!("{path} is missing from {}", source.repo)))?;
    let actual = git_blob_sha(&bytes);
    if &actual != expected {
        return Err(Error::Network(format!(
            "{path} does not match the commit it came from, so it was not used"
        )));
    }
    Ok(bytes)
}

/// Last commit that touched `folder`, for a source without tags: when the
/// author changed this config, not the repository. `(short sha, date)`.
fn folder_commit(
    remote: &dyn Remote,
    source: &SourceDef,
    folder: &str,
) -> Option<(String, String)> {
    let url = format!(
        "https://api.github.com/repos/{}/commits?per_page=1&path={}",
        source.repo,
        encode_path(folder)
    );
    let list = remote.get_json(&url).ok()?;
    let sha: String = list[0]["sha"].as_str()?.chars().take(7).collect();
    let date = date_of(&list[0]["commit"]["committer"]["date"]);
    (!date.is_empty()).then_some((sha, date))
}

fn store(data_dir: &Path, id: &str, name: &str, bytes: &[u8]) -> Result<()> {
    write_atomic(&file_path(data_dir, id, name), bytes)
}

/// Cache a config the player downloaded from GameBanana. `stock` seeds the
/// baseline when no GitHub fetch has run yet, so the tier list can diff against
/// the player's own stock file. No network.
pub fn store_hosted(
    data_dir: &Path,
    meta: &CachedConfig,
    gameinfo: &str,
    videos: &[(String, String)],
    stock: Option<&str>,
) -> Result<()> {
    store(data_dir, &meta.id, "gameinfo.gi", gameinfo.as_bytes())?;
    for (name, text) in videos {
        store(data_dir, &meta.id, name, text.as_bytes())?;
    }
    let baseline = file_path(data_dir, catalog::BASELINE_ID, "gameinfo.gi");
    if !baseline.exists()
        && let Some(stock) = stock
    {
        store(data_dir, catalog::BASELINE_ID, "gameinfo.gi", stock.as_bytes())?;
    }
    let mut index = read_index(data_dir);
    index.configs.insert(meta.id.clone(), meta.clone());
    write_index(data_dir, &index)
}

/// Check every source for a newer release and cache what changed. A source
/// that fails keeps its cached configs; a config whose new file fails the
/// sanity gates keeps its previous version.
pub fn refresh(remote: &dyn Remote, data_dir: &Path, now: &str) -> Result<RefreshReport> {
    let mut index = read_index(data_dir);
    let mut report = RefreshReport::default();
    let mut releases: BTreeMap<&'static str, Release> = BTreeMap::new();
    let mut any = false;
    for source in catalog::SOURCES {
        if source.id == "gamebanana" || source.id == "mod" {
            continue;
        }
        match resolve(remote, source) {
            Ok(release) => {
                releases.insert(source.id, release);
                any = true;
            }
            Err(error) => report.errors.push(format!("{}: {error}", source.repo)),
        }
    }
    if !any {
        return Err(Error::Network(
            report
                .errors
                .first()
                .cloned()
                .unwrap_or_else(|| "No source answered.".into()),
        ));
    }

    // The stock file first: every config is a diff against it.
    let base_def = catalog::BASELINE;
    let base_source = catalog::source(base_def.source);
    if let Some(release) = releases.get(base_def.source) {
        let blob = release.tree.get(base_def.path).cloned();
        let cached = index.baseline.as_ref().map(|b| b.blob.clone());
        if blob.is_some() && blob != cached {
            match download(remote, base_source, release, base_def.path).and_then(|bytes| {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                generate::validate_gameinfo(&text)
                    .map_err(|e| Error::msg(format!("stock gameinfo.gi {e}")))?;
                Ok(bytes)
            }) {
                Ok(bytes) => {
                    store(data_dir, catalog::BASELINE_ID, "gameinfo.gi", &bytes)?;
                    index.baseline = Some(CachedBaseline {
                        commit: release.commit.clone(),
                        date: release.date.clone(),
                        blob: blob.unwrap_or_default(),
                    });
                }
                Err(error) => report.errors.push(error.to_string()),
            }
        }
    }
    let baseline_text =
        std::fs::read_to_string(file_path(data_dir, catalog::BASELINE_ID, "gameinfo.gi")).ok();

    for def in catalog::CONFIGS {
        if def.gamebanana.is_some() {
            continue;
        }
        let source = catalog::source(def.source);
        let Some(release) = releases.get(def.source) else {
            continue;
        };
        match refresh_config(
            remote,
            data_dir,
            def,
            source,
            release,
            &index,
            baseline_text.as_deref(),
        ) {
            Ok(Some(entry)) => {
                report.updated.push(def.name.to_string());
                index.errors.remove(def.id);
                index.configs.insert(def.id.to_string(), entry);
            }
            Ok(None) => {
                index.errors.remove(def.id);
            }
            Err(error) => {
                index.errors.insert(def.id.to_string(), error.to_string());
                report.errors.push(format!("{}: {error}", def.name));
            }
        }
    }
    if let Err(error) = refresh_convars(remote, data_dir, &mut index) {
        report.errors.push(format!("Valve's convar list: {error}"));
    }
    index.checked_at = Some(now.to_string());
    write_index(data_dir, &index)?;
    Ok(report)
}

/// Valve's convar list says which convars the game refuses from gameinfo.gi.
/// Read for labels, to skip lines that would do nothing, and as the convar editor's catalogue
/// (`autoexec::convars` validates every name and default before use), so a sanity
/// check stands in for the blob check (the repository tree is too large to list).
fn refresh_convars(remote: &dyn Remote, data_dir: &Path, index: &mut CacheIndex) -> Result<()> {
    let list = remote.get_json(&format!(
        "https://api.github.com/repos/{CONVARS_REPO}/commits?per_page=1&path={}",
        encode_path(CONVARS_PATH)
    ))?;
    let commit = list[0]["sha"].as_str().unwrap_or_default().to_string();
    if commit.len() != 40 {
        return Err(Error::Network("no commit for the convar list".into()));
    }
    let path = file_path(data_dir, CONVARS_ID, CONVARS_FILE);
    if index.convars_commit.as_deref() == Some(commit.as_str()) && path.exists() {
        return Ok(());
    }
    let url = format!(
        "https://raw.githubusercontent.com/{CONVARS_REPO}/{commit}/{}",
        encode_path(CONVARS_PATH)
    );
    let bytes = remote
        .get_bytes(&url)?
        .ok_or_else(|| Error::Network("the convar list is missing".into()))?;
    let parsed = super::GameConvars::parse(&String::from_utf8_lossy(&bytes));
    if parsed.len() < 1000 {
        return Err(Error::Network(format!(
            "the convar list has only {} entries",
            parsed.len()
        )));
    }
    store(data_dir, CONVARS_ID, CONVARS_FILE, &bytes)?;
    index.convars_commit = Some(commit);
    Ok(())
}

/// Fetch Valve's convar list alone, for the player's Download the list button. A damaged cached
/// copy is replaced even when upstream has not moved.
pub fn refresh_convar_list(remote: &dyn Remote, data_dir: &Path) -> Result<()> {
    let mut index = read_index(data_dir);
    if crate::autoexec::convars::load_catalog(data_dir).convars.is_empty() {
        index.convars_commit = None;
    }
    refresh_convars(remote, data_dir, &mut index)?;
    // Read again: a Performance refresh may have finished during the download.
    let commit = index.convars_commit;
    let mut index = read_index(data_dir);
    index.convars_commit = commit;
    write_index(data_dir, &index)
}

/// The first of a config's candidate folders the release has.
fn locate<'a>(def: &'a ConfigDef, release: &Release) -> Option<&'a str> {
    def.folders
        .iter()
        .copied()
        .find(|folder| release.tree.contains_key(&join(folder, def.file)))
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// Fetch one config when its files changed. `Ok(None)`: already current.
fn refresh_config(
    remote: &dyn Remote,
    data_dir: &Path,
    def: &ConfigDef,
    source: &SourceDef,
    release: &Release,
    index: &CacheIndex,
    baseline: Option<&str>,
) -> Result<Option<CachedConfig>> {
    let folder =
        locate(def, release).ok_or_else(|| Error::msg("the author no longer ships this config"))?;
    let path = join(folder, def.file);
    let blob = release.tree.get(&path).cloned().unwrap_or_default();
    let videos: Vec<(String, String)> = def
        .videos
        .iter()
        .filter_map(|name| {
            release
                .tree
                .get(&join(folder, name))
                .map(|sha| (name.to_string(), sha.clone()))
        })
        .collect();
    let cached = index.configs.get(def.id);
    if let Some(cached) = cached
        && cached.blob == blob
        && cached.videos == videos
        && file_path(data_dir, def.id, "gameinfo.gi").exists()
    {
        return Ok(None);
    }

    let bytes = download(remote, source, release, &path)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    generate::validate_gameinfo(&text).map_err(|e| Error::msg(format!("gameinfo.gi {e}")))?;
    if let Some(baseline) = baseline {
        let base = generate::parse(baseline);
        let config = generate::parse(&text);
        let body = generate::generate(&base, &config);
        generate::validate_body(&base, &config, &body).map_err(Error::msg)?;
    }
    let mut video_files = Vec::new();
    for (name, _) in &videos {
        let video = download(remote, source, release, &join(folder, name))?;
        generate::validate_video(&String::from_utf8_lossy(&video)).map_err(Error::msg)?;
        video_files.push((name.clone(), video));
    }

    // Replace the folder only once everything downloaded and passed.
    let dir = cache_root(data_dir).join("files").join(def.id);
    let _ = std::fs::remove_dir_all(&dir);
    store(data_dir, def.id, "gameinfo.gi", &bytes)?;
    for (name, video) in &video_files {
        store(data_dir, def.id, name, video)?;
    }

    let (version, date) = match (&release.tag, source.versioning) {
        (Some(tag), _) => (tag.clone(), release.date.clone()),
        (None, _) => {
            let (sha, date) = folder_commit(remote, source, folder)
                .unwrap_or_else(|| (release.commit[..7].to_string(), release.date.clone()));
            // A file that states no version is known by the commit that last changed it.
            (catalog::stated_version(&text).unwrap_or(sha), date)
        }
    };
    Ok(Some(CachedConfig {
        id: def.id.to_string(),
        version,
        commit: release.commit.clone(),
        date,
        path,
        blob,
        videos,
    }))
}

/// The cache is older than `hours`, or empty.
pub fn stale(index: &CacheIndex, now: chrono::DateTime<chrono::Utc>, hours: i64) -> bool {
    let Some(checked) = index
        .checked_at
        .as_deref()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
    else {
        return true;
    };
    now.signed_duration_since(checked.with_timezone(&chrono::Utc)) > chrono::Duration::hours(hours)
}
