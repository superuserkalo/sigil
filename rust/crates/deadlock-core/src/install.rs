//! Browse install pipeline: download a mod archive, extract it, validate the
//! payload, choose an addon slot and deploy it behind approval.
//!
//! The stages are separate on purpose so a caller can stop at any one of them:
//!
//! 1. [`InstallSource::parse`] turns what the user typed into a mod id, an
//!    archive URL or a local file.
//! 2. [`plan_install`] downloads inside the workspace, extracts, classifies the
//!    candidate, validates it with [`crate::payload`], picks a slot and writes a
//!    plan file. It never touches the game install.
//! 3. [`apply_install`] is the only function here that writes into
//!    `game/citadel/addons`, and it refuses unless `approve` is true.
//!
//! Network access is limited to the GameBanana download hosts plus an explicit
//! local mirror, HTTPS only unless the mirror names `http://` itself. There is
//! no shell: the 7z and rar backends are resolved binaries run with an argument
//! array, never through a shell and never downloaded on demand.
//!
//! Formats: ZIP is read in process (stored and deflate). 7z and rar need
//! `DAH_7Z` / `DAH_UNRAR` or the tool on `PATH`; when it is missing the error
//! names the format and the tool. That gap is real: Deadlock uploads on
//! GameBanana are commonly `.7z`.

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::deploy::{addons_dir, gameinfo_path, repair_gameinfo};
use crate::discovery::{
    read_addons_status, read_game_info_status, discover_deadlock_install, format_slot_file_name,
    InstallDiscovery, InstallDiscoveryOptions,
};
use crate::error::{Error, Result};
use crate::payload::{
    format_payload_issues, has_errors, validate_payload, PayloadIssue, PayloadOptions, Severity,
};
use crate::util::{now_iso, write_atomic};
use crate::vpk::{read_vpk_directory_with, ReadVpkOptions};
use crate::vpk_write::{pack_vpk, PackOptions};

/// Schema version for plan files and journal entries.
pub const INSTALL_SCHEMA: u32 = 1;
/// Hard cap on one downloaded archive.
pub const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Hard cap on one extracted tree.
pub const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Hard cap on archive entries.
pub const MAX_ARCHIVE_ENTRIES: usize = 10_000;
/// Compression ratio cap for entries above [`RATIO_FLOOR_BYTES`].
pub const MAX_COMPRESSION_RATIO: u64 = 500;
/// Entries smaller than this are exempt from the ratio cap, so a small header
/// full of zeroes does not look like a bomb.
pub const RATIO_FLOOR_BYTES: u64 = 8 * 1024 * 1024;
/// Whole-request budget for a download.
pub const DOWNLOAD_TIMEOUT_SECONDS: u64 = 300;
/// Redirects are followed by hand so each hop can be checked.
pub const MAX_REDIRECTS: usize = 5;

pub const DOWNLOADS_DIR_NAME: &str = "downloads";
/// Install state inside the workspace. Plans, backups and the journal live here.
pub const INSTALL_STATE_DIR: &str = ".dah/install";
pub const JOURNAL_FILE_NAME: &str = "installs.jsonl";
/// Environment override for a local mirror origin, for example
/// `http://127.0.0.1:8080`.
pub const MIRROR_ENV: &str = "DAH_INSTALL_MIRROR";
/// Environment override for the 7z binary.
pub const SEVEN_ZIP_ENV: &str = "DAH_7Z";
/// Environment override for the unrar binary.
pub const UNRAR_ENV: &str = "DAH_UNRAR";

/// Hosts GameBanana serves download pages and files from.
pub const GAMEBANANA_DOWNLOAD_HOSTS: [&str; 3] =
    ["gamebanana.com", "www.gamebanana.com", "files.gamebanana.com"];

const GAMEBANANA_PROFILE_URL: &str = "https://gamebanana.com/apiv11/Mod";
const GAMEBANANA_USER_AGENT: &str = "dah/0.0.0";
const API_TIMEOUT_SECONDS: u64 = 30;
const BODY_SNIPPET_CHARS: usize = 200;
const READ_BUFFER_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

/// Where an install comes from. The enum is stored in the plan file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum InstallSource {
    /// Bare GameBanana mod id, resolved through the public API v11.
    GameBananaId(u64),
    /// Direct https URL to an archive.
    DirectUrl(String),
    /// Archive already on disk. Hashed in place, never copied.
    LocalFile(PathBuf),
}

impl InstallSource {
    /// Parse `715100`, `https://gamebanana.com/mods/715100`,
    /// `https://files.gamebanana.com/mods/thing.zip` or a local archive path.
    pub fn parse(target: &str) -> Result<Self> {
        let text = target.trim();

        if text.is_empty() {
            return Err(Error::msg(
                "An install target is required: a GameBanana mod id, a mod URL, an https archive URL or a local .zip/.7z/.rar file.",
            ));
        }

        if text.chars().all(|character| character.is_ascii_digit()) {
            let id: u64 = text.parse().map_err(|_| {
                Error::msg(format!("GameBanana mod id {text} is not a number."))
            })?;
            if id == 0 {
                return Err(Error::msg(
                    "GameBanana mod id 0 is not a submission. Pass the number from the mod URL.",
                ));
            }
            return Ok(Self::GameBananaId(id));
        }

        if let Some(rest) = text.strip_prefix("file://") {
            let path = PathBuf::from(rest);
            if !path.is_file() {
                return Err(Error::msg(format!(
                    "file:// target {} is not a file.",
                    path.display()
                )));
            }
            return Ok(Self::LocalFile(path));
        }

        if text.starts_with("http://") || text.starts_with("https://") {
            let url = WebUrl::parse(text)?;

            // A mod page is not an archive. Read the id and resolve the file list.
            if is_gamebanana_host(&url.host)
                && let Some(rest) = url.path.strip_prefix("/mods/")
                && let Some(id) = rest
                    .split(['/', '?', '#'])
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
            {
                return Ok(Self::GameBananaId(id));
            }

            return Ok(Self::DirectUrl(text.to_string()));
        }

        let path = Path::new(text);
        if path.is_file() {
            return Ok(Self::LocalFile(path.to_path_buf()));
        }

        Err(Error::msg(format!(
            "Could not read {target:?} as an install target. Use a GameBanana mod id such as 715100, a gamebanana.com mod URL, an https archive URL or a local archive file."
        )))
    }

    /// Stable folder name under `<workspace>/downloads`.
    pub fn cache_key(&self) -> String {
        match self {
            Self::GameBananaId(id) => format!("gamebanana-{id}"),
            Self::DirectUrl(url) => format!("url-{}", short_hash(url)),
            Self::LocalFile(path) => {
                format!("local-{}", short_hash(&path.to_string_lossy()))
            }
        }
    }

    /// Short label used in plans, journals and text output.
    pub fn label(&self) -> String {
        match self {
            Self::GameBananaId(id) => format!("gamebanana:{id}"),
            Self::DirectUrl(url) => url.clone(),
            Self::LocalFile(path) => path.display().to_string(),
        }
    }
}

/// One install request. Built by the CLI from the arguments it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    /// The value the user typed, kept for the plan and the journal.
    pub target: String,
    pub source: InstallSource,
    /// Requested slot. None means the lowest free slot.
    pub slot: Option<u8>,
    /// Replace an occupied slot instead of refusing.
    pub force: bool,
    pub workspace: PathBuf,
}

impl InstallRequest {
    pub fn new(target: impl Into<String>, workspace: PathBuf) -> Result<Self> {
        let target = target.into();
        let source = InstallSource::parse(&target)?;
        Ok(Self {
            target,
            source,
            slot: None,
            force: false,
            workspace,
        })
    }

    pub fn with_slot(mut self, slot: Option<u8>) -> Self {
        self.slot = slot;
        self
    }

    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

// ---------------------------------------------------------------------------
// URLs and the download allowlist
// ---------------------------------------------------------------------------

/// The parts of an HTTP URL this crate needs. Deliberately small: no userinfo,
/// no IPv6 literals, no relative forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUrl {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// Path plus query, always starting with `/`.
    pub path: String,
}

impl WebUrl {
    pub fn parse(input: &str) -> Result<Self> {
        let text = input.trim();

        if text.is_empty() {
            return Err(Error::msg("A URL is required."));
        }

        if text
            .chars()
            .any(|character| character.is_control() || character == ' ')
        {
            return Err(Error::msg(format!(
                "URL {text} contains whitespace or control characters."
            )));
        }

        let (scheme, rest) = text.split_once("://").ok_or_else(|| {
            Error::msg(format!(
                "URL {text} has no scheme. Use https://gamebanana.com or http://127.0.0.1:8080 for a local mirror."
            ))
        })?;
        let scheme = scheme.to_ascii_lowercase();

        if scheme != "https" && scheme != "http" {
            return Err(Error::msg(format!(
                "URL scheme {scheme} is not allowed. Only https is allowed, or http for an explicit local mirror."
            )));
        }

        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let tail = &rest[authority_end..];

        if authority.is_empty() {
            return Err(Error::msg(format!("URL {text} has no host.")));
        }

        if authority.contains('@') {
            return Err(Error::msg(format!(
                "URL {text} carries credentials. Download URLs never need them."
            )));
        }

        if authority.starts_with('[') {
            return Err(Error::msg(format!(
                "URL {text} uses an IPv6 literal host, which is not in the download allowlist."
            )));
        }

        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port.parse::<u16>().map_err(|_| {
                    Error::msg(format!("URL {text} has an invalid port {port:?}."))
                })?;
                (host.to_string(), port)
            }
            None => (
                authority.to_string(),
                if scheme == "https" { 443 } else { 80 },
            ),
        };

        let host = host.to_ascii_lowercase();

        if host.is_empty() {
            return Err(Error::msg(format!("URL {text} has no host.")));
        }

        if !host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
        {
            return Err(Error::msg(format!(
                "URL {text} has an invalid host name. Use a plain DNS host or a dotted IPv4 address."
            )));
        }

        let path = match tail {
            "" => "/".to_string(),
            value if value.starts_with('/') => value.split('#').next().unwrap_or("/").to_string(),
            value if value.starts_with('?') => {
                format!("/{}", value.split('#').next().unwrap_or("/"))
            }
            _ => {
                return Err(Error::msg(format!(
                    "URL {text} has a fragment before its path."
                )));
            }
        };

        Ok(Self {
            scheme,
            host,
            port,
            path,
        })
    }

    /// `https://gamebanana.com`, `http://127.0.0.1:8080`. Default ports are
    /// omitted so an allowlist entry matches either spelling.
    pub fn origin(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        if self.port == default {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }

    pub fn file_name(&self) -> Option<String> {
        let path = self.path.split('?').next().unwrap_or(&self.path);
        let name = path.rsplit('/').next()?;
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }
}

impl std::fmt::Display for WebUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.origin())?;
        formatter.write_str(&self.path)
    }
}

/// Origins a download may touch, plus its budget.
///
/// Hosts under `gamebanana.com` change: `gamebanana.com/dl/<id>` redirects to
/// `files.gamebanana.com` or to a numbered `filecacheNN.gamebanana.com`. The
/// policy therefore allows the apex domain and any of its subdomains, HTTPS
/// only, and keeps exact origins for the mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPolicy {
    /// Exact origins, for example `https://files.gamebanana.com`.
    pub allowed_origins: Vec<String>,
    /// Apex domains whose `https` subdomains are allowed.
    pub allowed_domains: Vec<String>,
    pub max_bytes: u64,
    pub timeout_seconds: u64,
}

impl DownloadPolicy {
    /// The GameBanana download hosts over HTTPS only.
    pub fn gamebanana() -> Self {
        Self {
            allowed_origins: GAMEBANANA_DOWNLOAD_HOSTS
                .iter()
                .map(|host| format!("https://{host}"))
                .collect(),
            allowed_domains: vec!["gamebanana.com".to_string()],
            max_bytes: MAX_DOWNLOAD_BYTES,
            timeout_seconds: DOWNLOAD_TIMEOUT_SECONDS,
        }
    }

    /// Allow one extra origin, for example a local mirror. A bare host defaults
    /// to https; pass `http://127.0.0.1:8080` to allow plain HTTP for a local
    /// server.
    pub fn with_mirror(mut self, value: &str) -> Result<Self> {
        let text = value.trim();
        if text.is_empty() {
            return Ok(self);
        }

        let origin = if text.contains("://") {
            WebUrl::parse(text)?.origin()
        } else {
            let url = WebUrl::parse(&format!("https://{text}"))?;
            url.origin()
        };

        if !self.allowed_origins.contains(&origin) {
            self.allowed_origins.push(origin);
        }

        Ok(self)
    }

    /// Read `DAH_INSTALL_MIRROR` when set.
    pub fn from_environment() -> Result<Self> {
        let policy = Self::gamebanana();
        match env::var(MIRROR_ENV) {
            Ok(value) if !value.trim().is_empty() => policy.with_mirror(value.trim()),
            _ => Ok(policy),
        }
    }

    pub fn allows(&self, url: &WebUrl) -> bool {
        let origin = url.origin();

        if self.allowed_origins.iter().any(|entry| entry == &origin) {
            return true;
        }

        if url.scheme != "https" || url.port != 443 {
            return false;
        }

        self.allowed_domains.iter().any(|domain| {
            url.host == *domain || url.host.ends_with(&format!(".{domain}"))
        })
    }

    fn check(&self, url: &WebUrl) -> Result<()> {
        if self.allows(url) {
            return Ok(());
        }

        Err(Error::Network(format!(
            "Refusing to download from {}. That origin is not in the install allowlist ({}; https subdomains of {}). Set {MIRROR_ENV} to a local mirror origin if you meant to use one.",
            url.origin(),
            self.allowed_origins.join(", "),
            if self.allowed_domains.is_empty() {
                "no domain".to_string()
            } else {
                self.allowed_domains.join(", ")
            }
        )))
    }
}

