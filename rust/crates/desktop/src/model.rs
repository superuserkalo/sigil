//! Domain data shown by the workbench.
//!
//! Adapter seam: `deadlock-core` is under an active port (`crates/deadlock-core`). This crate
//! keeps the render model separate from the source so the UI compiles on its own while that port
//! lands. Every value has an explicit source: `demo_*` fixtures are labelled in the UI, and the
//! default state is empty rather than invented. Swapping these builders for core calls does not
//! change a screen.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Workbench,
    Projects,
    Index,
    Patches,
    Impact,
    Release,
    Settings,
}

impl Screen {
    pub fn key(self) -> &'static str {
        match self {
            Screen::Workbench => "workbench",
            Screen::Projects => "projects",
            Screen::Index => "index",
            Screen::Patches => "patches",
            Screen::Impact => "impact",
            Screen::Release => "release",
            Screen::Settings => "settings",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Screen::Workbench => "Workbench",
            Screen::Projects => "Projects",
            Screen::Index => "Game Index",
            Screen::Patches => "Patches",
            Screen::Impact => "Impact",
            Screen::Release => "Release",
            Screen::Settings => "Settings",
        }
    }

    pub fn kicker(self) -> &'static str {
        match self {
            Screen::Workbench => "AGENT SESSION",
            Screen::Projects => "MOD PROJECTS",
            Screen::Index => "GAME PATHS",
            Screen::Patches => "PATCH FEED",
            Screen::Impact => "COMPATIBILITY",
            Screen::Release => "RELEASE AND REVIEW",
            Screen::Settings => "LOCAL CONFIGURATION",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Screen::Workbench => "Plan, stage and validate with the project in context",
            Screen::Projects => "One workspace per mod, with targets and history",
            Screen::Index => "Exact paths from the installed game, read only",
            Screen::Patches => "Steam news, with the targets each note touches",
            Screen::Impact => "What a patch does to the selected project",
            Screen::Release => "Latest build, deploy plan and the queue after an update",
            Screen::Settings => "Providers, paths and safety, nothing secret on screen",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Screen::Workbench => "◆",
            Screen::Projects => "▤",
            Screen::Index => "⌕",
            Screen::Patches => "❐",
            Screen::Impact => "⚑",
            Screen::Release => "◈",
            Screen::Settings => "⚙",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Screen::Workbench => "⌘1",
            Screen::Projects => "⌘2",
            Screen::Index => "⌘3",
            Screen::Patches => "⌘4",
            Screen::Impact => "⌘5",
            Screen::Release => "⌘6",
            Screen::Settings => "⌘7",
        }
    }

    pub fn from_command(c: char) -> Option<Screen> {
        match c {
            '1' => Some(Screen::Workbench),
            '2' => Some(Screen::Projects),
            '3' => Some(Screen::Index),
            '4' => Some(Screen::Patches),
            '5' => Some(Screen::Impact),
            '6' => Some(Screen::Release),
            '7' => Some(Screen::Settings),
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
    pub fn slot_file(&self) -> String {
        format!("pak{:02}_dir.vpk", self.slot)
    }

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
    pub metadata_path: String,
    pub dir_present: bool,
    pub metadata_present: bool,
    pub created_at: Option<String>,
    pub files: Vec<String>,
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
        "Workbench is ready. Open a project to give the agent context, or use Game Index to find exact game paths.\n\nPrompts run through the local dah-agent host. Each tool call is listed in the timeline; nothing writes to the game install.",
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

pub fn demo_release_state() -> ReleaseState {
    let release_dir = "~/Deadlock Mods/ashfall-recolor/builds/0.4.2".to_string();
    let publish_dir = "~/Deadlock Mods/ashfall-recolor/publish/0.4.2".to_string();
    let addons = "~/Library/Application Support/Steam/steamapps/common/Deadlock/game/citadel/addons";
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
        metadata_path: format!("{publish_dir}/metadata.json"),
        dir_present: true,
        metadata_present: true,
        created_at: Some("2026-11-15T18:09:44.000Z".to_string()),
        files: vec!["pak12_dir.vpk".to_string()],
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
                warnings: vec![
                    "slot pak07_dir.vpk already exists in the addon folder".to_string(),
                ],
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
        lookup_error: None,
        review: Some(review),
    }
}
