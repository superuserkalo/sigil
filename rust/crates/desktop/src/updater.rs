//! Sigil updates itself: a signed manifest, a verified download, and a swap that keeps the
//! previous version until the new one has started.
//!
//! A release on GitHub carries the zip for each platform plus `sigil-update.json` (version, and
//! per platform the asset name, size and SHA-256) and `sigil-update.json.sig`, an Ed25519
//! signature by the key in [`UPDATE_PUBLIC_KEY`]. `scripts/release-sign.mjs` writes both; the
//! private key never leaves the maintainer's machine (`docs/runbooks/releasing.md`).
//!
//! The flow:
//!
//! 1. [`check`] reads the newest release (Stable or Preview), downloads the manifest and its
//!    signature, verifies them and refuses anything that is not newer than this build.
//! 2. [`download`] streams the zip into the app data directory on a worker thread, checks the
//!    size and the hash against the signed manifest, unpacks it into `updates/staged/<version>`
//!    and records `updates/pending.json`.
//! 3. "Restart to update" ([`restart_now`]) or the next launch ([`on_start`]) copies the staged
//!    files next to the install, renames the current `Sigil.exe` (or `Sigil.app`) aside as
//!    `.old`, moves the new one in, records `updates/applied.json` and starts the new version.
//! 4. The new version writes "started" into `applied.json` once it has drawn a frame. If a launch
//!    of the new version finds `applied.json` unconfirmed from an earlier launch, it restores the
//!    `.old` files and starts the previous version. A confirmed update deletes `.old` on the next
//!    start.
//!
//! Every write is outside the game install. Renames are atomic on one volume; a failed step
//! undoes the renames before it. Network calls go through `deadlock_core::net`, so offline mode
//! blocks them; hosts are `api.github.com` and `github.com/<repo>/releases/download` (which
//! redirects to GitHub's release asset host).

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use rune::{El, Tx, Waker, row};
use sha2::{Digest, Sha256};

use crate::app::State;
use crate::jobs::Job;

/// This build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The release signing key (Ed25519, raw 32 bytes, base64). Key id `0cd4b3b09aeb594e`.
/// Rotating it: add the new key here beside the old one for a release, then drop the old one.
pub const UPDATE_PUBLIC_KEY: &str = "4evgXgXnW0EpMdq3EZoHd1M4uLc/VPFafEZ1IVWTQOk=";

/// Every key a manifest may be signed with.
pub const TRUSTED_KEYS: &[&str] = &[UPDATE_PUBLIC_KEY];

/// The signed message is this prefix plus the exact manifest bytes.
const SIGNING_CONTEXT: &[u8] = b"sigil-update-manifest v1\n";
const SIGNATURE_HEADER: &str = "sigil-update-signature v1";

pub const MANIFEST_ASSET: &str = "sigil-update.json";
pub const SIGNATURE_ASSET: &str = "sigil-update.json.sig";

const GITHUB_API: &str = "https://api.github.com";
const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_SIGNATURE: u64 = 1024;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;
/// The folder next to the install that the new files are copied into before the swap.
const NEW_ROOT: &str = ".sigil-update-new";
/// The Windows executable name a release zip carries.
const WINDOWS_EXE: &str = "Sigil.exe";
/// The macOS bundle name a release zip carries.
const MAC_BUNDLE: &str = "Sigil.app";
/// The variable Rune's single-instance step reads: the new process waits for this pid to exit
/// (`vendor/rune/crates/rune/src/quit/relaunch.rs`), as `Tx::relaunch` does.
const RELAUNCH_AFTER: &str = "RUNE_RELAUNCH_AFTER";

// --- versions --------------------------------------------------------------------

/// A release version: up to three numbers and an optional pre-release tail (`0.2.0-preview.1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}

/// Parses `x`, `x.y`, `x.y.z` with an optional `v` and `-pre`; `+build` is ignored.
pub fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim().trim_start_matches('v');
    let text = text.split('+').next()?;
    let (core_text, pre_text) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let parts: Vec<&str> = core_text.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut core = [0u64; 3];
    for (slot, part) in core.iter_mut().zip(&parts) {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    let pre = match pre_text {
        Some(pre) => {
            let ids: Vec<String> = pre.split('.').map(str::to_string).collect();
            if ids
                .iter()
                .any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
            {
                return None;
            }
            ids
        }
        None => Vec::new(),
    };
    Some(Version { core, pre })
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match self.core.cmp(&other.core) {
            Equal => {}
            unequal => return unequal,
        }
        // A release outranks its own pre-releases.
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => return Equal,
            (true, false) => return Greater,
            (false, true) => return Less,
            _ => {}
        }
        for (a, b) in self.pre.iter().zip(&other.pre) {
            let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                (Ok(_), Err(_)) => Less,
                (Err(_), Ok(_)) => Greater,
                (Err(_), Err(_)) => a.cmp(b),
            };
            if order != Equal {
                return order;
            }
        }
        self.pre.len().cmp(&other.pre.len())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// True only when `candidate` is strictly newer than `current`. An unreadable version is never
/// newer, so a broken tag cannot pass as an upgrade.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

// --- manifest and signature ------------------------------------------------------

/// One platform's download in a manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct PlatformAsset {
    pub asset: String,
    pub size: u64,
    pub sha256: [u8; 32],
    /// Windows: replace the `assets` folder beside `Sigil.exe` too.
    pub assets: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub platforms: Vec<(String, PlatformAsset)>,
}

impl Manifest {
    pub fn platform(&self, key: &str) -> Option<&PlatformAsset> {
        self.platforms.iter().find(|(k, _)| k == key).map(|(_, a)| a)
    }
}

const PLATFORMS: [&str; 3] = ["windows-x86_64", "macos-aarch64", "macos-x86_64"];

/// The manifest key for this build, or "" where Sigil ships no update.
pub fn platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        _ => "",
    }
}

/// Parse a manifest. Call it only on bytes [`verify_signature`] accepted.
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("The update manifest is not JSON: {e}"))?;
    let bad = |what: &str| format!("The update manifest has {what}.");
    if value.get("format").and_then(|v| v.as_u64()) != Some(1) {
        return Err(bad("an unknown format"));
    }
    if value.get("product").and_then(|v| v.as_str()) != Some("sigil") {
        return Err(bad("another product"));
    }
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .filter(|v| parse_version(v).is_some())
        .ok_or_else(|| bad("no readable version"))?
        .to_string();
    let map = value
        .get("platforms")
        .and_then(|v| v.as_object())
        .ok_or_else(|| bad("no platforms"))?;
    let mut platforms = Vec::new();
    for (key, entry) in map {
        if !PLATFORMS.contains(&key.as_str()) {
            return Err(bad(&format!("an unknown platform ({key})")));
        }
        let asset = entry
            .get("asset")
            .and_then(|v| v.as_str())
            .filter(|name| {
                name.ends_with(".zip")
                    && !name.starts_with('.')
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
            .ok_or_else(|| bad(&format!("a bad asset name for {key}")))?
            .to_string();
        let size = entry
            .get("size")
            .and_then(|v| v.as_u64())
            .filter(|s| *s > 0 && *s <= MAX_DOWNLOAD)
            .ok_or_else(|| bad(&format!("a bad size for {key}")))?;
        let sha256 = entry
            .get("sha256")
            .and_then(|v| v.as_str())
            .and_then(parse_hex32)
            .ok_or_else(|| bad(&format!("a bad sha256 for {key}")))?;
        let assets = entry.get("assets").and_then(|v| v.as_bool()).unwrap_or(false);
        platforms.push((key.clone(), PlatformAsset { asset, size, sha256, assets }));
    }
    if platforms.is_empty() {
        return Err(bad("no platforms"));
    }
    Ok(Manifest { version, platforms })
}

fn parse_hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The first 8 bytes of SHA-256 over the raw public key, in hex.
pub fn key_id(raw: &[u8]) -> String {
    hex(&Sha256::digest(raw)[..8])
}

/// Ok when `signature_file` is a valid signature over `manifest` by one of `keys` (base64 raw
/// Ed25519 public keys).
pub fn verify_signature(manifest: &[u8], signature_file: &str, keys: &[&str]) -> Result<(), String> {
    let invalid = || "The update signature is not valid. Sigil did not install it.".to_string();
    let mut lines = signature_file.lines().map(str::trim);
    if lines.next() != Some(SIGNATURE_HEADER) {
        return Err(invalid());
    }
    let id = lines.next().and_then(|l| l.strip_prefix("key ")).ok_or_else(invalid)?;
    let sig = lines
        .next()
        .and_then(|l| l.strip_prefix("sig "))
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        .filter(|s| s.len() == 64)
        .ok_or_else(invalid)?;
    let raw = keys
        .iter()
        .filter_map(|k| base64::engine::general_purpose::STANDARD.decode(k).ok())
        .filter(|k| k.len() == 32)
        .find(|k| key_id(k) == id)
        .ok_or_else(|| "The update is signed by a key Sigil does not trust.".to_string())?;
    let mut message = Vec::with_capacity(SIGNING_CONTEXT.len() + manifest.len());
    message.extend_from_slice(SIGNING_CONTEXT);
    message.extend_from_slice(manifest);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &raw)
        .verify(&message, &sig)
        .map_err(|_| invalid())
}

