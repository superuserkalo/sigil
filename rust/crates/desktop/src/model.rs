//! Domain data shown by the workbench.
//!
//! Adapter seam: `deadlock-core` is under an active port (`crates/deadlock-core`). This crate
//! keeps the render model separate from the source so the UI compiles on its own while that port
//! lands. Every value has an explicit source: `demo_*` fixtures are labelled in the UI, and the
//! default state is empty rather than invented. Swapping these builders for core calls does not
//! change a screen.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Installed,
    Browse,
    Conflicts,
    Updates,
    Profiles,
    Autoexec,
    Stats,
    Locker,
    Launch,
    Agent,
    Projects,
    Index,
    Patches,
    Impact,
    Release,
    Toolchain,
    Character,
    Settings,
}

impl Screen {
    /// Library screens own the top bar search; the rest keep the header quiet.
    pub fn searches(self) -> bool {
        matches!(
            self,
            Screen::Installed | Screen::Browse | Screen::Conflicts
        )
    }

    pub fn is_library(self) -> bool {
        matches!(
            self,
            Screen::Installed
                | Screen::Browse
                | Screen::Conflicts
                | Screen::Updates
                | Screen::Autoexec
                | Screen::Profiles
                | Screen::Stats
                | Screen::Locker
        )
    }

    pub fn key(self) -> &'static str {
        match self {
            Screen::Installed => "installed",
            Screen::Browse => "browse",
            Screen::Conflicts => "conflicts",
            Screen::Updates => "updates",
            Screen::Profiles => "profiles",
            Screen::Autoexec => "autoexec",
            Screen::Stats => "stats",
            Screen::Locker => "locker",
            Screen::Launch => "launch",
            Screen::Agent => "agent",
            Screen::Projects => "projects",
            Screen::Index => "index",
            Screen::Patches => "patches",
            Screen::Impact => "impact",
            Screen::Release => "release",
            Screen::Toolchain => "toolchain",
            Screen::Character => "character",
            Screen::Settings => "settings",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Screen::Installed => "Installed",
            Screen::Browse => "Browse",
            Screen::Conflicts => "Conflicts",
            Screen::Updates => "Updates",
            Screen::Profiles => "Profiles",
            Screen::Autoexec => "Autoexec",
            Screen::Stats => "Stats",
            Screen::Locker => "Locker",
            Screen::Launch => "Launch",
            Screen::Agent => "Agent",
            Screen::Projects => "Projects",
            Screen::Index => "Game Index",
            Screen::Patches => "Patches",
            Screen::Impact => "Impact",
            Screen::Release => "Release",
            Screen::Toolchain => "Toolchain",
            Screen::Character => "Character",
            Screen::Settings => "Settings",
        }
    }

    pub fn kicker(self) -> &'static str {
        match self {
            Screen::Installed => "LIBRARY",
            Screen::Browse => "GAMEBANANA CATALOG",
            Screen::Conflicts => "SHARED PATHS",
            Screen::Updates => "PATCH FEED AND BASELINES",
            Screen::Profiles => "SAVED SLOT SETS",
            Screen::Autoexec => "GAME CONFIG",
            Screen::Stats => "DEADLOCK API AND LOCAL DEMOS",
            Screen::Locker => "HERO ASSETS",
            Screen::Launch => "GAME LAUNCH",
            Screen::Agent => "AGENT SESSION",
            Screen::Projects => "MOD PROJECTS",
            Screen::Index => "GAME PATHS",
            Screen::Patches => "PATCH FEED",
            Screen::Impact => "COMPATIBILITY",
            Screen::Release => "RELEASE AND REVIEW",
            Screen::Toolchain => "DECOMPILE AND COMPILE",
            Screen::Character => "CHARACTER GENERATION",
            Screen::Settings => "LOCAL CONFIGURATION",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Screen::Installed => "Addon slots on disk, with the files the game loads",
            Screen::Browse => "The public Deadlock mod feed, read only",
            Screen::Conflicts => "Game paths that more than one enabled slot replaces",
            Screen::Updates => "Project baselines against the current patch feed",
            Screen::Profiles => "Named sets of enabled slots, captured and restored",
            Screen::Autoexec => "The game config file, read and edited behind approval",
            Screen::Stats => "Rank, heroes, matches and heatmaps from the API, depth from demos",
            Screen::Locker => "Hero renders and asset paths resolved from the local game",
            Screen::Launch => "Modded, vanilla and the restore path",
            Screen::Agent => "Plan, stage and validate with the project in context",
            Screen::Projects => "One workspace per mod, with targets and history",
            Screen::Index => "Exact paths from the installed game, read only",
            Screen::Patches => "Steam news, with the targets each note touches",
            Screen::Impact => "What a patch does to the selected project",
            Screen::Release => "Latest build, deploy plan and the queue after an update",
            Screen::Toolchain => "External decompile and compile tools, with real availability",
            Screen::Character => "Reference to mesh pipeline, stopped at a verified artifact",
            Screen::Settings => "Providers, paths and safety, nothing secret on screen",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Screen::Installed => "▦",
            Screen::Browse => "⊕",
            Screen::Conflicts => "⚑",
            Screen::Updates => "⟳",
            Screen::Profiles => "☰",
            Screen::Autoexec => "≡",
            Screen::Stats => "◉",
            Screen::Locker => "◐",
            Screen::Launch => "▶",
            Screen::Agent => "◆",
            Screen::Projects => "▤",
            Screen::Index => "⌕",
            Screen::Patches => "❐",
            Screen::Impact => "◇",
            Screen::Release => "◈",
            Screen::Toolchain => "⚒",
            Screen::Character => "◭",
            Screen::Settings => "⚙",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Screen::Installed => "CMD 1",
            Screen::Browse => "CMD 2",
            Screen::Conflicts => "CMD 3",
            Screen::Updates => "CMD 4",
            Screen::Profiles => "CMD P",
            Screen::Autoexec => "CMD E",
            Screen::Stats => "CMD 5",
            Screen::Locker => "CMD 6",
            Screen::Launch => "CMD L",
            Screen::Agent => "CMD 7",
            Screen::Projects => "CMD 8",
            Screen::Index => "CMD 9",
            Screen::Patches => "CMD 0",
            Screen::Impact => "",
            Screen::Release => "",
            Screen::Toolchain => "",
            Screen::Character => "",
            Screen::Settings => "CMD ,",
        }
    }

    pub fn from_command(c: char) -> Option<Screen> {
        match c {
            '1' => Some(Screen::Installed),
            '2' => Some(Screen::Browse),
            '3' => Some(Screen::Conflicts),
            '4' => Some(Screen::Updates),
            '5' => Some(Screen::Stats),
            '6' => Some(Screen::Locker),
            '7' => Some(Screen::Agent),
            '8' => Some(Screen::Projects),
            '9' => Some(Screen::Index),
            '0' => Some(Screen::Patches),
            'p' => Some(Screen::Profiles),
            'e' => Some(Screen::Autoexec),
            'l' => Some(Screen::Launch),
            ',' => Some(Screen::Settings),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Unchanged,
    Changed,
    Missing,
    Moved,
}

impl Compat {
    pub fn label(self) -> &'static str {
        match self {
            Compat::Unchanged => "unchanged",
            Compat::Changed => "changed",
            Compat::Missing => "missing",
            Compat::Moved => "moved",
        }
    }

    pub fn headline(self) -> &'static str {
        match self {
            Compat::Unchanged => "Unchanged",
            Compat::Changed => "Changed upstream",
            Compat::Missing => "Missing",
            Compat::Moved => "Moved",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectState {
    Ready,
    Attention,
    Draft,
}

impl ProjectState {
    pub fn label(self) -> &'static str {
        match self {
            ProjectState::Ready => "ready",
            ProjectState::Attention => "attention",
            ProjectState::Draft => "draft",
        }
    }
}

pub struct Target {
    pub path: String,
    pub kind: String,
    pub status: Compat,
}

pub struct Reference {
    pub vpk: String,
    pub mode: String,
}

pub struct HistoryEntry {
    pub version: String,
    pub date: String,
    pub note: String,
}

pub struct Project {
    pub name: String,
    pub version: String,
    pub slot: u8,
    pub state: ProjectState,
    pub updated: String,
    pub summary: String,
    pub targets: Vec<Target>,
    pub references: Vec<Reference>,
    pub history: Vec<HistoryEntry>,
}

impl Project {
    pub fn slot_label(&self) -> String {
        if self.slot == 0 {
            "no slot".to_string()
        } else {
            format!("{:02}", self.slot)
        }
    }

    pub fn slot_file(&self) -> String {
        if self.slot == 0 {
            "not assigned".to_string()
        } else {
            format!("pak{:02}_dir.vpk", self.slot)
        }
    }
}

pub struct Catalog {
    pub citadel: u32,
    pub core: u32,
    pub maps: u32,
    pub indexed_at: String,
    pub source: String,
}

pub struct IndexRow {
    pub path: String,
    pub vpk: String,
    pub size: u64,
    pub kind: String,
    pub hero: Option<String>,
}

impl IndexRow {
    pub fn size_label(&self) -> String {
        format_size(self.size)
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Compact counts for likes and views: 1234 becomes 1.2k, 12639 becomes 12.6k.
pub fn format_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

/// One slot in `game/citadel/addons`, as `list_installed` reports it.
pub struct AddonRow {
    pub slot: u8,
    pub file: String,
    pub size_bytes: u64,
    pub chunk_files: Vec<String>,
    pub entry_count: Option<usize>,
    pub enabled: bool,
}

impl AddonRow {
    pub fn slot_label(&self) -> String {
        format!("{:02}", self.slot)
    }

    pub fn size_label(&self) -> String {
        format_size(self.size_bytes)
    }

    pub fn entries_label(&self) -> String {
        match self.entry_count {
            Some(0) => "no entries".to_string(),
            Some(count) => format!(
                "{count} entr{}",
                if count == 1 { "y" } else { "ies" }
            ),
            None => "entries unknown".to_string(),
        }
    }

    pub fn chunks_label(&self) -> String {
        match self.chunk_files.len() {
            0 => "no chunks".to_string(),
            1 => "1 chunk".to_string(),
            count => format!("{count} chunks"),
        }
    }

    pub fn state_label(&self) -> &'static str {
        if self.enabled { "enabled" } else { "disabled" }
    }
}

/// A game path shipped by more than one enabled slot. `winner` is the lowest
/// slot, which the game loads first.
pub struct ConflictRow {
    pub path: String,
    pub slots: Vec<u8>,
    pub winner: u8,
}

impl ConflictRow {
    pub fn slot_labels(&self) -> Vec<String> {
        self.slots.iter().map(|slot| format!("{slot:02}")).collect()
    }

    pub fn winner_label(&self) -> String {
        format!("slot {:02} wins", self.winner)
    }

    pub fn detail(&self) -> String {
        let overridden: Vec<String> = self
            .slots
            .iter()
            .filter(|slot| **slot != self.winner)
            .map(|slot| format!("{slot:02}"))
            .collect();
        format!(
            "{} enabled slots ship this path. Slot {:02} loads first, so {} {} ignored for this file.",
            self.slots.len(),
            self.winner,
            if overridden.len() == 1 {
                format!("slot {}", overridden[0])
            } else {
                format!("slots {}", overridden.join(", "))
            },
            if overridden.len() == 1 { "is" } else { "are" }
        )
    }
}

/// The whole installed inventory plus the conflict pass. `scanned` separates
/// "not read yet" from a real empty folder.
#[derive(Default)]
pub struct AddonLibrary {
    pub addons: Vec<AddonRow>,
    pub conflicts: Vec<ConflictRow>,
    pub warnings: Vec<String>,
    pub scanned: bool,
    pub error: Option<String>,
}

impl AddonLibrary {
    pub fn enabled_count(&self) -> usize {
        self.addons.iter().filter(|addon| addon.enabled).count()
    }

    pub fn disabled_count(&self) -> usize {
        self.addons.len() - self.enabled_count()
    }

    pub fn chunk_count(&self) -> usize {
        self.addons.iter().map(|addon| addon.chunk_files.len()).sum()
    }

    pub fn total_bytes(&self) -> u64 {
        self.addons
            .iter()
            .map(|addon| addon.size_bytes)
            .sum::<u64>()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} addon{}, {} enabled, {} disabled",
            self.addons.len(),
            if self.addons.len() == 1 { "" } else { "s" },
            self.enabled_count(),
            self.disabled_count()
        )
    }
}

