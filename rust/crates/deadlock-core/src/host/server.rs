//! The server process: start `deadworks.exe -dedicated`, capture its console, stop it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::config::{SERVER_CFG_NAME, ServerConfig};
use super::content::start_downloads;
use super::fastdl_server::FastdlServer;
use super::network::{Network, RelayState, prepare_network};
use super::upnp::{PortProtocol, PortRequest, UpnpLease};
use super::{require_approval, require_windows};
use crate::error::{Error, Result};
use crate::util::{hidden_command, write_atomic};

/// Console lines kept per server. Older lines drop off the front.
pub const MAX_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 4096;

/// `<server_dir>/game/bin/win64/deadworks.exe`.
pub fn deadworks_exe(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("bin").join("win64").join("deadworks.exe")
}

/// A bounded ring of console lines, shared with the reader threads.
#[derive(Debug, Default)]
pub struct LineBuffer {
    inner: Mutex<(VecDeque<String>, u64)>,
    /// What the console said about Steam networking, read as lines arrive so it survives
    /// the ring dropping old lines.
    relay: Mutex<RelayState>,
}

impl LineBuffer {
    /// One line of server output.
    pub fn push(&self, line: &str) {
        if let Ok(mut relay) = self.relay.lock() {
            relay.feed(line);
        }
        self.keep(line);
    }

    /// The join code and relay readiness seen so far.
    pub fn relay(&self) -> RelayState {
        self.relay.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Keep a line without reading it for signals (Sigil's own echo of a typed command).
    fn keep(&self, line: &str) {
        let mut line: String = line.trim_end_matches(['\r', '\n']).to_string();
        if line.len() > MAX_LINE_CHARS {
            let mut cut = MAX_LINE_CHARS;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            line.truncate(cut);
        }
        if let Ok(mut guard) = self.inner.lock() {
            let (lines, total) = &mut *guard;
            if lines.len() == MAX_LINES {
                lines.pop_front();
            }
            lines.push_back(line);
            *total += 1;
        }
    }

    /// The kept lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.inner.lock().map(|g| g.0.iter().cloned().collect()).unwrap_or_default()
    }

    /// Lines pushed since the start, including the ones that dropped off. Compare it with an
    /// earlier value to know whether to redraw.
    pub fn total(&self) -> u64 {
        self.inner.lock().map(|g| g.1).unwrap_or(0)
    }
}

/// How [`ServerHandle::stop`] ended the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// It had already exited.
    AlreadyStopped,
    /// It quit after the `quit` console command.
    Graceful,
    /// It was killed after the grace period.
    Forced,
}

/// A running server. Dropping the handle kills the process and stops its map download server,
/// so neither outlives the app by accident.
#[derive(Debug)]
pub struct ServerHandle {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    log: Arc<LineBuffer>,
    /// The map download server, while the server has custom content to hand out.
    downloads: Mutex<Option<FastdlServer>>,
    pub port: u16,
    /// The TCP port joiners download custom maps from, when the download server runs.
    pub fastdl_port: Option<u16>,
    /// How friends reach it.
    pub network: Network,
    /// Router ports opened through UPnP, closed again on stop (and on drop).
    upnp: Mutex<Option<UpnpLease>>,
    /// What the UPnP request did, when the player turned it on.
    upnp_outcome: Option<UpnpOutcome>,
    pub started: Instant,
}

/// What asking the router for ports did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpnpOutcome {
    /// Each requested port, with the router's refusal when it refused that one.
    Mapped { ports: Vec<(PortProtocol, u16, Option<String>)>, external_ip: Option<Ipv4Addr> },
    /// No router answered, or it refused every port.
    Failed(String),
}

impl ServerHandle {
    pub fn pid(&self) -> u32 {
        self.child.lock().map(|c| c.id()).unwrap_or(0)
    }

    pub fn is_running(&self) -> bool {
        self.child
            .lock()
            .map(|mut c| matches!(c.try_wait(), Ok(None)))
            .unwrap_or(false)
    }

    /// The exit code once it has exited.
    pub fn exit_code(&self) -> Option<i32> {
        self.child.lock().ok()?.try_wait().ok()?.and_then(|s| s.code())
    }

