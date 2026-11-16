//! The workbench shell: state, scenes, sidebar, header, status bar and screen dispatch.

use std::collections::HashMap;
use std::time::Instant;

use rune::tok::motion;
use rune::{Color, Cx, El, Key, KeyEvent, TextEdit, Tx, Waker, col, el, row, spacer, text};
use serde_json::Value;

use crate::agent::{
    self, AgentEvent, AgentLink, AgentStatus, CommandKind, HostError, Pause, ResultEnvelope,
};
use crate::model::{
    self, Catalog, ImpactReport, IndexRow, Message, PatchNote, Paths, Project, ProjectState,
    Provider, ProviderState, ReleaseState, Role, Screen, ToolRun,
};
use crate::screens;
use crate::ui::{BODY, BODY_MED, H2, HEADER_H, LABEL, SIDEBAR_W, SMALL, STATUS_H, Tone, Ui};

pub struct State {
    pub screen: Screen,
    pub demo: bool,
    pub projects: Vec<Project>,
    pub selected_project: usize,
    pub catalog: Option<Catalog>,
    pub index: Vec<IndexRow>,
    pub index_query: TextEdit,
    pub hero_filter: Option<String>,
    pub type_filter: Option<String>,
    pub vpk_filter: Option<String>,
    pub index_selected: Option<usize>,
    pub patches: Vec<PatchNote>,
    pub selected_patch: usize,
    pub impact: Option<ImpactReport>,
    pub release: ReleaseState,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolRun>,
    pub composer: TextEdit,
    pub providers: Vec<Provider>,
    pub local_endpoint: TextEdit,
    pub paths: Paths,
    pub confirm_writes: bool,
    pub auto_recheck: bool,
    pub check_updates: bool,
    pub status: String,
    pub pulse: f32,
    pub game_detected: bool,
    pub game_label: String,
    pub model_label: String,
    pub elapsed: String,
    pub agent: AgentLink,
    pub wake: Option<Waker>,
    pub streaming: Option<usize>,
    pub run_started: Option<Instant>,
    pub tool_started: HashMap<String, Instant>,
}

impl Default for State {
    fn default() -> Self {
        State {
            screen: Screen::Workbench,
            demo: false,
            projects: Vec::new(),
            selected_project: 0,
            catalog: None,
            index: Vec::new(),
            index_query: TextEdit::new(""),
            hero_filter: None,
            type_filter: None,
            vpk_filter: None,
            index_selected: None,
            patches: Vec::new(),
            selected_patch: 0,
            impact: None,
            release: ReleaseState::default(),
            messages: model::welcome_messages(),
            tools: Vec::new(),
            composer: TextEdit::new(""),
            providers: model::default_providers(),
            local_endpoint: TextEdit::new(""),
            paths: model::empty_paths(),
            confirm_writes: true,
            auto_recheck: true,
            check_updates: false,
            status: String::new(),
            pulse: 0.0,
            game_detected: false,
            game_label: "Game not detected".to_string(),
            model_label: "No model".to_string(),
            elapsed: "0s".to_string(),
            agent: AgentLink::disconnected(),
            wake: None,
            streaming: None,
            run_started: None,
            tool_started: HashMap::new(),
        }
    }
}

impl State {
    pub fn project(&self) -> Option<&Project> {
        self.projects.get(self.selected_project)
    }

    pub fn provider(&self) -> Option<&Provider> {
        self.providers
            .iter()
            .find(|p| p.state == ProviderState::Configured)
    }

    pub fn provider_ready(&self) -> bool {
        self.provider().is_some()
    }

    pub fn game_tone(&self) -> Tone {
        if self.game_detected {
            Tone::Ok
        } else {
            Tone::Warn
        }
    }

    pub fn provider_tone(&self) -> Tone {
        if self.provider_ready() {
            Tone::Ok
        } else {
            Tone::Warn
        }
    }

    pub fn load_demo(&mut self) {
        self.demo = true;
        self.projects = model::demo_projects();
        self.selected_project = 0;
        self.catalog = Some(model::demo_catalog());
        self.index = model::demo_index();
        self.patches = model::demo_patches();
        self.selected_patch = 0;
        self.impact = Some(model::demo_impact());
        self.release = model::demo_release_state();
        self.messages = model::demo_messages();
        self.tools = model::demo_tools();
        self.providers = model::demo_providers();
        self.local_endpoint = TextEdit::new("http://127.0.0.1:8080/v1");
        self.paths = model::demo_paths();
        self.game_detected = true;
        self.game_label = "citadel · build 2210".to_string();
        self.model_label = "qwen3-coder-30b".to_string();
        self.elapsed = "2m 14s".to_string();
        self.status = "Demo fixture loaded. Nothing on this screen reads or writes a game install."
            .to_string();
    }