/// Verify, then parse.
pub fn verified_manifest(manifest: &[u8], signature_file: &str, keys: &[&str]) -> Result<Manifest, String> {
    verify_signature(manifest, signature_file, keys)?;
    parse_manifest(manifest)
}

// --- check -----------------------------------------------------------------------

/// The newest release and whether this build can install it in place.
#[derive(Clone, Debug)]
pub struct Offer {
    pub version: String,
    /// The release page.
    pub url: String,
    /// The release body from GitHub, as written.
    pub notes: String,
    /// What to download, or why this release cannot be installed from inside Sigil.
    pub install: Result<Download, String>,
}

/// A verified manifest and where its zip for this platform lives.
#[derive(Clone, Debug)]
pub struct Download {
    pub version: String,
    pub url: String,
    pub entry: PlatformAsset,
    pub manifest: Vec<u8>,
    pub signature: String,
}

/// The release API root. Debug builds honour `SIGIL_UPDATE_API` so a test can point the check
/// at a local server; release builds always ask GitHub.
pub fn api_base() -> String {
    #[cfg(debug_assertions)]
    if let Some(base) = std::env::var("SIGIL_UPDATE_API").ok().filter(|v| !v.trim().is_empty()) {
        return base.trim().trim_end_matches('/').to_string();
    }
    GITHUB_API.to_string()
}

/// A download URL from the release listing is fetched only when it is GitHub's release download
/// path for `repo` (or, in a debug run against a local server, under that server).
fn allowed_url(url: &str, api: &str, repo: &str) -> bool {
    let github = format!("https://github.com/{repo}/releases/download/");
    url.starts_with(&github) || (api != GITHUB_API && url.starts_with(&format!("{api}/")))
}

/// `wait`: for the response to start; `body`: for the whole body to arrive.
fn agent(wait: Duration, body: Duration) -> ureq::Agent {
    deadlock_core::net::with_proxy(
        ureq::Agent::config_builder()
            .middleware(deadlock_core::net::gate)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(wait))
            .timeout_recv_body(Some(body)),
    )
    .build()
    .new_agent()
}

fn get_limited(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "sigil-updater")
        .call()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    if out.len() as u64 > limit {
        return Err(format!("{url} is larger than an update file can be."));
    }
    Ok(out)
}

/// Read the newest release. `Ok(None)` when the repository has none.
pub fn check(
    api: &str,
    repo: &str,
    current: &str,
    preview: bool,
    keys: &[&str],
    platform: &str,
) -> Result<Option<Offer>, String> {
    let agent = agent(Duration::from_secs(20), Duration::from_secs(30));
    // Stable reads the latest release; Preview reads the newest one, pre-releases included.
    let url = if preview {
        format!("{api}/repos/{repo}/releases?per_page=10")
    } else {
        format!("{api}/repos/{repo}/releases/latest")
    };
    let mut response = match agent
        .get(&url)
        .header("User-Agent", "sigil-updater")
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let release = match value.as_array() {
        Some(list) => match list
            .iter()
            .find(|r| !r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false))
        {
            Some(r) => r.clone(),
            None => return Ok(None),
        },
        None => value,
    };
    let str_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let tag = str_of(&release, "tag_name");
    if tag.is_empty() {
        return Ok(None);
    }
    let version = tag.trim_start_matches('v').to_string();
    if parse_version(&version).is_none() {
        return Err(format!("The newest release has an unreadable version ({tag})."));
    }
    let assets: Vec<(String, String)> = release
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|list| {
            list.iter()
                .map(|a| (str_of(a, "name"), str_of(a, "browser_download_url")))
                .collect()
        })
        .unwrap_or_default();
    let install = if is_newer(&version, current) {
        signed_download(&agent, api, repo, &assets, &version, current, keys, platform)
    } else {
        Err("This version or a newer one is installed.".to_string())
    };
    Ok(Some(Offer {
        url: str_of(&release, "html_url"),
        notes: str_of(&release, "body"),
        version,
        install,
    }))
}

#[allow(clippy::too_many_arguments)]
fn signed_download(
    agent: &ureq::Agent,
    api: &str,
    repo: &str,
    assets: &[(String, String)],
    version: &str,
    current: &str,
    keys: &[&str],
    platform: &str,
) -> Result<Download, String> {
    let find = |name: &str| {
        assets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, url)| url.clone())
            .filter(|url| allowed_url(url, api, repo))
    };
    let unsigned = "This release has no signed update. Download it from the release page.".to_string();
    let manifest_url = find(MANIFEST_ASSET).ok_or_else(|| unsigned.clone())?;
    let signature_url = find(SIGNATURE_ASSET).ok_or(unsigned)?;
    let manifest = get_limited(agent, &manifest_url, MAX_MANIFEST)?;
    let signature = String::from_utf8(get_limited(agent, &signature_url, MAX_SIGNATURE)?)
        .map_err(|_| "The update signature is not text.".to_string())?;
    let parsed = verified_manifest(&manifest, &signature, keys)?;
    if parse_version(&parsed.version) != parse_version(version) {
        return Err(format!(
            "The signed manifest is for {}, but the release is {version}.",
            parsed.version
        ));
    }
    if !is_newer(&parsed.version, current) {
        return Err("Sigil does not install an older version.".to_string());
    }
    let entry = parsed
        .platform(platform)
        .cloned()
        .ok_or_else(|| "This release has no build for this system.".to_string())?;
    let url = find(&entry.asset).ok_or_else(|| format!("The release is missing {}.", entry.asset))?;
    Ok(Download {
        version: parsed.version,
        url,
        entry,
        manifest,
        signature,
    })
}

// --- download and stage ----------------------------------------------------------

/// Bytes so far and the expected total, shared with the worker.
#[derive(Default, Debug)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

/// A verified update, unpacked and waiting for a restart.
#[derive(Clone, Debug, PartialEq)]
pub struct Staged {
    pub version: String,
    pub tree: PathBuf,
    pub assets: bool,
}

/// `updates/` under the app data directory.
pub fn updates_dir() -> PathBuf {
    crate::data::data_dir().join("updates")
}

/// Download, verify, unpack and record `pending.json` under `dir`.
pub fn download(
    offer: &Download,
    dir: &Path,
    current: &str,
    keys: &[&str],
    platform: &str,
    progress: &Progress,
    wake: Option<Waker>,
) -> Result<Staged, String> {
    // The manifest is checked again here: nothing the worker trusts comes from the UI thread.
    let manifest = verified_manifest(&offer.manifest, &offer.signature, keys)?;
    if !is_newer(&manifest.version, current) {
        return Err("Sigil does not install an older version.".to_string());
    }
    let entry = manifest
        .platform(platform)
        .cloned()
        .ok_or_else(|| "This release has no build for this system.".to_string())?;
    let io = |e: std::io::Error| format!("Could not save the update: {e}");
    let parts = dir.join("download");
    fs::create_dir_all(&parts).map_err(io)?;
    let part = parts.join(format!("{}.part", entry.asset));
    progress.total.store(entry.size, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    // A slow line gets half an hour for the zip; a stalled one does not hold the worker forever.
    let agent = agent(Duration::from_secs(60), Duration::from_secs(30 * 60));
    let mut response = agent
        .get(&offer.url)
        .header("User-Agent", "sigil-updater")
        .call()
        .map_err(|e| format!("The download failed: {e}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&part).map_err(io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut count = 0u64;
    let mut last_wake = Instant::now();
    loop {
        let n = reader.read(&mut buffer).map_err(|e| format!("The download failed: {e}"))?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > entry.size {
            drop(file);
            let _ = fs::remove_file(&part);
            return Err("The download is larger than the signed manifest says.".to_string());
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(io)?;
        progress.done.store(count, Ordering::Relaxed);
        if last_wake.elapsed() >= Duration::from_millis(200) {
            last_wake = Instant::now();
            if let Some(wake) = &wake {
                wake();
            }
        }
    }
    file.sync_all().map_err(io)?;
    drop(file);
    let digest: [u8; 32] = hasher.finalize().into();
    if count != entry.size || digest != entry.sha256 {
        let _ = fs::remove_file(&part);
        return Err(if count != entry.size {
            "The download is shorter than the signed manifest says.".to_string()
        } else {
            "The download does not match the signed hash. Sigil did not install it.".to_string()
        });
    }

    let stage = dir.join("staged").join(&manifest.version);
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(io)?;
    }
    fs::create_dir_all(&stage).map_err(io)?;
    let zip_path = stage.join(&entry.asset);
    fs::rename(&part, &zip_path).map_err(io)?;
    fs::write(stage.join(MANIFEST_ASSET), &offer.manifest).map_err(io)?;
    fs::write(stage.join(SIGNATURE_ASSET), &offer.signature).map_err(io)?;
    let tree = stage.join("tree");
    unpack(&zip_path, &tree)?;
    check_tree(&tree, platform, &manifest.version)?;
    let staged = Staged {
        version: manifest.version.clone(),
        tree,
        assets: entry.assets,
    };
    write_json(&dir.join("pending.json"), &pending_json(&staged, platform))?;
    Ok(staged)
}

/// Unpack a release zip. A single top folder is dropped (`Sigil-0.2.0-windows-x64/`), except an
/// app bundle. Paths that leave the folder, links and oversized archives are refused.
pub fn unpack(zip_path: &Path, out: &Path) -> Result<(), String> {
    let bad = |e: &dyn std::fmt::Display| format!("The update archive could not be read: {e}");
    let mut archive = zip::ZipArchive::new(File::open(zip_path).map_err(|e| bad(&e))?).map_err(|e| bad(&e))?;
    let mut names = Vec::with_capacity(archive.len());
    let mut total = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| bad(&e))?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| "The update archive has an unsafe path.".to_string())?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err("The update archive contains a link.".to_string());
        }
        total = total.saturating_add(entry.size());
        names.push(name);
    }
    if total > MAX_UNPACKED {
        return Err("The update archive is too large.".to_string());
    }
    let strip = common_top(&names);
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| bad(&e))?;
    }
    fs::create_dir_all(out).map_err(|e| bad(&e))?;
    for (i, name) in names.iter().enumerate() {
        let relative = match &strip {
            Some(top) => match name.strip_prefix(top) {
                Ok(rest) => rest.to_path_buf(),
                Err(_) => continue,
            },
            None => name.clone(),
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = out.join(&relative);
        let mut entry = archive.by_index(i).map_err(|e| bad(&e))?;
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| bad(&e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| bad(&e))?;
        }
        let mut file = File::create(&target).map_err(|e| bad(&e))?;
        let size = entry.size();
        let copied = std::io::copy(&mut (&mut entry).take(size + 1), &mut file).map_err(|e| bad(&e))?;
        if copied != size {
            return Err("The update archive is damaged.".to_string());
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode & 0o755));
        }
    }
    Ok(())
}

