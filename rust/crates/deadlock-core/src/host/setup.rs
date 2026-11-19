//! Setup: prerequisites, SteamCMD, the server files' build and the pinned Deadworks release.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{SERVER_APP_ID, require_approval, require_windows};
use crate::error::{Error, Result};
use crate::install::{ExtractLimits, ToolPaths, extract_archive, safe_entry_path};
use crate::util::{hidden_command, now_iso, write_atomic};

/// Valve's SteamCMD for Windows. Unchanged since 2013 (Last-Modified 2013-12-06); the bootstrapper
/// updates itself on first run, from Steam.
pub const STEAMCMD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";
pub const STEAMCMD_SIZE: u64 = 774_825;
pub const STEAMCMD_SHA256: &str = "7669b170dee42db8ee2273775ed7dfb2d173bdba1b849f70d2c7b379290bce13";
const STEAMCMD_HOSTS: &[&str] = &["steamcdn-a.akamaihd.net"];

/// The pinned Deadworks release (latest on 2026-11-19, published 2026-11-06). Bump deliberately:
/// version, URL, size and the SHA-256 computed from the downloaded file.
pub const DEADWORKS_VERSION: &str = "v0.6.4";
pub const DEADWORKS_URL: &str =
    "https://github.com/Deadworks-net/deadworks/releases/download/v0.6.4/deadworks-v0.6.4.zip";
pub const DEADWORKS_SIZE: u64 = 5_500_660;
pub const DEADWORKS_SHA256: &str = "1e86b8698b4aa5c96e12e35063f539f4e4c0e9d452238774681cead9059cfc73";
/// GitHub answers a release download with a redirect to its asset host.
const DEADWORKS_HOSTS: &[&str] = &[
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// The record Sigil keeps in the server folder of what it extracted, so an update can remove
/// files the new release no longer ships.
pub const DEADWORKS_MARKER: &str = "sigil-deadworks.json";

/// The Deadworks guide's figure for the server files.
pub const REQUIRED_FREE_BYTES: u64 = 35 * 1024 * 1024 * 1024;

const STEAM_INF: &str = "game/citadel/steam.inf";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const USER_AGENT: &str = "sigil-host";
/// Folders an extract never writes into: the player's plugins, their off switch and settings.
const KEEP_PREFIXES: &[&str] = &[
    "game/bin/win64/managed/plugins/",
    "game/bin/win64/managed/plugins-off/",
    "game/bin/win64/configs/",
];

// ---------------------------------------------------------------------------------------------
// Prerequisites
// ---------------------------------------------------------------------------------------------

/// The .NET runtimes `dotnet --list-runtimes` reported.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DotnetStatus {
    /// `Microsoft.NETCore.App` versions, as printed.
    pub versions: Vec<String>,
    /// A 10.x runtime is present.
    pub ok: bool,
}

/// What the Deadworks marker records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadworksInstall {
    /// The release tag, or None when `deadworks.exe` is there but Sigil did not install it.
    pub version: Option<String>,
    /// The version is [`DEADWORKS_VERSION`].
    pub is_pinned: bool,
}

/// The setup checklist, one field per row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prereqs {
    pub dotnet: DotnetStatus,
    /// Free bytes on the server folder's drive, when it could be read.
    pub free_bytes: Option<u64>,
    /// At least [`REQUIRED_FREE_BYTES`] free, or the server files are already there.
    pub enough_disk: bool,
    /// `steamcmd.exe`, when present.
    pub steamcmd: Option<PathBuf>,
    /// `ClientVersion` from the server's `game/citadel/steam.inf`; None means no server files.
    pub server_build: Option<String>,
    pub deadworks: Option<DeadworksInstall>,
}

impl Prereqs {
    /// Every row is ready, so Start can be offered.
    pub fn ready(&self) -> bool {
        self.dotnet.ok && self.server_build.is_some() && self.deadworks.is_some()
    }
}