/// One GameBanana row, read only until the download and install backend exists.
pub struct BrowseCard {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub category: String,
    pub updated_at: String,
    pub likes: u64,
    pub views: u64,
    pub profile_url: String,
    pub thumbnail_url: Option<String>,
    pub version: Option<String>,
}

impl BrowseCard {
    pub fn author_label(&self) -> String {
        if self.author.is_empty() {
            "unknown author".to_string()
        } else {
            format!("by {}", self.author)
        }
    }

    pub fn meta_label(&self) -> String {
        let updated = if self.updated_at.is_empty() {
            "update date unknown".to_string()
        } else {
            format!("updated {}", self.updated_at.chars().take(10).collect::<String>())
        };
        format!(
            "{updated} · {} likes · {} views",
            format_count(self.likes),
            format_count(self.views)
        )
    }

    pub fn version_label(&self) -> Option<String> {
        self.version
            .as_ref()
            .filter(|version| !version.is_empty())
            .map(|version| format!("v{version}"))
    }
}

pub struct BrowsePageView {
    pub page: usize,
    pub per_page: usize,
    pub total: usize,
    pub complete: bool,
    pub page_count: usize,
    pub mods: Vec<BrowseCard>,
}

impl BrowsePageView {
    pub fn page_label(&self) -> String {
        format!("page {} of {}", self.page, self.page_count)
    }
}