/// Resolve a redirect target, which may be absolute or root-relative.
fn resolve_location(base: &WebUrl, location: &str) -> Result<WebUrl> {
    let value = location.trim();

    if value.starts_with("http://") || value.starts_with("https://") {
        return WebUrl::parse(value);
    }

    if value.starts_with('/') {
        return WebUrl::parse(&format!("{}{}", base.origin(), value));
    }

    Err(Error::Network(format!(
        "The download redirect {value:?} is not an absolute or root-relative URL."
    )))
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub requested_url: String,
    pub final_url: String,
    pub path: PathBuf,
    pub file_name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Download `url` into `destination_dir`, streaming to a `.part` file first.
///
/// The allowlist is checked before the request and again on every redirect.
/// The body is capped at [`DownloadPolicy::max_bytes`]. Any failure removes the
/// partial file.
pub fn download(url: &str, destination_dir: &Path, policy: &DownloadPolicy) -> Result<DownloadReport> {
    let mut current = WebUrl::parse(url)?;
    policy.check(&current)?;

    fs::create_dir_all(destination_dir)?;

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(policy.timeout_seconds)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .new_agent();

    let mut response = None;

    for _ in 0..=MAX_REDIRECTS {
        let mut attempt = agent
            .get(&current.to_string())
            .header("User-Agent", GAMEBANANA_USER_AGENT)
            .header("Accept", "application/octet-stream, */*")
            .call()
            .map_err(|error| {
                Error::Network(format!("Download request to {current} failed: {error}"))
            })?;

        let status = attempt.status().as_u16();

        if (300..400).contains(&status) {
            let location = attempt
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| {
                    Error::Network(format!(
                        "Download redirect from {current} returned HTTP {status} with no Location header."
                    ))
                })?;

            let next = resolve_location(&current, &location)?;
            policy.check(&next)?;
            current = next;
            continue;
        }

        if !(200..300).contains(&status) {
            let body = attempt
                .body_mut()
                .read_to_string()
                .unwrap_or_else(|_| String::new());
            return Err(Error::Network(format!(
                "Download from {current} returned HTTP {status}. {}",
                snippet(&body)
            )));
        }

        response = Some((current.clone(), attempt));
        break;
    }

    let Some((final_url, mut response)) = response else {
        return Err(Error::Network(format!(
            "Download from {url} exceeded {MAX_REDIRECTS} redirects. Every hop must stay on the install allowlist."
        )));
    };

    if let Some(length) = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && length > policy.max_bytes
    {
        return Err(Error::Network(format!(
            "Download from {final_url} declares {length} bytes, above the {} byte cap. Nothing was written.",
            policy.max_bytes
        )));
    }

    let file_name = file_name_for(&final_url, &response);
    let final_path = destination_dir.join(&file_name);
    let partial_path = destination_dir.join(format!("{file_name}.part"));

    let _ = fs::remove_file(&partial_path);

    let mut file = File::create(&partial_path)?;
    let mut hasher = Sha256::new();
    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];
    let mut total: u64 = 0;

    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                drop(file);
                let _ = fs::remove_file(&partial_path);
                return Err(Error::Network(format!(
                    "Download from {final_url} stopped after {total} bytes: {error}"
                )));
            }
        };

        if read == 0 {
            break;
        }

        total += read as u64;

        if total > policy.max_bytes {
            drop(file);
            let _ = fs::remove_file(&partial_path);
            return Err(Error::Network(format!(
                "Download from {final_url} passed the {} byte cap. Nothing was kept.",
                policy.max_bytes
            )));
        }

        if let Err(error) = file.write_all(&buffer[..read]) {
            drop(file);
            let _ = fs::remove_file(&partial_path);
            return Err(error.into());
        }

        hasher.update(&buffer[..read]);
    }

    file.flush()?;
    drop(file);

    let _ = fs::remove_file(&final_path);
    fs::rename(&partial_path, &final_path)?;

    Ok(DownloadReport {
        requested_url: url.to_string(),
        final_url: final_url.to_string(),
        path: final_path,
        file_name,
        bytes: total,
        sha256: hex(&hasher.finalize()),
    })
}

/// Hash a local archive in place. Nothing is copied or moved.
pub fn describe_local_file(path: &Path) -> Result<DownloadReport> {
    let metadata = fs::metadata(path)?;

    if !metadata.is_file() {
        return Err(Error::msg(format!(
            "{} is not a file.",
            path.display()
        )));
    }

    if metadata.len() > MAX_DOWNLOAD_BYTES {
        return Err(Error::msg(format!(
            "{} is {} bytes, above the {} byte cap for one archive.",
            path.display(),
            metadata.len(),
            MAX_DOWNLOAD_BYTES
        )));
    }

    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg(format!("{} has no file name.", path.display())))?;

    Ok(DownloadReport {
        requested_url: path.display().to_string(),
        final_url: path.display().to_string(),
        path: path.to_path_buf(),
        file_name,
        bytes: metadata.len(),
        sha256: sha256_file(path)?,
    })
}

/// Content-Disposition name when present, else the last segment of the URL.
fn file_name_for(url: &WebUrl, response: &ureq::http::Response<ureq::Body>) -> String {
    let from_header = response
        .headers()
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .and_then(disposition_file_name);
    let candidate = from_header.or_else(|| url.file_name());

    let name = candidate
        .map(|value| {
            value
                .rsplit('/')
                .next()
                .unwrap_or(&value)
                .trim_matches('"')
                .to_string()
        })
        .filter(|value| {
            !value.is_empty()
                && value != "."
                && value != ".."
                && !value.contains('\\')
                && !value.contains('\0')
                && !value.contains(':')
        });

    name.unwrap_or_else(|| "download".to_string())
}

/// A plain file name that is safe to join onto a directory.
fn safe_local_name(name: &str) -> Option<String> {
    let name = name.rsplit('/').next().unwrap_or(name).trim();

    let unusable = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('\\')
        || name.contains('\0')
        || name.contains(':');

    if unusable {
        None
    } else {
        Some(name.to_string())
    }
}

fn disposition_file_name(value: &str) -> Option<String> {
    let lowered = value.to_ascii_lowercase();
    let index = lowered.find("filename=")?;
    let rest = &value[index + "filename=".len()..];
    let rest = rest.trim_start();

    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.split('"').next().map(str::to_string);
    }

    rest.split(';').next().map(|value| value.trim().to_string())
}

// ---------------------------------------------------------------------------
// GameBanana metadata
// ---------------------------------------------------------------------------

/// One downloadable file from a GameBanana mod page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBananaFile {
    pub file_id: u64,
    pub file_name: String,
    pub download_url: String,
    pub size_bytes: Option<u64>,
    pub version: Option<String>,
    pub md5: Option<String>,
    /// How many files the mod page listed. More than one means a choice was made.
    pub file_count: usize,
}

/// Pick the newest file from a `Mod/<id>/ProfilePage` body.
///
/// Public API v11 lists every uploaded file in `_aFiles`; the newest
/// `_tsDateAdded` wins and a tie keeps the first listed. The shape is the one
/// GameBanana served for Deadlock mod 715100, checked on 2026-11-16.
pub fn parse_mod_profile(body: &str) -> Result<GameBananaFile> {
    let value: Value = serde_json::from_str(body).map_err(|error| {
        Error::Network(format!(
            "GameBanana profile response was not valid JSON ({error}). {}",
            snippet(body)
        ))
    })?;

    let files = value
        .get("_aFiles")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            Error::Network(format!(
                "GameBanana profile response had no _aFiles array. {}",
                snippet(body)
            ))
        })?;

    let mut best: Option<(i64, &Value)> = None;

    for file in files {
        let added = file.get("_tsDateAdded").and_then(Value::as_i64).unwrap_or(0);
        match best {
            Some((current, _)) if current >= added => {}
            _ => best = Some((added, file)),
        }
    }

    let (_, file) = best.ok_or_else(|| {
        Error::Network(
            "This GameBanana mod lists no downloadable files. Nothing to install.".to_string(),
        )
    })?;

    let download_url = file
        .get("_sDownloadUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::Network(
                "The GameBanana file record has no _sDownloadUrl. The mod may still be pending approval."
                    .to_string(),
            )
        })?;

    let file_name = file
        .get("_sFile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "download");

    Ok(GameBananaFile {
        file_id: file.get("_idRow").and_then(Value::as_u64).unwrap_or(0),
        file_name: file_name.to_string(),
        download_url: download_url.to_string(),
        size_bytes: file.get("_nFilesize").and_then(Value::as_u64),
        version: file
            .get("_sVersion")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        md5: file
            .get("_sMd5Checksum")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        file_count: files.len(),
    })
}

/// Read the public mod page for a mod id.
pub fn fetch_gamebanana_file(mod_id: u64) -> Result<GameBananaFile> {
    let url = format!("{GAMEBANANA_PROFILE_URL}/{mod_id}/ProfilePage");

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(API_TIMEOUT_SECONDS)))
        .http_status_as_error(false)
        .build()
        .new_agent();

    let mut response = agent
        .get(&url)
        .header("User-Agent", GAMEBANANA_USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|error| Error::Network(format!("GameBanana request to {url} failed: {error}")))?;

    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::Network(format!("GameBanana response for {url} failed: {error}")))?;

    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "GameBanana returned HTTP {status} for mod {mod_id}. {}",
            snippet(&body)
        )));
    }

    parse_mod_profile(&body)
}

// ---------------------------------------------------------------------------
// Archive formats and external backends
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    Rar,
    Unknown,
}

impl ArchiveFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
            Self::Rar => "rar",
            Self::Unknown => "unknown",
        }
    }

    fn tool_name(self) -> &'static str {
        match self {
            Self::SevenZip => "7z",
            Self::Rar => "unrar",
            _ => "",
        }
    }

    fn env_name(self) -> &'static str {
        match self {
            Self::SevenZip => SEVEN_ZIP_ENV,
            Self::Rar => UNRAR_ENV,
            _ => "",
        }
    }
}

/// Resolved external extractors. Empty means "not found"; nothing is downloaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPaths {
    pub seven_zip: Option<PathBuf>,
    pub unrar: Option<PathBuf>,
}

impl ToolPaths {
    /// Read `DAH_7Z` / `DAH_UNRAR`, then fall back to `PATH`.
    pub fn from_environment() -> Self {
        let seven_zip = env::var(SEVEN_ZIP_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| find_on_path(&["7z", "7zz", "7za"]));
        let unrar = env::var(UNRAR_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| find_on_path(&["unrar", "rar"]));

        Self { seven_zip, unrar }
    }

    fn for_format(&self, format: ArchiveFormat) -> Option<&PathBuf> {
        match format {
            ArchiveFormat::SevenZip => self.seven_zip.as_ref(),
            ArchiveFormat::Rar => self.unrar.as_ref(),
            _ => None,
        }
    }
}

/// Identify an archive from its magic bytes, falling back to the extension.
pub fn detect_archive_format(path: &Path) -> ArchiveFormat {
    let mut magic = [0u8; 8];
    let read = File::open(path)
        .and_then(|mut file| file.read(&mut magic))
        .unwrap_or(0);

    if read >= 4 && &magic[..4] == b"PK\x03\x04" {
        return ArchiveFormat::Zip;
    }
    if read >= 4 && (&magic[..4] == b"PK\x05\x06" || &magic[..4] == b"PK\x07\x08") {
        return ArchiveFormat::Zip;
    }
    if read >= 6 && &magic[..6] == b"7z\xbc\xaf\x27\x1c" {
        return ArchiveFormat::SevenZip;
    }
    if read >= 6 && &magic[..4] == b"Rar!" {
        return ArchiveFormat::Rar;
    }

    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "zip" => ArchiveFormat::Zip,
        "7z" => ArchiveFormat::SevenZip,
        "rar" => ArchiveFormat::Rar,
        _ => ArchiveFormat::Unknown,
    }
}

fn backend_unavailable(format: ArchiveFormat) -> Error {
    let tool = format.tool_name();
    let env_name = format.env_name();

    Error::msg(format!(
        "Archive backend unavailable: {} archives need {tool}, which was not found. Set {env_name} to the {tool} binary or put {tool} on PATH. Nothing was extracted and nothing was installed.",
        format.as_str().to_uppercase()
    ))
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Extraction budgets. Tests lower these instead of building huge archives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractLimits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_ratio: u64,
    pub ratio_floor_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_ARCHIVE_ENTRIES,
            max_total_bytes: MAX_EXTRACTED_BYTES,
            max_ratio: MAX_COMPRESSION_RATIO,
            ratio_floor_bytes: RATIO_FLOOR_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub format: ArchiveFormat,
    pub directory: PathBuf,
    /// Relative paths with forward slashes, sorted.
    pub files: Vec<String>,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// Extract `archive` into `destination`, which is cleared first.
///
/// Every entry name is resolved with [`safe_entry_path`] before anything is
/// written, symlinks and special files are rejected, and both the entry count
/// and the uncompressed byte total are capped. A failed extract removes the
/// destination tree.
pub fn extract_archive(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    tools: &ToolPaths,
) -> Result<ExtractReport> {
    if !archive.is_file() {
        return Err(Error::msg(format!(
            "Archive {} does not exist.",
            archive.display()
        )));
    }

    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::create_dir_all(destination)?;

    let format = detect_archive_format(archive);

    let result = match format {
        ArchiveFormat::Zip => extract_zip(archive, destination, limits),
        ArchiveFormat::SevenZip | ArchiveFormat::Rar => match tools.for_format(format) {
            Some(tool) => extract_with_tool(format, tool, archive, destination, limits),
            None => Err(backend_unavailable(format)),
        },
        ArchiveFormat::Unknown => Err(Error::msg(format!(
            "Could not identify the archive format of {}. Supported formats are .zip (built in), .7z and .rar (external tools).",
            archive.display()
        ))),
    };

    match result {
        Ok((files, bytes)) => Ok(ExtractReport {
            format,
            directory: destination.to_path_buf(),
            files,
            bytes,
            warnings: Vec::new(),
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(destination);
            Err(error)
        }
    }
}

fn extract_zip(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
) -> Result<(Vec<String>, u64)> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| Error::msg(format!("{} is not a readable ZIP: {error}", archive.display())))?;

    if zip.len() > limits.max_entries {
        return Err(Error::msg(format!(
            "ZIP {} holds {} entries, above the {} entry cap. Refusing to extract it.",
            archive.display(),
            zip.len(),
            limits.max_entries
        )));
    }

    let mut files = Vec::new();
    let mut total: u64 = 0;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| {
            Error::msg(format!(
                "ZIP entry {index} of {} could not be read: {error}",
                archive.display()
            ))
        })?;

        let raw_name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| {
                Error::msg(format!(
                    "ZIP entry {index} has a name that is not valid UTF-8. Refusing to extract it."
                ))
            })?
            .to_string();

        // Directory entries end with a slash. The slash is not a path segment.
        let is_dir = entry.is_dir();
        let checked = if is_dir {
            raw_name.trim_end_matches('/')
        } else {
            raw_name.as_str()
        };

        let target = safe_entry_path(destination, checked)?;

        if entry.is_symlink() {
            return Err(Error::msg(format!(
                "ZIP entry {raw_name} is a symbolic link. Mod archives may not contain links."
            )));
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            let allowed = kind == 0 || kind == 0o100000 || kind == 0o040000;
            if !allowed {
                return Err(Error::msg(format!(
                    "ZIP entry {raw_name} has file type {kind:o}, which is not a regular file or folder."
                )));
            }
        }

        if is_dir {
            continue;
        }

        check_entry_budget(&raw_name, entry.size(), entry.compressed_size(), limits, &mut total)?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut output = File::create(&target)?;
        let declared = entry.size();
        let mut written: u64 = 0;
        let mut buffer = [0u8; READ_BUFFER_BYTES];

        loop {
            let read = entry.read(&mut buffer).map_err(|error| {
                Error::msg(format!(
                    "ZIP entry {raw_name} failed after {written} bytes: {error}"
                ))
            })?;
            if read == 0 {
                break;
            }

            written += read as u64;

            if written > declared {
                return Err(Error::msg(format!(
                    "ZIP entry {raw_name} is larger than the {declared} bytes it declares. Refusing it."
                )));
            }

            output.write_all(&buffer[..read])?;
        }

        let relative = normalize_relative(&raw_name)?;
        files.push(relative);
    }

    files.sort();
    Ok((files, total))
}

