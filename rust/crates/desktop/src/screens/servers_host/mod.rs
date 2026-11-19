//! Host: run a Deadworks server on this PC (ADR 0015), the third tab of Servers. Reachable as
//! `crate::screens::host` too.
//!
//! The tab is a setup checklist (.NET 10, free space, SteamCMD, the server files, Deadworks),
//! the server card (settings, a map picker with the player's custom maps, Start and Stop,
//! status, the live console and a command line), plugins, the firewall rule and the ways in
//! (LAN join, `connect ip:port`, router forwarding). Every download, process start, file copy
//! and firewall change is a press here; nothing starts on its own and no agent tool reaches
//! any of it. Checks and installs run on workers through `bridge`, the only file that calls
//! `deadlock_core::host`. The console keeps at most `CONSOLE_LINES` lines in memory.
//!
//! SteamCMD opens in its own console window: the player signs in there with a Steam account
//! that owns Deadlock. Sigil passes the account name only and never sees, asks for or stores
//! the password or the Steam Guard code.

pub(crate) mod bridge;
pub mod scenes;
mod view;

#[path = "../settings/locales_host.rs"]
pub(crate) mod locales;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rune::{FileDialogOptions, TextEdit, Tx};

use crate::app::State;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};

pub use view::{dialog, overlay, tab_status, view};

/// Console lines kept in memory for the page. The core keeps its own bounded ring.
pub const CONSOLE_LINES: usize = 400;
/// How often the page reads the console and the SteamCMD status while something runs.
const TICK_SECONDS: f64 = 0.5;
/// Disk space the server files need, as the Deadworks guide states.
pub const NEED_BYTES: u64 = deadlock_core::host::REQUIRED_FREE_BYTES;
pub const DEFAULT_PORT: u16 = deadlock_core::host::DEFAULT_PORT;
/// Microsoft's .NET 10 download page, opened in the browser (Sigil downloads nothing here).
pub const DOTNET_URL: &str = "https://dotnet.microsoft.com/download/dotnet/10.0";

/// What the setup check found. Built by `bridge::check` from the core's prerequisites.
#[derive(Clone, Debug, Default)]
pub struct Setup {
    /// The .NET 10 runtime version found, if any.
    pub dotnet: Option<String>,
    /// Free bytes on the drive that holds the server folder.
    pub free_bytes: Option<u64>,
    /// Room for a fresh download, or the files are already there.
    pub enough_disk: bool,
    pub steamcmd: bool,
    /// The server files' build from `game/citadel/steam.inf`; None means no files yet.
    pub server_build: Option<String>,
    /// The player's own game build, for the "older than your game" line.
    pub game_build: Option<String>,
    /// The server files trail the player's game.
    pub needs_update: bool,
    /// The Deadworks version in the server folder; empty when it was not installed by Sigil.
    pub deadworks: Option<String>,
    pub deadworks_behind: bool,
    /// The Deadworks release Sigil pins and installs.
    pub deadworks_pinned: String,
    /// Custom maps in the player's enabled addon slots, with the VPK that carries each.
    pub custom_maps: Vec<Content>,
    /// Maps Sigil already put into the server folder.
    pub server_maps: Vec<String>,
}

impl Setup {
    pub fn space_ok(&self) -> bool {
        self.enough_disk
    }

    /// Everything the server needs to start.
    pub fn ready(&self) -> bool {
        self.dotnet.is_some() && self.server_build.is_some() && self.deadworks.is_some()
    }

    /// The server's files are an older build than the player's game.
    pub fn outdated(&self) -> bool {
        self.needs_update && self.server_build.is_some()
    }

    pub fn deadworks_outdated(&self) -> bool {
        self.deadworks.is_some() && self.deadworks_behind
    }
}

/// The server process as the page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Stopped,
    Starting,
    Running,
    Stopping,
    /// It stopped on its own; the exit line, if any.
    Exited(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub on: bool,
}

/// The firewall rule for the server port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Firewall {
    Unknown,
    Present,
    /// Only one of TCP and UDP is open.
    Partial,
    Missing,
}

/// One long piece of work the page waits on. Only one setup step runs at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    SteamCmd,
    Deadworks,
}

/// The confirm dialogs. A dialog says what will happen before a download or a prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Download the server files with SteamCMD: asks for the Steam account name.
    ServerFiles {
        update: bool,
    },
    Firewall,
}

