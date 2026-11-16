//! The mod-manager shell: state, scenes, sidebar, header, status bar and screen
//! dispatch. The library is the default surface; the agent is one workbench
//! screen inside the same shell.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use rune::tok::motion;
use rune::{
    Color, Cx, El, ImageCache, Key, KeyEvent, Pixels, TextEdit, Tx, Waker, col, el, row, text,
};
use serde_json::Value;

use crate::agent::{
    self, AgentEvent, AgentLink, AgentStatus, CommandKind, HostError, Pause, ResultEnvelope,
};
use crate::data::ThumbCrew;
use crate::jobs::Job;
use crate::model::{
    self, AddonLibrary, BrowseState, Catalog, ImpactReport, IndexRow, Message, PatchNote, Paths,
    Project, ProjectState, Provider, ProviderState, ReleaseDialog, ReleaseState, ReviewQueue, Role,
    Screen, ToolRun,
};
use crate::publish::PublishJob;
use crate::screens;
use crate::ui::{
    ActionKind, BODY, BODY_MED, H2, HEADER_H, LABEL, SIDEBAR_W, SMALL, STATUS_H, Tone, Ui,
};

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
    pub release_dialog: ReleaseDialog,
    pub confirm_url: TextEdit,
    pub publish_job: PublishJob,
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
    pub game_root: Option<PathBuf>,
    pub addons_dir: Option<String>,
    pub model_label: String,
    pub elapsed: String,
    pub agent: AgentLink,
    pub wake: Option<Waker>,
    pub streaming: Option<usize>,
    pub run_started: Option<Instant>,
    pub tool_started: HashMap<String, Instant>,
    pub addons: AddonLibrary,
    pub addon_job: Job<AddonLibrary>,
    pub installed_query: TextEdit,
    pub browse: BrowseState,
    pub browse_job: Job<model::BrowsePageView>,
    pub browse_query: TextEdit,
    pub install: screens::browse::InstallView,
    pub install_job: Job<screens::browse::InstallOutcome>,
    pub conflicts_query: TextEdit,
    pub images: Option<ImageCache>,
    pub thumbnails: HashMap<String, Pixels>,
    pub pending_thumbs: Vec<(String, Vec<u8>)>,
    pub thumb_crew: ThumbCrew,
    pub review_job: Job<ReviewQueue>,
    pub launch_dialog: bool,
    pub addon_dialog: Option<(u8, bool)>,
    pub watch: Option<model::WatchState>,
    pub capabilities: Vec<(String, String)>,
    pub crosshair: screens::crosshair::View,
    pub profiles: screens::profiles::View,
    pub profiles_job: Job<screens::profiles::Load>,
    pub autoexec: screens::autoexec::View,
    pub autoexec_original: String,
    pub autoexec_job: Job<screens::autoexec::Read>,
    pub stats: screens::stats::View,
    pub stats_job: Job<screens::stats::Load>,
    pub demo_job: Job<screens::stats::DemoLoad>,
    pub locker: screens::locker::View,
    pub locker_job: Job<screens::locker::Load>,
    pub launch: screens::launch::View,
    pub launch_job: Job<screens::launch::Read>,
    pub toolchain: screens::toolchain::View,
    pub toolchain_job: Job<deadlock_core::toolchain::ToolchainStatus>,
    pub character: screens::character::View,
    pub character_job: Job<screens::character::Load>,
}