/// The one top folder every entry sits in, unless it is an app bundle.
fn common_top(names: &[PathBuf]) -> Option<PathBuf> {
    let first = |p: &PathBuf| match p.components().next() {
        Some(Component::Normal(c)) => Some(PathBuf::from(c)),
        _ => None,
    };
    let top = first(names.first()?)?;
    if top.extension().is_some_and(|e| e == "app") {
        return None;
    }
    let nested = names.iter().any(|n| n.components().count() > 1);
    (nested && names.iter().all(|n| first(n).as_ref() == Some(&top))).then_some(top)
}

/// The unpacked tree must hold what this platform swaps in.
fn check_tree(tree: &Path, platform: &str, version: &str) -> Result<(), String> {
    if platform.starts_with("windows") {
        if !tree.join(WINDOWS_EXE).is_file() {
            return Err(format!("The update has no {WINDOWS_EXE}."));
        }
        return Ok(());
    }
    let contents = tree.join(MAC_BUNDLE).join("Contents");
    let plist = fs::read_to_string(contents.join("Info.plist"))
        .map_err(|_| format!("The update has no {MAC_BUNDLE}."))?;
    if !contents.join("MacOS").is_dir() {
        return Err(format!("The update's {MAC_BUNDLE} has no program."));
    }
    let short = plist
        .split("CFBundleShortVersionString")
        .nth(1)
        .and_then(|rest| rest.split("<string>").nth(1))
        .and_then(|rest| rest.split("</string>").next())
        .map(str::trim);
    if short != Some(version) {
        return Err("The update's app reports another version.".to_string());
    }
    Ok(())
}

// --- install layout and the swap plan ---------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `Sigil.exe` and `assets/` from the Windows zip.
    Windows,
    /// `Sigil.app` on macOS.
    MacBundle,
}

/// Where the running copy lives.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub kind: Kind,
    /// The folder that holds `target`.
    pub dir: PathBuf,
    /// `Sigil.exe` or `Sigil.app`.
    pub target: PathBuf,
    /// The art folder beside `Sigil.exe` (Windows only).
    pub assets: Option<PathBuf>,
}

impl Layout {
    /// `Sigil.exe.old` on Windows; a hidden `.Sigil.app.old` beside a bundle.
    pub fn sidecar(&self, path: &Path, suffix: &str) -> PathBuf {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = path.parent().unwrap_or(&self.dir);
        match self.kind {
            Kind::Windows => parent.join(format!("{name}.{suffix}")),
            Kind::MacBundle => parent.join(format!(".{name}.{suffix}")),
        }
    }

    pub fn new_root(&self) -> PathBuf {
        self.dir.join(NEW_ROOT)
    }

    /// What the new tree holds in place of `target`.
    fn incoming(&self, new_root: &Path) -> PathBuf {
        new_root.join(match self.kind {
            Kind::Windows => WINDOWS_EXE,
            Kind::MacBundle => MAC_BUNDLE,
        })
    }
}

/// Where an executable path sits, or why that copy cannot update itself.
pub fn detect(os: &str, exe: &Path) -> Result<Layout, String> {
    let not_release = "This copy of Sigil was not installed from a release, so it cannot update itself. Download the new version from the release page.";
    match os {
        "windows" => {
            let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !name.eq_ignore_ascii_case(WINDOWS_EXE) {
                return Err(not_release.to_string());
            }
            let dir = exe.parent().ok_or(not_release)?.to_path_buf();
            Ok(Layout {
                kind: Kind::Windows,
                assets: Some(dir.join("assets")),
                target: exe.to_path_buf(),
                dir,
            })
        }
        "macos" => {
            let macos = exe.parent().filter(|p| p.file_name().is_some_and(|n| n == "MacOS"));
            let bundle = macos
                .and_then(|m| m.parent())
                .filter(|c| c.file_name().is_some_and(|n| n == "Contents"))
                .and_then(|c| c.parent())
                .filter(|b| b.extension().is_some_and(|e| e == "app"))
                .ok_or(not_release)?;
            if bundle.to_string_lossy().contains("/AppTranslocation/") {
                return Err("Move Sigil to the Applications folder and open it again, then update.".to_string());
            }
            let dir = bundle.parent().ok_or(not_release)?.to_path_buf();
            Ok(Layout {
                kind: Kind::MacBundle,
                target: bundle.to_path_buf(),
                assets: None,
                dir,
            })
        }
        _ => Err("Sigil does not update itself on this system.".to_string()),
    }
}

/// The running copy's layout.
pub fn current_layout() -> Result<Layout, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Sigil could not find itself: {e}"))?;
    detect(std::env::consts::OS, &exe)
}

/// One file-system step of a swap.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Delete a file or folder if it is there. A failure stops the plan.
    Remove(PathBuf),
    /// Move a path. A failure undoes the moves before it, newest first.
    Rename { from: PathBuf, to: PathBuf },
    /// Move a file over another, best effort and not undone (the notices beside the exe).
    Replace { from: PathBuf, to: PathBuf },
    /// Delete if possible; a file in use stays for the next start.
    Tidy(PathBuf),
}

/// What the swap did, saved as `applied.json` for rollback and cleanup.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub from: String,
    pub to: String,
    /// The new version brought its own `assets` folder.
    pub assets_new: bool,
    /// The previous `assets` folder was kept as `assets.old`.
    pub assets_backup: bool,
    pub launches: u32,
    pub confirmed: bool,
}

/// Swap the new tree in. `has_assets`: an `assets` folder exists beside the exe now.
/// `extras`: loose files at the top of the new tree (the notices), replaced best effort.
pub fn apply_plan(layout: &Layout, new_root: &Path, replace_assets: bool, has_assets: bool, extras: &[String]) -> Vec<Step> {
    let old = layout.sidecar(&layout.target, "old");
    let mut steps = vec![
        Step::Remove(old.clone()),
        Step::Rename { from: layout.target.clone(), to: old },
        Step::Rename { from: layout.incoming(new_root), to: layout.target.clone() },
    ];
    if let (true, Some(assets)) = (replace_assets, &layout.assets) {
        let old_assets = layout.sidecar(assets, "old");
        steps.push(Step::Remove(old_assets.clone()));
        if has_assets {
            steps.push(Step::Rename { from: assets.clone(), to: old_assets });
        }
        steps.push(Step::Rename { from: new_root.join("assets"), to: assets.clone() });
    }
    for name in extras {
        steps.push(Step::Replace { from: new_root.join(name), to: layout.dir.join(name) });
    }
    steps.push(Step::Tidy(new_root.to_path_buf()));
    steps
}

/// Put the previous version back. The new one is kept as `.failed` until the next start.
pub fn rollback_plan(layout: &Layout, applied: &Applied) -> Vec<Step> {
    let failed = layout.sidecar(&layout.target, "failed");
    let mut steps = vec![
        Step::Remove(failed.clone()),
        Step::Rename { from: layout.target.clone(), to: failed.clone() },
        Step::Rename { from: layout.sidecar(&layout.target, "old"), to: layout.target.clone() },
    ];
    if let Some(assets) = &layout.assets {
        let failed_assets = layout.sidecar(assets, "failed");
        if applied.assets_new {
            steps.push(Step::Remove(failed_assets.clone()));
            steps.push(Step::Rename { from: assets.clone(), to: failed_assets.clone() });
        }
        if applied.assets_backup {
            steps.push(Step::Rename { from: layout.sidecar(assets, "old"), to: assets.clone() });
        }
        steps.push(Step::Tidy(failed_assets));
    }
    // The failed program is this process on Windows; it goes at the next start.
    steps.push(Step::Tidy(failed));
    steps
}