/// A custom map's VPK waiting to be copied into the server folder at the next Start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub name: String,
    pub vpk: PathBuf,
}

pub struct HostState {
    /// Windows only. A field so headless scenes can show the Windows page on any machine.
    pub windows: bool,
    pub server_dir: Option<PathBuf>,
    pub steamcmd_dir: Option<PathBuf>,
    pub setup: Option<Result<Setup, String>>,
    pub check_job: Job<Setup>,
    pub busy: Option<Busy>,
    pub install_job: Job<String>,
    /// SteamCMD's own window, while it is open.
    pub steamcmd: Option<bridge::SteamCmdRun>,
    pub steamcmd_status: Option<String>,
    pub steam_user: TextEdit,
    pub name: TextEdit,
    pub map: TextEdit,
    pub port: TextEdit,
    pub password: TextEdit,
    pub max_players: TextEdit,
    pub extra: TextEdit,
    pub map_menu: bool,
    /// Custom maps queued by `open_with_map`, copied in on the next Start.
    pub content: Vec<Content>,
    /// The map the running server loaded.
    pub live_map: Option<String>,
    pub phase: Phase,
    pub server: Option<bridge::Running>,
    pub start_job: Job<bridge::Running>,
    pub stop_job: Job<()>,
    pub console: VecDeque<String>,
    /// Lines the core has handed over so far, to take only new ones.
    pub console_seen: u64,
    pub command: TextEdit,
    pub plugins: Option<Result<Vec<Plugin>, String>>,
    pub plugin_job: Job<Vec<Plugin>>,
    pub firewall: Firewall,
    pub firewall_job: Job<Firewall>,
    pub lan: Vec<String>,
    pub join_job: Job<()>,
    pub confirm: Option<Confirm>,
    pub error: Option<String>,
    pub loaded: bool,
    pub next_tick: Option<f64>,
}

impl Default for HostState {
    fn default() -> Self {
        let config = bridge::Config::default();
        HostState {
            windows: deadlock_core::host::hosting_supported(),
            server_dir: None,
            steamcmd_dir: None,
            setup: None,
            check_job: Job::default(),
            busy: None,
            install_job: Job::default(),
            steamcmd: None,
            steamcmd_status: None,
            steam_user: TextEdit::new(""),
            name: TextEdit::new(config.name),
            map: TextEdit::new(config.map),
            port: TextEdit::new(config.port.to_string()),
            password: TextEdit::new(""),
            max_players: TextEdit::new(""),
            extra: TextEdit::new(""),
            map_menu: false,
            content: Vec::new(),
            live_map: None,
            phase: Phase::Stopped,
            server: None,
            start_job: Job::default(),
            stop_job: Job::default(),
            console: VecDeque::new(),
            console_seen: 0,
            command: TextEdit::new(""),
            plugins: None,
            plugin_job: Job::default(),
            firewall: Firewall::Unknown,
            firewall_job: Job::default(),
            lan: Vec::new(),
            join_job: Job::default(),
            confirm: None,
            error: None,
            loaded: false,
            next_tick: None,
        }
    }
}

impl HostState {
    /// The port the fields hold, or the default when the field is not a port.
    pub fn port(&self) -> u16 {
        parse_port(&self.port.text).unwrap_or(DEFAULT_PORT)
    }

    pub fn ready(&self) -> bool {
        matches!(&self.setup, Some(Ok(setup)) if setup.ready())
    }

    pub fn running(&self) -> bool {
        matches!(
            self.phase,
            Phase::Running | Phase::Starting | Phase::Stopping
        )
    }

    fn setup_ok(&self) -> Option<&Setup> {
        match &self.setup {
            Some(Ok(setup)) => Some(setup),
            _ => None,
        }
    }

    /// The custom maps the picker offers: queued, installed in the game, and on the server.
    pub fn custom_maps(&self) -> Vec<String> {
        let mut maps: Vec<String> = self.content.iter().map(|c| c.name.clone()).collect();
        if let Some(setup) = self.setup_ok() {
            for name in setup
                .custom_maps
                .iter()
                .map(|c| &c.name)
                .chain(&setup.server_maps)
            {
                if !maps.contains(name) && !bridge::STOCK_MAPS.contains(&name.as_str()) {
                    maps.push(name.clone());
                }
            }
        }
        maps
    }