impl Default for State {
    fn default() -> Self {
        State {
            screen: Screen::Installed,
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
            release_dialog: ReleaseDialog::None,
            confirm_url: TextEdit::new(""),
            publish_job: PublishJob::default(),
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
            game_root: None,
            addons_dir: None,
            model_label: "No model".to_string(),
            elapsed: "0s".to_string(),
            agent: AgentLink::disconnected(),
            wake: None,
            streaming: None,
            run_started: None,
            tool_started: HashMap::new(),
            addons: AddonLibrary::default(),
            addon_job: Job::default(),
            installed_query: TextEdit::new(""),
            browse: BrowseState::default(),
            browse_job: Job::default(),
            browse_query: TextEdit::new(""),
            install: screens::browse::InstallView::default(),
            install_job: Job::default(),
            conflicts_query: TextEdit::new(""),
            images: None,
            thumbnails: HashMap::new(),
            pending_thumbs: Vec::new(),
            thumb_crew: ThumbCrew::default(),
            review_job: Job::default(),
            launch_dialog: false,
            addon_dialog: None,
            watch: None,
            capabilities: Vec::new(),
            crosshair: screens::crosshair::View::default(),
            profiles: screens::profiles::View::default(),
            profiles_job: Job::default(),
            autoexec: screens::autoexec::View::default(),
            autoexec_original: String::new(),
            autoexec_job: Job::default(),
            stats: screens::stats::View::default(),
            stats_job: Job::default(),
            demo_job: Job::default(),
            locker: screens::locker::View::default(),
            locker_job: Job::default(),
            launch: screens::launch::View::default(),
            launch_job: Job::default(),
            toolchain: screens::toolchain::View::default(),
            toolchain_job: Job::default(),
            character: screens::character::View::default(),
            character_job: Job::default(),
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
        self.addons_dir = Some(
            "~/Library/Application Support/Steam/steamapps/common/Deadlock/game/citadel/addons"
                .to_string(),
        );
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
        self.addons = model::demo_addon_library();
        self.model_label = "qwen3-coder-30b".to_string();
        self.elapsed = "2m 14s".to_string();
        self.status = "Demo fixture loaded. Nothing on this screen reads or writes a game install."
            .to_string();
    }

    pub fn load_empty(&mut self) {
        let agent = std::mem::replace(&mut self.agent, AgentLink::disconnected());
        let wake = self.wake.take();
        let images = self.images.take();
        *self = State::default();
        self.agent = agent;
        self.wake = wake;
        self.images = images;
        self.paths.data = crate::data::data_dir().display().to_string();
    }

    /// Move to a screen and start the read that screen needs. Library screens
    /// fetch on first entry rather than on every frame.
    pub fn go(&mut self, screen: Screen) {
        self.screen = screen;
        self.status.clear();
        match screen {
            Screen::Installed | Screen::Conflicts => {
                if !self.addons.scanned && !self.addon_job.is_running() {
                    crate::data::start_addon_scan(self);
                }
            }
            Screen::Browse => {
                if self.browse.page.is_none() && !self.browse.loading {
                    crate::data::start_browse_fetch(self, 1);
                }
            }
            Screen::Updates => {
                if self.release.review.is_none() && !self.review_job.is_running() {
                    crate::data::start_review_scan(self);
                }
            }
            Screen::Profiles => {
                if !self.autoexec.loaded {
                    crate::data::load_autoexec_now(self);
                }
                crate::data::start_profile_list(self);
            }
            Screen::Autoexec => {
                if !self.autoexec.loaded && !self.autoexec_job.is_running() {
                    crate::data::start_autoexec_read(self);
                }
            }
            Screen::Stats => {
                if !self.stats.loaded && !self.stats_job.is_running() {
                    crate::data::start_stats_load(self);
                }
                if !self.demo_job.is_running() {
                    crate::data::start_demo_list(self);
                }
            }
            Screen::Locker => {
                if !self.locker.loaded && !self.locker_job.is_running() {
                    crate::data::start_locker_load(self);
                }
            }
            Screen::Launch => {
                if !self.launch.loaded && !self.launch_job.is_running() {
                    crate::data::start_launch_read(self);
                }
            }
            Screen::Toolchain => {
                if !self.toolchain.loaded && !self.toolchain_job.is_running() {
                    crate::data::start_toolchain_read(self);
                }
            }
            Screen::Character => {
                if !self.character.loaded && !self.character_job.is_running() {
                    crate::data::start_character_read(self);
                }
            }
            _ => {}
        }
    }

    /// Launch the game through Steam with the current addon folder. This is an
    /// app action behind the launch dialog, never an agent tool.
    pub fn launch_modded(&mut self) {
        self.launch_dialog = false;
        if rune::is_headless() {
            self.status =
                "Headless run: the launch request was not sent to Steam.".to_string();
            return;
        }
        let spawn = |program: &str, args: &[&str]| {
            std::process::Command::new(program)
                .args(args)
                .spawn()
                .map(|_| ())
                .map_err(|error| error.to_string())
        };
        let target = "steam://rungameid/1422450";
        #[cfg(target_os = "macos")]
        let opened = spawn("open", &[target]);
        #[cfg(target_os = "windows")]
        let opened = spawn("cmd", &["/C", "start", "", target]);
        #[cfg(all(unix, not(target_os = "macos")))]
        let opened = spawn("xdg-open", &[target]);
        match opened {
            Ok(()) => {
                self.status = format!(
                    "Asked Steam to launch Deadlock with {} addon{} enabled. The app wrote nothing.",
                    self.addons.enabled_count(),
                    if self.addons.enabled_count() == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            }
            Err(error) => self.status = format!("Steam could not be opened: {error}"),
        }
    }

    /// Start the Rust host for the real scenes and the window. Demo scenes get a labelled
    /// fixture link instead of a process, so snapshots stay hermetic.
    pub fn start_host(&mut self) {
        if self.demo {
            self.agent = AgentLink::demo();
            return;
        }
        // A headless render must not spawn the sidecar: its state would depend on
        // a local binary and the snapshot would not be reproducible.
        if rune::is_headless() {
            self.agent = AgentLink::disconnected();
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
                "Resolve the paused tool call first: approve or deny it on the Agent screen."
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
                                "Paused on {}. Approve or deny the payload on the Agent screen.",
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
        Screen::Installed => screens::installed::view(s, &u),
        Screen::Browse => screens::browse::view(s, &u),
        Screen::Conflicts => screens::conflicts::view(s, &u),
        Screen::Updates => screens::updates::view(s, &u),
        Screen::Profiles => screens::profiles::view(s, &u),
        Screen::Autoexec => screens::autoexec::view(s, &u),
        Screen::Crosshair => screens::crosshair::view(s, &u),
        Screen::Stats => screens::stats::view(s, &u),
        Screen::Locker => screens::locker::view(s, &u),
        Screen::Launch => screens::launch::view(s, &u),
        Screen::Agent => screens::workbench::view(s, &u),
        Screen::Projects => screens::projects::view(s, &u),
        Screen::Index => screens::index::view(s, &u),
        Screen::Patches => screens::patches::view(s, &u),
        Screen::Impact => screens::impact::view(s, &u),
        Screen::Release => screens::release::view(s, &u),
        Screen::Toolchain => screens::toolchain::view(s, &u),
        Screen::Character => screens::character::view(s, &u),
        Screen::Settings => screens::settings::view(s, &u),
    };
    let mut root = row()
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
        );
    if s.launch_dialog {
        root = root.child(u.modal_layer("launch-run-modal", launch_layer(s, &u)));
    }
    for (key, layer) in [
        ("browse-modal", screens::browse::dialog(s, &u)),
        ("profiles-modal", screens::profiles::dialog(s, &u)),
        ("autoexec-modal", screens::autoexec::dialog(s, &u)),
        ("launch-modal", screens::launch::dialog(s, &u)),
        ("toolchain-modal", screens::toolchain::dialog(s, &u)),
        ("addon-modal", screens::installed::dialog(s, &u)),
        ("review-modal", screens::stats::dialog(s, &u)),
        ("crosshair-modal", screens::crosshair::dialog(s, &u)),
    ] {
        if let Some(layer) = layer {
            root = root.child(u.modal_layer(key, layer));
        }
    }
    if s.screen == Screen::Release
        && let Some(layer) = screens::release::dialog(s, &u)
    {
        root = root.child(layer);
    }
    root
}

fn sidebar(s: &State, u: &Ui) -> El {
    let mut library = col().w_full().gap(1.0);
    for screen in [
        Screen::Installed,
        Screen::Browse,
        Screen::Conflicts,
        Screen::Updates,
    ] {
        library = library.child(nav_item(s, u, screen));
    }

    let mut modding = col().w_full().gap(1.0);
    for screen in [
        Screen::Profiles,
        Screen::Autoexec,
        Screen::Crosshair,
        Screen::Stats,
        Screen::Locker,
        Screen::Launch,
    ] {
        modding = modding.child(nav_item(s, u, screen));
    }

    let mut workbench = col().w_full().gap(1.0);
    for screen in [
        Screen::Agent,
        Screen::Projects,
        Screen::Index,
        Screen::Patches,
        Screen::Impact,
        Screen::Release,
    ] {
        workbench = workbench.child(nav_item(s, u, screen));
    }

    let mut foundry = col().w_full().gap(1.0);
    for screen in [Screen::Toolchain, Screen::Character] {
        foundry = foundry.child(nav_item(s, u, screen));
    }

    let project_line = match s.project() {
        Some(project) => project.name.clone(),
        None => "No project open".to_string(),
    };
    let project_tone = match s.project().map(|project| project.state) {
        Some(ProjectState::Ready) => Tone::Ok,
        Some(ProjectState::Attention) => Tone::Warn,
        Some(ProjectState::Draft) | None => Tone::Quiet,
    };
    let provider_label = match s.provider() {
        Some(provider) => provider.name.clone(),
        None => "No provider".to_string(),
    };

    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(u.sidebar)
        .border_sides([0.0, 1.0, 0.0, 0.0], u.hair)
        .pt(24.0)
        .child(
            row()
                .key("brand")
                .h(38.0)
                .px(16.0)
                .gap(9.0)
                .center_y()
                .child(u.glyph_box("◆", Tone::Accent, 26.0))
                .child(
                    col()
                        .gap(0.0)
                        .child(text("DEADLOCK").font(H2).color(u.ink))
                        .child(text("MOD MANAGER").font(LABEL).color(u.faint)),
                ),
        )
        .child(
            col()
                .key("sidebar-status")
                .w_full()
                .px(16.0)
                .pt(12.0)
                .pb(10.0)
                .gap(4.0)
                .child(
                    row()
                        .w_full()
                        .gap(7.0)
                        .center_y()
                        .child(u.dot(project_tone))
                        .child(
                            text(project_line)
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
                        .child(u.dot(s.game_tone()))
                        .child(
                            text(s.game_label.clone())
                                .font(LABEL)
                                .color(u.faint)
                                .grow(1.0)
                                .truncate(),
                        ),
                ),
        )
        .child(u.divider())
        .child(
            col()
                .key("nav")
                .grow(1.0)
                .min_h(0.0)
                .px(10.0)
                .gap(1.0)
                .child(section_gap(u, "LIBRARY", 6.0))
                .child(library)
                .child(el().h(12.0))
                .child(section_gap(u, "MODDING", 6.0))
                .child(modding)
                .child(el().h(12.0))
                .child(section_gap(u, "WORKBENCH", 6.0))
                .child(workbench)
                .child(el().h(12.0))
                .child(section_gap(u, "FOUNDRY", 6.0))
                .child(foundry),
        )
        .child(
            col()
                .key("sidebar-foot")
                .w_full()
                .px(10.0)
                .pb(12.0)
                .gap(4.0)
                .child(u.divider().mb(6.0))
                .child(launch_item(s, u))
                .child(nav_item(s, u, Screen::Settings))
                .child(
                    row()
                        .w_full()
                        .px(9.0)
                        .pt(8.0)
                        .gap(6.0)
                        .center_y()
                        .child(
                            text(provider_label)
                                .font(LABEL)
                                .color(u.faint)
                                .grow(1.0)
                                .truncate(),
                        )
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
    let count = nav_count(s, screen);
    let mut r = row()
        .key(format!("nav-{}", screen.key()))
        .h(32.0)
        .px(9.0)
        .gap(9.0)
        .center_y()
        .radius(6.0)
        .fill(if on {
            u.accent.alpha(0.09)
        } else {
            Color::TRANSPARENT
        })
        .hover(Color::WHITE.alpha(0.04))
        .press(Color::WHITE.alpha(0.055))
        .focusable(true)
        .role("tab")
        .label(format!("{} ({})", screen.title(), screen.shortcut()))
        .checked(on)
        .motion(motion::SNAPPY)
        .on_press(move |st: &mut State| st.go(screen));
    if on {
        r = r.child(
            el().w(2.0)
                .h(16.0)
                .radius(1.0)
                .fill(u.accent)
                .absolute(0.0, 8.0)
                .a11y_hidden(),
        );
    }
    r = r
        .child(
            text(screen.glyph())
                .font(BODY)
                .w(16.0)
                .color(if on { u.accent } else { u.faint })
                .a11y_hidden(),
        )
        .child(
            text(screen.title())
                .font(if on { BODY_MED } else { BODY })
                .color(if on { u.ink } else { u.muted })
                .grow(1.0)
                .truncate(),
        );
    let shortcut = screen.shortcut();
    match count {
        Some((label, tone)) => r.child(text(label).font(LABEL).color(tone.fg(u))),
        None if shortcut.is_empty() => r,
        None => r.child(text(shortcut).font(LABEL).color(if on {
            u.accent.alpha(0.85)
        } else {
            u.faint
        })),
    }
}

/// A live count for the library items. `None` means the scan has not run, so
/// the shortcut is shown instead of a number the shell does not have.
fn nav_count(s: &State, screen: Screen) -> Option<(String, Tone)> {
    match screen {
        Screen::Installed if s.addons.scanned && s.addons.error.is_none() => Some((
            s.addons.addons.len().to_string(),
            if s.addons.addons.is_empty() {
                Tone::Quiet
            } else {
                Tone::Accent
            },
        )),
        Screen::Conflicts if s.addons.scanned && s.addons.error.is_none() => {
            let count = s.addons.conflicts.len();
            Some((
                count.to_string(),
                if count > 0 { Tone::Warn } else { Tone::Quiet },
            ))
        }
        Screen::Updates if s.release.review.is_some() => {
            let count = s
                .release
                .review
                .as_ref()
                .map(|queue| queue.review_count())
                .unwrap_or(0);
            Some((
                count.to_string(),
                if count > 0 { Tone::Warn } else { Tone::Quiet },
            ))
        }
        _ => None,
    }
}

fn launch_item(s: &State, u: &Ui) -> El {
    row()
        .key("launch-modded")
        .h(36.0)
        .px(9.0)
        .gap(9.0)
        .center_y()
        .radius(6.0)
        .fill(u.accent.alpha(0.08))
        .border(1.0, u.accent.alpha(0.16))
        .hover(u.accent.alpha(0.14))
        .press(u.accent.alpha(0.20))
        .press_scale(0.985)
        .focusable(true)
        .role("button")
        .label("Launch Modded")
        .motion(motion::SNAPPY)
        .on_press(|st: &mut State| st.launch_dialog = true)
        .child(
            text("▶")
                .font(BODY)
                .w(16.0)
                .color(u.accent)
                .a11y_hidden(),
        )
        .child(
            text("Launch Modded")
                .font(BODY_MED)
                .color(u.ink)
                .grow(1.0),
        )
        .child(if s.game_detected {
            text("STEAM").font(LABEL).color(u.accent.alpha(0.75))
        } else {
            text("NO GAME").font(LABEL).color(u.faint)
        })
}

fn header(s: &State, u: &Ui) -> El {
    let mut chips = row().gap(8.0).center_y();
    if s.demo {
        chips = chips.child(u.chip("DEMO DATA", u.accent_ink, u.accent));
    }
    if s.agent.pause.is_some() {
        chips = chips.child(u.status_chip("approval paused", Tone::Warn));
    }
    chips = chips.child(
        row()
            .gap(6.0)
            .center_y()
            .child(u.dot(s.game_tone()))
            .child(
                text(if s.game_detected {
                    s.game_label.clone()
                } else {
                    "No game install".to_string()
                })
                .font(SMALL)
                .color(u.muted)
                .max_w(230.0)
                .truncate(),
            ),
    );
    if !s.screen.is_library() {
        chips = chips.child(u.status_chip(
            if s.provider_ready() {
                "provider ready"
            } else {
                "no provider"
            },
            s.provider_tone(),
        ));
    }

    let mut bar = row()
        .key("header")
        .w_full()
        .h(HEADER_H)
        .px(22.0)
        .gap(12.0)
        .center_y()
        .fill(u.bg)
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .window_drag()
        .child(text(s.screen.title()).font(H2).color(u.ink))
        .child(
            text(s.screen.subtitle())
                .font(SMALL)
                .color(u.faint)
                .grow(1.0)
                .truncate(),
        );
    if s.screen.searches() {
        bar = bar.child(search(s, u));
    }
    bar.child(chips)
}

fn search(s: &State, u: &Ui) -> El {
    let field = match s.screen {
        Screen::Installed => rune::text_input(&s.installed_query, SMALL)
            .key("search-installed")
            .label("Search installed addons")
            .single_line()
            .color(u.ink)
            .placeholder("Search installed", u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(|st: &mut State| &mut st.installed_query),
        Screen::Browse => rune::text_input(&s.browse_query, SMALL)
            .key("search-browse")
            .label("Filter this GameBanana page")
            .single_line()
            .color(u.ink)
            .placeholder("Filter this page", u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(|st: &mut State| &mut st.browse_query),
        _ => rune::text_input(&s.conflicts_query, SMALL)
            .key("search-conflicts")
            .label("Search conflicting paths")
            .single_line()
            .color(u.ink)
            .placeholder("Search paths", u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(|st: &mut State| &mut st.conflicts_query),
    };
    u.search_shell(field)
}

fn status_bar(s: &State, u: &Ui) -> El {
    let lower = s.status.to_lowercase();
    let tone = if s.status.is_empty() {
        Tone::Quiet
    } else if lower.contains("not ")
        || lower.contains("no ")
        || lower.contains("blocked")
        || lower.contains("failed")
        || lower.contains("error")
        || lower.contains("could not")
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
        .child(text("APPROVAL GATED").font(LABEL).color(u.faint))
        .child(text("·").font(LABEL).color(u.faint).a11y_hidden())
        .child(text("NO TELEMETRY").font(LABEL).color(u.faint))
}

/// The launch approval. Opening Steam is the only thing it does; the app never
/// renames or copies an addon here.
fn launch_layer(s: &State, u: &Ui) -> El {
    let addons = if !s.addons.scanned {
        "The addon folder has not been read yet.".to_string()
    } else if s.addons.addons.is_empty() {
        "No addon files were found. The game launches with its own assets.".to_string()
    } else {
        format!(
            "{} addon{} in the folder, {} enabled.",
            s.addons.addons.len(),
            if s.addons.addons.len() == 1 { "" } else { "s" },
            s.addons.enabled_count()
        )
    };
    let body = col()
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(6.0)
                        .child(u.eyebrow("APP ACTION · STEAM"))
                        .child(text("Launch Deadlock").font(H2).color(u.ink))
                        .child(
                            text("Steam starts the game with the addon folder as it sits on disk. Nothing is copied, renamed or repaired.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("▶", Tone::Accent, 38.0)),
        )
        .child(u.divider())
        .child(u.kv("Game", &s.paths.game, u.ink))
        .child(u.kv("Addons", &addons, u.muted))
        .child(u.kv(
            "gameinfo.gi",
            if s.game_label.contains("addons missing") {
                "the addons folder is missing from the install"
            } else {
                "read by the game only when SearchPaths lists citadel/addons"
            },
            u.faint,
        ))
        .child(
            row()
                .w_full()
                .gap(9.0)
                .child(
                    u.action("Cancel", ActionKind::Ghost)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.launch_dialog = false),
                )
                .child(
                    u.action("Launch through Steam", ActionKind::Primary)
                        .grow(1.0)
                        .on_press(|st: &mut State| st.launch_modded()),
                ),
        );
    let frame = u
        .dialog_shell()
        .key("launch-dialog")
        .role("dialog")
        .label("Launch Deadlock through Steam")
        .w(480.0)
        .padding(1.0)
        .modal(true, |st: &mut State| st.launch_dialog = false)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.launch_dialog = false;
                true
            } else {
                false
            }
        })
        .child(u.dialog_core().w_full().padding(24.0).child(body));
    el().key("launch-modal")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .fill(Color::BLACK.alpha(0.68))
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .padding(34.0)
                .center()
                .child(frame),
        )
}

/// Start the agent host and the library readers once, after Rune has applied
/// the selected scene.
pub fn setup(s: &mut State, waker: Waker) {
    s.wake = Some(waker.clone());
    s.images = Some(ImageCache::new(waker));
    preload_thumbnails(s);
    s.start_host();
    if s.demo || rune::is_headless() {
        return;
    }
    if !s.addons.scanned {
        crate::data::start_addon_scan(s);
    }
    if s.screen == Screen::Browse && s.browse.page.is_none() {
        crate::data::start_browse_fetch(s, 1);
    }
    if s.screen == Screen::Updates && s.release.review.is_none() {
        crate::data::start_review_scan(s);
    }
}

/// A headless scene fetches thumbnails before `setup` because there is no
/// window to wake. Decode is still on Rune's worker, and `wait` makes the
/// fixture deterministic.
fn preload_thumbnails(s: &mut State) {
    let pending = std::mem::take(&mut s.pending_thumbs);
    let Some(cache) = s.images.as_ref() else {
        return;
    };
    for (url, bytes) in pending {
        let image = cache.load(bytes);
        if rune::is_headless() {
            let _ = image.wait();
        }
        s.thumbnails.insert(url, image.pixels().clone());
    }
}

pub fn poll(s: &mut State, t: f64, _tx: &mut Tx) -> bool {
    let mut changed = false;
    while let Some(event) = s.agent.try_event() {
        s.apply_agent_event(event);
        changed = true;
    }
    if let Some(result) = s.addon_job.try_take() {
        match result {
            Ok(library) => crate::data::apply_addon_library(s, library),
            Err(error) => {
                s.addons.scanned = true;
                s.addons.error = Some(error.clone());
                s.status = error;
            }
        }
        changed = true;
    }
    if let Some(result) = s.browse_job.try_take() {
        crate::data::apply_browse_page(s, result);
        changed = true;
    }
    if let Some(result) = s.install_job.try_take() {
        crate::data::apply_install_outcome(
            s,
            result.unwrap_or(crate::screens::browse::InstallOutcome {
                plan: None,
                applied: None,
                error: Some("The install worker stopped before reporting.".to_string()),
            }),
        );
        changed = true;
    }
    if crate::data::drain_thumbnails(s) {
        changed = true;
    }
    if let Some(result) = s.review_job.try_take() {
        crate::data::apply_review_queue(s, result);
        changed = true;
    }
    if let Some(result) = s.profiles_job.try_take() {
        crate::data::apply_profile_list(s, result);
        changed = true;
    }
    if let Some(result) = s.autoexec_job.try_take() {
        crate::data::apply_autoexec(s, result);
        changed = true;
    }
    if let Some(result) = s.stats_job.try_take() {
        crate::data::apply_stats(s, result);
        changed = true;
    }
    if let Some(result) = s.demo_job.try_take() {
        crate::data::apply_demo_list(s, result);
        changed = true;
    }
    if let Some(result) = s.locker_job.try_take() {
        crate::data::apply_locker(s, result);
        changed = true;
    }
    if let Some(result) = s.launch_job.try_take() {
        crate::data::apply_launch(s, result);
        changed = true;
    }
    if let Some(result) = s.toolchain_job.try_take() {
        crate::data::apply_toolchain(s, result);
        changed = true;
    }
    if let Some(result) = s.character_job.try_take() {
        crate::data::apply_character(s, result);
        changed = true;
    }
    if let Some(outcome) = s.publish_job.try_outcome() {
        let ok = outcome.ok;
        s.status = outcome.summary();
        if ok {
            crate::data::refresh_upload(s);
            if let Some(upload) = &s.release.upload {
                s.status
                    .push_str(&format!(" · state {}", upload.state_label()));
                if let Some(url) = &upload.submission_url {
                    s.status.push_str(&format!(" · {url}"));
                }
            } else {
                s.status
                    .push_str(" · no upload.json was written, so the state is unchanged");
            }
        }
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
    if s.screen == Screen::Agent && running {
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
        s.go(screen);
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

// Headless scenes read synchronously before the first frame so a snapshot is
// deterministic. A live scene lets `setup` start the same read on a worker.
pub fn scene_installed(s: &mut State) {
    s.screen = Screen::Installed;
    if rune::is_headless() {
        crate::data::scan_addons_now(s);
    }
}

pub fn scene_browse(s: &mut State) {
    s.screen = Screen::Browse;
    if rune::is_headless() {
        crate::data::browse_now(s, 1);
    }
}

pub fn scene_review(s: &mut State) {
    s.screen = Screen::Stats;
    crate::data::demos_now(s);
    if let Some(first) = s.stats.demos.first().map(|demo| demo.match_id.clone()) {
        crate::data::open_review(s, &first);
    }
}

pub fn scene_toggle(s: &mut State) {
    s.screen = Screen::Installed;
    if rune::is_headless() {
        crate::data::scan_addons_now(s);
        if let Some(addon) = s.addons.addons.iter().find(|addon| addon.enabled) {
            s.addon_dialog = Some((addon.slot, false));
        }
    }
}

pub fn scene_install(s: &mut State) {
    s.screen = Screen::Browse;
    if rune::is_headless() {
        crate::data::browse_now(s, 1);
        if let Some(page) = &s.browse.page
            && let Some(first) = page.mods.first()
        {
            s.install = screens::browse::InstallView::for_mod(first.id, &first.name);
        }
    }
}

pub fn scene_conflicts(s: &mut State) {
    s.screen = Screen::Conflicts;
    if rune::is_headless() {
        crate::data::scan_addons_now(s);
    }
}

pub fn scene_updates(s: &mut State) {
    s.screen = Screen::Updates;
    if rune::is_headless() {
        crate::data::review_now(s);
    }
}

pub fn scene_profiles(s: &mut State) {
    s.screen = Screen::Profiles;
    if rune::is_headless() {
        crate::data::scan_addons_now(s);
        crate::data::profiles_now(s);
        crate::data::load_autoexec_now(s);
    }
}

pub fn scene_crosshair(s: &mut State) {
    s.screen = Screen::Crosshair;
    if rune::is_headless() {
        crate::data::import_crosshair(s);
    }
}

pub fn scene_autoexec(s: &mut State) {
    s.screen = Screen::Autoexec;
    if rune::is_headless() {
        crate::data::load_autoexec_now(s);
    }
}

pub fn scene_stats(s: &mut State) {
    s.screen = Screen::Stats;
    if rune::is_headless() {
        crate::data::demos_now(s);
        crate::data::stats_now(s);
    }
}

pub fn scene_locker(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
}

pub fn scene_launch(s: &mut State) {
    s.screen = Screen::Launch;
    if rune::is_headless() {
        crate::data::launch_now(s);
    }
}

pub fn scene_toolchain(s: &mut State) {
    s.screen = Screen::Toolchain;
    crate::data::toolchain_now(s);
}

pub fn scene_character(s: &mut State) {
    s.screen = Screen::Character;
    if rune::is_headless() {
        crate::data::character_now(s);
    }
}

pub fn scene_agent(s: &mut State) {
    s.screen = Screen::Agent;
}

pub fn scene_demo_agent(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Agent;
}

pub fn scene_demo_installed(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Installed;
}

pub fn scene_demo_conflicts(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Conflicts;
}

pub fn scene_demo_updates(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Updates;
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
    s.release = model::demo_release_state();
    s.release_dialog = ReleaseDialog::None;
    s.screen = Screen::Release;
}

pub fn scene_release_verified(s: &mut State) {
    s.load_demo();
    s.release = model::demo_release_verified_state();
    s.release_dialog = ReleaseDialog::None;
    s.screen = Screen::Release;
}

pub fn scene_release_approval(s: &mut State) {
    s.load_demo();
    s.release = model::demo_release_state();
    s.release_dialog = ReleaseDialog::ApproveUpload;
    s.screen = Screen::Release;
}

pub fn scene_release_confirm(s: &mut State) {
    s.load_demo();
    s.release = model::demo_release_verified_state();
    s.confirm_url = TextEdit::new("https://gamebanana.com/mods/512480");
    s.release_dialog = ReleaseDialog::ConfirmUrl;
    s.screen = Screen::Release;
}

pub fn scene_local_release(s: &mut State) {
    let agent = std::mem::replace(&mut s.agent, AgentLink::disconnected());
    let wake = s.wake.take();
    let images = s.images.take();
    *s = crate::data::init_release_state();
    s.agent = agent;
    s.wake = wake;
    s.images = images;
}

pub fn scene_settings(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Settings;
}

pub fn scene_launch_approval(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Installed;
    s.launch_dialog = true;
}

pub fn scene_empty(s: &mut State) {
    s.load_empty();
}

pub fn scene_local_agent(s: &mut State) {
    s.screen = Screen::Agent;
}

pub fn scene_local_projects(s: &mut State) {
    s.screen = Screen::Projects;
}

pub fn scene_local_patches(s: &mut State) {
    s.screen = Screen::Patches;
}

pub fn scenes() -> &'static [(&'static str, fn(&mut State))] {
    &[
        ("installed", scene_installed),
        ("browse", scene_browse),
        ("install", scene_install),
        ("toggle", scene_toggle),
        ("review", scene_review),
        ("conflicts", scene_conflicts),
        ("updates", scene_updates),
        ("profiles", scene_profiles),
        ("autoexec", scene_autoexec),
        ("crosshair", scene_crosshair),
        ("stats", scene_stats),
        ("locker", scene_locker),
        ("launch", scene_launch),
        ("toolchain", scene_toolchain),
        ("character", scene_character),
        ("agent", scene_agent),
        ("demo-agent", scene_demo_agent),
        ("demo-installed", scene_demo_installed),
        ("demo-conflicts", scene_demo_conflicts),
        ("demo-updates", scene_demo_updates),
        ("projects", scene_projects),
        ("index", scene_index),
        ("patches", scene_patches),
        ("impact", scene_impact),
        ("release", scene_release),
        ("release-verified", scene_release_verified),
        ("release-approval", scene_release_approval),
        ("release-confirm", scene_release_confirm),
        ("settings", scene_settings),
        ("launch-approval", scene_launch_approval),
        ("empty", scene_empty),
        ("local", scene_local_agent),
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