fn extract_with_tool(
    format: ArchiveFormat,
    tool: &Path,
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
) -> Result<(Vec<String>, u64)> {
    if !tool.is_file() {
        return Err(Error::msg(format!(
            "Archive backend unavailable: {} is not a file, so {} archives cannot be extracted. Set {} to the binary path.",
            tool.display(),
            format.as_str().to_uppercase(),
            format.env_name()
        )));
    }

    let mut command = Command::new(tool);

    match format {
        ArchiveFormat::SevenZip => {
            // `-o` must be glued to its path, that is the 7z CLI contract.
            command
                .arg("x")
                .arg("-y")
                .arg("-bd")
                .arg(format!("-o{}", destination.display()))
                .arg(archive);
        }
        ArchiveFormat::Rar => {
            command
                .arg("x")
                .arg("-o+")
                .arg("-p-")
                .arg(archive)
                .arg(destination);
        }
        _ => return Err(backend_unavailable(format)),
    }

    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            Error::msg(format!(
                "Could not run {} ({}): {error}",
                tool.display(),
                format.as_str()
            ))
        })?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let fallback = String::from_utf8_lossy(&output.stdout);
        let text = if detail.trim().is_empty() { fallback } else { detail };
        return Err(Error::msg(format!(
            "{} could not extract {} ({}). {}",
            tool.display(),
            archive.display(),
            output.status,
            last_lines(&text, 6)
        )));
    }

    // The external tool does its own path handling, so the tree is checked
    // again here: no links, nothing outside the destination, caps enforced.
    let files = walk_tree(destination)?;
    let mut total: u64 = 0;

    if files.len() > limits.max_entries {
        return Err(Error::msg(format!(
            "{} extracted {} entries from {}, above the {} entry cap.",
            tool.display(),
            files.len(),
            archive.display(),
            limits.max_entries
        )));
    }

    for (_, size) in &files {
        total += size;
        if total > limits.max_total_bytes {
            return Err(Error::msg(format!(
                "{} extracted more than {} bytes from {}, above the cap.",
                tool.display(),
                limits.max_total_bytes,
                archive.display()
            )));
        }
    }

    let mut paths: Vec<String> = files.into_iter().map(|(path, _)| path).collect();
    paths.sort();
    Ok((paths, total))
}

/// Resolve one archive entry name inside `destination`.
///
/// Rejects empty names, backslashes, absolute paths, drive letters, `..`, `.`,
/// control characters and alternate data stream colons. The lexical result is
/// checked against the destination again.
pub fn safe_entry_path(destination: &Path, raw: &str) -> Result<PathBuf> {
    if raw.is_empty() {
        return Err(Error::msg(
            "Archive entry has an empty name. Refusing to extract it.",
        ));
    }

    if raw.contains('\0') {
        return Err(Error::msg(format!(
            "Archive entry {raw:?} contains a NUL byte."
        )));
    }

    if raw.contains('\\') {
        return Err(Error::msg(format!(
            "Archive entry {raw} uses backslashes. Refusing to extract it."
        )));
    }

    if raw.starts_with('/') {
        return Err(Error::msg(format!(
            "Archive entry {raw} is an absolute path. Refusing to extract it."
        )));
    }

    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(Error::msg(format!(
            "Archive entry {raw} starts with a drive letter. Refusing to extract it."
        )));
    }

    let mut relative = PathBuf::new();

    for segment in raw.split('/') {
        if segment.is_empty() {
            return Err(Error::msg(format!(
                "Archive entry {raw} has an empty path segment. Refusing to extract it."
            )));
        }

        if segment == "." || segment == ".." {
            return Err(Error::msg(format!(
                "Archive entry {raw} escapes the extract folder. Refusing it."
            )));
        }

        if segment.contains(':') {
            return Err(Error::msg(format!(
                "Archive entry {raw} has a colon in a path segment, which is never a game path."
            )));
        }

        if segment.chars().any(|character| character.is_control()) {
            return Err(Error::msg(format!(
                "Archive entry {raw} has a control character in a path segment."
            )));
        }

        relative.push(segment);
    }

    let joined = destination.join(&relative);

    if !joined.starts_with(destination) {
        return Err(Error::msg(format!(
            "Archive entry {raw} resolves outside {}. Refusing it.",
            destination.display()
        )));
    }

    Ok(joined)
}

fn check_entry_budget(
    name: &str,
    declared: u64,
    compressed: u64,
    limits: &ExtractLimits,
    total: &mut u64,
) -> Result<()> {
    if declared > limits.max_total_bytes {
        return Err(Error::msg(format!(
            "Archive entry {name} declares {declared} bytes, above the {} byte cap.",
            limits.max_total_bytes
        )));
    }

    if declared >= limits.ratio_floor_bytes && compressed > 0 {
        let ratio = declared / compressed;
        if ratio > limits.max_ratio {
            return Err(Error::msg(format!(
                "Archive entry {name} expands {compressed} compressed bytes to {declared} bytes ({ratio}:1), above the {}:1 ratio cap. Refusing a compression bomb.",
                limits.max_ratio
            )));
        }
    }

    *total += declared;

    if *total > limits.max_total_bytes {
        return Err(Error::msg(format!(
            "Archive expands beyond the {} byte cap. Refusing it.",
            limits.max_total_bytes
        )));
    }

    Ok(())
}

/// Every regular file below `root`, as forward slash relative paths plus sizes.
/// Symbolic links and special files are errors, not skips.
fn walk_tree(root: &Path) -> Result<Vec<(String, u64)>> {
    let mut output = Vec::new();
    walk_tree_into(root, "", &mut output)?;
    output.sort();
    Ok(output)
}

fn walk_tree_into(directory: &Path, prefix: &str, output: &mut Vec<(String, u64)>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            return Err(Error::msg(format!(
                "Extracted tree contains the symbolic link {}. Refusing it.",
                relative
            )));
        }

        if file_type.is_dir() {
            walk_tree_into(&entry.path(), &relative, output)?;
            continue;
        }

        if !file_type.is_file() {
            return Err(Error::msg(format!(
                "Extracted tree contains {relative}, which is not a regular file."
            )));
        }

        let size = entry.metadata()?.len();
        output.push((relative, size));
    }

    Ok(())
}

fn normalize_relative(raw: &str) -> Result<String> {
    let segments: Vec<&str> = raw.split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(Error::msg(format!(
            "Archive entry {raw} is not a safe relative path."
        )));
    }
    Ok(segments.join("/"))
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// One file that maps to a game path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateFile {
    /// Normalised game path, for example `materials/heroes/billy/billy_color.vmat_c`.
    path: String,
    /// Path inside the extracted tree. Empty for a VPK candidate.
    relative: String,
    size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateArtifact {
    Vpk {
        dir_path: PathBuf,
        chunk_paths: Vec<PathBuf>,
    },
    Tree {
        root: PathBuf,
    },
}

#[derive(Debug, Clone)]
struct Candidate {
    artifact: CandidateArtifact,
    prefix: Option<String>,
    /// Every mapped file. The payload validator runs over these.
    mapped: Vec<CandidateFile>,
    /// Mapped files that actually ship (compiled `_c` only for a tree).
    payload: Vec<String>,
    payload_bytes: u64,
    /// Files outside the detected game path prefix.
    unmapped: Vec<String>,
    warnings: Vec<String>,
}

/// Prefixes a mod archive may carry before the game path, deepest first.
const GAME_PATH_PREFIXES: [&str; 4] = ["game/citadel/", "citadel/", "game/", ""];

/// Walk an extracted tree and group it into one VPK or one compiled tree.
fn classify_extracted(extracted: &Path, limits: &ExtractLimits) -> Result<Candidate> {
    let files = walk_tree(extracted)?;

    if files.len() > limits.max_entries {
        return Err(Error::msg(format!(
            "Extracted tree holds {} entries, above the {} entry cap.",
            files.len(),
            limits.max_entries
        )));
    }

    let total: u64 = files.iter().map(|(_, size)| *size).sum();
    if total > limits.max_total_bytes {
        return Err(Error::msg(format!(
            "Extracted tree holds {total} bytes, above the {} byte cap.",
            limits.max_total_bytes
        )));
    }

    let vpks: Vec<&(String, u64)> = files
        .iter()
        .filter(|(path, _)| is_pak_dir_file(path))
        .collect();

    if vpks.len() > 1 {
        let names: Vec<&str> = vpks.iter().map(|(path, _)| path.as_str()).collect();
        return Err(Error::msg(format!(
            "The archive ships more than one pakNN_dir.vpk ({}). dah install refuses to guess which one is the mod. Extract the one you want and plan it as a local file.",
            names.join(", ")
        )));
    }

    if let Some((relative, _)) = vpks.first() {
        return classify_vpk(extracted, relative, &files);
    }

    classify_tree(extracted, files)
}

fn is_pak_dir_file(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);

    if !name.is_ascii() {
        return false;
    }

    let lowered = name.to_ascii_lowercase();
    let bytes = lowered.as_bytes();

    bytes.len() == "pak00_dir.vpk".len()
        && lowered.starts_with("pak")
        && lowered.ends_with("_dir.vpk")
        && bytes[3..5].iter().all(|byte| byte.is_ascii_digit())
}

fn classify_vpk(
    extracted: &Path,
    relative: &str,
    files: &[(String, u64)],
) -> Result<Candidate> {
    let dir_path = extracted.join(relative);
    let name = relative.rsplit('/').next().unwrap_or(relative).to_ascii_lowercase();
    let slot_digits = &name[3..5];

    let chunk_relatives: Vec<String> = files
        .iter()
        .map(|(path, _)| path.clone())
        .filter(|path| is_chunk_for(path, slot_digits))
        .collect();

    let chunks: Vec<PathBuf> = chunk_relatives
        .iter()
        .map(|path| extracted.join(path))
        .collect();

    let directory = read_vpk_directory_with(&dir_path, &ReadVpkOptions {
        keep_preload: false,
    })?;

    let mut mapped = Vec::with_capacity(directory.entries.len());
    for entry in &directory.entries {
        mapped.push(CandidateFile {
            path: entry.path.clone(),
            relative: String::new(),
            size: u64::from(entry.total_length),
        });
    }

    let mut warnings = Vec::new();

    for entry in &directory.entries {
        if entry.archive_index == crate::vpk::VPK_DIRECTORY_ARCHIVE {
            continue;
        }

        let expected = format!("pak{slot_digits}_{:03}.vpk", entry.archive_index);
        let present = chunk_relatives.iter().any(|path| {
            path.rsplit('/')
                .next()
                .map(|value| value.eq_ignore_ascii_case(&expected))
                .unwrap_or(false)
        });

        if !present {
            return Err(Error::msg(format!(
                "The archive VPK {relative} references chunk {expected}, which is not next to it. The mod would load with missing data, so nothing was installed."
            )));
        }
    }

    let others: Vec<String> = files
        .iter()
        .map(|(path, _)| path.clone())
        .filter(|path| path != relative && !chunk_relatives.contains(path))
        .collect();

    let payload: Vec<String> = mapped.iter().map(|file| file.path.clone()).collect();
    let payload_bytes = mapped.iter().map(|file| file.size).sum();

    if !others.is_empty() {
        warnings.push(format!(
            "The archive also holds {} file(s) outside the VPK; they are not installed: {}",
            others.len(),
            preview(&others)
        ));
    }

    Ok(Candidate {
        artifact: CandidateArtifact::Vpk {
            dir_path,
            chunk_paths: chunks,
        },
        prefix: None,
        mapped,
        payload,
        payload_bytes,
        unmapped: others,
        warnings,
    })
}

fn is_chunk_for(relative: &str, slot_digits: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);

    if !name.is_ascii() {
        return false;
    }

    let lowered = name.to_ascii_lowercase();
    let bytes = lowered.as_bytes();

    bytes.len() == "pak00_000.vpk".len()
        && lowered.starts_with("pak")
        && lowered.ends_with(".vpk")
        && &lowered[3..5] == slot_digits
        && bytes[5] == b'_'
        && bytes[6..9].iter().all(|byte| byte.is_ascii_digit())
}

fn classify_tree(extracted: &Path, files: Vec<(String, u64)>) -> Result<Candidate> {
    let paths: Vec<String> = files.iter().map(|(path, _)| path.clone()).collect();
    let (prefix, mapped_paths, unmapped) = detect_prefix(&paths);

    let mut mapped = Vec::new();
    let mut skipped = Vec::new();
    let mut payload = Vec::new();
    let mut payload_bytes = 0;

    for (relative, size) in &files {
        if !mapped_paths.iter().any(|path| path == relative) {
            continue;
        }

        let game_path = match prefix.as_deref() {
            Some(value) if !value.is_empty() => relative[value.len()..].to_string(),
            _ => relative.clone(),
        };

        if game_path.is_empty() {
            continue;
        }

        mapped.push(CandidateFile {
            path: game_path.clone(),
            relative: relative.clone(),
            size: *size,
        });

        if game_path.to_ascii_lowercase().ends_with("_c") {
            payload.push(game_path);
            payload_bytes += size;
        } else {
            skipped.push(game_path);
        }
    }

    let mut warnings = Vec::new();

    if !skipped.is_empty() {
        warnings.push(format!(
            "{} mapped file(s) are not compiled and are not shipped: {}",
            skipped.len(),
            preview(&skipped)
        ));
    }

    if !unmapped.is_empty() {
        warnings.push(format!(
            "{} file(s) could not be mapped to a game path and are not installed: {}",
            unmapped.len(),
            preview(&unmapped)
        ));
    }

    Ok(Candidate {
        artifact: CandidateArtifact::Tree {
            root: extracted.to_path_buf(),
        },
        prefix,
        mapped,
        payload,
        payload_bytes,
        unmapped,
        warnings,
    })
}