    /// The map is not one of the game's own.
    pub fn is_custom(&self, map: &str) -> bool {
        !bridge::STOCK_MAPS.contains(&map) && self.custom_maps().iter().any(|m| m == map)
    }

    /// The server folder has this map from an earlier Start.
    pub fn on_server(&self, map: &str) -> bool {
        self.setup_ok()
            .is_some_and(|s| s.server_maps.iter().any(|m| m == map))
    }

    /// The VPK the next Start copies into the server for `map`: a queued one, or the player's
    /// installed copy when the server does not have the map yet.
    pub fn content_for(&self, map: &str) -> Option<&Content> {
        if let Some(item) = self.content.iter().find(|c| c.name == map) {
            return Some(item);
        }
        if self.on_server(map) {
            return None;
        }
        self.setup_ok()?.custom_maps.iter().find(|c| c.name == map)
    }

    /// The settings the fields hold, checked. An error names the field to fix.
    pub fn config(&self) -> Result<bridge::Config, String> {
        let port = parse_port(&self.port.text)
            .ok_or_else(|| tr("The port must be a number from 1024 to 65535.").to_string())?;
        let max = self.max_players.text.trim();
        let max_players = if max.is_empty() {
            None
        } else {
            Some(
                max.parse::<u32>()
                    .ok()
                    .filter(|n| (1..=64).contains(n))
                    .ok_or_else(|| tr("Max players must be a number from 1 to 64, or empty for the server's default.").to_string())?,
            )
        };
        let map = self.map.text.trim().to_string();
        if map.is_empty() || !map.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(tr(
                "The map is a console name like dl_midtown: letters, digits and _ only.",
            )
            .to_string());
        }
        let config = bridge::Config {
            name: self.name.text.trim().to_string(),
            map,
            port,
            password: self.password.text.clone(),
            max_players,
            extra: self.extra.text.trim().to_string(),
        };
        config.validate().map_err(|e| e.to_string())?;
        Ok(config)
    }

    fn take_config(&mut self, config: &bridge::Config) {
        self.name = TextEdit::new(config.name.clone());
        self.map = TextEdit::new(config.map.clone());
        self.port = TextEdit::new(config.port.to_string());
        self.password = TextEdit::new(config.password.clone());
        self.max_players = TextEdit::new(
            config
                .max_players
                .map(|n| n.to_string())
                .unwrap_or_default(),
        );
        self.extra = TextEdit::new(config.extra.clone());
    }

    /// Append console lines, keeping the newest `CONSOLE_LINES`.
    pub fn push_lines(&mut self, lines: impl IntoIterator<Item = String>) {
        for line in lines {
            if self.console.len() == CONSOLE_LINES {
                self.console.pop_front();
            }
            self.console.push_back(line);
        }
    }

    /// Queue a custom map's VPK for the next Start, replacing an older queue entry for it.
    pub fn queue_content(&mut self, name: &str, vpk: &Path) {
        self.content.retain(|item| item.name != name);
        self.content.push(Content {
            name: name.to_string(),
            vpk: vpk.to_path_buf(),
        });
    }
}

pub fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p >= 1024)
}

/// `connect 192.168.1.20:27067`, the line a friend types in the game console.
pub fn connect_line(ip: &str, port: u16) -> String {
    format!("connect {ip}:{port}")
}

/// Bytes as GB with no decimals past ten (`412 GB`, `7.5 GB`).
pub fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

fn idle(st: &State) -> bool {
    st.demo || rune::is_headless()
}

// ---------------------------------------------------------------------------------------------
// Actions. Each is a press; each long one runs on a worker.
// ---------------------------------------------------------------------------------------------

/// Open Servers > Host with `name` as the server's map and its VPK queued: the next Start
/// copies it into the server folder (`install_server_content`), then loads the map. Nothing is
/// copied or started by this call.
pub fn open_with_map(st: &mut State, name: &str, vpk: &Path) {
    st.go(Screen::Servers);
    st.servers.tab = crate::screens::servers::Tab::Host;
    st.servers.menu = None;
    if !st.servers.host.loaded {
        // Load the saved settings first so they do not overwrite the map chosen here.
        load(st);
    }
    let host = &mut st.servers.host;
    host.queue_content(name, vpk);
    if !host.running() {
        host.map = TextEdit::new(name.to_string());
    }
}

