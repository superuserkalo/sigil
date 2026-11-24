//! The mod-manager shell: state, scenes, sidebar, header, status bar and screen
//! dispatch. The library is the default surface; the agent is one workbench
//! screen inside the same shell.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::screens::settings::i18n::tr;
use rune::{
    Color, Cx, El, ImageCache, Key, KeyEvent, Pixels, TextEdit, Tx, Waker, col, el, row, spacer,
    text,
};
use serde_json::Value;

use crate::agent::{
    self, AgentEvent, AgentLink, AgentStatus, CommandKind, HostError, Pause, ResultEnvelope,
};
use crate::data::ThumbCrew;
use crate::icons;
use crate::jobs::Job;
use crate::model::{
    self, AddonLibrary, BrowseState, Catalog, ImpactReport, IndexRow, Message, PatchNote, Paths,
    Project, Provider, ProviderState, ReleaseDialog, ReleaseState, ReviewQueue, Role,
    Screen, ToolRun,
};
use crate::publish::PublishJob;
use crate::screens;
use crate::ui::{
    ActionKind, BODY, BODY_MED, LABEL, MICRO, NAV, PAGE_X, SMALL_NUM, TITLEBAR_H, Tone,
    Ui,
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
    /// Agent transcript: tool groups the user expanded, keyed by the first message index.
    pub open_tool_groups: Vec<usize>,
    pub composer: TextEdit,
    pub providers: Vec<Provider>,
    pub local_endpoint: TextEdit,
    pub paths: Paths,
    /// The saved settings (`settings.json`), see `crate::prefs`.
    pub prefs: crate::prefs::Prefs,
    pub status: String,
    pub game_detected: bool,
    pub game_label: String,
    pub game_root: Option<PathBuf>,
    pub addons_dir: Option<String>,
    pub model_label: String,
    pub elapsed: String,
    pub agent: AgentLink,
    /// The agent host starts when Agent first opens, not at launch: it is a second process.
    pub host_started: bool,
    pub wake: Option<Waker>,
    pub streaming: Option<usize>,
    pub run_started: Option<Instant>,
    pub tool_started: HashMap<String, Instant>,
    pub addons: AddonLibrary,
    pub addon_job: Job<AddonLibrary>,
    pub installed_query: TextEdit,
    pub installed: screens::installed::View,
    pub browse: BrowseState,
    pub browse_job: Job<model::BrowsePageView>,
    pub browse_query: TextEdit,
    pub install: screens::browse::InstallView,
    pub browse_view: screens::browse::View,
    pub install_job: Job<screens::browse::InstallOutcome>,
    pub conflicts_query: TextEdit,
    pub images: Option<ImageCache>,
    pub thumbnails: HashMap<String, Pixels>,
    pub pending_thumbs: Vec<(String, Vec<u8>)>,
    pub thumb_crew: ThumbCrew,
    /// Windows: drops a system title bar that shows above Sigil's (`win_frame`).
    pub frame_guard: crate::win_frame::Guard,
    pub review_job: Job<ReviewQueue>,
    pub launch_dialog: bool,
    /// The launch confirm is for a vanilla run (mods set aside first).
    pub launch_vanilla: bool,
    pub addon_dialog: Option<(u16, bool)>,
    pub watch: Option<model::WatchState>,
    pub subscriptions: Vec<model::SubscriptionRow>,
    pub host_model: TextEdit,
    pub subscription_log: Option<String>,
    pub crosshair: screens::crosshair::View,
    pub profiles: screens::profiles::View,
    pub profiles_job: Job<screens::profiles::Load>,
    pub autoexec: screens::autoexec::View,
    pub autoexec_original: String,
    pub autoexec_job: Job<screens::autoexec::Read>,
    pub stats: screens::stats::View,
    pub stats_job: Job<screens::stats::Load>,
    pub demo_job: Job<screens::stats::DemoLoad>,
    pub model_review_job: Job<Result<String, String>>,
    pub locker: screens::locker::View,
    pub locker_job: Job<screens::locker::Load>,
    pub launch: screens::launch::View,
    pub launch_job: Job<screens::launch::Read>,
    pub toolchain: screens::toolchain::View,
    pub toolchain_job: Job<deadlock_core::toolchain::ToolchainStatus>,
    pub character: screens::character::View,
    pub character_job: Job<screens::character::Load>,
    pub signin: crate::signin::Flow,
    pub settings: screens::settings::View,
    pub performance: screens::performance::View,
    pub servers: screens::servers::View,
    /// Toasts (`crate::notify`). `status` above feeds it: a line written there becomes a toast.
    pub notices: crate::notify::Notices<State>,
    /// The gameinfo.gi banner (`crate::banner`).
    pub gameinfo: crate::banner::Banner,
    /// Drag, zoom, the shortcuts sheet, What's new and the background blur (`crate::shell`).
    pub shell: crate::shell::Shell,
    /// The status bar's cached counts and its volume popover (`crate::statusbar`).
    pub bar: crate::statusbar::Bar,
    /// Map mod answers, the launch question and the random lists (`screens::launch_extras`).
    pub launch_extras: screens::launch_extras::LaunchExtras,
    /// Custom maps (`screens::maps`).
    pub maps: screens::maps::Page,
    /// The 3D viewer's open viewports (`crate::viewer`).
    pub viewers: crate::viewer::Viewers,
    /// The Game index's Files tab (`screens::explorer`).
    pub explorer: screens::explorer::Explorer,
    /// The Projects page's build tools (`screens::project_tools`).
    pub project_tools: screens::project_tools::Tools,
}

