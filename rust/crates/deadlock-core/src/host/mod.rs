//! Host a Deadworks server from the player's PC (ADR 0015, `docs/decisions/0015-host-a-server.md`).
//!
//! Follows the Deadworks "Run a server" guide
//! (<https://docs.deadworks.net/getting-started/server-admins/run-a-server>) and the launch line
//! of its Docker entrypoint (`docker/entrypoint.sh` in `Deadworks-net/deadworks`):
//!
//! - Server files: SteamCMD `app_update 1422450 validate` into a folder of its own, with a Steam
//!   login that owns Deadlock. The login happens in SteamCMD's own console window; Sigil never
//!   asks for, passes, reads or stores the Steam password or Steam Guard code.
//! - Deadworks: the pinned release zip extracted over that folder (its `game/` merges).
//! - The server: `game\bin\win64\deadworks.exe -dedicated -console ... +hostport <port> +map <map>`.
//!   Name and join password go through a generated cfg, never the command line.
//! - Reach: a Windows Firewall rule for TCP and UDP on the port, LAN join links.
//!
//! Windows only: process starts, downloads and the prerequisite check return
//! [`HOSTING_NEEDS_WINDOWS`] elsewhere. Pure parsing, config and plugin file moves work on every
//! system so they can be tested anywhere.
//!
//! Every process start and every write is an app action the player triggers: each such function
//! takes `approve: bool` and refuses without it. Agent tools never call this module.

mod config;
mod content;
mod plugins;
mod reach;
mod server;
mod setup;

#[cfg(test)]
mod tests;

pub use config::{
    DEFAULT_MAP, DEFAULT_PORT, HostSettings, SERVER_CFG_NAME, ServerConfig, load_settings,
    save_settings, settings_path,
};
pub use content::{
    CONTENT_BACKUPS, CONTENT_LEDGER, FASTDL_DIR, ServerContent, ServerContentKind, build_fastdl,
    classify_content, deadworks_config_path, default_fastdl_dir, fastdl_url, install_server_content,
    list_server_content, remove_server_content, server_addons_dir, server_map_names, server_maps_dir,
    set_fastdl_url, strip_jsonc, vmap_names,
};
pub use plugins::{Plugin, list_plugins, plugins_dir, plugins_off_dir, set_plugin};
pub use reach::{
    FirewallStatus, add_firewall_rule, console_connect, firewall_script, firewall_rule_name,
    firewall_rule_status, join_url, lan_ipv4s, parse_firewall_show, remove_firewall_rule,
};
pub use server::{
    LineBuffer, MAX_LINES, ServerHandle, StopOutcome, deadworks_exe, server_command, start_server,
    unsupported_build,
};
pub use setup::{
    DEADWORKS_MARKER, DEADWORKS_SHA256, DEADWORKS_SIZE, DEADWORKS_URL, DEADWORKS_VERSION,
    DeadworksInstall, DeadworksReport, DotnetStatus, Prereqs, REQUIRED_FREE_BYTES, STEAMCMD_SHA256,
    STEAMCMD_SIZE, STEAMCMD_URL, SteamCmdEvent, UpdateAdvice, check_server_dir, deadworks_installed,
    dotnet_status, free_bytes, install_deadworks, install_deadworks_from_zip, install_steamcmd,
    parse_dotnet_runtimes, parse_steam_inf, parse_steamcmd_line, prerequisites, read_build,
    start_steamcmd_update, steamcmd_args, steamcmd_exe, steamcmd_log_lines, steamcmd_update_command,
    update_advice, valid_steam_user,
};

use crate::error::{Error, Result};

/// Steam app id of the Deadlock dedicated server files (the same id as the game).
pub const SERVER_APP_ID: u32 = 1422450;

/// What every Windows-only call answers on macOS and Linux.
pub const HOSTING_NEEDS_WINDOWS: &str = "Hosting needs Windows. Deadlock's dedicated server and Deadworks run only on Windows (Deadworks' Linux option runs the Windows server under Wine in Docker).";

/// True on Windows, where hosting works.
pub fn hosting_supported() -> bool {
    cfg!(windows)
}

/// Ok on Windows; [`HOSTING_NEEDS_WINDOWS`] elsewhere.
pub fn require_windows() -> Result<()> {
    if hosting_supported() {
        Ok(())
    } else {
        Err(Error::msg(HOSTING_NEEDS_WINDOWS))
    }
}

fn require_approval(approve: bool, what: &str) -> Result<()> {
    if approve {
        Ok(())
    } else {
        Err(Error::msg(format!("{what} was not approved. Nothing was changed.")))
    }
}