    /// The last [`MAX_LINES`] console lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.log.lines()
    }

    pub fn line_total(&self) -> u64 {
        self.log.total()
    }

    /// Shared access to the buffer for a UI that polls it.
    pub fn buffer(&self) -> Arc<LineBuffer> {
        Arc::clone(&self.log)
    }

    /// Type a console command into the server (`status`, `map dl_midtown`, ...). One line only.
    pub fn send_command(&self, command: &str) -> Result<()> {
        let command = command.trim();
        if command.is_empty() || command.chars().any(char::is_control) {
            return Err(Error::msg("Type one console command on one line."));
        }
        let mut guard = self.stdin.lock().map_err(|_| Error::msg("The server console is busy."))?;
        let stdin = guard.as_mut().ok_or_else(|| Error::msg("The server console is closed."))?;
        stdin.write_all(format!("{command}\n").as_bytes())?;
        stdin.flush()?;
        self.log.keep(&format!("> {command}"));
        Ok(())
    }

    /// The join code in relay mode: the `ServerSteamID` the console printed, once it has.
    /// It changes every restart. None in direct mode.
    pub fn join_code(&self) -> Option<String> {
        (self.network == Network::Relay).then(|| self.log.relay().join_code).flatten()
    }

    /// Everything the console said about Steam networking so far.
    pub fn relay_state(&self) -> RelayState {
        self.log.relay()
    }

    /// What the UPnP request did. None when UPnP was off or there was nothing to open.
    pub fn upnp_outcome(&self) -> Option<&UpnpOutcome> {
        self.upnp_outcome.as_ref()
    }

    /// Remove the router ports UPnP opened. Safe to call more than once.
    pub fn close_router_ports(&self) {
        if let Ok(mut upnp) = self.upnp.lock()
            && let Some(mut lease) = upnp.take()
        {
            lease.release();
        }
    }

    /// True while the map download server is up.
    pub fn serving_downloads(&self) -> bool {
        self.downloads.lock().map(|d| d.as_ref().is_some_and(FastdlServer::is_running)).unwrap_or(false)
    }

    /// Close the map download server and its open downloads. The game server keeps running.
    pub fn stop_downloads(&self) {
        if let Ok(mut downloads) = self.downloads.lock()
            && let Some(mut server) = downloads.take()
        {
            server.stop();
        }
    }

    /// Ask the server to `quit`, wait up to `grace`, then kill it. The map download server
    /// stops with it.
    pub fn stop(&self, grace: Duration) -> Result<StopOutcome> {
        let outcome = self.stop_process(grace);
        self.stop_downloads();
        self.close_router_ports();
        outcome
    }

    fn stop_process(&self, grace: Duration) -> Result<StopOutcome> {
        if !self.is_running() {
            return Ok(StopOutcome::AlreadyStopped);
        }
        let _ = self.send_command("quit");
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if !self.is_running() {
                return Ok(StopOutcome::Graceful);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut child = self.child.lock().map_err(|_| Error::msg("The server handle is busy."))?;
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Ok(StopOutcome::Graceful);
        }
        child.kill()?;
        let _ = child.wait();
        Ok(StopOutcome::Forced)
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut()
            && matches!(child.try_wait(), Ok(None))
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The `deadworks.exe` command, without a window, its console piped. Pure: nothing runs.
pub fn server_command(server_dir: &Path, config: &ServerConfig) -> Result<Command> {
    config.validate()?;
    let exe = deadworks_exe(server_dir);
    let mut command = hidden_command(&exe);
    command
        .args(config.launch_args())
        .current_dir(exe.parent().unwrap_or(server_dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

/// Write the server cfg and start the server. The app calls this when the player presses Start.
///
/// When the server folder holds custom content Sigil installed, the same press also refreshes
/// `sigil-fastdl/`, advertises `http://0.0.0.0:<download port>` as the download address and
/// starts the read-only map download server on that port ([`ServerConfig::download_port`]).
/// It stops when the handle is stopped or dropped.
pub fn start_server(server_dir: &Path, config: &ServerConfig, approve: bool) -> Result<ServerHandle> {
    require_approval(approve, "Starting the server")?;
    require_windows()?;
    config.validate()?;
    if !deadworks_exe(server_dir).is_file() {
        return Err(Error::msg("Deadworks is not installed in the server folder yet."));
    }
    let maps = super::content::server_map_names(server_dir);
    if !maps.is_empty() && !maps.contains(&config.map) {
        return Err(Error::msg(format!(
            "The server has no map {}. Pick one of: {}.",
            config.map,
            maps.join(", ")
        )));
    }
    if UdpSocket::bind(("0.0.0.0", config.port)).is_err() {
        return Err(Error::msg(format!(
            "Port {} is already in use. Stop the other server or pick another port.",
            config.port
        )));
    }
    let download_port = config
        .download_port()
        .ok_or_else(|| Error::msg("The map download port would be past 65535."))?;
    // The network mode's gameinfo.gi edits and listing go in first: SteamCMD's validate puts
    // Steam's gameinfo.gi back after every update.
    prepare_network(server_dir, config.network, true)?;
    // Deadworks reads deadworks.jsonc at startup, so the download address goes in first.
    let downloads = start_downloads(server_dir, download_port)?;
    let fastdl_port = downloads.as_ref().map(FastdlServer::port);
    let (upnp, upnp_outcome) = if config.upnp {
        open_router_ports(&router_ports(config.network, config.port, fastdl_port))
    } else {
        (None, None)
    };
    let cfg = server_dir.join("game").join("citadel").join("cfg").join(SERVER_CFG_NAME);
    write_atomic(&cfg, config.cfg_text().as_bytes())?;

    let mut child = server_command(server_dir, config)?.spawn()?;
    let log = Arc::new(LineBuffer::default());
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, Arc::clone(&log));
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, Arc::clone(&log));
    }
    let stdin = child.stdin.take();
    Ok(ServerHandle {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        log,
        downloads: Mutex::new(downloads),
        port: config.port,
        fastdl_port,
        network: config.network,
        upnp: Mutex::new(upnp),
        upnp_outcome,
        started: Instant::now(),
    })
}

/// The router ports UPnP opens: the game port (TCP and UDP) in direct mode, and the map
/// download port (TCP) whenever it serves. A relay server needs no game port.
pub fn router_ports(network: Network, game_port: u16, download_port: Option<u16>) -> Vec<PortRequest> {
    let mut ports = Vec::new();
    if network == Network::Direct {
        ports.push(PortRequest { protocol: PortProtocol::Tcp, port: game_port });
        ports.push(PortRequest { protocol: PortProtocol::Udp, port: game_port });
    }
    if let Some(port) = download_port {
        ports.push(PortRequest { protocol: PortProtocol::Tcp, port });
    }
    ports
}

/// Ask the router for `ports`, inside the player's Start press (UPnP is on). A router that
/// does not answer does not stop the server; the outcome says what happened.
fn open_router_ports(ports: &[PortRequest]) -> (Option<UpnpLease>, Option<UpnpOutcome>) {
    if ports.is_empty() {
        return (None, None);
    }
    match super::upnp::open_ports(ports, "Sigil Deadworks server", true) {
        Ok(lease) => {
            let outcome = UpnpOutcome::Mapped {
                ports: lease.results.iter().map(|r| (r.protocol, r.port, r.error.clone())).collect(),
                external_ip: lease.external_ip,
            };
            (Some(lease), Some(outcome))
        }
        Err(error) => (None, Some(UpnpOutcome::Failed(error.to_string()))),
    }
}

fn pump(stream: impl Read + Send + 'static, log: Arc<LineBuffer>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => log.push(&String::from_utf8_lossy(&bytes)),
            }
        }
    });
}

/// True when the console says this Deadworks does not support the installed Deadlock build
/// (its signatures no longer match, as the Docker entrypoint detects). Update Deadworks, or wait
/// for a Deadworks release for the new build.
pub fn unsupported_build(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("Failed to find signature") || l.contains("Failed to find patch signature"))
}