impl Default for State {
    fn default() -> Self {
        State {
            screen: Screen::Browse,
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
            open_tool_groups: Vec::new(),
            composer: TextEdit::new(""),
            providers: model::default_providers(),
            local_endpoint: TextEdit::new(""),
            paths: model::empty_paths(),
            prefs: crate::prefs::Prefs::default(),
            status: String::new(),
            game_detected: false,
            game_label: "Game not detected".to_string(),
            game_root: None,
            addons_dir: None,
            model_label: "No model".to_string(),
            elapsed: "0s".to_string(),
            agent: AgentLink::disconnected(),
            host_started: false,
            wake: None,
            streaming: None,
            run_started: None,
            tool_started: HashMap::new(),
            addons: AddonLibrary::default(),
            addon_job: Job::default(),
            installed_query: TextEdit::new(""),
            installed: screens::installed::View::default(),
            browse: BrowseState::default(),
            browse_job: Job::default(),
            browse_query: TextEdit::new(""),
            install: screens::browse::InstallView::default(),
            browse_view: screens::browse::View::default(),
            install_job: Job::default(),
            conflicts_query: TextEdit::new(""),
            images: None,
            thumbnails: HashMap::new(),
            pending_thumbs: Vec::new(),
            thumb_crew: ThumbCrew::shrinking(),
            frame_guard: Default::default(),
            review_job: Job::default(),
            launch_dialog: false,
            launch_vanilla: false,
            addon_dialog: None,
            watch: None,
            subscriptions: Vec::new(),
            host_model: TextEdit::new(""),
            subscription_log: None,
            crosshair: screens::crosshair::View::default(),
            profiles: screens::profiles::View::default(),
            profiles_job: Job::default(),
            autoexec: screens::autoexec::View::default(),
            autoexec_original: String::new(),
            autoexec_job: Job::default(),
            stats: screens::stats::View::default(),
            stats_job: Job::default(),
            demo_job: Job::default(),
            model_review_job: Job::default(),
            locker: screens::locker::View::default(),
            locker_job: Job::default(),
            launch: screens::launch::View::default(),
            launch_job: Job::default(),
            toolchain: screens::toolchain::View::default(),
            toolchain_job: Job::default(),
            character: screens::character::View::default(),
            character_job: Job::default(),
            signin: crate::signin::Flow::default(),
            settings: screens::settings::View::default(),
            performance: screens::performance::View::default(),
            servers: screens::servers::View::default(),
            notices: crate::notify::Notices::default(),
            gameinfo: crate::banner::Banner::default(),
            shell: crate::shell::Shell::default(),
            bar: crate::statusbar::Bar::default(),
            launch_extras: Default::default(),
            maps: Default::default(),
            viewers: Default::default(),
            explorer: Default::default(),
            project_tools: Default::default(),
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
        // Demo data replaces whatever the real discovery said at startup.
        self.status.clear();
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
        // Settings is a sheet over the current page, not a page.
        if screen == Screen::Settings {
            screens::settings::open(self);
            return;
        }
        // A page turned off in Settings has no route: a shortcut or a link lands on Browse.
        let screen = if self.prefs.page_on(screen) { screen } else { Screen::Browse };
        // The mouse buttons walk this history; `shell::back` and `shell::forward` move it
        // themselves and set `navigating` so the move is not recorded again.
        if self.screen != screen && !self.shell.navigating {
            self.shell.back.push(self.screen);
            if self.shell.back.len() > 64 {
                self.shell.back.remove(0);
            }
            self.shell.forward.clear();
        }
        self.shell.navigating = false;
        self.settings.open = false;
        if under_agent(screen) {
            self.ensure_host();
        }
        // A page lets go of its images when it is left; it decodes them again on return.
        if self.screen != screen {
            match self.screen {
                Screen::Locker => crate::data::release_locker_art(self),
                Screen::Installed => screens::installed::release_previews(self),
                // Leaving Browse by any route (a toast, the stats strip, the sidebar, a key)
                // closes the mod's details, as its X would (#6).
                Screen::Browse => screens::browse::close_details(self),
                _ => {}
            }
        }
        if self.screen != screen
            && let Some(name) = screens::settings::telemetry::page_event(screen)
        {
            screens::settings::telemetry::event(self, name, None);
        }
        self.screen = screen;
        self.status.clear();
        crate::shell_w4::mark_seen(self, screen);
        crate::shell_tips::on_page(self, screen);
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
            Screen::Servers => {
                if !screens::servers::loaded(self) {
                    screens::servers::start_list(self);
                }
            }
            Screen::Maps => screens::maps::on_open(self),
            Screen::Performance => {
                if !self.performance.loaded && !self.performance.read_job.is_running() {
                    screens::performance::start_read(self);
                }
            }
            Screen::Autoexec => {
                // Reload on every visit unless there are edits to keep: the page has no
                // reload button, and the crosshair page writes the same file.
                let fresh = !self.autoexec.loaded
                    || (!self.demo && !self.autoexec.dirty(&self.autoexec_original));
                if fresh && !self.autoexec_job.is_running() {
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
                } else if self.locker.loaded {
                    crate::data::request_locker_grid(self);
                }
            }
            Screen::Launch => {
                if !self.launch.loaded && !self.launch_job.is_running() {
                    crate::data::start_launch_read(self);
                }
            }
            Screen::Downloads => {
                // The queue's worker; the launch read also starts it, and the monitor.
                crate::data::start_launch_read(self);
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
    /// `vanilla` is true when a vanilla run already set the mods aside and only Steam is left.
    pub fn launch_modded(&mut self, vanilla: bool) {
        self.launch_dialog = false;
        if rune::is_headless() {
            self.status = "Headless run: the launch request was not sent to Steam.".to_string();
            return;
        }
        // Mods past slot 99 load through gameinfo.gi's extra folder block, which a game update
        // or a vanilla launch takes out: mount the folders that hold mods before Steam starts.
        // Last line of defence after the launch gate: never start the game without the Mod and
        // Write lines (the build 10973 crash). Adds only those lines, with a backup.
        if let Some(game_root) = self.game_root.clone() {
            let _ = deadlock_core::deploy::ensure_launch_search_paths(&game_root);
        }
        if let Some(game_root) = self.game_root.clone()
            && let Err(error) = deadlock_core::addons::sync_overflow_mount(&deadlock_core::deploy::addons_dir(&game_root))
        {
            crate::notify::push(
                self,
                crate::notify::Notice::warning(tr("Mods past slot 99 may not load"))
                    .detail(format!("Adding their folders to gameinfo.gi failed: {error}")),
            );
        }
        // Locker's "shuffle on launch" already ran in the launch's Roll step
        // (`launch_extras::roll_before_launch`); running it here too shuffled twice.
        let spawn = |program: &str, args: &[&str]| {
            deadlock_core::util::hidden_command(program)
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
                self.settings.launched = true;
                // The marker starts a new session: a crash dump from before it is an old one,
                // and the Launch page must not show it as the last session's crash.
                crate::data::mark_launch(
                    self,
                    if vanilla { deadlock_core::launch::MODE_VANILLA } else { deadlock_core::launch::MODE_MODDED },
                );
                let on = self.addons.enabled_count();
                self.status = if vanilla {
                    "Starting Deadlock without mods.".to_string()
                } else {
                    format!("Starting Deadlock with {on} mod{} on.", if on == 1 { "" } else { "s" })
                };
                // A game update can land with the launch; look at gameinfo.gi again.
                crate::banner::recheck(self);
            }
            Err(error) => self.status = format!("Steam could not be opened: {error}"),
        }
    }

    /// Start the Rust host for the real scenes and the window. Demo scenes get a labelled
    /// fixture link instead of a process, so snapshots stay hermetic.
    /// Start the host the first time anything needs it.
    pub fn ensure_host(&mut self) {
        // Experimental and off by default: no host process unless the Agent flag is on.
        if !self.host_started && (self.demo || self.prefs.flag("agent")) {
            self.start_host();
        }
    }

    pub fn start_host(&mut self) {
        self.host_started = true;
        if self.demo {
            // A scene may have staged its own fixture link (a paused approval); keep it.
            if !self.agent.demo {
                self.agent = AgentLink::demo();
            }
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
        }
        // A healthy start says nothing: the Agent page shows the host's state.
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
        self.ensure_host();
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
        // The transcript shows the run, so a sent prompt needs no toast.
        if !self.agent.prompt(&body) {
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
    crate::ui::set_look(s.prefs.theme, s.prefs.accent_color());
    // Settings > Appearance > Personalize: a custom palette and corner radius, off by default.
    screens::settings::personalize::apply(s);
    // Zoom from the shortcuts, times the density chosen in Appearance (100% by default).
    cx.zoom(s.prefs.zoom * crate::ui::density());
    // Thumbnails decode at the pixels a card covers: the screen scale times the zoom.
    crate::data::set_pixel_scale(cx.env().scale * s.prefs.zoom * crate::ui::density());
    let window = cx
        .layout_size("root")
        .map_or((1360.0, 860.0), |size| (size[0], size[1]));
    // Reduce motion: the system setting, or Settings > Appearance. Every motion token reads it.
    crate::motion::set_reduced(cx.is_reduced_motion() || s.prefs.reduce_motion);
    let u = Ui::new();
    // Transparent sidebar: glass behind the sidebar when the system shows it, solid otherwise.
    crate::glass::set_contrast(cx.system_increased_contrast());
    let glass = crate::glass::live(s.prefs.transparent_sidebar);
    crate::glass::sync(s.prefs.transparent_sidebar, glass, !u.light);
    cx.foreground(u.ink);
    cx.caption_color(u.muted);
    cx.focus_ring(crate::ui::FOCUS_W, crate::ui::FOCUS_GAP, u.accent);
    cx.help_look(u.help_look());
    let insets = cx.chrome_insets();
    crate::ui::set_window(window);
    // The page toolbars fold by width alone: the window less the sidebar and the page margins.
    crate::ui::set_fit(crate::ui::Fit::from_width(
        window.0 - crate::shell_w4::sidebar_width(s, insets.leading) - 2.0 * PAGE_X,
    ));
    // A status line a handler wrote since the last poll becomes a toast on the next one.
    crate::notify::needs_poll(s);
    let body = match s.screen {
        Screen::Installed => screens::installed::view(s, &u),
        Screen::Browse => screens::browse::view(s, &u),
        Screen::Conflicts => screens::conflicts::view(s, &u),
        Screen::Updates => screens::updates::view(s, &u),
        Screen::Profiles => screens::profiles::view(s, &u),
        Screen::Autoexec => screens::autoexec::view(s, &u),
        Screen::Performance => screens::performance::view(s, &u),
        Screen::Servers => screens::servers::view(s, &u),
        Screen::Crosshair => screens::crosshair::view(s, &u),
        Screen::Stats => screens::stats::view(s, &u),
        Screen::Locker => screens::locker::view(s, &u),
        Screen::Launch => screens::launch::view(s, &u),
        Screen::Downloads => screens::downloads::page(s, &u),
        Screen::Agent => screens::workbench::view(s, &u),
        Screen::Projects => screens::projects::view(s, &u),
        Screen::Index => screens::index::view(s, &u),
        Screen::Patches => screens::patches::view(s, &u),
        Screen::Impact => screens::impact::view(s, &u),
        Screen::Release => screens::release::view(s, &u),
        Screen::Toolchain => screens::toolchain::view(s, &u),
        Screen::Character => screens::character::view(s, &u),
        Screen::Maps => screens::maps::view(s, &u),
        // Settings opens as a sheet; a stale Settings screen shows the library under it.
        Screen::Settings => screens::installed::view(s, &u),
    };
    let mut main = col()
        .key("main")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        // The band over the page exists only where it holds something for the whole run: the
        // Windows caption buttons, or the transient chips while the status bar is hidden. Pages
        // otherwise start at the top, and their toolbar line moves the window.
        .children(titlebar_shown(s, insets).then(|| titlebar(s, &u, insets)))
        // macOS without the band: a drag strip that puts the page's toolbar line on the
        // sidebar's brand row (the traffic lights' strip sits above that row).
        .children((!titlebar_shown(s, insets) && insets.leading > 0.0).then(|| {
            el().key("page-lights-strip").w_full().h(LIGHTS_H - crate::ui::PAGE_TOP).window_drag()
        }));
    if let Some(banner) = crate::shell_w4::update_banner(s, &u) {
        main = main.child(banner);
    }
    if let Some(banner) = crate::banner::view(s, &u) {
        main = main.child(banner);
    }
    let mut root = crate::motion::root(row().key("root"))
        .w_full()
        .h_full()
        // Under glass the window is see-through: the pages get their own ground below.
        .fill(if glass.is_some() { Color::TRANSPARENT } else { u.bg })
        // Files dropped on a page without its own target import from anywhere in the window.
        .accepts_extensions(&crate::shell::MODS)
        .on_drop_files(|st: &mut State, files: &[PathBuf], _tx: &mut Tx| {
            crate::shell::dropped(st, files)
        })
        .on_drop_hover(|st: &mut State, over: bool, _tx: &mut Tx| st.shell.drop_hover = over)
        // Any pointer move in the window counts as input for the Browse billboard's idle stop.
        .on_pointer_move_local(|st: &mut State, _p, tx: &mut Tx| screens::browse::top::note_input(st, tx))
        // Mouse Back and Forward walk pages and the details or creator opened over them.
        .pointer_buttons([rune::PointerButton::Back, rune::PointerButton::Forward])
        .on_press_tx(|st: &mut State, tx: &mut Tx| match tx.button() {
            rune::PointerButton::Back => crate::shell::back(st),
            rune::PointerButton::Forward => crate::shell::forward(st),
            _ => {}
        })
        // Back in front: a game update or Steam's file check may have reset gameinfo.gi.
        .on_window_focus(|st: &mut State, focused: bool, _tx: &mut Tx| {
            if focused {
                crate::banner::recheck(st);
                // Windows' transparency effects may have changed meanwhile.
                crate::glass::recheck();
            }
        })
        .child(sidebar(s, &u, insets))
        .child({
            // The page body is keyed per screen: after a sidebar press the new one fades in over
            // 120 ms and the old one is gone in the same frame, so one page is ever mounted.
            let body = col()
                .key(format!("page-{}", s.screen.key()))
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .transition(crate::motion::page_enter())
                .child(body);
            let main = main.child(col().key("page").grow(1.0).min_h(0.0).w_full().child(body));
            match crate::statusbar::view(s, &u) {
                Some(bar) => main.child(bar),
                None => main,
            }
        });
    if glass.is_some() {
        // The pages' opaque ground, under the Personalize backdrop; only the sidebar is glass.
        let left = crate::shell_w4::sidebar_width(s, insets.leading);
        root = root.child(
            el().key("page-ground")
                .pin(Some(left), Some(0.0), Some(0.0), Some(0.0))
                .fill(u.bg)
                .paint_order(-2)
                .a11y_hidden(),
        );
        // A scene previews the glass over a painted wallpaper (headless only).
        if let Some(kind) = crate::glass::preview() {
            root = root.child(glass_wallpaper(window, kind, u.light));
        }
    }
    // Personalize: the background image and glow, painted under the sidebar and pages.
    if let Some(backdrop) = screens::settings::personalize::backdrop(s, &u, window) {
        root = root.child(backdrop);
    }
    // Wave 4: themed art behind the pages (off by default).
    if let Some(art) = screens::settings::misc_w4::themed_art(s, &u, window) {
        root = root.child(art);
    }
    if let Some(menu) = screens::installed::overlay(s, &u) {
        root = root.child(menu);
    }
    if let Some(menu) = screens::browse::overlay(s, &u) {
        root = root.child(menu);
    }
    if let Some(menu) = screens::servers::overlay(s, &u) {
        root = root.child(menu);
    }
    // Row menus and popovers of Locker, Profiles, Autoexec, Crosshair and Overlap: anchored,
    // no dimming layer.
    for menu in [
        screens::locker::overlay(s, &u),
        screens::profiles::overlay(s, &u),
        screens::autoexec::overlay(s, &u),
        screens::crosshair::overlay(s, &u),
        screens::conflicts::overlay(s, &u),
    ]
    .into_iter()
    .flatten()
    {
        root = root.child(menu);
    }
    if let Some(panel) = screens::downloads::panel(s, &u) {
        root = root.child(panel);
    }
    if s.launch_dialog {
        root = root.child(u.modal_layer("launch-run-modal", launch_layer(s, &u)));
    }
    for (key, layer) in [
        ("browse-modal", screens::browse::dialog(s, &u)),
        ("browse-replace-modal", screens::browse::replace::dialog(s, &u)),
        ("profiles-modal", screens::profiles::dialog(s, &u)),
        ("autoexec-modal", screens::autoexec::dialog(s, &u)),
        ("performance-modal", screens::performance::dialog(s, &u)),
        ("settings-review-modal", screens::browse::quick::settings_review::dialog(s, &u)),
        ("servers-modal", screens::servers::dialog(s, &u)),
        ("launch-modal", screens::launch::dialog(s, &u)),
        ("toolchain-modal", screens::toolchain::dialog(s, &u)),
        ("project-tools-modal", screens::project_tools::dialog(s, &u)),
        ("addon-modal", screens::installed::dialog(s, &u)),
        ("review-modal", screens::stats::dialog(s, &u)),
        ("crosshair-modal", screens::crosshair::dialog(s, &u)),
        ("recolor-modal", screens::locker::dialog(s, &u)),
        ("gameinfo-modal", crate::banner::dialog(s, &u)),
        ("map-mods-modal", screens::launch_extras::dialog(s, &u)),
        ("telemetry-modal", screens::settings::telemetry::dialog(s, &u)),
        ("shell-modal", crate::shell::whats_new_dialog(s, &u).or_else(|| crate::shell::shortcuts_dialog(s, &u))),
        ("toy-modal", crate::shell_tips::toy(s, &u)),
        // Preview in 3D, over a mod's details when it opened from there.
        ("viewer-modal", crate::viewer::dialog::view(s, &u)),
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
    if let Some(card) = screens::launch_progress::overlay(s, &u) {
        root = root.child(u.modal_layer("launch-progress-modal", card));
    }
    if let Some(card) = screens::launch_gate::dialog(s, &u) {
        root = root.child(u.modal_layer("launch-gate-modal", card));
    }
    if let Some(sheet) = screens::settings::modal(s, &u, window) {
        root = root.child(sheet);
    }
    // Send diagnostics opens from inside Settings, so it sits over the sheet.
    if let Some(preview) = screens::settings::telemetry::diagnostics_dialog(s, &u) {
        root = root.child(u.modal_layer("diagnostics-modal", preview));
    }
    // A drag anywhere in the window, over any dialog, shows the same target.
    if s.shell.drop_hover {
        root = root.child(crate::shell::drop_veil(&u));
    }
    // Wave 4: seasonal effects over the pages (off by default; never takes input).
    if let Some(layer) = crate::shell_tips::effects(s, &u, window) {
        root = root.child(layer);
    }
    // Wave 4: a feature tip anchored to its control (the Settings one lives in the sheet).
    if !s.settings.open
        && let Some(tip) = crate::shell_tips::view(s, &u, false)
    {
        root = root.child(tip);
    }
    // The 3D viewer full screen, over the pages and dialogs.
    if let Some(full) = crate::viewer::overlay(s, &u) {
        root = root.child(full);
    }
    // The viewer's Controls panel, anchored to its strip button.
    if let Some(panel) = crate::viewer::popover(s, &u) {
        root = root.child(panel);
    }
    // Toasts sit above everything, dialogs included, as Grimoire's do.
    if let Some(toasts) = crate::notify::region(s, &u) {
        root = root.child(toasts);
    }
    root
}

/// The primary places, in Grimoire's order, then Game (the game's state, the crash report, the
/// vanilla stash and Restore) and Downloads (the queue). Everything else is reached from inside
/// a place. "Launch" stays on the two sidebar buttons only.
const PLACES: [Screen; 12] = [
    Screen::Browse,
    Screen::Installed,
    Screen::Locker,
    Screen::Crosshair,
    Screen::Autoexec,
    Screen::Performance,
    Screen::Stats,
    Screen::Conflicts,
    Screen::Profiles,
    Screen::Servers,
    Screen::Launch,
    Screen::Downloads,
];

/// Screens that live under the Agent place, so it stays selected while one is open.
pub fn under_agent(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Agent
            | Screen::Projects
            | Screen::Index
            | Screen::Patches
            | Screen::Impact
            | Screen::Release
            | Screen::Toolchain
            | Screen::Character
    )
}

/// The scene preview's stand-in for the desktop behind the window: white, a bright yellow, a
/// saturated blue and a dark green from top to bottom, the worst a wallpaper does to text on
/// glass. Under Mica or vibrancy the system's own tint (dark or light, as Sigil's theme) veils it.
fn glass_wallpaper(window: (f32, f32), kind: crate::glass::Backdrop, light: bool) -> El {
    let veil = if kind.tinted() {
        if light { Color::WHITE.alpha(0.55) } else { rune::oklch(0.2, 0.0, 0.0).alpha(0.65) }
    } else {
        Color::TRANSPARENT
    };
    el().key("glass-wallpaper")
        .absolute(0.0, 0.0)
        .size(window.0, window.1)
        .paint_order(-3)
        .linear_gradient(
            std::f32::consts::PI,
            [
                (0.0, Color::WHITE),
                (0.35, rune::oklch(0.9, 0.15, 95.0)),
                (0.65, rune::oklch(0.55, 0.2, 260.0)),
                (1.0, rune::oklch(0.25, 0.05, 160.0)),
            ],
        )
        .child(el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(veil))
        .a11y_hidden()
}

/// The macOS traffic lights' own strip over the brand row: they sit 9 to 23 points down.
const LIGHTS_H: f32 = 28.0;

fn sidebar(s: &State, u: &Ui, insets: rune::ChromeInsets) -> El {
    // On glass: the tint over the system's backdrop, a 1 point highlight inside the right edge,
    // and hover and selected fills a step stronger.
    let kind = crate::glass::live(s.prefs.transparent_sidebar);
    let glass = kind.map(|k| u.glass(k.tinted()));
    let on_glass = glass.map(|g| u.on_glass(&g));
    let u = on_glass.as_ref().unwrap_or(u);
    let mut nav = col().key("nav").w_full().px(8.0).gap(2.0);
    for screen in PLACES {
        // Settings > General > Pages turns a page off; it leaves the sidebar and its route.
        if !s.prefs.page_on(screen) {
            continue;
        }
        nav = nav.child(nav_item(s, u, screen));
        // Settings > Experimental > Custom maps adds Maps under Browse (DMM's Maps page).
        if screen == Screen::Browse && s.prefs.page_on(Screen::Maps) {
            nav = nav.child(nav_item(s, u, Screen::Maps));
        }
    }
    if s.prefs.flag("agent") {
        nav = nav.child(el().h(12.0)).child(nav_item(s, u, Screen::Agent));
    }
    let collapsed = s.shell.w4.collapsed;

    // macOS: the traffic lights take the first row, so the lockup gets its own row under them.
    let lights_row = insets.leading > 0.0;
    let foot = if collapsed {
        // Icons only (wave 4): the launch buttons and Settings keep their tooltips.
        col()
            .key("sidebar-foot")
            .w_full()
            .px(8.0)
            .pt(8.0)
            .pb(8.0)
            .gap(4.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
            .child(launch_lock(s, compact_launch(s, u, false)))
            .child(if s.prefs.single_launch { el() } else { launch_lock(s, compact_launch(s, u, true)) })
            .child(el().h(4.0))
            .children(crate::shell_tips::season_mark(s, u))
            // macOS: the traffic lights fill the narrow top row, so Expand sits here, centred.
            .child(if insets.leading > 0.0 {
                row().w_full().center().child(crate::shell_w4::collapse_button(s, u))
            } else {
                el()
            })
            .child(settings_item(s, u))
    } else {
        col()
            .key("sidebar-foot")
            .w_full()
            .px(8.0)
            .pt(8.0)
            .pb(8.0)
            .gap(4.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
            // The game is up: a line that stops it, on the Launch page.
            .children(screens::launch::sidebar_state(s, u))
            .child(if s.prefs.single_launch {
                // Personalize's art for the one button too, from the surface it will use.
                screens::settings::personalize::launch_art(
                    s,
                    u,
                    screens::settings::tabs::single_launch(s, u),
                    s.settings.launch_vanilla_pick,
                )
            } else {
                screens::settings::personalize::launch_art(s, u, launch_lock(s, launch_modded(u)), false)
            })
            .child(if s.prefs.single_launch {
                el()
            } else {
                screens::settings::personalize::launch_art(s, u, launch_lock(s, launch_vanilla(u)), true)
            })
            .child(el().h(4.0))
            .children(crate::shell_tips::season_mark(s, u))
            .child(settings_item(s, u))
    };
    col()
        .key("sidebar")
        .w(crate::shell_w4::sidebar_width(s, insets.leading))
        .h_full()
        .fill(screens::settings::personalize::sidebar_fill(s, u))
        .border_sides([0.0, 1.0, 0.0, 0.0], glass.map_or(u.hair, |g| g.edge))
        .children((kind == Some(crate::glass::Backdrop::Vibrancy)).then(|| {
            // macOS: the vibrancy view under the sidebar. Where no native view backs it (an
            // opaque window, a headless render) it draws the solid sidebar instead.
            el().key("sidebar-material")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .paint_order(-1)
                .system_material(rune::SystemMaterial::Popover)
                .fill(u.sidebar)
                .a11y_hidden()
        }))
        .children((lights_row && !collapsed).then(|| {
            // The traffic lights alone, as tight as they allow; the brand row follows.
            row().key("sidebar-lights").w_full().h(LIGHTS_H).window_drag()
        }))
        .child(if collapsed {
            // Icons only: the rail's top row, with Expand centred where the row is free (on
            // macOS the traffic lights fill it and Expand sits in the foot).
            row()
                .key("brand")
                .w_full()
                .h(TITLEBAR_H)
                .center()
                .window_drag()
                .child(if lights_row { el() } else { crate::shell_w4::collapse_button(s, u) })
        } else {
            row()
                .key("brand")
                .w_full()
                .h(TITLEBAR_H)
                // The mark spans the nav icon column (16 to 36) and the wordmark starts on the
                // label edge (44); the collapse button mirrors the icon axis on the right.
                .pl(16.0)
                .pr(12.0)
                .center_y()
                .window_drag()
                .child(if s.shell.w4.eggs {
                    // Easter eggs: five knocks on the wordmark open the news computer.
                    el().on_press(crate::shell_tips::knock).child(crate::brand::header(s, u))
                } else {
                    crate::brand::header(s, u)
                })
                .child(spacer())
                .child(crate::shell_w4::collapse_button(s, u))
        })
        // The active profile, on the nav grid under the brand: every page gets the band back.
        .child(
            col()
                .key("sidebar-profile")
                .w_full()
                .px(8.0)
                .pt(4.0)
                .child(screens::profiles::titlebar_switcher(s, u, collapsed)),
        )
        .child(
            u.scroll_area(col()
                .key("nav-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .pt(8.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(nav)),
        )
        .child(foot)
}

fn nav_row(u: &Ui, key: String, on: bool) -> El {
    // The fill, icon and label ease to and from selected; the row's place lands at once.
    crate::motion::colors(row())
        .key(key)
        .w_full()
        .h(crate::ui::CONTROL_L)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(if on { u.selected } else { u.hover })
        .press(Color::WHITE.alpha(0.06))
        .focusable(true)
}

fn nav_item(s: &State, u: &Ui, screen: Screen) -> El {
    let on = if screen == Screen::Agent {
        under_agent(s.screen)
    } else {
        s.screen == screen
    };
    let mut r = nav_row(u, format!("nav-{}", screen.key()), on)
        .role("tab")
        .label(tr(screen.title()))
        .checked(on)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| st.go(screen))
        .child(
            rune::icon(screen.icon(), 16.0)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
        .child(
            text(tr(screen.title()))
                .font(NAV)
                .color(if on { u.ink } else { u.prose })
                .grow(1.0)
                .truncate(),
        );
    if s.shell.w4.collapsed {
        // Icons only: the title is the tooltip, and only a warning count stays, as a dot
        // (Grimoire).
        return nav_row(u, format!("nav-{}", screen.key()), on)
            .role("tab")
            .label(tr(screen.title()))
            .help(tr(screen.title()))
            .help_side(rune::Side::Right)
            .checked(on)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.go(screen))
            .px(0.0)
            .center()
            .child(
                rune::icon(screen.icon(), 16.0)
                    .color(if on { u.ink } else { u.muted })
                    .a11y_hidden(),
            )
            .children(
                nav_count(s, screen)
                    .filter(|(_, warn)| *warn)
                    .map(|_| crate::shell_w4::dot(u.amber).pin(None, Some(6.0), Some(10.0), None)),
            );
    }
    // The trailing slot is always there, so a count or "New" arriving never re-truncates the
    // label.
    let mut slot = row().key("nav-slot").min_w(28.0).h(20.0).center_y().justify_end();
    if crate::shell_w4::is_new(s, screen) {
        slot = slot.child(crate::shell_w4::new_pill(u));
    }
    if let Some((label, warn)) = nav_count(s, screen) {
        slot = slot.child(
            text(label)
                .font(SMALL_NUM)
                .color(if warn { u.amber } else { u.muted }),
        );
    }
    r = r.child(slot);
    if let Some(joke) = crate::shell_tips::meme(s, screen) {
        r = r.help(joke);
    }
    // Personalize: art behind the active tab, off by default.
    screens::settings::personalize::tab_art(s, u, r, on)
}

/// A live count. `None` means the scan has not run, so nothing is shown rather than a number
/// the shell does not have.
fn nav_count(s: &State, screen: Screen) -> Option<(String, bool)> {
    // Downloads: queued or running, from the status bar's once-a-second count.
    if screen == Screen::Downloads {
        let queued = s.bar.counts.queued();
        return (queued > 0).then(|| (queued.to_string(), false));
    }
    if !s.addons.scanned || s.addons.error.is_some() {
        return None;
    }
    match screen {
        Screen::Installed if !s.addons.addons.is_empty() => {
            Some((s.addons.addons.len().to_string(), false))
        }
        Screen::Conflicts => {
            let count = screens::conflicts::badge_count(s);
            (count > 0).then(|| (count.to_string(), true))
        }
        _ => None,
    }
}

fn launch_modded(u: &Ui) -> El {
    row()
        .key("launch-modded")
        .w_full()
        .h(crate::ui::CONTROL_L)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.accent)
        .hover(Color::WHITE.alpha(0.10))
        .press(Color::BLACK.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(tr("Launch modded"))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|st: &mut State, tx: &mut Tx| ask_or_launch(st, tx, false))
        .child(
            rune::icon(&icons::PLAY, 16.0)
                .color(u.accent_ink)
                .a11y_hidden(),
        )
        .child(
            text(tr("Launch modded"))
                .font(BODY_MED)
                .color(u.accent_ink)
                .grow(1.0),
        )
}

/// A launch button while a launch runs or the game is up: off, dimmed in place (same size, no
/// layout change), the reason in its tooltip.
pub fn launch_lock(s: &State, button: El) -> El {
    match screens::launch_progress::busy_line(s) {
        Some(why) => button.disabled(true).opacity(0.45).cursor(rune::Cursor::Default).help(why),
        None => button,
    }
}

/// The single launch button in the sidebar (Settings, General).
pub fn launch_from_settings(st: &mut State, tx: &mut Tx, vanilla: bool) {
    ask_or_launch(st, tx, vanilla);
}

/// The sidebar launches at once unless "Ask before launching" is on in Settings.
fn ask_or_launch(st: &mut State, tx: &mut Tx, vanilla: bool) {
    if st.prefs.confirm_launch {
        st.launch_vanilla = vanilla;
        st.launch_dialog = true;
        return;
    }
    let _ = tx;
    launch_from_sidebar(st, vanilla);
}

/// The sidebar launches at once: the press is the user's explicit request. Vanilla sets
/// the mods aside first; modded puts back mods a vanilla run set aside. The steps run through
/// the launch progress overlay (`screens::launch_progress`), which also writes the saved launch
/// options into Steam before Steam gets the game.
fn launch_from_sidebar(st: &mut State, vanilla: bool) {
    screens::launch_progress::begin(st, vanilla, false);
}

fn launch_vanilla(u: &Ui) -> El {
    row()
        .key("launch-vanilla")
        .w_full()
        .h(crate::ui::CONTROL_L)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .press(Color::WHITE.alpha(0.06))
        .focusable(true)
        .role("button")
        .label(tr("Launch vanilla"))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|st: &mut State, tx: &mut Tx| ask_or_launch(st, tx, true))
        .child(rune::icon(&icons::PLAY, 16.0).color(u.muted).a11y_hidden())
        .child(text(tr("Launch vanilla")).font(NAV).color(u.prose).grow(1.0))
}

/// A launch button with its icon only, for the collapsed sidebar. With one launch button the
/// pick from Settings decides which launch it is.
fn compact_launch(s: &State, u: &Ui, vanilla: bool) -> El {
    let vanilla = if s.prefs.single_launch { s.settings.launch_vanilla_pick } else { vanilla };
    let label = tr(if vanilla { "Launch vanilla" } else { "Launch modded" });
    row()
        .key(if vanilla { "launch-vanilla" } else { "launch-modded" })
        .w_full()
        .h(crate::ui::CONTROL_L)
        .center()
        .radius(crate::ui::r(8.0))
        .fill(if vanilla { Color::TRANSPARENT } else { u.accent })
        .hover(if vanilla { u.hover } else { Color::WHITE.alpha(0.10) })
        .focusable(true)
        .role("button")
        .label(label)
        .help(label)
        .help_side(rune::Side::Right)
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |st: &mut State, tx: &mut Tx| ask_or_launch(st, tx, vanilla))
        .child(
            rune::icon(&icons::PLAY, 16.0)
                .color(if vanilla { u.muted } else { u.accent_ink })
                .a11y_hidden(),
        )
}

fn settings_item(s: &State, u: &Ui) -> El {
    let on = s.settings.open;
    if s.shell.w4.collapsed {
        return nav_row(u, "nav-settings".to_string(), on)
            .role("tab")
            .label(tr("Settings"))
            .help(tr("Settings"))
            .help_side(rune::Side::Right)
            .checked(on)
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.go(Screen::Settings))
            .px(0.0)
            .center()
            .child(
                rune::icon(&icons::SETTINGS, 16.0)
                    .color(if on { u.ink } else { u.muted })
                    .a11y_hidden(),
            )
            .children(
                crate::updater::sidebar_badge(s, u).map(|badge| badge.pin(None, Some(6.0), Some(10.0), None)),
            );
    }
    nav_row(u, "nav-settings".to_string(), on)
        .role("tab")
        .label(tr("Settings"))
        .checked(on)
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| st.go(Screen::Settings))
        .child(
            rune::icon(&icons::SETTINGS, 16.0)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
        .child(
            text(tr("Settings"))
                .font(NAV)
                .color(if on { u.ink } else { u.prose })
                .grow(1.0),
        )
        // An update is downloaded and ready: a dot (crate::updater). The version lives in
        // Settings, About.
        .children(crate::updater::sidebar_badge(s, u))
}

/// The band across the top of the content. It moves the window, holds the profile switcher
/// and transient chips, and reports a problem only when there is one. Search lives in each
/// page's toolbar. Windows caption buttons sit over its trailing edge.
/// The band shows for the Windows caption buttons, or while the status bar is hidden (it then
/// carries the chips). Both hold for the whole run, so pages never jump when a chip appears.
fn titlebar_shown(s: &State, insets: rune::ChromeInsets) -> bool {
    insets.trailing > 0.0 || !s.prefs.status_bar
}

fn titlebar(s: &State, u: &Ui, insets: rune::ChromeInsets) -> El {
    let mut bar = row()
        .key("titlebar")
        .w_full()
        .h(TITLEBAR_H)
        .pl(PAGE_X)
        // On the page's right edge, or 12 clear of the Windows caption buttons.
        .pr(if insets.trailing > 0.0 { insets.trailing + 12.0 } else { PAGE_X })
        .gap(8.0)
        .center_y()
        .window_drag();
    if s.demo && !s.prefs.status_bar {
        // Development only: a quiet chip, never the accent.
        bar = bar.child(
            row()
                .h(crate::ui::CONTROL_S)
                .px(8.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .fill(u.over.alpha(0.07))
                .child(text("Demo data").font(MICRO).color(u.muted)),
        );
    }
    bar = bar.child(spacer());
    // With the status bar on, the chips live there; the band keeps only the caption buttons.
    if s.prefs.status_bar {
        return bar;
    }
    // Downloads running: on every page, a press opens the list.
    if let Some(downloads) = screens::downloads::indicator(s, u) {
        bar = bar.child(downloads);
    }
    let review = s
        .release
        .review
        .as_ref()
        .map(|queue| queue.review_count())
        .unwrap_or(0);
    if review > 0 && s.screen != Screen::Updates {
        bar = bar.child(
            u.icon_action_m(
                &icons::REFRESH_CW,
                &format!("{review} to check after the update"),
                ActionKind::Ghost,
            )
            .key("titlebar-updates")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.go(Screen::Updates)),
        );
    }
    if s.agent.pause.is_some() && !under_agent(s.screen) {
        bar = bar.child(
            u.action_m("Approval waiting", ActionKind::Secondary)
                .key("titlebar-approval")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.go(Screen::Agent)),
        );
    }
    bar
}

/// The current page's search field for its own toolbar line (`Ui::page_toolbar`). There is no
/// app-wide search: each page that searches carries its own field.
pub fn page_search(s: &State, u: &Ui) -> El {
    u.page_search(search_field(s, u, None))
}

/// `page_search` with the page's own short placeholder, e.g. "Search mods".
pub fn page_search_as(s: &State, u: &Ui, placeholder: &str) -> El {
    u.page_search(search_field(s, u, Some(placeholder)))
}

/// The key of the current page's search field, for Cmd or Ctrl+F.
fn search_key(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Installed => Some("search-installed"),
        Screen::Browse => Some("search-browse"),
        Screen::Servers => Some("search-servers"),
        Screen::Conflicts => Some("search-conflicts"),
        _ => None,
    }
}

/// The one map from a page to its query, field key, name and default placeholder.
fn search_field(s: &State, u: &Ui, placeholder: Option<&str>) -> El {
    let (edit, key, label, hint): (&rune::TextEdit, &str, &str, &str) = match s.screen {
        Screen::Installed => {
            (&s.installed_query, "search-installed", "Search installed addons", "Search installed")
        }
        Screen::Browse => (&s.browse_query, "search-browse", "Search mods", "Search mods"),
        Screen::Servers => (&s.servers.query, "search-servers", "Search servers", "Search servers"),
        _ => (&s.conflicts_query, "search-conflicts", "Search conflicting paths", "Search paths"),
    };
    let field = rune::text_input(edit, crate::ui::BODY)
        .key(key)
        .label(tr(label))
        .single_line()
        .color(u.ink)
        .placeholder(tr(placeholder.unwrap_or(hint)), u.faint)
        .caret(u.accent, u.accent.alpha(0.20));
    match s.screen {
        Screen::Installed => field.on_edit(|st: &mut State| &mut st.installed_query),
        Screen::Browse => field.on_edit(|st: &mut State| &mut st.browse_query),
        Screen::Servers => field.on_edit(|st: &mut State| &mut st.servers.query),
        _ => field.on_edit(|st: &mut State| &mut st.conflicts_query),
    }
}

/// The launch confirm. Opening Steam is the only thing it does; the app never renames or copies
/// an addon here.
fn launch_layer(s: &State, u: &Ui) -> El {
    let mods = if !s.addons.scanned {
        String::new()
    } else if s.addons.enabled_count() == 0 {
        "No mods are on. The game starts as it ships.".to_string()
    } else {
        let on = s.addons.enabled_count();
        format!("{on} mod{} on.", if on == 1 { "" } else { "s" })
    };
    let vanilla = s.launch_vanilla;
    let mut body = col()
        .w_full()
        .gap(8.0)
        .child(
            text(if vanilla {
                "Launch without mods"
            } else {
                "Launch Deadlock"
            })
            .font(crate::ui::H1)
            .color(u.ink),
        )
        .child(
            text(if vanilla {
                "Sigil sets your mods aside, then Steam starts the game as it ships."
            } else {
                "Steam starts the game with your mods as they are."
            })
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    if !mods.is_empty() && !vanilla {
        body = body.child(text(mods).font(BODY).color(u.muted));
    }
    if s.game_label.contains("addons missing") {
        body = body.child(
            text("The game is not set up to load mods yet. Repair it in Settings first.")
                .font(BODY)
                .color(u.amber)
                .wrap(),
        );
    }
    body = body.child(el().h(12.0)).child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                u.action("Cancel", ActionKind::Ghost)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.launch_dialog = false),
            )
            .child(
                u.icon_action(&icons::PLAY, "Launch", ActionKind::Primary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        st.launch_dialog = false;
                        launch_from_sidebar(st, vanilla);
                    }),
            ),
    );
    u.dialog_shell()
        .key("launch-dialog")
        .role("dialog")
        .label("Launch Deadlock")
        .w(440.0)
        .padding(24.0)
        .modal(true, |st: &mut State| st.launch_dialog = false)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                st.launch_dialog = false;
                true
            } else {
                false
            }
        })
        .child(body)
}

