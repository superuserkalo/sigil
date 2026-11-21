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
//! no shell: a fallback 7z or unrar backend is a resolved binary run with an
//! argument array, never through a shell and never downloaded on demand.
//!
//! Formats: ZIP, 7z (sevenz-rust2) and RAR (RARLAB's UnRAR through the unrar
//! crate) are all read in process, so a player needs no 7-Zip or WinRAR. A tool
//! from `DAH_7Z` / `DAH_UNRAR` or `PATH` is only tried when the built-in reader
//! fails (an unusual method). Many GameBanana uploads are `.rar` or `.7z`.
//!
//! A Browse install can be stopped while it runs: [`PlanOptions::cancel`] carries a flag the app
//! sets, and the download, extract and validate stages check it between chunks and entries. A
//! cancelled run deletes its partial archive and its half-extracted folder, never stores a plan
//! and never writes a game file. [`is_cancelled`] tells the app to show "Cancelled" instead of an
//! error line.

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

#[path = "install_shapes.rs"]
mod shapes;

#[cfg(test)]
#[path = "install_shape_tests.rs"]
mod shape_tests;

/// Schema version for plan files and journal entries.
pub const INSTALL_SCHEMA: u32 = 1;
/// Hard cap on one downloaded archive.
pub const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Hard cap on one extracted tree.
pub const MAX_EXTRACTED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Hard cap on archive entries.
pub const MAX_ARCHIVE_ENTRIES: usize = 50_000;
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
// Cancellation
// ---------------------------------------------------------------------------

/// True when the app set the cancel flag for a running Browse install.
///
/// The same flag is read by every stage, so one store from the app stops the download, the
/// extract or the validate that is running at that moment.
fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::SeqCst))
}