    pub fn load_empty(&mut self) {
        let agent = std::mem::replace(&mut self.agent, AgentLink::disconnected());
        let wake = self.wake.take();
        *self = State::default();
        self.agent = agent;
        self.wake = wake;
        self.paths.data = crate::data::data_dir().display().to_string();
    }

    /// Start the Rust host for the real scenes and the window. Demo scenes get a labelled
    /// fixture link instead of a process, so snapshots stay hermetic.
    pub fn start_host(&mut self) {
        if self.demo {
            self.agent = AgentLink::demo();
            return;
        }
        let waker = self.wake.clone();
        self.agent = match waker {
            Some(wake) => AgentLink::start(wake),
            None => AgentLink::config_error_from(
                "The app has no UI waker yet, so the agent host cannot report back.",
            ),
        };
        self.apply_host_config();
        if self.agent.status == AgentStatus::Error
            && let Some(error) = &self.agent.error
        {
            self.status = error.clone();
        } else if self.agent.configured {
            self.status = format!(
                "Agent host starting: {} from {}.",
                self.agent.provider_label(),
                self.agent.binary_label()
            );
        }
    }

    pub fn restart_host(&mut self) {
        if self.demo {
            self.status =
                "Demo scene: the fixture host is not a process, so it cannot restart.".to_string();
            return;
        }
        self.agent.shutdown();
        self.agent = AgentLink::disconnected();
        self.streaming = None;
        self.run_started = None;
        self.tool_started.clear();
        self.start_host();
        if self.agent.status != AgentStatus::Error {
            self.status = "Agent host restarted. The session directory is shared, so stored sessions can be reopened by id."
                .to_string();
        }
    }

    fn apply_host_config(&mut self) {
        let provider = self.agent.provider.clone();
        let model = self.agent.model.clone().unwrap_or_default();
        let base_url = self.agent.base_url.clone();
        let configured = self.agent.configured && self.agent.key_present;
        for entry in &mut self.providers {
            let matches = match provider.as_str() {
                "anthropic" => entry.name == "Anthropic",
                "chat" => entry.kind == "localhost",
                _ => entry.name == "OpenAI",
            };
            if !matches {
                entry.state = ProviderState::NotConfigured;
                continue;
            }
            entry.state = if configured {
                ProviderState::Configured
            } else {
                ProviderState::NotConfigured
            };
            entry.detail = if configured {
                let endpoint = base_url
                    .as_deref()
                    .map(|url| format!(" · {url}"))
                    .unwrap_or_default();
                format!("{model}{endpoint} · key read by the host, never shown")
            } else if provider == "chat" {
                format!("{model} · local server, no key required")
            } else {
                format!("{model} · missing DAH_AGENT_API_KEY in the host environment")
            };
        }
        if let Some(model) = &self.agent.model {
            self.model_label = model.clone();
        }
    }

    pub fn send_prompt(&mut self) {
        let body = self.composer.text.trim().to_string();
        if body.is_empty() {
            self.status = "Type a prompt before sending.".to_string();
            return;
        }
        if self.agent.demo {
            self.messages.push(Message::user(body, "now"));
            self.composer = TextEdit::new("");
            self.messages.push(Message::assistant(
                "Demo fixture. This scene is not connected to the host, so the prompt was not sent and no model was called.",
                "now",
                None,
            ));
            self.status = "Demo fixture: the prompt stayed on screen.".to_string();
            return;
        }
        if self.agent.status == AgentStatus::Paused || self.agent.pause.is_some() {
            self.status =
                "Resolve the paused tool call first: approve or deny it in the workbench."
                    .to_string();
            return;
        }
        if self.agent.status == AgentStatus::Starting {
            self.status =
                "The agent host is still starting. Wait for ready, then send.".to_string();
            return;
        }
        if self.agent.status == AgentStatus::Error || self.agent.status == AgentStatus::Stopped {
            let reason = self.agent.error.clone().unwrap_or_else(|| {
                "The agent host is not running. Restart it from Settings.".to_string()
            });
            self.status = reason;
            return;
        }
        if self.agent.status == AgentStatus::Running {
            self.status =
                "A run is already active. Abort it before sending another prompt.".to_string();
            return;
        }

        self.messages.push(Message::user(body.clone(), "now"));
        self.composer = TextEdit::new("");
        self.streaming = None;
        self.run_started = Some(Instant::now());
        self.elapsed = "0s".to_string();
        if self.agent.prompt(&body) {
            self.status = format!(
                "Sent to {}. Streaming events into the transcript.",
                self.agent.provider_label()
            );
        } else {
            self.status = self
                .agent
                .error
                .clone()
                .unwrap_or_else(|| "The agent host did not accept the prompt.".to_string());
        }
    }