/// Start the agent host and the library readers once, after Rune has applied
/// the selected scene.
pub fn setup(s: &mut State, waker: Waker) {
    s.wake = Some(waker.clone());
    s.images = Some(ImageCache::new(waker));
    crate::brand::load(s);
    preload_thumbnails(s);
    if s.demo || rune::is_headless() {
        // Scenes get their fixture link (or none) as before.
        s.start_host();
        return;
    }
    // The agent host is a second process: it starts when Agent opens (`go`).
    if under_agent(s.screen) {
        s.ensure_host();
    }
    if !s.addons.scanned {
        crate::data::start_addon_scan(s);
    }
    // The titlebar switcher shows the active profile before Profiles is ever opened.
    if !s.profiles.loaded && !s.profiles_job.is_running() {
        crate::data::start_profile_list(s);
    }
    // The process monitor behind the sidebar running row and the download queue.
    crate::data::start_launch_read(s);
    if s.screen == Screen::Browse && s.browse.page.is_none() {
        crate::data::start_browse_fetch(s, 1);
    }
    if s.screen == Screen::Updates && s.release.review.is_none() {
        crate::data::start_review_scan(s);
    }
    // The startup page from Settings: run the read it needs, keeping a discovery warning.
    if !matches!(s.screen, Screen::Browse | Screen::Installed | Screen::Updates) {
        let status = std::mem::take(&mut s.status);
        let screen = s.screen;
        s.go(screen);
        if s.status.is_empty() {
            s.status = status;
        }
    }
    screens::settings::on_startup(s);
    crate::shell::on_startup(s);
    screens::performance::benchmark::on_startup(s);
    crate::banner::recheck(s);
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
    s.frame_guard.poll(_tx);
    let mut changed = crate::signin::poll(s);
    changed |= screens::performance::poll(s);
    changed |= screens::servers::poll(s, t, _tx);
    changed |= screens::profiles::poll(s);
    changed |= screens::installed::poll(s);
    changed |= screens::stats::poll(s);
    changed |= screens::browse::poll(s, _tx);
    changed |= screens::crosshair::poll(s, _tx);
    changed |= screens::settings::poll(s, _tx);
    changed |= screens::launch_progress::poll(s, _tx);
    changed |= screens::downloads::poll(s);
    changed |= crate::statusbar::poll(s);
    changed |= screens::launch_extras::poll(s);
    changed |= screens::maps::poll(s);
    changed |= screens::locker::cursors::poll(s);
    changed |= screens::locker::assets::poll(s);
    changed |= screens::explorer::poll(s);
    changed |= screens::project_tools::poll(s);
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
    // Images that landed fade in over their placeholders (`motion.md` 4.16), decided at the end.
    let thumbs_landed = crate::data::drain_thumbnails(s);
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
    if let Some(result) = s.model_review_job.try_take() {
        crate::data::apply_review_generation(s, result.unwrap_or_else(|error| Err(error)));
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
    changed |= crate::banner::poll(s);
    changed |= crate::shell::poll(s);
    // Last, so a status line any poll above wrote becomes a toast this frame.
    changed |= crate::notify::poll(s, t, _tx);
    if let Some(start) = s.run_started {
        let label = elapsed_label(start.elapsed().as_secs());
        if label != s.elapsed {
            s.elapsed = label;
            changed = true;
        }
    }
    if thumbs_landed {
        // Animated only when the images are the commit's sole change, so no late data
        // elsewhere slides; never under Reduce motion.
        if !changed && !crate::motion::reduced() {
            _tx.animated();
        }
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
    screens::browse::top::note_key(s, tx);
    // The shortcuts sheet, zoom, and the mods folder. These win over the screen chords.
    if crate::shell::on_key(s, ev, tx) {
        return true;
    }
    // Escape leaves the 3D viewer's full screen wherever focus is (it covers the whole window).
    if ev.key == Key::Escape && crate::viewer::escape(s) {
        return true;
    }
    // Cmd or Ctrl+F, or a bare "/", puts the caret in the page's own search field.
    let find = ev.key == Key::Command('f')
        || matches!(&ev.key, Key::Text(t) if t.as_str() == "/" && !ev.mods.shift);
    if find && let Some(key) = search_key(s.screen) {
        tx.focus(key, true);
        return true;
    }
    // Installed: Ctrl or Cmd+A selects every shown mod, with nothing focused (A3).
    if screens::installed::select_all_key(s, ev) {
        return true;
    }
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

pub fn scene_crosshair_apply(s: &mut State) {
    s.screen = Screen::Crosshair;
    if rune::is_headless() {
        crate::data::import_crosshair(s);
    }
    s.crosshair.pending = true;
}

pub fn scene_recolor(s: &mut State) {
    s.screen = Screen::Locker;
    if rune::is_headless() {
        crate::data::locker_now(s);
    }
    let chosen = s
        .locker
        .heroes
        .iter()
        .find(|hero| {
            s.locker.pinned.as_deref().is_some_and(|pinned| {
                let key = deadlock_core::locker::hero_asset_key(pinned);
                deadlock_core::locker::hero_asset_key(&hero.codename) == key
                    || deadlock_core::locker::hero_asset_key(&hero.display_name) == key
            })
        })
        .or_else(|| {
            s.locker.heroes.iter().find(|hero| {
                hero.display_name.eq_ignore_ascii_case("billy")
                    || hero.codename.eq_ignore_ascii_case("punkgoat")
            })
        })
        .or_else(|| s.locker.heroes.first());
    if let Some(hero) = chosen {
        s.locker.selected = Some(hero.codename.clone());
        s.locker.panel = Some(screens::locker::Panel {
            codename: hero.codename.clone(),
            display_name: hero.display_name.clone(),
            in_catalog: hero.in_catalog,
            targets: hero.recolor_paths.clone(),
            palette: 0,
            note: rune::TextEdit::new(""),
            log: None,
        });
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

pub fn scene_agent_approval(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Agent;
    s.agent = AgentLink::demo();
    s.agent.status = AgentStatus::Paused;
    s.agent.pause = Some(Pause {
        tool_call_id: "call-demo-deploy".to_string(),
        tool_name: "deploy_apply".to_string(),
        payload: serde_json::json!({
            "slot": 12,
            "file": "pak12_dir.vpk",
            "from": ".dah/staging/ashfall-012/pak12_dir.vpk",
            "to": "game/citadel/addons/pak12_dir.vpk"
        }),
    });
}

pub fn scene_demo_installed(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Installed;
    screens::installed::stage_demo(s);
}

pub fn scene_installed_menu(s: &mut State) {
    scene_demo_installed(s);
    s.installed.menu = Some(screens::installed::Menu::Card(2));
    s.installed.hovered = Some(2);
}

pub fn scene_installed_import_menu(s: &mut State) {
    scene_demo_installed(s);
    screens::import::stage_demo_sources(s);
    s.installed.menu = Some(screens::installed::Menu::Import);
}

pub fn scene_installed_import(s: &mut State) {
    scene_demo_installed(s);
    let thumbs = screens::installed::demo_thumbs(s, 5);
    screens::import::stage_demo_review(s, &thumbs);
}

/// The local import review of three loose files read as one Haze skin with parts.
pub fn scene_gaps_add_mods(s: &mut State) {
    scene_demo_installed(s);
    screens::import::parts::stage_scene(s, false);
}

/// The same review with "Import as one mod" switched off.
pub fn scene_gaps_add_mods_separate(s: &mut State) {
    scene_demo_installed(s);
    screens::import::parts::stage_scene(s, true);
}

pub fn scene_installed_empty(s: &mut State) {
    screens::installed::stage_empty(s);
}

pub fn scene_installed_empty_menu(s: &mut State) {
    screens::installed::stage_empty(s);
    s.installed.menu = Some(screens::installed::Menu::Managers);
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
    screens::settings::scene_tab(s, screens::settings::Tab::Game);
}

pub fn scene_settings_models(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Models);
}

pub fn scene_settings_signin(s: &mut State) {
    scene_settings_models(s);
    crate::signin::start(s, "codex");
}

pub fn scene_settings_advanced(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Storage);
}

fn scene_settings_backups(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Backups);
}

fn scene_settings_general(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::General);
}

fn scene_settings_appearance(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Appearance);
}