/// Stop at the next safe point when the caller cancelled. A stage that writes removes what it
/// wrote before it returns (see [`download_with_cancel`] and [`extract_archive_with_cancel`]).
fn check_cancel(cancel: Option<&AtomicBool>) -> Result<()> {
    if cancelled(cancel) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// True when `error` is [`Error::Cancelled`]. The app shows "Cancelled" for it instead of an
/// error line, the same way the download queue shows its own cancelled state.
///
/// ```
/// # use deadlock_core::error::Error;
/// # use deadlock_core::install::is_cancelled;
/// assert!(is_cancelled(&Error::Cancelled));
/// assert!(!is_cancelled(&Error::msg("boom")));
/// ```
pub fn is_cancelled(error: &Error) -> bool {
    matches!(error, Error::Cancelled)
}

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
    pub slot: Option<u16>,
    /// Replace an occupied slot instead of refusing.
    pub force: bool,
    /// The install started from a single press (Browse, a `sigil:` link). Archives that
    /// carry a `.disable_gb1click` marker refuse this, as their creator asked.
    #[serde(default)]
    pub one_click: bool,
    /// One `pakNN_dir.vpk` inside an archive that holds several.
    #[serde(default)]
    pub vpk: Option<String>,
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
            one_click: false,
            vpk: None,
            workspace,
        })
    }

    pub fn with_slot(mut self, slot: Option<u16>) -> Self {
        self.slot = slot;
        self
    }

    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    pub fn with_one_click(mut self, one_click: bool) -> Self {
        self.one_click = one_click;
        self
    }

    pub fn with_vpk(mut self, vpk: Option<String>) -> Self {
        self.vpk = vpk;
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
pub(crate) fn resolve_location(base: &WebUrl, location: &str) -> Result<WebUrl> {
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
/// partial file. Callers that hand the player a stop button use
/// [`download_with_cancel`].
pub fn download(url: &str, destination_dir: &Path, policy: &DownloadPolicy) -> Result<DownloadReport> {
    download_with_cancel(url, destination_dir, policy, None)
}

/// Download `url` into `destination_dir`, stopping when `cancel` is set.
///
/// The flag is read before the request, on every redirect, in the read loop after each chunk
/// and once more before the finished file is renamed into place. On a cancel the `.part` file
/// is deleted and [`Error::Cancelled`] is returned, so no later run can take a half file for a
/// complete download. The multi-range path in [`crate::fast_download`] reads the same flag after
/// every chunk of every part, so a cancellable install keeps the faster download.
pub fn download_with_cancel(
    url: &str,
    destination_dir: &Path,
    policy: &DownloadPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<DownloadReport> {
    check_cancel(cancel)?;

    let mut current = WebUrl::parse(url)?;
    policy.check(&current)?;

    fs::create_dir_all(destination_dir)?;

    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(policy.timeout_seconds)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .new_agent();

    let mut response = None;

    // Settings > Mods > Download server: a file-server redirect goes to the chosen server
    // first; if that one fails, the server GameBanana picked is tried (crate::net::route_download).
    let mut unrouted: Option<WebUrl> = None;

    for _ in 0..=MAX_REDIRECTS {
        check_cancel(cancel)?;

        let called = agent
            .get(&current.to_string())
            .header("User-Agent", GAMEBANANA_USER_AGENT)
            .header("Accept", "application/octet-stream, */*")
            .call();
        let mut attempt = match called {
            Ok(attempt) if !(unrouted.is_some() && attempt.status().as_u16() >= 400) => attempt,
            result => match unrouted.take() {
                Some(original) => {
                    current = original;
                    continue;
                }
                None => result.map_err(|error| {
                    Error::Network(format!("Download request to {current} failed: {error}"))
                })?,
            },
        };

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
            let routed = WebUrl::parse(&crate::net::route_download(&next.to_string()))
                .ok()
                .filter(|routed| *routed != next && policy.allows(routed));
            match routed {
                Some(routed) => {
                    unrouted = Some(next);
                    current = routed;
                }
                None => current = next,
            }
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

    let declared_length = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let file_name = file_name_for(&final_url, &response);
    let final_path = destination_dir.join(&file_name);
    let partial_path = destination_dir.join(format!("{file_name}.part"));

    let _ = fs::remove_file(&partial_path);

    // Settings > Network > Faster downloads: a large file that allows ranges comes in parts
    // (crate::fast_download). Any failure falls back to the one stream below, except a cancel:
    // the parts read the same flag after every chunk, and a stop ends the run.
    let ranged = crate::fast_download::splittable(
        response.headers().get("accept-ranges").and_then(|v| v.to_str().ok()),
        response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok()),
        crate::net::download_parts(),
    );
    if let Some(total) = ranged {
        match crate::fast_download::fetch_ranges_with_cancel(
            &agent,
            &final_url.to_string(),
            GAMEBANANA_USER_AGENT,
            &final_path,
            total,
            crate::net::download_parts(),
            cancel,
        ) {
            Ok((bytes, sha256)) => {
                return Ok(DownloadReport {
                    requested_url: url.to_string(),
                    final_url: final_url.to_string(),
                    path: final_path,
                    file_name,
                    bytes,
                    sha256,
                });
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => {}
        }
    }

    let mut file = File::create(&partial_path)?;
    let mut hasher = Sha256::new();
    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];
    let mut total: u64 = 0;

    loop {
        if cancelled(cancel) {
            drop(file);
            let _ = fs::remove_file(&partial_path);
            return Err(Error::Cancelled);
        }

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

    // A connection the server closed cleanly before the end still reads as "done": the length
    // it declared says otherwise, and a cut archive must never reach the extractor.
    if let Some(length) = declared_length
        && length != total
    {
        let _ = fs::remove_file(&partial_path);
        return Err(Error::Network(format!(
            "Download from {final_url} ended after {total} of {length} bytes. Nothing was kept."
        )));
    }

    // A cancel that lands after the last chunk still wins: the bytes are complete, but the plan
    // was stopped, so the cache does not gain an archive for a run the player ended.
    if cancelled(cancel) {
        let _ = fs::remove_file(&partial_path);
        return Err(Error::Cancelled);
    }

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

/// [`download_with_cancel`] with retries for a dropped connection or a server hiccup, as Deadlock
/// Mod Manager retries (1 s, then 3 s). A refusal (an origin off the allowlist, a size cap, an
/// HTTP 4xx answer) and a cancel are not retried.
pub fn download_retrying(
    url: &str,
    destination_dir: &Path,
    policy: &DownloadPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<DownloadReport> {
    let mut attempt = 0;
    loop {
        match download_with_cancel(url, destination_dir, policy, cancel) {
            Err(Error::Network(text)) if attempt < 2 && is_transient(&text) => {
                attempt += 1;
                for _ in 0..(attempt * 10) {
                    check_cancel(cancel)?;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            other => return other,
        }
    }
}

fn is_transient(text: &str) -> bool {
    if text.contains("allowlist") || text.contains("cap") || text.contains("redirect") {
        return false;
    }
    if let Some(at) = text.find("returned HTTP ") {
        let code = &text[at + 14..];
        return code.starts_with('5') || code.starts_with("429") || code.starts_with("408");
    }
    true
}

/// How a GameBanana download differs from its file record, or None when it matches.
fn integrity_problem(file: &GameBananaFile, report: &DownloadReport) -> Option<String> {
    if let Some(expected) = file.size_bytes
        && expected > 0
        && expected != report.bytes
    {
        return Some(format!("is {} bytes where GameBanana lists {expected}", report.bytes));
    }
    let expected = file.md5.as_deref()?.trim().to_ascii_lowercase();
    if expected.len() != 32 {
        return None;
    }
    let actual = md5_file(&report.path).ok()?;
    (actual != expected).then(|| "does not match the checksum GameBanana lists".to_string())
}

/// The file's MD5 is `expected` (hex, any case). A file that cannot be read does not match.
pub fn md5_file_matches(path: &Path, expected: &str) -> bool {
    md5_file(path).is_ok_and(|actual| actual.eq_ignore_ascii_case(expected.trim()))
}

fn md5_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }
    Ok(format!("{:x}", context.compute()))
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

    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
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
    /// A bare `.vpk` download or file: nothing to unpack, the VPK (and its numbered parts next to
    /// it) is the mod. GameBanana hosts some mods this way, and Grimoire imports them directly.
    Vpk,
    Unknown,
}

impl ArchiveFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
            Self::Rar => "rar",
            Self::Vpk => "vpk",
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
    if read >= 4 && u32::from_le_bytes([magic[0], magic[1], magic[2], magic[3]]) == 0x55aa_1234 {
        return ArchiveFormat::Vpk;
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

/// What a file that is no archive looks like, for the "could not open" message: a web page
/// GameBanana served instead of the file, an empty download, or something else entirely.
fn describe_unknown_file(path: &Path) -> String {
    let mut head = [0u8; 512];
    let read = File::open(path).and_then(|mut file| file.read(&mut head)).unwrap_or(0);
    let text = String::from_utf8_lossy(&head[..read]).to_ascii_lowercase();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if read == 0 {
        return format!("{name} is empty: the download did not finish. Sigil deleted it; press Install again to download it fresh.");
    }
    if text.trim_start().starts_with("<!doctype html") || text.trim_start().starts_with("<html") || text.contains("<head") {
        return format!("{name} is a web page, not a mod archive: GameBanana or a network filter answered with a page instead of the file. Sigil deleted it; press Install again, and if it repeats, download the file in your browser and import it.");
    }
    if head[..read.min(2)] == [0x1f, 0x8b] || head[..read.min(3)] == *b"BZh" || (read >= 262 && &head[257..262] == b"ustar") {
        return format!("{name} is a .tar, .gz or .bz2 archive. Neither Sigil nor the other Deadlock mod managers open those: unpack it with 7-Zip and import the .vpk inside.");
    }
    format!("{name} is not a .zip, .7z, .rar or .vpk file (its first bytes match none of them), so Sigil cannot open it. If it is a mod, unpack it with 7-Zip and import the .vpk inside.")
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
/// destination tree. Callers that hand the player a stop button use
/// [`extract_archive_with_cancel`].
pub fn extract_archive(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    tools: &ToolPaths,
) -> Result<ExtractReport> {
    extract_archive_with_cancel(archive, destination, limits, tools, None)
}

/// Extract `archive` into `destination`, stopping when `cancel` is set.
///
/// The flag is read before the first entry, between entries and between the chunks of a ZIP
/// entry, so a stop lands in a fraction of a second even in a large archive. On a cancel the
/// whole destination tree is removed and [`Error::Cancelled`] is returned: a half-extracted
/// folder never survives to be classified or staged. An external `7z` or `unrar` helper cannot
/// be interrupted while it runs; the flag is checked before it starts and after it returns.
pub fn extract_archive_with_cancel(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    tools: &ToolPaths,
    cancel: Option<&AtomicBool>,
) -> Result<ExtractReport> {
    if !archive.is_file() {
        return Err(Error::msg(format!(
            "Archive {} does not exist.",
            archive.display()
        )));
    }

    if destination.exists() {
        shapes::remove_tree(destination)?;
    }
    fs::create_dir_all(destination)?;

    let format = detect_archive_format(archive);

    let result = if cancelled(cancel) {
        Err(Error::Cancelled)
    } else {
        extract_one(format, archive, destination, limits, tools, cancel)
    };

    // Nested archives (a .zip of variant .zips, a .rar inside the .7z), junk such as __MACOSX,
    // and read-only flags are dealt with on the written tree, before anything classifies it.
    let result = result.and_then(|_| {
        let mut warnings = Vec::new();
        shapes::tidy_tree(destination)?;
        shapes::expand_nested(destination, limits, tools, cancel, &mut warnings)?;
        let (files, bytes) = verify_tree(format.as_str(), archive, destination, limits)?;
        Ok((files, bytes, warnings))
    });

    match result {
        Ok((files, bytes, warnings)) => Ok(ExtractReport {
            format,
            directory: destination.to_path_buf(),
            files,
            bytes,
            warnings,
        }),
        Err(error) => {
            let _ = shapes::remove_tree(destination);
            Err(error)
        }
    }
}

/// Unpack one archive of a known format into `destination` (already empty).
fn extract_one(
    format: ArchiveFormat,
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    tools: &ToolPaths,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<String>, u64)> {
    // A web page or an empty file saved under an archive's name reads as one by its extension:
    // say what it really is before a reader calls it damaged.
    if format != ArchiveFormat::Vpk && shapes::not_an_archive(archive) {
        return Err(Error::msg(describe_unknown_file(archive)));
    }
    match format {
        ArchiveFormat::Zip => extract_zip(archive, destination, limits, cancel),
        // Built in, as for ZIP: no 7-Zip, WinRAR or unrar needed. A tool on PATH (or DAH_7Z /
        // DAH_UNRAR) is only a fallback for a method the built-in reader lacks, and never a
        // path out of a cancel.
        ArchiveFormat::SevenZip | ArchiveFormat::Rar => {
            let built_in = match format {
                ArchiveFormat::SevenZip => extract_7z(archive, destination, limits, cancel),
                _ => extract_rar(archive, destination, limits, cancel),
            };
            match (built_in, tools.for_format(format)) {
                (Ok(done), _) => Ok(done),
                (Err(error), _) if is_cancelled(&error) || shapes::is_password_error(&error) => Err(error),
                (Err(error), Some(tool)) => {
                    let _ = shapes::remove_tree(destination);
                    fs::create_dir_all(destination)?;
                    extract_with_tool(format, tool, archive, destination, limits, cancel)
                        .map_err(|tool_error| Error::msg(format!("{error} The {} fallback also failed: {tool_error}", format.tool_name())))
                }
                (Err(error), None) => Err(error),
            }
        }
        ArchiveFormat::Vpk => shapes::copy_bare_vpk(archive, destination, limits),
        ArchiveFormat::Unknown => Err(Error::msg(describe_unknown_file(archive))),
    }
}

fn extract_zip(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<String>, u64)> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| Error::msg(damaged_archive("ZIP", archive, &error.to_string())))?;

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
        check_cancel(cancel)?;

        let mut entry = zip.by_index(index).map_err(|error| {
            let text = error.to_string();
            if text.to_ascii_lowercase().contains("password") || text.contains("encrypt") {
                Error::msg(shapes::password_message(archive))
            } else {
                Error::msg(format!(
                    "ZIP entry {index} of {} could not be read: {error}",
                    archive.display()
                ))
            }
        })?;

        if entry.encrypted() {
            return Err(Error::msg(shapes::password_message(archive)));
        }

        // A name that is not UTF-8 (a ZIP made on a Chinese or Japanese Windows without the UTF-8
        // flag) is read as the ZIP default code page instead of refused, as Grimoire does: the
        // name only labels a variant folder, game paths are ASCII. Windows tools also write `\`.
        let raw_name = match std::str::from_utf8(entry.name_raw()) {
            Ok(name) => name.to_string(),
            Err(_) => entry.name().to_string(),
        };
        let raw_name = portable_name(&raw_name);

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
            if cancelled(cancel) {
                return Err(Error::Cancelled);
            }

            let read = entry.read(&mut buffer).map_err(|error| {
                Error::msg(damaged_archive(
                    "ZIP",
                    archive,
                    &format!("entry {raw_name} failed after {written} bytes: {error}"),
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

/// An archive's entry name with `/` separators (7z and RAR store the creator's separator).
fn portable_name(raw: &str) -> String {
    raw.replace('\\', "/")
}

/// The marker every "damaged or partly downloaded archive" message carries, so the plan can
/// delete a cached download that will never open and a retry downloads it again.
pub const DAMAGED_ARCHIVE_MARKER: &str = "is damaged or only partly downloaded";

/// A reader could not get through an archive: almost always a download cut short or a broken
/// upload. Says so, and what to do, with the reader's own words at the end for a bug report.
fn damaged_archive(kind: &str, archive: &Path, detail: &str) -> String {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!(
        "The {kind} archive {name} {DAMAGED_ARCHIVE_MARKER}, so it cannot be unpacked. Press Install again to download a fresh copy. If it fails the same way twice, the upload itself is broken: try another file of the mod, or tell the creator. (Reader said: {detail})"
    )
}

/// A .7z mod, in process (sevenz-rust2). Each name goes through [`safe_entry_path`]; the entry
/// count and the bytes written are capped while writing, not after.
fn extract_7z(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<String>, u64)> {
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
        .map_err(|error| match error {
            sevenz_rust2::Error::PasswordRequired => Error::msg(shapes::password_message(archive)),
            other => Error::msg(damaged_archive("7z", archive, &other.to_string())),
        })?;
    let mut files: Vec<String> = Vec::new();
    let mut total: u64 = 0;
    let mut refused: Option<Error> = None;
    let outcome = reader.for_each_entries(|entry, data| {
        if cancelled(cancel) {
            refused = Some(Error::Cancelled);
            return Ok(false);
        }
        if entry.is_directory() {
            return Ok(true);
        }
        if files.len() >= limits.max_entries {
            refused = Some(Error::msg(format!(
                "{} holds more than {} files, above the cap.",
                archive.display(),
                limits.max_entries
            )));
            return Ok(false);
        }
        let name = portable_name(entry.name());
        let path = match safe_entry_path(destination, &name) {
            Ok(path) => path,
            Err(error) => {
                refused = Some(error);
                return Ok(false);
            }
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&path)?;
        let room = limits.max_total_bytes.saturating_sub(total);
        total += std::io::copy(&mut data.take(room + 1), &mut out)?;
        if total > limits.max_total_bytes {
            refused = Some(Error::msg(format!(
                "{} unpacks to more than {} bytes, above the cap.",
                archive.display(),
                limits.max_total_bytes
            )));
            return Ok(false);
        }
        files.push(name);
        Ok(true)
    });
    if let Some(error) = refused {
        return Err(error);
    }
    outcome.map_err(|error| match error {
        // AES is the only 7z cipher, and it means a password.
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => {
            Error::msg(shapes::password_message(archive))
        }
        sevenz_rust2::Error::UnsupportedCompressionMethod(method) if method.contains("AES") => {
            Error::msg(shapes::password_message(archive))
        }
        other => Error::msg(damaged_archive("7z", archive, &other.to_string())),
    })?;
    // The same tree check as a tool's output: no links, nothing outside, caps.
    verify_tree("7z", archive, destination, limits)
}

/// A .rar mod, in process (the unrar crate builds RARLAB's UnRAR library). Names go through
/// [`safe_entry_path`] and each file is written to the path that returns, never to the name
/// the archive gives; directories, split volumes and encrypted entries are not followed.
fn extract_rar(
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<String>, u64)> {
    let fail = |error: unrar::error::UnrarError| {
        let text = error.to_string();
        if text.to_ascii_lowercase().contains("password") {
            Error::msg(shapes::password_message(archive))
        } else {
            Error::msg(damaged_archive("RAR", archive, &text))
        }
    };
    let mut open = unrar::Archive::new(archive).open_for_processing().map_err(fail)?;
    let mut count = 0usize;
    let mut declared: u64 = 0;
    while let Some(header) = open.read_header().map_err(fail)? {
        check_cancel(cancel)?;

        let entry = header.entry();
        if entry.is_encrypted() {
            return Err(Error::msg(shapes::password_message(archive)));
        }
        if entry.is_split_before() {
            return Err(Error::msg(format!(
                "{} is a later part of a split RAR (name.part2.rar and on). Download every part into one folder and import the first one (name.part1.rar).",
                archive.display()
            )));
        }
        if entry.is_directory() {
            open = header.skip().map_err(fail)?;
            continue;
        }
        count += 1;
        declared = declared.saturating_add(entry.unpacked_size);
        if count > limits.max_entries || declared > limits.max_total_bytes {
            return Err(Error::msg(format!(
                "{} is larger than Sigil unpacks ({} files or {} bytes at most).",
                archive.display(),
                limits.max_entries,
                limits.max_total_bytes
            )));
        }
        let name = portable_name(&entry.filename.to_string_lossy());
        let path = safe_entry_path(destination, &name)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        open = header.extract_to(&path).map_err(fail)?;
    }
    // Sizes the archive declares can lie: the written tree is checked again.
    verify_tree("RAR", archive, destination, limits)
}

/// The files under `destination` after an extract, with the caps and the no-links rule applied
/// to what was actually written. `what` names the extractor in an error.
fn verify_tree(
    what: &str,
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
) -> Result<(Vec<String>, u64)> {
    let files = walk_tree(destination)?;
    if files.len() > limits.max_entries {
        return Err(Error::msg(format!(
            "{what} extracted {} entries from {}, above the {} entry cap.",
            files.len(),
            archive.display(),
            limits.max_entries
        )));
    }
    let mut total: u64 = 0;
    for (_, size) in &files {
        total += size;
        if total > limits.max_total_bytes {
            return Err(Error::msg(format!(
                "{what} extracted more than {} bytes from {}, above the cap.",
                limits.max_total_bytes,
                archive.display()
            )));
        }
    }
    let mut paths: Vec<String> = files.into_iter().map(|(path, _)| path).collect();
    paths.sort();
    Ok((paths, total))
}

fn extract_with_tool(
    format: ArchiveFormat,
    tool: &Path,
    archive: &Path,
    destination: &Path,
    limits: &ExtractLimits,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<String>, u64)> {
    check_cancel(cancel)?;

    if !tool.is_file() {
        return Err(Error::msg(format!(
            "Archive backend unavailable: {} is not a file, so {} archives cannot be extracted. Set {} to the binary path.",
            tool.display(),
            format.as_str().to_uppercase(),
            format.env_name()
        )));
    }

    let mut command = crate::util::hidden_command(tool);

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

    // The helper ran to the end; a stop that arrived meanwhile still removes its tree.
    check_cancel(cancel)?;

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

/// Walk an extracted tree and group it into one VPK or one compiled tree.
///
/// `chosen` is a path from [`vpk_choices`]: a VPK file, or a loose mod folder ending in `/`
/// (`./` for files at the top of the archive).
fn classify_extracted(
    extracted: &Path,
    limits: &ExtractLimits,
    chosen_vpk: Option<&str>,
) -> Result<Candidate> {
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

    let vpks: Vec<&(String, u64)> = vpk_dir_files(extracted, &files);

    if let Some(chosen) = chosen_vpk.map(str::trim).filter(|vpk| !vpk.is_empty()) {
        if chosen.ends_with('/') {
            let folder = if chosen == "./" { "" } else { chosen };
            return classify_tree(extracted, files.clone(), Some(folder));
        }
        let picked = vpks
            .iter()
            .find(|(path, _)| path == chosen)
            .or_else(|| {
                vpks.iter().find(|(path, _)| {
                    path.eq_ignore_ascii_case(chosen) || path.ends_with(&format!("/{chosen}"))
                })
            })
            .ok_or_else(|| {
                Error::msg(format!(
                    "The archive has no VPK called {chosen}. It ships: {}.",
                    vpks.iter()
                        .map(|(path, _)| path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        return classify_vpk(extracted, &picked.0, &files);
    }

    if vpks.len() > 1 {
        let names: Vec<&str> = vpks.iter().map(|(path, _)| path.as_str()).collect();
        return Err(Error::msg(format!(
            "The archive ships more than one VPK ({}). Choose the one you want.",
            names.join(", ")
        )));
    }

    if let Some((relative, _)) = vpks.first() {
        return classify_vpk(extracted, relative, &files);
    }

    let groups = loose_groups(&files);
    if groups.len() > 1 {
        let names: Vec<String> = groups.iter().map(|(prefix, _)| folder_label(prefix).to_string()).collect();
        return Err(Error::msg(format!(
            "The archive ships more than one VPK or mod folder ({}). Choose the one you want.",
            names.join(", ")
        )));
    }
    let prefix = groups.first().map(|(prefix, _)| prefix.clone());
    classify_tree(extracted, files, prefix.as_deref())
}

/// A loose mod folder's label for the picker: the creator's folder name, or "top level".
fn folder_label(prefix: &str) -> &str {
    let mut trimmed = prefix.trim_end_matches('/');
    for tail in ["/game/citadel", "/citadel", "/game", "/addons"] {
        if trimmed.len() > tail.len() && trimmed.to_ascii_lowercase().ends_with(tail) {
            trimmed = &trimmed[..trimmed.len() - tail.len()];
        }
    }
    if trimmed.is_empty() || trimmed == "." || ["game/citadel", "citadel", "game", "addons"].contains(&trimmed.to_ascii_lowercase().as_str()) {
        return "top level files";
    }
    trimmed
}

/// The loose mod folders of an archive: each packaging prefix above a game folder that holds at
/// least one installable file, with the bytes it would install. Variant folders (`Red/materials`,
/// `Blue/materials`) are separate groups the player chooses between. A second engine tree
/// (`game/core/...` next to `game/citadel/...`) is not a variant and is dropped.
fn loose_groups(files: &[(String, u64)]) -> Vec<(String, u64)> {
    let mut groups: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for (relative, size) in files {
        if let Some((prefix, game_path)) = shapes::split_at_game_root(relative)
            && shapes::is_loose_payload(&game_path)
        {
            *groups.entry(prefix).or_default() += size;
        }
    }
    // A font pack ships `fonts/Name.ttf` and a script to copy it: the fonts alone are a mod.
    if groups.is_empty() {
        let fonts: u64 = files
            .iter()
            .filter(|(relative, _)| shapes::loose_font_path(relative).is_some())
            .map(|(_, size)| size)
            .sum();
        if fonts > 0 {
            groups.insert(String::new(), fonts);
        }
    }
    let citadel = groups.keys().any(|prefix| {
        let lower = prefix.to_ascii_lowercase();
        lower.is_empty() || lower.ends_with("citadel/")
    });
    if citadel && groups.len() > 1 {
        groups.retain(|prefix, _| {
            let lower = prefix.to_ascii_lowercase();
            !(lower.ends_with("core/") || lower.ends_with("bin/"))
        });
    }
    groups.into_iter().collect()
}

/// The file name without folders, lower case, with copy decorations (`pak01_dir (1).vpk`)
/// removed.
fn vpk_file_name(relative: &str) -> String {
    shapes::canonical_vpk_name(relative.rsplit('/').next().unwrap_or(relative))
}

/// The folder part of an archive path, with its trailing slash, or "" at the top.
fn parent_of(relative: &str) -> &str {
    relative.rfind('/').map(|at| &relative[..=at]).unwrap_or("")
}

/// `<base>` when `name` is a numbered part `<base>_NNN.vpk`.
fn vpk_chunk_base(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".vpk")?;
    let (base, digits) = stem.rsplit_once('_')?;
    (digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit()) && !base.is_empty()).then_some(base)
}

/// The prefix a VPK's numbered parts share: `pak05` for `pak05_dir.vpk`, `josuke` for
/// `josuke_dir.vpk`, and the stem of a self-contained `Josuke Remaster.vpk`.
fn vpk_chunk_prefix(relative: &str) -> String {
    let name = vpk_file_name(relative);
    name.strip_suffix("_dir.vpk")
        .or_else(|| name.strip_suffix(".vpk"))
        .unwrap_or(&name)
        .to_string()
}

/// True when a `.vpk` at this archive path is a map, which ships inside an addon VPK at
/// `maps/<name>.vpk` rather than being an addon itself.
fn is_map_vpk_path(relative: &str) -> bool {
    shapes::split_at_game_root(relative)
        .is_some_and(|(_, game_path)| game_path.to_ascii_lowercase().starts_with("maps/"))
}

/// Every VPK directory file an archive ships, under any name and case, at any depth, as
/// Grimoire and Deadlock Mod Manager accept them: `pakNN_dir.vpk`, `<name>_dir.vpk` with its
/// `<name>_NNN.vpk` parts in the same folder, or one self-contained `<name>.vpk`. A numbered
/// part next to its `_dir.vpk` is not a VPK of its own, a map under `maps/` is payload, not an
/// addon, and every candidate must carry the VPK header (a renamed text file is not a mod).
fn vpk_dir_files<'a>(extracted: &Path, files: &'a [(String, u64)]) -> Vec<&'a (String, u64)> {
    let dirs: std::collections::BTreeSet<String> = files
        .iter()
        .filter(|(path, _)| vpk_file_name(path).ends_with("_dir.vpk"))
        .map(|(path, _)| format!("{}{}", parent_of(path), vpk_file_name(path)))
        .collect();
    files
        .iter()
        .filter(|(path, _)| {
            let name = vpk_file_name(path);
            if !name.ends_with(".vpk") || is_map_vpk_path(path) {
                return false;
            }
            // A `_dir.vpk` name is a VPK on its name alone, as before: one with a broken header
            // then fails on its file list with a message that says so.
            if name.ends_with("_dir.vpk") {
                return true;
            }
            if let Some(base) = vpk_chunk_base(&name)
                && dirs.contains(&format!("{}{base}_dir.vpk", parent_of(path)))
            {
                return false;
            }
            has_vpk_magic(&extracted.join(path))
        })
        .collect()
}

fn has_vpk_magic(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && u32::from_le_bytes(magic) == 0x55aa_1234
}

fn classify_vpk(
    extracted: &Path,
    relative: &str,
    files: &[(String, u64)],
) -> Result<Candidate> {
    let dir_path = extracted.join(relative);
    let prefix = vpk_chunk_prefix(relative);
    let folder = parent_of(relative);

    // Parts sit next to their directory file: a second variant folder with the same names
    // keeps its own parts.
    let chunk_relatives: Vec<String> = files
        .iter()
        .map(|(path, _)| path.clone())
        .filter(|path| parent_of(path) == folder && path != relative && is_chunk_for(path, &prefix))
        .collect();

    let chunks: Vec<PathBuf> = chunk_relatives
        .iter()
        .map(|path| extracted.join(path))
        .collect();

    let directory = read_vpk_directory_with(&dir_path, &ReadVpkOptions {
        keep_preload: false,
    })
    .map_err(|error| {
        Error::msg(format!(
            "{relative} has a VPK header but its file list cannot be read ({error}). The file {DAMAGED_ARCHIVE_MARKER} or broken: press Install again to download a fresh copy, and if it repeats, tell the creator."
        ))
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

        let expected = format!("{prefix}_{:03}.vpk", entry.archive_index);
        let present = chunk_relatives
            .iter()
            .any(|path| vpk_file_name(path) == expected);

        if !present {
            return Err(Error::msg(format!(
                "The archive VPK {relative} needs its part {expected}, which is not next to it. The mod would load with missing data, so nothing was installed. Multi-part mods are sometimes uploaded as several files on GameBanana: download every part into one folder and import the _dir.vpk from there."
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

fn is_chunk_for(relative: &str, prefix: &str) -> bool {
    let name = vpk_file_name(relative);
    vpk_chunk_base(&name) == Some(prefix)
}

/// Loose files under `prefix` (the packaging folders above the game folders), mapped to game
/// paths. Only files the game reads as they are ship ([`shapes::is_loose_payload`]); source
/// files, notes and programs are listed in a warning and left out, never a reason to refuse.
fn classify_tree(extracted: &Path, files: Vec<(String, u64)>, prefix: Option<&str>) -> Result<Candidate> {
    let mut mapped = Vec::new();
    let mut skipped = Vec::new();
    let mut programs = Vec::new();
    let mut unmapped = Vec::new();
    let mut payload = Vec::new();
    let mut payload_bytes = 0;

    for (relative, size) in &files {
        let game_path = prefix.and_then(|prefix| {
            shapes::game_path_under(prefix, relative).or_else(|| shapes::loose_font_path(relative))
        });
        let Some(game_path) = game_path else {
            unmapped.push(relative.clone());
            continue;
        };

        mapped.push(CandidateFile {
            path: game_path.clone(),
            relative: relative.clone(),
            size: *size,
        });

        if shapes::is_loose_payload(&game_path) {
            payload.push(game_path);
            payload_bytes += size;
        } else {
            skipped.push(game_path);
        }
    }

    for path in unmapped.iter().chain(skipped.iter()) {
        let lower = path.to_ascii_lowercase();
        if [".exe", ".bat", ".cmd", ".ps1", ".msi", ".dll", ".vbs", ".scr", ".py", ".sh"]
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            programs.push(path.clone());
        }
    }

    let mut warnings = Vec::new();

    if !skipped.is_empty() {
        warnings.push(format!(
            "{} file(s) at game paths are not compiled and are left out (the game reads only compiled _c files): {}",
            skipped.len(),
            preview(&skipped)
        ));
    }

    if !unmapped.is_empty() {
        warnings.push(format!(
            "{} file(s) outside the mod's game folders are not installed: {}",
            unmapped.len(),
            preview(&unmapped)
        ));
    }

    if !programs.is_empty() {
        warnings.push(format!(
            "The archive ships {} program(s) or script(s); Sigil never installs or runs them: {}",
            programs.len(),
            preview(&programs)
        ));
    }

    Ok(Candidate {
        artifact: CandidateArtifact::Tree {
            root: extracted.to_path_buf(),
        },
        prefix: prefix.map(str::to_string),
        mapped,
        payload,
        payload_bytes,
        unmapped,
        warnings,
    })
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
    pub slot: u16,
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
    pub slot: u16,
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

/// What one plan run may use: where the game is, what it may download, what it may unpack, and
/// the flag that stops it.
#[derive(Debug, Clone)]
pub struct PlanOptions {
    /// Explicit game root. None means Steam discovery, then `DAH_GAME_DIR`.
    pub game_root: Option<PathBuf>,
    pub policy: DownloadPolicy,
    pub limits: ExtractLimits,
    pub tools: ToolPaths,
    /// Set by the app to stop a running Browse install. None means the run cannot be
    /// cancelled, which is what the CLI and the tests use.
    ///
    /// The same flag reaches the download, the extract and the validate stage. A cancelled run
    /// returns [`Error::Cancelled`], leaves no plan file and writes no game file. Default None.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl PartialEq for PlanOptions {
    /// Every setting is compared except the cancel flag: a flag is a handle the app keeps, not a
    /// value two option sets agree on, and two runs with the same settings are the same run
    /// whether or not one of them can be stopped.
    fn eq(&self, other: &Self) -> bool {
        self.game_root == other.game_root
            && self.policy == other.policy
            && self.limits == other.limits
            && self.tools == other.tools
    }
}

impl Eq for PlanOptions {}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            game_root: None,
            policy: DownloadPolicy::gamebanana(),
            limits: ExtractLimits::default(),
            tools: ToolPaths::from_environment(),
            cancel: None,
        }
    }
}

/// One downloaded and extracted archive, shared by the plan and the VPK picker.
struct Prepared {
    cache: PathBuf,
    download: DownloadReport,
    extracted: PathBuf,
    extract: ExtractReport,
    gamebanana: Option<GameBananaFile>,
    warnings: Vec<String>,
}

/// Fetch the archive inside the workspace, extract it, and refuse an archive that asks for
/// no one-click installs when this request is one.
///
/// Every stage takes the plan's cancel flag. A cancel deletes the partial archive (inside the
/// download) or the half-extracted folder here, so a stopped run leaves nothing a later run
/// could take for a finished download or a finished extract.
fn prepare_archive(request: &InstallRequest, options: &PlanOptions) -> Result<Prepared> {
    let cancel = options.cancel.as_deref();
    check_cancel(cancel)?;

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
                (Some(path), Some(expected), Some(size))
                    if expected == size
                        && file.md5.as_deref().is_none_or(|md5| {
                            md5_file(path).map(|actual| actual.eq_ignore_ascii_case(md5.trim())).unwrap_or(false)
                        }) =>
                {
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
                _ => download_retrying(&file.download_url, &cache, &options.policy, cancel)?,
            };

            // GameBanana lists each file's size and MD5. A download that matches neither is
            // cut short or swapped for an error page: it is deleted and fetched once more, and a
            // second miss is an error rather than a broken install.
            let report = match integrity_problem(&file, &report) {
                None => report,
                Some(problem) => {
                    let _ = fs::remove_file(&report.path);
                    warnings.push(format!("The first download of {} {problem}; it was downloaded again.", file.file_name));
                    let again = download_retrying(&file.download_url, &cache, &options.policy, cancel)?;
                    if let Some(problem) = integrity_problem(&file, &again) {
                        let _ = fs::remove_file(&again.path);
                        return Err(Error::Network(format!(
                            "The download of {} {problem}, twice. The file was deleted and nothing was installed. GameBanana or a network filter is serving a broken copy: try again later, switch the download server in Settings > Network, or download it in your browser and import it.",
                            file.file_name
                        )));
                    }
                    again
                }
            };

            gamebanana = Some(file);
            report
        }
        InstallSource::DirectUrl(url) => download_retrying(url, &cache, &options.policy, cancel)?,
        InstallSource::LocalFile(path) => describe_local_file(path)?,
    };

    // A stop between the download and the extract keeps the completed archive (the bytes are
    // good and a retry can reuse them) but never starts the extract.
    check_cancel(cancel)?;

    let extracted = cache.join("extracted");
    let extract =
        match extract_archive_with_cancel(&fetched.path, &extracted, &options.limits, &options.tools, cancel) {
            Ok(extract) => extract,
            Err(error) => {
                // A downloaded archive that will not open is deleted, so the next press downloads a
                // fresh copy instead of failing on the same bytes. A player's own file stays.
                let downloaded = !matches!(request.source, InstallSource::LocalFile(_));
                let text = error.to_string();
                if downloaded && (text.contains(DAMAGED_ARCHIVE_MARKER) || text.contains("not a .zip, .7z, .rar") || text.contains("is a web page") || text.contains("is empty")) {
                    let _ = fs::remove_file(&fetched.path);
                }
                return Err(error);
            }
        };

    // A stop between the extract and the return removes the half tree, or the whole tree when
    // the extract had just finished: a cancelled plan has no extracted folder behind it.
    if cancelled(cancel) {
        let _ = fs::remove_dir_all(&extracted);
        return Err(Error::Cancelled);
    }

    refuse_one_click_marker(&extracted, request.one_click)?;

    Ok(Prepared {
        cache,
        download: fetched,
        extracted,
        extract,
        gamebanana,
        warnings,
    })
}

/// The GameBanana mod id an install came from, when there is one.
fn gamebanana_mod_id(source: &InstallSource) -> Option<u64> {
    match source {
        InstallSource::GameBananaId(id) => Some(*id),
        _ => None,
    }
}

/// The marker files creators put in an archive to say "no one-click installs".
pub const ONE_CLICK_MARKERS: [&str; 2] = [".disable_gb1click", ".disable_gb1click_sigil"];

/// The display name of a marker file, lower case, or None for any other entry.
pub fn one_click_marker(file_name: &str) -> Option<&'static str> {
    let name = file_name.rsplit('/').next().unwrap_or(file_name);
    ONE_CLICK_MARKERS
        .iter()
        .find(|marker| name.eq_ignore_ascii_case(marker))
        .copied()
}

/// An archive with a marker refuses a one-click install. Extracting it by hand and
/// importing the file stays allowed, which is what the marker asks for.
fn refuse_one_click_marker(extracted: &Path, one_click: bool) -> Result<()> {
    if !one_click {
        return Ok(());
    }
    let files = walk_tree(extracted)?;
    for (relative, _) in &files {
        if one_click_marker(relative).is_some() {
            return Err(Error::msg(
                "This creator does not allow one-click installs. Download the file from GameBanana, then import it from Installed. Nothing was installed.",
            ));
        }
    }
    Ok(())
}

/// One `pakNN_dir.vpk` an archive ships, for the picker that follows a multi-mod download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpkChoice {
    /// Path inside the archive, which is what the plan takes back.
    pub path: String,
    /// The directory VPK plus its chunk files.
    pub bytes: u64,
    pub chunks: usize,
}

impl VpkChoice {
    /// The row label: the path inside the archive, so two variant folders that both ship a
    /// `pak01_dir.vpk` read apart, or the creator's folder name for a loose mod folder.
    pub fn name(&self) -> &str {
        if self.path.ends_with('/') {
            return if self.path == "./" { "top level files" } else { folder_label(&self.path) };
        }
        let mut label = self.path.as_str();
        while let Some((first, rest)) = label.split_once('/') {
            if !["game", "citadel", "addons"].contains(&first.to_ascii_lowercase().as_str()) {
                break;
            }
            label = rest;
        }
        label
    }
}

/// Every VPK an archive holds, without planning one. Reads the cached download, so the
/// picker never downloads twice.
pub fn vpk_choices(request: &InstallRequest, options: &PlanOptions) -> Result<Vec<VpkChoice>> {
    let prepared = prepare_archive(request, options)?;
    let files = walk_tree(&prepared.extracted)?;
    Ok(collect_vpk_choices(&prepared.extracted, &files))
}

fn collect_vpk_choices(extracted: &Path, files: &[(String, u64)]) -> Vec<VpkChoice> {
    let mut choices = Vec::new();
    let vpks = vpk_dir_files(extracted, files);
    if vpks.is_empty() {
        // Loose variant folders: each one is a choice, packed into its own VPK.
        for (prefix, bytes) in loose_groups(files) {
            choices.push(VpkChoice {
                path: if prefix.is_empty() { "./".to_string() } else { prefix },
                bytes,
                chunks: 0,
            });
        }
    }
    for (relative, bytes) in vpks {
        let prefix = vpk_chunk_prefix(relative);
        let folder = parent_of(relative);
        let chunks: Vec<&(String, u64)> = files
            .iter()
            .filter(|(path, _)| parent_of(path) == folder && path != relative && is_chunk_for(path, &prefix))
            .collect();
        let total = bytes + chunks.iter().map(|(_, size)| *size).sum::<u64>();
        choices.push(VpkChoice {
            path: relative.clone(),
            bytes: total,
            chunks: chunks.len(),
        });
    }
    choices.sort_by(|a, b| a.path.cmp(&b.path));
    choices
}

/// Download, extract, validate and plan. Writes only inside the workspace.
/// What [`install_vpks`] managed: each file it installed, in order, and the error that stopped it
/// (`None` when every file went in).
#[derive(Debug)]
pub struct MultiInstall {
    pub installed: Vec<(InstallPlan, InstallRecord)>,
    pub error: Option<Error>,
}

/// Install several `pakNN_dir.vpk` files from one archive (a model and its voice pack, say), each
/// into its own free slot, in the order given: the first gets the lowest slot, so it wins where
/// two of them change the same file, as the author ordered them. `vpks` are paths from
/// [`vpk_choices`]. Each file is planned, stored and applied before the next is planned, so the
/// next lands in the next free slot. Stops at the first failure; files already installed stay
/// (each is a normal install the user can turn off or remove).
pub fn install_vpks(
    request: &InstallRequest,
    vpks: &[String],
    options: &PlanOptions,
    apply: ApplyOptions,
    approve: bool,
) -> MultiInstall {
    let mut installed = Vec::new();
    for vpk in vpks {
        let one = request.clone().with_vpk(Some(vpk.clone()));
        let step = plan_install(&one, options).and_then(|plan| {
            store_plan(&plan)?;
            let record = apply_install_with(&plan, approve, apply)?;
            Ok((plan, record))
        });
        match step {
            Ok(done) => installed.push(done),
            Err(error) => return MultiInstall { installed, error: Some(error) },
        }
    }
    MultiInstall { installed, error: None }
}

/// The start of the message for an archive with nothing to install, so the app can tell that
/// refusal apart (and offer the settings review, `crate::mod_settings`).
pub const NOTHING_INSTALLABLE: &str = "Nothing in this archive is a mod Sigil can install.";

pub fn plan_install(request: &InstallRequest, options: &PlanOptions) -> Result<InstallPlan> {
    if let Some(slot) = request.slot
        && !crate::addons::is_slot(slot)
    {
        return Err(Error::msg(format!(
            "Slot {slot} is outside 1 to {}. Nothing was written.",
            crate::addons::MAX_SLOT
        )));
    }

    let prepared = prepare_archive(request, options)?;
    let cancel = options.cancel.as_deref();
    check_cancel(cancel)?;
    let game_root = resolve_game_root(options.game_root.clone())?;
    let addons = addons_dir(&game_root);
    let gameinfo = gameinfo_path(&game_root);
    let Prepared {
        cache,
        download: fetched,
        extracted,
        extract,
        gamebanana,
        mut warnings,
        ..
    } = prepared;

    // A stop before validation: nothing has been written beyond the cache at this point.
    check_cancel(cancel)?;

    let candidate = classify_extracted(&extracted, &options.limits, request.vpk.as_deref())?;
    warnings.extend(extract.warnings);
    warnings.extend(candidate.warnings.clone());

    if candidate.payload.is_empty() {
        let files = walk_tree(&extracted)?;
        let mut message = shapes::nothing_installable(&files);
        // A GameBanana config Sigil already lists on the Performance page: name it there.
        if let Some(id) = gamebanana_mod_id(&request.source)
            && let Some(config) = crate::performance::catalog::CONFIGS
                .iter()
                .find(|config| config.gamebanana.as_ref().is_some_and(|hosted| hosted.mod_id == id))
        {
            message.push_str(&format!(
                " Sigil lists this mod on the Performance page as \"{}\": apply it from there.",
                config.name
            ));
        }
        return Err(Error::msg(message));
    }

    // The rules run over what ships: a packed tree's payload, or every entry of a VPK. A source
    // file inside a creator's VPK is harmless (the game ignores it), as both Grimoire and Deadlock
    // Mod Manager treat it, so it is a warning there rather than a refusal.
    let is_vpk = matches!(candidate.artifact, CandidateArtifact::Vpk { .. });
    let mut issues = validate_payload(&candidate.payload, &PayloadOptions::default());
    if is_vpk {
        for issue in &mut issues {
            if matches!(issue.code, "source_file" | "archive" | "duplicate_path" | "tool_cache") {
                issue.severity = Severity::Warning;
            }
        }
    }

    if has_errors(&issues) {
        let errors: Vec<PayloadIssue> = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .cloned()
            .collect();

        let script = errors.iter().any(|issue| matches!(issue.code, "skeleton" | "gameplay_data"));
        let hint = if script {
            "\nThis is a script mod. To install it anyway, turn on Settings > Preferences > Allow script mods."
        } else {
            ""
        };
        return Err(Error::msg(format!(
            "Payload rejected: the archive ships files a Deadlock mod may not contain. Nothing was installed.{hint}\n{}\nPaths: {}",
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


    // Staging copies the payload into the cache, so a stop checked here saves that work.
    check_cancel(cancel)?;

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

    let target = crate::addons::slot_folder(&addons, slot).join(format_slot_file_name(slot)?);

    let conflicts = scan_conflicts(&addons, slot, &candidate.payload, &mut warnings)?;
    let gameinfo_status = read_game_info_status(&gameinfo)?;
    // Also when a required line (Mod and Write citadel) is missing: see
    // `deploy::ensure_launch_search_paths`.
    let gameinfo_repair = !(gameinfo_status.present
        && gameinfo_status.addons_search_path
        && gameinfo_status.missing_paths.is_empty());

    if !gameinfo_status.present {
        warnings.push(format!(
            "{} is missing. The addons SearchPath cannot be repaired until Steam restores it.",
            gameinfo.display()
        ));
    }

    let mut files = candidate.payload.clone();
    files.sort();

    // A stopped run stops before the plan file, so the app never offers to apply a plan the
    // player cancelled.
    check_cancel(cancel)?;

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
        shapes::remove_tree(staging)?;
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
        shapes::make_writable(&destination);
    }

    Ok(())
}

/// Choose a slot, refuse an occupied one without force, and list what a write
/// would replace.
fn choose_slot(
    addons: &Path,
    requested: Option<u16>,
    force: bool,
    payload_entries: usize,
) -> Result<(u16, Vec<PathBuf>, bool, Vec<String>)> {
    let status = read_addons_status(addons)?;
    // A disabled slot or a slot with only chunk files left is still taken: the
    // lowest free slot must hold none of its names.
    let used = occupied_slots(addons)?;

    let slot = match requested {
        Some(value) => value,
        // Past 99 the mod goes to the next extra folder (`sigil-addons2` and on), which is
        // mounted in gameinfo.gi after the write.
        None => (1..=crate::addons::MAX_SLOT)
            .find(|slot| !used.contains(slot))
            .ok_or_else(|| {
                Error::msg(format!(
                    "Every addon slot (1 to {}) is taken. Remove an addon before installing another.",
                    crate::addons::MAX_SLOT
                ))
            })?,
    };

    let mut replaced = Vec::new();
    let mut warnings = Vec::new();

    let dir_name = format_slot_file_name(slot)?;
    let folder = crate::addons::slot_folder(addons, slot);
    let target = folder.join(&dir_name);
    let disabled = folder.join(format!("{dir_name}.disabled"));

    if target.is_file() {
        replaced.push(target.clone());
    }
    if disabled.is_file() {
        replaced.push(disabled.clone());
        warnings.push(format!(
            "Slot {slot:02} holds a disabled addon ({dir_name}.disabled). Installing over it replaces that file after a backup."
        ));
    }

    if crate::addons::folder_index(slot) == 0 {
        for chunk in &status.chunk_files {
            if is_slot_chunk_name(chunk, slot) {
                replaced.push(addons.join(chunk));
            }
        }
    } else if let Some(names) = crate::load_order::read_slot_files(addons)?.get(&slot) {
        for name in names {
            if is_slot_chunk_name(name, slot) {
                replaced.push(folder.join(name));
            }
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
    slot: u16,
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
    pub slot: u16,
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

/// How an approved plan lands in the addon folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Write the directory file as `pakNN_dir.vpk.disabled`, the same name the
    /// enable switch renames from. The game ignores it until the user turns the
    /// slot on. Chunk files keep their `pakNN_MMM.vpk` names either way.
    pub land_disabled: bool,
}

/// Copy an approved plan into `game/citadel/addons`, enabled. The CLI path.
///
/// Refuses without `approve`, when the plan is stale, when the recorded game
/// root disappeared, or when the artifact no longer matches the plan's file
/// list. Existing files are backed up under
/// `<workspace>/.dah/install/backups/<stamp>/` before anything is replaced.
pub fn apply_install(plan: &InstallPlan, approve: bool) -> Result<InstallRecord> {
    apply_install_with(plan, approve, ApplyOptions::default())
}

/// [`apply_install`] with a choice of landing name.
///
/// Game install write. With `land_disabled` the directory VPK is written as
/// `pakNN_dir.vpk.disabled` and an enabled `pakNN_dir.vpk` already in the slot
/// is backed up and removed, so the slot ends with exactly one name. Sigil's
/// Browse install uses this so a new mod stays off until the user enables it.
pub fn apply_install_with(
    plan: &InstallPlan,
    approve: bool,
    options: ApplyOptions,
) -> Result<InstallRecord> {
    if !approve {
        return Err(Error::msg(format!(
            "Install of slot {:02} into {} was not approved. Nothing was written. Re-run with --approve.",
            plan.slot,
            plan.target.display()
        )));
    }

    let slot_folder = crate::addons::slot_folder(&plan.addons_dir, plan.slot);
    if plan.target != slot_folder.join(format_slot_file_name(plan.slot)?) {
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
        || slot_folder.join(format!("{slot_file}.disabled")).is_file()
        || plan.replaced.iter().any(|path| path.is_file())
        || slot_chunks_present(&slot_folder, plan.slot)?;

    if plan.replace_required != affected_exists {
        return Err(Error::msg(
            "Install plan is stale: the addons folder changed since planning. Plan the install again.",
        ));
    }

    // Repair a stale gameinfo before the slot is touched. A failed repair then
    // aborts with nothing written, instead of leaving a VPK the game ignores.
    if plan.gameinfo_repair {
        let status = read_game_info_status(&plan.gameinfo_path)?;
        if status.present {
            repair_gameinfo(&plan.gameinfo_path, true)?;
        }
    }

    fs::create_dir_all(&plan.addons_dir)?;

    let destination = if options.land_disabled {
        plan.addons_dir
            .join(format!("{slot_file}{}", crate::addons::DISABLED_SUFFIX))
    } else {
        plan.target.clone()
    };

    let backups = backup_existing(plan)?;
    let removed_files = remove_other_slot_name(plan, &destination)?;

    let (written, chunk_files) = match &plan.artifact {
        InstallArtifact::Vpk {
            dir_path,
            chunk_paths,
        } => deploy_archive_vpk(plan, &destination, dir_path, chunk_paths)?,
        InstallArtifact::Staging { path } => deploy_staged(plan, &destination, path)?,
    };

    let gameinfo_repaired = plan.gameinfo_repair
        && read_game_info_status(&plan.gameinfo_path)?.addons_search_path;

    let removed_chunks = remove_leftover_chunks(plan, &chunk_files)?;

    if !written.is_file() {
        return Err(Error::msg(format!(
            "Install wrote nothing to {}.",
            destination.display()
        )));
    }

    let written_name = written
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| slot_file.clone());

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
        file: written_name,
        target: written.clone(),
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

    // A slot past 99 lives in an extra folder that gameinfo.gi has to mount. The files are
    // already in place, so a failed mount leaves an unmounted mod, never a broken game; it
    // becomes a warning the app shows.
    let mut entry = entry;
    if crate::addons::folder_index(plan.slot) > 0
        && let Err(error) = crate::addons::sync_overflow_mount(&plan.addons_dir)
    {
        entry.warnings.push(format!(
            "Slot {} is in an extra addon folder, and adding it to gameinfo.gi failed: {error}",
            crate::addons::slot_label(plan.slot)
        ));
    }

    Ok(InstallRecord {
        entry,
        target: written,
        backups,
        gameinfo_repaired,
    })
}

/// Remove the slot's other directory name once it has been backed up.
///
/// Leaving both `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled` behind would make
/// the slot ambiguous for `dah addons enable`, so the name the install does not
/// write goes away after the backup. `keep` is the name being written.
fn remove_other_slot_name(plan: &InstallPlan, keep: &Path) -> Result<Vec<String>> {
    let enabled = format_slot_file_name(plan.slot)?;
    let disabled = format!("{enabled}{}", crate::addons::DISABLED_SUFFIX);
    let other = if keep.file_name().map(|name| name.to_string_lossy().into_owned())
        == Some(disabled.clone())
    {
        enabled
    } else {
        disabled
    };
    let path = crate::addons::slot_folder(&plan.addons_dir, plan.slot).join(&other);

    if !path.is_file() {
        return Ok(Vec::new());
    }

    if !plan.replaced.iter().any(|replaced| replaced == &path) {
        return Err(Error::msg(format!(
            "{} appeared after planning and has no backup. Plan the install again.",
            path.display()
        )));
    }

    fs::remove_file(&path)?;
    Ok(vec![other])
}

/// Slots that hold any name in `addons` or its extra folders: `pakNN_dir.vpk`,
/// `pakNN_dir.vpk.disabled` or a `pakNN_MMM.vpk` chunk. Read only.
pub fn occupied_slots(addons: &Path) -> Result<BTreeSet<u16>> {
    // Every slot file in the main folder and the extra ones, by global slot.
    Ok(crate::load_order::read_slot_files(addons)?.into_keys().collect())
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
    destination: &Path,
    dir_path: &Path,
    chunk_paths: &[PathBuf],
) -> Result<(PathBuf, Vec<String>)> {

    let mut chunk_files = Vec::new();

    for chunk in chunk_paths {
        let name = chunk
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| {
                Error::msg(format!("Chunk {} has no file name.", chunk.display()))
            })?;
        let renamed = rename_chunk(&name, plan.slot)?;
        copy_into_place(chunk, &crate::addons::slot_folder(&plan.addons_dir, plan.slot).join(&renamed))?;
        chunk_files.push(renamed);
    }

    // The directory file goes last, so the game never sees a slot whose chunks
    // are still being copied.
    copy_into_place(dir_path, destination)?;

    chunk_files.sort();
    Ok((destination.to_path_buf(), chunk_files))
}

fn deploy_staged(
    plan: &InstallPlan,
    destination: &Path,
    staging: &Path,
) -> Result<(PathBuf, Vec<String>)> {
    let build_dir = install_state_dir(&plan.workspace).join("build").join(&plan.id);
    let packed = pack_vpk(&PackOptions::new(
        staging.to_path_buf(),
        build_dir,
        format!("pak{:02}", crate::addons::local_slot(plan.slot)),
    ))?;

    let mut chunk_files = Vec::new();

    for chunk in &packed.chunk_paths {
        let name = chunk
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| Error::msg(format!("Chunk {} has no file name.", chunk.display())))?;
        copy_into_place(chunk, &crate::addons::slot_folder(&plan.addons_dir, plan.slot).join(&name))?;
        chunk_files.push(name);
    }

    copy_into_place(&packed.dir_path, destination)?;

    chunk_files.sort();
    Ok((destination.to_path_buf(), chunk_files))
}

/// Remove chunk files of the slot that this install does not ship.
///
/// A previous, larger mod at the same slot can leave `pakNN_001.vpk` behind.
/// The game would still load it, so it is backed up first and then removed.
fn remove_leftover_chunks(plan: &InstallPlan, written: &[String]) -> Result<Vec<String>> {
    let mut removed = Vec::new();

    // The slot's own folder: an overflow slot's `pakNN` name repeats in `citadel/addons`.
    let entries = match fs::read_dir(crate::addons::slot_folder(&plan.addons_dir, plan.slot)) {
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
fn is_slot_chunk_name(name: &str, slot: u16) -> bool {    if !name.is_ascii() {
        return false;
    }

    let lowered = name.to_ascii_lowercase();
    let bytes = lowered.as_bytes();

    bytes.len() == "pak00_000.vpk".len()
        && lowered.starts_with("pak")
        && &lowered[3..5] == format!("{:02}", crate::addons::local_slot(slot)).as_str()
        && bytes[5] == b'_'
        && bytes[6..9].iter().all(|byte| byte.is_ascii_digit())
        && lowered.ends_with(".vpk")
}

/// True when any chunk file of `slot` sits in the addons folder.
fn slot_chunks_present(addons: &Path, slot: u16) -> Result<bool> {
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

fn rename_chunk(name: &str, slot: u16) -> Result<String> {
    let lowered = name.to_lowercase();
    let digits = vpk_chunk_base(&lowered)
        .map(|base| &lowered[base.len() + 1..base.len() + 4])
        .ok_or_else(|| Error::msg(format!("Chunk {name:?} is not named <name>_NNN.vpk.")))?;
    Ok(format!("pak{:02}_{digits}.vpk", crate::addons::local_slot(slot)))
}

fn copy_into_place(source: &Path, destination: &Path) -> Result<()> {
    let name = destination
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| {
            Error::msg(format!("Cannot copy to {}: it has no file name.", destination.display()))
        })?;
    let temporary = destination.with_file_name(format!("{name}.dah-tmp"));
    // The first mod past slot 99 creates its extra folder.
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }

    // A read-only flag from the archive must not follow the file into the game folder, where
    // it would block the next update, disable or uninstall.
    shapes::make_writable(&temporary);
    if destination.exists() {
        shapes::make_writable(destination);
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
    pub(super) fn scratch(name: &str) -> PathBuf {
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

    pub(super) fn fake_game(root: &Path) -> PathBuf {
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
    pub(super) fn build_stored_zip(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
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

    /// Serve one body in slow chunks and set `cancel` after `after_chunks` chunks, while the
    /// client is still reading. The chunks keep flowing afterwards so a read that is waiting
    /// wakes up and sees the flag, the way a player pressing stop mid-stream does.
    fn serve_with_cancel(
        body: Vec<u8>,
        chunk: usize,
        after_chunks: usize,
        cancel: Arc<AtomicBool>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .ok();

                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);

                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/zip\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(head.as_bytes()).is_err() {
                    continue;
                }

                for (index, piece) in body.chunks(chunk).enumerate() {
                    if index == after_chunks {
                        cancel.store(true, Ordering::SeqCst);
                    }
                    if stream.write_all(piece).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_millis(2));
                }
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
            cancel: None,
        }
    }

    /// Build a real pakNN_dir.vpk plus chunk through the crate writer.
    pub(super) fn make_vpk(directory: &Path, base: &str, game_paths: &[(&str, &[u8])]) -> (PathBuf, Vec<PathBuf>) {
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
            ("backslash", "materials\\..\\..\\evil.vmat_c", "escapes the extract folder"),
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
    fn broken_7z_and_rar_fail_cleanly_without_a_tool() {
        let root = scratch("backends");
        let limits = ExtractLimits::default();
        let tools = ToolPaths::default();

        let seven = root.join("mod.7z");
        fs::write(&seven, b"7z\xbc\xaf\x27\x1c\x00\x04rest").unwrap();
        let text = extract_archive(&seven, &root.join("out-7z"), &limits, &tools)
            .unwrap_err()
            .to_string();
        assert!(text.contains("mod.7z"), "{text}");
        assert!(!text.contains("PATH"), "the built-in reader needs no tool: {text}");
        assert!(!root.join("out-7z").exists());

        let rar = root.join("mod.rar");
        fs::write(&rar, b"Rar!\x1a\x07\x00rest").unwrap();
        let text = extract_archive(&rar, &root.join("out-rar"), &limits, &tools)
            .unwrap_err()
            .to_string();
        assert!(text.contains("mod.rar"), "{text}");
        assert!(!text.contains("PATH"), "{text}");
        assert!(!root.join("out-rar").exists());

        assert_eq!(detect_archive_format(&seven), ArchiveFormat::SevenZip);
        assert_eq!(detect_archive_format(&rar), ArchiveFormat::Rar);

        let _ = fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Classification
    // ------------------------------------------------------------------

    #[test]
    fn detects_the_game_path_prefix() {
        let files = |paths: &[&str]| paths.iter().map(|p| (p.to_string(), 1u64)).collect::<Vec<_>>();
        let prefixes = |paths: &[&str]| {
            loose_groups(&files(paths)).into_iter().map(|(prefix, _)| prefix).collect::<Vec<_>>()
        };
        assert_eq!(
            prefixes(&[
                "game/citadel/materials/heroes/billy/billy_color.vmat_c",
                "game/citadel/models/heroes_wip/billy/billy.vmdl_c",
                "readme.md",
            ]),
            vec!["game/citadel/".to_string()]
        );
        assert_eq!(prefixes(&["materials/heroes/billy/billy_color.vmat_c"]), vec![String::new()]);
        assert_eq!(prefixes(&["citadel/materials/a.vmat_c"]), vec!["citadel/".to_string()]);
        // Any packaging above the game folders: a folder named after the mod, addons/.
        assert_eq!(prefixes(&["Cool Mod v2/materials/a.vmat_c", "Cool Mod v2/readme.txt"]), vec!["Cool Mod v2/".to_string()]);
        assert_eq!(prefixes(&["addons/sounds/a.vsnd_c"]), vec!["addons/".to_string()]);
        // A second engine tree is not a variant.
        assert_eq!(
            prefixes(&["game/citadel/materials/a.vmat_c", "game/core/materials/b.vmat_c"]),
            vec!["game/citadel/".to_string()]
        );
        // Variant folders are separate groups for the picker.
        assert_eq!(prefixes(&["Red/materials/a.vmat_c", "Blue/materials/a.vmat_c"]).len(), 2);
        // Source files and notes are no group at all.
        assert!(prefixes(&["docs/readme.md", "materials/a.vmat", "materials/a.png"]).is_empty());
        assert!(prefixes(&[]).is_empty());
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
        // macOS junk is dropped at extract time, before anything reads the tree.
        assert!(!plan.extracted.join("__MACOSX").exists());
        assert!(!plan.warnings.iter().any(|warning| warning.contains("__MACOSX")), "{:?}", plan.warnings);
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
    fn land_disabled_writes_the_disabled_name_and_skips_taken_slots() {
        let root = scratch("land-disabled");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        let addons = addons_dir(&game_root);

        // Slot 01 is off, slot 02 only has a stray chunk: neither is free.
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk.disabled"), b"user's disabled mod").unwrap();
        fs::write(addons.join("pak02_000.vpk"), b"stray chunk").unwrap();
        assert_eq!(
            occupied_slots(&addons).unwrap().into_iter().collect::<Vec<_>>(),
            vec![1, 2]
        );

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
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone()).unwrap();
        let plan = plan_install(&request, &options).unwrap();
        assert_eq!(plan.slot, 3);
        assert!(!plan.replace_required);

        let record = apply_install_with(&plan, true, ApplyOptions { land_disabled: true }).unwrap();
        let disabled = addons.join("pak03_dir.vpk.disabled");
        assert!(disabled.is_file());
        assert!(!addons.join("pak03_dir.vpk").exists());
        assert_eq!(record.target, disabled);
        assert_eq!(record.entry.file, "pak03_dir.vpk.disabled");
        // The user's own disabled slot is untouched.
        assert_eq!(fs::read(addons.join("pak01_dir.vpk.disabled")).unwrap(), b"user's disabled mod");

        // The enable switch turns it on with the ordinary rename.
        crate::addons::set_enabled(&game_root, 3, true, true).unwrap();
        assert!(addons.join("pak03_dir.vpk").is_file());
        assert!(!disabled.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn land_disabled_over_an_enabled_slot_backs_it_up_and_leaves_one_name() {
        let root = scratch("land-disabled-force");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        let addons = addons_dir(&game_root);

        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak01_dir.vpk"), b"old enabled").unwrap();

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
            .with_slot(Some(1))
            .with_force(true);
        let plan = plan_install(&request, &options).unwrap();

        let record = apply_install_with(&plan, true, ApplyOptions { land_disabled: true }).unwrap();
        assert!(addons.join("pak01_dir.vpk.disabled").is_file());
        assert!(!addons.join("pak01_dir.vpk").exists());
        assert_eq!(record.entry.removed_files, vec!["pak01_dir.vpk".to_string()]);
        assert!(
            record
                .backups
                .iter()
                .any(|path| path.to_string_lossy().ends_with("pak01_dir.vpk"))
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
        ];
        // Source files and programs next to compiled files are left out with a warning, not a
        // refusal (install_shape_tests.rs covers them).

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
    fn a_vpk_under_any_name_is_a_vpk() {
        // GameBanana mods ship `Josuke Remaster.vpk` or `skin_dir.vpk` as often as pakNN_dir.vpk.
        let root = scratch("vpk-names");
        let files: Vec<(String, u64)> = [
            "Josuke Remaster.vpk",
            "skin_dir.vpk",
            "skin_000.vpk",
            "notes.vpk",
            "readme.txt",
        ]
        .iter()
        .map(|name| (name.to_string(), 1))
        .collect();
        fs::write(root.join("Josuke Remaster.vpk"), [0x34, 0x12, 0xaa, 0x55, 2, 0, 0, 0]).unwrap();
        fs::write(root.join("notes.vpk"), b"not a vpk").unwrap();
        let found: Vec<&str> = vpk_dir_files(&root, &files).iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(found, vec!["Josuke Remaster.vpk", "skin_dir.vpk"]);
        assert_eq!(vpk_chunk_prefix("a/Josuke Remaster.vpk"), "josuke remaster");
        assert_eq!(vpk_chunk_prefix("skin_dir.vpk"), "skin");
        assert!(is_chunk_for("x/skin_000.vpk", "skin") && !is_chunk_for("skin_dir.vpk", "skin"));
        assert_eq!(rename_chunk("Skin_002.vpk", 7).unwrap(), "pak07_002.vpk");
        assert_eq!(rename_chunk("pak03_011.vpk", 7).unwrap(), "pak07_011.vpk");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installs_an_archive_whose_vpk_has_its_own_name() {
        let root = scratch("vpk-named");
        let game_root = fake_game(&root);
        let source_dir = root.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let (dir_vpk, chunks) = make_vpk(
            &source_dir,
            "pak02",
            &[("materials/heroes/billy/billy_color.vmat_c", b"packed payload")],
        );
        let mut entries: Vec<(String, Vec<u8>, u32)> =
            vec![("Josuke Remaster_dir.vpk".to_string(), fs::read(&dir_vpk).unwrap(), 0o100644)];
        for chunk in &chunks {
            let name = chunk.file_name().unwrap().to_string_lossy().replace("pak02", "Josuke Remaster");
            entries.push((name, fs::read(chunk).unwrap(), 0o100644));
        }
        let refs: Vec<(&str, &[u8], u32)> =
            entries.iter().map(|(name, data, mode)| (name.as_str(), data.as_slice(), *mode)).collect();
        let archive = root.join("josuke.zip");
        fs::write(&archive, build_stored_zip(&refs)).unwrap();

        let options = PlanOptions { game_root: Some(game_root.clone()), tools: ToolPaths::default(), ..PlanOptions::default() };
        let request = InstallRequest::new(archive.display().to_string(), root.join("workspace")).unwrap();
        let plan = plan_install(&request, &options).unwrap();
        assert!(matches!(plan.artifact, InstallArtifact::Vpk { .. }));
        assert_eq!(plan.files, vec!["materials/heroes/billy/billy_color.vmat_c".to_string()]);
        let record = apply_install(&plan, true).unwrap();
        assert_eq!(record.entry.chunk_files, vec!["pak01_000.vpk".to_string()]);
        assert!(addons_dir(&game_root).join("pak01_000.vpk").is_file());
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

    #[test]
    fn one_click_markers_match_by_file_name() {
        assert_eq!(one_click_marker("game/citadel/.disable_gb1click"), Some(".disable_gb1click"));
        assert_eq!(one_click_marker(".disable_gb1click_sigil"), Some(".disable_gb1click_sigil"));
        assert_eq!(one_click_marker("Game/Citadel/.DISABLE_GB1CLICK"), Some(".disable_gb1click"));
        assert_eq!(one_click_marker("readme.txt"), None);
    }

    #[test]
    fn an_archive_can_refuse_one_click_installs() {
        let root = scratch("one-click");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let entries: [(&str, &[u8], u32); 1] =
            [(".disable_gb1click", b"no one-click installs", 0o644)];
        let archive = root.join("marked.zip");
        fs::write(&archive, build_stored_zip(&entries)).unwrap();

        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            tools: ToolPaths::default(),
            ..PlanOptions::default()
        };
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone())
            .unwrap()
            .with_one_click(true);
        let error = plan_install(&request, &options).unwrap_err().to_string();
        assert!(error.contains("does not allow one-click installs"), "{error}");

        // The marker only stops the one-click path; the normal path reports the empty payload.
        let request = InstallRequest::new(archive.display().to_string(), workspace).unwrap();
        let error = plan_install(&request, &options).unwrap_err().to_string();
        assert!(!error.contains("one-click"), "{error}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_multi_vpk_archive_is_listed_and_the_chosen_one_plans() {
        let root = scratch("multi-vpk");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (dir_a, chunks_a) = make_vpk(
            &root.join("build-a"),
            "pak01",
            &[("materials/heroes/billy/billy_color.vmat_c", b"one")],
        );
        let (dir_b, chunks_b) = make_vpk(
            &root.join("build-b"),
            "pak02",
            &[("materials/heroes/haze/haze_color.vmat_c", b"two")],
        );
        let _ = fs::create_dir_all(root.join("build-a"));
        let _ = fs::create_dir_all(root.join("build-b"));
        let mut files: Vec<(String, Vec<u8>, u32)> = Vec::new();
        for (path, mode) in [
            (dir_a, 0o100644),
            (dir_b, 0o100644),
        ] {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            files.push((name, fs::read(path).unwrap(), mode));
        }
        for path in chunks_a.iter().chain(chunks_b.iter()) {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            files.push((name, fs::read(path).unwrap(), 0o100644));
        }
        let refs: Vec<(&str, &[u8], u32)> = files
            .iter()
            .map(|(name, bytes, mode)| (name.as_str(), bytes.as_slice(), *mode))
            .collect();
        let archive = root.join("both.zip");
        fs::write(&archive, build_stored_zip(&refs)).unwrap();

        let options = PlanOptions {
            game_root: Some(game_root),
            tools: ToolPaths::default(),
            ..PlanOptions::default()
        };
        let request = InstallRequest::new(archive.display().to_string(), workspace.clone())
            .unwrap()
            .with_one_click(false);
        let error = plan_install(&request, &options).unwrap_err().to_string();
        assert!(error.contains("more than one VPK"), "{error}");

        let choices = vpk_choices(&request, &options).unwrap();
        assert_eq!(choices.len(), 2, "{choices:?}");
        let chosen = choices
            .iter()
            .find(|choice| choice.name() == "pak02_dir.vpk")
            .expect("the second VPK is listed");
        let request = request.with_vpk(Some(chosen.path.clone()));
        let plan = plan_install(&request, &options).unwrap();
        assert_eq!(
            plan.files,
            vec!["materials/heroes/haze/haze_color.vmat_c".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn every_vpk_of_an_archive_installs_into_its_own_slot_in_order() {
        let root = scratch("multi-install");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (dir_a, chunks_a) = make_vpk(
            &root.join("build-a"),
            "pak01",
            &[("materials/heroes/billy/billy_color.vmat_c", b"one")],
        );
        let (dir_b, chunks_b) = make_vpk(
            &root.join("build-b"),
            "pak02",
            &[("materials/heroes/haze/haze_color.vmat_c", b"two")],
        );
        let mut files: Vec<(String, Vec<u8>, u32)> = Vec::new();
        for path in [&dir_a, &dir_b].into_iter().chain(chunks_a.iter()).chain(chunks_b.iter()) {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            files.push((name, fs::read(path).unwrap(), 0o100644));
        }
        let refs: Vec<(&str, &[u8], u32)> =
            files.iter().map(|(name, bytes, mode)| (name.as_str(), bytes.as_slice(), *mode)).collect();
        let archive = root.join("both.zip");
        fs::write(&archive, build_stored_zip(&refs)).unwrap();
        let options = PlanOptions { game_root: Some(game_root.clone()), tools: ToolPaths::default(), ..PlanOptions::default() };
        let request = InstallRequest::new(archive.display().to_string(), workspace).unwrap().with_one_click(false);
        let mut picks: Vec<String> = vpk_choices(&request, &options).unwrap().into_iter().map(|c| c.path).collect();
        picks.sort();

        let done = install_vpks(&request, &picks, &options, ApplyOptions::default(), true);
        assert!(done.error.is_none(), "{:?}", done.error);
        assert_eq!(done.installed.len(), 2);
        let slots: Vec<u16> = done.installed.iter().map(|(plan, _)| plan.slot).collect();
        assert!(slots[0] < slots[1], "the first file takes the lower slot: {slots:?}");
        for (plan, _) in &done.installed {
            assert!(plan.target.is_file(), "{} was written", plan.target.display());
        }

        // Without approval nothing more is written and the error says so.
        let refused = install_vpks(&request, &picks, &options, ApplyOptions::default(), false);
        assert!(refused.installed.is_empty() && refused.error.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn vpk_choices_count_chunks_and_bytes() {
        let files = vec![
            ("game/citadel/pak01_dir.vpk".to_string(), 100),
            ("game/citadel/pak01_000.vpk".to_string(), 40),
            ("game/citadel/pak01_001.vpk".to_string(), 60),
            ("game/citadel/pak02_dir.vpk".to_string(), 200),
            ("game/citadel/readme.txt".to_string(), 5),
        ];
        let choices = collect_vpk_choices(Path::new("/nonexistent"), &files);
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].name(), "pak01_dir.vpk");
        assert_eq!(choices[0].bytes, 200);
        assert_eq!(choices[0].chunks, 2);
        assert_eq!(choices[1].bytes, 200);
        assert_eq!(choices[1].chunks, 0);
    }

    // ------------------------------------------------------------------
    // Cancellation
    // ------------------------------------------------------------------

    #[test]
    fn the_cancel_error_is_recognised_and_reads_plainly() {
        assert!(is_cancelled(&Error::Cancelled));
        assert_eq!(
            Error::Cancelled.to_string(),
            "Cancelled. Nothing was installed."
        );
        assert!(!is_cancelled(&Error::msg("boom")));
        assert!(!is_cancelled(&Error::Network("offline".to_string())));
    }

    #[test]
    fn a_cancel_flag_stops_a_download_and_leaves_no_partial_file() {
        let root = scratch("cancel-download");
        let cache = root.join("cache");
        let body: Vec<u8> = (0..600_000).map(|index| (index % 251) as u8).collect();
        let flag = Arc::new(AtomicBool::new(false));
        let origin = serve_with_cancel(body, 16 * 1024, 2, Arc::clone(&flag));
        let policy = DownloadPolicy::gamebanana().with_mirror(&origin).unwrap();

        let error = download_with_cancel(
            &format!("{origin}/mod.zip"),
            &cache,
            &policy,
            Some(flag.as_ref()),
        )
        .unwrap_err();

        assert!(is_cancelled(&error), "{error}");
        assert!(flag.load(Ordering::SeqCst), "the server set the flag mid-stream");
        assert!(!cache.join("mod.zip").exists(), "a cancelled download leaves no archive");
        assert!(!cache.join("mod.zip.part").exists(), "a cancelled download leaves no part file");
        let left: Vec<PathBuf> = fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert!(left.is_empty(), "the cache holds no entry a later run could reuse: {left:?}");

        // A flag that is already set stops before the socket opens and creates nothing.
        let early = root.join("early");
        let already = Arc::new(AtomicBool::new(true));
        let error = download_with_cancel(
            &format!("{origin}/mod.zip"),
            &early,
            &policy,
            Some(already.as_ref()),
        )
        .unwrap_err();
        assert!(is_cancelled(&error), "{error}");
        assert!(!early.exists(), "a cancel before the request creates no folder");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cancel_flag_stops_an_extract_and_removes_the_folder() {
        let root = scratch("cancel-extract");
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for index in 0..64 {
            files.push((
                format!("citadel/materials/heroes/billy/file-{index:02}.vmat_c"),
                format!("payload {index}").into_bytes(),
            ));
        }
        let refs: Vec<(&str, &[u8], u32)> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice(), 0o100644))
            .collect();
        let archive = root.join("many.zip");
        fs::write(&archive, build_stored_zip(&refs)).unwrap();

        let flag = Arc::new(AtomicBool::new(true));
        let destination = root.join("out");
        let error = extract_archive_with_cancel(
            &archive,
            &destination,
            &ExtractLimits::default(),
            &ToolPaths::default(),
            Some(flag.as_ref()),
        )
        .unwrap_err();

        assert!(is_cancelled(&error), "{error}");
        assert!(!destination.exists(), "a cancelled extract removes its folder");
        assert_eq!(files.len(), 64, "the fixture keeps the archive realistic");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cancel_flag_between_entries_leaves_no_half_extracted_tree() {
        let root = scratch("cancel-mid-extract");
        let payload = b"vmat payload".to_vec();
        let names: Vec<String> = (0..4_000)
            .map(|index| format!("citadel/files/entry-{index:04}.vmat_c"))
            .collect();
        let refs: Vec<(&str, &[u8], u32)> = names
            .iter()
            .map(|name| (name.as_str(), payload.as_slice(), 0o100644))
            .collect();
        let archive = root.join("many.zip");
        fs::write(&archive, build_stored_zip(&refs)).unwrap();

        let destination = root.join("out");
        let first = destination.join("citadel/files/entry-0000.vmat_c");
        let flag = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicBool::new(false));
        let watcher = {
            let flag = Arc::clone(&flag);
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                // The first entry on disk proves the loop is running, so the flag lands
                // between entries rather than before the first one.
                while !first.exists() {
                    std::hint::spin_loop();
                }
                seen.store(true, Ordering::SeqCst);
                flag.store(true, Ordering::SeqCst);
            })
        };

        let result = extract_archive_with_cancel(
            &archive,
            &destination,
            &ExtractLimits::default(),
            &ToolPaths::default(),
            Some(flag.as_ref()),
        );
        watcher.join().unwrap();

        assert!(seen.load(Ordering::SeqCst), "the extract started before the flag was set");
        let error = result.unwrap_err();
        assert!(is_cancelled(&error), "{error}");
        assert!(!destination.exists(), "a cancelled extract removes its half tree");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cancelled_plan_writes_no_plan_and_no_game_file() {
        let root = scratch("cancel-plan");
        let game_root = fake_game(&root);
        let workspace = root.join("workspace");

        // Big enough to stream in several chunks, so the flag lands mid-download.
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for index in 0..40 {
            files.push((
                format!("citadel/materials/heroes/billy/file-{index:02}.vmat_c"),
                vec![b'x'; 4 * 1024],
            ));
        }
        let refs: Vec<(&str, &[u8], u32)> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice(), 0o100644))
            .collect();
        let zip = build_stored_zip(&refs);
        assert!(zip.len() > 3 * 8192, "the fixture needs more than three chunks");

        let flag = Arc::new(AtomicBool::new(false));
        let origin = serve_with_cancel(zip, 8192, 2, Arc::clone(&flag));
        let policy = DownloadPolicy::gamebanana().with_mirror(&origin).unwrap();
        let request = InstallRequest::new(format!("{origin}/mod.zip"), workspace.clone()).unwrap();
        let options = PlanOptions {
            game_root: Some(game_root.clone()),
            policy,
            limits: ExtractLimits::default(),
            tools: ToolPaths::default(),
            cancel: Some(Arc::clone(&flag)),
        };

        let error = plan_install(&request, &options).unwrap_err();
        assert!(is_cancelled(&error), "{error}");
        assert!(flag.load(Ordering::SeqCst), "the server set the flag mid-stream");

        // Nothing reached the game and no plan was stored for the app to offer.
        assert!(!addons_dir(&game_root).join("pak01_dir.vpk").exists());
        let present: Vec<PathBuf> = fs::read_dir(addons_dir(&game_root))
            .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
            .unwrap_or_default();
        assert!(present.is_empty(), "the addons folder stays empty: {present:?}");
        assert!(!plans_dir(&workspace).exists(), "a cancelled plan is never stored");
        let cache = download_dir(&workspace, &request.source);
        let left: Vec<PathBuf> = fs::read_dir(&cache)
            .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
            .unwrap_or_default();
        assert!(
            left.is_empty(),
            "no archive, part file or extracted folder survives: {left:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archives").join(name)
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("sigil-archive-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// No tool on PATH, so only the built-in readers run.
    fn no_tools() -> ToolPaths {
        ToolPaths { seven_zip: None, unrar: None }
    }

    fn extracts(name: &str) -> ExtractReport {
        let out = scratch(name);
        let report = extract_archive(&fixture(name), &out, &ExtractLimits::default(), &no_tools())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(!report.files.is_empty(), "{name} extracted nothing");
        for file in &report.files {
            let path = out.join(file);
            assert!(path.is_file(), "{name}: {file} missing");
            assert!(path.starts_with(&out));
        }
        let _ = fs::remove_dir_all(&out);
        report
    }

    #[test]
    fn seven_zip_and_rar_open_without_any_tool() {
        assert_eq!(extracts("two_files_with_content_lzma.7z").format, ArchiveFormat::SevenZip);
        assert_eq!(extracts("solid.7z").format, ArchiveFormat::SevenZip);
        assert_eq!(extracts("solid.rar").format, ArchiveFormat::Rar);
        assert_eq!(extracts("unicode.rar").format, ArchiveFormat::Rar);
    }

    #[test]
    fn a_password_protected_7z_says_so() {
        let out = scratch("encrypted");
        let error = extract_archive(&fixture("encrypted.7z"), &out, &ExtractLimits::default(), &no_tools())
            .unwrap_err()
            .to_string();
        assert!(error.contains("password") || error.contains("Password"), "{error}");
        assert!(!out.exists(), "a failed extract removes its folder");
    }

    #[test]
    fn the_byte_cap_holds_for_7z_and_rar() {
        let tight = ExtractLimits { max_total_bytes: 1, ..ExtractLimits::default() };
        for name in ["two_files_with_content_lzma.7z", "solid.rar"] {
            let out = scratch(&format!("cap-{name}"));
            let result = extract_archive(&fixture(name), &out, &tight, &no_tools());
            assert!(result.is_err(), "{name} passed a 1 byte cap");
            assert!(!out.exists());
        }
    }

    #[test]
    fn archive_names_use_forward_slashes() {
        assert_eq!(portable_name("materials\\hud\\a.vtex_c"), "materials/hud/a.vtex_c");
    }
}