/// The checklist for `server_dir`. Reads only; runs `dotnet --list-runtimes` hidden.
pub fn prerequisites(server_dir: &Path, steamcmd_dir: Option<&Path>) -> Result<Prereqs> {
    require_windows()?;
    let server_build = read_build(server_dir);
    let free = free_bytes(server_dir);
    Ok(Prereqs {
        dotnet: dotnet_status(),
        free_bytes: free,
        enough_disk: server_build.is_some() || free.is_some_and(|f| f >= REQUIRED_FREE_BYTES),
        steamcmd: steamcmd_dir.map(steamcmd_exe).filter(|p| p.is_file()),
        server_build,
        deadworks: deadworks_installed(server_dir),
    })
}

/// Parse `dotnet --list-runtimes`: lines like
/// `Microsoft.NETCore.App 10.0.0 [C:\Program Files\dotnet\shared\Microsoft.NETCore.App]`.
pub fn parse_dotnet_runtimes(output: &str) -> DotnetStatus {
    let versions: Vec<String> = output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next()? == "Microsoft.NETCore.App").then(|| parts.next().map(str::to_string))?
        })
        .collect();
    let ok = versions
        .iter()
        .any(|v| v.split(['.', '-']).next() == Some("10"));
    DotnetStatus { versions, ok }
}

/// Run `dotnet --list-runtimes` (no window). Not found reads as no runtimes.
pub fn dotnet_status() -> DotnetStatus {
    let mut candidates = vec![PathBuf::from("dotnet")];
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("dotnet").join("dotnet.exe"));
    }
    for program in candidates {
        if let Ok(output) = hidden_command(&program).arg("--list-runtimes").output()
            && output.status.success()
        {
            return parse_dotnet_runtimes(&String::from_utf8_lossy(&output.stdout));
        }
    }
    DotnetStatus::default()
}

/// Free bytes on the drive that holds `path` (or its nearest existing parent). Windows only.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    free_bytes_at(probe)
}

#[cfg(windows)]
fn free_bytes_at(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available: u64 = 0;
    // SAFETY: `wide` is NUL terminated and outlives the call; the out pointers are valid or null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut())
    };
    (ok != 0).then_some(available)
}

#[cfg(not(windows))]
fn free_bytes_at(_path: &Path) -> Option<u64> {
    None
}