fn scene_settings_mods(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Mods);
}

fn scene_settings_privacy(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Privacy);
}

fn scene_settings_updates(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::About);
}

fn scene_settings_integrations(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Integrations);
}

fn scene_settings_network(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Network);
}

fn scene_settings_logs(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Logs);
}

fn scene_settings_experimental(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Experimental);
}

fn scene_settings_search(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Game);
    s.settings.search = rune::TextEdit::new("disc");
}

fn scene_settings_sharing(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::Privacy);
    s.prefs.share_matches = true;
    s.prefs.single_launch = true;
    s.prefs.transparent_sidebar = true;
    s.settings.sharing.record = Some(screens::settings::sharing::demo_record());
    s.settings.sharing.last = Some(Ok((14, 3)));
}

fn scene_single_launch(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Installed;
    s.prefs.single_launch = true;
    s.prefs.transparent_sidebar = true;
}

fn scene_settings_german(s: &mut State) {
    s.prefs.language = screens::settings::i18n::Lang::German;
    screens::settings::i18n::set(s.prefs.language);
    screens::settings::scene_tab(s, screens::settings::Tab::General);
}

fn scene_settings_about(s: &mut State) {
    screens::settings::scene_tab(s, screens::settings::Tab::About);
}

fn scene_settings_confirm(s: &mut State) {
    scene_settings(s);
    s.settings.confirm = Some(screens::settings::Confirm::LaunchOptions(
        "-novid +fps_max 240".into(),
    ));
}

