//! `dah-agent`: the Rust TinyAgent v2 host for the Deadlock workbench.
//!
//! A library with a thin `dah-agent` binary. Sigil runs the same entry as `Sigil agent-host`,
//! still as its own child process: only the file is shared.
//!
//! Newline-delimited JSON on stdin and stdout. stdout carries protocol
//! envelopes only; diagnostics go to stderr. See `README.md` for the commands
//! and the approval flow.

mod config;
mod dah_rs;
mod host;
mod payload;
mod prompt;
mod provider;
mod redact;
mod tools;

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Stdout},
    sync::Mutex,
};

/// A single command line may not exceed this many UTF-8 bytes.
const MAX_LINE_BYTES: usize = 1_000_000;

/// Serialized access to stdout. Every envelope from the host and from a run
/// passes through here, so two tasks never interleave a line.
#[derive(Clone)]
pub struct Writer {
    out: Arc<Mutex<Stdout>>,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            out: Arc::new(Mutex::new(tokio::io::stdout())),
        }
    }

    pub async fn send(&self, value: &Value) {
        let mut value = value.clone();
        redact::value(&mut value);
        let mut text = match serde_json::to_string(&value) {
            Ok(text) => text,
            Err(error) => {
                log(&format!("could not encode an envelope: {error}"));
                return;
            }
        };
        text.push('\n');
        let mut out = self.out.lock().await;
        if let Err(error) = out.write_all(text.as_bytes()).await {
            log(&format!("could not write to stdout: {error}"));
            return;
        }
        let _ = out.flush().await;
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

/// A diagnostic line. Never stdout, never unredacted.
pub fn log(line: &str) {
    eprintln!("[agent-host] {}", redact::secrets(line));
}

pub fn error_envelope(id: Option<&str>, code: &str, message: &str) -> Value {
    json!({
        "id": id,
        "ok": false,
        "error": { "code": code, "message": message },
    })
}

/// How the host reaches the `dah` CLI for its tools when `DAH_RS_BIN` is not set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliLaunch {
    /// The standalone `dah-agent`: a sibling `dah-rs` or `dah`, the repo debug build, then PATH.
    Search,
    /// Inside Sigil: this same executable with the `cli` subcommand.
    SelfCli,
}

/// Run the host until stdin closes or a shutdown command arrives.
pub fn run(launch: CliLaunch) -> std::process::ExitCode {
    // Move the data folder from its old name before the sessions or workspace are touched.
    // Started by the app, `SIGIL_DATA_DIR` is set and this does nothing.
    if let Some(line) = deadlock_core::app_data::init().log_line() {
        log(&line);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log(&format!("could not start the async runtime: {error}"));
            return std::process::ExitCode::FAILURE;
        }
    };
    runtime.block_on(serve(launch));
    std::process::ExitCode::SUCCESS
}

async fn serve(launch: CliLaunch) {
    let writer = Writer::new();

    let config = match config::Config::load() {
        Ok(config) => config,
        Err(message) => {
            log(&message);
            writer
                .send(&error_envelope(None, "config_error", &message))
                .await;
            std::process::exit(1);
        }
    };

    if let Err(message) = config.prepare() {
        log(&message);
        writer
            .send(&error_envelope(None, "workspace_error", &message))
            .await;
        std::process::exit(1);
    }

    let cli = Arc::new(dah_rs::DahRsCli::new(
        config.dah_rs_bin.clone(),
        launch,
        config.workspace_dir.clone(),
        config.data_dir.clone(),
    ));

    let agent = match provider::build_agent(&config, cli.clone()) {
        Ok(agent) => agent,
        Err(error) => {
            let message = error.to_string();
            log(&message);
            writer
                .send(&error_envelope(None, "config_error", &message))
                .await;
            std::process::exit(1);
        }
    };

    log(&format!(
        "TinyAgent host ready: provider={} model={} sessions={} workspace={} dah-rs={}",
        config.provider.as_str(),
        config.model,
        config.session_dir.display(),
        config.workspace_dir.display(),
        cli.binary()
    ));

    let mut host = host::Host::new(writer.clone(), agent, config.session_dir.clone());
    let mut input = BufReader::new(tokio::io::stdin());
    let mut buffer: Vec<u8> = Vec::new();
    let mut keep_running = true;

    while keep_running {
        buffer.clear();
        match input.read_until(b'\n', &mut buffer).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                log(&format!("could not read stdin: {error}"));
                break;
            }
        }

        if buffer.len() > MAX_LINE_BYTES {
            writer
                .send(&error_envelope(
                    None,
                    "line_too_large",
                    &format!("Input line exceeds {MAX_LINE_BYTES} bytes."),
                ))
                .await;
            continue;
        }

        let line = match std::str::from_utf8(&buffer) {
            Ok(line) => line.trim(),
            Err(_) => {
                writer
                    .send(&error_envelope(
                        None,
                        "invalid_utf8",
                        "Input line is not valid UTF-8.",
                    ))
                    .await;
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        keep_running = host.dispatch(line).await;
    }

    if keep_running {
        host.shutdown_quiet().await;
    }
}