/// After a confirmed update or a rollback: delete what the swap left beside the install.
pub fn cleanup_plan(layout: &Layout) -> Vec<Step> {
    let mut steps = vec![
        Step::Tidy(layout.sidecar(&layout.target, "old")),
        Step::Tidy(layout.sidecar(&layout.target, "failed")),
    ];
    if let Some(assets) = &layout.assets {
        steps.push(Step::Tidy(layout.sidecar(assets, "old")));
        steps.push(Step::Tidy(layout.sidecar(assets, "failed")));
    }
    steps.push(Step::Tidy(layout.new_root()));
    steps
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Run a plan. On a failed `Remove` or `Rename`, the renames already done are reversed.
pub fn run(steps: &[Step]) -> Result<(), String> {
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo = |done: &mut Vec<(PathBuf, PathBuf)>| {
        while let Some((from, to)) = done.pop() {
            let _ = fs::rename(&to, &from);
        }
    };
    for step in steps {
        match step {
            Step::Remove(path) => {
                if let Err(e) = remove_any(path) {
                    undo(&mut done);
                    return Err(format!("Could not remove {}: {e}", path.display()));
                }
            }
            Step::Rename { from, to } => {
                if let Err(e) = fs::rename(from, to) {
                    undo(&mut done);
                    return Err(format!("Could not move {} to {}: {e}", from.display(), to.display()));
                }
                done.push((from.clone(), to.clone()));
            }
            Step::Replace { from, to } => {
                let _ = fs::rename(from, to);
            }
            Step::Tidy(path) => {
                let _ = remove_any(path);
            }
        }
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copy the staged tree beside the install (same volume, so the swap is a rename) and swap it
/// in. Returns the record for `applied.json`.
pub fn install(layout: &Layout, staged: &Staged, from: &str) -> Result<Applied, String> {
    if !is_newer(&staged.version, from) {
        return Err("Sigil does not install an older version.".to_string());
    }
    let new_root = layout.new_root();
    remove_any(&new_root).map_err(|e| format!("Could not prepare the update: {e}"))?;
    if let Err(e) = copy_tree(&staged.tree, &new_root) {
        let _ = remove_any(&new_root);
        return Err(format!("Could not copy the update next to Sigil: {e}"));
    }
    let incoming = layout.incoming(&new_root);
    if !incoming.exists() {
        let _ = remove_any(&new_root);
        return Err("The staged update is incomplete.".to_string());
    }
    let new_assets = new_root.join("assets").is_dir();
    let replace_assets = layout.kind == Kind::Windows && staged.assets && new_assets;
    let has_assets = layout.assets.as_ref().is_some_and(|a| a.is_dir());
    let extras: Vec<String> = match layout.kind {
        Kind::Windows => fs::read_dir(&new_root)
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !n.eq_ignore_ascii_case(WINDOWS_EXE))
                    .collect()
            })
            .unwrap_or_default(),
        Kind::MacBundle => Vec::new(),
    };
    let plan = apply_plan(layout, &new_root, replace_assets, has_assets, &extras);
    if let Err(error) = run(&plan) {
        let _ = remove_any(&new_root);
        return Err(error);
    }
    Ok(Applied {
        from: from.to_string(),
        to: staged.version.clone(),
        assets_new: replace_assets,
        assets_backup: replace_assets && has_assets,
        launches: 0,
        confirmed: false,
    })
}

// --- records ---------------------------------------------------------------------

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let io = |e: std::io::Error| format!("Could not save {}: {e}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = File::create(&tmp).map_err(io)?;
    file.write_all(serde_json::to_string_pretty(value).unwrap_or_default().as_bytes())
        .map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    fs::rename(&tmp, path).map_err(io)
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn pending_json(staged: &Staged, platform: &str) -> serde_json::Value {
    serde_json::json!({
        "version": staged.version,
        "platform": platform,
        "tree": staged.tree,
        "assets": staged.assets,
    })
}

fn read_pending(dir: &Path, platform: &str) -> Option<Staged> {
    let value = read_json(&dir.join("pending.json"))?;
    if value.get("platform")?.as_str()? != platform {
        return None;
    }
    Some(Staged {
        version: value.get("version")?.as_str()?.to_string(),
        tree: PathBuf::from(value.get("tree")?.as_str()?),
        assets: value.get("assets").and_then(|v| v.as_bool()).unwrap_or(false),
    })
}

fn applied_json(a: &Applied) -> serde_json::Value {
    serde_json::json!({
        "from": a.from,
        "to": a.to,
        "assets_new": a.assets_new,
        "assets_backup": a.assets_backup,
        "launches": a.launches,
        "confirmed": a.confirmed,
    })
}

fn read_applied(dir: &Path) -> Option<Applied> {
    let v = read_json(&dir.join("applied.json"))?;
    let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    Some(Applied {
        from: v.get("from")?.as_str()?.to_string(),
        to: v.get("to")?.as_str()?.to_string(),
        assets_new: b("assets_new"),
        assets_backup: b("assets_backup"),
        launches: v.get("launches").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        confirmed: b("confirmed"),
    })
}

// --- start-up --------------------------------------------------------------------

/// What a launch does about updates, decided from the records alone.
#[derive(Clone, Debug, PartialEq)]
pub enum StartAction {
    Nothing,
    /// The new version started before: delete `.old` and the record.
    Cleanup,
    /// First launch of the new version: count it, wait for the first frame.
    CountLaunch,
    /// An earlier launch of this version never drew a frame: put the old one back.
    Rollback,
    /// This is not the version the record names (a manual reinstall): forget the record.
    Forget,
    /// A staged update is waiting: install it now.
    Apply,
    /// A staged update is not newer: delete it.
    DropPending,
}

pub fn decide(applied: Option<&Applied>, pending: Option<&Staged>, current: &str) -> StartAction {
    if let Some(a) = applied {
        if parse_version(&a.to) != parse_version(current) {
            return StartAction::Forget;
        }
        return if a.confirmed {
            StartAction::Cleanup
        } else if a.launches == 0 {
            StartAction::CountLaunch
        } else {
            StartAction::Rollback
        };
    }
    match pending {
        Some(p) if is_newer(&p.version, current) => StartAction::Apply,
        Some(_) => StartAction::DropPending,
        None => StartAction::Nothing,
    }
}

/// Start the installed copy at `layout` and let it wait for this process to exit.
fn launch(layout: &Layout) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let pid = std::process::id().to_string();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = |e: std::io::Error| format!("Could not start the new version: {e}");
    match layout.kind {
        Kind::MacBundle => {
            let status = Command::new("/usr/bin/open")
                .arg("-n")
                .arg(&layout.target)
                .arg("--env")
                .arg(format!("{RELAUNCH_AFTER}={pid}"))
                .arg("--args")
                .args(&args)
                .status()
                .map_err(error)?;
            if !status.success() {
                return Err(format!("Could not start the new version: open exited with {status}"));
            }
            Ok(())
        }
        Kind::Windows => {
            let mut command = Command::new(&layout.target);
            command
                .args(&args)
                .env(RELAUNCH_AFTER, &pid)
                .current_dir(&layout.dir)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
            command.spawn().map(drop).map_err(error)
        }
    }
}

fn log(line: String) {
    crate::screens::settings::log::info(line);
}

/// Called once from `main` before the window opens. Rolls back a version that did not start,
/// tidies after one that did, and installs a staged update. `Some` means this process should
/// exit because another version was started in its place.
pub fn on_start() -> Option<std::process::ExitCode> {
    if rune::is_headless() {
        return None;
    }
    let layout = current_layout().ok()?;
    let dir = updates_dir();
    let code = start_with(&layout, &dir, VERSION, platform_key(), true)?;
    // The started version waits for this process; let go of the single-instance claim first.
    rune::release_instance();
    Some(code)
}

/// [`on_start`] with its inputs, for tests. `relaunch` false skips starting a process.
pub fn start_with(
    layout: &Layout,
    dir: &Path,
    current: &str,
    platform: &str,
    relaunch: bool,
) -> Option<std::process::ExitCode> {
    let applied = read_applied(dir);
    let pending = read_pending(dir, platform);
    match decide(applied.as_ref(), pending.as_ref(), current) {
        StartAction::Nothing => {
            // Leftovers of a rollback or an interrupted swap.
            let _ = run(&cleanup_plan(layout));
            None
        }
        StartAction::Cleanup | StartAction::Forget => {
            let _ = run(&cleanup_plan(layout));
            let _ = fs::remove_file(dir.join("applied.json"));
            let _ = fs::remove_dir_all(dir.join("staged"));
            None
        }
        StartAction::CountLaunch => {
            let mut a = applied?;
            a.launches += 1;
            let _ = write_json(&dir.join("applied.json"), &applied_json(&a));
            None
        }
        StartAction::Rollback => {
            let a = applied?;
            match run(&rollback_plan(layout, &a)) {
                Ok(()) => {
                    log(format!("Sigil {} did not start; went back to {}", a.to, a.from));
                    let _ = write_json(
                        &dir.join("rolled-back.json"),
                        &serde_json::json!({ "version": a.to, "back_to": a.from }),
                    );
                    let _ = fs::remove_file(dir.join("applied.json"));
                    let _ = fs::remove_dir_all(dir.join("staged"));
                    if relaunch && launch(layout).is_ok() {
                        return Some(std::process::ExitCode::SUCCESS);
                    }
                    None
                }
                Err(error) => {
                    // Keep running the new version rather than loop on a rollback that fails.
                    log(format!("Rollback failed: {error}"));
                    let mut a = a;
                    a.confirmed = true;
                    let _ = write_json(&dir.join("applied.json"), &applied_json(&a));
                    None
                }
            }
        }
        StartAction::DropPending => {
            let _ = fs::remove_file(dir.join("pending.json"));
            let _ = fs::remove_dir_all(dir.join("staged"));
            None
        }
        StartAction::Apply => {
            let staged = pending?;
            // The record goes first so a crash mid-swap does not retry forever.
            let _ = fs::remove_file(dir.join("pending.json"));
            match install(layout, &staged, current) {
                Ok(a) => {
                    let _ = write_json(&dir.join("applied.json"), &applied_json(&a));
                    log(format!("Installed Sigil {} over {}", a.to, a.from));
                    if relaunch && launch(layout).is_ok() {
                        return Some(std::process::ExitCode::SUCCESS);
                    }
                    None
                }
                Err(error) => {
                    log(format!("The staged update did not install: {error}"));
                    None
                }
            }
        }
    }
}