/// Transparent sidebar over a painted wallpaper: Mica's look on Windows 11 (`glass.rs`).
fn scene_glass_sidebar(s: &mut State) {
    scene_demo_installed(s);
    s.prefs.transparent_sidebar = true;
    crate::glass::set_preview(Some(crate::glass::Backdrop::Mica));
}

/// The Windows 10 blur: no system tint, Sigil's denser one.
fn scene_glass_sidebar_blur(s: &mut State) {
    scene_glass_sidebar(s);
    crate::glass::set_preview(Some(crate::glass::Backdrop::Blur));
}

fn scene_glass_sidebar_light(s: &mut State) {
    scene_glass_sidebar(s);
    s.prefs.theme = crate::ui::Theme::Light;
}

fn scene_settings_light(s: &mut State) {
    scene_settings_appearance(s);
    s.prefs.theme = crate::ui::Theme::Light;
}

fn scene_light_installed(s: &mut State) {
    scene_demo_installed(s);
    s.prefs.theme = crate::ui::Theme::Light;
}

fn scene_light_browse(s: &mut State) {
    screens::browse::scene_fixture(s);
    s.prefs.theme = crate::ui::Theme::Light;
}

fn scene_midnight_installed(s: &mut State) {
    scene_demo_installed(s);
    s.prefs.theme = crate::ui::Theme::Midnight;
    s.prefs.accent = crate::prefs::Accent::Preset("azure");
}

