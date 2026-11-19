//! The one file that calls `deadlock_core::host`. The checks, installs, starts and stops run on
//! workers; the poll makes only cheap reads here (`lines_since`, `exited`, `steamcmd_status`).
//! Every call that writes, downloads or starts a process passes `approve: true` because it only
//! runs after the player's press on the Host tab.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

use deadlock_core::host::{self as core, ServerHandle, SteamCmdEvent};
use deadlock_core::servers::invite::{Invite, InviteTarget};

use super::{Content, Firewall, Plugin, Setup};

pub use deadlock_core::host::ServerConfig as Config;

/// How long Stop waits for `quit` before it ends the process.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// The stock maps the picker always offers, by console name.
pub const STOCK_MAPS: &[&str] = &["dl_midtown", "dl_midtownpause", "street_test"];

/// What was saved last time, with Sigil's defaults filled in.
pub struct Saved {
    pub server_dir: PathBuf,
    pub steamcmd_dir: PathBuf,
    pub steam_user: String,
    pub config: Config,
}

fn host_dir() -> PathBuf {
    crate::data::data_dir().join("host")
}

pub fn load_saved() -> Saved {
    let settings = core::load_settings(&crate::data::data_dir()).unwrap_or_default();
    Saved {
        server_dir: settings
            .server_dir
            .unwrap_or_else(|| host_dir().join("server")),
        steamcmd_dir: settings
            .steamcmd_dir
            .unwrap_or_else(|| host_dir().join("steamcmd")),
        steam_user: settings.steam_user,
        config: settings.server,
    }
}

/// Save the folders, the account name and the settings. Settings that do not pass the core's
/// check keep the last saved ones.
pub fn save(
    server_dir: &Path,
    steamcmd_dir: &Path,
    steam_user: &str,
    config: Option<&Config>,
) -> Result<(), String> {
    let data = crate::data::data_dir();
    let mut settings = core::load_settings(&data).unwrap_or_default();
    settings.server_dir = Some(server_dir.to_path_buf());
    settings.steamcmd_dir = Some(steamcmd_dir.to_path_buf());
    settings.steam_user = steam_user.to_string();
    if let Some(config) = config.filter(|c| c.validate().is_ok()) {
        settings.server = config.clone();
    }
    core::save_settings(&data, &settings).map_err(|e| e.to_string())
}

/// A folder the player picked: refused inside their Deadlock install.
pub fn check_folder(server_dir: &Path, game: Option<&Path>) -> Result<(), String> {
    core::check_server_dir(server_dir, game).map_err(|e| e.to_string())
}

/// The setup checklist, the build comparison, the player's custom maps (from their enabled addon
/// slots) and the maps already in the server folder. Runs `dotnet` hidden.
pub fn check(
    server_dir: &Path,
    steamcmd_dir: &Path,
    game: Option<&Path>,
    addons: Option<&Path>,
) -> Result<Setup, String> {
    let prereqs = core::prerequisites(server_dir, Some(steamcmd_dir)).map_err(|e| e.to_string())?;
    let advice = core::update_advice(server_dir, game);
    let mut custom_maps: Vec<Content> = Vec::new();
    for map in addons
        .map(deadlock_core::maps::installed_maps)
        .unwrap_or_default()
    {
        if !STOCK_MAPS.contains(&map.name.as_str())
            && !custom_maps.iter().any(|c| c.name == map.name)
        {
            custom_maps.push(Content {
                name: map.name,
                vpk: map.vpk,
            });
        }
    }
    let server_maps = core::server_map_names(server_dir);
    Ok(Setup {
        dotnet: prereqs
            .dotnet
            .ok
            .then(|| {
                prereqs
                    .dotnet
                    .versions
                    .iter()
                    .rev()
                    .find(|v| v.starts_with("10."))
                    .cloned()
            })
            .map(|v| v.unwrap_or_else(|| "10".to_string())),
        free_bytes: prereqs.free_bytes,
        enough_disk: prereqs.enough_disk,
        steamcmd: prereqs.steamcmd.is_some(),
        needs_update: advice.needs_update && prereqs.server_build.is_some(),
        server_build: prereqs.server_build,
        game_build: advice.game_build,
        deadworks: prereqs
            .deadworks
            .as_ref()
            .map(|d| d.version.clone().unwrap_or_default()),
        deadworks_behind: prereqs.deadworks.as_ref().is_some_and(|d| !d.is_pinned),
        deadworks_pinned: core::DEADWORKS_VERSION.to_string(),
        custom_maps,
        server_maps,
    })
}

