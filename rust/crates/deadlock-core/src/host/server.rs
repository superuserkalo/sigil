//! The server process: start `deadworks.exe -dedicated`, capture its console, stop it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::config::{SERVER_CFG_NAME, ServerConfig};
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
}

impl LineBuffer {
    pub fn push(&self, line: &str) {
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

/// A running server. Dropping the handle kills the process, so a server never outlives the app
/// by accident.
#[derive(Debug)]
pub struct ServerHandle {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    log: Arc<LineBuffer>,
    pub port: u16,
    pub started: Instant,
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
        self.log.push(&format!("> {command}"));
        Ok(())
    }

    /// Ask the server to `quit`, wait up to `grace`, then kill it.
    pub fn stop(&self, grace: Duration) -> Result<StopOutcome> {
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
        port: config.port,
        started: Instant::now(),
    })
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