    /// Send a prompt from a screen action. The composer is filled first so the
    /// transcript and the input agree on what was asked.
    pub fn ask_agent(&mut self, body: impl Into<String>) {
        self.composer = TextEdit::new(body.into());
        self.send_prompt();
    }

    pub fn approve_pause(&mut self) {
        let Some(pause) = self.agent.pause.clone() else {
            self.status = "There is no paused tool call to approve.".to_string();
            return;
        };
        self.streaming = None;
        self.run_started = Some(Instant::now());
        if self
            .agent
            .resume_result(serde_json::json!("approved by user"))
        {
            self.status = format!("Approved {}. The run is resuming.", pause.tool_name);
        } else {
            self.status = self
                .agent
                .error
                .clone()
                .unwrap_or_else(|| "Could not send the approval.".to_string());
        }
    }

    pub fn deny_pause(&mut self) {
        let Some(pause) = self.agent.pause.clone() else {
            self.status = "There is no paused tool call to deny.".to_string();
            return;
        };
        self.streaming = None;
        self.run_started = Some(Instant::now());
        if self.agent.resume_error("denied by user") {
            self.status = format!(
                "Denied {}. The run resumes with the refusal in its transcript.",
                pause.tool_name
            );
        } else {
            self.status = self
                .agent
                .error
                .clone()
                .unwrap_or_else(|| "Could not send the denial.".to_string());
        }
    }

    pub fn abort_run(&mut self) {
        if self.agent.abort() {
            self.status = "Abort requested. The host settles the run as aborted.".to_string();
        } else {
            self.status = self
                .agent
                .error
                .clone()
                .unwrap_or_else(|| "The agent host is not running.".to_string());
        }
    }

    pub fn begin_session(&mut self) {
        self.messages = model::welcome_messages();
        self.tools.clear();
        self.composer = TextEdit::new("");
        self.elapsed = "0s".to_string();
        self.streaming = None;
        self.run_started = None;
        self.tool_started.clear();
        if self.agent.demo {
            self.status = "Demo fixture session. No host process is running.".to_string();
            return;
        }
        if self.agent.new_session() {
            self.agent.pause = None;
            self.status =
                "Starting a new host session. The previous transcript stays in its journal."
                    .to_string();
        } else {
            self.status = self
                .agent
                .error
                .clone()
                .unwrap_or_else(|| "The agent host is not running.".to_string());
        }
    }

    pub fn selected_note(&self) -> Option<&PatchNote> {
        self.patches.get(self.selected_patch)
    }