pub fn install_steamcmd(steamcmd_dir: &Path) -> Result<(), String> {
    core::install_steamcmd(steamcmd_dir, true)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Install the pinned Deadworks release. Returns its version.
pub fn install_deadworks(server_dir: &Path) -> Result<String, String> {
    core::install_deadworks(server_dir, true)
        .map(|report| report.version)
        .map_err(|e| e.to_string())
}

/// SteamCMD's console window while it is open.
pub struct SteamCmdRun {
    child: Option<Child>,
    dir: PathBuf,
}

impl SteamCmdRun {
    /// A window that is not there, for headless scenes.
    pub fn fixture() -> Self {
        SteamCmdRun {
            child: None,
            dir: PathBuf::new(),
        }
    }
}

pub struct SteamCmdStatus {
    pub line: Option<String>,
    pub closed: bool,
    /// The exit code when it closed with an error.
    pub failed: Option<String>,
}

/// Open SteamCMD in its own console window. The account name goes on its command line; the
/// password and Steam Guard code are typed into that window and never reach Sigil.
pub fn open_steamcmd(
    steamcmd_dir: &Path,
    server_dir: &Path,
    user: &str,
) -> Result<SteamCmdRun, String> {
    if !core::valid_steam_user(user) {
        return Err(crate::screens::settings::i18n::tr(
            "Type the Steam account name you sign in with.",
        )
        .to_string());
    }
    let exe = core::steamcmd_exe(steamcmd_dir);
    let child =
        core::start_steamcmd_update(&exe, server_dir, user, true).map_err(|e| e.to_string())?;
    Ok(SteamCmdRun {
        child: Some(child),
        dir: steamcmd_dir.to_path_buf(),
    })
}

/// Whether the window is still open, and the newest line of SteamCMD's own log.
pub fn steamcmd_status(run: &mut SteamCmdRun) -> SteamCmdStatus {
    let Some(child) = run.child.as_mut() else {
        return SteamCmdStatus {
            line: None,
            closed: false,
            failed: None,
        };
    };
    let exit = child.try_wait().ok().flatten();
    let lines = core::steamcmd_log_lines(&run.dir, 40);
    let line = lines
        .iter()
        .rev()
        .find_map(|l| core::parse_steamcmd_line(l).map(|event| (l, event)))
        .map(|(raw, event)| match event {
            SteamCmdEvent::Progress { percent, .. } => format!("{percent:.0}%  {}", raw.trim()),
            SteamCmdEvent::Installed => raw.trim().to_string(),
            SteamCmdEvent::Failed { message } => message,
        })
        .or_else(|| lines.last().map(|l| l.trim().to_string()));
    SteamCmdStatus {
        line,
        closed: exit.is_some(),
        failed: exit.filter(|s| !s.success()).map(|s| match s.code() {
            Some(code) => format!("exit code {code}"),
            None => String::new(),
        }),
    }
}

/// The running server, shared with the stop worker. Dropping the last one ends the process.
pub type Running = Arc<ServerHandle>;

/// Start the server. With custom content in the server folder this also starts the map
/// download server on the download port; both stop with [`stop`] or when the last handle drops.
pub fn start(server_dir: &Path, config: &Config) -> Result<Running, String> {
    core::start_server(server_dir, config, true)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

pub fn stop(server: &Running) -> Result<(), String> {
    server
        .stop(STOP_GRACE)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The port friends download custom maps from, while the download server runs.
pub fn serving_downloads(server: &Running) -> Option<u16> {
    server.fastdl_port.filter(|_| server.serving_downloads())
}

/// Type one console command into the server.
pub fn send(server: &Running, command: &str) -> Result<(), String> {
    server.send_command(command).map_err(|e| e.to_string())
}

/// Lines added since `seen` (a running total), and the new total.
pub fn lines_since(server: &Running, seen: u64) -> (Vec<String>, u64) {
    let total = server.line_total();
    if total == seen {
        return (Vec::new(), seen);
    }
    let lines = server.lines();
    let new = ((total.saturating_sub(seen)) as usize).min(lines.len());
    (lines[lines.len() - new..].to_vec(), total)
}

/// The exit line once the process has ended on its own.
pub fn exited(server: &Running) -> Option<String> {
    if server.is_running() {
        return None;
    }
    if core::unsupported_build(&server.lines()) {
        return Some(
            crate::screens::settings::i18n::tr("Deadworks does not support this Deadlock build yet. Wait for a Deadworks update, then install it here.")
                .to_string(),
        );
    }
    Some(
        server
            .exit_code()
            .map(|code| format!("exit code {code}"))
            .unwrap_or_default(),
    )
}

pub fn plugins(server_dir: &Path) -> Result<Vec<Plugin>, String> {
    core::list_plugins(server_dir)
        .map(|list| {
            list.into_iter()
                .map(|p| Plugin {
                    name: p.name,
                    on: p.enabled,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}

pub fn set_plugin(server_dir: &Path, name: &str, on: bool) -> Result<(), String> {
    core::set_plugin(server_dir, name, on, true)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn firewall(status: core::FirewallStatus) -> Firewall {
    if status.open() {
        Firewall::Present
    } else if status.closed() {
        Firewall::Missing
    } else {
        Firewall::Partial
    }
}

/// The game port's TCP and UDP rules and the map download port's TCP rule.
pub fn firewall_status(port: u16, downloads: u16) -> Result<Firewall, String> {
    core::firewall_rule_status(port, Some(downloads))
        .map(firewall)
        .map_err(|e| e.to_string())
}

/// Add the rules for the game port (TCP and UDP) and the map download port (TCP). Windows
/// shows its admin prompt once; a declined prompt is an error.
pub fn add_firewall(port: u16, downloads: u16) -> Result<Firewall, String> {
    core::add_firewall_rule(port, Some(downloads), true)
        .map(firewall)
        .map_err(|e| e.to_string())
}

/// Remove the rules again. Windows shows its admin prompt once; a declined prompt is an error.
pub fn remove_firewall(port: u16, downloads: u16) -> Result<Firewall, String> {
    core::remove_firewall_rule(port, Some(downloads), true)
        .map(firewall)
        .map_err(|e| e.to_string())
}

pub fn lan_addresses() -> Vec<String> {
    core::lan_ipv4s()
        .into_iter()
        .map(|ip| ip.to_string())
        .collect()
}

/// Join this PC's server through Steam (`steam://connect`).
pub fn join(port: u16, password: Option<&str>) -> Result<(), String> {
    let address = format!("{}:{port}", IpAddr::V4(Ipv4Addr::LOCALHOST));
    deadlock_core::servers::open_connect(&address, password)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Copy a custom map's VPK into the server folder and advertise it, so Deadworks offers it to
/// joiners. Writes inside the server folder only.
pub fn install_content(server_dir: &Path, vpk: &Path) -> Result<(), String> {
    core::install_server_content(server_dir, &[vpk.to_path_buf()], true)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The relay join code once the console printed it, and whether Valve's relays are ready.
pub fn relay(server: &Running) -> (Option<String>, bool) {
    let state = server.relay_state();
    (server.join_code(), state.relay_ready || state.logged_on)
}

/// What asking the router for ports did, as one line per port, and the router's public
/// address when it said.
pub fn upnp(server: &Running) -> Option<super::Upnp> {
    use core::UpnpOutcome;
    Some(match server.upnp_outcome()? {
        UpnpOutcome::Failed(error) => super::Upnp::Failed(error.clone()),
        UpnpOutcome::Mapped { ports, external_ip } => super::Upnp::Mapped {
            ports: ports
                .iter()
                .map(|(protocol, port, error)| (format!("{} {port}", protocol.as_str()), error.clone()))
                .collect(),
            external_ip: external_ip.map(|ip| ip.to_string()),
        },
    })
}

/// Where each piece of Sigil's server content came from on GameBanana, from the install
/// journal and the library's slot to mod table. Reads only.
pub fn origins(server_dir: &Path, library: &[(u16, u64)]) -> Vec<core::ContentOrigin> {
    let journal = deadlock_core::install::read_journal(&crate::data::workspace_dir()).unwrap_or_default();
    let by_slot = |slot: u16| library.iter().find(|(s, _)| *s == slot).map(|(_, id)| *id);
    core::content_origins(server_dir, &journal, &by_slot)
}

/// The invite text, and the content friends cannot get from GameBanana.
pub fn invite(target: InviteTarget, password: bool, origins: &[core::ContentOrigin]) -> Option<(Invite, Vec<String>)> {
    core::host_invite(target, password, origins).ok()
}