/// `ClientVersion` from a `steam.inf` text (`ClientVersion=6123`).
pub fn parse_steam_inf(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("ClientVersion=")?.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The build of a Deadlock folder (the server's or the player's game root).
pub fn read_build(root: &Path) -> Option<String> {
    parse_steam_inf(&fs::read_to_string(root.join(STEAM_INF)).ok()?)
}

/// Refuse a server folder inside the player's Deadlock install or a Steam library, or one that
/// holds that install. The server always gets a folder of its own.
pub fn check_server_dir(server_dir: &Path, player_game_root: Option<&Path>) -> Result<()> {
    if !server_dir.is_absolute() {
        return Err(Error::msg("Pick a full folder path for the server."));
    }
    if server_dir.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Error::msg("The server folder path may not contain \"..\"."));
    }
    let in_library = server_dir.components().any(|c| {
        matches!(c, Component::Normal(name) if name.to_string_lossy().eq_ignore_ascii_case("steamapps"))
    });
    if in_library {
        return Err(Error::msg(
            "Pick a folder outside your Steam library. The server gets its own copy of the game files.",
        ));
    }
    if let Some(game) = player_game_root
        && (server_dir.starts_with(game) || game.starts_with(server_dir))
    {
        return Err(Error::msg(
            "Pick a folder outside your Deadlock install. The server gets its own copy of the game files.",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Build compare
// ---------------------------------------------------------------------------------------------

/// Whether the server files trail the player's game after a patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAdvice {
    pub server_build: Option<String>,
    pub game_build: Option<String>,
    /// Run the SteamCMD update.
    pub needs_update: bool,
    /// The installed Deadworks is not the pinned release.
    pub deadworks_behind: bool,
    pub message: String,
}

/// Compare the server's build with the player's game build.
pub fn update_advice(server_dir: &Path, game_root: Option<&Path>) -> UpdateAdvice {
    let server_build = read_build(server_dir);
    let game_build = game_root.and_then(read_build);
    let deadworks_behind = deadworks_installed(server_dir).is_some_and(|d| !d.is_pinned);
    let (needs_update, message) = match (&server_build, &game_build) {
        (None, _) => (true, "No server files yet. Run the SteamCMD install.".to_string()),
        (Some(server), Some(game)) if server != game => {
            let older = matches!((server.parse::<u64>(), game.parse::<u64>()), (Ok(s), Ok(g)) if s < g);
            if older {
                (true, format!("Server files are older than your game (build {server}, game {game}). Run the SteamCMD update."))
            } else {
                (false, format!("Server build {server} differs from your game build {game}. Update the game, or the server if Steam has a newer build."))
            }
        }
        (Some(server), Some(_)) => (false, format!("Server files match your game (build {server}).")),
        (Some(server), None) => (false, format!("Server build {server}. Your game build is unknown.")),
    };
    let message = if deadworks_behind {
        format!("{message} Deadworks {DEADWORKS_VERSION} is available.")
    } else {
        message
    };
    UpdateAdvice { server_build, game_build, needs_update, deadworks_behind, message }
}

// ---------------------------------------------------------------------------------------------
// SteamCMD
// ---------------------------------------------------------------------------------------------

/// `<dir>/steamcmd.exe`.
pub fn steamcmd_exe(dir: &Path) -> PathBuf {
    dir.join("steamcmd.exe")
}

/// Download Valve's pinned SteamCMD zip into `dir` and extract `steamcmd.exe` there.
pub fn install_steamcmd(dir: &Path, approve: bool) -> Result<PathBuf> {
    require_approval(approve, "Installing SteamCMD")?;
    require_windows()?;
    fs::create_dir_all(dir)?;
    let zip = dir.join("steamcmd.zip.download");
    fetch_pinned(STEAMCMD_URL, STEAMCMD_HOSTS, STEAMCMD_SIZE, STEAMCMD_SHA256, &zip)?;
    let staging = dir.join(".sigil-extract");
    let result = (|| {
        let report = extract_archive(&zip, &staging, &ExtractLimits::default(), &ToolPaths::default())?;
        if !report.files.iter().any(|f| f == "steamcmd.exe") {
            return Err(Error::msg("The SteamCMD zip did not contain steamcmd.exe."));
        }
        let target = steamcmd_exe(dir);
        fs::copy(staging.join("steamcmd.exe"), &target)?;
        Ok(target)
    })();
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&zip);
    result
}

/// A Steam account name SteamCMD can take on its command line: 1 to 64 ASCII letters, digits
/// or underscores. Nothing else rides along with the login.
pub fn valid_steam_user(user: &str) -> bool {
    !user.is_empty() && user.len() <= 64 && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// SteamCMD's arguments: install or update app 1422450 into `server_dir`, then quit. Only the
/// account name; SteamCMD asks for the password and Steam Guard code in its own window.
pub fn steamcmd_args(server_dir: &Path, steam_user: &str) -> Result<Vec<OsString>> {
    if !valid_steam_user(steam_user) {
        return Err(Error::msg(
            "Enter your Steam account name (letters, digits and underscores). SteamCMD asks for the password in its own window.",
        ));
    }
    let mut args: Vec<OsString> = vec!["+force_install_dir".into(), server_dir.as_os_str().to_owned()];
    for arg in ["+login", steam_user, "+app_update", &SERVER_APP_ID.to_string(), "validate", "+quit"] {
        args.push(arg.into());
    }
    Ok(args)
}

/// The SteamCMD command, set to open in its OWN visible console (`CREATE_NEW_CONSOLE`) so the
/// player signs in there. Its input and output stay on that console: Sigil never sees the
/// password prompt. Follow progress with [`steamcmd_log_lines`].
pub fn steamcmd_update_command(steamcmd: &Path, server_dir: &Path, steam_user: &str) -> Result<Command> {
    check_server_dir(server_dir, None)?;
    let args = steamcmd_args(server_dir, steam_user)?;
    let mut command = Command::new(steamcmd);
    command.args(args);
    if let Some(dir) = steamcmd.parent() {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    Ok(command)
}

/// Start [`steamcmd_update_command`]. The app calls this when the player presses Install or Update.
pub fn start_steamcmd_update(steamcmd: &Path, server_dir: &Path, steam_user: &str, approve: bool) -> Result<Child> {
    require_approval(approve, "Running SteamCMD")?;
    require_windows()?;
    if !steamcmd.is_file() {
        return Err(Error::msg("SteamCMD is not installed yet."));
    }
    fs::create_dir_all(server_dir)?;
    Ok(steamcmd_update_command(steamcmd, server_dir, steam_user)?.spawn()?)
}

/// One line of SteamCMD's log, read for the status row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SteamCmdEvent {
    /// `Update state (0x61) downloading, progress: 45.20 (123 / 456)`.
    Progress { percent: f32, state: String },
    /// `Success! App '1422450' fully installed.`
    Installed,
    /// `ERROR! ...` or `FAILED ...`.
    Failed { message: String },
}

/// Read one SteamCMD output line.
pub fn parse_steamcmd_line(line: &str) -> Option<SteamCmdEvent> {
    let line = line.trim();
    if line.starts_with("Success!") && line.contains("fully installed") {
        return Some(SteamCmdEvent::Installed);
    }
    if line.starts_with("ERROR!") || line.starts_with("FAILED") {
        return Some(SteamCmdEvent::Failed { message: line.to_string() });
    }
    let rest = line.strip_prefix("Update state (")?;
    let (_, rest) = rest.split_once(") ")?;
    let (state, rest) = rest.split_once(", progress: ")?;
    let percent = rest.split_whitespace().next()?.parse::<f32>().ok()?;
    Some(SteamCmdEvent::Progress { percent, state: state.to_string() })
}

/// The last `max` lines of SteamCMD's own log files (`logs/console_log.txt`, then
/// `logs/content_log.txt`) when they exist. Any line that mentions a password is left out.
pub fn steamcmd_log_lines(steamcmd_dir: &Path, max: usize) -> Vec<String> {
    for name in ["console_log.txt", "content_log.txt"] {
        let Ok(bytes) = fs::read(steamcmd_dir.join("logs").join(name)) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = text
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.trim().is_empty() && !l.to_ascii_lowercase().contains("password"))
            .map(str::to_string)
            .collect();
        let start = lines.len().saturating_sub(max);
        return lines[start..].to_vec();
    }
    Vec::new()
}

// ---------------------------------------------------------------------------------------------
// Deadworks
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    version: String,
    sha256: String,
    installed_at: String,
    files: Vec<String>,
}