/// Detect the game path prefix for an archive tree.
///
/// The deepest known prefix that covers at least one compiled file wins.
/// A more specific prefix is preferred even when a shallower one would cover
/// more files, because mapping a second tree such as `game/core/...` into the
/// citadel root would ship the wrong game paths. Everything outside the chosen
/// prefix is reported as unmapped, never guessed.
fn detect_prefix(paths: &[String]) -> (Option<String>, Vec<String>, Vec<String>) {
    let mut chosen = "";

    for prefix in GAME_PATH_PREFIXES {
        let covered = paths
            .iter()
            .filter(|path| {
                path.starts_with(prefix) && path.len() > prefix.len() && path.ends_with("_c")
            })
            .count();

        if covered > 0 {
            chosen = prefix;
            break;
        }
    }

    let mapped: Vec<String> = paths
        .iter()
        .filter(|path| path.starts_with(chosen) && path.len() > chosen.len())
        .cloned()
        .collect();

    if mapped.is_empty() {
        return (None, Vec::new(), paths.to_vec());
    }

    let unmapped: Vec<String> = paths
        .iter()
        .filter(|path| !mapped.iter().any(|candidate| candidate == *path))
        .cloned()
        .collect();

    (Some(chosen.to_string()), mapped, unmapped)
}

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

/// How the validated payload reaches the addons folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum InstallArtifact {
    /// Archive shipped a Ready `pakNN_dir.vpk` that is copied as-is (renamed for
    /// the chosen slot, chunk files renamed to match).
    Vpk {
        dir_path: PathBuf,
        chunk_paths: Vec<PathBuf>,
    },
    /// Compiled tree that is packed into a VPK in the workspace first.
    Staging { path: PathBuf },
}

/// Overlap with an installed addon that the game loads first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallConflict {
    pub slot: u8,
    pub file: String,
    pub paths: Vec<String>,
}

/// A validated, slot-assigned install plan. Read the plan, then approve apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub schema: u32,
    pub id: String,
    pub created_at: String,
    /// What the user typed.
    pub source: String,
    pub source_label: String,
    pub gamebanana: Option<GameBananaFile>,
    pub download_url: Option<String>,
    pub archive: PathBuf,
    pub archive_bytes: u64,
    pub archive_sha256: String,
    pub format: ArchiveFormat,
    pub extracted: PathBuf,
    pub prefix: Option<String>,
    pub artifact: InstallArtifact,
    pub workspace: PathBuf,
    pub game_root: PathBuf,
    pub addons_dir: PathBuf,
    pub slot: u8,
    pub target: PathBuf,
    /// Game paths the payload ships, sorted.
    pub files: Vec<String>,
    /// Uncompressed payload bytes.
    pub bytes: u64,
    pub conflicts: Vec<InstallConflict>,
    /// True when the target slot or its chunks already exist.
    pub replace_required: bool,
    /// Existing files that would be replaced or removed.
    pub replaced: Vec<PathBuf>,
    pub gameinfo_path: PathBuf,
    pub gameinfo_repair: bool,
    pub unmapped: Vec<String>,
    pub warnings: Vec<String>,
    pub plan_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanOptions {
    /// Explicit game root. None means Steam discovery, then `DAH_GAME_DIR`.
    pub game_root: Option<PathBuf>,
    pub policy: DownloadPolicy,
    pub limits: ExtractLimits,
    pub tools: ToolPaths,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            game_root: None,
            policy: DownloadPolicy::gamebanana(),
            limits: ExtractLimits::default(),
            tools: ToolPaths::from_environment(),
        }
    }
}

/// Download, extract, validate and plan. Writes only inside the workspace.
pub fn plan_install(request: &InstallRequest, options: &PlanOptions) -> Result<InstallPlan> {
    let game_root = resolve_game_root(options.game_root.clone())?;
    let addons = addons_dir(&game_root);
    let gameinfo = gameinfo_path(&game_root);

    if let Some(slot) = request.slot
        && !(1..=99).contains(&slot)
    {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to 99. Addon slots are pak01_dir.vpk to pak99_dir.vpk."
        )));
    }

    let cache = download_dir(&request.workspace, &request.source);
    fs::create_dir_all(&cache)?;

    let mut warnings = Vec::new();
    let mut gamebanana = None;

    let fetched = match &request.source {
        InstallSource::GameBananaId(id) => {
            let file = fetch_gamebanana_file(*id)?;

            if file.file_count > 1 {
                warnings.push(format!(
                    "GameBanana lists {} files for mod {id}; the newest ({}) was chosen. Download another file and plan it as a local archive to install a different one.",
                    file.file_count, file.file_name
                ));
            }

            // A GameBanana file id is immutable, so a cached archive whose size
            // matches the file record can be reused instead of downloaded again.
            let cached = safe_local_name(&file.file_name).map(|name| cache.join(name));
            let cached_size = cached
                .as_ref()
                .and_then(|path| fs::metadata(path).ok())
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len());

            let report = match (&cached, file.size_bytes, cached_size) {
                (Some(path), Some(expected), Some(size)) if expected == size => {
                    warnings.push(format!(
                        "Reused the cached download {} because its size matches the GameBanana file record. Run `dah install cache prune` to force a fresh download.",
                        path.display()
                    ));
                    DownloadReport {
                        requested_url: file.download_url.clone(),
                        final_url: file.download_url.clone(),
                        path: path.clone(),
                        file_name: safe_local_name(&file.file_name)
                            .unwrap_or_else(|| "download".to_string()),
                        bytes: size,
                        sha256: sha256_file(path)?,
                    }
                }
                _ => download(&file.download_url, &cache, &options.policy)?,
            };

            if let Some(expected) = file.size_bytes
                && expected != report.bytes
            {
                warnings.push(format!(
                    "GameBanana declared {expected} bytes for {} but the download is {} bytes.",
                    file.file_name, report.bytes
                ));
            }

            gamebanana = Some(file);
            report
        }
        InstallSource::DirectUrl(url) => download(url, &cache, &options.policy)?,
        InstallSource::LocalFile(path) => describe_local_file(path)?,
    };

    let extracted = cache.join("extracted");
    let extract = extract_archive(&fetched.path, &extracted, &options.limits, &options.tools)?;

    let candidate = classify_extracted(&extracted, &options.limits)?;
    warnings.extend(extract.warnings);
    warnings.extend(candidate.warnings.clone());

    let mapped_paths: Vec<String> = candidate.mapped.iter().map(|file| file.path.clone()).collect();
    let issues = validate_payload(&mapped_paths, &PayloadOptions::default());

    if has_errors(&issues) {
        let errors: Vec<PayloadIssue> = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .cloned()
            .collect();

        return Err(Error::msg(format!(
            "Payload rejected: the archive ships files a Deadlock mod may not contain. Nothing was installed.\n{}\nPaths: {}",
            format_payload_issues(&errors),
            preview(
                &errors
                    .iter()
                    .filter_map(|issue| issue.entry.clone())
                    .collect::<Vec<_>>()
            )
        )));
    }

    // Group validator warnings by rule so a map that ships 300 config files
    // produces one line, not 300. Errors were rejected above.
    let mut grouped: std::collections::BTreeMap<(&'static str, String), Vec<String>> =
        std::collections::BTreeMap::new();

    for issue in &issues {
        grouped
            .entry((issue.code, issue.severity.as_str().to_string()))
            .or_default()
            .push(issue.entry.clone().unwrap_or_default());
    }

    for ((code, severity), entries) in grouped {
        warnings.push(format!(
            "{severity} {code}: {} entry/entries: {}",
            entries.len(),
            preview(&entries)
        ));
    }

    if !candidate
        .payload
        .iter()
        .any(|path| path.to_ascii_lowercase().ends_with("_c"))
    {
        return Err(Error::msg(
            "No compiled _c files were found in the archive, so there is nothing to install. A Deadlock mod payload is compiled files at exact game paths.",
        ));
    }

    let artifact = match &candidate.artifact {
        CandidateArtifact::Tree { root } => {
            let staging = cache.join("staging");
            prepare_staging(root, &candidate, &staging)?;
            InstallArtifact::Staging { path: staging }
        }
        CandidateArtifact::Vpk {
            dir_path,
            chunk_paths,
        } => InstallArtifact::Vpk {
            dir_path: dir_path.clone(),
            chunk_paths: chunk_paths.clone(),
        },
    };

    let (slot, replaced, replace_required, slot_warnings) = choose_slot(
        &addons,
        request.slot,
        request.force,
        candidate.payload.len(),
    )?;
    warnings.extend(slot_warnings);

    let target = addons.join(format_slot_file_name(slot)?);

    let conflicts = scan_conflicts(&addons, slot, &candidate.payload, &mut warnings)?;
    let gameinfo_status = read_game_info_status(&gameinfo)?;
    let gameinfo_repair = !(gameinfo_status.present && gameinfo_status.addons_search_path);

    if !gameinfo_status.present {
        warnings.push(format!(
            "{} is missing. The addons SearchPath cannot be repaired until Steam restores it.",
            gameinfo.display()
        ));
    }

    let mut files = candidate.payload.clone();
    files.sort();

    let id = plan_id(&request.target);
    let plan_path = plans_dir(&request.workspace).join(format!("{id}.json"));

    let plan = InstallPlan {
        schema: INSTALL_SCHEMA,
        id,
        created_at: now_iso(),
        source: request.target.clone(),
        source_label: request.source.label(),
        gamebanana,
        download_url: Some(fetched.final_url.clone()),
        archive: fetched.path.clone(),
        archive_bytes: fetched.bytes,
        archive_sha256: fetched.sha256.clone(),
        format: extract.format,
        extracted,
        prefix: candidate.prefix.clone(),
        artifact,
        workspace: request.workspace.clone(),
        game_root,
        addons_dir: addons,
        slot,
        target,
        files,
        bytes: candidate.payload_bytes,
        conflicts,
        replace_required,
        replaced,
        gameinfo_path: gameinfo,
        gameinfo_repair,
        unmapped: candidate.unmapped.clone(),
        warnings,
        plan_path,
    };

    store_plan(&plan)?;

    Ok(plan)
}

fn prepare_staging(root: &Path, candidate: &Candidate, staging: &Path) -> Result<()> {
    if staging.exists() {
        fs::remove_dir_all(staging)?;
    }
    fs::create_dir_all(staging)?;

    for file in &candidate.mapped {
        if !candidate.payload.iter().any(|path| path == &file.path) {
            continue;
        }

        let source = root.join(&file.relative);
        let destination = staging.join(&file.path);

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(&source, &destination).map_err(|error| {
            Error::msg(format!(
                "Could not stage {} as {}: {error}",
                source.display(),
                file.path
            ))
        })?;
    }

    Ok(())
}

