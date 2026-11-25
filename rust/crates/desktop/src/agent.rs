//! The `dah-agent` client: one child process, JSONL on stdin and stdout.
//!
//! The desktop app is the parent. It spawns the Rust TinyAgent host, keeps it alive for the
//! session, writes one command per line and drains one envelope per line. A reader thread parses
//! stdout and a second thread forwards stderr diagnostics. Both wake the window through the
//! waker Rune hands to `AppSpec::setup`, so the render loop never blocks on the pipe.
//!
//! The protocol is exactly the one in `rust/crates/agent-host/README.md`; there is no second
//! protocol and no shell tool here.

use std::{
    collections::HashMap,
    env,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Stdio},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use rune::Waker;
use serde_json::{Value, json};

/// Lifecycle of the host process and its active run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentStatus {
    Starting,
    Ready,
    Running,
    Paused,
    Error,
    Stopped,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Starting => "starting",
            AgentStatus::Ready => "ready",
            AgentStatus::Running => "running",
            AgentStatus::Paused => "paused",
            AgentStatus::Error => "error",
            AgentStatus::Stopped => "stopped",
        }
    }
}

/// Token usage as the host reports it, camelCase on the wire.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
}

impl Usage {
    pub fn summary(self) -> String {
        format!(
            "{} in / {} out · {} request{}",
            self.input_tokens,
            self.output_tokens,
            self.requests,
            if self.requests == 1 { "" } else { "s" }
        )
    }
}

/// The durable pause the host reports for an approval-gated tool.
#[derive(Clone, Debug)]
pub struct Pause {
    pub tool_call_id: String,
    pub tool_name: String,
    pub payload: Value,
}

impl Pause {
    pub fn payload_text(&self) -> String {
        serde_json::to_string_pretty(&self.payload).unwrap_or_else(|_| self.payload.to_string())
    }
}

/// What a result envelope answers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandKind {
    Prompt,
    Resume,
    Abort,
    State,
    ListSessions,
    NewSession,
    Shutdown,
}

/// One successful reply envelope.
#[derive(Debug)]
pub struct ResultEnvelope {
    pub id: Option<String>,
    pub kind: Option<CommandKind>,
    pub session_id: Option<String>,
    pub text: String,
    pub pause: Option<Pause>,
    pub usage: Option<Usage>,
    pub aborted: Option<bool>,
    pub session_ids: Vec<String>,
    pub previous_session_id: Option<String>,
    pub state_running: bool,
}

/// One failed reply or host-level error envelope.
#[derive(Clone, Debug)]
pub struct HostError {
    pub id: Option<String>,
    pub code: String,
    pub message: String,
}

/// Everything the reader threads deliver to the UI.
#[derive(Debug)]
pub enum AgentEvent {
    /// A reply to a command, with its kind attached once the link correlates the id.
    Result(ResultEnvelope),
    /// A raw TinyAgent event object: agent_start, text_delta, tool_execution_start and so on.
    Event(Value),
    /// An `ok:false` envelope from the host.
    Error(HostError),
    /// A stderr diagnostic line. Never contains a key by host contract.
    Diagnostic(String),
    /// stdout closed. The host has exited.
    Eof,
}

/// The live connection to `dah-agent`.
pub struct AgentLink {
    pub status: AgentStatus,
    pub binary: Option<String>,
    pub provider: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub key_present: bool,
    pub configured: bool,
    pub demo: bool,
    pub session_id: Option<String>,
    pub previous_session_id: Option<String>,
    pub session_ids: Vec<String>,
    pub pause: Option<Pause>,
    pub usage: Option<Usage>,
    pub error: Option<String>,
    pub diagnostic: Option<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: Option<Receiver<AgentEvent>>,
    pending: HashMap<String, CommandKind>,
    next_id: u64,
    shutting_down: bool,
}