/// What an install wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadworksReport {
    pub version: String,
    pub files: Vec<String>,
    /// Files of the previous release the new one no longer ships, removed.
    pub removed: Vec<String>,
}

/// The installed Deadworks, if `deadworks.exe` is in place.
pub fn deadworks_installed(server_dir: &Path) -> Option<DeadworksInstall> {
    if !super::deadworks_exe(server_dir).is_file() {
        return None;
    }
    let version = fs::read_to_string(server_dir.join(DEADWORKS_MARKER))
        .ok()
        .and_then(|text| serde_json::from_str::<Marker>(&text).ok())
        .map(|m| m.version);
    let is_pinned = version.as_deref() == Some(DEADWORKS_VERSION);
    Some(DeadworksInstall { version, is_pinned })
}

/// Download the pinned Deadworks release from GitHub, check its size and SHA-256, and extract it
/// over `server_dir`. The player's plugins and configs are never touched.
pub fn install_deadworks(server_dir: &Path, approve: bool) -> Result<DeadworksReport> {
    require_approval(approve, "Installing Deadworks")?;
    require_windows()?;
    check_server_dir(server_dir, None)?;
    fs::create_dir_all(server_dir)?;
    let zip = server_dir.join(format!("deadworks-{DEADWORKS_VERSION}.zip.download"));
    fetch_pinned(DEADWORKS_URL, DEADWORKS_HOSTS, DEADWORKS_SIZE, DEADWORKS_SHA256, &zip)?;
    let result = install_deadworks_from_zip(&zip, server_dir, DEADWORKS_VERSION, approve);
    let _ = fs::remove_file(&zip);
    result
}