/// Choose a slot, refuse an occupied one without force, and list what a write
/// would replace.
fn choose_slot(
    addons: &Path,
    requested: Option<u8>,
    force: bool,
    payload_entries: usize,
) -> Result<(u8, Vec<PathBuf>, bool, Vec<String>)> {
    let status = read_addons_status(addons)?;
    let used: BTreeSet<u8> = status.slots.iter().map(|slot| slot.slot).collect();

    let slot = match requested {
        Some(value) => value,
        None => (1..=99)
            .find(|slot| !used.contains(slot))
            .ok_or_else(|| {
                Error::msg(
                    "Every addon slot 01 to 99 is taken. Remove an addon before installing another.",
                )
            })?,
    };

    let mut replaced = Vec::new();
    let mut warnings = Vec::new();

    let dir_name = format_slot_file_name(slot)?;
    let target = addons.join(&dir_name);
    let disabled = addons.join(format!("{dir_name}.disabled"));

    if target.is_file() {
        replaced.push(target.clone());
    }
    if disabled.is_file() {
        replaced.push(disabled.clone());
        warnings.push(format!(
            "Slot {slot:02} holds a disabled addon ({dir_name}.disabled). Installing over it replaces that file after a backup."
        ));
    }

    for chunk in &status.chunk_files {
        if is_slot_chunk_name(chunk, slot) {
            replaced.push(addons.join(chunk));
        }
    }

    if !replaced.is_empty() && requested.is_none() {
        // The lowest free slot was used, so a file here is a stale entry the
        // addon scan did not count. Refuse rather than overwrite silently.
        return Err(Error::msg(format!(
            "Slot {slot:02} is not empty: {}. Pick another slot with --slot or re-run with --force to replace it.",
            preview(
                &replaced
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
            )
        )));
    }

    if !replaced.is_empty() && !force {
        return Err(Error::msg(format!(
            "Slot {slot:02} is taken. {} would be replaced. Nothing was written. Re-run with --force to back up and replace it, or pick another slot with --slot.",
            replaced
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    if !replaced.is_empty() {
        warnings.push(format!(
            "--force: {} would be replaced and backed up. The payload holds {payload_entries} file(s).",
            replaced
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let replace_required = !replaced.is_empty();
    Ok((slot, replaced, replace_required, warnings))
}

/// Lower slots that already ship a payload path. The lower slot wins in game.
fn scan_conflicts(
    addons: &Path,
    slot: u8,
    files: &[String],
    warnings: &mut Vec<String>,
) -> Result<Vec<InstallConflict>> {
    if !addons.is_dir() || files.is_empty() {
        return Ok(Vec::new());
    }

    let status = read_addons_status(addons)?;
    let mut grouped: Vec<InstallConflict> = Vec::new();

    for addon in &status.slots {
        if addon.slot >= slot {
            continue;
        }

        let path = addons.join(&addon.file_name);
        let directory = match read_vpk_directory_with(&path, &ReadVpkOptions {
            keep_preload: false,
        }) {
            Ok(directory) => directory,
            Err(error) => {
                warnings.push(format!(
                    "Existing addon {} could not be read, so its conflicts are unknown: {error}",
                    addon.file_name
                ));
                continue;
            }
        };

        let shared: Vec<String> = files
            .iter()
            .filter(|file| {
                directory
                    .by_path
                    .contains_key(file.to_ascii_lowercase().as_str())
            })
            .cloned()
            .collect();

        if !shared.is_empty() {
            grouped.push(InstallConflict {
                slot: addon.slot,
                file: addon.file_name.clone(),
                paths: shared,
            });
        }
    }

    Ok(grouped)
}

/// Discovery error used when no game install can be found.
pub fn require_game_root(discovery: &InstallDiscovery) -> Result<PathBuf> {
    match &discovery.install {
        Some(install) => Ok(install.game_root.clone()),
        None => {
            let searched = if discovery.searched_roots.is_empty() {
                "no Steam library roots were found".to_string()
            } else {
                discovery
                    .searched_roots
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            Err(Error::msg(format!(
                "No Deadlock install found, so nothing can be installed. Searched: {searched}. Set DAH_GAME_DIR to the Deadlock folder, its game folder or game/citadel. Nothing was downloaded or written."
            )))
        }
    }
}

fn resolve_game_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if !path.join("game").join("citadel").is_dir() {
            return Err(Error::msg(format!(
                "{} is not a Deadlock game root: it has no game/citadel folder.",
                path.display()
            )));
        }
        return Ok(path);
    }

    let discovery = discover_deadlock_install(InstallDiscoveryOptions::default())?;
    require_game_root(&discovery)
}

// ---------------------------------------------------------------------------
// Apply, journal and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallJournalEntry {
    pub schema: u32,
    pub plan_id: String,
    pub installed_at: String,
    pub source: String,
    pub source_label: String,
    pub download_url: Option<String>,
    pub archive: PathBuf,
    pub archive_sha256: String,
    pub slot: u8,
    /// Directory file name, for example `pak03_dir.vpk`.
    pub file: String,
    pub target: PathBuf,
    pub chunk_files: Vec<String>,
    pub entries: usize,
    /// Uncompressed payload bytes.
    pub bytes: u64,
    /// Size of the written directory VPK.
    pub vpk_bytes: u64,
    pub vpk_sha256: String,
    pub backups: Vec<PathBuf>,
    /// Chunk files of the slot that the new payload does not use and that were
    /// backed up and removed.
    pub removed_chunks: Vec<String>,
    /// Other slot files that were backed up and removed, for example a
    /// `pakNN_dir.vpk.disabled` the install replaced.
    pub removed_files: Vec<String>,
    pub gameinfo_repaired: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub entry: InstallJournalEntry,
    pub target: PathBuf,
    pub backups: Vec<PathBuf>,
    pub gameinfo_repaired: bool,
}

/// Copy an approved plan into `game/citadel/addons`.
///
/// Refuses without `approve`, when the plan is stale, when the recorded game
/// root disappeared, or when the artifact no longer matches the plan's file
/// list. Existing files are backed up under
/// `<workspace>/.dah/install/backups/<stamp>/` before anything is replaced.
pub fn apply_install(plan: &InstallPlan, approve: bool) -> Result<InstallRecord> {
    if !approve {
        return Err(Error::msg(format!(
            "Install of slot {:02} into {} was not approved. Nothing was written. Re-run with --approve.",
            plan.slot,
            plan.target.display()
        )));
    }

    if plan.target != plan.addons_dir.join(format_slot_file_name(plan.slot)?) {
        return Err(Error::msg(format!(
            "Plan target {} does not match slot {:02} in {}.",
            plan.target.display(),
            plan.slot,
            plan.addons_dir.display()
        )));
    }

    if !plan.game_root.join("game").join("citadel").is_dir() {
        return Err(Error::msg(format!(
            "The game folder recorded in the plan ({}) no longer holds game/citadel. Plan the install again.",
            plan.game_root.display()
        )));
    }

    verify_artifact(plan)?;

    let slot_file = format_slot_file_name(plan.slot)?;
    let affected_exists = plan.target.is_file()
        || plan.addons_dir.join(format!("{slot_file}.disabled")).is_file()
        || plan.replaced.iter().any(|path| path.is_file())
        || slot_chunks_present(&plan.addons_dir, plan.slot)?;

    if plan.replace_required != affected_exists {
        return Err(Error::msg(
            "Install plan is stale: the addons folder changed since planning. Plan the install again.",
        ));
    }

    fs::create_dir_all(&plan.addons_dir)?;

    let backups = backup_existing(plan)?;
    let removed_files = remove_disabled_slot_file(plan)?;

    let (written, chunk_files) = match &plan.artifact {
        InstallArtifact::Vpk {
            dir_path,
            chunk_paths,
        } => deploy_archive_vpk(plan, dir_path, chunk_paths)?,
        InstallArtifact::Staging { path } => deploy_staged(plan, path)?,
    };

    if plan.gameinfo_repair {
        let status = read_game_info_status(&plan.gameinfo_path)?;
        if status.present {
            repair_gameinfo(&plan.gameinfo_path, true)?;
        }
    }

    let gameinfo_repaired = plan.gameinfo_repair
        && read_game_info_status(&plan.gameinfo_path)?.addons_search_path;

    let removed_chunks = remove_leftover_chunks(plan, &chunk_files)?;

    if !written.is_file() {
        return Err(Error::msg(format!(
            "Install wrote nothing to {}.",
            plan.target.display()
        )));
    }

    let entry = InstallJournalEntry {
        schema: INSTALL_SCHEMA,
        plan_id: plan.id.clone(),
        installed_at: now_iso(),
        source: plan.source.clone(),
        source_label: plan.source_label.clone(),
        download_url: plan.download_url.clone(),
        archive: plan.archive.clone(),
        archive_sha256: plan.archive_sha256.clone(),
        slot: plan.slot,
        file: format_slot_file_name(plan.slot)?,
        target: plan.target.clone(),
        chunk_files,
        entries: plan.files.len(),
        bytes: plan.bytes,
        vpk_bytes: fs::metadata(&written)?.len(),
        vpk_sha256: sha256_file(&written)?,
        backups: backups.clone(),
        removed_chunks,
        removed_files,
        gameinfo_repaired,
        warnings: plan.warnings.clone(),
    };

    append_journal(&plan.workspace, &entry)?;

    Ok(InstallRecord {
        entry,
        target: plan.target.clone(),
        backups,
        gameinfo_repaired,
    })
}

/// Remove the slot's `.disabled` file once it has been backed up.
///
/// Leaving both `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled` behind would make
/// the slot ambiguous for `dah addons enable`, so the disabled name goes away
/// after the backup and the install writes the enabled one.
fn remove_disabled_slot_file(plan: &InstallPlan) -> Result<Vec<String>> {
    let name = format!("{}.disabled", format_slot_file_name(plan.slot)?);
    let path = plan.addons_dir.join(&name);

    if !path.is_file() {
        return Ok(Vec::new());
    }

    fs::remove_file(&path)?;
    Ok(vec![name])
}

fn verify_artifact(plan: &InstallPlan) -> Result<()> {
    match &plan.artifact {
        InstallArtifact::Vpk { dir_path, .. } => {
            if !dir_path.is_file() {
                return Err(Error::msg(format!(
                    "The archive VPK {} is gone. Plan the install again.",
                    dir_path.display()
                )));
            }

            let directory = read_vpk_directory_with(dir_path, &ReadVpkOptions {
                keep_preload: false,
            })?;
            let mut entries: Vec<String> = directory.entries.iter().map(|entry| entry.path.clone()).collect();
            entries.sort();

            if entries != plan.files {
                return Err(Error::msg(format!(
                    "{} no longer ships the {} paths the plan validated. Plan the install again.",
                    dir_path.display(),
                    plan.files.len()
                )));
            }
        }
        InstallArtifact::Staging { path } => {
            if !path.is_dir() {
                return Err(Error::msg(format!(
                    "The staging tree {} is gone. Plan the install again.",
                    path.display()
                )));
            }

            let entries = crate::vpk_write::collect_staging_entries(path)?;
            if entries != plan.files {
                return Err(Error::msg(format!(
                    "{} no longer matches the {} validated paths. Plan the install again.",
                    path.display(),
                    plan.files.len()
                )));
            }
        }
    }

    Ok(())
}

fn backup_existing(plan: &InstallPlan) -> Result<Vec<PathBuf>> {
    let existing: Vec<&PathBuf> = plan.replaced.iter().filter(|path| path.is_file()).collect();

    if existing.is_empty() {
        return Ok(Vec::new());
    }

    let backup_dir = backups_dir(&plan.workspace).join(backup_stamp());
    fs::create_dir_all(&backup_dir)?;

    let mut backups = Vec::new();

    for source in existing {
        let name = source.file_name().ok_or_else(|| {
            Error::msg(format!("Cannot back up {}: it has no file name.", source.display()))
        })?;
        let destination = backup_dir.join(name);

        if destination.exists() {
            return Err(Error::msg(format!(
                "Backup target {} already exists. Nothing was written.",
                destination.display()
            )));
        }

        fs::copy(source, &destination)?;
        backups.push(destination);
    }

    Ok(backups)
}

fn deploy_archive_vpk(
    plan: &InstallPlan,
    dir_path: &Path,
    chunk_paths: &[PathBuf],
) -> Result<(PathBuf, Vec<String>)> {
    copy_into_place(dir_path, &plan.target)?;

    let mut chunk_files = Vec::new();

    for chunk in chunk_paths {
        let name = chunk
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| {
                Error::msg(format!("Chunk {} has no file name.", chunk.display()))
            })?;
        let renamed = rename_chunk(&name, plan.slot)?;
        let destination = plan.addons_dir.join(&renamed);
        copy_into_place(chunk, &destination)?;
        chunk_files.push(renamed);
    }

    chunk_files.sort();
    Ok((plan.target.clone(), chunk_files))
}

fn deploy_staged(plan: &InstallPlan, staging: &Path) -> Result<(PathBuf, Vec<String>)> {
    let build_dir = install_state_dir(&plan.workspace).join("build").join(&plan.id);
    let packed = pack_vpk(&PackOptions::new(
        staging.to_path_buf(),
        build_dir,
        format!("pak{:02}", plan.slot),
    ))?;

    copy_into_place(&packed.dir_path, &plan.target)?;

    let mut chunk_files = Vec::new();

    for chunk in &packed.chunk_paths {
        let name = chunk
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| Error::msg(format!("Chunk {} has no file name.", chunk.display())))?;
        let destination = plan.addons_dir.join(&name);
        copy_into_place(chunk, &destination)?;
        chunk_files.push(name);
    }

    chunk_files.sort();
    Ok((plan.target.clone(), chunk_files))
}

/// Remove chunk files of the slot that this install does not ship.
///
/// A previous, larger mod at the same slot can leave `pakNN_001.vpk` behind.
/// The game would still load it, so it is backed up first and then removed.
fn remove_leftover_chunks(plan: &InstallPlan, written: &[String]) -> Result<Vec<String>> {
    let mut removed = Vec::new();

    let entries = match fs::read_dir(&plan.addons_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
        Err(error) => return Err(error.into()),
    };

    let backup_dir = backups_dir(&plan.workspace).join(backup_stamp());

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();

        if !is_slot_chunk_name(&name, plan.slot)
            || written.iter().any(|value| value.eq_ignore_ascii_case(&name))
        {
            continue;
        }

        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        fs::create_dir_all(&backup_dir)?;
        let destination = backup_dir.join(&name);

        if destination.exists() {
            return Err(Error::msg(format!(
                "Backup target {} already exists. Leftover chunk {name} was not removed.",
                destination.display()
            )));
        }

        fs::copy(&path, &destination)?;
        fs::remove_file(&path)?;
        removed.push(name);
    }

    removed.sort();
    Ok(removed)
}

/// True for `pakNN_MMM.vpk` at the given slot. `pakNN_dir.vpk` is not a chunk.
fn is_slot_chunk_name(name: &str, slot: u8) -> bool {    if !name.is_ascii() {
        return false;
    }

    let lowered = name.to_ascii_lowercase();
    let bytes = lowered.as_bytes();

    bytes.len() == "pak00_000.vpk".len()
        && lowered.starts_with("pak")
        && &lowered[3..5] == format!("{slot:02}").as_str()
        && bytes[5] == b'_'
        && bytes[6..9].iter().all(|byte| byte.is_ascii_digit())
        && lowered.ends_with(".vpk")
}

/// True when any chunk file of `slot` sits in the addons folder.
fn slot_chunks_present(addons: &Path, slot: u8) -> Result<bool> {
    let entries = match fs::read_dir(addons) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() && is_slot_chunk_name(&name, slot) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn rename_chunk(name: &str, slot: u8) -> Result<String> {
    let lowered = name.to_ascii_lowercase();

    if !name.is_ascii()
        || lowered.len() != "pak00_000.vpk".len()
        || !lowered.ends_with(".vpk")
    {
        return Err(Error::msg(format!(
            "Chunk {name:?} is not named pakNN_MMM.vpk."
        )));
    }

    Ok(format!("pak{slot:02}_{}.vpk", &name[6..9]))
}

fn copy_into_place(source: &Path, destination: &Path) -> Result<()> {
    let name = destination
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| {
            Error::msg(format!("Cannot copy to {}: it has no file name.", destination.display()))
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

// ---------------------------------------------------------------------------
// Workspace layout, plan storage and journal
// ---------------------------------------------------------------------------

pub fn downloads_dir(workspace: &Path) -> PathBuf {
    workspace.join(DOWNLOADS_DIR_NAME)
}

pub fn download_dir(workspace: &Path, source: &InstallSource) -> PathBuf {
    downloads_dir(workspace).join(source.cache_key())
}

pub fn install_state_dir(workspace: &Path) -> PathBuf {
    workspace.join(INSTALL_STATE_DIR)
}

pub fn plans_dir(workspace: &Path) -> PathBuf {
    install_state_dir(workspace).join("plans")
}

pub fn backups_dir(workspace: &Path) -> PathBuf {
    install_state_dir(workspace).join("backups")
}

pub fn journal_path(workspace: &Path) -> PathBuf {
    workspace.join(JOURNAL_FILE_NAME)
}

pub fn plan_path(workspace: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(Error::msg(format!(
            "Plan id {id:?} is not a plan file name. Use the id printed by `dah install plan`."
        )));
    }

    Ok(plans_dir(workspace).join(format!("{id}.json")))
}

pub fn store_plan(plan: &InstallPlan) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(plan)?;
    write_atomic(&plan.plan_path, &bytes)
}

pub fn load_plan(workspace: &Path, id: &str) -> Result<InstallPlan> {
    let path = plan_path(workspace, id)?;

    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "No install plan {id}. Run `dah install plan <URL|ID>` first. Plans live in {}.",
                plans_dir(workspace).display()
            ))
        } else {
            error.into()
        }
    })?;

    serde_json::from_str(&text).map_err(|error| {
        Error::msg(format!(
            "Install plan {} is not valid JSON: {error}",
            path.display()
        ))
    })
}

pub fn read_journal(workspace: &Path) -> Result<Vec<InstallJournalEntry>> {
    let path = journal_path(workspace);

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut entries = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let entry: InstallJournalEntry = serde_json::from_str(line).map_err(|error| {
            Error::msg(format!(
                "{} line {} is not a valid install record: {error}",
                path.display(),
                index + 1
            ))
        })?;
        entries.push(entry);
    }

    Ok(entries)
}

fn append_journal(workspace: &Path, entry: &InstallJournalEntry) -> Result<()> {
    let path = journal_path(workspace);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut line = serde_json::to_string(entry)?;
    line.push('\n');

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.flush()?;

    Ok(())
}

/// One journal entry plus whether its file is still in the addons folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModStatus {
    pub entry: InstallJournalEntry,
    pub present: bool,
    pub size_bytes: Option<u64>,
    /// True when a later install wrote the same target.
    pub superseded: bool,
}