/// The new version drew its first frame: record it, so the next start keeps it.
fn confirm_started(dir: &Path, current: &str) {
    if let Some(mut a) = read_applied(dir)
        && parse_version(&a.to) == parse_version(current)
        && !a.confirmed
    {
        a.confirmed = true;
        let _ = write_json(&dir.join("applied.json"), &applied_json(&a));
        log(format!("Sigil {current} started after an update"));
    }
}

// --- app state -------------------------------------------------------------------

/// The updater's part of the Settings state.
#[derive(Default)]
pub struct Updater {
    pub job: Job<Staged>,
    pub progress: Option<Arc<Progress>>,
    /// The version being downloaded.
    pub downloading: Option<String>,
    /// Ready for "Restart to update".
    pub staged: Option<Staged>,
    pub error: Option<String>,
    /// A version that failed to start and was rolled back, and the one Sigil went back to.
    pub rolled_back: Option<(String, String)>,
    /// The version an automatic download already started for, so a failure does not loop.
    auto_for: Option<String>,
    loaded: bool,
    first_poll: Option<Instant>,
    polls: u32,
    confirmed: bool,
}

impl Updater {
    pub fn ready(&self) -> Option<&str> {
        self.staged.as_ref().map(|s| s.version.as_str())
    }

    /// `(done, total)` while a download runs.
    pub fn progress(&self) -> Option<(u64, u64)> {
        let p = self.progress.as_ref()?;
        Some((p.done.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed)))
    }
}

/// The download the last check offered, if this copy can install it.
pub fn offered(st: &State) -> Option<Download> {
    match &st.settings.update {
        Some(Ok(check)) => check.latest.as_ref()?.install.clone().ok(),
        _ => None,
    }
}

/// Called from Settings' poll on every frame.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    let headless = rune::is_headless();
    let up = &mut st.settings.updater;
    if !up.loaded {
        up.loaded = true;
        #[cfg(debug_assertions)]
        if headless {
            changed |= snapshot_fixture(st);
        }
        let up = &mut st.settings.updater;
        if !headless && !st.demo {
            let path = updates_dir().join("rolled-back.json");
            if let Some(v) = read_json(&path) {
                let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
                up.rolled_back = Some((s("version"), s("back_to")));
                let _ = fs::remove_file(path);
                changed = true;
            }
        }
    }
    let up = &mut st.settings.updater;
    // "Started OK": the first poll comes before the first frame; a later one, a second on,
    // comes after it.
    if !up.confirmed && !headless {
        up.polls += 1;
        match up.first_poll {
            None => {
                up.first_poll = Some(Instant::now());
                tx.wake_after(Duration::from_millis(1200));
            }
            Some(at) if up.polls >= 2 && at.elapsed() >= Duration::from_secs(1) => {
                up.confirmed = true;
                // A development build shares the data folder; only an installed copy confirms.
                if current_layout().is_ok() {
                    confirm_started(&updates_dir(), VERSION);
                }
            }
            Some(_) => {}
        }
    }
    if let Some(result) = up.job.try_take() {
        up.progress = None;
        up.downloading = None;
        match result {
            Ok(staged) => {
                log(format!("Sigil {} is downloaded and verified", staged.version));
                up.staged = Some(staged);
                up.error = None;
            }
            Err(error) => {
                log(format!("Update download failed: {error}"));
                up.error = Some(error);
            }
        }
        changed = true;
    }
    if up.downloading.is_some() {
        changed = true;
    }
    // A check that found a signed update starts the download once.
    if let Some(offer) = offered(st) {
        let up = &st.settings.updater;
        let have = up.staged.as_ref().is_some_and(|s| s.version == offer.version);
        if !have && up.downloading.is_none() && up.auto_for.as_deref() != Some(&offer.version) {
            st.settings.updater.auto_for = Some(offer.version.clone());
            start_download(st);
            changed = true;
        }
    }
    changed
}

/// Debug snapshots only: `SIGIL_UPDATE_SCENE=available|downloading|ready` fills the updater with
/// a fake 0.2.0 release so `--snap` can show each state. Release builds have no such switch.
#[cfg(debug_assertions)]
fn snapshot_fixture(st: &mut State) -> bool {
    let Ok(scene) = std::env::var("SIGIL_UPDATE_SCENE") else {
        return false;
    };
    let entry = PlatformAsset { asset: "Sigil-0.2.0-windows-x64.zip".into(), size: 41_200_000, sha256: [0; 32], assets: true };
    let offer = Offer {
        version: "0.2.0".into(),
        url: "https://github.com/superuserkalo/sigil/releases/tag/v0.2.0".into(),
        notes: "## What's new\n\n- Sigil updates itself from Settings.\n- Browse remembers your last filter.\n- Faster start on large mod folders.\n\n## Fixes\n\n- Load order saves after a drag.".into(),
        install: Ok(Download { version: "0.2.0".into(), url: String::new(), entry, manifest: Vec::new(), signature: String::new() }),
    };
    st.settings.update = Some(Ok(crate::screens::settings::UpdateCheck { latest: Some(offer), newer: true }));
    let up = &mut st.settings.updater;
    up.auto_for = Some("0.2.0".into());
    match scene.as_str() {
        "downloading" => {
            let progress = Arc::new(Progress::default());
            progress.done.store(18_400_000, Ordering::Relaxed);
            progress.total.store(41_200_000, Ordering::Relaxed);
            up.progress = Some(progress);
            up.downloading = Some("0.2.0".into());
        }
        "ready" => {
            up.staged = Some(Staged { version: "0.2.0".into(), tree: PathBuf::new(), assets: true });
        }
        "failed" => {
            up.error = Some("The download does not match the signed hash. Sigil did not install it.".into());
        }
        "undone" => {
            st.settings.update = None;
            up.rolled_back = Some(("0.2.0".into(), "0.1.0".into()));
        }
        _ => {}
    }
    true
}

/// Start downloading the offered update on a worker thread.
pub fn start_download(st: &mut State) {
    let Some(offer) = offered(st) else {
        return;
    };
    if st.settings.updater.job.is_running() || st.demo || rune::is_headless() {
        return;
    }
    if let Err(error) = deadlock_core::net::check() {
        st.settings.updater.error = Some(error);
        return;
    }
    if let Err(error) = current_layout() {
        st.settings.updater.error = Some(error);
        return;
    }
    let progress = Arc::new(Progress::default());
    progress.total.store(offer.entry.size, Ordering::Relaxed);
    let up = &mut st.settings.updater;
    up.progress = Some(progress.clone());
    up.downloading = Some(offer.version.clone());
    up.error = None;
    let wake = st.wake.clone();
    let dir = updates_dir();
    up.job.start(wake.clone(), move || {
        download(&offer, &dir, VERSION, TRUSTED_KEYS, platform_key(), &progress, wake)
    });
}

/// "Restart to update": swap the staged version in, start it and quit.
pub fn restart_now(st: &mut State, tx: &mut Tx) {
    let Some(staged) = st.settings.updater.staged.clone() else {
        return;
    };
    let result = current_layout().and_then(|layout| {
        let a = install(&layout, &staged, VERSION)?;
        let dir = updates_dir();
        let _ = fs::remove_file(dir.join("pending.json"));
        write_json(&dir.join("applied.json"), &applied_json(&a))?;
        log(format!("Installed Sigil {} over {}; restarting", a.to, a.from));
        launch(&layout)
    });
    match result {
        Ok(()) => {
            st.agent.shutdown();
            tx.quit();
        }
        Err(error) => {
            log(format!("Restart to update failed: {error}"));
            st.settings.updater.error = Some(error);
        }
    }
}

/// The sidebar mark beside the version label once an update is ready. A press opens
/// Settings on About and updates.
pub fn sidebar_badge(s: &State, u: &crate::ui::Ui) -> Option<El> {
    let version = s.settings.updater.ready()?;
    let label = crate::screens::settings::i18n::fill(
        crate::screens::settings::i18n::tr("Sigil {} is ready to install"),
        &[version],
    );
    Some(
        row()
            .key("nav-update-ready")
            .size(16.0, 16.0)
            .center()
            .role("button")
            .label(label)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                st.settings.tab = crate::screens::settings::Tab::About;
                st.go(crate::model::Screen::Settings);
            })
            .child(u.dot(crate::ui::Tone::Accent)),
    )
}