/// Extract a Deadworks zip over `server_dir`. Works on every system so it can be tested; the
/// app goes through [`install_deadworks`]. Every entry must sit under `game/`, and each path is
/// checked with the installer's [`safe_entry_path`]: an entry that would escape is refused before
/// anything lands in the server folder.
pub fn install_deadworks_from_zip(zip: &Path, server_dir: &Path, version: &str, approve: bool) -> Result<DeadworksReport> {
    require_approval(approve, "Installing Deadworks")?;
    let staging = server_dir.join(".sigil-extract");
    let result = (|| {
        let report = extract_archive(zip, &staging, &ExtractLimits::default(), &ToolPaths::default())?;
        let files: Vec<String> = report
            .files
            .iter()
            .filter(|f| !KEEP_PREFIXES.iter().any(|keep| f.starts_with(keep)))
            .cloned()
            .collect();
        if let Some(stray) = files.iter().find(|f| !f.starts_with("game/")) {
            return Err(Error::msg(format!(
                "The Deadworks zip has {stray} outside its game folder. Nothing was installed."
            )));
        }
        if !files.iter().any(|f| f == "game/bin/win64/deadworks.exe") {
            return Err(Error::msg("The Deadworks zip has no game/bin/win64/deadworks.exe. Nothing was installed."));
        }
        // Resolve every target first, so a bad name stops the install before any copy.
        let targets = files
            .iter()
            .map(|f| safe_entry_path(server_dir, f).map(|t| (f, t)))
            .collect::<Result<Vec<_>>>()?;
        for (relative, target) in &targets {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(staging.join(relative), target)?;
        }

        let previous = fs::read_to_string(server_dir.join(DEADWORKS_MARKER))
            .ok()
            .and_then(|text| serde_json::from_str::<Marker>(&text).ok());
        let now: BTreeSet<&String> = files.iter().collect();
        let mut removed = Vec::new();
        for old in previous.map(|m| m.files).unwrap_or_default() {
            if now.contains(&old) || KEEP_PREFIXES.iter().any(|keep| old.starts_with(keep)) {
                continue;
            }
            if let Ok(path) = safe_entry_path(server_dir, &old)
                && path.is_file()
                && fs::remove_file(&path).is_ok()
            {
                removed.push(old);
            }
        }

        let marker = Marker {
            version: version.to_string(),
            sha256: crate::servers::file_sha256(zip)?,
            installed_at: now_iso(),
            files: files.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&marker)?;
        bytes.push(b'\n');
        write_atomic(&server_dir.join(DEADWORKS_MARKER), &bytes)?;
        Ok(DeadworksReport { version: version.to_string(), files, removed })
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

// ---------------------------------------------------------------------------------------------
// Pinned download
// ---------------------------------------------------------------------------------------------

fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    Some(authority.split(':').next()?.to_ascii_lowercase())
}

/// Ok for an https URL on one of `hosts`.
pub(super) fn check_download_url(url: &str, hosts: &[&str]) -> Result<()> {
    match host_of(url) {
        Some(host) if hosts.contains(&host.as_str()) => Ok(()),
        _ => Err(Error::Network(format!("Refused {url}: not on this download's allowlist."))),
    }
}

/// GET `url` into `out`, following redirects only to `hosts`, and check the exact size and
/// SHA-256. A mismatch deletes the file.
fn fetch_pinned(url: &str, hosts: &[&str], size: u64, sha256: &str, out: &Path) -> Result<()> {
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut current = url.to_string();
    for _ in 0..5 {
        check_download_url(&current, hosts)?;
        let response = agent
            .get(&current)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| Error::Network(format!("Download from {current} failed: {e}")))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            current = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| Error::Network(format!("{current} redirected without an address.")))?;
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(Error::Network(format!("{current} answered HTTP {status}.")));
        }
        let mut reader = response.into_body().into_with_config().limit(size + 1).reader();
        let mut file = File::create(out)?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let n = reader
                .read(&mut buffer)
                .map_err(|e| Error::Network(format!("Download from {current} stopped: {e}")))?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > size {
                break;
            }
            hasher.update(&buffer[..n]);
            file.write_all(&buffer[..n])?;
        }
        file.sync_all()?;
        drop(file);
        let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if written != size || digest != sha256 {
            let _ = fs::remove_file(out);
            return Err(Error::msg(format!(
                "The download from {url} is not the pinned file (got {written} bytes, sha256 {digest}). Nothing was installed."
            )));
        }
        return Ok(());
    }
    Err(Error::Network(format!("{url} redirected too many times.")))
}