fn scene_mocha_browse(s: &mut State) {
    screens::browse::scene_fixture(s);
    s.prefs.theme = crate::ui::Theme::Mocha;
    s.prefs.accent = crate::prefs::Accent::Preset("emerald");
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
        ("notify-stack", crate::notify::scene_stack),
        ("notify-undo", crate::notify::scene_undo),
        ("notify-banner", crate::notify::scene_with_banner),
        ("toast-launch-busy", crate::notify::scene_launch_busy),
        ("toast-long-error", crate::notify::scene_long_error),
        ("toast-path", crate::notify::scene_path),
        ("toast-stack", crate::notify::scene_toast_stack),
        ("shell-shortcuts", crate::shell::scene_shortcuts),
        ("shell-whats-new", crate::shell::scene_whats_new),
        ("shell-drop", crate::shell::scene_drop),
        ("shell-background", crate::shell::scene_background),
        ("shell-density", crate::shell::scene_density),
        ("gameinfo-banner", crate::banner::scene_banner),
        ("gameinfo-fix", crate::banner::scene_fix),
        ("browse", scene_browse),
        ("install", scene_install),
        ("browse-fixture", screens::browse::scene_fixture),
        ("browse-card-stress", screens::browse::scene_card_stress),
        ("browse-card-stress-de", screens::browse::scene_card_stress_de),
        ("browse-card-stress-small", screens::browse::scene_card_stress_small),
        ("browse-card-stress-large", screens::browse::scene_card_stress_large),
        ("browse-long-filters", screens::browse::scene_long_filters),
        ("browse-details", screens::browse::scene_details),
        ("browse-docked", screens::browse::scene_docked),
        ("browse-filters", screens::browse::scene_filters),
        ("browse-hero-menu", screens::browse::scene_hero_menu),
        ("browse-sort-menu", screens::browse::scene_sort_menu),
        ("browse-more-menu", screens::browse::scene_more_menu),
        ("browse-category-menu", screens::browse::scene_category_menu),
        ("browse-tree-menu", screens::browse::tree::scene_tree_menu),
        ("browse-tree-crumbs", screens::browse::tree::scene_tree_crumbs),
        ("browse-creator", screens::browse::creator_page::scene_creator),
        ("browse-favorites-empty", screens::browse::scene_favorites_empty),
        ("browse-live-filtered", screens::browse::scene_live_filtered),
        ("browse-live-sounds", screens::browse::scene_live_sounds),
        ("browse-filters-exclude", screens::browse::scene_filters_exclude),
        ("browse-date-menu", screens::browse::scene_date_menu),
        ("browse-view-menu", screens::browse::scene_view_menu),
        ("browse-context", screens::browse::scene_context_menu),
        ("browse-list", screens::browse::scene_list),
        ("browse-paged", screens::browse::scene_paged),
        ("browse-dashboard", screens::browse::scene_dashboard),
        ("browse-top-loading", screens::browse::scenes::scene_top_loading),
        ("browse-top-failed", screens::browse::scenes::scene_top_failed),
        ("browse-top-hover", screens::browse::scenes::scene_top_hover),
        ("browse-top-blur", screens::browse::scenes::scene_top_blur),
        ("browse-mirror", screens::browse::scene_mirror),
        ("browse-collections", screens::browse::scene_collections),
        ("browse-collection-open", screens::browse::scene_collection_open),
        ("browse-maps", screens::browse::scene_maps),
        ("browse-combined", screens::browse::scene_combined),
        ("browse-details-extras", screens::browse::scene_details_extras),
        ("browse-hero-picker", screens::browse::scene_hero_picker),
        ("browse-hidden", screens::browse::scene_hidden),
        ("browse-ignored", screens::browse::scene_ignored),
        ("browse-vpk", screens::browse::scene_vpk_picker),
        ("browse-audio", screens::browse::scene_audio),
        ("browse-sounds", screens::browse::scene_sounds),
        ("browse-install-downloading", screens::browse::scene_install_downloading),
        ("browse-install-installing", screens::browse::scene_install_installing),
        ("browse-install-installed", screens::browse::scene_install_installed),
        ("browse-install-update", screens::browse::scene_install_update),
        ("browse-install-mixed", screens::browse::scene_install_mixed),
        ("browse-install-list", screens::browse::scene_install_list),
        ("browse-install-details", screens::browse::scene_install_details),
        ("browse-install-details-done", screens::browse::scene_install_details_done),
        ("browse-install-files-menu", screens::browse::scene_install_files_menu),
        ("toggle", scene_toggle),
        ("review", scene_review),
        ("recolor", scene_recolor),
        ("conflicts", scene_conflicts),
        ("updates", scene_updates),
        ("profiles", scene_profiles),
        ("profiles-list", screens::profiles::scene_list),
        ("profiles-apply", screens::profiles::scene_apply),
        ("profiles-export", screens::profiles::scene_export),
        ("profiles-import", screens::profiles::scene_import),
        ("profiles-import-input", screens::profiles::scene_import_input),
        ("profiles-import-early", screens::profiles::scene_import_early),
        ("profiles-one-skin", screens::one_skin::scene),
        ("profiles-pack-all", screens::installed::pack_all::scene),
        ("profiles-cursor-create", screens::locker::cursor_create::scene),
        ("profiles-create", screens::profiles::scene_create),
        ("autoexec", scene_autoexec),
        ("autoexec-file", screens::autoexec::scene_file),
        ("autoexec-presets", screens::autoexec::scene_presets),
        ("autoexec-empty", screens::autoexec::scene_empty),
        ("autoexec-clear", screens::autoexec::scene_clear),
        ("autoexec-conflict", screens::autoexec::scene_conflict),
        ("autoexec-launch", screens::autoexec::scene_launch),
        ("crosshair", scene_crosshair),
        ("crosshair-apply", scene_crosshair_apply),
        ("crosshair-presets", screens::crosshair::scene_presets),
        ("crosshair-preset-name", screens::crosshair::scene_preset_name),
        ("crosshair-import", screens::crosshair::scene_import),
        ("crosshair-pinned", screens::crosshair::scene_pinned),
        ("crosshair-colour", screens::crosshair::scene_colour),
        ("crosshair-remove", screens::crosshair::scene_remove),
        ("stats", scene_stats),
        ("stats-profile", screens::stats::scene_profile),
        ("stats-drill", screens::stats::scene_drill),
        ("stats-entry", screens::stats::scene_entry),
        ("stats-board", screens::stats::scene_board),
        ("stats-people", screens::stats::scene_people),
        ("stats-tracked", screens::stats::scene_tracked),
        ("stats-live-idle", screens::stats::live::scene_idle),
        ("stats-live-console", screens::stats::live::scene_console),
        ("stats-live-bridge", screens::stats::live::scene_bridge),
        ("stats-live-waiting", screens::stats::live::scene_waiting),
        ("stats-live-ended", screens::stats::live::scene_ended),
        ("stats-live-confirm", screens::stats::live::scene_confirm),
        ("stats-live-condebug", screens::stats::live::scene_condebug),
        ("settings-live", screens::stats::live::scene_settings),
        ("locker", scene_locker),
        ("locker-hero", screens::locker::scene_hero),
        ("locker-browse-hero", screens::locker::scene_browse_hero),
        ("locker-list", screens::locker::scene_list),
        ("locker-starred", screens::locker::scene_starred),
        ("locker-search", screens::locker::scene_search),
        ("locker-sounds", screens::locker::scene_sounds),
        ("locker-card", screens::locker::scene_card),
        ("locker-shuffle", screens::locker::scene_shuffle),
        ("locker-applied", screens::locker::scene_applied),
        ("locker-global", screens::locker::scene_global),
        ("launch", scene_launch),
        ("launch-page", screens::launch::scene_page),
        // The Game page (once Launch); `launch-page` stays as its old name.
        ("game-page", screens::launch::scene_page),
        ("launch-running", screens::launch::scene_running),
        ("launch-crash", screens::launch::scene_crash),
        ("launch-checklist", screens::launch::scene_checklist),
        ("launch-gate", screens::launch_gate::scene),
        ("launch-stop", screens::launch::scene_stop),
        ("launch-vanilla", screens::launch::scene_vanilla),
        ("launch-downloads", screens::launch::scene_downloads),
        // The Downloads page; `launch-downloads` stays as its old name.
        ("downloads-page", screens::launch::scene_downloads),
        ("launch-downloads-review", screens::launch::scene_downloads_review),
        ("conflicts-resolve", screens::conflicts::scene_resolve),
        ("conflicts-ignored", screens::conflicts::scene_ignored),
        ("toolchain", scene_toolchain),
        ("character", scene_character),
        ("agent", scene_agent),
        ("demo-agent", scene_demo_agent),
        ("agent-approval", scene_agent_approval),
        ("demo-installed", scene_demo_installed),
        ("installed-menu", scene_installed_menu),
        ("installed-bulk", screens::installed::scene_bulk),
        ("installed-source-menu", screens::installed::scene_source_menu),
        ("installed-drop", screens::installed::scene_drop),
        ("installed-select", screens::installed::scene_select),
        ("installed-delete", screens::installed::scene_delete),
        ("installed-pending", screens::installed::waiting::scene),
        ("installed-pending-review", screens::installed::waiting::scene_review),
        ("installed-reorder", screens::installed::scene_reorder),
        ("installed-updates", screens::installed::scene_updates),
        ("installed-update-all", screens::installed::scene_update_all),
        ("installed-card-menu", screens::installed::scene_card_menu),
        ("installed-trash", screens::installed::scene_trash),
        ("installed-filters", screens::installed::scene_filters),
        ("installed-list", screens::installed::scene_list),
        ("installed-variants", screens::installed::scene_variants),
        ("installed-tags", screens::installed::scene_tags),
        ("installed-tag-menu", screens::installed::scene_tag_menu),
        ("installed-folder-fix", screens::installed::scene_folder_fix),
        ("installed-lists", screens::installed::scene_lists),
        ("installed-edit", screens::installed::scene_edit),
        ("installed-card-more", screens::installed::scene_card_more),
        ("installed-tools-menu", screens::installed::scene_tools_menu),
        ("installed-safety", screens::installed::scene_safety),
        ("installed-identify", screens::installed::scene_identify),
        ("installed-health", screens::installed::scene_health),
        ("installed-merge", screens::installed::scene_merge),
        ("installed-solo", screens::installed::scene_solo),
        ("installed-bisect", screens::installed::scene_bisect),
        ("installed-recap", screens::installed::scene_recap),
        ("installed-fonts", screens::installed::scene_fonts),
        ("installed-hero-conflict", screens::installed::wave_scenes::hero),
        ("installed-needs", screens::installed::wave_scenes::needs),
        ("installed-breaks", screens::installed::wave_scenes::breaks),
        ("installed-hidden-model", screens::installed::wave_scenes::hidden),
        ("installed-drag", screens::installed::wave_scenes::dragging),
        ("installed-drag-list", screens::installed::wave_scenes::dragging_list),
        ("installed-variants-on", screens::installed::wave_scenes::variants_on),
        ("installed-selection-actions", screens::installed::wave_scenes::selection),
        ("installed-update-changelog", screens::installed::wave_scenes::changelog),
        ("installed-load-order", screens::installed::wave_scenes::load_order),
        ("installed-load-order-open", screens::installed::wave_scenes::load_order_open),
        ("installed-overlap-badges", screens::installed::wave_scenes::overlap_badges),
        ("installed-overlap-list", screens::installed::wave_scenes::overlap_list),
        ("installed-hero-from-models", screens::installed::wave_scenes::hero_from_models),
        ("downloads-slow-toast", screens::installed::wave_scenes::slow_toast),
        ("wave3-pages", screens::wave3_scenes::pages),
        ("wave3-pages-mods-only", screens::wave3_scenes::mods_only),
        ("wave3-status-bar", screens::wave3_scenes::status_bar),
        ("wave3-status-volume", screens::wave3_scenes::status_volume),
        ("wave3-statlocker", screens::wave3_scenes::statlocker),
        ("wave3-statlocker-board", screens::wave3_scenes::statlocker_board),
        ("wave3-statlocker-people", screens::wave3_scenes::statlocker_people),
        ("wave3-browse-strip", screens::wave3_scenes::browse_strip),
        ("wave3-health-outdated", screens::wave3_scenes::health_outdated),
        ("wave3-health-outdated-confirm", screens::wave3_scenes::health_outdated_confirm),
        ("wave3-order-fix", screens::installed::wave_scenes::order_fix),
        ("wave3-lists-random", screens::wave3_scenes::lists_random),
        ("wave3-map-mods", screens::wave3_scenes::map_mods),
        ("wave3-stats-more", screens::wave3_scenes::stats_more),
        ("wave3-locker-cursors", screens::wave3_scenes::locker_cursors),
        ("wave3-repairs", screens::wave3_scenes::repairs),
        ("wave3-repairs-ask", screens::wave3_scenes::repairs_ask),
        ("wave3-install-cancel", screens::wave3_scenes::install_cancel),
        ("wave3-telemetry-consent", screens::wave3_scenes::telemetry_consent),
        ("wave3-telemetry-privacy", screens::wave3_scenes::telemetry_privacy),
        ("wave3-telemetry-diagnostics", screens::wave3_scenes::telemetry_diagnostics),
        // Parity wave 4, UI B.
        ("w4b-settings-open", screens::w4b_scenes::settings_open),
        ("w4b-lightbox", screens::w4b_scenes::lightbox),
        ("w4b-lightbox-docked", screens::w4b_scenes::lightbox_docked),
        ("w4b-proxy-socks", screens::w4b_scenes::proxy_socks),
        ("w4b-sidebar-collapsed", screens::w4b_scenes::sidebar_collapsed),
        ("w4b-update-banner", screens::w4b_scenes::update_banner),
        ("w4b-logs", screens::w4b_scenes::logs),
        ("w4b-logs-ask", screens::w4b_scenes::logs_ask),
        ("w4b-launch-options", screens::w4b_scenes::launch_options),
        ("w4b-channel-notes", screens::w4b_scenes::channel_notes),
        ("w4b-browse-featured", screens::w4b_scenes::browse_featured),
        ("w4b-browse-details-links", screens::w4b_scenes::browse_details_links),
        ("w4b-browse-image-menu", screens::w4b_scenes::browse_image_menu),
        ("w4b-browse-dock-wide", screens::w4b_scenes::browse_dock_wide),
        ("w4b-browse-rate-limited", screens::w4b_scenes::browse_rate_limited),
        ("w4b-troubleshoot", screens::w4b_scenes::troubleshoot),
        ("w4b-catalog", screens::w4b_scenes::catalog),
        ("w4b-stats", screens::w4b_scenes::stats),
        ("w4b-crosshair-hero", screens::w4b_scenes::crosshair_hero),
        ("w4b-onboarding-network", screens::w4b_scenes::onboarding_network),
        ("w4b-onboarding-mods", screens::w4b_scenes::onboarding_mods),
        ("w4b-feature-tip", screens::w4b_scenes::feature_tip),
        ("w4b-tour", screens::w4b_scenes::tour),
        ("w4b-easter-eggs", screens::w4b_scenes::easter_eggs),
        ("w4b-seasons-settings", screens::w4b_scenes::seasons_settings),
        ("w4b-browse-paste", screens::w4b_scenes::browse_paste),
        ("w4b-themed-grid", screens::w4b_scenes::themed_grid),
        ("w4b-downloads-failover", screens::w4b_scenes::downloads_failover),
        ("w4b-lightbox-zoomed", screens::w4b_scenes::lightbox_zoomed),
        ("w4b-browse-paste-files", screens::w4b_scenes::browse_paste_files),
        ("w4b-themed-crest", screens::w4b_scenes::themed_crest),
        ("w4b-seasonal-effects", screens::w4b_scenes::seasonal_effects),
        ("w4b-themed-terminal", screens::w4b_scenes::themed_terminal),
        ("browse-install-enable", screens::browse::scene_install_enable),
        ("browse-replace", screens::browse::scene_replace),
        ("browse-replace-first", screens::browse::scene_replace_first),
        ("browse-vpk-unchecked", screens::browse::scene_vpk_unchecked),
        ("downloads-indicator", screens::launch::scene_downloads_indicator),
        ("downloads-panel", screens::launch::scene_downloads_panel),
        ("downloads-slow", screens::launch::scene_downloads_slow),
        ("downloads-drag", screens::launch::scene_downloads_drag),
        ("launch-progress", screens::launch::scene_progress),
        ("launch-progress-waiting", screens::launch::scene_progress_waiting),
        ("autoexec-flags", screens::autoexec::scene_flags),
        ("installed-import-menu", scene_installed_import_menu),
        ("installed-import", scene_installed_import),
        ("gaps-add-mods", scene_gaps_add_mods),
        ("gaps-add-mods-separate", scene_gaps_add_mods_separate),
        ("installed-empty", scene_installed_empty),
        ("installed-empty-menu", scene_installed_empty_menu),
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
        ("performance", scene_performance),
        ("servers", scene_servers),
        ("servers-detail", scene_servers_detail),
        ("servers-join", scene_servers_join),
        ("servers-join-progress", scene_servers_join_progress),
        ("servers-live", scene_servers_live),
        ("servers-filters", scene_servers_filters),
        ("servers-favorites", scene_servers_favorites),
        ("servers-menu", scene_servers_menu),
        ("servers-connect", scene_servers_connect),
        ("servers-maps", scene_servers_maps),
        ("servers-map-detail", scene_servers_map_detail),
        ("servers-map-unresolved", scene_servers_map_unresolved),
        ("maps", screens::maps::scene_list),
        ("maps-installed", screens::maps::scene_installed),
        ("maps-watching", screens::maps::scene_watching),
        ("maps-invite", screens::maps::scene_invite),
        ("maps-not-installed", screens::maps::scene_not_installed),
        ("maps-unnamed", screens::maps::scene_unnamed),
        ("update-banner-progress", screens::settings::smalls::scene_update_banner),
        ("update-settings-progress", screens::settings::smalls::scene_update_settings),
        ("update-settings-managed", screens::settings::smalls::scene_update_managed),
        ("profiles-snapshots", scene_profiles_snapshots),
        ("profiles-snapshot-restore", scene_profiles_snapshot_restore),
        ("profiles-titlebar", scene_profiles_titlebar),
        ("performance-on", scene_performance_on),
        ("performance-apply", scene_performance_apply),
        ("performance-detail", scene_performance_detail),
        ("performance-detail-on", scene_performance_detail_on),
        ("performance-offline", scene_performance_offline),
        ("performance-fetching", scene_performance_fetching),
        ("performance-first-offline", scene_performance_first_offline),
        ("performance-editor", scene_performance_editor),
        ("performance-editor-history", scene_performance_editor_history),
        ("performance-import", scene_performance_import),
        ("performance-import-paste", scene_performance_import_paste),
        ("performance-edit-value", scene_performance_edit_value),
        ("performance-hosted-files", scene_performance_hosted_files),
        ("performance-hosted-review", scene_performance_hosted_review),
        ("settings-models", scene_settings_models),
        ("settings-signin", scene_settings_signin),
        ("settings-advanced", scene_settings_advanced),
        ("settings-storage", scene_settings_advanced),
        ("settings-backups", scene_settings_backups),
        ("settings-general", scene_settings_general),
        ("settings-appearance", scene_settings_appearance),
        ("settings-mods", scene_settings_mods),
        ("settings-privacy", scene_settings_privacy),
        ("settings-updates", scene_settings_updates),
        ("settings-about", scene_settings_about),
        ("settings-integrations", scene_settings_integrations),
        ("settings-network", scene_settings_network),
        ("settings-logs", scene_settings_logs),
        ("settings-experimental", scene_settings_experimental),
        ("settings-search", scene_settings_search),
        ("settings-german", scene_settings_german),
        ("settings-sharing", scene_settings_sharing),
        ("single-launch", scene_single_launch),
        ("settings-confirm", scene_settings_confirm),
        ("settings-light", scene_settings_light),
        ("glass-sidebar", scene_glass_sidebar),
        ("glass-sidebar-blur", scene_glass_sidebar_blur),
        ("glass-sidebar-light", scene_glass_sidebar_light),
        ("parity-game", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Game)),
        ("parity-backups", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Backups)),
        ("parity-mods", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Mods)),
        ("parity-general", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::General)),
        ("parity-integrations", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Integrations)),
        ("parity-network", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Network)),
        ("parity-logs", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Logs)),
        ("parity-storage", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Storage)),
        ("parity-experimental", |s| screens::settings::extra::scene_parity(s, screens::settings::Tab::Experimental)),
        ("parity-templates", |s| screens::settings::extra::scene_dialog(s, "templates")),
        ("parity-editor", |s| screens::settings::extra::scene_dialog(s, "editor")),
        ("parity-crash", |s| screens::settings::extra::scene_dialog(s, "crash")),
        ("parity-recovery", |s| screens::settings::extra::scene_dialog(s, "recovery")),
        ("parity-source", |s| screens::settings::extra::scene_dialog(s, "source")),
        ("parity-source-add", |s| screens::settings::extra::scene_dialog(s, "source-add")),
        ("parity-onboarding", |s| screens::settings::extra::scene_dialog(s, "onboarding")),
        ("parity-scripts", |s| screens::settings::extra::scene_dialog(s, "scripts")),
        ("parity-mods2", |s| screens::settings::extra::scene_dialog(s, "mods2")),
        ("parity-mods3", |s| screens::settings::extra::scene_dialog(s, "mods3")),
        ("light-installed", scene_light_installed),
        ("light-browse", scene_light_browse),
        ("midnight-installed", scene_midnight_installed),
        ("mocha-browse", scene_mocha_browse),
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
    crate::motion::colors(row())
        .h(crate::ui::CONTROL_M)
        .px(10.0)
        .center()
        .radius(crate::ui::r(4.0))
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
        .child(
            text(label)
                .font(LABEL)
                .color(if on { u.accent } else { u.muted })
                .a11y_hidden(),
        )
}

