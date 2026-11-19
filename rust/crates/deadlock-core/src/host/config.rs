//! The server's settings, saved as JSON in the app data directory.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::write_atomic;

/// The Deadworks guide's port (`+hostport`), TCP and UDP.
pub const DEFAULT_PORT: u16 = 27067;
/// The Deadworks Docker image's default map.
pub const DEFAULT_MAP: &str = "dl_midtown";
/// The cfg Sigil writes into `game/citadel/cfg/` before each start and runs with `+exec`.
/// The Docker entrypoint does the same: deadworks.exe re-joins its arguments with spaces, so a
/// name with a space would not survive the command line, and passwords on a command line show
/// up in process listings.
pub const SERVER_CFG_NAME: &str = "sigil_server.cfg";

/// One server's launch settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfig {
    /// `hostname`, shown in the server browser.
    pub name: String,
    /// A stock or custom map name: any `game/citadel/maps/<name>.vpk` in the server folder
    /// ([`super::server_map_names`]). A custom current map is advertised to joiners by itself.
    pub map: String,
    pub port: u16,
    /// `sv_password`: the join password. Empty for none. Written to the server cfg only.
    pub password: String,
    /// `-maxplayers`. None leaves the server's own default. Not confirmed for Deadlock yet.
    pub max_players: Option<u32>,
    /// More launch options, split on whitespace (the Docker image's `EXTRA_ARGS`).
    pub extra: String,
    /// TCP port of the map download server Sigil runs beside the server when it has custom
    /// content (ADR 0015 amendment). None: the game port plus one (27068 by default).
    pub fastdl_port: Option<u16>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "Sigil Server".to_string(),
            map: DEFAULT_MAP.to_string(),
            port: DEFAULT_PORT,
            password: String::new(),
            max_players: None,
            extra: String::new(),
            fastdl_port: None,
        }
    }
}

/// Everything the Host page remembers. No Steam password: SteamCMD asks for it in its window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostSettings {
    /// The server folder the player picked. Never inside their Deadlock install.
    pub server_dir: Option<PathBuf>,
    /// Where SteamCMD lives (`steamcmd.exe` inside it).
    pub steamcmd_dir: Option<PathBuf>,
    /// The Steam account name SteamCMD logs in with. Not a secret; the password never is stored.
    pub steam_user: String,
    pub server: ServerConfig,
}

/// `<data_dir>/host/settings.json`.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("host").join("settings.json")
}

/// The saved settings, or the defaults when none are saved yet.
pub fn load_settings(data_dir: &Path) -> Result<HostSettings> {
    let path = settings_path(data_dir);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HostSettings::default()),
        Err(error) => Err(error.into()),
    }
}

/// Save the settings (app data only, so no approval gate). The config is checked first.
pub fn save_settings(data_dir: &Path, settings: &HostSettings) -> Result<()> {
    settings.server.validate()?;
    let mut bytes = serde_json::to_vec_pretty(settings)?;
    bytes.push(b'\n');
    write_atomic(&settings_path(data_dir), &bytes)
}

fn cfg_safe(value: &str) -> bool {
    !value.chars().any(|c| c == '"' || c == ';' || c.is_control())
}

impl ServerConfig {
    /// Refuse values the cfg or the command line could not carry as typed.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().count() > 64 || !cfg_safe(&self.name) {
            return Err(Error::msg(
                "Give the server a name of 1 to 64 characters, without quotes or semicolons.",
            ));
        }
        let map_ok = !self.map.is_empty()
            && self.map.len() <= 64
            && self.map.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !map_ok {
            return Err(Error::msg(format!(
                "{:?} is not a map name. Use letters, digits and underscores, like {DEFAULT_MAP}.",
                self.map
            )));
        }
        if self.port < 1024 {
            return Err(Error::msg("Pick a port from 1024 to 65535."));
        }
        if self.password.chars().count() > 64 || !cfg_safe(&self.password) || self.password.contains(' ') {
            return Err(Error::msg(
                "The join password can have up to 64 characters, without spaces, quotes or semicolons.",
            ));
        }
        if let Some(players) = self.max_players
            && !(1..=64).contains(&players)
        {
            return Err(Error::msg("Max players must be from 1 to 64."));
        }
        if self.extra.chars().any(|c| c.is_control() && c != ' ' && c != '\t') {
            return Err(Error::msg("Extra launch options must be on one line."));
        }
        match self.download_port() {
            None => {
                return Err(Error::msg(
                    "The map download port would be past 65535. Pick a game port below 65535 or set the download port.",
                ));
            }
            Some(port) if port < 1024 => {
                return Err(Error::msg("Pick a map download port from 1024 to 65535."));
            }
            Some(port) if port == self.port => {
                return Err(Error::msg("The map download port must differ from the game port."));
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// The map download server's TCP port: [`Self::fastdl_port`], or the game port plus one.
    /// None when the game port is 65535 and no download port is set.
    pub fn download_port(&self) -> Option<u16> {
        self.fastdl_port.or_else(|| self.port.checked_add(1))
    }

    /// The `deadworks.exe` arguments, in order. Never carries the join password or the name.
    pub fn launch_args(&self) -> Vec<String> {
        let mut args: Vec<String> = [
            "-dedicated",
            "-console",
            "-dev",
            "-insecure",
            "-allow_no_lobby_connect",
            "+tv_citadel_auto_record",
            "0",
            "+spec_replay_enable",
            "0",
            "+tv_enable",
            "0",
            "+citadel_upload_replay_enabled",
            "0",
            "+hostport",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        args.push(self.port.to_string());
        if let Some(players) = self.max_players {
            args.push("-maxplayers".to_string());
            args.push(players.to_string());
        }
        args.push("+exec".to_string());
        args.push(SERVER_CFG_NAME.to_string());
        args.push("+map".to_string());
        args.push(self.map.clone());
        args.extend(self.extra.split_whitespace().map(str::to_string));
        args
    }

    /// The text of [`SERVER_CFG_NAME`]: `hostname` and `sv_password`.
    pub fn cfg_text(&self) -> String {
        format!(
            "// Written by Sigil before each start. Change these on the Host page.\nhostname \"{}\"\nsv_password \"{}\"\n",
            self.name.trim(),
            self.password
        )
    }
}