impl AgentLink {
    /// Start the host. A missing model or binary returns a link in the error state, so the UI
    /// always has something to render and an explicit reason.
    pub fn start(wake: Waker) -> AgentLink {
        // `<data>/host.json` wins over the environment. The Settings screen and
        // `dah auth use` write it after a subscription sign-in.
        let host = deadlock_core::subscription::load_host_config()
            .ok()
            .flatten();
        let provider = host
            .as_ref()
            .map(|host| host.provider.clone())
            .or_else(|| env_value("DAH_AGENT_PROVIDER"))
            .unwrap_or_else(|| "openai".to_string());
        let model = host
            .as_ref()
            .map(|host| host.model.clone())
            .or_else(|| env_value("DAH_AGENT_MODEL"));
        let base_url = env_value("DAH_AGENT_BASE_URL");
        let subscription = matches!(provider.as_str(), "codex" | "grok");
        let subscription_login = subscription
            && deadlock_core::subscription::Provider::parse(&provider)
                .and_then(|provider| deadlock_core::subscription::load(provider).ok().flatten())
                .is_some();
        let key_present = if subscription {
            subscription_login
        } else {
            match provider.as_str() {
                "chat" => true,
                "anthropic" => {
                    env_value("DAH_AGENT_API_KEY").is_some()
                        || env_value("ANTHROPIC_API_KEY").is_some()
                }
                _ => {
                    env_value("DAH_AGENT_API_KEY").is_some()
                        || env_value("OPENAI_API_KEY").is_some()
                }
            }
        };
        let mut link = AgentLink::blank(provider.clone(), model.clone());
        link.base_url = base_url.clone();
        link.key_present = key_present;

        if model.is_none() {
            return AgentLink::config_error(
                link,
                "DAH_AGENT_MODEL is not set. Set the exact model id for the configured provider, or run `Sigil cli auth use <provider> --model <id>`.",
            );
        }
        if !matches!(
            provider.as_str(),
            "openai" | "anthropic" | "chat" | "codex" | "grok"
        ) {
            return AgentLink::config_error(
                link,
                &format!(
                    "DAH_AGENT_PROVIDER is {provider:?}. Use openai, anthropic, chat, codex or grok."
                ),
            );
        }
        if subscription && !subscription_login {
            return AgentLink::config_error(
                link,
                &format!(
                    "No {} login yet. Run `Sigil cli auth login {}` in a terminal, then restart the app.",
                    provider, provider
                ),
            );
        }

        let binary = match resolve_binary() {
            Ok(binary) => binary,
            Err(message) => return AgentLink::config_error(link, &message),
        };
        link.binary = Some(binary.label());
        link.configured = true;

        let mut command = binary.command();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("DAH_AGENT_PROVIDER", &provider);
        if let Some(model) = &model {
            command.env("DAH_AGENT_MODEL", model);
        }
        if let Some(base_url) = &base_url {
            command.env("DAH_AGENT_BASE_URL", base_url);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                link.status = AgentStatus::Error;
                link.error = Some(format!(
                    "Could not start {}: {error}.",
                    binary.label()
                ));
                return link;
            }
        };

        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            link.status = AgentStatus::Error;
            link.error = Some("The host started without a stdin pipe.".to_string());
            return link;
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::channel();
        if let Some(stdout) = stdout {
            spawn_stdout_reader(stdout, tx.clone(), wake.clone());
        } else {
            let _ = child.kill();
            let _ = child.wait();
            link.status = AgentStatus::Error;
            link.error = Some("The host started without a stdout pipe.".to_string());
            return link;
        }
        if let Some(stderr) = stderr {
            spawn_stderr_reader(stderr, tx, wake);
        }