fn servers_scene_base(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Servers;
    s.game_detected = true;
}

pub fn scene_servers(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene(s);
}

pub fn scene_servers_detail(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_detail(s);
}

pub fn scene_servers_join(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_join(s);
}

pub fn scene_servers_join_progress(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_join_progress(s);
}

pub fn scene_servers_live(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_live(s);
}

pub fn scene_servers_filters(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_filters(s);
}

pub fn scene_servers_favorites(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_favorites(s);
}

pub fn scene_servers_menu(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_menu(s);
}

pub fn scene_servers_connect(s: &mut State) {
    servers_scene_base(s);
    screens::servers::scene_connect(s);
}

pub fn scene_servers_maps(s: &mut State) {
    servers_scene_base(s);
    s.servers.tab = screens::servers::Tab::Maps;
    screens::servers_maps::scene(s);
}

pub fn scene_servers_map_detail(s: &mut State) {
    servers_scene_base(s);
    s.servers.tab = screens::servers::Tab::Maps;
    screens::servers_maps::scene_detail(s);
}

pub fn scene_servers_map_unresolved(s: &mut State) {
    servers_scene_base(s);
    s.servers.tab = screens::servers::Tab::Maps;
    screens::servers_maps::scene_unresolved(s);
}

pub fn scene_profiles_snapshots(s: &mut State) {
    screens::profiles::scene_snapshots(s);
}