#[derive(Default)]
pub struct BrowseState {
    pub page: Option<BrowsePageView>,
    pub loading: bool,
    pub error: Option<String>,
}

impl BrowseState {
    pub fn can_prev(&self) -> bool {
        self.page.as_ref().is_some_and(|page| page.page > 1)
    }

    pub fn can_next(&self) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| page.page < page.page_count)
    }
}

pub struct PatchNote {
    pub title: String,
    pub date: String,
    pub build: String,
    pub body: Vec<String>,
    pub changed: Vec<String>,
    pub targets: Vec<String>,
    pub is_new: bool,
    pub source: String,
}

pub struct ImpactCard {
    pub target: String,
    pub status: Compat,
    pub old_path: String,
    pub new_path: Option<String>,
    pub reason: String,
    pub action: String,
}

pub enum WarnLevel {
    Block,
    Warn,
}

pub struct Warning {
    pub level: WarnLevel,
    pub title: String,
    pub detail: String,
    pub action: String,
}

pub struct Correlation {
    pub patch: String,
    pub date: String,
    pub detail: String,
}

pub struct ImpactReport {
    pub project: String,
    pub checked_at: String,
    pub unchanged: u32,
    pub changed: u32,
    pub missing: u32,
    pub moved: u32,
    pub cards: Vec<ImpactCard>,
    pub warnings: Vec<Warning>,
    pub correlation: Option<Correlation>,
}

impl ImpactReport {
    pub fn total(&self) -> u32 {
        self.unchanged + self.changed + self.missing + self.moved
    }
}

/// Everything the Release screen renders. Empty by default: each field is
/// filled by a small read in `data.rs` or by the labelled demo fixture.
#[derive(Default)]
pub struct ReleaseState {
    pub info: Option<ReleaseInfo>,
    pub plan: Option<DeployPlanView>,
    pub plan_error: Option<String>,
    pub publish: Option<PublishInfo>,
    pub upload: Option<UploadInfo>,
    pub upload_error: Option<String>,
    pub lookup_error: Option<String>,
    pub review: Option<ReviewQueue>,
}

pub struct ReleaseInfo {
    pub version: String,
    pub slot: u8,
    pub vpk_file: String,
    pub chunk_files: Vec<String>,
    pub entries: usize,
    pub total_bytes: u64,
    pub created_at: String,
    pub baseline_captured_at: Option<String>,
    pub notes: Option<String>,
    pub dir: String,
    pub vpk_present: bool,
}

impl ReleaseInfo {
    pub fn size_label(&self) -> String {
        format_size(self.total_bytes)
    }

    pub fn chunks_label(&self) -> String {
        if self.chunk_files.is_empty() {
            "none".to_string()
        } else {
            self.chunk_files.join(", ")
        }
    }

    pub fn file_label(&self) -> String {
        if self.vpk_present {
            format!("{} · present", self.vpk_file)
        } else {
            format!("{} · missing", self.vpk_file)
        }
    }
}

pub struct PublishInfo {
    pub dir: String,
    pub archive_file: Option<String>,
    pub archive_sha256: Option<String>,
    pub archive_bytes: Option<u64>,
}

/// The durable GameBanana handoff record at `<publish>/<version>/upload.json`.
/// It is written by `dah publish upload --approve` and updated by
/// `dah publish confirm`. The app only reads it; the browser upload stays the
/// user's action.
pub struct UploadInfo {
    pub state: String,
    pub archive_file: String,
    pub archive_sha256: String,
    pub archive_bytes: u64,
    pub submit_url: String,
    pub submission_url: Option<String>,
    pub verified: bool,
    pub created_at: Option<String>,
    pub confirmed_at: Option<String>,
    pub record_path: String,
}

impl UploadInfo {
    pub fn state_label(&self) -> String {
        if self.state.is_empty() {
            "unknown".to_string()
        } else {
            self.state.clone()
        }
    }

    pub fn state_tone(&self) -> ToneKey {
        match self.state.as_str() {
            "verified" => ToneKey::Ok,
            "recorded" => ToneKey::Info,
            "awaiting_user" => ToneKey::Warn,
            _ => ToneKey::Quiet,
        }
    }

    pub fn archive_size_label(&self) -> String {
        format_size(self.archive_bytes)
    }

    pub fn pending(&self) -> bool {
        self.state == "awaiting_user"
    }

    pub fn submit_label(&self) -> String {
        if self.submit_url.is_empty() {
            "https://gamebanana.com/mods/submit".to_string()
        } else {
            self.submit_url.clone()
        }
    }
}

/// Semantic tone without depending on the UI module, so the data layer stays
/// renderable on its own. The Release screen maps it to a `ui::Tone`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToneKey {
    Ok,
    Warn,
    Info,
    Quiet,
}

/// An in-app modal owned by the Release screen. The dialog layer is mounted at
/// the window root so it can dim the whole canvas without a web view.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ReleaseDialog {
    #[default]
    None,
    ApproveUpload,
    ConfirmUrl,
}