/// Bytes as "12.3 MB".
pub fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// The release body as plain lines: Markdown marks dropped, one blank line before each heading
/// and none elsewhere, at most `max` lines.
pub fn plain_notes(body: &str, max: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let heading = trimmed.starts_with('#');
        let mut s = trimmed.trim_start_matches('#').trim().to_string();
        if let Some(rest) = s.strip_prefix("* ").or_else(|| s.strip_prefix("- ")) {
            s = format!("- {rest}");
        }
        s = s.replace("**", "").replace('`', "");
        if heading && !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(s);
        if lines.len() >= max {
            break;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    // A throwaway key made with `scripts/release-sign.mjs keygen` and a manifest it signed with
    // `release-sign.mjs sign`: proves Node's signer and this verifier agree.
    const NODE_KEY: &str = "gQoe7lqcbgC7zT/wnGslSLL35SvTdqq1swg/j3XGrcE=";
    const NODE_MANIFEST: &str = "{\n  \"format\": 1,\n  \"product\": \"sigil\",\n  \"version\": \"9.9.9\",\n  \"platforms\": {\n    \"windows-x86_64\": {\"asset\": \"Sigil-9.9.9-windows-x64.zip\", \"size\": 3, \"sha256\": \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\", \"assets\": true}\n  }\n}\n";
    const NODE_SIG: &str = "sigil-update-signature v1\nkey de5c578c1f14b6c0\nsig efOWCWkwqvKpsPMLL9rbTafvAC2xQU8DCrxl9PEqTFuSVUYzjLQ8FXDMPkXgxCtTu2B04PIomc+NguwXT9t2Ag==\n";

    fn key(seed: u8) -> (Ed25519KeyPair, String) {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
        let public = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
        (pair, public)
    }

    fn sign(pair: &Ed25519KeyPair, manifest: &[u8]) -> String {
        let mut message = SIGNING_CONTEXT.to_vec();
        message.extend_from_slice(manifest);
        let sig = pair.sign(&message);
        format!(
            "{SIGNATURE_HEADER}\nkey {}\nsig {}\n",
            key_id(pair.public_key().as_ref()),
            base64::engine::general_purpose::STANDARD.encode(sig.as_ref())
        )
    }

    fn manifest_for(version: &str, platform: &str, asset: &str, bytes: &[u8], assets: bool) -> Vec<u8> {
        serde_json::to_vec_pretty(&serde_json::json!({
            "format": 1,
            "product": "sigil",
            "version": version,
            "platforms": { platform: {
                "asset": asset,
                "size": bytes.len(),
                "sha256": hex(&Sha256::digest(bytes)),
                "assets": assets,
            }},
        }))
        .unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sigil-updater-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            for (name, data) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        out.into_inner()
    }

    fn windows_layout(dir: &Path) -> Layout {
        Layout {
            kind: Kind::Windows,
            dir: dir.to_path_buf(),
            target: dir.join("Sigil.exe"),
            assets: Some(dir.join("assets")),
        }
    }

    fn read(path: impl AsRef<Path>) -> String {
        fs::read_to_string(path).unwrap()
    }

    // --- versions

    #[test]
    fn versions_order_and_refuse_downgrades() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99"));
        assert!(!is_newer("0.1.0", "0.2.0"), "a downgrade is never newer");
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0-preview.1", "0.2.0"));
        assert!(is_newer("0.2.0", "0.2.0-preview.3"));
        assert!(is_newer("0.2.0-preview.10", "0.2.0-preview.9"));
        assert!(is_newer("0.2.0-rc.1", "0.2.0-preview.1"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(!is_newer("0.2.0", "garbage"));
        assert!(parse_version("1.2.3.4").is_none());
        assert!(parse_version("1..2").is_none());
    }

    // --- manifest

    #[test]
    fn manifest_parses_and_rejects_bad_fields() {
        let m = parse_manifest(NODE_MANIFEST.as_bytes()).unwrap();
        assert_eq!(m.version, "9.9.9");
        let w = m.platform("windows-x86_64").unwrap();
        assert_eq!(w.asset, "Sigil-9.9.9-windows-x64.zip");
        assert_eq!(w.size, 3);
        assert!(w.assets);
        assert!(m.platform("macos-aarch64").is_none());

        let base = serde_json::from_str::<serde_json::Value>(NODE_MANIFEST).unwrap();
        let broken = |path: &[&str], value: serde_json::Value| {
            let mut v = base.clone();
            let mut at = &mut v;
            for key in &path[..path.len() - 1] {
                at = at.get_mut(*key).unwrap();
            }
            at[path[path.len() - 1]] = value;
            parse_manifest(&serde_json::to_vec(&v).unwrap())
        };
        let w = "windows-x86_64";
        assert!(broken(&["format"], 2.into()).is_err());
        assert!(broken(&["product"], "other".into()).is_err());
        assert!(broken(&["version"], "nine".into()).is_err());
        assert!(broken(&["platforms", w, "asset"], "../evil.zip".into()).is_err());
        assert!(broken(&["platforms", w, "asset"], "Sigil.exe".into()).is_err());
        assert!(broken(&["platforms", w, "size"], 0.into()).is_err());
        assert!(broken(&["platforms", w, "sha256"], "ABC".into()).is_err());
        assert!(broken(&["platforms"], serde_json::json!({ "linux-x86_64": base["platforms"][w] })).is_err());
        assert!(broken(&["platforms"], serde_json::json!({})).is_err());
        assert!(parse_manifest(b"not json").is_err());
    }

    // --- signatures

    #[test]
    fn a_node_signature_verifies() {
        verify_signature(NODE_MANIFEST.as_bytes(), NODE_SIG, &[NODE_KEY]).unwrap();
        let m = verified_manifest(NODE_MANIFEST.as_bytes(), NODE_SIG, &[UPDATE_PUBLIC_KEY, NODE_KEY]).unwrap();
        assert_eq!(m.version, "9.9.9");
    }

    #[test]
    fn a_tampered_manifest_fails() {
        let tampered = NODE_MANIFEST.replace("9.9.9", "9.9.8");
        assert!(verify_signature(tampered.as_bytes(), NODE_SIG, &[NODE_KEY]).is_err());
        let one_byte = NODE_MANIFEST.replace("\"size\": 3", "\"size\": 4");
        assert!(verified_manifest(one_byte.as_bytes(), NODE_SIG, &[NODE_KEY]).is_err());
        // A flipped signature byte.
        let (pair, public) = key(1);
        let sig = sign(&pair, NODE_MANIFEST.as_bytes());
        let mut lines: Vec<String> = sig.lines().map(str::to_string).collect();
        let mut raw = base64::engine::general_purpose::STANDARD
            .decode(lines[2].trim_start_matches("sig "))
            .unwrap();
        raw[10] ^= 1;
        lines[2] = format!("sig {}", base64::engine::general_purpose::STANDARD.encode(raw));
        assert!(verify_signature(NODE_MANIFEST.as_bytes(), &lines.join("\n"), &[&public]).is_err());
        verify_signature(NODE_MANIFEST.as_bytes(), &sig, &[&public]).unwrap();
    }

    #[test]
    fn a_wrong_key_fails() {
        // Signed by Node's key, checked against Sigil's release key only.
        let error = verify_signature(NODE_MANIFEST.as_bytes(), NODE_SIG, TRUSTED_KEYS).unwrap_err();
        assert!(error.contains("does not trust"));
        // A signature whose key line claims a trusted id but was made by another key.
        let (other, _) = key(2);
        let (_, trusted) = key(3);
        let forged = sign(&other, NODE_MANIFEST.as_bytes()).replace(
            &key_id(other.public_key().as_ref()),
            &key_id(&base64::engine::general_purpose::STANDARD.decode(&trusted).unwrap()),
        );
        assert!(verify_signature(NODE_MANIFEST.as_bytes(), &forged, &[&trusted]).is_err());
        assert!(verify_signature(NODE_MANIFEST.as_bytes(), "", &[&trusted]).is_err());
        assert!(verify_signature(NODE_MANIFEST.as_bytes(), "sigil-update-signature v1\nkey x\nsig !!", &[&trusted]).is_err());
    }

    #[test]
    fn the_release_key_is_well_formed() {
        let raw = base64::engine::general_purpose::STANDARD.decode(UPDATE_PUBLIC_KEY).unwrap();
        assert_eq!(raw.len(), 32);
        assert_eq!(key_id(&raw), "0cd4b3b09aeb594e");
    }

    /// Signed by `scripts/sign-release.sh` with the real release key: the compiled public key
    /// matches the private key on the maintainer's machine.
    #[test]
    fn the_release_key_verifies_a_real_signature() {
        let manifest = "{\n  \"format\": 1,\n  \"product\": \"sigil\",\n  \"version\": \"0.0.1\",\n  \"platforms\": {\n    \"windows-x86_64\": {\n      \"asset\": \"Sigil-0.0.1-windows-x64.zip\",\n      \"size\": 8,\n      \"sha256\": \"9a53aa7be7f38b1807449c3607863ece9be4ff9305b68875684185185cb07a4b\",\n      \"assets\": true\n    }\n  },\n  \"published\": \"2026-11-17T00:00:00Z\"\n}\n";
        let sig = "sigil-update-signature v1\nkey 0cd4b3b09aeb594e\nsig iKNKbitzaG9oUGZnM4IGM+ZdILdwCtbE5ydeXkjrhOAFgJFUWBqPCH3Btnsts8JTWt7hnqhCk7DCSNqQJeexAQ==\n";
        let m = verified_manifest(manifest.as_bytes(), sig, TRUSTED_KEYS).unwrap();
        assert_eq!(m.version, "0.0.1");
        // Old, so a check never offers it: the downgrade rule holds even for a valid signature.
        assert!(!is_newer(&m.version, VERSION));
    }

    // --- layout and plans

    #[test]
    fn layouts_come_from_the_executable_path() {
        let w = detect("windows", Path::new("D:/Games/Sigil/Sigil.exe")).unwrap();
        assert_eq!(w.kind, Kind::Windows);
        assert_eq!(w.sidecar(&w.target, "old"), Path::new("D:/Games/Sigil/Sigil.exe.old"));
        assert!(detect("windows", Path::new("D:/rust/target/debug/dah-desktop.exe")).is_err());
        let m = detect("macos", Path::new("/Applications/Sigil.app/Contents/MacOS/dah-desktop")).unwrap();
        assert_eq!(m.target, Path::new("/Applications/Sigil.app"));
        assert_eq!(m.sidecar(&m.target, "old"), Path::new("/Applications/.Sigil.app.old"));
        assert!(detect("macos", Path::new("/private/var/folders/x/AppTranslocation/ABC/d/Sigil.app/Contents/MacOS/dah-desktop")).is_err());
        assert!(detect("macos", Path::new("/Users/me/dl/rust/target/debug/dah-desktop")).is_err());
        assert!(detect("linux", Path::new("/usr/bin/sigil")).is_err());
    }

    fn windows_install(dir: &Path) {
        fs::write(dir.join("Sigil.exe"), "old exe").unwrap();
        fs::create_dir_all(dir.join("assets/locker")).unwrap();
        fs::write(dir.join("assets/locker/a.png"), "old art").unwrap();
        fs::write(dir.join("README.txt"), "old readme").unwrap();
    }

    fn windows_tree(root: &Path) {
        fs::create_dir_all(root.join("assets/locker")).unwrap();
        fs::write(root.join("Sigil.exe"), "new exe").unwrap();
        fs::write(root.join("assets/locker/a.png"), "new art").unwrap();
        fs::write(root.join("README.txt"), "new readme").unwrap();
    }

    #[test]
    fn windows_swap_then_rollback_then_cleanup() {
        let dir = temp("win-swap");
        windows_install(&dir);
        let layout = windows_layout(&dir);
        let new_root = layout.new_root();
        windows_tree(&new_root);
        let plan = apply_plan(&layout, &new_root, true, true, &["README.txt".to_string()]);
        run(&plan).unwrap();
        assert_eq!(read(dir.join("Sigil.exe")), "new exe");
        assert_eq!(read(dir.join("Sigil.exe.old")), "old exe");
        assert_eq!(read(dir.join("assets/locker/a.png")), "new art");
        assert_eq!(read(dir.join("assets.old/locker/a.png")), "old art");
        assert_eq!(read(dir.join("README.txt")), "new readme");
        assert!(!new_root.exists());

        let applied = Applied {
            from: "0.1.0".into(),
            to: "0.2.0".into(),
            assets_new: true,
            assets_backup: true,
            launches: 1,
            confirmed: false,
        };
        run(&rollback_plan(&layout, &applied)).unwrap();
        assert_eq!(read(dir.join("Sigil.exe")), "old exe");
        assert_eq!(read(dir.join("assets/locker/a.png")), "old art");
        assert!(!dir.join("Sigil.exe.old").exists());
        assert!(!dir.join("Sigil.exe.failed").exists(), "not in use here, so it is removed");

        run(&cleanup_plan(&layout)).unwrap();
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["README.txt", "Sigil.exe", "assets"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_swap_undoes_its_renames() {
        let dir = temp("win-undo");
        windows_install(&dir);
        let layout = windows_layout(&dir);
        let new_root = layout.new_root();
        // The new tree lacks Sigil.exe, so the third step fails after the second moved the exe.
        fs::create_dir_all(&new_root).unwrap();
        let error = run(&apply_plan(&layout, &new_root, false, true, &[])).unwrap_err();
        assert!(error.contains("Could not move"));
        assert_eq!(read(dir.join("Sigil.exe")), "old exe");
        assert!(!dir.join("Sigil.exe.old").exists());

        // An assets swap that fails puts the exe and the art back.
        windows_tree(&new_root);
        fs::remove_dir_all(new_root.join("assets")).unwrap();
        assert!(run(&apply_plan(&layout, &new_root, true, true, &[])).is_err());
        assert_eq!(read(dir.join("Sigil.exe")), "old exe");
        assert_eq!(read(dir.join("assets/locker/a.png")), "old art");
        assert!(!dir.join("assets.old").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_previous_old_copy_is_replaced() {
        let dir = temp("win-stale");
        windows_install(&dir);
        fs::write(dir.join("Sigil.exe.old"), "older exe").unwrap();
        let layout = windows_layout(&dir);
        windows_tree(&layout.new_root());
        run(&apply_plan(&layout, &layout.new_root(), false, true, &[])).unwrap();
        assert_eq!(read(dir.join("Sigil.exe.old")), "old exe");
        assert_eq!(read(dir.join("assets/locker/a.png")), "old art", "assets stay when not asked");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mac_bundle_swap_and_rollback() {
        let dir = temp("mac-swap");
        let bundle = dir.join("Sigil.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/MacOS/dah-desktop"), "old").unwrap();
        let layout = Layout { kind: Kind::MacBundle, dir: dir.clone(), target: bundle.clone(), assets: None };
        let incoming = layout.new_root().join("Sigil.app/Contents/MacOS");
        fs::create_dir_all(&incoming).unwrap();
        fs::write(incoming.join("dah-desktop"), "new").unwrap();
        run(&apply_plan(&layout, &layout.new_root(), true, false, &[])).unwrap();
        assert_eq!(read(bundle.join("Contents/MacOS/dah-desktop")), "new");
        assert_eq!(read(dir.join(".Sigil.app.old/Contents/MacOS/dah-desktop")), "old");
        let applied = Applied { from: "0.1.0".into(), to: "0.2.0".into(), assets_new: false, assets_backup: false, launches: 1, confirmed: false };
        run(&rollback_plan(&layout, &applied)).unwrap();
        assert_eq!(read(bundle.join("Contents/MacOS/dah-desktop")), "old");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn start_up_decisions() {
        let a = |launches, confirmed| Applied { from: "0.1.0".into(), to: "0.2.0".into(), assets_new: false, assets_backup: false, launches, confirmed };
        let p = |v: &str| Staged { version: v.into(), tree: PathBuf::new(), assets: false };
        assert_eq!(decide(None, None, "0.1.0"), StartAction::Nothing);
        assert_eq!(decide(Some(&a(0, false)), None, "0.2.0"), StartAction::CountLaunch);
        assert_eq!(decide(Some(&a(1, false)), None, "0.2.0"), StartAction::Rollback);
        assert_eq!(decide(Some(&a(1, true)), None, "0.2.0"), StartAction::Cleanup);
        assert_eq!(decide(Some(&a(1, false)), None, "0.1.0"), StartAction::Forget);
        assert_eq!(decide(None, Some(&p("0.2.0")), "0.1.0"), StartAction::Apply);
        assert_eq!(decide(None, Some(&p("0.1.0")), "0.1.0"), StartAction::DropPending);
        assert_eq!(decide(None, Some(&p("0.0.9")), "0.1.0"), StartAction::DropPending, "no downgrade on start");
    }

    #[test]
    fn a_launch_that_never_draws_rolls_back() {
        let root = temp("start-rollback");
        let install_dir = root.join("install");
        let data = root.join("updates");
        fs::create_dir_all(&install_dir).unwrap();
        windows_install(&install_dir);
        let layout = windows_layout(&install_dir);
        let tree = root.join("tree");
        windows_tree(&tree);
        let staged = Staged { version: "0.2.0".into(), tree: tree.clone(), assets: true };
        write_json(&data.join("pending.json"), &pending_json(&staged, "windows-x86_64")).unwrap();

        // Next launch of 0.1.0 installs the staged 0.2.0.
        assert!(start_with(&layout, &data, "0.1.0", "windows-x86_64", false).is_none());
        assert_eq!(read(install_dir.join("Sigil.exe")), "new exe");
        assert!(!data.join("pending.json").exists());
        assert!(read_applied(&data).is_some_and(|a| a.launches == 0 && !a.confirmed));
        // First launch of 0.2.0 is counted; it dies before a frame. The second rolls back.
        start_with(&layout, &data, "0.2.0", "windows-x86_64", false);
        assert_eq!(read_applied(&data).unwrap().launches, 1);
        start_with(&layout, &data, "0.2.0", "windows-x86_64", false);
        assert_eq!(read(install_dir.join("Sigil.exe")), "old exe");
        assert_eq!(read(install_dir.join("assets/locker/a.png")), "old art");
        assert!(read_applied(&data).is_none());
        assert!(data.join("rolled-back.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_started_update_keeps_and_tidies() {
        let root = temp("start-confirm");
        let install_dir = root.join("install");
        let data = root.join("updates");
        fs::create_dir_all(&install_dir).unwrap();
        windows_install(&install_dir);
        let layout = windows_layout(&install_dir);
        let tree = root.join("tree");
        windows_tree(&tree);
        let staged = Staged { version: "0.2.0".into(), tree, assets: true };
        let applied = install(&layout, &staged, "0.1.0").unwrap();
        write_json(&data.join("applied.json"), &applied_json(&applied)).unwrap();
        start_with(&layout, &data, "0.2.0", "windows-x86_64", false);
        confirm_started(&data, "0.2.0");
        assert!(install_dir.join("Sigil.exe.old").exists(), "kept until the next start");
        start_with(&layout, &data, "0.2.0", "windows-x86_64", false);
        assert!(!install_dir.join("Sigil.exe.old").exists());
        assert!(!install_dir.join("assets.old").exists());
        assert_eq!(read(install_dir.join("Sigil.exe")), "new exe");
        assert!(read_applied(&data).is_none());
        // Installing an older staged version is refused.
        let old = Staged { version: "0.1.5".into(), tree: root.join("tree"), assets: false };
        assert!(install(&layout, &old, "0.2.0").is_err());
        let _ = fs::remove_dir_all(root);
    }

    // --- archives

    #[test]
    fn unpack_drops_the_top_folder_and_refuses_escapes() {
        let root = temp("unpack");
        let zip = root.join("a.zip");
        fs::write(&zip, zip_of(&[("Sigil-0.2.0-windows-x64/Sigil.exe", b"exe"), ("Sigil-0.2.0-windows-x64/assets/x.png", b"png")])).unwrap();
        unpack(&zip, &root.join("t")).unwrap();
        assert_eq!(read(root.join("t/Sigil.exe")), "exe");
        assert!(check_tree(&root.join("t"), "windows-x86_64", "0.2.0").is_ok());

        fs::write(&zip, zip_of(&[("Sigil.app/Contents/Info.plist", b"<key>CFBundleShortVersionString</key><string>0.2.0</string>"), ("Sigil.app/Contents/MacOS/dah-desktop", b"bin")])).unwrap();
        unpack(&zip, &root.join("m")).unwrap();
        assert!(root.join("m/Sigil.app/Contents/MacOS/dah-desktop").is_file());
        assert!(check_tree(&root.join("m"), "macos-aarch64", "0.2.0").is_ok());
        assert!(check_tree(&root.join("m"), "macos-aarch64", "0.3.0").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join("m/Sigil.app/Contents/MacOS/dah-desktop")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the program stays executable");
        }

        fs::write(&zip, zip_of(&[("../evil.txt", b"x")])).unwrap();
        assert!(unpack(&zip, &root.join("e")).is_err());
        assert!(!root.join("evil.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    // --- end to end against a local server

    fn release(base: &str, tag: &str, names: &[&str]) -> Vec<u8> {
        let assets: Vec<serde_json::Value> = names
            .iter()
            .map(|n| serde_json::json!({ "name": n, "browser_download_url": format!("{base}/dl/{n}") }))
            .collect();
        serde_json::to_vec(&serde_json::json!({
            "tag_name": tag,
            "html_url": format!("{base}/release/{tag}"),
            "body": "## Fixes\n\n* **Faster** start\n",
            "draft": false,
            "assets": assets,
        }))
        .unwrap()
    }

    /// A fake release on a local server: check, download, verify, stage, install, roll back.
    #[test]
    fn end_to_end_against_a_local_release() {
        let (pair, public) = key(9);
        let platform = "windows-x86_64";
        let asset = "Sigil-0.2.0-windows-x64.zip";
        let zip = zip_of(&[
            ("Sigil-0.2.0-windows-x64/Sigil.exe", b"new exe"),
            ("Sigil-0.2.0-windows-x64/assets/locker/a.png", b"new art"),
            ("Sigil-0.2.0-windows-x64/README.txt", b"new readme"),
        ]);
        let manifest = manifest_for("0.2.0", platform, asset, &zip, true);
        let signature = sign(&pair, &manifest);
        let routes = |base: &str, zip: Vec<u8>, sig: String| {
            vec![
                ("/repos/o/r/releases/latest".to_string(), release(base, "v0.2.0", &[asset, MANIFEST_ASSET, SIGNATURE_ASSET])),
                (format!("/dl/{MANIFEST_ASSET}"), manifest.clone()),
                (format!("/dl/{SIGNATURE_ASSET}"), sig.into_bytes()),
                (format!("/dl/{asset}"), zip),
            ]
        };
        // Two servers: one honest, one serving a different zip under the same manifest.
        let honest = serve_with(|base| routes(base, zip.clone(), signature.clone()));
        let mut evil_zip = zip.clone();
        let last = evil_zip.len() - 1;
        evil_zip[last] ^= 0xff;
        let evil = serve_with(|base| routes(base, evil_zip.clone(), signature.clone()));

        // The check verifies the manifest and offers the download.
        let offer = check(&honest, "o/r", "0.1.0", false, &[&public], platform).unwrap().unwrap();
        assert_eq!(offer.version, "0.2.0");
        assert!(offer.notes.contains("Faster"));
        let download_offer = offer.install.clone().unwrap();
        // The same release is not offered to a newer build: no downgrade.
        let newer = check(&honest, "o/r", "0.3.0", false, &[&public], platform).unwrap().unwrap();
        assert!(newer.install.is_err());
        // A key Sigil does not trust: no install.
        let (_, stranger) = key(10);
        assert!(check(&honest, "o/r", "0.1.0", false, &[&stranger], platform).unwrap().unwrap().install.is_err());
        // No build for this platform.
        assert!(check(&honest, "o/r", "0.1.0", false, &[&public], "macos-aarch64").unwrap().unwrap().install.is_err());

        // Download, verify and stage.
        let root = temp("e2e");
        let data = root.join("updates");
        let progress = Progress::default();
        let staged = download(&download_offer, &data, "0.1.0", &[&public], platform, &progress, None).unwrap();
        assert_eq!(progress.done.load(Ordering::Relaxed), zip.len() as u64);
        assert_eq!(read(staged.tree.join("Sigil.exe")), "new exe");
        assert!(data.join("pending.json").exists());

        // A tampered zip is refused and nothing is staged for it.
        let evil_offer = check(&evil, "o/r", "0.1.0", false, &[&public], platform).unwrap().unwrap().install.unwrap();
        let evil_data = root.join("evil-updates");
        let error = download(&evil_offer, &evil_data, "0.1.0", &[&public], platform, &Progress::default(), None).unwrap_err();
        assert!(error.contains("hash"), "{error}");
        assert!(!evil_data.join("pending.json").exists());

        // Install on a fake Windows folder through the start-up path, then confirm.
        let install_dir = root.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        windows_install(&install_dir);
        let layout = windows_layout(&install_dir);
        assert!(start_with(&layout, &data, "0.1.0", platform, false).is_none());
        assert_eq!(read(install_dir.join("Sigil.exe")), "new exe");
        assert_eq!(read(install_dir.join("assets/locker/a.png")), "new art");
        assert_eq!(read(install_dir.join("README.txt")), "new readme");
        assert_eq!(read(install_dir.join("Sigil.exe.old")), "old exe");
        let _ = fs::remove_dir_all(root);
    }

    /// Serve the routes built for this server's address over plain HTTP until the process ends.
    fn serve_with(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&base);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", b"{}".to_vec()),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        base
    }

    #[test]
    fn only_release_download_urls_are_fetched() {
        assert!(allowed_url("https://github.com/o/r/releases/download/v1/a.zip", GITHUB_API, "o/r"));
        assert!(!allowed_url("https://github.com/x/r/releases/download/v1/a.zip", GITHUB_API, "o/r"));
        assert!(!allowed_url("https://evil.example/o/r/releases/download/v1/a.zip", GITHUB_API, "o/r"));
        assert!(!allowed_url("http://127.0.0.1:9/dl/a.zip", GITHUB_API, "o/r"));
        assert!(allowed_url("http://127.0.0.1:9/dl/a.zip", "http://127.0.0.1:9", "o/r"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_builds_honour_the_release_url_override() {
        // SAFETY: no other test reads this variable.
        unsafe { std::env::set_var("SIGIL_UPDATE_API", "http://127.0.0.1:1/") };
        assert_eq!(api_base(), "http://127.0.0.1:1");
        unsafe { std::env::remove_var("SIGIL_UPDATE_API") };
        assert_eq!(api_base(), GITHUB_API);
    }

    #[test]
    fn notes_lose_their_markdown() {
        assert_eq!(plain_notes("## Fixes\n\n* **Faster** start\n\n\n- `x` fixed\n", 10), "Fixes\n- Faster start\n- x fixed");
        assert_eq!(plain_notes("## New\n\n- a\n\n## Fixes\n\n- b", 10), "New\n- a\n\nFixes\n- b");
        assert_eq!(plain_notes("a\nb\nc", 2), "a\nb");
    }
}