        link.status = AgentStatus::Starting;
        link.child = Some(child);
        link.stdin = Some(stdin);
        link.rx = Some(rx);
        link
    }

    /// A labelled fixture link for the demo scenes. No process is spawned.
    pub fn demo() -> AgentLink {
        let mut link = AgentLink::blank("fixture".to_string(), Some("qwen3-coder-30b".to_string()));
        link.demo = true;
        link.status = AgentStatus::Ready;
        link.session_id = Some("demo-session".to_string());
        link
    }

    /// A link with no process, used as the default before `setup` runs.
    /// Whether a host process was started for this link.
    #[cfg(test)]
    pub fn has_process(&self) -> bool {
        self.child.is_some()
    }

    pub fn disconnected() -> AgentLink {
        let provider = env_value("DAH_AGENT_PROVIDER").unwrap_or_else(|| "openai".to_string());
        let model = env_value("DAH_AGENT_MODEL");
        let mut link = AgentLink::blank(provider, model);
        link.status = AgentStatus::Stopped;
        link
    }

    /// A link that carries a configuration error and runs no process.
    pub fn config_error_from(message: &str) -> AgentLink {
        let mut link = AgentLink::disconnected();
        link.status = AgentStatus::Error;
        link.error = Some(message.to_string());
        link
    }

    fn blank(provider: String, model: Option<String>) -> AgentLink {
        AgentLink {
            status: AgentStatus::Stopped,
            binary: None,
            provider,
            model,
            base_url: None,
            key_present: false,
            configured: false,
            demo: false,
            session_id: None,
            previous_session_id: None,
            session_ids: Vec::new(),
            pause: None,
            usage: None,
            error: None,
            diagnostic: None,
            child: None,
            stdin: None,
            rx: None,
            pending: HashMap::new(),
            next_id: 0,
            shutting_down: false,
        }
    }

    fn config_error(mut link: AgentLink, message: &str) -> AgentLink {
        link.status = AgentStatus::Error;
        link.error = Some(message.to_string());
        link
    }

    /// A short label for the host card and Settings.
    pub fn binary_label(&self) -> String {
        if self.demo {
            return "demo fixture, no process".to_string();
        }
        match &self.binary {
            Some(label) => label.clone(),
            None => "not found".to_string(),
        }
    }

    pub fn model_label(&self) -> String {
        self.model.clone().unwrap_or_else(|| "no model".to_string())
    }

    pub fn provider_label(&self) -> String {
        format!("{} / {}", self.provider, self.model_label())
    }

    pub fn session_label(&self) -> String {
        self.session_id
            .clone()
            .unwrap_or_else(|| "none".to_string())
    }

    pub fn usage_label(&self) -> String {
        match self.usage {
            Some(usage) => usage.summary(),
            None => "no usage yet".to_string(),
        }
    }

    /// Send a prompt. Returns false when the pipe is gone; the caller shows `error`.
    pub fn prompt(&mut self, text: &str) -> bool {
        if !self.configured || self.demo {
            return false;
        }
        let sent = self.send(json!({"type": "prompt", "text": text}), CommandKind::Prompt);
        if sent {
            self.status = AgentStatus::Running;
            self.pause = None;
            self.error = None;
        }
        sent
    }

    /// Answer a paused tool call with a result value or an error string.
    pub fn resume(&mut self, answer: Result<Value, String>) -> bool {
        let command = match answer {
            Ok(result) => json!({"type": "resume", "result": result}),
            Err(message) => json!({"type": "resume", "error": message}),
        };
        let sent = self.send(command, CommandKind::Resume);
        if sent {
            self.status = AgentStatus::Running;
            self.pause = None;
        }
        sent
    }

    /// Approve a paused tool call with a result value.
    pub fn resume_result(&mut self, result: Value) -> bool {
        self.resume(Ok(result))
    }

    /// Refuse a paused tool call with an error message.
    pub fn resume_error(&mut self, message: &str) -> bool {
        self.resume(Err(message.to_string()))
    }

    pub fn abort(&mut self) -> bool {
        self.send(json!({"type": "abort"}), CommandKind::Abort)
    }

    pub fn state(&mut self) -> bool {
        self.send(json!({"type": "state"}), CommandKind::State)
    }

    pub fn list_sessions(&mut self) -> bool {
        self.send(json!({"type": "list_sessions"}), CommandKind::ListSessions)
    }

    pub fn new_session(&mut self) -> bool {
        let sent = self.send(json!({"type": "new_session"}), CommandKind::NewSession);
        if sent {
            self.pause = None;
        }
        sent
    }

    /// Ask the host to stop and wait briefly for it to settle its journal.
    pub fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        if self.stdin.is_some() {
            self.send(json!({"type": "shutdown"}), CommandKind::Shutdown);
        }
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_millis(400);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        }
        self.rx = None;
        self.status = AgentStatus::Stopped;
    }

    /// Drain one event without blocking, correlating result ids with the command kind.
    pub fn try_event(&mut self) -> Option<AgentEvent> {
        let received = {
            let Some(rx) = self.rx.as_ref() else {
                return None;
            };
            match rx.try_recv() {
                Ok(event) => Ok(event),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err(()),
            }
        };
        let event = match received {
            Ok(event) => event,
            Err(()) => {
                self.rx = None;
                self.status = AgentStatus::Stopped;
                if self.error.is_none() {
                    self.error = Some(
                        "The agent host reader stopped without an exit envelope. Restart the host."
                            .to_string(),
                    );
                }
                return Some(AgentEvent::Eof);
            }
        };
        match event {
            AgentEvent::Result(mut result) => {
                self.error = None;
                if let Some(id) = &result.id
                    && let Some(kind) = self.pending.remove(id)
                {
                    result.kind = Some(kind);
                }
                if result.session_id.is_some() {
                    self.session_id = result.session_id.clone();
                }
                if result.usage.is_some() {
                    self.usage = result.usage;
                }
                if result.state_running {
                    self.status = AgentStatus::Running;
                }
                if !result.session_ids.is_empty() {
                    self.session_ids = result.session_ids.clone();
                }
                if result.previous_session_id.is_some() {
                    self.previous_session_id = result.previous_session_id.clone();
                }
                if let Some(pause) = &result.pause {
                    self.pause = Some(pause.clone());
                    self.status = AgentStatus::Paused;
                }
                if result.aborted == Some(true) {
                    self.status = AgentStatus::Running;
                }
                Some(AgentEvent::Result(result))
            }
            AgentEvent::Error(error) => {
                if let Some(id) = &error.id {
                    self.pending.remove(id);
                }
                if error.code == "config_error" || error.code == "workspace_error" {
                    self.status = AgentStatus::Error;
                } else if error.code == "busy" {
                    self.status = AgentStatus::Running;
                }
                self.error = Some(format!("{}: {}", error.code, error.message));
                Some(AgentEvent::Error(error))
            }
            AgentEvent::Diagnostic(line) => {
                if self.status == AgentStatus::Starting && line.contains("host ready") {
                    self.status = AgentStatus::Ready;
                }
                self.diagnostic = Some(line.clone());
                Some(AgentEvent::Diagnostic(line))
            }
            AgentEvent::Eof => {
                let was_shutting_down = self.shutting_down;
                self.stdin = None;
                let code = self
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
                    .and_then(|status| status.code());
                let expected = was_shutting_down || self.status == AgentStatus::Stopped;
                self.child = None;
                self.status = AgentStatus::Stopped;
                if !expected && self.error.is_none() {
                    self.error = Some(match code {
                        Some(code) => format!(
                            "The agent host exited with code {code}. Restart it from Settings."
                        ),
                        None => {
                            "The agent host exited. Restart it from Settings.".to_string()
                        }
                    });
                }
                Some(AgentEvent::Eof)
            }
            other => Some(other),
        }
    }

    fn send(&mut self, mut command: Value, kind: CommandKind) -> bool {
        self.next_id += 1;
        let id = self.next_id.to_string();
        command["id"] = json!(id);
        let Some(stdin) = self.stdin.as_mut() else {
            self.error =
                Some("The agent host is not running. Restart it from Settings.".to_string());
            self.status = AgentStatus::Error;
            return false;
        };
        let Ok(mut line) = serde_json::to_string(&command) else {
            self.error = Some("Could not encode the host command.".to_string());
            self.status = AgentStatus::Error;
            return false;
        };
        line.push('\n');
        if let Err(error) = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()) {
            self.error = Some(format!("Could not write to the agent host: {error}"));
            self.status = AgentStatus::Error;
            self.stdin = None;
            return false;
        }
        self.pending.insert(id, kind);
        true
    }
}