pub struct DeployPlanView {
    pub slot: u8,
    pub target: String,
    pub chunk_targets: Vec<String>,
    pub replace_required: bool,
    pub conflicts: Vec<DeployConflictView>,
    pub gameinfo_repair: bool,
    pub warnings: Vec<String>,
    pub entry_count: usize,
    pub entry_preview: Vec<String>,
}

pub struct DeployConflictView {
    pub slot: u8,
    pub file: String,
    pub paths: Vec<String>,
}

impl DeployConflictView {
    pub fn slot_label(&self) -> String {
        format!("{:02}", self.slot)
    }

    pub fn paths_label(&self) -> String {
        format!(
            "{} shared path{}",
            self.paths.len(),
            if self.paths.len() == 1 { "" } else { "s" }
        )
    }
}

pub struct ReviewQueue {
    pub workspace: String,
    pub catalog_label: String,
    pub items: Vec<ReviewItem>,
    /// Queue-level note: no install, no catalog or a failed scan.
    pub error: Option<String>,
}

impl ReviewQueue {
    pub fn review_count(&self) -> usize {
        self.items.iter().filter(|item| item.needs_review()).count()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} project{}, {} need review",
            self.items.len(),
            if self.items.len() == 1 { "" } else { "s" },
            self.review_count()
        )
    }
}

pub struct PathMove {
    pub from: String,
    pub to: String,
}

pub struct ReviewItem {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub path: String,
    pub slot: Option<u8>,
    pub unchanged: usize,
    pub changed: usize,
    pub missing: usize,
    pub moved: usize,
    pub unbaselined: usize,
    pub migrations: Vec<PathMove>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

impl ReviewItem {
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            self.slug.clone()
        } else {
            self.name.clone()
        }
    }

    pub fn slot_label(&self) -> String {
        match self.slot {
            Some(slot) => format!("{slot:02}"),
            None => "no slot".to_string(),
        }
    }

    pub fn needs_review(&self) -> bool {
        self.error.is_some()
            || !self.migrations.is_empty()
            || !self.warnings.is_empty()
            || self.changed > 0
            || self.missing > 0
            || self.moved > 0
            || self.unbaselined > 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Done,
    Running,
    Queued,
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            RunState::Done => "done",
            RunState::Running => "running",
            RunState::Queued => "queued",
        }
    }
}

pub struct ToolRun {
    pub name: String,
    pub state: RunState,
    pub duration: String,
    pub args: String,
    pub result: String,
}

pub struct Message {
    pub role: Role,
    pub meta: String,
    pub body: String,
    pub model: Option<String>,
    pub tool: Option<ToolRun>,
}