/// First visit: the saved folders and settings, then the checks. Reads only.
fn load(st: &mut State) {
    st.servers.host.loaded = true;
    if !st.servers.host.windows || idle(st) {
        return;
    }
    let saved = bridge::load_saved();
    let host = &mut st.servers.host;
    host.server_dir = Some(saved.server_dir);
    host.steamcmd_dir = Some(saved.steamcmd_dir);
    host.steam_user = TextEdit::new(saved.steam_user);
    host.take_config(&saved.config);
    recheck(st);
}

/// Run every check again: prerequisites, plugins and the firewall rule.
pub fn recheck(st: &mut State) {
    if idle(st) || !st.servers.host.windows {
        return;
    }
    let host = &st.servers.host;
    let (Some(dir), Some(steamcmd)) = (host.server_dir.clone(), host.steamcmd_dir.clone()) else {
        return;
    };
    let game = st.game_root.clone();
    let addons = st.addons_dir.clone().map(PathBuf::from);
    let wake = st.wake.clone();
    st.servers.host.check_job.start(wake, move || {
        bridge::check(&dir, &steamcmd, game.as_deref(), addons.as_deref())
    });
    refresh_plugins(st);
    refresh_firewall(st);
    st.servers.host.lan = bridge::lan_addresses();
}

fn refresh_plugins(st: &mut State) {
    let Some(dir) = st.servers.host.server_dir.clone() else {
        return;
    };
    let wake = st.wake.clone();
    st.servers
        .host
        .plugin_job
        .start(wake, move || bridge::plugins(&dir));
}

fn refresh_firewall(st: &mut State) {
    let port = st.servers.host.port();
    let wake = st.wake.clone();
    st.servers
        .host
        .firewall_job
        .start(wake, move || bridge::firewall_status(port));
}

/// The folder picker for the server files. The choice is saved; nothing is moved.
pub fn choose_folder(st: &mut State, tx: &mut Tx) {
    if st.servers.host.running()
        || st.servers.host.busy.is_some()
        || st.servers.host.steamcmd.is_some()
    {
        return;
    }
    let options = FileDialogOptions {
        title: tr("Choose a folder for the server files").to_string(),
        ..FileDialogOptions::default()
    };
    tx.open_folder(options, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => {
            if let Err(error) = bridge::check_folder(&path, st.game_root.as_deref()) {
                st.servers.host.error = Some(error);
                return;
            }
            st.servers.host.server_dir = Some(path);
            st.servers.host.error = None;
            save(st);
            recheck(st);
        }
        Ok(None) => {}
        Err(error) => {
            st.status = fill(
                tr("The file picker could not open: {}"),
                &[&error.to_string()],
            )
        }
    });
}

/// Keep the folders, account name and settings for next time.
fn save(st: &mut State) {
    if idle(st) {
        return;
    }
    let host = &st.servers.host;
    let (Some(dir), Some(steamcmd)) = (host.server_dir.clone(), host.steamcmd_dir.clone()) else {
        return;
    };
    let config = host.config().ok();
    if let Err(error) = bridge::save(
        &dir,
        &steamcmd,
        host.steam_user.text.trim(),
        config.as_ref(),
    ) {
        st.status = error;
    }
}

pub fn install_steamcmd(st: &mut State) {
    let host = &mut st.servers.host;
    if host.busy.is_some() {
        return;
    }
    let Some(dir) = host.steamcmd_dir.clone() else {
        return;
    };
    host.busy = Some(Busy::SteamCmd);
    host.error = None;
    if idle(st) {
        return;
    }
    let wake = st.wake.clone();
    st.servers.host.install_job.start(wake, move || {
        bridge::install_steamcmd(&dir).map(|_| String::new())
    });
}

