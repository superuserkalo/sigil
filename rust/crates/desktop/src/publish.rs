//! GameBanana publish commands: one background process, no render-thread blocking.
//!
//! The CLI is the same `dah` binary the rest of the harness uses. Each command
//! runs on its own thread, captures stdout and stderr, and sends one outcome
//! back through a channel. `app::poll` drains it, updates the status line and
//! re-reads `upload.json`. The app never uploads a file itself: `--approve`
//! records the handoff and `--open` hands the page to the user's browser.

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use rune::Waker;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PublishAction {
    Upload,
    Confirm,
}

impl PublishAction {
    pub fn label(self) -> &'static str {
        match self {
            PublishAction::Upload => "GameBanana handoff",
            PublishAction::Confirm => "Submission confirm",
        }
    }
}

/// One finished command. `ok` is the real exit status, never assumed.
#[derive(Debug)]
pub struct PublishOutcome {
    pub action: PublishAction,
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub spawn_error: Option<String>,
}

impl PublishOutcome {
    fn failed(action: PublishAction, message: String) -> PublishOutcome {
        PublishOutcome {
            action,
            ok: false,
            code: None,
            stdout: String::new(),
            stderr: message,
            spawn_error: None,
        }
    }

    pub fn summary(&self) -> String {
        let exit = match self.code {
            Some(code) => format!("exit {code}"),
            None => "no exit code".to_string(),
        };
        let mut parts = vec![format!(
            "{} {} ({exit})",
            self.action.label(),
            if self.ok { "finished" } else { "failed" }
        )];
        if let Some(error) = &self.spawn_error {
            parts.push(error.clone());
        }
        if !self.stdout.trim().is_empty() {
            parts.push(format!("stdout: {}", tail(&self.stdout, 220)));
        }
        if !self.stderr.trim().is_empty() {
            parts.push(format!("stderr: {}", tail(&self.stderr, 220)));
        }
        parts.join(" · ")
    }
}

/// The running command, if any, and the channel it reports on.
#[derive(Default)]
pub struct PublishJob {
    running: bool,
    action: Option<PublishAction>,
    rx: Option<Receiver<PublishOutcome>>,
}

impl PublishJob {
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn label(&self) -> Option<&'static str> {
        self.action.map(PublishAction::label)
    }

    /// Start one CLI command on a background thread. Returns the reason when
    /// the binary or the workspace is missing, so the UI can say so at once.
    pub fn start(
        &mut self,
        action: PublishAction,
        workspace: &Path,
        url: Option<&str>,
        wake: Option<Waker>,
    ) -> Result<(), String> {
        if self.running {
            return Err("A publish command is already running.".to_string());
        }
        if workspace.as_os_str().is_empty() {
            return Err(
                "No project workspace is open, so the publish command has no target.".to_string(),
            );
        }
        if workspace.to_string_lossy().trim() == "Not selected" {
            return Err(
                "Open a mod project first. The publish command reads project.json and its release."
                    .to_string(),
            );
        }
        let binary = resolve_binary()?;
        let args = match action {
            PublishAction::Upload => vec![
                "publish".to_string(),
                "upload".to_string(),
                "--workspace".to_string(),
                workspace.display().to_string(),
                "--approve".to_string(),
                "--open".to_string(),
            ],
            PublishAction::Confirm => {
                let url = url
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .ok_or_else(|| "Enter the submission URL before confirming.".to_string())?;
                let mut args = vec![
                    "publish".to_string(),
                    "confirm".to_string(),
                    "--workspace".to_string(),
                    workspace.display().to_string(),
                    "--url".to_string(),
                    url.to_string(),
                ];
                if is_local_http(url) {
                    args.push("--allow-http".to_string());
                }
                args
            }
        };

        let (tx, rx) = mpsc::channel();
        let action_for_thread = action;
        let binary_for_thread = binary.clone();
        let spawned = thread::Builder::new()
            .name("dah-publish".to_string())
            .spawn(move || {
                let outcome = match binary_for_thread.command().args(&args).output() {
                    Ok(output) => PublishOutcome {
                        action: action_for_thread,
                        ok: output.status.success(),
                        code: output.status.code(),
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                        spawn_error: None,
                    },
                    Err(error) => PublishOutcome {
                        action: action_for_thread,
                        ok: false,
                        code: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        spawn_error: Some(format!(
                            "Could not run {}: {error}.",
                            binary_for_thread.label()
                        )),
                    },
                };
                let _ = tx.send(outcome);
                if let Some(wake) = wake {
                    wake();
                }
            });

        if let Err(error) = spawned {
            return Err(format!("Could not start the publish worker: {error}"));
        }

        self.running = true;
        self.action = Some(action);
        self.rx = Some(rx);
        Ok(())
    }

    /// Drain one outcome. Called from `poll`, never from the render loop body.
    pub fn try_outcome(&mut self) -> Option<PublishOutcome> {
        let received = {
            let Some(rx) = self.rx.as_ref() else {
                return None;
            };
            match rx.try_recv() {
                Ok(outcome) => Ok(outcome),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err(()),
            }
        };
        let action = self.action.unwrap_or(PublishAction::Upload);
        self.rx = None;
        self.running = false;
        self.action = None;
        match received {
            Ok(outcome) => Some(outcome),
            Err(()) => Some(PublishOutcome::failed(
                action,
                "The publish worker stopped without reporting a result.".to_string(),
            )),
        }
    }
}

/// `--allow-http` is only ever passed for a local test server, matching the CLI
/// rule in `dah publish confirm`.
pub fn is_local_http(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
}

/// The CLI: `DAH_RS_BIN`, else this executable as `cli` (`crate::helpers`).
fn resolve_binary() -> Result<crate::helpers::Helper, String> {
    crate::helpers::cli()
}

/// The last lines the user needs, collapsed onto one status line.
fn tail(text: &str, limit: usize) -> String {
    let words: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let joined = words.join(" · ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut out: String = joined.chars().take(limit).collect();
    out.push_str(" ...");
    out
}