    /// Apply one envelope or event from the host to the render model.
    pub fn apply_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Result(result) => self.apply_result(result),
            AgentEvent::Event(value) => self.apply_run_event(&value),
            AgentEvent::Error(error) => self.apply_host_error(error),
            AgentEvent::Diagnostic(line) => {
                if self.agent.status == AgentStatus::Starting && line.contains("host ready") {
                    self.apply_host_config();
                    self.agent.state();
                    self.status = format!(
                        "Agent host ready: {}. Send a prompt when you are ready.",
                        self.agent.provider_label()
                    );
                }
                self.agent.diagnostic = Some(line);
            }
            AgentEvent::Eof => {
                self.streaming = None;
                self.run_started = None;
                self.agent.pause = None;
                self.status = self.agent.error.clone().unwrap_or_else(|| {
                    "The agent host stopped. Restart it from Settings.".to_string()
                });
            }
        }
    }

    fn apply_result(&mut self, result: ResultEnvelope) {
        if let Some(usage) = result.usage {
            self.agent.usage = Some(usage);
        }
        if result.session_id.is_some() {
            self.agent.session_id = result.session_id.clone();
        }
        if let Some(previous) = &result.previous_session_id {
            self.agent.previous_session_id = Some(previous.clone());
        }
        if !result.session_ids.is_empty() {
            self.agent.session_ids = result.session_ids.clone();
        }
        if let Some(pause) = &result.pause {
            self.agent.pause = Some(pause.clone());
            self.agent.status = AgentStatus::Paused;
            self.mark_paused_tool(pause);
            self.status = format!(
                "Paused on {}. Review the payload, then approve or deny.",
                pause.tool_name
            );
            return;
        }

        match result.kind {
            Some(CommandKind::Prompt) | Some(CommandKind::Resume) => {
                self.agent.status = AgentStatus::Ready;
                self.agent.pause = None;
                self.finish_assistant(&result.text);
                self.streaming = None;
                self.run_started = None;
                self.status = format!("Run complete. {}", self.agent.usage_label());
            }
            Some(CommandKind::Abort) => {
                if result.aborted == Some(true) {
                    self.status = "Abort requested. The run settles as aborted.".to_string();
                } else {
                    self.status = "There was no active run to abort.".to_string();
                    self.agent.status = AgentStatus::Ready;
                }
            }
            Some(CommandKind::NewSession) => {
                self.agent.status = AgentStatus::Ready;
                self.status = format!(
                    "New session {}. The previous transcript stays in its journal.",
                    self.agent.session_label()
                );
            }
            Some(CommandKind::ListSessions) => {
                self.status = format!(
                    "{} stored session{} under {}.",
                    self.agent.session_ids.len(),
                    if self.agent.session_ids.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    self.state_sessions_dir()
                );
            }
            Some(CommandKind::State) => {
                if let Some(pause) = &result.pause {
                    self.agent.pause = Some(pause.clone());
                    self.agent.status = AgentStatus::Paused;
                    self.mark_paused_tool(pause);
                } else if result.state_running {
                    self.agent.status = AgentStatus::Running;
                } else if self.agent.status != AgentStatus::Error {
                    self.agent.status = AgentStatus::Ready;
                }
                self.status = format!("Host state: {}.", self.agent.status.label());
            }
            Some(CommandKind::Shutdown) => {
                self.agent.status = AgentStatus::Stopped;
                self.status = "Agent host stopped.".to_string();
            }
            None => {}
        }
    }

    fn apply_host_error(&mut self, error: HostError) {
        self.streaming = None;
        self.run_started = None;
        self.agent.error = Some(format!("{}: {}", error.code, error.message));
        match error.code.as_str() {
            "busy" => {
                self.agent.status = AgentStatus::Running;
            }
            "aborted" => {
                self.agent.status = AgentStatus::Ready;
                self.agent.pause = None;
            }
            _ => {
                self.agent.status = AgentStatus::Error;
                self.agent.pause = None;
            }
        }
        self.status = format!("Agent host error ({}): {}", error.code, error.message);
    }

    fn apply_run_event(&mut self, value: &Value) {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "agent_start" => {
                self.agent.status = AgentStatus::Running;
                self.agent.pause = None;
                self.agent.error = None;
                self.streaming = None;
                self.run_started = Some(Instant::now());
                self.status = "Run started. Waiting for the model.".to_string();
            }
            "model_request_start" => {
                self.status = "Model request in flight.".to_string();
            }
            "model_request_end" => {
                if let Some(usage) = value.get("usage").and_then(agent::parse_usage) {
                    self.agent.usage = Some(usage);
                }
            }
            "text_delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.append_delta(delta);
                }
            }
            "reasoning_delta" => {}
            "tool_execution_start" => {
                let name = value
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let args = value
                    .get("arguments")
                    .map(|args| agent::truncate(&args.to_string(), 1200))
                    .unwrap_or_default();
                self.tool_started.insert(name.clone(), Instant::now());
                let existing = self
                    .tools
                    .iter_mut()
                    .rev()
                    .find(|run| run.name == name && run.state != model::RunState::Done);
                if let Some(run) = existing {
                    run.state = model::RunState::Running;
                    run.args = args.clone();
                    run.result = "Running".to_string();
                } else {
                    self.tools.push(ToolRun {
                        name: name.clone(),
                        state: model::RunState::Running,
                        duration: String::new(),
                        args: args.clone(),
                        result: "Running".to_string(),
                    });
                }
                let existing_message = self.messages.iter_mut().rev().find_map(|message| {
                    message
                        .tool
                        .as_mut()
                        .filter(|tool| tool.name == name && tool.state != model::RunState::Done)
                });
                if let Some(tool) = existing_message {
                    tool.state = model::RunState::Running;
                    tool.args = args;
                    tool.result = "Running".to_string();
                } else {
                    self.messages.push(Message::tool(ToolRun {
                        name: name.clone(),
                        state: model::RunState::Running,
                        duration: String::new(),
                        args,
                        result: "Running".to_string(),
                    }));
                }
                self.streaming = None;
                self.status = format!("Running {name}.");
            }
            "tool_execution_end" => {
                let name = value
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let output = value.get("output").and_then(Value::as_str).unwrap_or("");
                let is_error = value
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let duration = self
                    .tool_started
                    .remove(&name)
                    .map(duration_label)
                    .unwrap_or_default();
                let result = agent::truncate(output, 2000);
                let result = if result.trim().is_empty() {
                    if is_error {
                        "Tool failed without output.".to_string()
                    } else {
                        "Done.".to_string()
                    }
                } else {
                    result
                };
                let mut updated = false;
                if let Some(run) = self
                    .tools
                    .iter_mut()
                    .rev()
                    .find(|run| run.name == name && run.state != model::RunState::Done)
                {
                    run.state = model::RunState::Done;
                    run.duration = duration.clone();
                    run.result = result.clone();
                    updated = true;
                }
                if let Some(tool) = self.messages.iter_mut().rev().find_map(|message| {
                    message
                        .tool
                        .as_mut()
                        .filter(|tool| tool.name == name && tool.state != model::RunState::Done)
                }) {
                    tool.state = model::RunState::Done;
                    tool.duration = duration;
                    tool.result = result.clone();
                    updated = true;
                }
                if !updated {
                    self.tools.push(ToolRun {
                        name: name.clone(),
                        state: model::RunState::Done,
                        duration: String::new(),
                        args: String::new(),
                        result,
                    });
                }
                self.status = format!("{name} finished.");
            }
            "agent_end" => {
                let outcome = value
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
                if let Some(usage) = value.get("usage").and_then(agent::parse_usage) {
                    self.agent.usage = Some(usage);
                }
                self.streaming = None;
                self.run_started = None;
                match outcome {
                    "completed" => {
                        let text = value.get("text").and_then(Value::as_str).unwrap_or("");
                        self.finish_assistant(text);
                        self.agent.status = AgentStatus::Ready;
                        self.agent.pause = None;
                        self.status = format!("Run complete. {}", self.agent.usage_label());
                    }
                    "paused" => {
                        if let Some(pause) = value.get("pause").and_then(agent::parse_pause) {
                            self.agent.pause = Some(pause.clone());
                            self.agent.status = AgentStatus::Paused;
                            self.mark_paused_tool(&pause);
                            self.status = format!(
                                "Paused on {}. Approve or deny the payload in the workbench.",
                                pause.tool_name
                            );
                        } else {
                            self.agent.status = AgentStatus::Paused;
                            self.status = "The run paused for approval.".to_string();
                        }
                    }
                    "failed" => {
                        let message = value
                            .get("error")
                            .and_then(|error| error.get("message"))
                            .and_then(Value::as_str)
                            .unwrap_or("The run failed.");
                        let code = value
                            .get("error")
                            .and_then(|error| error.get("code"))
                            .and_then(Value::as_str)
                            .unwrap_or("run_failed");
                        self.agent.status = AgentStatus::Error;
                        self.agent.error = Some(format!("{code}: {message}"));
                        self.status = format!("Run failed ({code}): {message}");
                    }
                    "aborted" => {
                        self.agent.status = AgentStatus::Ready;
                        self.agent.pause = None;
                        self.status = "Run aborted.".to_string();
                    }
                    other => {
                        self.status = format!("Run ended: {other}.");
                    }
                }
            }
            _ => {}
        }
    }

    fn append_delta(&mut self, delta: &str) {
        let idx = match self.streaming {
            Some(idx) if idx < self.messages.len() => idx,
            _ => {
                let model = self.agent.model.clone();
                self.messages
                    .push(Message::assistant("", "streaming", model.as_deref()));
                self.messages.len() - 1
            }
        };
        self.streaming = Some(idx);
        if let Some(message) = self.messages.get_mut(idx) {
            message.body.push_str(delta);
        }
        if self.agent.status != AgentStatus::Running {
            self.agent.status = AgentStatus::Running;
        }
    }

    fn finish_assistant(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(idx) = self.streaming
            && let Some(message) = self.messages.get_mut(idx)
        {
            if message.body.trim().is_empty() {
                message.body = text.to_string();
            }
            message.meta = "now".to_string();
            return;
        }
        let already = self
            .messages
            .last()
            .map(|message| message.role == Role::Assistant && message.body == text)
            .unwrap_or(false);
        if !already {
            let model = self.agent.model.clone();
            self.messages
                .push(Message::assistant(text, "now", model.as_deref()));
        }
    }

    fn mark_paused_tool(&mut self, pause: &Pause) {
        if let Some(run) = self
            .tools
            .iter_mut()
            .rev()
            .find(|run| run.name == pause.tool_name && run.state != model::RunState::Done)
        {
            run.state = model::RunState::Queued;
            run.result = format!("Paused for approval: {}", pause.tool_name);
        }
        if let Some(tool) = self.messages.iter_mut().rev().find_map(|message| {
            message
                .tool
                .as_mut()
                .filter(|tool| tool.name == pause.tool_name && tool.state != model::RunState::Done)
        }) {
            tool.state = model::RunState::Queued;
            tool.result = format!("Paused for approval: {}", pause.tool_name);
        }
    }

    pub fn state_sessions_dir(&self) -> String {
        format!("{}/agent-sessions", self.paths.data)
    }
}