pub fn install_deadworks(st: &mut State) {
    let host = &mut st.servers.host;
    if host.busy.is_some() || host.running() {
        return;
    }
    let Some(dir) = host.server_dir.clone() else {
        return;
    };
    host.busy = Some(Busy::Deadworks);
    host.error = None;
    if idle(st) {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .host
        .install_job
        .start(wake, move || bridge::install_deadworks(&dir));
}

/// Open SteamCMD in its own window to download or update the server files. The account name
/// goes on its command line; the password is typed into SteamCMD, never into Sigil.
pub fn open_steamcmd(st: &mut State) {
    let user = st.servers.host.steam_user.text.trim().to_string();
    if !deadlock_core::host::valid_steam_user(&user) {
        st.servers.host.error =
            Some(tr("Type the Steam account name you sign in with.").to_string());
        return;
    }
    st.servers.host.confirm = None;
    st.servers.host.error = None;
    save(st);
    if idle(st) || st.servers.host.steamcmd.is_some() || st.servers.host.running() {
        return;
    }
    let host = &st.servers.host;
    let (Some(dir), Some(steamcmd)) = (host.server_dir.clone(), host.steamcmd_dir.clone()) else {
        return;
    };
    // Spawning the console window is quick; the download runs in SteamCMD's own process.
    match bridge::open_steamcmd(&steamcmd, &dir, &user) {
        Ok(run) => {
            st.servers.host.steamcmd = Some(run);
            st.servers.host.steamcmd_status = None;
        }
        Err(error) => st.servers.host.error = Some(error),
    }
}

pub fn start(st: &mut State) {
    let host = &mut st.servers.host;
    if host.running() || !host.ready() || host.busy.is_some() || host.steamcmd.is_some() {
        return;
    }
    let config = match host.config() {
        Ok(config) => config,
        Err(error) => {
            host.error = Some(error);
            return;
        }
    };
    let Some(dir) = host.server_dir.clone() else {
        return;
    };
    host.error = None;
    host.map_menu = false;
    host.phase = Phase::Starting;
    host.live_map = Some(config.map.clone());
    host.console.clear();
    host.console_seen = 0;
    let copy = host.content_for(&config.map).map(|c| c.vpk.clone());
    save(st);
    if idle(st) {
        return;
    }
    let wake = st.wake.clone();
    st.servers.host.start_job.start(wake, move || {
        if let Some(vpk) = &copy {
            bridge::install_content(&dir, vpk)?;
        }
        bridge::start(&dir, &config)
    });
}

pub fn stop(st: &mut State) {
    let host = &mut st.servers.host;
    if host.phase != Phase::Running {
        return;
    }
    let Some(server) = host.server.clone() else {
        return;
    };
    host.phase = Phase::Stopping;
    if idle(st) {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .host
        .stop_job
        .start(wake, move || bridge::stop(&server));
}

/// Send the command line to the running server's console.
pub fn send_command(st: &mut State) {
    let host = &mut st.servers.host;
    let command = host.command.text.trim().to_string();
    if command.is_empty() {
        return;
    }
    let Some(server) = host.server.clone() else {
        return;
    };
    match bridge::send(&server, &command) {
        Ok(()) => host.command = TextEdit::new(""),
        Err(error) => host.error = Some(error),
    }
}

pub fn set_plugin(st: &mut State, name: String, on: bool) {
    if st.servers.host.plugin_job.is_running() {
        return;
    }
    if let Some(Ok(list)) = &mut st.servers.host.plugins
        && let Some(row) = list.iter_mut().find(|p| p.name == name)
    {
        row.on = on;
    }
    let Some(dir) = st.servers.host.server_dir.clone() else {
        return;
    };
    if idle(st) {
        return;
    }
    let wake = st.wake.clone();
    st.servers.host.plugin_job.start(wake, move || {
        bridge::set_plugin(&dir, &name, on)?;
        bridge::plugins(&dir)
    });
}

/// Add the inbound rule. Windows shows its admin prompt; the result is read back afterwards.
pub fn add_firewall(st: &mut State) {
    st.servers.host.confirm = None;
    let port = st.servers.host.port();
    if idle(st) || st.servers.host.firewall_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .host
        .firewall_job
        .start(wake, move || bridge::add_firewall(port));
}

/// Close the server port again: the player's press, behind Windows' admin prompt.
pub fn remove_firewall(st: &mut State) {
    let port = st.servers.host.port();
    if idle(st) || st.servers.host.firewall_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .host
        .firewall_job
        .start(wake, move || bridge::remove_firewall(port));
}

/// Join the server from this PC through Steam.
pub fn join_here(st: &mut State) {
    let port = st.servers.host.port();
    let password = Some(st.servers.host.password.text.clone()).filter(|p| !p.is_empty());
    if idle(st) || st.servers.host.join_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.servers
        .host
        .join_job
        .start(wake, move || bridge::join(port, password.as_deref()));
}

// ---------------------------------------------------------------------------------------------
// Poll
// ---------------------------------------------------------------------------------------------

/// Take finished work and new console lines. Returns true when the page changed.
pub fn poll(st: &mut State, now: f64, tx: &mut Tx) -> bool {
    let on_page =
        st.screen == Screen::Servers && st.servers.tab == crate::screens::servers::Tab::Host;
    let mut changed = false;
    if on_page && !st.servers.host.loaded {
        load(st);
        changed = true;
    }
    let host = &mut st.servers.host;
    if let Some(result) = host.check_job.try_take() {
        host.setup = Some(result);
        changed = true;
    }
    if let Some(result) = host.install_job.try_take() {
        let busy = host.busy.take();
        match result {
            Ok(version) => {
                let line = match busy {
                    Some(Busy::Deadworks) => fill(tr("Deadworks {} is installed."), &[&version]),
                    _ => tr("SteamCMD is installed.").to_string(),
                };
                crate::notify::push(st, crate::notify::Notice::success(line));
            }
            Err(error) => st.servers.host.error = Some(error),
        }
        recheck(st);
        changed = true;
    }
    let host = &mut st.servers.host;
    if let Some(result) = host.plugin_job.try_take() {
        host.plugins = Some(result);
        changed = true;
    }
    if let Some(result) = host.firewall_job.try_take() {
        match result {
            Ok(state) => host.firewall = state,
            Err(error) => host.error = Some(error),
        }
        changed = true;
    }
    if let Some(result) = host.join_job.try_take() {
        if let Err(error) = result {
            host.error = Some(error);
        }
        changed = true;
    }
    if let Some(result) = host.start_job.try_take() {
        match result {
            Ok(running) => {
                host.server = Some(running);
                host.phase = Phase::Running;
                // The map is in the server folder now; the next check lists it there.
                if let Some(map) = &host.live_map {
                    host.content.retain(|c| &c.name != map);
                    if let Some(Ok(setup)) = &mut host.setup
                        && !setup.server_maps.contains(map)
                        && !bridge::STOCK_MAPS.contains(&map.as_str())
                    {
                        setup.server_maps.push(map.clone());
                    }
                }
            }
            Err(error) => {
                // A queued map stays queued for the next try.
                host.live_map = None;
                host.phase = Phase::Failed(error);
            }
        }
        changed = true;
    }
    if let Some(result) = host.stop_job.try_take() {
        host.server = None;
        host.live_map = None;
        host.phase = match result {
            Ok(()) => Phase::Stopped,
            Err(error) => Phase::Failed(error),
        };
        changed = true;
    }
    // The live parts: console lines, the exit, SteamCMD's window and status.
    let mut live = false;
    if let Some(server) = host.server.clone() {
        live = true;
        let (lines, seen) = bridge::lines_since(&server, host.console_seen);
        if seen != host.console_seen {
            host.console_seen = seen;
            host.push_lines(lines);
            changed = true;
        }
        if host.phase == Phase::Running
            && let Some(exit) = bridge::exited(&server)
        {
            host.server = None;
            host.live_map = None;
            host.phase = Phase::Exited(exit);
            changed = true;
        }
    }
    let mut closed = false;
    if let Some(run) = &mut host.steamcmd {
        live = true;
        let status = bridge::steamcmd_status(run);
        if status.line.is_some() && status.line != host.steamcmd_status {
            host.steamcmd_status = status.line;
            changed = true;
        }
        if status.closed {
            closed = true;
            if let Some(code) = status.failed {
                host.error = Some(fill(
                    tr(
                        "SteamCMD closed with an error ({}). Its window showed what went wrong; try again.",
                    ),
                    &[&code],
                ));
            }
        }
    }
    if closed {
        st.servers.host.steamcmd = None;
        changed = true;
        recheck(st);
    }
    let host = &mut st.servers.host;
    if live {
        let due = host
            .next_tick
            .filter(|due| *due > now)
            .unwrap_or(now + TICK_SECONDS);
        host.next_tick = Some(due);
        tx.wake_at(due);
    } else {
        host.next_tick = None;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_keeps_only_the_newest_lines() {
        let mut host = HostState::default();
        host.push_lines((0..CONSOLE_LINES + 25).map(|i| format!("line {i}")));
        assert_eq!(host.console.len(), CONSOLE_LINES);
        assert_eq!(host.console.front().map(String::as_str), Some("line 25"));
        assert_eq!(
            host.console.back().map(String::as_str),
            Some(format!("line {}", CONSOLE_LINES + 24).as_str())
        );
    }

    #[test]
    fn settings_are_checked_before_a_start() {
        let mut host = HostState::default();
        host.name = TextEdit::new("Friday scrims");
        let config = host.config().unwrap();
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.map, "dl_midtown");
        assert_eq!(
            config.max_players, None,
            "empty leaves the server's default"
        );
        host.max_players = TextEdit::new("12");
        assert_eq!(host.config().unwrap().max_players, Some(12));
        host.port = TextEdit::new("80");
        assert!(host.config().is_err());
        host.port = TextEdit::new("27067");
        host.map = TextEdit::new("dl_midtown +rcon_password x");
        assert!(
            host.config().is_err(),
            "the map field cannot carry extra arguments"
        );
        host.map = TextEdit::new("dl_midtown");
        host.max_players = TextEdit::new("0");
        assert!(host.config().is_err());
        host.max_players = TextEdit::new("");
        host.name = TextEdit::new("bad\"name");
        assert!(host.config().is_err(), "the core's own check runs too");
    }

    #[test]
    fn setup_reads_ready_space_and_age() {
        let mut setup = Setup {
            dotnet: Some("10.0.1".into()),
            free_bytes: Some(20 * 1024 * 1024 * 1024),
            enough_disk: false,
            steamcmd: true,
            game_build: Some("6123456".into()),
            deadworks_pinned: "v0.6.4".into(),
            ..Setup::default()
        };
        assert!(!setup.space_ok());
        assert!(!setup.ready());
        assert!(!setup.outdated(), "no files is not an update");
        setup.server_build = Some("6120000".into());
        setup.needs_update = true;
        assert!(setup.outdated());
        setup.deadworks = Some("v0.6.3".into());
        setup.deadworks_behind = true;
        assert!(setup.ready());
        assert!(setup.deadworks_outdated());
    }

    #[test]
    fn a_map_from_the_maps_page_is_queued_once() {
        let mut host = HostState::default();
        host.queue_content("harbor_night", Path::new("/maps/harbor_night.vpk"));
        host.queue_content("harbor_night", Path::new("/maps/harbor_night_v2.vpk"));
        assert_eq!(host.content.len(), 1);
        assert_eq!(
            host.content[0].vpk,
            PathBuf::from("/maps/harbor_night_v2.vpk")
        );
        assert!(host.is_custom("harbor_night"));
        assert!(!host.is_custom("dl_midtown"));
        assert_eq!(host.custom_maps(), vec!["harbor_night".to_string()]);
        assert_eq!(
            host.content_for("harbor_night").map(|c| c.vpk.clone()),
            Some(PathBuf::from("/maps/harbor_night_v2.vpk"))
        );
    }

    #[test]
    fn an_installed_custom_map_is_copied_only_when_the_server_lacks_it() {
        let mut host = HostState::default();
        host.setup = Some(Ok(Setup {
            custom_maps: vec![Content {
                name: "st_dome_a2".into(),
                vpk: PathBuf::from("/addons/pak03_dir.vpk"),
            }],
            server_maps: vec!["harbor_night".into()],
            ..Setup::default()
        }));
        assert_eq!(
            host.content_for("st_dome_a2").map(|c| c.vpk.clone()),
            Some(PathBuf::from("/addons/pak03_dir.vpk"))
        );
        assert!(
            host.content_for("harbor_night").is_none(),
            "already on the server"
        );
        assert!(host.content_for("dl_midtown").is_none());
        assert!(host.is_custom("harbor_night"));
        assert_eq!(
            host.custom_maps(),
            vec!["st_dome_a2".to_string(), "harbor_night".to_string()]
        );
    }

    #[test]
    fn connect_lines_and_sizes() {
        assert_eq!(
            connect_line("192.168.1.20", 27067),
            "connect 192.168.1.20:27067"
        );
        assert_eq!(gigabytes(NEED_BYTES), "35 GB");
        assert_eq!(
            gigabytes(7 * 1024 * 1024 * 1024 + 512 * 1024 * 1024),
            "7.5 GB"
        );
        assert_eq!(parse_port(" 27067 "), Some(27067));
        assert_eq!(parse_port("99999"), None);
    }
}