impl Message {
    pub fn user(body: impl Into<String>, meta: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            meta: meta.into(),
            body: body.into(),
            model: None,
            tool: None,
        }
    }

    pub fn assistant(
        body: impl Into<String>,
        meta: impl Into<String>,
        model: Option<&str>,
    ) -> Message {
        Message {
            role: Role::Assistant,
            meta: meta.into(),
            body: body.into(),
            model: model.map(str::to_string),
            tool: None,
        }
    }

    pub fn tool(tool: ToolRun) -> Message {
        Message {
            role: Role::Tool,
            meta: String::new(),
            body: String::new(),
            model: None,
            tool: Some(tool),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProviderState {
    Configured,
    NotConfigured,
}

pub struct Provider {
    pub name: String,
    pub kind: String,
    pub state: ProviderState,
    pub detail: String,
}

pub struct Paths {
    pub data: String,
    pub workspace: String,
    pub staging: String,
    pub game: String,
}

pub fn empty_paths() -> Paths {
    Paths {
        data: "~/Library/Application Support/DeadlockAgentHarness".to_string(),
        workspace: "Not selected".to_string(),
        staging: "Not created".to_string(),
        game: "Not detected".to_string(),
    }
}

pub fn default_providers() -> Vec<Provider> {
    vec![
        Provider {
            name: "Anthropic".to_string(),
            kind: "BYOK".to_string(),
            state: ProviderState::NotConfigured,
            detail: "Set a key with `pnpm dah auth set anthropic`.".to_string(),
        },
        Provider {
            name: "OpenAI".to_string(),
            kind: "BYOK".to_string(),
            state: ProviderState::NotConfigured,
            detail: "Set a key with `pnpm dah auth set openai`.".to_string(),
        },
        Provider {
            name: "Local OpenAI compatible".to_string(),
            kind: "localhost".to_string(),
            state: ProviderState::NotConfigured,
            detail: "Point at a server that speaks the OpenAI API. No key required.".to_string(),
        },
    ]
}

pub fn welcome_messages() -> Vec<Message> {
    vec![Message::assistant(
        "The agent is ready. Open a project to give it context, or use Game Index to find exact game paths.\n\nPrompts run through the local dah-agent host. Each tool call is listed in the timeline; nothing writes to the game install.",
        "now",
        None,
    )]
}

pub fn demo_messages() -> Vec<Message> {
    vec![
        Message::user(
            "Recolor Infernus's jacket to the ash palette and keep the original material layout.",
            "14:02",
        ),
        Message::assistant(
            "I found the hero material set and one exact path in citadel. The jacket is a single material override, so this stays cosmetic.\n\nPlan: read the project manifest, confirm the target against the game index, then stage the replacement. I will not write to the game install.",
            "14:02",
            Some("qwen3-coder-30b"),
        ),
        Message::tool(ToolRun {
            name: "read_project".to_string(),
            state: RunState::Done,
            duration: "6 ms".to_string(),
            args: "{\"project\":\"ashfall-recolor\"}".to_string(),
            result: "4 targets, slot 12, 2 references".to_string(),
        }),
        Message::tool(ToolRun {
            name: "search_paths".to_string(),
            state: RunState::Done,
            duration: "21 ms".to_string(),
            args: "{\"query\":\"infernus_body\",\"vpk\":true}".to_string(),
            result: "1 exact path in pak01_dir.vpk, 2 near matches".to_string(),
        }),
        Message::assistant(
            "The exact path is materials/models/heroes_wip/infernus/infernus_body.vmat_c. The recolor is staged and the payload check is running now. One target is missing upstream, so review Impact before packing.",
            "14:03",
            Some("qwen3-coder-30b"),
        ),
        Message::tool(ToolRun {
            name: "validate_payload".to_string(),
            state: RunState::Running,
            duration: "0.8 s".to_string(),
            args: "{\"staging\":\".dah/staging/ashfall-012\"}".to_string(),
            result: "Checking 4 targets against the payload rules".to_string(),
        }),
    ]
}

pub fn demo_tools() -> Vec<ToolRun> {
    vec![
        ToolRun {
            name: "read_game_status".to_string(),
            state: RunState::Done,
            duration: "8 ms".to_string(),
            args: String::new(),
            result: "citadel found, addons slot 12 empty".to_string(),
        },
        ToolRun {
            name: "read_project".to_string(),
            state: RunState::Done,
            duration: "6 ms".to_string(),
            args: String::new(),
            result: "ashfall-recolor 0.4.2".to_string(),
        },
        ToolRun {
            name: "search_paths".to_string(),
            state: RunState::Done,
            duration: "21 ms".to_string(),
            args: String::new(),
            result: "1 exact, 2 near".to_string(),
        },
        ToolRun {
            name: "validate_payload".to_string(),
            state: RunState::Running,
            duration: "0.8 s".to_string(),
            args: String::new(),
            result: "4 targets".to_string(),
        },
        ToolRun {
            name: "pack_vpk".to_string(),
            state: RunState::Queued,
            duration: String::new(),
            args: String::new(),
            result: "Waits for validation".to_string(),
        },
    ]
}

pub fn demo_projects() -> Vec<Project> {
    vec![
        Project {
            name: "Ashfall Recolor".to_string(),
            version: "0.4.2".to_string(),
            slot: 12,
            state: ProjectState::Attention,
            updated: "2026-11-15 18:04".to_string(),
            summary: "Ash and ember recolor for Infernus. One material set, one particle trail, no gameplay data."
                .to_string(),
            targets: vec![
                Target {
                    path: "materials/models/heroes_wip/infernus/infernus_body.vmat_c".to_string(),
                    kind: "material".to_string(),
                    status: Compat::Changed,
                },
                Target {
                    path: "materials/models/heroes_wip/infernus/infernus_head.vmat_c".to_string(),
                    kind: "material".to_string(),
                    status: Compat::Unchanged,
                },
                Target {
                    path: "particles/abilities/infernus/flame_trail.vpcf_c".to_string(),
                    kind: "particle".to_string(),
                    status: Compat::Moved,
                },
                Target {
                    path: "sounds/heroes/infernus/fire_loop.vsnd_c".to_string(),
                    kind: "sound".to_string(),
                    status: Compat::Missing,
                },
            ],
            references: vec![
                Reference { vpk: "pak01_dir.vpk".to_string(), mode: "read".to_string() },
                Reference { vpk: "pak03_dir.vpk".to_string(), mode: "read".to_string() },
            ],
            history: vec![
                HistoryEntry {
                    version: "0.4.2".to_string(),
                    date: "2026-11-15".to_string(),
                    note: "Retargeted the flame trail after the Hideout update.".to_string(),
                },
                HistoryEntry {
                    version: "0.4.1".to_string(),
                    date: "2026-11-08".to_string(),
                    note: "Added the head material and tightened the palette.".to_string(),
                },
                HistoryEntry {
                    version: "0.4.0".to_string(),
                    date: "2026-10-30".to_string(),
                    note: "First pack. Body material only.".to_string(),
                },
            ],
        },
        Project {
            name: "Hideout HUD".to_string(),
            version: "0.2.0".to_string(),
            slot: 7,
            state: ProjectState::Attention,
            updated: "2026-11-14 09:40".to_string(),
            summary: "Panorama layout refresh for the hideout scoreboard. Slot collides with another local mod."
                .to_string(),
            targets: vec![
                Target {
                    path: "panorama/layout/hud/scoreboard.vxml_c".to_string(),
                    kind: "layout".to_string(),
                    status: Compat::Changed,
                },
                Target {
                    path: "panorama/styles/hud/scoreboard.vcss_c".to_string(),
                    kind: "style".to_string(),
                    status: Compat::Unchanged,
                },
            ],
            references: vec![Reference { vpk: "pak01_dir.vpk".to_string(), mode: "read".to_string() }],
            history: vec![HistoryEntry {
                version: "0.2.0".to_string(),
                date: "2026-11-14".to_string(),
                note: "Moved to slot 07 after the collision check.".to_string(),
            }],
        },
        Project {
            name: "Wardrobe Accents".to_string(),
            version: "1.0.1".to_string(),
            slot: 4,
            state: ProjectState::Ready,
            updated: "2026-11-16 02:18".to_string(),
            summary: "Two shared clothing accents. The last compatibility check was clean.".to_string(),
            targets: vec![
                Target {
                    path: "materials/models/heroes_wip/shared/cloth_accent.vmat_c".to_string(),
                    kind: "material".to_string(),
                    status: Compat::Unchanged,
                },
                Target {
                    path: "materials/models/heroes_wip/shared/leather_trim.vmat_c".to_string(),
                    kind: "material".to_string(),
                    status: Compat::Unchanged,
                },
            ],
            references: vec![Reference {
                vpk: "pak01_dir.vpk".to_string(),
                mode: "read".to_string(),
            }],
            history: vec![HistoryEntry {
                version: "1.0.1".to_string(),
                date: "2026-11-16".to_string(),
                note: "Confirmed both targets against the current index.".to_string(),
            }],
        },
        Project {
            name: "Ember Trail FX".to_string(),
            version: "0.1.3".to_string(),
            slot: 21,
            state: ProjectState::Draft,
            updated: "2026-11-11 21:12".to_string(),
            summary: "Particle-only experiment. Not packed or validated yet.".to_string(),
            targets: vec![Target {
                path: "particles/abilities/infernus/flame_trail.vpcf_c".to_string(),
                kind: "particle".to_string(),
                status: Compat::Moved,
            }],
            references: vec![],
            history: vec![],
        },
    ]
}

pub fn demo_catalog() -> Catalog {
    Catalog {
        citadel: 184_320,
        core: 96_140,
        maps: 12,
        indexed_at: "2026-11-15 18:04".to_string(),
        source: "/Games/Deadlock/game/citadel".to_string(),
    }
}

pub fn demo_index() -> Vec<IndexRow> {
    let rows = [
        (
            "materials/models/heroes_wip/infernus/infernus_body.vmat_c",
            "pak01_dir.vpk",
            2_516_582,
            "material",
            Some("Infernus"),
        ),
        (
            "materials/models/heroes_wip/infernus/infernus_head.vmat_c",
            "pak01_dir.vpk",
            1_887_436,
            "material",
            Some("Infernus"),
        ),
        (
            "models/heroes_wip/infernus/infernus.vmdl_c",
            "pak01_dir.vpk",
            19_083_264,
            "model",
            Some("Infernus"),
        ),
        (
            "particles/abilities/infernus/infernus_flame_trail.vpcf_c",
            "pak01_dir.vpk",
            188_416,
            "particle",
            Some("Infernus"),
        ),
        (
            "sounds/heroes/infernus/infernus_fire_loop.vsnd_c",
            "pak01_dir.vpk",
            655_360,
            "sound",
            Some("Infernus"),
        ),
        (
            "materials/models/heroes_wip/lash/lash_body.vmat_c",
            "pak01_dir.vpk",
            3_251_200,
            "material",
            Some("Lash"),
        ),
        (
            "models/heroes_wip/vindicta/vindicta.vmdl_c",
            "pak01_dir.vpk",
            24_117_248,
            "model",
            Some("Vindicta"),
        ),
        (
            "panorama/layout/hud/scoreboard.vxml_c",
            "pak01_dir.vpk",
            47_104,
            "layout",
            None,
        ),
        (
            "maps/midtown/midtown.vmap_c",
            "citadel/maps/pak01_dir.vpk",
            88_604_672,
            "map",
            None,
        ),
    ];
    rows.into_iter()
        .map(|(path, vpk, size, kind, hero)| IndexRow {
            path: path.to_string(),
            vpk: vpk.to_string(),
            size,
            kind: kind.to_string(),
            hero: hero.map(str::to_string),
        })
        .collect()
}

pub fn demo_patches() -> Vec<PatchNote> {
    vec![
        PatchNote {
            title: "The Hideout Update".to_string(),
            date: "2026-11-14".to_string(),
            build: "2.11.14".to_string(),
            body: vec![
                "The Hideout gets a new social space, a refreshed scoreboard and a pass over ambient audio. Matchmaking is unchanged.".to_string(),
                "Hero assets were reorganised under heroes_wip. Mods that pin a material or particle path should re-check their targets before packing.".to_string(),
            ],
            changed: vec![
                "Infernus, Lash and Vindicta material paths moved under heroes_wip.".to_string(),
                "The flame trail particle was renamed and its spawn rate changed.".to_string(),
                "Hideout panorama layout and scoreboard styles were revised.".to_string(),
            ],
            targets: vec![
                "materials/models/heroes_wip/infernus/infernus_body.vmat_c".to_string(),
                "particles/abilities/infernus/infernus_flame_trail.vpcf_c".to_string(),
                "panorama/layout/hud/scoreboard.vxml_c".to_string(),
            ],
            is_new: true,
            source: "Steam news · demo fixture".to_string(),
        },
        PatchNote {
            title: "Balance pass: mobility and lane items".to_string(),
            date: "2026-11-12".to_string(),
            build: "2.11.12".to_string(),
            body: vec![
                "Movement acceleration and a set of lane item costs changed. Several ability descriptions were corrected.".to_string(),
                "This note does not touch compiled cosmetic assets. It is listed because it updates gameplay data that a mod must never ship.".to_string(),
            ],
            changed: vec![
                "Lane item costs adjusted across the early game.".to_string(),
                "Movement acceleration tuned for three heroes.".to_string(),
            ],
            targets: vec![],
            is_new: true,
            source: "Steam news · demo fixture".to_string(),
        },
        PatchNote {
            title: "Hotfix: matchmaking and stability".to_string(),
            date: "2026-11-09".to_string(),
            build: "2.11.9".to_string(),
            body: vec![
                "A server-side matchmaking fix and two client crash fixes. No asset paths changed.".to_string(),
            ],
            changed: vec!["No cosmetic asset paths changed.".to_string()],
            targets: vec![],
            is_new: false,
            source: "Steam news · demo fixture".to_string(),
        },
        PatchNote {
            title: "Hero Lab: new experiments".to_string(),
            date: "2026-11-05".to_string(),
            build: "2.11.6".to_string(),
            body: vec![
                "Hero Lab opens a rotating experimental queue. Asset sets for the featured heroes are staged under heroes_staging and may move without notice.".to_string(),
            ],
            changed: vec![
                "Experimental hero assets live under models/heroes_staging.".to_string(),
                "Do not target staging paths in a published mod.".to_string(),
            ],
            targets: vec!["models/heroes_staging/infernus/infernus.vmdl_c".to_string()],
            is_new: false,
            source: "Steam news · demo fixture".to_string(),
        },
    ]
}

pub fn demo_impact() -> ImpactReport {
    ImpactReport {
        project: "Ashfall Recolor".to_string(),
        checked_at: "2026-11-15 18:04".to_string(),
        unchanged: 1,
        changed: 1,
        missing: 1,
        moved: 1,
        cards: vec![
            ImpactCard {
                target: "infernus_body.vmat_c".to_string(),
                status: Compat::Changed,
                old_path: "materials/models/heroes_wip/infernus/infernus_body.vmat_c".to_string(),
                new_path: None,
                reason: "The path exists, but the material was revised in the Hideout update. The staged override still loads and may look different.".to_string(),
                action: "Re-export from the current source before packing.".to_string(),
            },
            ImpactCard {
                target: "infernus_head.vmat_c".to_string(),
                status: Compat::Unchanged,
                old_path: "materials/models/heroes_wip/infernus/infernus_head.vmat_c".to_string(),
                new_path: None,
                reason: "Path and size match the current index. No action needed.".to_string(),
                action: "Keep as is.".to_string(),
            },
            ImpactCard {
                target: "flame_trail.vpcf_c".to_string(),
                status: Compat::Moved,
                old_path: "particles/abilities/infernus/flame_trail.vpcf_c".to_string(),
                new_path: Some("particles/abilities/infernus/infernus_flame_trail.vpcf_c".to_string()),
                reason: "The particle was renamed in the Hideout update. A payload at the old path is ignored by the game.".to_string(),
                action: "Update the target path in the project manifest.".to_string(),
            },
            ImpactCard {
                target: "fire_loop.vsnd_c".to_string(),
                status: Compat::Missing,
                old_path: "sounds/heroes/infernus/fire_loop.vsnd_c".to_string(),
                new_path: None,
                reason: "Not present at this path after the 11-14 update. The index has a different fire loop at sounds/heroes/infernus/infernus_fire_loop.vsnd_c.".to_string(),
                action: "Retarget the sound or drop it from the project.".to_string(),
            },
        ],
        warnings: vec![
            Warning {
                level: WarnLevel::Block,
                title: "gameinfo.gi does not list citadel/addons".to_string(),
                detail: "The game will ignore every addon until `Game citadel/addons` is listed above `Game citadel` in SearchPaths. A Steam file verification resets this file.".to_string(),
                action: "Repair gameinfo".to_string(),
            },
            Warning {
                level: WarnLevel::Warn,
                title: "Shared paths collide with Hideout HUD (slot 07)".to_string(),
                detail: "Both projects ship two of the same paths. Lower number wins, so pak07 overrides pak12 for those paths. Ashfall loses its changes while both are enabled.".to_string(),
                action: "Review slots".to_string(),
            },
        ],
        correlation: Some(Correlation {
            patch: "The Hideout Update".to_string(),
            date: "2026-11-14".to_string(),
            detail: "3 of 4 targets correlate with this note: 1 path move, 1 material revision and 1 removed sound.".to_string(),
        }),
    }
}

pub fn demo_providers() -> Vec<Provider> {
    vec![
        Provider {
            name: "Local OpenAI compatible".to_string(),
            kind: "localhost".to_string(),
            state: ProviderState::Configured,
            detail: "http://127.0.0.1:8080/v1 · qwen3-coder-30b".to_string(),
        },
        Provider {
            name: "Anthropic".to_string(),
            kind: "BYOK".to_string(),
            state: ProviderState::Configured,
            detail: "Key present in the OS credential store. The window sees status only."
                .to_string(),
        },
        Provider {
            name: "OpenAI".to_string(),
            kind: "BYOK".to_string(),
            state: ProviderState::NotConfigured,
            detail: "Set a key with `pnpm dah auth set openai`.".to_string(),
        },
    ]
}

pub fn demo_paths() -> Paths {
    Paths {
        data: "~/Library/Application Support/DeadlockAgentHarness".to_string(),
        workspace: "~/Deadlock Mods/ashfall-recolor".to_string(),
        staging: ".dah/staging/ashfall-012".to_string(),
        game: "~/Library/Application Support/Steam/steamapps/common/Deadlock".to_string(),
    }
}

/// Fixture library for the explicit demo scenes. It is never loaded by the
/// window or by a real scene; the `DEMO DATA` chip is the label.
pub fn demo_addon_library() -> AddonLibrary {
    let addons = vec![
        AddonRow {
            slot: 1,
            file: "pak01_dir.vpk".to_string(),
            size_bytes: 18_663_424,
            chunk_files: vec!["pak01_000.vpk".to_string()],
            entry_count: Some(148),
            enabled: true,
        },
        AddonRow {
            slot: 2,
            file: "pak02_dir.vpk".to_string(),
            size_bytes: 4_128_768,
            chunk_files: Vec::new(),
            entry_count: Some(36),
            enabled: true,
        },
        AddonRow {
            slot: 3,
            file: "pak03_dir.vpk".to_string(),
            size_bytes: 9_437_184,
            chunk_files: vec!["pak03_000.vpk".to_string()],
            entry_count: Some(81),
            enabled: true,
        },
        AddonRow {
            slot: 4,
            file: "pak04_dir.vpk".to_string(),
            size_bytes: 2_412_544,
            chunk_files: Vec::new(),
            entry_count: Some(22),
            enabled: true,
        },
        AddonRow {
            slot: 5,
            file: "pak05_dir.vpk".to_string(),
            size_bytes: 30_838_784,
            chunk_files: vec![
                "pak05_000.vpk".to_string(),
                "pak05_001.vpk".to_string(),
            ],
            entry_count: Some(212),
            enabled: true,
        },
        AddonRow {
            slot: 7,
            file: "pak07_dir.vpk".to_string(),
            size_bytes: 1_048_576,
            chunk_files: Vec::new(),
            entry_count: Some(14),
            enabled: true,
        },
        AddonRow {
            slot: 9,
            file: "pak09_dir.vpk.disabled".to_string(),
            size_bytes: 12_582_912,
            chunk_files: Vec::new(),
            entry_count: Some(64),
            enabled: false,
        },
        AddonRow {
            slot: 11,
            file: "pak11_dir.vpk.disabled".to_string(),
            size_bytes: 6_291_456,
            chunk_files: Vec::new(),
            entry_count: Some(41),
            enabled: false,
        },
        AddonRow {
            slot: 12,
            file: "pak12_dir.vpk.disabled".to_string(),
            size_bytes: 5_248_950,
            chunk_files: Vec::new(),
            entry_count: Some(4),
            enabled: false,
        },
        AddonRow {
            slot: 18,
            file: "pak18_dir.vpk.disabled".to_string(),
            size_bytes: 855_244,
            chunk_files: Vec::new(),
            entry_count: Some(9),
            enabled: false,
        },
    ];
    let conflicts = vec![
        ConflictRow {
            path: "materials/models/heroes_wip/infernus/infernus_body.vmat_c".to_string(),
            slots: vec![1, 5, 7],
            winner: 1,
        },
        ConflictRow {
            path: "panorama/layout/hud/scoreboard.vxml_c".to_string(),
            slots: vec![2, 7],
            winner: 2,
        },
        ConflictRow {
            path: "particles/abilities/infernus/infernus_flame_trail.vpcf_c".to_string(),
            slots: vec![3, 5],
            winner: 3,
        },
        ConflictRow {
            path: "sounds/heroes/infernus/infernus_fire_loop.vsnd_c".to_string(),
            slots: vec![1, 4],
            winner: 1,
        },
        ConflictRow {
            path: "models/heroes_wip/lash/lash_body.vmdl_c".to_string(),
            slots: vec![3, 5],
            winner: 3,
        },
    ];
    AddonLibrary {
        addons,
        conflicts,
        warnings: vec![
            "Addon pak18_dir.vpk.disabled could not be read, so its paths are not compared."
                .to_string(),
        ],
        scanned: true,
        error: None,
    }
}

pub fn demo_release_state() -> ReleaseState {
    let release_dir = "~/Deadlock Mods/ashfall-recolor/builds/0.4.2".to_string();
    let publish_dir = "~/Deadlock Mods/ashfall-recolor/publish/0.4.2".to_string();
    let addons =
        "~/Library/Application Support/Steam/steamapps/common/Deadlock/game/citadel/addons";
    let info = ReleaseInfo {
        version: "0.4.2".to_string(),
        slot: 12,
        vpk_file: "pak12_dir.vpk".to_string(),
        chunk_files: Vec::new(),
        entries: 4,
        total_bytes: 5_248_950,
        created_at: "2026-11-15T18:04:12.000Z".to_string(),
        baseline_captured_at: Some("2026-11-15T17:52:31.000Z".to_string()),
        notes: Some("Retargeted the flame trail after the Hideout update.".to_string()),
        dir: release_dir,
        vpk_present: true,
    };
    let publish = PublishInfo {
        dir: publish_dir.clone(),
        archive_file: Some("ashfall-recolor-0.4.2.zip".to_string()),
        archive_sha256: Some(
            "8f1c0a5d2b7e4493a6c8d1f0e5b2937461c8d5a9e2f7b0436d9a1c7e5b8f2d64".to_string(),
        ),
        archive_bytes: Some(5_201_408),
    };
    let upload = UploadInfo {
        state: "awaiting_user".to_string(),
        archive_file: "ashfall-recolor-0.4.2.zip".to_string(),
        archive_sha256: "8f1c0a5d2b7e4493a6c8d1f0e5b2937461c8d5a9e2f7b0436d9a1c7e5b8f2d64"
            .to_string(),
        archive_bytes: 5_201_408,
        submit_url: "https://gamebanana.com/mods/submit".to_string(),
        submission_url: None,
        verified: false,
        created_at: Some("2026-11-15T18:11:02.000Z".to_string()),
        confirmed_at: None,
        record_path: format!("{publish_dir}/upload.json"),
    };
    let plan = DeployPlanView {
        slot: 12,
        target: format!("{addons}/pak12_dir.vpk"),
        chunk_targets: Vec::new(),
        replace_required: false,
        conflicts: vec![DeployConflictView {
            slot: 7,
            file: "pak07_dir.vpk".to_string(),
            paths: vec![
                "materials/models/heroes_wip/infernus/infernus_body.vmat_c".to_string(),
                "panorama/layout/hud/scoreboard.vxml_c".to_string(),
            ],
        }],
        gameinfo_repair: true,
        warnings: vec![
            "gameinfo.gi has no `Game citadel/addons` SearchPaths line. Deploy plans a repair before the copy."
                .to_string(),
            "Lower slot 07 already ships 2 of these paths. pak07 wins for those files while both mods are enabled."
                .to_string(),
        ],
        entry_count: 4,
        entry_preview: vec![
            "materials/models/heroes_wip/infernus/infernus_body.vmat_c".to_string(),
            "materials/models/heroes_wip/infernus/infernus_head.vmat_c".to_string(),
            "particles/abilities/infernus/infernus_flame_trail.vpcf_c".to_string(),
            "panorama/layout/hud/scoreboard.vxml_c".to_string(),
        ],
    };
    let review = ReviewQueue {
        workspace: "~/Deadlock Mods".to_string(),
        catalog_label: "312,448 paths from 2 cached game sources".to_string(),
        items: vec![
            ReviewItem {
                slug: "ashfall-recolor".to_string(),
                name: "Ashfall Recolor".to_string(),
                version: "0.4.2".to_string(),
                path: "~/Deadlock Mods/mods/ashfall-recolor".to_string(),
                slot: Some(12),
                unchanged: 1,
                changed: 1,
                missing: 1,
                moved: 1,
                unbaselined: 0,
                migrations: vec![PathMove {
                    from: "particles/abilities/infernus/flame_trail.vpcf_c".to_string(),
                    to: "particles/abilities/infernus/infernus_flame_trail.vpcf_c".to_string(),
                }],
                warnings: Vec::new(),
                error: None,
            },
            ReviewItem {
                slug: "hideout-hud".to_string(),
                name: "Hideout HUD".to_string(),
                version: "0.2.0".to_string(),
                path: "~/Deadlock Mods/mods/hideout-hud".to_string(),
                slot: Some(7),
                unchanged: 1,
                changed: 1,
                missing: 0,
                moved: 0,
                unbaselined: 0,
                migrations: Vec::new(),
                warnings: vec!["slot pak07_dir.vpk already exists in the addon folder".to_string()],
                error: None,
            },
            ReviewItem {
                slug: "wardrobe-accents".to_string(),
                name: "Wardrobe Accents".to_string(),
                version: "1.0.1".to_string(),
                path: "~/Deadlock Mods/mods/wardrobe-accents".to_string(),
                slot: Some(4),
                unchanged: 2,
                changed: 0,
                missing: 0,
                moved: 0,
                unbaselined: 0,
                migrations: Vec::new(),
                warnings: Vec::new(),
                error: None,
            },
            ReviewItem {
                slug: "ember-trail-fx".to_string(),
                name: "Ember Trail FX".to_string(),
                version: "0.1.3".to_string(),
                path: "~/Deadlock Mods/mods/ember-trail-fx".to_string(),
                slot: Some(21),
                unchanged: 0,
                changed: 0,
                missing: 0,
                moved: 0,
                unbaselined: 1,
                migrations: Vec::new(),
                warnings: Vec::new(),
                error: None,
            },
        ],
        error: None,
    };
    ReleaseState {
        info: Some(info),
        plan: Some(plan),
        plan_error: None,
        publish: Some(publish),
        upload: Some(upload),
        upload_error: None,
        lookup_error: None,
        review: Some(review),
    }
}

/// The awaiting handoff fixture with the browser submission already verified.
pub fn demo_release_verified_state() -> ReleaseState {
    let mut state = demo_release_state();
    if let Some(upload) = state.upload.as_mut() {
        upload.state = "verified".to_string();
        upload.submission_url = Some("https://gamebanana.com/mods/512480".to_string());
        upload.verified = true;
        upload.confirmed_at = Some("2026-11-15T18:24:37.000Z".to_string());
    }
    state
}