impl Drop for AgentLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The host: `DAH_AGENT_BIN`, else this executable as `agent-host` (`crate::helpers`).
fn resolve_binary() -> Result<crate::helpers::Helper, String> {
    crate::helpers::agent_host()
}

fn spawn_stdout_reader(stdout: std::process::ChildStdout, tx: Sender<AgentEvent>, wake: Waker) {
    let fallback = tx.clone();
    let spawned = thread::Builder::new()
        .name("dah-agent-stdout".to_string())
        .spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let Some(event) = parse_envelope(&line) else {
                    continue;
                };
                if tx.send(event).is_err() {
                    return;
                }
                wake();
            }
            let _ = tx.send(AgentEvent::Eof);
            wake();
        });
    if spawned.is_err() {
        let _ = fallback.send(AgentEvent::Eof);
    }
}

fn spawn_stderr_reader(stderr: std::process::ChildStderr, tx: Sender<AgentEvent>, wake: Waker) {
    let _ = thread::Builder::new()
        .name("dah-agent-stderr".to_string())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if tx.send(AgentEvent::Diagnostic(line)).is_err() {
                    return;
                }
                wake();
            }
        });
}

fn parse_envelope(line: &str) -> Option<AgentEvent> {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Some(AgentEvent::Diagnostic(line.to_string())),
    };
    if let Some(event) = value.get("event") {
        return Some(AgentEvent::Event(event.clone()));
    }
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => Some(AgentEvent::Result(ResultEnvelope {
            id: opt_string(&value, "id"),
            kind: None,
            session_id: opt_string(&value, "sessionId"),
            text: opt_string(&value, "text").unwrap_or_default(),
            pause: value.get("pause").and_then(parse_pause),
            usage: value.get("usage").and_then(parse_usage),
            aborted: value.get("aborted").and_then(Value::as_bool),
            session_ids: value
                .get("sessionIds")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            previous_session_id: opt_string(&value, "previousSessionId"),
            state_running: value
                .get("state")
                .and_then(|state| state.get("status"))
                .and_then(Value::as_str)
                .map(|status| status == "running")
                .unwrap_or(false),
        })),
        Some(false) => {
            let error = value.get("error");
            Some(AgentEvent::Error(HostError {
                id: opt_string(&value, "id"),
                code: error
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or("internal_error")
                    .to_string(),
                message: error
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("The host returned an error envelope.")
                    .to_string(),
            }))
        }
        None => Some(AgentEvent::Diagnostic(line.to_string())),
    }
}

fn opt_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn parse_usage(value: &Value) -> Option<Usage> {
    if !value.is_object() {
        return None;
    }
    Some(Usage {
        input_tokens: value
            .get("inputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        requests: value.get("requests").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub(crate) fn parse_pause(value: &Value) -> Option<Pause> {
    let tool_name = value.get("toolName").and_then(Value::as_str)?;
    Some(Pause {
        tool_call_id: value
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tool_name: tool_name.to_string(),
        payload: value.get("payload").cloned().unwrap_or(Value::Null),
    })
}

pub(crate) fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str(" ...");
    out
}

fn env_value(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