pub fn scene_profiles_snapshot_restore(s: &mut State) {
    screens::profiles::scene_snapshot_restore(s);
}

pub fn scene_profiles_titlebar(s: &mut State) {
    screens::profiles::scene_titlebar(s);
}

pub fn scene_performance(s: &mut State) {
    screens::performance::scene(s, "performance");
}

pub fn scene_performance_on(s: &mut State) {
    screens::performance::scene(s, "performance-on");
}

pub fn scene_performance_apply(s: &mut State) {
    screens::performance::scene(s, "performance-apply");
}

pub fn scene_performance_detail(s: &mut State) {
    screens::performance::scene(s, "performance-detail");
}

pub fn scene_performance_detail_on(s: &mut State) {
    screens::performance::scene(s, "performance-detail-on");
}

pub fn scene_performance_offline(s: &mut State) {
    screens::performance::scene(s, "performance-offline");
}

pub fn scene_performance_fetching(s: &mut State) {
    screens::performance::scene(s, "performance-fetching");
}

pub fn scene_performance_first_offline(s: &mut State) {
    screens::performance::scene(s, "performance-first-offline");
}

pub fn scene_performance_editor(s: &mut State) {
    screens::performance::scene(s, "performance-editor");
}

pub fn scene_performance_editor_history(s: &mut State) {
    screens::performance::scene(s, "performance-editor-history");
}

pub fn scene_performance_import(s: &mut State) {
    screens::performance::scene(s, "performance-import");
}

pub fn scene_performance_import_paste(s: &mut State) {
    screens::performance::scene(s, "performance-import-paste");
}

pub fn scene_performance_edit_value(s: &mut State) {
    screens::performance::scene(s, "performance-edit-value");
}

pub fn scene_performance_hosted_files(s: &mut State) {
    screens::performance::scene(s, "performance-hosted-files");
}

pub fn scene_performance_hosted_review(s: &mut State) {
    screens::performance::scene(s, "performance-hosted-review");
}