pub fn install_status(workspace: &Path) -> Result<Vec<InstalledModStatus>> {
    let entries = read_journal(workspace)?;
    let mut newest: Vec<(PathBuf, usize)> = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        match newest.iter_mut().find(|(path, _)| path == &entry.target) {
            Some((_, newest_index)) => *newest_index = index,
            None => newest.push((entry.target.clone(), index)),
        }
    }

    let mut status = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        let present = entry.target.is_file();
        let size_bytes = fs::metadata(&entry.target).ok().map(|data| data.len());
        let superseded = newest
            .iter()
            .any(|(path, newest_index)| path == &entry.target && *newest_index != index);

        status.push(InstalledModStatus {
            entry: entry.clone(),
            present,
            size_bytes,
            superseded,
        });
    }

    status.reverse();
    Ok(status)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachePruneEntry {
    pub key: String,
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachePruneReport {
    pub removed: Vec<CachePruneEntry>,
    pub total_bytes: u64,
}

/// Delete `<workspace>/downloads/*`. Plans, backups and the journal stay.
pub fn prune_download_cache(workspace: &Path) -> Result<CachePruneReport> {
    let root = downloads_dir(workspace);
    let mut removed = Vec::new();
    let mut total_bytes = 0;

    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CachePruneReport {
                removed,
                total_bytes,
            });
        }
        Err(error) => return Err(error.into()),
    };

    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();

    for path in paths {
        let size = directory_size(&path)?;
        let key = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();

        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }

        total_bytes += size;
        removed.push(CachePruneEntry {
            key,
            path,
            bytes: size,
        });
    }

    Ok(CachePruneReport {
        removed,
        total_bytes,
    })
}

fn directory_size(path: &Path) -> Result<u64> {
    let mut total = 0;

    if path.is_file() {
        return Ok(fs::metadata(path).map(|data| data.len()).unwrap_or(0));
    }

    let mut stack = vec![path.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };

            if metadata.is_dir() {
                stack.push(path);
            } else {
                total += metadata.len();
            }
        }
    }

    Ok(total)
}

// ---------------------------------------------------------------------------
// Text output
// ---------------------------------------------------------------------------

pub fn format_install_plan(plan: &InstallPlan) -> String {
    let mut lines = vec![format!("Install plan {}", plan.id)];

    lines.push(format!("  Source:    {}", plan.source_label));

    if let Some(file) = &plan.gamebanana {
        let version = file.version.clone().unwrap_or_else(|| "no version".to_string());
        lines.push(format!(
            "  File:      {} (GameBanana file {}, version {version})",
            file.file_name, file.file_id
        ));
    }

    lines.push(format!(
        "  Archive:   {} ({}, {}, sha256 {})",
        plan.archive.display(),
        human_bytes(plan.archive_bytes),
        plan.format.as_str(),
        short_hash_sha(&plan.archive_sha256)
    ));
    lines.push(format!(
        "  Prefix:    {}",
        match plan.prefix.as_deref() {
            Some("") | None => "none, paths are already game paths".to_string(),
            Some(prefix) => prefix.to_string(),
        }
    ));

    match &plan.artifact {
        InstallArtifact::Vpk { dir_path, chunk_paths } => {
            lines.push(format!(
                "  Artifact: VPK from the archive {}, {} chunk file(s)",
                dir_path.display(),
                chunk_paths.len()
            ));
        }
        InstallArtifact::Staging { path } => {
            lines.push(format!(
                "  Artifact: packed at apply from {}",
                path.display()
            ));
        }
    }

    lines.push(format!(
        "  Slot:      {:02} -> {}",
        plan.slot,
        plan.target.display()
    ));
    lines.push(format!(
        "  Files:     {} payload file(s), {}",
        plan.files.len(),
        human_bytes(plan.bytes)
    ));

    if plan.conflicts.is_empty() {
        lines.push("  Conflicts: none with a lower slot".to_string());
    } else {
        lines.push("  Conflicts:".to_string());
        for conflict in &plan.conflicts {
            lines.push(format!(
                "    slot {:02} ({}): {} shared path(s), the lower slot wins",
                conflict.slot,
                conflict.file,
                conflict.paths.len()
            ));
        }
    }

    if plan.replace_required {
        lines.push(format!(
            "  Replace:   {} existing file(s) would be backed up and replaced:",
            plan.replaced.len()
        ));
        for path in &plan.replaced {
            lines.push(format!("    {}", path.display()));
        }
    } else {
        lines.push("  Replace:   not required, the slot is free".to_string());
    }

    lines.push(format!(
        "  gameinfo:  {}",
        if plan.gameinfo_repair {
            format!(
                "the addons SearchPath is missing and would be repaired in {}",
                plan.gameinfo_path.display()
            )
        } else {
            "addons SearchPath present".to_string()
        }
    ));

    for warning in &plan.warnings {
        lines.push(format!("  WARNING {warning}"));
    }

    lines.push(format!(
        "Next: `dah install apply {} --approve` writes into the game addons folder.",
        plan.id
    ));

    lines.join("\n")
}

