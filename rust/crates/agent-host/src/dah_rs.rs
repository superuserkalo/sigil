//! The `dah-rs` child process. Arguments are passed as an array, never a
//! shell string, so a path or query cannot become a second command. Provider
//! credentials are removed from the child environment.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde_json::Value;
use tiny_agent::{CancellationToken, Error, Result};
use tokio::process::Command;

use crate::redact;

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const EXCERPT_CHARS: usize = 2_000;

/// Environment names that never reach the child.
const SECRET_ENV: [&str; 3] = ["DAH_AGENT_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

pub struct DahRsCli {
    binary: String,
    workspace: PathBuf,
    data_dir: PathBuf,
}

impl DahRsCli {
    pub fn new(configured: Option<String>, workspace: PathBuf, data_dir: PathBuf) -> Self {
        let binary = configured.unwrap_or_else(|| default_binary(&workspace));
        Self {
            binary,
            workspace,
            data_dir,
        }
    }

    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// Run the CLI and return its compact JSON. A failure becomes a tool error
    /// the model can read, with stderr excerpted.
    pub async fn json(&self, args: &[String], cancellation: &CancellationToken) -> Result<String> {
        let label = if args.is_empty() {
            "dah-rs".to_string()
        } else {
            format!("dah-rs {}", args.join(" "))
        };

        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("DAH_DATA_DIR", &self.data_dir)
            .env("DAH_WORKSPACE_DIR", &self.workspace)
            .kill_on_drop(true);
        for name in SECRET_ENV {
            command.env_remove(name);
        }

        let output = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(Error::Cancelled),
            result = tokio::time::timeout(TIMEOUT, command.output()) => result,
        };

        let output = match output {
            Err(_) => {
                return Err(Error::Tool(format!(
                    "{label} stopped after {} s. Narrow the query or retry.",
                    TIMEOUT.as_secs()
                )));
            }
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Tool(format!(
                    "Could not run the dah-rs CLI at {}. Build it with `cargo build -p dah-rs` or set DAH_RS_BIN.",
                    self.binary
                )));
            }
            Ok(Err(error)) => {
                return Err(Error::Tool(format!("{label} failed. {error}")));
            }
            Ok(Ok(output)) => output,
        };

        if output.stdout.len() > MAX_OUTPUT_BYTES {
            return Err(Error::Tool(format!(
                "{label} returned more than {} bytes on stdout.",
                MAX_OUTPUT_BYTES
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if let Some(value) = parse_json(&stdout) {
            return Ok(value.to_string());
        }

        if !output.status.success() {
            let status = match output.status.code() {
                Some(code) => format!(" (exit code {code})"),
                None => String::new(),
            };
            return Err(Error::Tool(format!(
                "{label} failed{status}.{}",
                excerpt_block(&stderr, &stdout)
            )));
        }

        Err(Error::Tool(format!(
            "{label} returned no JSON on stdout.{}",
            excerpt_block(&stderr, &stdout)
        )))
    }
}

/// The configured binary, a sibling of this executable, the repo debug build,
/// or PATH. The CLI crate currently names its binary `dah`, so both names are
/// accepted.
fn default_binary(workspace: &Path) -> String {
    for name in ["dah-rs", "dah"] {
        let file_name = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        };

        if let Ok(executable) = std::env::current_exe()
            && let Some(directory) = executable.parent()
        {
            let candidate = directory.join(&file_name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }

        let mut directory = Some(workspace);
        for _ in 0..6 {
            let Some(current) = directory else { break };
            let candidate = current
                .join("rust")
                .join("target")
                .join("debug")
                .join(&file_name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
            directory = current.parent();
        }
    }

    if cfg!(windows) {
        "dah-rs.exe".to_string()
    } else {
        "dah-rs".to_string()
    }
}

fn parse_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed).ok()
}

fn excerpt(text: &str) -> String {
    let trimmed = redact::secrets(text.trim());
    if trimmed.chars().count() <= EXCERPT_CHARS {
        trimmed
    } else {
        let head: String = trimmed.chars().take(EXCERPT_CHARS).collect();
        format!("{head}\n[... excerpt truncated ...]")
    }
}

fn excerpt_block(stderr: &str, stdout: &str) -> String {
    let mut block = String::new();
    if !stderr.trim().is_empty() {
        block.push_str(&format!("\nstderr:\n{}", excerpt(stderr)));
    }
    if !stdout.trim().is_empty() {
        block.push_str(&format!("\nstdout:\n{}", excerpt(stdout)));
    }
    block
}