fn duration_label(start: Instant) -> String {
    let millis = start.elapsed().as_millis();
    if millis < 1000 {
        format!("{millis} ms")
    } else {
        format!("{:.1} s", millis as f64 / 1000.0)
    }
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let u = Ui::new();
    cx.foreground(u.ink);
    let body = match s.screen {
        Screen::Workbench => screens::workbench::view(s, &u),
        Screen::Projects => screens::projects::view(s, &u),
        Screen::Index => screens::index::view(s, &u),
        Screen::Patches => screens::patches::view(s, &u),
        Screen::Impact => screens::impact::view(s, &u),
        Screen::Release => screens::release::view(s, &u),
        Screen::Settings => screens::settings::view(s, &u),
    };
    row()
        .key("root")
        .w_full()
        .h_full()
        .fill(u.bg)
        .child(sidebar(s, &u))
        .child(
            col()
                .key("main")
                .grow(1.0)
                .h_full()
                .min_w(0.0)
                .child(header(s, &u))
                .child(body)
                .child(status_bar(s, &u)),
        )
}

fn sidebar(s: &State, u: &Ui) -> El {
    let project = s.project();
    let mut nav = col().key("nav").grow(1.0).min_h(0.0).px(10.0).gap(2.0);
    nav = nav
        .child(section_gap(u, "WORKBENCH", 14.0))
        .child(nav_item(s, u, Screen::Workbench))
        .child(nav_item(s, u, Screen::Projects))
        .child(nav_item(s, u, Screen::Index))
        .child(nav_item(s, u, Screen::Patches))
        .child(nav_item(s, u, Screen::Impact))
        .child(nav_item(s, u, Screen::Release))
        .child(el().h(10.0))
        .child(section_gap(u, "SYSTEM", 14.0))
        .child(nav_item(s, u, Screen::Settings));

    let workspace = match project {
        Some(p) => {
            let tone = match p.state {
                ProjectState::Ready => Tone::Ok,
                ProjectState::Attention => Tone::Warn,
                ProjectState::Draft => Tone::Quiet,
            };
            col()
                .key("workspace-card")
                .mx(12.0)
                .mb(10.0)
                .padding(10.0)
                .gap(5.0)
                .radius(8.0)
                .fill(Color::WHITE.alpha(0.03))
                .border(1.0, u.hair)
                .child(
                    row()
                        .w_full()
                        .gap(7.0)
                        .center_y()
                        .child(u.dot(tone))
                        .child(
                            text(p.name.clone())
                                .font(BODY_MED)
                                .color(u.ink)
                                .grow(1.0)
                                .truncate(),
                        )
                        .child(u.chip(&format!("{:02}", p.slot), u.accent, u.accent.alpha(0.13))),
                )
                .child(
                    text(p.summary.clone())
                        .font(SMALL)
                        .color(u.faint)
                        .max_lines(2)
                        .wrap(),
                )
        }
        None => col()
            .key("workspace-card")
            .mx(12.0)
            .mb(10.0)
            .padding(10.0)
            .gap(5.0)
            .radius(8.0)
            .fill(Color::WHITE.alpha(0.025))
            .border(1.0, u.hair)
            .child(
                row()
                    .w_full()
                    .gap(7.0)
                    .center_y()
                    .child(u.dot(Tone::Quiet))
                    .child(
                        text("No project open")
                            .font(BODY_MED)
                            .color(u.muted)
                            .grow(1.0),
                    ),
            )
            .child(
                text("Open a project to scope the session.")
                    .font(SMALL)
                    .color(u.faint),
            ),
    };

    let provider_label = match s.provider() {
        Some(p) => p.name.clone(),
        None => "No provider".to_string(),
    };

    let footer_tone = s.game_tone();
    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(u.sidebar)
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .pt(26.0)
        .child(
            row()
                .key("brand")
                .h(38.0)
                .px(15.0)
                .gap(9.0)
                .center_y()
                .child(u.glyph_box("◆", Tone::Accent, 27.0))
                .child(
                    col()
                        .gap(0.0)
                        .child(text("DEADLOCK").font(H2).color(u.ink))
                        .child(text("Modding Workbench").font(LABEL).color(u.faint)),
                ),
        )
        .child(workspace)
        .child(nav)
        .child(
            col()
                .key("sidebar-foot")
                .px(14.0)
                .pb(12.0)
                .gap(7.0)
                .child(u.divider())
                .child(
                    row()
                        .w_full()
                        .gap(7.0)
                        .center_y()
                        .child(u.dot(footer_tone))
                        .child(
                            text(s.game_label.clone())
                                .font(SMALL)
                                .color(u.muted)
                                .grow(1.0)
                                .truncate(),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .gap(7.0)
                        .center_y()
                        .child(u.dot(s.provider_tone()))
                        .child(
                            text(provider_label)
                                .font(SMALL)
                                .color(u.muted)
                                .grow(1.0)
                                .truncate(),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .gap(6.0)
                        .center_y()
                        .child(text("dah-desktop 0.0.0").font(LABEL).color(u.faint))
                        .child(spacer())
                        .child(text("RUST · RUNE").font(LABEL).color(u.faint)),
                ),
        )
}

fn section_gap(u: &Ui, label: &str, bottom: f32) -> El {
    col()
        .w_full()
        .px(9.0)
        .pt(4.0)
        .pb(bottom)
        .child(text(label).font(LABEL).color(u.faint))
}

fn nav_item(s: &State, u: &Ui, screen: Screen) -> El {
    let on = s.screen == screen;
    let mut r = row()
        .key(format!("nav-{}", screen.key()))
        .h(33.0)
        .px(9.0)
        .gap(9.0)
        .center_y()
        .radius(6.0)
        .fill(if on {
            u.accent.alpha(0.10)
        } else {
            Color::TRANSPARENT
        })
        .hover(Color::WHITE.alpha(0.04))
        .press(Color::WHITE.alpha(0.055))
        .focusable(true)
        .role("tab")
        .label(screen.title())
        .checked(on)
        .motion(motion::SNAPPY)
        .on_press(move |st: &mut State| {
            st.screen = screen;
            st.status.clear();
        });
    if on {
        r = r.child(
            el().w(2.0)
                .h(17.0)
                .radius(1.0)
                .fill(u.accent)
                .absolute(0.0, 8.0)
                .a11y_hidden(),
        );
    }
    r.child(
        text(screen.glyph())
            .font(BODY)
            .w(17.0)
            .color(if on { u.accent } else { u.faint })
            .a11y_hidden(),
    )
    .child(
        text(screen.title())
            .font(if on { BODY_MED } else { BODY })
            .color(if on { u.ink } else { u.muted })
            .grow(1.0),
    )
    .child(text(screen.shortcut()).font(LABEL).color(if on {
        u.accent.alpha(0.85)
    } else {
        u.faint
    }))
}

fn header(s: &State, u: &Ui) -> El {
    let mut chips = row().gap(7.0).center_y();
    if s.demo {
        chips = chips.child(u.chip("DEMO DATA", u.accent_ink, u.accent));
    }
    if s.agent.pause.is_some() {
        chips = chips.child(u.status_chip("approval paused", Tone::Warn));
    }
    chips = chips
        .child(u.status_chip(
            if s.game_detected {
                "citadel found"
            } else {
                "no game install"
            },
            s.game_tone(),
        ))
        .child(u.status_chip(
            if s.provider_ready() {
                "provider ready"
            } else {
                "no provider"
            },
            s.provider_tone(),
        ))
        .child(u.chip("LOCAL ONLY", u.faint, Color::WHITE.alpha(0.035)));

    row()
        .key("header")
        .w_full()
        .h(HEADER_H)
        .px(22.0)
        .gap(9.0)
        .center_y()
        .fill(u.bg)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .window_drag()
        .child(text(s.screen.title()).font(H2).color(u.ink))
        .child(text("/").font(SMALL).color(u.faint).a11y_hidden())
        .child(
            text(s.screen.subtitle())
                .font(SMALL)
                .color(u.faint)
                .grow(1.0)
                .truncate(),
        )
        .child(chips)
}

fn status_bar(s: &State, u: &Ui) -> El {
    let tone = if s.status.is_empty() {
        Tone::Quiet
    } else if s.status.contains("not ") || s.status.contains("No ") || s.status.contains("blocked")
    {
        Tone::Warn
    } else {
        Tone::Info
    };
    row()
        .key("statusbar")
        .w_full()
        .h(STATUS_H)
        .px(14.0)
        .gap(8.0)
        .center_y()
        .fill(u.sidebar)
        .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
        .child(u.dot(tone))
        .child(
            text(if s.status.is_empty() {
                "Ready. Tools propose, the app writes.".to_string()
            } else {
                s.status.clone()
            })
            .font(SMALL)
            .color(u.muted)
            .grow(1.0)
            .truncate(),
        )
        .child(text("READ ONLY").font(LABEL).color(u.faint))
        .child(text("·").font(LABEL).color(u.faint).a11y_hidden())
        .child(text("NO TELEMETRY").font(LABEL).color(u.faint))
}

/// Start the agent host once, after Rune has applied the selected scene.
pub fn setup(s: &mut State, waker: Waker) {
    s.wake = Some(waker);
    s.start_host();
}

pub fn poll(s: &mut State, t: f64, _tx: &mut Tx) -> bool {
    let mut changed = false;
    while let Some(event) = s.agent.try_event() {
        s.apply_agent_event(event);
        changed = true;
    }
    if let Some(start) = s.run_started {
        let label = elapsed_label(start.elapsed().as_secs());
        if label != s.elapsed {
            s.elapsed = label;
            changed = true;
        }
    }
    let running = s.agent.status == AgentStatus::Running
        || s.tools.iter().any(|r| r.state == model::RunState::Running);
    if s.screen == Screen::Workbench && running {
        s.pulse = ((t * std::f64::consts::TAU * 0.75).sin() * 0.5 + 0.5) as f32;
        changed = true;
    }
    changed
}

fn elapsed_label(seconds: u64) -> String {
    if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if let Key::Command(c) = ev.key
        && let Some(screen) = Screen::from_command(c)
    {
        s.screen = screen;
        s.status.clear();
        return true;
    }
    // Rune's `run` has no close hook. Take the quit chords it does deliver here, send the host
    // its shutdown command, then let the window close. If a platform quits without reaching
    // `on_key`, the child still exits on stdin EOF when this process ends.
    if let Key::Command('q') = ev.key {
        s.agent.shutdown();
        tx.quit();
        return true;
    }
    if let Key::Command('w') = ev.key {
        s.agent.shutdown();
        tx.close_window();
        return true;
    }
    false
}

pub fn scene_workbench(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Workbench;
}

pub fn scene_projects(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Projects;
}

pub fn scene_index(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Index;
}

pub fn scene_patches(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Patches;
}

pub fn scene_impact(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Impact;
}

pub fn scene_release(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Release;
}

pub fn scene_local_release(s: &mut State) {
    let agent = std::mem::replace(&mut s.agent, AgentLink::disconnected());
    let wake = s.wake.take();
    *s = crate::data::init_release_state();
    s.agent = agent;
    s.wake = wake;
}

pub fn scene_settings(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Settings;
}

pub fn scene_empty(s: &mut State) {
    s.load_empty();
}

pub fn scene_local_workbench(s: &mut State) {
    s.screen = Screen::Workbench;
}

pub fn scene_local_projects(s: &mut State) {
    s.screen = Screen::Projects;
}

pub fn scene_local_patches(s: &mut State) {
    s.screen = Screen::Patches;
}

pub fn scenes() -> &'static [(&'static str, fn(&mut State))] {
    &[
        ("workbench", scene_workbench),
        ("projects", scene_projects),
        ("index", scene_index),
        ("patches", scene_patches),
        ("impact", scene_impact),
        ("release", scene_release),
        ("settings", scene_settings),
        ("empty", scene_empty),
        ("local", scene_local_workbench),
        ("local-projects", scene_local_projects),
        ("local-patches", scene_local_patches),
        ("local-release", scene_local_release),
    ]
}

/// Shared by screens: a small uppercase filter chip.
pub fn filter_chip(u: &Ui, label: &str, on: bool) -> El {
    row()
        .h(27.0)
        .px(10.0)
        .center()
        .radius(13.0)
        .fill(if on {
            u.accent.alpha(0.16)
        } else {
            Color::WHITE.alpha(0.03)
        })
        .border(1.0, if on { u.accent.alpha(0.36) } else { u.hair })
        .hover(Color::WHITE.alpha(0.05))
        .press(Color::WHITE.alpha(0.07))
        .focusable(true)
        .role("button")
        .label(label)
        .checked(on)
        .motion(motion::SNAPPY)
        .child(
            text(label)
                .font(LABEL)
                .color(if on { u.accent } else { u.muted })
                .a11y_hidden(),
        )
}