pub fn format_install_status(status: &[InstalledModStatus]) -> String {
    if status.is_empty() {
        return "No installs recorded. Run `dah install plan <URL|ID>` first.".to_string();
    }

    let mut lines = vec![format!("{} install record(s)", status.len())];

    for item in status {
        let state = if item.present {
            format!(
                "present, {}",
                item.size_bytes
                    .map(human_bytes)
                    .unwrap_or_else(|| "unknown size".to_string())
            )
        } else {
            "MISSING from the addons folder".to_string()
        };

        let superseded = if item.superseded { ", superseded" } else { "" };

        lines.push(format!(
            "slot {:02}  {}  {}{}",
            item.entry.slot,
            item.entry.file,
            state,
            superseded
        ));
        lines.push(format!("  target:    {}", item.entry.target.display()));
        lines.push(format!(
            "  installed: {} from {}",
            item.entry.installed_at, item.entry.source_label
        ));
        lines.push(format!(
            "  entries:   {} file(s), {}",
            item.entry.entries,
            human_bytes(item.entry.bytes)
        ));
        if !item.entry.chunk_files.is_empty() {
            lines.push(format!(
                "  chunks:    {}",
                item.entry.chunk_files.join(", ")
            ));
        }
        if !item.entry.backups.is_empty() {
            lines.push(format!(
                "  backups:   {}",
                item.entry
                    .backups
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    lines.join("\n")
}

pub fn format_cache_prune(report: &CachePruneReport) -> String {
    if report.removed.is_empty() {
        return format!(
            "Download cache is empty. {} would be created on the next plan.",
            "downloads/"
        );
    }

    let mut lines = vec![format!(
        "Removed {} cached download(s), {}",
        report.removed.len(),
        human_bytes(report.total_bytes)
    )];

    for item in &report.removed {
        lines.push(format!(
            "  {}  {}  {}",
            item.key,
            human_bytes(item.bytes),
            item.path.display()
        ));
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn is_gamebanana_host(host: &str) -> bool {
    GAMEBANANA_DOWNLOAD_HOSTS.contains(&host)
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;

    for directory in env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }

            if cfg!(windows) {
                let candidate = directory.join(format!("{name}.exe"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex(&digest)[..12].to_string()
}

fn short_hash_sha(value: &str) -> String {
    value.chars().take(16).collect()
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex(&hasher.finalize()))
}

fn plan_id(target: &str) -> String {
    let stamp: String = now_iso()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.subsec_nanos())
        .unwrap_or(0);

    format!("{stamp}-{nanos:08x}-{}", short_hash(target))
}

fn backup_stamp() -> String {
    format!(
        "{}-{}",
        now_iso().replace([':', '.'], "-"),
        std::process::id()
    )
}

fn preview(values: &[String]) -> String {
    const LIMIT: usize = 8;

    if values.is_empty() {
        return "none".to_string();
    }

    let mut text = values.iter().take(LIMIT).cloned().collect::<Vec<_>>().join(", ");

    if values.len() > LIMIT {
        text.push_str(&format!(", and {} more", values.len() - LIMIT));
    }

    text
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let value = bytes as f64;

    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.is_empty() {
        return "The response body was empty.".to_string();
    }

    let mut text: String = collapsed.chars().take(BODY_SNIPPET_CHARS).collect();
    if collapsed.chars().count() > BODY_SNIPPET_CHARS {
        text.push_str("...");
    }
    format!("Response starts with: {text}")
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // ------------------------------------------------------------------
    // Fixtures
    // ------------------------------------------------------------------

    /// Scratch root under `.scratch/install-test`, removed at the start of a run.
    fn scratch(name: &str) -> PathBuf {
        let root = repo_root()
            .join(".scratch")
            .join("install-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn repo_root() -> PathBuf {
        let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut current = start.clone();

        loop {
            if current.join("AGENTS.md").is_file() {
                return current;
            }
            if !current.pop() {
                return start;
            }
        }
    }

    fn fake_game(root: &Path) -> PathBuf {
        let game_root = root.join("Deadlock");
        fs::create_dir_all(game_root.join("game").join("citadel")).unwrap();
        fs::write(
            game_root.join("game").join("citadel").join("gameinfo.gi"),
            "// fixture gameinfo\nSearchPaths\n{\n\tGame citadel\n}\n",
        )
        .unwrap();
        game_root
    }

    /// Minimal stored ZIP writer. The zip crate writer is not used on purpose:
    /// hostile entry names and file types have to survive into the fixture.
    fn build_stored_zip(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut central = Vec::new();
        let mut offsets = Vec::new();

        for (name, data, _) in entries {
            let crc = crc32fast::hash(data);
            let offset = output.len() as u32;
            offsets.push(offset);

            output.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            output.extend_from_slice(&20_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&crc.to_le_bytes());
            output.extend_from_slice(&(data.len() as u32).to_le_bytes());
            output.extend_from_slice(&(data.len() as u32).to_le_bytes());
            output.extend_from_slice(&(name.len() as u16).to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(name.as_bytes());
            output.extend_from_slice(data);
        }

        let central_start = output.len() as u32;

        for (index, (name, data, mode)) in entries.iter().enumerate() {
            let crc = crc32fast::hash(data);

            central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            central.extend_from_slice(&(((3_u16) << 8) | 20).to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&(mode << 16).to_le_bytes());
            central.extend_from_slice(&offsets[index].to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }

        output.extend_from_slice(&central);
        output.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        output.extend_from_slice(&(central.len() as u32).to_le_bytes());
        output.extend_from_slice(&central_start.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output
    }

    struct Route {
        path: &'static str,
        status: u16,
        location: Option<&'static str>,
        body: Vec<u8>,
    }

    /// Serve routes from 127.0.0.1 on an ephemeral port. The thread is detached
    /// and dies with the test process.
    fn serve(routes: Vec<Route>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .ok();

                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();

                let (status, location, body) = match routes.iter().find(|route| route.path == path) {
                    Some(route) => (route.status, route.location, route.body.as_slice()),
                    None => (404, None, b"not found".as_slice()),
                };

                let mut head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                if let Some(location) = location {
                    head.push_str(&format!("Location: {location}\r\n"));
                }
                if !body.is_empty() {
                    head.push_str("Content-Type: application/zip\r\n");
                }
                head.push_str("\r\n");

                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });

        format!("http://127.0.0.1:{port}")
    }

    fn plan_options(game_root: &Path, origin: &str) -> PlanOptions {
        PlanOptions {
            game_root: Some(game_root.to_path_buf()),
            policy: DownloadPolicy::gamebanana().with_mirror(origin).unwrap(),
            limits: ExtractLimits::default(),
            tools: ToolPaths::default(),
        }
    }

    /// Build a real pakNN_dir.vpk plus chunk through the crate writer.
    fn make_vpk(directory: &Path, base: &str, game_paths: &[(&str, &[u8])]) -> (PathBuf, Vec<PathBuf>) {
        let staging = directory.join(format!("staging-{base}"));
        let _ = fs::remove_dir_all(&staging);

        for (path, data) in game_paths {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, data).unwrap();
        }

        let packed = pack_vpk(&PackOptions::new(staging, directory.to_path_buf(), base)).unwrap();
        (packed.dir_path, packed.chunk_paths)
    }

    // ------------------------------------------------------------------
    // Source and URL handling
    // ------------------------------------------------------------------

    #[test]
    fn parses_install_sources() {
        assert_eq!(
            InstallSource::parse("715100").unwrap(),
            InstallSource::GameBananaId(715100)
        );
        assert_eq!(
            InstallSource::parse("https://gamebanana.com/mods/715100").unwrap(),
            InstallSource::GameBananaId(715100)
        );
        assert_eq!(
            InstallSource::parse("https://gamebanana.com/mods/715100/files").unwrap(),
            InstallSource::GameBananaId(715100)
        );
        assert_eq!(
            InstallSource::parse("https://files.gamebanana.com/mods/thing.7z").unwrap(),
            InstallSource::DirectUrl("https://files.gamebanana.com/mods/thing.7z".to_string())
        );
        // Allowed by the parser, refused by the download allowlist.
        assert!(matches!(
            InstallSource::parse("https://example.com/thing.zip").unwrap(),
            InstallSource::DirectUrl(_)
        ));

        assert!(InstallSource::parse("").is_err());
        assert!(InstallSource::parse("ftp://gamebanana.com/x.zip").is_err());
        assert!(InstallSource::parse("0").is_err());
        assert!(InstallSource::parse("/no/such/archive.zip").is_err());
        assert!(InstallSource::parse("https://user:pass@files.gamebanana.com/x.zip").is_err());
        assert!(InstallSource::parse("https://files.gamebanana.com:notaport/x.zip").is_err());
    }

    #[test]
    fn download_policy_is_https_gamebanana_plus_an_explicit_mirror() {
        let policy = DownloadPolicy::gamebanana();

        assert!(policy.allows(&WebUrl::parse("https://files.gamebanana.com/mods/x.7z").unwrap()));
        assert!(policy.allows(&WebUrl::parse("https://gamebanana.com/dl/123").unwrap()));
        // Real downloads redirect to numbered filecache hosts.
        assert!(policy.allows(
            &WebUrl::parse("https://filecache52.gamebanana.com/mods/x.zip").unwrap()
        ));
        // http is never allowed for the public hosts.
        assert!(!policy.allows(&WebUrl::parse("http://gamebanana.com/dl/123").unwrap()));
        assert!(!policy.allows(
            &WebUrl::parse("http://filecache52.gamebanana.com/mods/x.zip").unwrap()
        ));
        // A suffix that only looks like the domain is a different origin.
        assert!(!policy.allows(&WebUrl::parse("https://notgamebanana.com/x.zip").unwrap()));
        assert!(!policy.allows(&WebUrl::parse("https://gamebanana.com.evil.test/x.zip").unwrap()));
        assert!(!policy.allows(&WebUrl::parse("https://example.com/x.zip").unwrap()));
        // A different port on a public host is a different origin.
        assert!(!policy.allows(&WebUrl::parse("https://files.gamebanana.com:8443/x.zip").unwrap()));

        let mirrored = policy.with_mirror("http://127.0.0.1:8123").unwrap();
        assert!(mirrored.allows(&WebUrl::parse("http://127.0.0.1:8123/mod.zip").unwrap()));
        assert!(!mirrored.allows(&WebUrl::parse("http://127.0.0.1:9999/mod.zip").unwrap()));

        // Bare host defaults to https.
        let bare = DownloadPolicy::gamebanana()
            .with_mirror("mirror.internal")
            .unwrap();
        assert!(bare.allows(&WebUrl::parse("https://mirror.internal/mod.zip").unwrap()));
        assert!(!bare.allows(&WebUrl::parse("http://mirror.internal/mod.zip").unwrap()));
    }

    #[test]
    fn resolves_redirects_and_rejects_relative_ones() {
        let base = WebUrl::parse("https://gamebanana.com/dl/123").unwrap();

        assert_eq!(
            resolve_location(&base, "https://files.gamebanana.com/mods/x.7z")
                .unwrap()
                .to_string(),
            "https://files.gamebanana.com/mods/x.7z"
        );
        assert_eq!(
            resolve_location(&base, "/mod.zip").unwrap().to_string(),
            "https://gamebanana.com/mod.zip"
        );
        assert!(resolve_location(&base, "mod.zip").is_err());
    }

    #[test]
    fn download_refuses_an_origin_outside_the_allowlist_before_connecting() {
        let root = scratch("allowlist");
        let error = download(
            "https://example.com/mod.zip",
            &root.join("downloads"),
            &DownloadPolicy::gamebanana(),
        )
        .unwrap_err();

        let text = error.to_string();
        assert!(text.contains("not in the install allowlist"), "{text}");
        assert!(text.contains("https://example.com"), "{text}");
        assert!(!root.join("downloads").exists());
        let _ = fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Extraction safety
    // ------------------------------------------------------------------

    #[test]
    fn zip_slip_absolute_backslash_and_links_are_rejected() {
        let root = scratch("zip-slip");
        let limits = ExtractLimits::default();

        for (name, entry, needle) in [
            ("parent", "../evil.vmat_c", "escapes the extract folder"),
            ("nested", "materials/../../evil.vmat_c", "escapes the extract folder"),
            ("absolute", "/tmp/evil.vmat_c", "absolute path"),
            ("drive", "C:/evil.vmat_c", "drive letter"),
            (
                "backslash",
                "materials\\heroes\\evil.vmat_c",
                "uses backslashes",
            ),
        ] {
            let archive = root.join(format!("{name}.zip"));
            fs::write(&archive, build_stored_zip(&[(entry, b"payload", 0o100644)])).unwrap();

            let destination = root.join(format!("out-{name}"));
            let error = extract_archive(&archive, &destination, &limits, &ToolPaths::default())
                .unwrap_err();
            let text = error.to_string();

            assert!(text.contains(needle), "{entry}: {text}");
            assert!(!destination.exists(), "{entry}: destination must be cleaned up");
            assert!(!root.join("evil.vmat_c").exists());
        }

        let link = root.join("link.zip");
        fs::write(
            &link,
            build_stored_zip(&[("citadel/link.vmat_c", b"/etc/passwd", 0o120777)]),
        )
        .unwrap();
        let error = extract_archive(&link, &root.join("out-link"), &limits, &ToolPaths::default())
            .unwrap_err();
        assert!(error.to_string().contains("symbolic link"), "{error}");

        let device = root.join("device.zip");
        fs::write(
            &device,
            build_stored_zip(&[("citadel/dev.vmat_c", b"x", 0o020644)]),
        )
        .unwrap();
        let error = extract_archive(&device, &root.join("out-device"), &limits, &ToolPaths::default())
            .unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_entries_are_accepted_but_still_validated() {
        let root = scratch("dir-entries");
        let limits = ExtractLimits::default();

        let archive = root.join("dirs.zip");
        fs::write(
            &archive,
            build_stored_zip(&[
                ("citadel/", b"", 0o040755),
                ("citadel/materials/", b"", 0o040755),
                (
                    "citadel/materials/heroes/billy/billy_color.vmat_c",
                    b"payload",
                    0o100644,
                ),
            ]),
        )
        .unwrap();

        let report = extract_archive(&archive, &root.join("out"), &limits, &ToolPaths::default())
            .unwrap();
        assert_eq!(
            report.files,
            vec!["citadel/materials/heroes/billy/billy_color.vmat_c".to_string()]
        );
        assert!(root.join("out").join("citadel").join("materials").is_dir());

        // A directory entry that climbs out is still refused.
        let escape = root.join("escape.zip");
        fs::write(
            &escape,
            build_stored_zip(&[("../", b"", 0o040755), ("ok.vmat_c", b"x", 0o100644)]),
        )
        .unwrap();
        let error = extract_archive(&escape, &root.join("out-escape"), &limits, &ToolPaths::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("escapes the extract folder"), "{error}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn entry_count_and_size_caps_are_enforced() {
        let root = scratch("caps");
        let archive = root.join("many.zip");
        fs::write(
            &archive,
            build_stored_zip(&[
                ("a.vmat_c", b"a", 0o100644),
                ("b.vmat_c", b"b", 0o100644),
                ("c.vmat_c", b"c", 0o100644),
            ]),
        )
        .unwrap();

        let limits = ExtractLimits {
            max_entries: 2,
            ..ExtractLimits::default()
        };
        let error = extract_archive(&archive, &root.join("out"), &limits, &ToolPaths::default())
            .unwrap_err();
        assert!(error.to_string().contains("above the 2 entry cap"), "{error}");

        let size_limits = ExtractLimits {
            max_total_bytes: 1,
            ..ExtractLimits::default()
        };
        let error = extract_archive(&archive, &root.join("out"), &size_limits, &ToolPaths::default())
            .unwrap_err();
        assert!(error.to_string().contains("byte cap"), "{error}");

        // Ratio bomb guard: 100 MiB declared from 100 bytes.
        let mut total = 0;
        let error = check_entry_budget(
            "bomb.vtex_c",
            100 * 1024 * 1024,
            100,
            &ExtractLimits::default(),
            &mut total,
        )
        .unwrap_err();
        assert!(error.to_string().contains("compression bomb"), "{error}");

        // Small entries are exempt from the ratio cap.
        let mut total = 0;
        check_entry_budget(
            "small.vtex_c",
            1024,
            1,
            &ExtractLimits::default(),
            &mut total,
        )
        .unwrap();

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn seven_zip_and_rar_name_the_missing_backend() {
        let root = scratch("backends");
        let limits = ExtractLimits::default();
        let tools = ToolPaths::default();

        let seven = root.join("mod.7z");
        fs::write(&seven, b"7z\xbc\xaf\x27\x1c\x00\x04rest").unwrap();
        let text = extract_archive(&seven, &root.join("out-7z"), &limits, &tools)
            .unwrap_err()
            .to_string();
        assert!(text.contains("Archive backend unavailable"), "{text}");
        assert!(text.contains("7Z"), "{text}");
        assert!(text.contains("7z"), "{text}");
        assert!(text.contains("DAH_7Z"), "{text}");
        assert!(!root.join("out-7z").exists());

        let rar = root.join("mod.rar");
        fs::write(&rar, b"Rar!\x1a\x07\x00rest").unwrap();
        let text = extract_archive(&rar, &root.join("out-rar"), &limits, &tools)
            .unwrap_err()
            .to_string();
        assert!(text.contains("RAR"), "{text}");
        assert!(text.contains("unrar"), "{text}");
        assert!(text.contains("DAH_UNRAR"), "{text}");

        assert_eq!(detect_archive_format(&seven), ArchiveFormat::SevenZip);
        assert_eq!(detect_archive_format(&rar), ArchiveFormat::Rar);

        let _ = fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Classification
    // ------------------------------------------------------------------

    #[test]
    fn detects_the_game_path_prefix() {
        let (prefix, mapped, unmapped) = detect_prefix(&[
            "game/citadel/materials/heroes/billy/billy_color.vmat_c".to_string(),
            "game/citadel/models/heroes_wip/billy/billy.vmdl_c".to_string(),
            "readme.md".to_string(),
        ]);
        assert_eq!(prefix.as_deref(), Some("game/citadel/"));
        assert_eq!(mapped.len(), 2);
        assert_eq!(unmapped, vec!["readme.md".to_string()]);

        let (prefix, mapped, unmapped) =
            detect_prefix(&["materials/heroes/billy/billy_color.vmat_c".to_string()]);
        assert_eq!(prefix.as_deref(), Some(""));
        assert_eq!(mapped.len(), 1);
        assert!(unmapped.is_empty());

        let (prefix, _, _) =
            detect_prefix(&["citadel/materials/heroes/billy/billy_color.vmat_c".to_string()]);
        assert_eq!(prefix.as_deref(), Some("citadel/"));

        // Deepest prefix wins a tie, so `game/citadel` is not read as `game`.
        let (prefix, _, _) = detect_prefix(&[
            "game/citadel/materials/a.vmat_c".to_string(),
            "game/core/materials/b.vmat_c".to_string(),
        ]);
        assert_eq!(prefix.as_deref(), Some("game/citadel/"));

        // No compiled file means the plan later fails with "nothing to install".
        let (prefix, mapped, unmapped) = detect_prefix(&["docs/readme.md".to_string()]);
        assert_eq!(prefix.as_deref(), Some(""));
        assert_eq!(mapped, vec!["docs/readme.md".to_string()]);
        assert!(unmapped.is_empty());

        assert_eq!(detect_prefix(&[]), (None, Vec::new(), Vec::new()));
    }

    #[test]
    fn parses_a_real_gamebanana_profile_shape() {
        // Trimmed from the live Mod/715100/ProfilePage response.
        let body = r#"{
            "_idRow": 715100,
            "_sName": "[VIOLET FIX] Taokaka over Drifter",
            "_sDownloadUrl": "https://gamebanana.com/mods/download/715100",
            "_aFiles": [
                {
                    "_idRow": 1810830,
                    "_sFile": "drifter_taokaka_vo.7z",
                    "_nFilesize": 714875,
                    "_tsDateAdded": 1788866361,
                    "_sDownloadUrl": "https://gamebanana.com/dl/1810830",
                    "_sMd5Checksum": "03c0ddc7a9579cbeb0eb2f7941ed77d1",
                    "_sVersion": "1.0.0"
                },
                {
                    "_idRow": 1892033,
                    "_sFile": "drifter_taokaka_3d31a.7z",
                    "_nFilesize": 14453644,
                    "_tsDateAdded": 1794831164,
                    "_sDownloadUrl": "https://gamebanana.com/dl/1892033",
                    "_sMd5Checksum": "579a89134b2152f85f13302695fd1059",
                    "_sVersion": "1.0.8"
                }
            ]
        }"#;

        let file = parse_mod_profile(body).unwrap();
        assert_eq!(file.file_id, 1892033);
        assert_eq!(file.file_name, "drifter_taokaka_3d31a.7z");
        assert_eq!(file.download_url, "https://gamebanana.com/dl/1892033");
        assert_eq!(file.size_bytes, Some(14_453_644));
        assert_eq!(file.version.as_deref(), Some("1.0.8"));
        assert_eq!(file.file_count, 2);

        let empty = parse_mod_profile(r#"{"_aFiles": []}"#);
        assert!(empty.unwrap_err().to_string().contains("no downloadable files"));

        let missing = parse_mod_profile(r#"{"_aFiles": [{"_sFile": "x.zip"}]}"#);
        assert!(missing.unwrap_err().to_string().contains("_sDownloadUrl"));

        assert!(parse_mod_profile("not json").is_err());
    }

    #[test]
    fn discovery_failure_is_reported_before_anything_is_written() {
        let discovery = InstallDiscovery {
            install: None,
            searched_roots: vec![PathBuf::from("/nope/steam")],
        };
        let text = require_game_root(&discovery).unwrap_err().to_string();
        assert!(text.contains("No Deadlock install"), "{text}");
        assert!(text.contains("/nope/steam"), "{text}");
        assert!(text.contains("Nothing was downloaded"), "{text}");
    }

    // ------------------------------------------------------------------
    // End to end
    // ------------------------------------------------------------------

    #[test]
    fn downloads_extracts_validates_plans_and_applies() {
        let root = scratch("e2e");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");

        let hero = "materials/heroes/billy/billy_color.vmat_c";
        let model = "models/heroes_wip/billy/billy.vmdl_c";
        let zip = build_stored_zip(&[
            (
                "citadel/materials/heroes/billy/billy_color.vmat_c",
                b"vmat payload",
                0o100644,
            ),
            (
                "citadel/models/heroes_wip/billy/billy.vmdl_c",
                b"vmdl payload",
                0o100644,
            ),
            ("citadel/readme.txt", b"install notes", 0o100644),
            ("__MACOSX/._junk", b"finder junk", 0o100644),
        ]);

        let origin = serve(vec![
            Route {
                path: "/mod.zip",
                status: 200,
                location: None,
                body: zip.clone(),
            },
            Route {
                path: "/dl",
                status: 302,
                location: Some("/mod.zip"),
                body: Vec::new(),
            },
        ]);

        let policy = DownloadPolicy::gamebanana().with_mirror(&origin).unwrap();
        let request = InstallRequest::new(format!("{origin}/dl"), workspace.clone()).unwrap();

        // Download first, on its own, so the hash and byte count are checked.
        let cache = download_dir(&workspace, &request.source);
        let report = download(&format!("{origin}/dl"), &cache, &policy).unwrap();
        assert_eq!(report.file_name, "mod.zip");
        assert_eq!(report.bytes, zip.len() as u64);
        assert_eq!(report.sha256, hex(&Sha256::digest(&zip)));
        assert_eq!(report.final_url, format!("{origin}/mod.zip"));
        assert!(report.path.is_file());
        assert!(!cache.join("mod.zip.part").exists());
        fs::remove_dir_all(&cache).unwrap();

        let plan = plan_install(&request, &plan_options(&game_root, &origin)).unwrap();

        assert_eq!(plan.schema, INSTALL_SCHEMA);
        assert_eq!(plan.slot, 1);
        assert_eq!(plan.prefix.as_deref(), Some("citadel/"));
        assert_eq!(plan.format, ArchiveFormat::Zip);
        assert_eq!(plan.files, vec![hero.to_string(), model.to_string()]);
        assert_eq!(plan.bytes, (b"vmat payload".len() + b"vmdl payload".len()) as u64);
        assert_eq!(plan.archive_sha256, hex(&Sha256::digest(&zip)));
        assert_eq!(plan.archive_bytes, zip.len() as u64);
        assert_eq!(plan.target, addons_dir(&game_root).join("pak01_dir.vpk"));
        assert!(!plan.replace_required);
        assert!(plan.replaced.is_empty());
        assert!(plan.gameinfo_repair);
        assert!(plan.conflicts.is_empty());
        assert!(
            plan.warnings.iter().any(|warning| warning.contains("readme.txt")),
            "{:?}",
            plan.warnings
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("__MACOSX/._junk")),
            "{:?}",
            plan.warnings
        );
        assert!(plan.plan_path.is_file());
        assert!(plan.extracted.is_dir());

        let loaded = load_plan(&workspace, &plan.id).unwrap();
        assert_eq!(loaded, plan);

        // Refuses without approval and writes nothing.
        let refused = apply_install(&plan, false).unwrap_err();
        assert!(refused.to_string().contains("not approved"), "{refused}");
        assert!(!plan.target.exists());

        let record = apply_install(&plan, true).unwrap();
        assert!(record.target.is_file());
        assert!(record.gameinfo_repaired);
        assert!(record.backups.is_empty());
        assert!(!record.entry.chunk_files.is_empty());

        // The written VPK holds exactly the validated payload.
        let directory = read_vpk_directory_with(&record.target, &ReadVpkOptions {
            keep_preload: false,
        })
        .unwrap();
        let mut shipped: Vec<String> = directory
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        shipped.sort();
        assert_eq!(shipped, plan.files);
        assert_eq!(record.entry.entries, 2);
        assert_eq!(record.entry.vpk_bytes, fs::metadata(&record.target).unwrap().len());
        assert_eq!(record.entry.vpk_sha256, sha256_file(&record.target).unwrap());

        for chunk in &record.entry.chunk_files {
            assert!(addons_dir(&game_root).join(chunk).is_file(), "{chunk}");
        }

        let gameinfo = fs::read_to_string(gameinfo_path(&game_root)).unwrap();
        assert!(gameinfo.contains("Game citadel/addons"), "{gameinfo}");

        let journal = read_journal(&workspace).unwrap();
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].plan_id, plan.id);

        let status = install_status(&workspace).unwrap();
        assert_eq!(status.len(), 1);
        assert!(status[0].present);
        assert!(!status[0].superseded);
        assert!(format_install_status(&status).contains("present"));

        // A second plan lands in the next free slot.
        let second = plan_install(&request, &plan_options(&game_root, &origin)).unwrap();
        assert_eq!(second.slot, 2);
        assert_eq!(second.conflicts.len(), 1);
        assert_eq!(second.conflicts[0].slot, 1);
        assert_eq!(second.conflicts[0].paths, plan.files);

        let pruned = prune_download_cache(&workspace).unwrap();
        assert_eq!(pruned.removed.len(), 1, "{:?}", pruned.removed);
        assert!(pruned.total_bytes > 0);
        assert!(!plan.extracted.exists());
        // The journal and the deployed file survive a cache prune.
        assert_eq!(install_status(&workspace).unwrap()[0].present, true);
        assert!(record.target.is_file());

        let _ = fs::remove_dir_all(&root);
    }

    /// The 7z/rar path is proved with a stand-in script: no real 7z binary is
    /// bundled or downloaded, so this checks the exact argument array and the
    /// post-extraction tree scan, not 7z's own archive handling.
    #[cfg(unix)]
    #[test]
    fn external_backend_arguments_and_post_scan_are_checked() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("external");
        let limits = ExtractLimits::default();

        let archive = root.join("mod.7z");
        fs::write(&archive, b"7z\xbc\xaf\x27\x1c\x00\x04fake").unwrap();

        let tool = root.join("fake7z");
        fs::write(
            &tool,
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in -o*) dest=\"${arg#-o}\" ;; esac; done\nmkdir -p \"$dest/materials\"\nprintf 'cfg payload' > \"$dest/materials/hero.cfg\"\n",
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();

        let tools = ToolPaths {
            seven_zip: Some(tool),
            unrar: None,
        };

        let report = extract_archive(&archive, &root.join("out"), &limits, &tools).unwrap();
        assert_eq!(report.format, ArchiveFormat::SevenZip);
        assert_eq!(report.files, vec!["materials/hero.cfg".to_string()]);
        assert_eq!(report.bytes, b"cfg payload".len() as u64);
        assert!(root.join("out").join("materials").join("hero.cfg").is_file());

        // A backend that writes a symbolic link is caught by the tree scan.
        let linking = root.join("linking7z");
        fs::write(
            &linking,
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in -o*) dest=\"${arg#-o}\" ;; esac; done\nmkdir -p \"$dest/materials\"\nln -s /etc/passwd \"$dest/materials/hero.cfg\"\n",
        )
        .unwrap();
        fs::set_permissions(&linking, fs::Permissions::from_mode(0o755)).unwrap();

        let tools = ToolPaths {
            seven_zip: Some(linking),
            unrar: None,
        };
        let error = extract_archive(&archive, &root.join("out-link"), &limits, &tools)
            .unwrap_err()
            .to_string();
        assert!(error.contains("symbolic link"), "{error}");
        assert!(!root.join("out-link").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_over_a_disabled_slot_leaves_one_name() {
        let root = scratch("disabled-slot");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        let addons = addons_dir(&game_root);

        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk.disabled"), b"old disabled").unwrap();

        let archive = root.join("mod.zip");
        fs::write(
            &archive,
            build_stored_zip(&[(
                "citadel/materials/heroes/billy/billy_color.vmat_c",
                b"new payload",
                0o100644,
            )]),
        )
        .unwrap();

        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            tools: ToolPaths::default(),
            ..PlanOptions::default()
        };
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone())
            .unwrap()
            .with_slot(Some(1));

        let error = plan_install(&request, &options).unwrap_err().to_string();
        assert!(error.contains("pak01_dir.vpk.disabled"), "{error}");

        let plan = plan_install(&request.with_force(true), &options).unwrap();
        assert!(plan.replace_required);
        assert!(
            plan.replaced
                .iter()
                .any(|path| path.to_string_lossy().ends_with(".disabled"))
        );

        let record = apply_install(&plan, true).unwrap();
        assert!(plan.target.is_file());
        assert!(!addons.join("pak01_dir.vpk.disabled").exists());
        assert_eq!(
            record.entry.removed_files,
            vec!["pak01_dir.vpk.disabled".to_string()]
        );
        assert!(
            record
                .backups
                .iter()
                .any(|path| path.to_string_lossy().ends_with(".disabled"))
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tree_without_compiled_files_installs_nothing() {
        let root = scratch("no-compiled");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");

        let archive = root.join("docs.zip");
        fs::write(
            &archive,
            build_stored_zip(&[("citadel/docs/readme.txt", b"notes", 0o100644)]),
        )
        .unwrap();

        let request =
            InstallRequest::new(archive.display().to_string(), workspace.clone()).unwrap();
        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            ..PlanOptions::default()
        };

        let error = plan_install(&request, &options).unwrap_err().to_string();
        assert!(error.contains("No compiled _c files"), "{error}");
        assert!(!addons_dir(&game_root).exists());
        assert!(!plans_dir(&workspace).exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_refuses_when_the_plan_artifacts_are_gone_or_the_slot_changed() {
        let root = scratch("stale");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");

        let archive = root.join("mod.zip");
        fs::write(
            &archive,
            build_stored_zip(&[(
                "citadel/materials/heroes/billy/billy_color.vmat_c",
                b"payload",
                0o100644,
            )]),
        )
        .unwrap();

        let request =
            InstallRequest::new(archive.display().to_string(), workspace.clone()).unwrap();
        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            ..PlanOptions::default()
        };

        // 1. Cache pruned before apply: no staging tree, so apply refuses.
        let plan = plan_install(&request, &options).unwrap();
        prune_download_cache(&workspace).unwrap();
        let error = apply_install(&plan, true).unwrap_err().to_string();
        assert!(error.contains("is gone"), "{error}");
        assert!(!plan.target.exists());

        // 2. Plan, then something takes the slot: the plan is stale.
        let plan = plan_install(&request, &options).unwrap();
        fs::create_dir_all(plan.target.parent().unwrap()).unwrap();
        fs::write(&plan.target, b"someone else").unwrap();
        let error = apply_install(&plan, true).unwrap_err().to_string();
        assert!(error.contains("stale"), "{error}");
        assert_eq!(fs::read(&plan.target).unwrap(), b"someone else");

        // 3. The recorded game folder disappears: apply refuses.
        let plan = plan_install(&request, &options).unwrap();
        let _ = fs::remove_file(&plan.target);
        fs::remove_dir_all(game_root.join("game")).unwrap();
        let error = apply_install(&plan, true).unwrap_err().to_string();
        assert!(error.contains("no longer holds game/citadel"), "{error}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn denied_payload_is_rejected_with_the_offending_paths() {
        let root = scratch("denied");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");

        let cases = [
            (
                "gameplay",
                "citadel/scripts/heroes/billy.vdata_c",
                "scripts/heroes/billy.vdata_c",
            ),
            (
                "skeleton",
                "citadel/models/heroes_wip/billy/billy.vnmskel_c",
                "models/heroes_wip/billy/billy.vnmskel_c",
            ),
            (
                "source",
                "citadel/materials/heroes/billy/billy_color.vmat",
                "materials/heroes/billy/billy_color.vmat",
            ),
            ("binary", "citadel/tools/setup.exe", "tools/setup.exe"),
        ];

        for (name, entry, expected) in cases {
            let archive = root.join(format!("{name}.zip"));
            fs::write(
                &archive,
                build_stored_zip(&[
                    ("citadel/materials/heroes/billy/billy_color.vmat_c", b"ok", 0o100644),
                    (entry, b"denied", 0o100644),
                ]),
            )
            .unwrap();

            let request = InstallRequest::new(archive.display().to_string(), workspace.clone())
                .unwrap();
            let options = PlanOptions {
                game_root: Some(game_root.clone()),
                ..PlanOptions::default()
            };

            let error = plan_install(&request, &options).unwrap_err().to_string();
            assert!(error.contains("Payload rejected"), "{name}: {error}");
            assert!(error.contains(expected), "{name}: {error}");
        }

        assert!(!addons_dir(&game_root).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn occupied_slot_needs_force_and_is_backed_up() {
        let root = scratch("slot");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        let addons = addons_dir(&game_root);

        let (existing_vpk, existing_chunks) = make_vpk(
            &addons,
            "pak01",
            &[("materials/heroes/billy/billy_color.vmat_c", b"old payload")],
        );
        let original = fs::read(&existing_vpk).unwrap();
        assert!(existing_vpk.is_file());

        let zip = build_stored_zip(&[(
            "citadel/materials/heroes/billy/billy_color.vmat_c",
            b"new payload",
            0o100644,
        )]);
        let archive = root.join("mod.zip");
        fs::write(&archive, &zip).unwrap();

        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            tools: ToolPaths::default(),
            ..PlanOptions::default()
        };

        // No slot asked for: the next free one, no replacement.
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone()).unwrap();
        let free = plan_install(&request, &options).unwrap();
        assert_eq!(free.slot, 2);
        assert!(!free.replace_required);
        assert_eq!(free.conflicts.len(), 1);

        // Slot 1 is taken and force was not passed.
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone())
            .unwrap()
            .with_slot(Some(1));
        let error = plan_install(&request, &options).unwrap_err().to_string();
        assert!(error.contains("taken"), "{error}");
        assert!(error.contains("pak01_dir.vpk"), "{error}");
        assert!(error.contains("--force"), "{error}");
        assert!(error.contains("Nothing was written"), "{error}");
        assert_eq!(fs::read(&existing_vpk).unwrap(), original);

        // With force the plan lists every file it would replace.
        let request = request.with_force(true);
        let plan = plan_install(&request, &options).unwrap();
        assert_eq!(plan.slot, 1);
        assert!(plan.replace_required);
        assert!(plan.replaced.contains(&plan.target));
        for chunk in &existing_chunks {
            assert!(plan.replaced.contains(chunk), "{}", chunk.display());
        }

        let record = apply_install(&plan, true).unwrap();
        assert!(!record.backups.is_empty());
        assert!(
            record
                .backups
                .iter()
                .any(|path| path.file_name().unwrap() == "pak01_dir.vpk")
        );
        assert!(record.backups.iter().all(|path| path.is_file()));
        assert_ne!(fs::read(&plan.target).unwrap(), original);

        let directory = read_vpk_directory_with(&plan.target, &ReadVpkOptions {
            keep_preload: false,
        })
        .unwrap();
        assert_eq!(directory.entries.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installs_an_archive_that_already_ships_a_vpk() {
        let root = scratch("vpk-candidate");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");

        let source_dir = root.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let (dir_vpk, chunks) = make_vpk(
            &source_dir,
            "pak02",
            &[("materials/heroes/billy/billy_color.vmat_c", b"packed payload")],
        );
        assert!(dir_vpk.is_file());
        assert!(!chunks.is_empty());

        let mut entries: Vec<(String, Vec<u8>, u32)> = vec![(
            "pak02_dir.vpk".to_string(),
            fs::read(&dir_vpk).unwrap(),
            0o100644,
        )];
        for chunk in &chunks {
            entries.push((
                chunk.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read(chunk).unwrap(),
                0o100644,
            ));
        }
        let refs: Vec<(&str, &[u8], u32)> = entries
            .iter()
            .map(|(name, data, mode)| (name.as_str(), data.as_slice(), *mode))
            .collect();

        let archive = root.join("mod.zip");
        fs::write(&archive, build_stored_zip(&refs)).unwrap();

        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            tools: ToolPaths::default(),
            ..PlanOptions::default()
        };
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone()).unwrap();
        let plan = plan_install(&request, &options).unwrap();

        assert!(plan.prefix.is_none());
        assert!(matches!(plan.artifact, InstallArtifact::Vpk { .. }));
        assert_eq!(
            plan.files,
            vec!["materials/heroes/billy/billy_color.vmat_c".to_string()]
        );

        // Slot 1, so the dir file and its chunk are renamed from pak02 to pak01.
        assert_eq!(plan.slot, 1);
        let record = apply_install(&plan, true).unwrap();
        assert!(plan.target.is_file());
        assert_eq!(record.entry.chunk_files, vec!["pak01_000.vpk".to_string()]);
        assert!(addons_dir(&game_root).join("pak01_000.vpk").is_file());

        let directory = read_vpk_directory_with(&plan.target, &ReadVpkOptions {
            keep_preload: false,
        })
        .unwrap();
        assert_eq!(directory.entries.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }
}
