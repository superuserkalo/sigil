//! Character generation as a stage graph.
//!
//! The pipeline is our own. It is not a wrapper around one vendor call: it is an
//! ordered graph of eleven stages, where the deterministic stages are native
//! Rust and the generative stages sit behind the [`MeshBackend`] trait. Meshy is
//! one adapter for two of those stages, kept for compatibility, and it is not
//! the default. See `docs/research/meshy-reverse-engineering.md` for the
//! observed process this graph is modelled on and for the evidence id each
//! stage carries.
//!
//! Deterministic, always available, no model and no key:
//!
//! - `prepare_reference`: format, dimensions, alpha channel, matte coverage,
//!   subject centring and edge quality.
//! - `cleanup`, `uv_bake`, `pbr_synthesize`, `retopo`, `rig`: the measurement
//!   and budget half of the stage (triangle count, UV presence, PBR presence,
//!   LOD budget, triangle limits). The geometry half needs a backend.
//! - `source2_map`: the artifact plan: `vmdl`, `vmat`, `vtex`, compiled `_c`
//!   paths, skeleton and weight requirements, AnimGraph 2 binding.
//! - `export`: place and hash the mesh.
//!
//! A generative stage with no configured backend is recorded as `unsupported`
//! with a precise reason. It is never silently skipped and never reported as
//! success.
//!
//! Anti-claim rule: this module has no path that sets `inGameReady` to true,
//! `source2Compile` to anything but `"required"`, or `publishAllowed` to true.
//! A generated mesh is a starting point, not a hero model.
//!
//! Reference images are inputs, not assets. A reference image is only sent to a
//! remote provider when the request carries explicit consent. See
//! `docs/decisions/0005-reference-images-and-likeness.md`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};
use crate::vpk::crc32;

/// Schema version for `task.json` and `character.json`. Version 1 records are
/// still readable and get an empty stage graph.
pub const CHARACTER_SCHEMA: u32 = 2;
/// Oldest record this build can read.
pub const MIN_READABLE_SCHEMA: u32 = 1;

/// Meshy API root. `MESHY_API_BASE_URL` overrides it for a local test server.
pub const MESHY_API_BASE: &str = "https://api.meshy.ai";
/// Environment variable holding the bearer token.
pub const MESHY_API_KEY_ENV: &str = "MESHY_API_KEY";
/// Environment variable overriding the API root, for a local scripted server.
pub const MESHY_BASE_URL_ENV: &str = "MESHY_API_BASE_URL";
/// Identifies this client to the provider without naming a user.
pub const MESHY_USER_AGENT: &str = "dah/0.0.0";
/// Meshy caps a prompt or texture prompt at 800 characters.
pub const MESHY_PROMPT_LIMIT: usize = 800;
/// Meshy accepts at most 380,000 triangles for automatic rigging.
pub const MESHY_RIG_TRIANGLE_LIMIT: u64 = 380_000;

/// Local cap for a reference image.
pub const MAX_REFERENCE_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Local cap for one downloaded or generated model file.
pub const MAX_MODEL_BYTES: u64 = 512 * 1024 * 1024;
/// Meshy documents a floor of 32 pixels per side.
pub const MIN_REFERENCE_SIDE: u32 = 32;
/// Meshy documents a ceiling of 178,956,970 pixels.
pub const MAX_REFERENCE_PIXELS: u64 = 178_956_970;

pub const TASK_FILE_NAME: &str = "task.json";
pub const MODEL_FILE_NAME: &str = "model.glb";
pub const CHARACTER_FILE_NAME: &str = "character.json";
pub const SOURCE2_PLAN_FILE_NAME: &str = "source2-plan.json";
pub const STAGES_DIR_NAME: &str = "stages";
pub const EXPORT_DIR_NAME: &str = "export";

/// The generated mesh still has to be compiled for Source 2.
pub const SOURCE2_COMPILE_REQUIRED: &str = "required";

/// Endpoint family recorded on a provider task.
pub const STAGE_IMAGE_TO_3D: &str = "image-to-3d";
pub const STAGE_TEXT_TO_3D: &str = "text-to-3d";
/// Our own stage graph, for tasks that are not a single vendor call.
pub const STAGE_PIPELINE: &str = "stage-graph";

/// Recorded when the caller does not supply a rights basis.
pub const DEFAULT_RIGHTS_NOTE: &str = "Reference image and likeness rights are not confirmed. \
Generated assets stay local and are not cleared for publish.";

/// Default LOD0 triangle budget for a hero-sized character. Source 2 hero
/// models are far below this; the number is a ceiling, not a target.
pub const DEFAULT_TRIANGLE_BUDGET: u64 = 60_000;

/// LOD ladder used by the budget check. LOD0 is the full mesh.
pub const LOD_BUDGET_LADDER: [(u8, u64); 4] = [(0, 60_000), (1, 30_000), (2, 12_000), (3, 5_000)];

/// Local command backend: an executable that runs one stage.
pub const MESHGEN_COMMAND_ENV: &str = "DAH_MESHGEN_COMMAND";
/// JSON array of arguments for the command. No shell, no string splitting.
pub const MESHGEN_ARGS_ENV: &str = "DAH_MESHGEN_ARGS";
/// HTTP backend endpoint. `POST` with a JSON stage request.
pub const MESHGEN_URL_ENV: &str = "DAH_MESHGEN_URL";
/// Optional bearer token for the HTTP backend. Never logged.
pub const MESHGEN_TOKEN_ENV: &str = "DAH_MESHGEN_TOKEN";
/// Stage ids the configured backend claims to implement, comma separated.
pub const MESHGEN_STAGES_ENV: &str = "DAH_MESHGEN_STAGES";
/// Per-stage timeout for a backend call, in seconds.
pub const MESHGEN_TIMEOUT_ENV: &str = "DAH_MESHGEN_TIMEOUT_SECS";

/// Stages a backend implements when the operator does not declare a list.
pub const DEFAULT_BACKEND_STAGES: [CharacterStage; 2] =
    [CharacterStage::GenerateViews, CharacterStage::Reconstruct];

const CREATE_TIMEOUT_SECONDS: u64 = 60;
const POLL_TIMEOUT_SECONDS: u64 = 30;
const DOWNLOAD_TIMEOUT_SECONDS: u64 = 300;
const DEFAULT_STAGE_TIMEOUT_SECONDS: u64 = 900;
const BODY_SNIPPET_CHARS: usize = 240;
const MAX_SLUG_LENGTH: usize = 48;
/// Above this many pixels the reference pixel analysis is skipped, not faked.
const MAX_ANALYSED_PIXELS: u64 = 16_000_000;
const ALPHA_THRESHOLD: u8 = 128;
const SOFT_EDGE_MIN: u8 = 8;
const SOFT_EDGE_MAX: u8 = 247;

// ---------------------------------------------------------------------------
// Stages
// ---------------------------------------------------------------------------

/// One stage of the character pipeline. The serialized name is written into
/// `task.json`, so it is a stable identifier.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CharacterStage {
    PrepareReference,
    GenerateViews,
    Reconstruct,
    Cleanup,
    UvBake,
    PbrSynthesize,
    Retopo,
    Rig,
    AnimateRetarget,
    Source2Map,
    Export,
}

impl CharacterStage {
    /// Every stage, in pipeline order.
    pub const ALL: [CharacterStage; 11] = [
        CharacterStage::PrepareReference,
        CharacterStage::GenerateViews,
        CharacterStage::Reconstruct,
        CharacterStage::Cleanup,
        CharacterStage::UvBake,
        CharacterStage::PbrSynthesize,
        CharacterStage::Retopo,
        CharacterStage::Rig,
        CharacterStage::AnimateRetarget,
        CharacterStage::Source2Map,
        CharacterStage::Export,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::PrepareReference => "prepare_reference",
            Self::GenerateViews => "generate_views",
            Self::Reconstruct => "reconstruct",
            Self::Cleanup => "cleanup",
            Self::UvBake => "uv_bake",
            Self::PbrSynthesize => "pbr_synthesize",
            Self::Retopo => "retopo",
            Self::Rig => "rig",
            Self::AnimateRetarget => "animate_retarget",
            Self::Source2Map => "source2_map",
            Self::Export => "export",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::PrepareReference => "prepare reference",
            Self::GenerateViews => "generate views",
            Self::Reconstruct => "reconstruct mesh",
            Self::Cleanup => "cleanup and decimate",
            Self::UvBake => "UV and bake",
            Self::PbrSynthesize => "synthesize PBR maps",
            Self::Retopo => "retopology",
            Self::Rig => "rig",
            Self::AnimateRetarget => "animate and retarget",
            Self::Source2Map => "Source 2 artifact plan",
            Self::Export => "export",
        }
    }

    /// True when the stage needs a model or a generative backend to change
    /// geometry or images.
    pub fn is_generative(self) -> bool {
        matches!(
            self,
            Self::GenerateViews
                | Self::Reconstruct
                | Self::UvBake
                | Self::PbrSynthesize
                | Self::Retopo
                | Self::Rig
                | Self::AnimateRetarget
        )
    }

    /// True when the stage can spend money at a provider.
    pub fn is_paid(self) -> bool {
        matches!(
            self,
            Self::GenerateViews
                | Self::Reconstruct
                | Self::UvBake
                | Self::PbrSynthesize
                | Self::Retopo
                | Self::Rig
                | Self::AnimateRetarget
        )
    }

    /// True when the stage is normally an asynchronous task with a task id.
    pub fn is_async(self) -> bool {
        matches!(
            self,
            Self::GenerateViews
                | Self::Reconstruct
                | Self::UvBake
                | Self::PbrSynthesize
                | Self::Retopo
                | Self::Rig
                | Self::AnimateRetarget
        )
    }

    /// True when the stage discards information from an earlier stage.
    pub fn is_lossy(self) -> bool {
        matches!(
            self,
            Self::Cleanup | Self::UvBake | Self::PbrSynthesize | Self::Retopo | Self::Rig
        )
    }

    /// Evidence ids from `docs/research/meshy-reverse-engineering.md`.
    pub fn evidence(self) -> &'static [&'static str] {
        match self {
            Self::PrepareReference => &["M1", "E24", "E25", "E34"],
            Self::GenerateViews => &["M2", "E6", "E15"],
            Self::Reconstruct => &["M3"],
            Self::Cleanup => &["M4", "E38"],
            Self::UvBake => &["M5"],
            Self::PbrSynthesize => &["M6", "E36"],
            Self::Retopo => &["M7"],
            Self::Rig => &["M8", "E18", "E21"],
            Self::AnimateRetarget => &["M9", "E20"],
            Self::Source2Map => &["E41"],
            Self::Export => &["M10", "E16"],
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|stage| stage.id() == value)
    }
}

impl std::fmt::Display for CharacterStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}

/// Static description of a stage, for `dah character stages` without a slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSpec {
    pub stage: CharacterStage,
    pub id: &'static str,
    pub title: &'static str,
    pub generative: bool,
    pub paid: bool,
    pub asynchronous: bool,
    pub lossy: bool,
    pub evidence: &'static [&'static str],
}

/// The canonical stage graph in pipeline order.
pub fn stage_graph() -> Vec<StageSpec> {
    CharacterStage::ALL
        .into_iter()
        .map(|stage| StageSpec {
            stage,
            id: stage.id(),
            title: stage.title(),
            generative: stage.is_generative(),
            paid: stage.is_paid(),
            asynchronous: stage.is_async(),
            lossy: stage.is_lossy(),
            evidence: stage.evidence(),
        })
        .collect()
}

/// Recorded state of one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageState {
    /// Nothing to do: the input already satisfies the stage.
    Skipped,
    /// Waiting for work, usually a backend task.
    Pending,
    /// Work started.
    Running,
    /// The stage produced its outputs.
    Succeeded,
    /// The stage ran and failed.
    Failed,
    /// No configured backend can run this stage.
    Unsupported,
}

impl StageState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }

    /// True when no later pass will change this state without new input.
    pub fn is_settled(self) -> bool {
        matches!(
            self,
            Self::Skipped | Self::Succeeded | Self::Failed | Self::Unsupported
        )
    }
}

impl std::fmt::Display for StageState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One stage record. `bytes` and `hash` describe the primary output when there
/// is exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageRecord {
    pub id: CharacterStage,
    pub state: StageState,
    /// Backend that ran or refused the stage. `native` for Rust-only stages.
    pub backend: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Provider or backend task id when the stage is asynchronous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Credits the provider reported for this stage, when it reports any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<u32>,
    #[serde(default)]
    pub updated_at: String,
}

impl StageRecord {
    pub fn new(stage: CharacterStage, backend: impl Into<String>) -> Self {
        Self {
            id: stage,
            state: StageState::Pending,
            backend: backend.into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            bytes: None,
            hash: None,
            notes: Vec::new(),
            evidence: stage.evidence().iter().map(|id| (*id).to_string()).collect(),
            task_id: None,
            credits: None,
            updated_at: now_iso(),
        }
    }

}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// Which backend family runs the generative stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CharacterProviderKind {
    /// Use a local command or HTTP backend if one is configured, else nothing.
    Auto,
    /// User-configured executable, argument array, no shell.
    Local,
    /// User-configured HTTP JSON endpoint.
    Http,
    /// Meshy, kept as a compatibility adapter. Never the default.
    Meshy,
    /// Deterministic offline provider for tests and demos.
    Mock,
}

impl CharacterProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Local => "local",
            Self::Http => "http",
            Self::Meshy => "meshy",
            Self::Mock => "mock",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(Self::Auto),
            "local" | "command" | "cli" => Some(Self::Local),
            "http" | "server" => Some(Self::Http),
            "meshy" => Some(Self::Meshy),
            "mock" | "offline" => Some(Self::Mock),
            _ => None,
        }
    }
}

impl Default for CharacterProviderKind {
    fn default() -> Self {
        Self::Auto
    }
}

/// One character request. `style` guides the texture stage, and `target_hero`
/// is recorded for the Source 2 plan; it is never sent to a provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRequest {
    pub prompt: String,
    /// Local reference image. Goes to a remote provider only with consent.
    pub reference_image: Option<PathBuf>,
    /// Deadlock hero codename this character is intended for, for example
    /// `billy`. Recorded, never uploaded.
    pub target_hero: Option<String>,
    /// Visual style, used as the texture prompt.
    pub style: String,
    pub provider: CharacterProviderKind,
    /// Rights basis recorded with the task. See ADR 0005.
    pub rights_note: String,
    /// Explicit consent to upload the reference image to a remote provider.
    pub allow_reference_upload: bool,
    /// Explicit backend id. `none` disables generative stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Asked-for face count for the cleanup and retopo stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_polycount: Option<u32>,
    /// `triangle` or `quad`, when a backend supports both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<String>,
    /// `a-pose`, `t-pose`, or absent for no pose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose_mode: Option<String>,
    /// LOD0 triangle ceiling for the budget check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triangle_budget: Option<u64>,
}

impl CharacterRequest {
    pub fn new(prompt: impl Into<String>, provider: CharacterProviderKind) -> Self {
        Self {
            prompt: prompt.into().trim().to_string(),
            reference_image: None,
            target_hero: None,
            style: "stylized game character".to_string(),
            provider,
            rights_note: DEFAULT_RIGHTS_NOTE.to_string(),
            allow_reference_upload: false,
            backend: None,
            target_polycount: None,
            topology: None,
            pose_mode: None,
            triangle_budget: None,
        }
    }

    pub fn triangle_budget(&self) -> u64 {
        self.triangle_budget.unwrap_or(DEFAULT_TRIANGLE_BUDGET)
    }

    /// Prompt-only or reference-driven. One of the two is required.
    pub fn has_input(&self) -> bool {
        !self.prompt.trim().is_empty() || self.reference_image.is_some()
    }
}

// ---------------------------------------------------------------------------
// Provider task
// ---------------------------------------------------------------------------

/// Provider task state. The wire values match the Meshy status strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CharacterStatus {
    Pending,
    InProgress,
    Succeeded,
    Failed,
    Canceled,
    /// The pipeline stopped because a required stage has no backend. Final
    /// until the operator configures one.
    Blocked,
}

impl CharacterStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::InProgress => "IN_PROGRESS",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Canceled => "CANCELED",
            Self::Blocked => "BLOCKED",
        }
    }

    pub fn is_final(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Canceled | Self::Blocked
        )
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "PENDING" => Some(Self::Pending),
            "IN_PROGRESS" => Some(Self::InProgress),
            "SUCCEEDED" => Some(Self::Succeeded),
            "FAILED" => Some(Self::Failed),
            "CANCELED" | "CANCELLED" => Some(Self::Canceled),
            "BLOCKED" => Some(Self::Blocked),
            _ => None,
        }
    }
}

impl std::fmt::Display for CharacterStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Provider-neutral task state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterTask {
    pub id: String,
    pub status: CharacterStatus,
    pub progress: u32,
    /// Download URLs by format, for example `glb`. A format the provider did
    /// not generate is absent.
    #[serde(default)]
    pub model_urls: BTreeMap<String, String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Provider hint for the next poll, in seconds.
    #[serde(default)]
    pub retry_after_seconds: Option<u64>,
    /// Endpoint family this task belongs to. One of the `STAGE_*` constants.
    #[serde(default)]
    pub stage: String,
    /// Credits the provider reported for the whole task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_credits: Option<u32>,
}

impl CharacterTask {
    pub fn new(id: impl Into<String>, stage: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CharacterStatus::Pending,
            progress: 0,
            model_urls: BTreeMap::new(),
            thumbnail_url: None,
            error: None,
            retry_after_seconds: None,
            stage: stage.into(),
            consumed_credits: None,
        }
    }

    pub fn glb_url(&self) -> Option<&str> {
        self.model_urls
            .get("glb")
            .map(String::as_str)
            .filter(|url| !url.is_empty())
    }
}

/// One provider backend. Implementations must not invent endpoints.
pub trait CharacterProvider {
    /// Stable provider id, matching `CharacterProviderKind`.
    fn name(&self) -> &'static str;

    /// Create a task and return its initial state.
    fn create_task(&self, request: &CharacterRequest) -> Result<CharacterTask>;

    /// Read the current state of a task created by `create_task`.
    fn poll_task(&self, task: &CharacterTask) -> Result<CharacterTask>;

    /// Download one model file to `destination`. Returns the byte count.
    fn download_asset(&self, url: &str, destination: &Path) -> Result<u64>;
}

/// Which backend family ran a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshBackendKind {
    None,
    LocalCommand,
    Http,
    Meshy,
    Mock,
}

impl MeshBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LocalCommand => "local-command",
            Self::Http => "http",
            Self::Meshy => "meshy",
            Self::Mock => "mock",
        }
    }
}

/// One produced file, relative to the character directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageOutput {
    pub path: String,
    pub bytes: u64,
    pub hash: String,
}

/// What a backend did with one stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageOutcome {
    Succeeded {
        outputs: Vec<StageOutput>,
        notes: Vec<String>,
        credits: Option<u32>,
        /// Final provider task state, when the stage was an async task.
        task: Option<CharacterTask>,
    },
    /// An asynchronous task is in flight.
    Pending {
        task: Option<CharacterTask>,
        notes: Vec<String>,
    },
    Skipped {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

impl StageOutcome {
    fn succeeded(outputs: Vec<StageOutput>, notes: Vec<String>) -> Self {
        Self::Succeeded {
            outputs,
            notes,
            credits: None,
            task: None,
        }
    }
}

/// Everything a backend may read while running one stage.
pub struct StageContext<'a> {
    pub stage: CharacterStage,
    /// Snapshot of the record as it stood before this stage.
    pub record: &'a CharacterTaskRecord,
    /// Existing record for this stage, including a task id from an earlier pass.
    pub stage_record: &'a StageRecord,
    pub character_dir: &'a Path,
    /// Scratch directory owned by this stage.
    pub stage_dir: &'a Path,
    /// Current mesh, when a previous stage produced one.
    pub mesh: Option<&'a Path>,
}

/// The generative seam. Deterministic stages never go through this trait.
pub trait MeshBackend {
    /// Stable id recorded in `task.json`, for example `local-command:/opt/trellis`.
    fn id(&self) -> String;
    fn kind(&self) -> MeshBackendKind;
    /// Whether this backend claims to implement the stage.
    fn supports(&self, stage: CharacterStage) -> bool;
    fn run(&self, context: &StageContext<'_>) -> Result<StageOutcome>;
}

// ---------------------------------------------------------------------------
// Rights, asset, records
// ---------------------------------------------------------------------------

/// Rights state recorded next to every task and import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRights {
    pub note: String,
    /// Local path of the reference image, if one was supplied.
    pub reference_image: Option<String>,
    /// True only when the image was sent to a remote provider.
    pub reference_uploaded: bool,
    /// Always false: this pipeline has no publish path.
    pub publish_allowed: bool,
}

impl CharacterRights {
    fn for_request(request: &CharacterRequest, reference_uploaded: bool) -> Self {
        Self {
            note: request.rights_note.trim().to_string(),
            reference_image: request
                .reference_image
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            reference_uploaded,
            publish_allowed: false,
        }
    }
}

/// Downloaded or generated model metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAsset {
    pub path: String,
    pub bytes: u64,
    /// CRC32 of the stored bytes, using the same helper as VPK verification.
    pub crc32: String,
    pub format: String,
    pub glb_version: u32,
}

/// Result of the reference-image checks. All fields are measured, not guessed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceReport {
    pub path: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    /// True when the file carries an alpha channel, not when it is transparent.
    pub has_alpha_channel: bool,
    /// Fraction of pixels at or above the alpha threshold. `None` when the
    /// pixel data was not decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_coverage: Option<f64>,
    /// Tight alpha bounding box as `[x, y, width, height]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_box: Option<[u32; 4]>,
    /// True when the subject box centre is within 8 percent of the image centre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centered: Option<bool>,
    /// Share of alpha edge pixels that are soft (8 to 247) rather than a hard
    /// threshold. A clean matte has a soft, antialiased edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_quality: Option<f64>,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl ReferenceReport {
    pub fn is_transparent(&self) -> bool {
        self.alpha_coverage.unwrap_or(0.0) > 0.02
    }
}

/// Structural facts about a GLB. Produced by parsing the file, never by
/// trusting the producer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshReport {
    pub format: String,
    pub bytes: u64,
    pub glb_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    pub scenes: u32,
    pub nodes: u32,
    pub meshes: u32,
    pub primitives: u32,
    pub triangles: u64,
    pub vertices: u64,
    pub materials: u32,
    pub textures: u32,
    pub has_normals: bool,
    pub has_uvs: bool,
    pub has_tangents: bool,
    pub has_skin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_min: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_max: Option<[f64; 3]>,
    pub accessors_checked: u32,
    /// Non-fatal findings. A mesh can be valid glTF and still be useless here.
    #[serde(default)]
    pub problems: Vec<String>,
}

impl MeshReport {
    /// Bounding box size in mesh units, when the accessors carry min and max.
    pub fn bounds_size(&self) -> Option<[f64; 3]> {
        let min = self.bounds_min?;
        let max = self.bounds_max?;
        Some([max[0] - min[0], max[1] - min[1], max[2] - min[2]])
    }

    /// Tallest axis, used as the height proxy.
    pub fn height(&self) -> Option<f64> {
        self.bounds_size()
            .map(|size| size[0].max(size[1]).max(size[2]))
    }

    pub fn over_budget(&self, budget: u64) -> bool {
        self.triangles > budget
    }

    /// LODs the mesh would fit inside the ladder at, LOD0 first.
    pub fn fitting_lods(&self) -> Vec<u32> {
        LOD_BUDGET_LADDER
            .into_iter()
            .filter(|(_, budget)| self.triangles <= *budget)
            .map(|(lod, _)| lod as u32)
            .collect()
    }
}

/// One required Source 2 artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Source2Artifact {
    /// `vmdl`, `vmat`, `vtex`, `vmdl_c`, `vmat_c`, `vtex_c`, `vnmskel_c`,
    /// `animgraph`, `mesh`.
    pub kind: String,
    /// Source path written by a human or the material tool.
    pub source: String,
    /// Compiled path the game reads.
    pub compiled: String,
    /// False when the artifact comes from the game and must not be shipped.
    pub shipped_by_mod: bool,
    pub note: String,
}

/// Skeleton and weight requirements for the plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Source2SkeletonRequirements {
    pub skeleton: String,
    pub ship_skeleton: bool,
    pub weights: String,
    pub note: String,
}

/// AnimGraph 2 binding requirements for the plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Source2AnimGraphRequirements {
    pub graph: String,
    pub bindings: Vec<String>,
    pub note: String,
}

/// The `source2_map` output. This is a plan, never a claim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Source2Plan {
    pub generated_at: String,
    pub hero: Option<String>,
    /// Always false. No generated mesh is in-game ready.
    pub in_game_ready: bool,
    /// Always `"required"`.
    pub source2_compile: String,
    /// Always false.
    pub publish_allowed: bool,
    pub artifacts: Vec<Source2Artifact>,
    pub skeleton: Source2SkeletonRequirements,
    pub anim_graph: Source2AnimGraphRequirements,
    pub requirements: Vec<String>,
    pub notes: Vec<String>,
}

/// `<workspace>/generated/characters/<slug>/task.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterTaskRecord {
    pub schema: u32,
    pub slug: String,
    pub created_at: String,
    pub updated_at: String,
    pub request: CharacterRequest,
    pub task: CharacterTask,
    pub rights: CharacterRights,
    /// Backend id that ran the generative stages, or `none`.
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub stages: Vec<StageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<CharacterAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshReport>,
}

impl CharacterTaskRecord {
    /// Stage records in pipeline order, filling any gap with a pending record.
    pub fn stages(&self) -> Vec<StageRecord> {
        let backend = if self.backend.is_empty() {
            "none".to_string()
        } else {
            self.backend.clone()
        };
        CharacterStage::ALL
            .into_iter()
            .map(|stage| {
                self.stages
                    .iter()
                    .find(|record| record.id == stage)
                    .cloned()
                    .unwrap_or_else(|| StageRecord::new(stage, backend.clone()))
            })
            .collect()
    }

    pub fn stage(&self, stage: CharacterStage) -> Option<&StageRecord> {
        self.stages.iter().find(|record| record.id == stage)
    }

    /// Stages that need a backend and have none.
    pub fn unsupported_stages(&self) -> Vec<CharacterStage> {
        self.stages()
            .into_iter()
            .filter(|record| record.state == StageState::Unsupported)
            .map(|record| record.id)
            .collect()
    }
}

/// `<workspace>/generated/characters/<slug>/character.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterImport {
    pub schema: u32,
    pub slug: String,
    pub created_at: String,
    pub imported_at: String,
    pub prompt: String,
    pub style: String,
    pub provider: String,
    pub backend: String,
    pub target_hero: Option<String>,
    pub task_id: String,
    pub asset: CharacterAsset,
    pub rights: CharacterRights,
    /// `required`. Never `done` from this pipeline.
    pub source2_compile: String,
    /// Always false until a mesh is compiled and verified in game.
    pub in_game_ready: bool,
    #[serde(default)]
    pub stages: Vec<StageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source2_plan: Option<Source2Plan>,
    pub compile_requirements: Vec<String>,
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// `<workspace>/generated/characters`.
pub fn characters_dir(workspace: &Path) -> PathBuf {
    workspace.join("generated").join("characters")
}

/// `<workspace>/generated/characters/<slug>`.
pub fn character_dir(workspace: &Path, slug: &str) -> PathBuf {
    characters_dir(workspace).join(slug)
}

pub fn task_path(workspace: &Path, slug: &str) -> PathBuf {
    character_dir(workspace, slug).join(TASK_FILE_NAME)
}

pub fn model_path(workspace: &Path, slug: &str) -> PathBuf {
    character_dir(workspace, slug).join(MODEL_FILE_NAME)
}

pub fn character_manifest_path(workspace: &Path, slug: &str) -> PathBuf {
    character_dir(workspace, slug).join(CHARACTER_FILE_NAME)
}

pub fn source2_plan_path(workspace: &Path, slug: &str) -> PathBuf {
    character_dir(workspace, slug).join(SOURCE2_PLAN_FILE_NAME)
}

pub fn export_path(workspace: &Path, slug: &str) -> PathBuf {
    character_dir(workspace, slug)
        .join(EXPORT_DIR_NAME)
        .join(format!("{slug}.glb"))
}

fn stage_dir(workspace: &Path, slug: &str, stage: CharacterStage) -> PathBuf {
    character_dir(workspace, slug)
        .join(STAGES_DIR_NAME)
        .join(stage.id())
}

// ---------------------------------------------------------------------------
// Meshy provider
// ---------------------------------------------------------------------------

/// Meshy image-to-3D and text-to-3D client.
///
/// Image path: `POST /openapi/v1/image-to-3d`, `GET /openapi/v1/image-to-3d/:id`.
/// Text path: `POST /openapi/v2/text-to-3d` with `mode: "preview"`,
/// `GET /openapi/v2/text-to-3d/:id`.
#[derive(Clone)]
pub struct MeshyProvider {
    api_key: String,
    base_url: String,
}

impl std::fmt::Debug for MeshyProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MeshyProvider")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl MeshyProvider {
    /// Read `MESHY_API_KEY` and the optional `MESHY_API_BASE_URL` override.
    pub fn from_env() -> Result<Self> {
        let api_key = env::var(MESHY_API_KEY_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::msg(format!(
                    "Meshy needs an API key. Set {MESHY_API_KEY_ENV}, or configure a local backend \
                     with {MESHGEN_COMMAND_ENV} or {MESHGEN_URL_ENV}. Meshy is not the default."
                ))
            })?;

        let base_url = env::var(MESHY_BASE_URL_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| MESHY_API_BASE.to_string());

        Self::with_base_url(api_key, base_url)
    }

    /// Build a client for a specific key and API root. The root override is
    /// what lets a test point the real code at a local server.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into().trim().to_string();
        if api_key.is_empty() {
            return Err(Error::msg("Meshy API key must not be empty."));
        }

        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
            return Err(Error::msg(format!(
                "Meshy base URL must start with https:// or http://, got {base_url:?}."
            )));
        }

        Ok(Self { api_key, base_url })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn post_json(&self, path: &str, body: &Value, action: &str) -> Result<(Value, Option<u64>)> {
        let url = format!("{}{}", self.base_url, path);
        let agent = http_agent(CREATE_TIMEOUT_SECONDS);
        let request = agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .header("User-Agent", MESHY_USER_AGENT)
            .send_json(body);

        finish_json(action, request)
    }

    fn get_json(&self, path: &str, action: &str) -> Result<(Value, Option<u64>)> {
        let url = format!("{}{}", self.base_url, path);
        let agent = http_agent(POLL_TIMEOUT_SECONDS);
        let request = agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .header("User-Agent", MESHY_USER_AGENT)
            .call();

        finish_json(action, request)
    }

    fn reference_data_uri(&self, request: &CharacterRequest) -> Result<String> {
        let path = request
            .reference_image
            .as_ref()
            .ok_or_else(|| Error::msg("The image-to-3D path needs a reference image."))?;

        if !request.allow_reference_upload {
            return Err(Error::msg(format!(
                "Refusing to upload {} to Meshy without consent. Pass --allow-reference-upload \
                 after checking the rights to the image. Reference images never leave the machine \
                 unless this is set.",
                path.display()
            )));
        }

        let metadata = fs::metadata(path).map_err(|error| {
            Error::msg(format!(
                "Reference image {} could not be read: {error}",
                path.display()
            ))
        })?;

        if !metadata.is_file() {
            return Err(Error::msg(format!(
                "Reference image {} is not a file.",
                path.display()
            )));
        }

        if metadata.len() > MAX_REFERENCE_IMAGE_BYTES {
            return Err(Error::msg(format!(
                "Reference image {} is {} bytes. Meshy accepts at most {} bytes.",
                path.display(),
                metadata.len(),
                MAX_REFERENCE_IMAGE_BYTES
            )));
        }

        if metadata.len() == 0 {
            return Err(Error::msg(format!(
                "Reference image {} is empty.",
                path.display()
            )));
        }

        let mime = reference_mime(path)?;
        let bytes = fs::read(path)?;

        Ok(format!("data:{mime};base64,{}", base64_encode(&bytes)))
    }

    fn texture_prompt(request: &CharacterRequest) -> String {
        let prompt = request.prompt.trim();
        let style = request.style.trim();

        match (prompt.is_empty(), style.is_empty()) {
            (true, true) => String::new(),
            (false, true) => prompt.to_string(),
            (true, false) => style.to_string(),
            (false, false) => format!("{prompt}. {style}"),
        }
    }
}

impl CharacterProvider for MeshyProvider {
    fn name(&self) -> &'static str {
        "meshy"
    }

    fn create_task(&self, request: &CharacterRequest) -> Result<CharacterTask> {
        let prompt = request.prompt.trim();
        let has_image = request.reference_image.is_some();

        if prompt.is_empty() && !has_image {
            return Err(Error::msg(
                "Character request needs a prompt or a reference image.",
            ));
        }

        if prompt.chars().count() > MESHY_PROMPT_LIMIT {
            return Err(Error::msg(format!(
                "Character prompt is {} characters. Meshy accepts at most {MESHY_PROMPT_LIMIT}.",
                prompt.chars().count()
            )));
        }

        if request.style.chars().count() > MESHY_PROMPT_LIMIT {
            return Err(Error::msg(format!(
                "Style text is {} characters. Meshy accepts at most {MESHY_PROMPT_LIMIT}.",
                request.style.chars().count()
            )));
        }

        let (path, body, stage) = match request.reference_image {
            Some(_) => {
                let image_url = self.reference_data_uri(request)?;
                let texture_prompt = Self::texture_prompt(request);
                let mut body = json!({
                    "image_url": image_url,
                    "should_texture": true,
                    "enable_pbr": true,
                    "pose_mode": request.pose_mode.clone().unwrap_or_else(|| "a-pose".to_string()),
                    "target_formats": ["glb"],
                });
                if let Some(polycount) = request.target_polycount {
                    body["should_remesh"] = Value::Bool(true);
                    body["topology"] = Value::String(
                        request
                            .topology
                            .clone()
                            .unwrap_or_else(|| "triangle".to_string()),
                    );
                    body["target_polycount"] = Value::from(polycount);
                }
                if !texture_prompt.is_empty() {
                    if texture_prompt.chars().count() > MESHY_PROMPT_LIMIT {
                        return Err(Error::msg(format!(
                            "Combined prompt and style is {} characters. Meshy accepts at most \
                             {MESHY_PROMPT_LIMIT}.",
                            texture_prompt.chars().count()
                        )));
                    }
                    body["texture_prompt"] = Value::String(texture_prompt);
                }
                ("/openapi/v1/image-to-3d", body, STAGE_IMAGE_TO_3D)
            }
            None => {
                let body = json!({
                    "mode": "preview",
                    "prompt": prompt,
                    "pose_mode": request.pose_mode.clone().unwrap_or_else(|| "a-pose".to_string()),
                    "target_formats": ["glb"],
                });
                ("/openapi/v2/text-to-3d", body, STAGE_TEXT_TO_3D)
            }
        };

        let (value, retry_after) = self.post_json(path, &body, "Meshy create task")?;
        let id = value
            .get("result")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::msg(
                    "Meshy accepted the request but returned no task id in `result`. \
                     The task was not recorded.",
                )
            })?;

        let mut task = CharacterTask::new(id, stage);
        task.retry_after_seconds = retry_after;
        Ok(task)
    }

    fn poll_task(&self, task: &CharacterTask) -> Result<CharacterTask> {
        validate_task_id(&task.id)?;

        let path = match task.stage.as_str() {
            STAGE_IMAGE_TO_3D => format!("/openapi/v1/image-to-3d/{}", task.id),
            STAGE_TEXT_TO_3D => format!("/openapi/v2/text-to-3d/{}", task.id),
            other => {
                return Err(Error::msg(format!(
                    "Task {} has stage {other:?}, which is not a Meshy stage. Recreate the task.",
                    task.id
                )));
            }
        };

        let (value, retry_after) = self.get_json(&path, "Meshy task poll")?;
        let mut polled = parse_meshy_task(&value, &task.stage)?;
        polled.retry_after_seconds = retry_after.or(polled.retry_after_seconds);
        Ok(polled)
    }

    fn download_asset(&self, url: &str, destination: &Path) -> Result<u64> {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(Error::msg(format!(
                "Meshy model URL must be http or https, got {url:?}."
            )));
        }

        let agent = http_agent(DOWNLOAD_TIMEOUT_SECONDS);
        let request = agent.get(url).header("User-Agent", MESHY_USER_AGENT).call();
        let bytes = finish_bytes("Meshy model download", request)?;
        validate_glb_bytes(&bytes)?;

        write_atomic(destination, &bytes)?;
        Ok(bytes.len() as u64)
    }
}

/// Parse the Meshy task object shared by the image and text endpoints.
fn parse_meshy_task(value: &Value, stage: &str) -> Result<CharacterTask> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::msg("Meshy task response has no id."))?;

    let status_text = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg(format!("Meshy task {id} has no status.")))?;
    let status = CharacterStatus::parse(status_text).ok_or_else(|| {
        Error::msg(format!(
            "Meshy task {id} returned unknown status {status_text:?}."
        ))
    })?;

    let progress = value
        .get("progress")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(100) as u32;

    let mut model_urls = BTreeMap::new();
    if let Some(object) = value.get("model_urls").and_then(Value::as_object) {
        for (format, url) in object {
            if let Some(url) = url.as_str()
                && !url.trim().is_empty()
            {
                model_urls.insert(format.clone(), url.to_string());
            }
        }
    }

    let thumbnail_url = value
        .get("thumbnail_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string);

    let error = value.get("task_error").and_then(format_task_error);
    let consumed_credits = value
        .get("consumed_credits")
        .and_then(Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32);

    Ok(CharacterTask {
        id: id.to_string(),
        status,
        progress,
        model_urls,
        thumbnail_url,
        error,
        retry_after_seconds: None,
        stage: stage.to_string(),
        consumed_credits,
    })
}

fn format_task_error(value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }

    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    let code = value.get("code").and_then(Value::as_str).unwrap_or("");
    let message = value.get("message").and_then(Value::as_str).unwrap_or("");

    let mut label = String::new();
    for part in [kind, code] {
        if !part.is_empty() {
            if !label.is_empty() {
                label.push('/');
            }
            label.push_str(part);
        }
    }

    let message = if message.is_empty() {
        "The provider failed the task without a message.".to_string()
    } else {
        message.to_string()
    };

    if label.is_empty() {
        Some(message)
    } else {
        Some(format!("[{label}] {message}"))
    }
}

// ---------------------------------------------------------------------------
// Mock provider
// ---------------------------------------------------------------------------

static MOCK_TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Local scripted provider for tests and offline demos. It never touches the
/// network. Each poll advances the state machine one step:
/// `PENDING` to `IN_PROGRESS` to `SUCCEEDED`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockProvider;

impl MockProvider {
    pub fn new() -> Self {
        Self
    }
}

impl CharacterProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn create_task(&self, request: &CharacterRequest) -> Result<CharacterTask> {
        if !request.has_input() {
            return Err(Error::msg(
                "Character request needs a prompt or a reference image.",
            ));
        }

        let stage = if request.reference_image.is_some() {
            STAGE_IMAGE_TO_3D
        } else {
            STAGE_TEXT_TO_3D
        };
        let sequence = MOCK_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("mock-{sequence:08}");

        Ok(CharacterTask::new(id, stage))
    }

    fn poll_task(&self, task: &CharacterTask) -> Result<CharacterTask> {
        if !task.id.starts_with("mock-") {
            return Err(Error::msg(format!(
                "Task {} was not created by the mock provider.",
                task.id
            )));
        }

        let mut next = task.clone();

        match task.status {
            CharacterStatus::Pending => {
                next.status = CharacterStatus::InProgress;
                next.progress = 50;
                next.retry_after_seconds = Some(1);
            }
            CharacterStatus::InProgress => {
                next.status = CharacterStatus::Succeeded;
                next.progress = 100;
                next.retry_after_seconds = None;
                next.model_urls
                    .insert("glb".to_string(), "mock://character/model.glb".to_string());
                next.thumbnail_url = Some("mock://character/thumbnail.png".to_string());
            }
            CharacterStatus::Succeeded
            | CharacterStatus::Failed
            | CharacterStatus::Canceled
            | CharacterStatus::Blocked => {}
        }

        Ok(next)
    }

    fn download_asset(&self, url: &str, destination: &Path) -> Result<u64> {
        if !url.starts_with("mock://") {
            return Err(Error::msg(format!(
                "The mock provider only serves mock:// URLs, got {url:?}."
            )));
        }

        let bytes = minimal_glb_bytes();
        write_atomic(destination, &bytes)?;
        Ok(bytes.len() as u64)
    }
}

// ---------------------------------------------------------------------------
// Provider-backed adapter
// ---------------------------------------------------------------------------

/// Borrows and boxes both work as a provider, so an injected provider never
/// has to be cloned.
impl<T: CharacterProvider + ?Sized> CharacterProvider for &T {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn create_task(&self, request: &CharacterRequest) -> Result<CharacterTask> {
        (**self).create_task(request)
    }

    fn poll_task(&self, task: &CharacterTask) -> Result<CharacterTask> {
        (**self).poll_task(task)
    }

    fn download_asset(&self, url: &str, destination: &Path) -> Result<u64> {
        (**self).download_asset(url, destination)
    }
}

/// Runs the mesh stages through a [`CharacterProvider`]. Meshy and the mock
/// provider both use this, so the rest of the graph does not change. One
/// provider task produces both `generate_views` and `reconstruct`.
pub struct ProviderBackend<P> {
    provider: P,
}

impl<P: CharacterProvider> ProviderBackend<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    pub fn provider_name(&self) -> &'static str {
        self.provider.name()
    }
}

impl<P: CharacterProvider> std::fmt::Debug for ProviderBackend<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderBackend")
            .field("provider", &self.provider.name())
            .finish()
    }
}

impl<P: CharacterProvider> MeshBackend for ProviderBackend<P> {
    fn id(&self) -> String {
        self.provider.name().to_string()
    }

    fn kind(&self) -> MeshBackendKind {
        match self.provider.name() {
            "meshy" => MeshBackendKind::Meshy,
            "mock" => MeshBackendKind::Mock,
            _ => MeshBackendKind::None,
        }
    }

    fn supports(&self, stage: CharacterStage) -> bool {
        matches!(
            stage,
            CharacterStage::GenerateViews | CharacterStage::Reconstruct
        )
    }

    fn run(&self, context: &StageContext<'_>) -> Result<StageOutcome> {
        if context.stage == CharacterStage::Reconstruct {
            // The provider returns one mesh for the whole task, so the mesh from
            // generate_views is also the reconstruct result.
            let mesh = context.character_dir.join(MODEL_FILE_NAME);
            if mesh.is_file() {
                let output = output_for(context.character_dir, &mesh)?;
                return Ok(StageOutcome::succeeded(
                    vec![output],
                    vec![
                        "The provider returns the mesh in the same task as the views, so this \
                         stage adds no work."
                            .to_string(),
                    ],
                ));
            }

            return Ok(StageOutcome::Skipped {
                reason: "the provider task has not produced a mesh yet.".to_string(),
            });
        }

        if context.stage != CharacterStage::GenerateViews {
            return Ok(StageOutcome::Unsupported {
                reason: format!(
                    "{} only runs the mesh task. Stage {} needs a mesh backend.",
                    self.provider.name(),
                    context.stage
                ),
            });
        }

        // A task already exists: poll it.
        if context.stage_record.task_id.is_some() {
            let polled = self.provider.poll_task(&context.record.task)?;

            return Ok(match polled.status {
                CharacterStatus::Succeeded => {
                    let url = polled.glb_url().ok_or_else(|| {
                        Error::msg(format!(
                            "Task {} succeeded without a GLB URL. The provider output cannot be \
                             imported.",
                            polled.id
                        ))
                    })?;
                    let destination = context.character_dir.join(MODEL_FILE_NAME);
                    let bytes = self.provider.download_asset(url, &destination)?;
                    let output = output_for(context.character_dir, &destination)?;
                    let notes = vec![format!(
                        "Provider task {} reached SUCCEEDED. The GLB was downloaded.",
                        polled.id
                    )];
                    let credits = polled.consumed_credits;
                    StageOutcome::Succeeded {
                        outputs: vec![StageOutput { bytes, ..output }],
                        notes,
                        credits,
                        task: Some(polled),
                    }
                }
                CharacterStatus::Failed | CharacterStatus::Canceled => StageOutcome::Failed {
                    reason: format!(
                        "Provider task {} ended {}: {}",
                        polled.id,
                        polled.status,
                        polled
                            .error
                            .clone()
                            .unwrap_or_else(|| "no error message".to_string())
                    ),
                },
                _ => StageOutcome::Pending {
                    notes: vec![format!(
                        "Provider task {} is {} at {} percent.",
                        polled.id, polled.status, polled.progress
                    )],
                    task: Some(polled),
                },
            });
        }

        let task = self.provider.create_task(&context.record.request)?;
        Ok(StageOutcome::Pending {
            notes: vec![format!(
                "Provider task {} created for {}. Re-run refresh to poll it.",
                task.id, task.stage
            )],
            task: Some(task),
        })
    }
}

// ---------------------------------------------------------------------------
// Local command backend
// ---------------------------------------------------------------------------

/// Runs a user-configured executable, one stage per call.
///
/// Contract:
/// - no shell: the program and its arguments are separate values,
/// - working directory is the character folder,
/// - `DAH_STAGE`, `DAH_CHARACTER_DIR`, `DAH_STAGE_DIR`, `DAH_REFERENCE`,
///   `DAH_PROMPT`, `DAH_STYLE`, `DAH_TARGET_HERO`, `DAH_POLYCOUNT`,
///   `DAH_TOPOLOGY`, `DAH_POSE` and `DAH_MESH_IN` are set,
/// - `reconstruct` must write `model.glb` in the character folder,
/// - every other stage may write files under the stage folder,
/// - a non-zero exit is a failed stage, a timeout kills the process.
///
/// This is how TRELLIS, Hunyuan3D, ComfyUI CLI or a Blender script plug in.
#[derive(Debug, Clone)]
pub struct LocalCommandBackend {
    program: PathBuf,
    args: Vec<String>,
    stages: Vec<CharacterStage>,
    timeout: Duration,
}

impl LocalCommandBackend {
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            stages: DEFAULT_BACKEND_STAGES.to_vec(),
            timeout: Duration::from_secs(DEFAULT_STAGE_TIMEOUT_SECONDS),
        }
    }

    pub fn with_stages(mut self, stages: Vec<CharacterStage>) -> Self {
        if !stages.is_empty() {
            self.stages = stages;
        }
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Read `DAH_MESHGEN_COMMAND`, `DAH_MESHGEN_ARGS`, `DAH_MESHGEN_STAGES`
    /// and `DAH_MESHGEN_TIMEOUT_SECS`.
    pub fn from_env() -> Result<Option<Self>> {
        let program = match env::var(MESHGEN_COMMAND_ENV) {
            Ok(value) => value.trim().to_string(),
            Err(_) => return Ok(None),
        };

        if program.is_empty() {
            return Ok(None);
        }

        let args = match env::var(MESHGEN_ARGS_ENV) {
            Ok(value) if !value.trim().is_empty() => {
                let parsed: Value = serde_json::from_str(&value).map_err(|error| {
                    Error::msg(format!(
                        "{MESHGEN_ARGS_ENV} must be a JSON array of strings: {error}"
                    ))
                })?;
                let array = parsed.as_array().ok_or_else(|| {
                    Error::msg(format!("{MESHGEN_ARGS_ENV} must be a JSON array of strings."))
                })?;
                let mut args = Vec::with_capacity(array.len());
                for item in array {
                    let value = item.as_str().ok_or_else(|| {
                        Error::msg(format!(
                            "{MESHGEN_ARGS_ENV} must contain only strings."
                        ))
                    })?;
                    args.push(value.to_string());
                }
                args
            }
            _ => Vec::new(),
        };

        let stages = match env::var(MESHGEN_STAGES_ENV) {
            Ok(value) if !value.trim().is_empty() => {
                let mut stages = Vec::new();
                for part in value.split(',') {
                    let stage = CharacterStage::parse(part).ok_or_else(|| {
                        Error::msg(format!(
                            "{MESHGEN_STAGES_ENV} names unknown stage {part:?}. Known stages: {}",
                            CharacterStage::ALL
                                .iter()
                                .map(|stage| stage.id())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    })?;
                    stages.push(stage);
                }
                stages
            }
            _ => DEFAULT_BACKEND_STAGES.to_vec(),
        };

        let timeout = env::var(MESHGEN_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_STAGE_TIMEOUT_SECONDS));

        Ok(Some(
            Self::new(PathBuf::from(program), args)
                .with_stages(stages)
                .with_timeout(timeout),
        ))
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn stages(&self) -> &[CharacterStage] {
        &self.stages
    }
}

impl MeshBackend for LocalCommandBackend {
    fn id(&self) -> String {
        format!(
            "local-command:{}",
            self.program.file_name().map_or_else(
                || self.program.to_string_lossy(),
                |value| value.to_string_lossy()
            )
        )
    }

    fn kind(&self) -> MeshBackendKind {
        MeshBackendKind::LocalCommand
    }

    fn supports(&self, stage: CharacterStage) -> bool {
        self.stages.contains(&stage)
    }

    fn run(&self, context: &StageContext<'_>) -> Result<StageOutcome> {
        if !self.supports(context.stage) {
            return Ok(StageOutcome::Unsupported {
                reason: format!(
                    "Backend {} does not claim stage {}. Set {MESHGEN_STAGES_ENV} to include it.",
                    self.id(),
                    context.stage
                ),
            });
        }

        if !self.program.is_file() {
            return Ok(StageOutcome::Failed {
                reason: format!(
                    "Backend program {} does not exist. Set {MESHGEN_COMMAND_ENV} to an absolute \
                     path.",
                    self.program.display()
                ),
            });
        }

        fs::create_dir_all(context.stage_dir)?;

        let stdout_path = context.stage_dir.join("stdout.log");
        let stderr_path = context.stage_dir.join("stderr.log");
        let stdout = File::create(&stdout_path)?;
        let stderr = File::create(&stderr_path)?;

        let request = &context.record.request;
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(context.character_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .env("DAH_STAGE", context.stage.id())
            .env("DAH_CHARACTER_DIR", context.character_dir)
            .env("DAH_STAGE_DIR", context.stage_dir)
            .env("DAH_PROMPT", &request.prompt)
            .env("DAH_STYLE", &request.style);

        if let Some(path) = &request.reference_image {
            let absolute = crate::util::absolute(path);
            command.env("DAH_REFERENCE", absolute);
        }
        if let Some(hero) = &request.target_hero {
            command.env("DAH_TARGET_HERO", hero);
        }
        if let Some(polycount) = request.target_polycount {
            command.env("DAH_POLYCOUNT", polycount.to_string());
        }
        if let Some(topology) = &request.topology {
            command.env("DAH_TOPOLOGY", topology);
        }
        if let Some(pose) = &request.pose_mode {
            command.env("DAH_POSE", pose);
        }
        if let Some(mesh) = context.mesh {
            command.env("DAH_MESH_IN", mesh);
        }

        let status = match run_with_timeout(&mut command, self.timeout) {
            Ok(status) => status,
            Err(error) => {
                return Ok(StageOutcome::Failed {
                    reason: error.to_string(),
                });
            }
        };

        let tail = read_tail(&stderr_path, 400);

        if !status.success() {
            let code = status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string());
            return Ok(StageOutcome::Failed {
                reason: format!(
                    "Backend {} exited {code} on stage {}. stderr: {}",
                    self.id(),
                    context.stage,
                    if tail.is_empty() {
                        "empty".to_string()
                    } else {
                        tail
                    }
                ),
            });
        }

        let produced = collect_stage_outputs(context.character_dir, context.stage_dir)?;
        let mut notes = Vec::new();
        if !tail.is_empty() {
            notes.push(format!("stderr: {tail}"));
        }

        if context.stage == CharacterStage::Reconstruct {
            let mesh = context.character_dir.join(MODEL_FILE_NAME);
            if !mesh.is_file() {
                return Ok(StageOutcome::Failed {
                    reason: format!(
                        "Backend {} exited 0 on reconstruct but wrote no {MODEL_FILE_NAME}.",
                        self.id()
                    ),
                });
            }
            let report = read_asset(&mesh, &context.record.slug)?;
            notes.push(format!(
                "Reconstructed mesh: {} bytes, glTF {}.",
                report.bytes, report.glb_version
            ));
        } else if produced.is_empty() {
            notes.push(format!(
                "Backend {} exited 0 and wrote no files for this stage.",
                self.id()
            ));
        }

        Ok(StageOutcome::Succeeded {
            outputs: produced,
            notes,
            credits: None,
            task: None,
        })
    }
}

fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<std::process::ExitStatus> {
    let mut child = command.spawn().map_err(|error| {
        Error::msg(format!(
            "Backend command could not start: {error}. The command is run without a shell."
        ))
    })?;

    let started = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status),
            None => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::msg(format!(
                        "Backend command timed out after {} seconds and was killed.",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn read_tail(path: &Path, limit: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.replace('\n', " ");
    }
    let tail: String = trimmed
        .chars()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{tail}").replace('\n', " ")
}

/// Every regular file under the stage folder, as paths relative to the
/// character folder, excluding the captured logs.
fn collect_stage_outputs(character_dir: &Path, directory: &Path) -> Result<Vec<StageOutput>> {
    let mut outputs = Vec::new();
    let mut stack = vec![directory.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };

        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if name == "stdout.log" || name == "stderr.log" {
                continue;
            }

            outputs.push(output_for(character_dir, &path)?);
        }
    }

    outputs.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(outputs)
}

fn output_for(root: &Path, path: &Path) -> Result<StageOutput> {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    if !canonical.starts_with(&canonical_root) {
        return Err(Error::msg(format!(
            "Backend wrote {} outside the character folder {}. The file was refused.",
            canonical.display(),
            canonical_root.display()
        )));
    }

    let relative = canonical
        .strip_prefix(&canonical_root)
        .unwrap_or(&canonical)
        .to_string_lossy()
        .replace('\\', "/");

    let bytes = fs::read(&canonical)?;

    Ok(StageOutput {
        path: relative,
        bytes: bytes.len() as u64,
        hash: format!("crc32:{:08x}", crc32(&bytes)),
    })
}

// ---------------------------------------------------------------------------
// HTTP backend
// ---------------------------------------------------------------------------

/// Posts one stage request to a local model server.
///
/// Request:
/// ```json
/// {"stage":"reconstruct","character":"my-slug","prompt":"...","style":"...",
///  "targetPolycount":40000,"topology":"triangle","pose":"a-pose",
///  "reference":"/abs/img.png","meshIn":"/abs/model.glb",
///  "characterDir":"/abs/dir","stageDir":"/abs/dir/stages/reconstruct"}
/// ```
///
/// Response, one of:
/// ```json
/// {"state":"succeeded","outputs":[{"path":"model.glb","dataBase64":"..."}],
///  "notes":["..."],"credits":12}
/// {"state":"succeeded","outputs":[{"path":"model.glb","sourcePath":"/abs/x.glb"}]}
/// {"state":"pending","taskId":"abc","notes":["queued"]}
/// {"state":"unsupported","reason":"no texture model loaded"}
/// {"state":"failed","reason":"CUDA out of memory"}
/// ```
#[derive(Debug, Clone)]
pub struct HttpBackend {
    url: String,
    token: Option<String>,
    stages: Vec<CharacterStage>,
    timeout: Duration,
}

impl HttpBackend {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: None,
            stages: DEFAULT_BACKEND_STAGES.to_vec(),
            timeout: Duration::from_secs(DEFAULT_STAGE_TIMEOUT_SECONDS),
        }
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn with_stages(mut self, stages: Vec<CharacterStage>) -> Self {
        if !stages.is_empty() {
            self.stages = stages;
        }
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Read `DAH_MESHGEN_URL` and the optional token and stage list.
    pub fn from_env() -> Result<Option<Self>> {
        let url = match env::var(MESHGEN_URL_ENV) {
            Ok(value) => value.trim().to_string(),
            Err(_) => return Ok(None),
        };

        if url.is_empty() {
            return Ok(None);
        }

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(Error::msg(format!(
                "{MESHGEN_URL_ENV} must start with http:// or https://, got {url:?}."
            )));
        }

        let token = env::var(MESHGEN_TOKEN_ENV).ok();
        let timeout = env::var(MESHGEN_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_STAGE_TIMEOUT_SECONDS));

        let stages = match env::var(MESHGEN_STAGES_ENV) {
            Ok(value) if !value.trim().is_empty() => {
                let mut stages = Vec::new();
                for part in value.split(',') {
                    let stage = CharacterStage::parse(part).ok_or_else(|| {
                        Error::msg(format!(
                            "{MESHGEN_STAGES_ENV} names unknown stage {part:?}."
                        ))
                    })?;
                    stages.push(stage);
                }
                stages
            }
            _ => DEFAULT_BACKEND_STAGES.to_vec(),
        };

        Ok(Some(
            Self::new(url)
                .with_token(token)
                .with_stages(stages)
                .with_timeout(timeout),
        ))
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl MeshBackend for HttpBackend {
    fn id(&self) -> String {
        format!("http:{}", self.url)
    }

    fn kind(&self) -> MeshBackendKind {
        MeshBackendKind::Http
    }

    fn supports(&self, stage: CharacterStage) -> bool {
        self.stages.contains(&stage)
    }

    fn run(&self, context: &StageContext<'_>) -> Result<StageOutcome> {
        if !self.supports(context.stage) {
            return Ok(StageOutcome::Unsupported {
                reason: format!(
                    "Backend {} does not claim stage {}. Set {MESHGEN_STAGES_ENV} to include it.",
                    self.url, context.stage
                ),
            });
        }

        fs::create_dir_all(context.stage_dir)?;

        let request = &context.record.request;
        let body = json!({
            "stage": context.stage.id(),
            "character": context.record.slug,
            "prompt": request.prompt,
            "style": request.style,
            "targetHero": request.target_hero,
            "targetPolycount": request.target_polycount,
            "topology": request.topology,
            "pose": request.pose_mode,
            "reference": request.reference_image.as_ref().map(|path| {
                crate::util::absolute(path).to_string_lossy().into_owned()
            }),
            "meshIn": context.mesh.map(|path| path.to_string_lossy().into_owned()),
            "characterDir": context.character_dir.to_string_lossy(),
            "stageDir": context.stage_dir.to_string_lossy(),
        });

        let agent = http_agent(self.timeout.as_secs().max(1));
        let mut agent_request = agent
            .post(&self.url)
            .header("Accept", "application/json")
            .header("User-Agent", MESHY_USER_AGENT);

        if let Some(token) = &self.token {
            agent_request = agent_request.header("Authorization", format!("Bearer {token}"));
        }

        let (value, _) = finish_json(
            "Mesh backend request",
            agent_request.send_json(&body),
        )?;

        let state = value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("succeeded")
            .to_ascii_lowercase();

        let notes = string_list(value.get("notes"));
        let credits = value
            .get("credits")
            .and_then(Value::as_u64)
            .map(|value| value.min(u32::MAX as u64) as u32);

        match state.as_str() {
            "unsupported" => Ok(StageOutcome::Unsupported {
                reason: value
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the backend refused the stage without a reason")
                    .to_string(),
            }),
            "skipped" => Ok(StageOutcome::Skipped {
                reason: value
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the backend skipped the stage without a reason")
                    .to_string(),
            }),
            "failed" | "error" => Ok(StageOutcome::Failed {
                reason: value
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the backend failed the stage without a reason")
                    .to_string(),
            }),
            "pending" | "queued" | "running" => {
                let mut task = context.record.task.clone();
                if let Some(id) = value.get("taskId").and_then(Value::as_str) {
                    task.id = id.to_string();
                    task.status = CharacterStatus::InProgress;
                    task.progress = value
                        .get("progress")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .min(100) as u32;
                }
                Ok(StageOutcome::Pending {
                    task: Some(task),
                    notes,
                })
            }
            _ => {
                let mut outputs = Vec::new();
                if let Some(array) = value.get("outputs").and_then(Value::as_array) {
                    for item in array {
                        outputs.push(write_http_output(context.character_dir, item)?);
                    }
                }

                if context.stage == CharacterStage::Reconstruct {
                    let mesh = context.character_dir.join(MODEL_FILE_NAME);
                    if !mesh.is_file() {
                        return Ok(StageOutcome::Failed {
                            reason: format!(
                                "Backend {} reported success on reconstruct but {} does not exist.",
                                self.url,
                                mesh.display()
                            ),
                        });
                    }
                }

                Ok(StageOutcome::Succeeded {
                    outputs,
                    notes,
                    credits,
                    task: None,
                })
            }
        }
    }
}

fn write_http_output(root: &Path, item: &Value) -> Result<StageOutput> {
    let relative = item
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| Error::msg("Backend output entry has no `path`."))?;

    if relative.starts_with('/') || relative.contains("..") {
        return Err(Error::msg(format!(
            "Backend output path {relative:?} must be relative to the character folder and must \
             not contain `..`."
        )));
    }

    let destination = root.join(relative);

    if let Some(encoded) = item.get("dataBase64").and_then(Value::as_str) {
        let bytes = base64_decode(encoded)?;
        if bytes.len() as u64 > MAX_MODEL_BYTES {
            return Err(Error::msg(format!(
                "Backend output {relative:?} is {} bytes, over the {} byte cap.",
                bytes.len(),
                MAX_MODEL_BYTES
            )));
        }
        write_atomic(&destination, &bytes)?;
    } else if let Some(source) = item.get("sourcePath").and_then(Value::as_str) {
        let source = PathBuf::from(source);
        if !source.is_file() {
            return Err(Error::msg(format!(
                "Backend output sourcePath {} does not exist.",
                source.display()
            )));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &destination)?;
    } else {
        return Err(Error::msg(format!(
            "Backend output {relative:?} has neither `dataBase64` nor `sourcePath`."
        )));
    }

    output_for(root, &destination)
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

/// Resolve the backend for a request.
///
/// Precedence for [`CharacterProviderKind::Auto`]: local command, then HTTP,
/// then nothing. Meshy is only used when the request names it.
pub fn backend_for_request(request: &CharacterRequest) -> Result<Option<Box<dyn MeshBackend>>> {
    if let Some(name) = request
        .backend
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let name = name.to_ascii_lowercase();
        if name == "none" {
            return Ok(None);
        }
        if let Some(stage_list) = name.strip_prefix("local:") {
            let backend = LocalCommandBackend::from_env()?
                .ok_or_else(|| {
                    Error::msg(format!(
                        "Backend {name:?} needs {MESHGEN_COMMAND_ENV} to be set."
                    ))
                })?
                .with_stages(parse_stage_list(stage_list)?);
            return Ok(Some(Box::new(backend)));
        }
        if let Some(url) = name.strip_prefix("http:") {
            let backend = HttpBackend::from_env()?
                .unwrap_or_else(|| HttpBackend::new(url))
                .with_token(env::var(MESHGEN_TOKEN_ENV).ok());
            return Ok(Some(Box::new(backend)));
        }
        if name == "meshy" {
            return Ok(Some(Box::new(ProviderBackend::new(MeshyProvider::from_env()?))));
        }
        if name == "mock" {
            return Ok(Some(Box::new(ProviderBackend::new(MockProvider::new()))));
        }
        if name == "local" || name == "command" {
            let backend = LocalCommandBackend::from_env()?.ok_or_else(|| {
                Error::msg(format!(
                    "Backend {name:?} needs {MESHGEN_COMMAND_ENV} to be set."
                ))
            })?;
            return Ok(Some(Box::new(backend)));
        }
        if name == "http" {
            let backend = HttpBackend::from_env()?.ok_or_else(|| {
                Error::msg(format!(
                    "Backend {name:?} needs {MESHGEN_URL_ENV} to be set."
                ))
            })?;
            return Ok(Some(Box::new(backend)));
        }
        return Err(Error::msg(format!(
            "Unknown mesh backend {name:?}. Use none, local, http, meshy or mock."
        )));
    }

    match request.provider {
        CharacterProviderKind::Auto => {
            if let Some(backend) = LocalCommandBackend::from_env()? {
                return Ok(Some(Box::new(backend)));
            }
            if let Some(backend) = HttpBackend::from_env()? {
                return Ok(Some(Box::new(backend)));
            }
            Ok(None)
        }
        CharacterProviderKind::Local => {
            let backend = LocalCommandBackend::from_env()?.ok_or_else(|| {
                Error::msg(format!(
                    "The local backend needs {MESHGEN_COMMAND_ENV} to be set to an executable."
                ))
            })?;
            Ok(Some(Box::new(backend)))
        }
        CharacterProviderKind::Http => {
            let backend = HttpBackend::from_env()?.ok_or_else(|| {
                Error::msg(format!(
                    "The http backend needs {MESHGEN_URL_ENV} to be set to a JSON endpoint."
                ))
            })?;
            Ok(Some(Box::new(backend)))
        }
        CharacterProviderKind::Meshy => Ok(Some(Box::new(ProviderBackend::new(
            MeshyProvider::from_env()?,
        )))),
        CharacterProviderKind::Mock => {
            Ok(Some(Box::new(ProviderBackend::new(MockProvider::new()))))
        }
    }
}

fn parse_stage_list(list: &str) -> Result<Vec<CharacterStage>> {
    let mut stages = Vec::new();
    for part in list.split(',') {
        let stage = CharacterStage::parse(part).ok_or_else(|| {
            Error::msg(format!(
                "Unknown stage {part:?} in backend id. Known stages: {}",
                CharacterStage::ALL
                    .iter()
                    .map(|stage| stage.id())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        stages.push(stage);
    }
    Ok(stages)
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Create a character task and run the stages that need no backend.
pub fn create_character(
    workspace: &Path,
    request: &CharacterRequest,
) -> Result<CharacterTaskRecord> {
    let backend = backend_for_request(request)?;
    create_character_with_backend(workspace, request, backend.as_deref())
}

/// `create_character` with an injected provider, kept for the Meshy and mock
/// paths that already hold a configured provider.
pub fn create_character_with_provider(
    workspace: &Path,
    request: &CharacterRequest,
    provider: &dyn CharacterProvider,
) -> Result<CharacterTaskRecord> {
    let backend = ProviderBackend::new(provider);
    create_character_with_backend(workspace, request, Some(&backend))
}

fn create_character_with_backend(
    workspace: &Path,
    request: &CharacterRequest,
    backend: Option<&dyn MeshBackend>,
) -> Result<CharacterTaskRecord> {
    validate_request(request)?;

    if request.reference_image.is_some() {
        validate_reference_file(request.reference_image.as_deref().unwrap_or(Path::new("")))?;

        // Enforce consent here as well as in the Meshy client, so a future
        // backend cannot upload a reference image by forgetting the check.
        if !request.allow_reference_upload && is_remote(backend) {
            return Err(Error::msg(format!(
                "Refusing to send {} to {} without consent. Pass --allow-reference-upload after \
                 checking the rights to the image.",
                request
                    .reference_image
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                backend.map(MeshBackend::id).unwrap_or_else(|| "none".to_string())
            )));
        }
    }

    let slug = allocate_slug(workspace, &slug_source(request))?;
    let directory = character_dir(workspace, &slug);

    // Create the folder before any backend call so a permission problem cannot
    // spend provider credits on a task the workspace cannot record.
    fs::create_dir_all(&directory)?;

    let backend_id = backend
        .map(MeshBackend::id)
        .unwrap_or_else(|| MeshBackendKind::None.as_str().to_string());

    let stages = CharacterStage::ALL
        .into_iter()
        .map(|stage| StageRecord::new(stage, backend_id.clone()))
        .collect();

    let timestamp = now_iso();
    let mut record = CharacterTaskRecord {
        schema: CHARACTER_SCHEMA,
        slug: slug.clone(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
        request: request.clone(),
        task: CharacterTask::new(format!("pipeline-{slug}"), STAGE_PIPELINE),
        rights: CharacterRights::for_request(request, false),
        backend: backend_id,
        stages,
        reference: None,
        asset: None,
        mesh: None,
    };

    if backend.is_some() && is_remote(backend) {
        record.rights.reference_uploaded =
            request.reference_image.is_some() && request.allow_reference_upload;
    }

    advance(workspace, &mut record, backend)?;

    write_task_record(workspace, &record)?;
    Ok(record)
}

fn is_remote(backend: Option<&dyn MeshBackend>) -> bool {
    matches!(
        backend.map(MeshBackend::kind),
        Some(MeshBackendKind::Meshy) | Some(MeshBackendKind::Http)
    )
}

fn validate_request(request: &CharacterRequest) -> Result<()> {
    if !request.has_input() {
        return Err(Error::msg(
            "Character request needs a prompt or a reference image.",
        ));
    }

    if request.prompt.chars().count() > MESHY_PROMPT_LIMIT {
        return Err(Error::msg(format!(
            "Character prompt is {} characters. Backends accept at most {MESHY_PROMPT_LIMIT}.",
            request.prompt.chars().count()
        )));
    }

    if request.rights_note.trim().is_empty() {
        return Err(Error::msg(
            "Character tasks need a rights note. The default note or a caller supplied note is \
             required by ADR 0005.",
        ));
    }

    if let Some(topology) = &request.topology
        && !matches!(topology.as_str(), "triangle" | "quad")
    {
        return Err(Error::msg(format!(
            "Topology must be `triangle` or `quad`, got {topology:?}."
        )));
    }

    if let Some(pose) = &request.pose_mode
        && !matches!(pose.as_str(), "a-pose" | "t-pose")
    {
        return Err(Error::msg(format!(
            "Pose mode must be `a-pose` or `t-pose`, got {pose:?}."
        )));
    }

    Ok(())
}

/// Poll a task, update `task.json`, and keep advancing the graph.
pub fn refresh_character(workspace: &Path, slug: &str) -> Result<CharacterTaskRecord> {
    let record = read_task_record(workspace, slug)?;
    let backend = backend_for_request(&record.request)?;
    refresh_character_with_backend(workspace, slug, backend.as_deref())
}

/// `refresh_character` with an injected provider.
pub fn refresh_character_with_provider(
    workspace: &Path,
    slug: &str,
    provider: &dyn CharacterProvider,
) -> Result<CharacterTaskRecord> {
    let backend = ProviderBackend::new(provider);
    refresh_character_with_backend(workspace, slug, Some(&backend))
}

fn refresh_character_with_backend(
    workspace: &Path,
    slug: &str,
    backend: Option<&dyn MeshBackend>,
) -> Result<CharacterTaskRecord> {
    let mut record = read_task_record(workspace, slug)?;

    // Advance is idempotent: settled stages are skipped, an unsupported stage
    // is re-gated so a newly configured backend gets a chance.
    advance(workspace, &mut record, backend)?;
    record.updated_at = now_iso();
    write_task_record(workspace, &record)?;
    Ok(record)
}

/// Run every stage that can run now. Stops at the first asynchronous stage that
/// is still in flight.
fn advance(
    workspace: &Path,
    record: &mut CharacterTaskRecord,
    backend: Option<&dyn MeshBackend>,
) -> Result<()> {
    run_prepare_reference(workspace, record)?;
    restore_missing_mesh(workspace, record);

    for stage in CharacterStage::ALL {
        if stage == CharacterStage::PrepareReference
            || stage == CharacterStage::Source2Map
            || stage == CharacterStage::Export
        {
            continue;
        }

        let existing = record
            .stage(stage)
            .map(|stage| stage.state)
            .unwrap_or(StageState::Pending);

        if existing.is_settled() && existing != StageState::Unsupported {
            continue;
        }

        match run_generative_stage(workspace, record, backend, stage)? {
            StageRun::Settled => {}
            StageRun::Pending => {
                refresh_task_status(record);
                return Ok(());
            }
        }
    }

    run_native_tail(workspace, record)?;
    refresh_task_status(record);
    Ok(())
}

enum StageRun {
    Settled,
    Pending,
}

/// A provider task keeps its result behind a signed URL that expires. If the
/// stored GLB disappeared, drop back to the provider instead of leaving a
/// succeeded stage with no file.
fn restore_missing_mesh(workspace: &Path, record: &mut CharacterTaskRecord) {
    let missing = !model_path(workspace, &record.slug).is_file();
    if !missing {
        return;
    }

    let restorable = record
        .stage(CharacterStage::GenerateViews)
        .map(|stage| stage.state == StageState::Succeeded && stage.task_id.is_some())
        .unwrap_or(false);

    if !restorable {
        return;
    }

    if let Some(stage) = record
        .stages
        .iter_mut()
        .find(|stage| stage.id == CharacterStage::GenerateViews)
    {
        stage.state = StageState::Pending;
        stage.outputs.clear();
        stage.notes.clear();
        stage.notes.push(
            "The stored GLB is missing, so the provider task is polled again.".to_string(),
        );
    }

    record.asset = None;
    record.mesh = None;
}

fn run_prepare_reference(_workspace: &Path, record: &mut CharacterTaskRecord) -> Result<()> {
    if let Some(stage) = record.stage(CharacterStage::PrepareReference)
        && stage.state.is_settled()
    {
        return Ok(());
    }

    let now = now_iso();
    let mut stage = StageRecord::new(CharacterStage::PrepareReference, "native");
    stage.updated_at = now;

    let Some(path) = record.request.reference_image.clone() else {
        stage.state = StageState::Skipped;
        stage.notes.push(
            "No reference image. The request is prompt only, so there is no matte, centring or \
             edge check to run."
                .to_string(),
        );
        merge_stage(record, stage);
        record.reference = None;
        return Ok(());
    };

    match analyse_reference(&path) {
        Ok(report) => {
            stage.inputs.push(path.to_string_lossy().into_owned());
            stage.outputs.push(path.to_string_lossy().into_owned());
            stage.bytes = Some(report.bytes);
            stage.notes.extend(report.notes.clone());

            if report.has_alpha_channel && report.is_transparent() {
                stage.notes.push(
                    "The reference carries an alpha matte. The matte is an input mask; it is \
                     never shipped and it does not guarantee a clean silhouette for generation."
                        .to_string(),
                );
            } else if report.has_alpha_channel {
                stage.notes.push(
                    "The reference has an alpha channel but the image is fully opaque. Treat the \
                     matte as absent."
                        .to_string(),
                );
            } else {
                stage.notes.push(
                    "The reference has no alpha channel. Background removal is a separate, \
                     optional step, as it is in the vendor's own uploader."
                        .to_string(),
                );
            }

            if let Some(centered) = report.centered {
                stage.notes.push(if centered {
                    "Subject coverage is within 8 percent of the image centre.".to_string()
                } else {
                    "Subject coverage is off centre. A centred, padded reference generates a \
                     better mesh."
                        .to_string()
                });
            }

            if let Some(quality) = report.edge_quality {
                stage.notes.push(format!(
                    "Alpha edge quality is {:.2}: the share of silhouette edge pixels that are \
                     soft rather than hard-thresholded.",
                    quality
                ));
            }

            for problem in &report.problems {
                stage.notes.push(format!("problem: {problem}"));
            }

            if report.problems.iter().any(|problem| problem.contains("fatal")) {
                stage.state = StageState::Failed;
            } else {
                stage.state = StageState::Succeeded;
            }

            record.reference = Some(report);
        }
        Err(error) => {
            stage.state = StageState::Failed;
            stage.notes.push(error.to_string());
        }
    }

    stage.updated_at = now_iso();
    merge_stage(record, stage);
    Ok(())
}

fn merge_stage(record: &mut CharacterTaskRecord, stage: StageRecord) {
    if let Some(existing) = record.stages.iter_mut().find(|item| item.id == stage.id) {
        *existing = stage;
    } else {
        record.stages.push(stage);
    }
}

fn run_generative_stage(
    workspace: &Path,
    record: &mut CharacterTaskRecord,
    backend: Option<&dyn MeshBackend>,
    stage_id: CharacterStage,
) -> Result<StageRun> {
    let existing = record.stage(stage_id).cloned().unwrap_or_else(|| {
        StageRecord::new(
            stage_id,
            backend
                .map(MeshBackend::id)
                .unwrap_or_else(|| "none".to_string()),
        )
    });

    // Gate the stage on its prerequisites before asking a backend. A reason
    // starting with `blocked:` or `skipped:` is a skip, not a refusal.
    if let Some(reason) = gate_message(record, stage_id, backend) {
        let blocked = reason.starts_with("blocked");
        let skipped = blocked || reason.starts_with("skipped");
        let mut stage = existing;
        stage.state = if skipped {
            StageState::Skipped
        } else {
            StageState::Unsupported
        };
        stage.notes = vec![strip_gate_prefix(&reason).to_string()];
        stage.backend = if blocked {
            "-".to_string()
        } else if skipped {
            "native".to_string()
        } else {
            backend
                .map(MeshBackend::id)
                .unwrap_or_else(|| MeshBackendKind::None.as_str().to_string())
        };
        stage.updated_at = now_iso();
        merge_stage(record, stage);
        return Ok(StageRun::Settled);
    }

    let Some(backend) = backend else {
        let mut stage = existing;
        stage.state = StageState::Unsupported;
        stage.backend = MeshBackendKind::None.as_str().to_string();
        stage.notes = vec![missing_backend_reason(record, stage_id)];
        stage.updated_at = now_iso();
        merge_stage(record, stage);
        return Ok(StageRun::Settled);
    };

    let directory = character_dir(workspace, &record.slug);
    let stage_dir = stage_dir(workspace, &record.slug, stage_id);
    fs::create_dir_all(&stage_dir)?;

    let mesh = model_path(workspace, &record.slug);
    let mesh_ref = mesh.is_file().then_some(mesh.as_path());

    let snapshot = record.clone();
    let context = StageContext {
        stage: stage_id,
        record: &snapshot,
        stage_record: &existing,
        character_dir: &directory,
        stage_dir: &stage_dir,
        mesh: mesh_ref,
    };

    let outcome = backend.run(&context)?;
    let mut stage = existing;
    stage.backend = backend.id();
    stage.updated_at = now_iso();

    match outcome {
        StageOutcome::Succeeded {
            outputs,
            notes,
            credits,
            task,
        } => {
            if let Some(task) = task {
                stage.task_id = Some(task.id.clone());
                record.task = task;
            }
            stage.state = StageState::Succeeded;
            stage.notes.extend(notes);
            stage.credits = credits.or(stage.credits);
            stage.inputs = stage_inputs(record, stage_id);
            if let Some(primary) = outputs.first() {
                stage.bytes = Some(primary.bytes);
                stage.hash = Some(primary.hash.clone());
            }
            stage.outputs = outputs.into_iter().map(|output| output.path).collect();
            merge_stage(record, stage);
            refresh_mesh(workspace, record)?;
            Ok(StageRun::Settled)
        }
        StageOutcome::Pending { task, notes } => {
            stage.state = StageState::Pending;
            stage.notes.extend(notes);
            if let Some(task) = task {
                stage.task_id = Some(task.id.clone());
                if let Some(credits) = task.consumed_credits {
                    stage.credits = Some(credits);
                }
                record.task = task;
            }
            merge_stage(record, stage);
            Ok(StageRun::Pending)
        }
        StageOutcome::Skipped { reason } => {
            stage.state = StageState::Skipped;
            stage.notes.push(reason);
            merge_stage(record, stage);
            Ok(StageRun::Settled)
        }
        StageOutcome::Unsupported { reason } => {
            stage.state = StageState::Unsupported;
            stage.notes.push(reason);
            merge_stage(record, stage);
            Ok(StageRun::Settled)
        }
        StageOutcome::Failed { reason } => {
            stage.state = StageState::Failed;
            stage.notes.push(reason);
            merge_stage(record, stage);
            Ok(StageRun::Settled)
        }
    }
}

/// `Some(reason)` blocks the stage. A reason starting with `blocked` means the
/// stage is waiting on an earlier stage; otherwise no backend can do it.
fn gate_message(
    record: &CharacterTaskRecord,
    stage: CharacterStage,
    backend: Option<&dyn MeshBackend>,
) -> Option<String> {
    let mesh = record.mesh.as_ref();
    let has_mesh = mesh.is_some();
    let budget = record.request.triangle_budget();

    match stage {
        CharacterStage::GenerateViews => {
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(missing_backend_reason(record, stage))
            }
        }
        CharacterStage::Reconstruct => {
            let views = record.stage(CharacterStage::GenerateViews);
            if views.map(|stage| stage.state == StageState::Succeeded).unwrap_or(false) {
                None
            } else {
                Some(
                    "blocked: no views. generate_views has not succeeded, so there is nothing to \
                     reconstruct."
                        .to_string(),
                )
            }
        }
        CharacterStage::Cleanup => {
            let Some(mesh) = mesh else {
                return Some("blocked: no mesh. cleanup needs a reconstructed mesh.".to_string());
            };
            if mesh.primitives == 0 {
                return Some(
                    "skipped: the mesh has no primitives, so there is nothing to decimate."
                        .to_string(),
                );
            }
            if !mesh.over_budget(budget) {
                return Some(format!(
                    "skipped: {} triangles is within the {} triangle LOD0 budget. No geometry was \
                     changed.",
                    mesh.triangles, budget
                ));
            }
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(format!(
                    "{} triangles is {} over the {} triangle LOD0 budget and no backend provides \
                     decimation. Set {MESHGEN_STAGES_ENV} to include cleanup, or raise \
                     --triangle-budget deliberately.",
                    mesh.triangles,
                    mesh.triangles - budget,
                    budget
                ))
            }
        }
        CharacterStage::UvBake => {
            let Some(mesh) = mesh else {
                return Some("blocked: no mesh. UV work needs a reconstructed mesh.".to_string());
            };
            if mesh.has_uvs {
                return Some(
                    "skipped: the mesh already carries TEXCOORD_0, so no UV pass is needed."
                        .to_string(),
                );
            }
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(missing_backend_reason(record, stage))
            }
        }
        CharacterStage::PbrSynthesize => {
            let Some(mesh) = mesh else {
                return Some(
                    "blocked: no mesh. PBR synthesis needs a reconstructed mesh.".to_string(),
                );
            };
            if mesh.textures > 0 {
                return Some(format!(
                    "skipped: the mesh already references {} texture(s). Re-baking would discard \
                     them.",
                    mesh.textures
                ));
            }
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(missing_backend_reason(record, stage))
            }
        }
        CharacterStage::Retopo => {
            if !has_mesh {
                return Some("blocked: no mesh. Retopology needs a mesh.".to_string());
            }
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(missing_backend_reason(record, stage))
            }
        }
        CharacterStage::Rig => {
            if !has_mesh {
                return Some("blocked: no mesh. Rigging needs a mesh.".to_string());
            }
            if let Some(mesh) = mesh
                && mesh.triangles > MESHY_RIG_TRIANGLE_LIMIT
            {
                return Some(format!(
                    "{} triangles is over the {} triangle automatic rigging limit. Remesh first.",
                    mesh.triangles, MESHY_RIG_TRIANGLE_LIMIT
                ));
            }
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(missing_backend_reason(record, stage))
            }
        }
        CharacterStage::AnimateRetarget => {
            let rigged = record
                .stage(CharacterStage::Rig)
                .map(|stage| stage.state == StageState::Succeeded)
                .unwrap_or(false);
            if !rigged {
                return Some(
                    "blocked: no rig. Retargeting needs a succeeded rig stage.".to_string(),
                );
            }
            if backend.is_some_and(|backend| backend.supports(stage)) {
                None
            } else {
                Some(missing_backend_reason(record, stage))
            }
        }
        CharacterStage::PrepareReference | CharacterStage::Source2Map | CharacterStage::Export => {
            None
        }
    }
}

fn strip_gate_prefix(reason: &str) -> &str {
    reason
        .strip_prefix("blocked: ")
        .or_else(|| reason.strip_prefix("skipped: "))
        .unwrap_or(reason)
}

fn missing_backend_reason(record: &CharacterTaskRecord, stage: CharacterStage) -> String {    let declared = if record.backend.is_empty() || record.backend == "none" {
        "no mesh backend is configured".to_string()
    } else {
        format!("backend {} does not claim this stage", record.backend)
    };

    format!(
        "{declared}. {stage} needs a generative backend. Set {MESHGEN_COMMAND_ENV} to an \
         executable with {MESHGEN_ARGS_ENV} as a JSON argument array, or {MESHGEN_URL_ENV} to a \
         JSON model server. Meshy is available with --provider meshy and is not the default."
    )
}

fn stage_inputs(record: &CharacterTaskRecord, stage: CharacterStage) -> Vec<String> {
    let mut inputs = Vec::new();

    if matches!(
        stage,
        CharacterStage::GenerateViews | CharacterStage::Reconstruct | CharacterStage::Retopo
    ) && let Some(path) = &record.request.reference_image
    {
        inputs.push(path.to_string_lossy().into_owned());
    }

    if !matches!(
        stage,
        CharacterStage::GenerateViews | CharacterStage::Reconstruct
    ) && record.asset.is_some()
    {
        inputs.push(MODEL_FILE_NAME.to_string());
    }

    inputs
}

/// Re-read the mesh after any stage that could have replaced it.
fn refresh_mesh(workspace: &Path, record: &mut CharacterTaskRecord) -> Result<()> {
    let path = model_path(workspace, &record.slug);
    if !path.is_file() {
        return Ok(());
    }

    let asset = read_asset(&path, &record.slug)?;
    let bytes = fs::read(&path)?;
    let report = parse_glb(&bytes, "glb")?;
    record.asset = Some(asset);
    record.mesh = Some(report);
    Ok(())
}

fn run_native_tail(workspace: &Path, record: &mut CharacterTaskRecord) -> Result<()> {
    refresh_mesh(workspace, record)?;

    let has_mesh = record.mesh.is_some();

    // cleanup is native analysis even when no backend ran. Record the budget
    // reading so the graph is never blank next to a mesh.
    if has_mesh
        && record
            .stage(CharacterStage::Cleanup)
            .map(|stage| stage.state == StageState::Pending)
            .unwrap_or(true)
    {
        let mut stage = record
            .stage(CharacterStage::Cleanup)
            .cloned()
            .unwrap_or_else(|| StageRecord::new(CharacterStage::Cleanup, "native"));
        let mesh = record
            .mesh
            .clone()
            .unwrap_or_else(MeshReport::default_report);
        let budget = record.request.triangle_budget();
        stage.state = StageState::Skipped;
        stage.backend = "native".to_string();
        stage.notes.push(format!(
            "Measured {} triangles, {} vertices, {} primitive(s). LOD0 budget is {}. Fitting LODs: \
             {:?}.",
            mesh.triangles,
            mesh.vertices,
            mesh.primitives,
            budget,
            mesh.fitting_lods()
        ));
        if mesh.over_budget(budget) {
            stage.state = StageState::Unsupported;
            stage.notes.push(format!(
                "Over budget by {} triangles and this run has no decimation backend. The mesh was \
                 not changed.",
                mesh.triangles - budget
            ));
        }
        stage.updated_at = now_iso();
        merge_stage(record, stage);
    }

    // source2_map always runs when a mesh exists. It is a plan, never a claim.
    let mut source2 = record
        .stage(CharacterStage::Source2Map)
        .cloned()
        .unwrap_or_else(|| StageRecord::new(CharacterStage::Source2Map, "native"));
    source2.backend = "native".to_string();
    source2.updated_at = now_iso();

    if !has_mesh {
        source2.state = StageState::Skipped;
        source2.notes = vec![
            "blocked: no mesh. There is nothing to map to Source 2 artifacts yet.".to_string(),
        ];
    } else {
        let plan = build_source2_plan(record);
        let mut text = serde_json::to_string_pretty(&plan)?;
        text.push('\n');
        write_atomic(&source2_plan_path(workspace, &record.slug), text.as_bytes())?;

        source2.state = StageState::Succeeded;
        source2.outputs = vec![SOURCE2_PLAN_FILE_NAME.to_string()];
        source2.bytes = Some(text.len() as u64);
        source2.hash = Some(format!("crc32:{:08x}", crc32(text.as_bytes())));
        source2.notes = vec![
            format!(
                "Plan written with {} artifact(s). inGameReady stays false and Source 2 compile is \
                 required.",
                plan.artifacts.len()
            ),
            "Hero folder names move between game updates. Resolve every path through the game \
             index instead of trusting this plan's literals."
                .to_string(),
        ];
    }
    merge_stage(record, source2);

    // export places the mesh. No conversion, no rename into a game path.
    let mut export = record
        .stage(CharacterStage::Export)
        .cloned()
        .unwrap_or_else(|| StageRecord::new(CharacterStage::Export, "native"));
    export.backend = "native".to_string();
    export.updated_at = now_iso();

    if !has_mesh {
        export.state = StageState::Skipped;
        export.notes =
            vec!["blocked: no mesh. There is nothing to export yet.".to_string()];
        merge_stage(record, export);
        return Ok(());
    }

    let source = model_path(workspace, &record.slug);
    let destination = export_path(workspace, &record.slug);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &destination)?;
    let output = output_for(&character_dir(workspace, &record.slug), &destination)?;

    export.state = StageState::Succeeded;
    export.notes = vec![
        "The GLB is copied unchanged. Axis, scale, skeleton and material conversion happen in the \
         Source 2 authoring step, not here."
            .to_string(),
    ];
    export.bytes = Some(output.bytes);
    export.hash = Some(output.hash.clone());
    export.outputs = vec![output.path];
    merge_stage(record, export);

    Ok(())
}

fn refresh_task_status(record: &mut CharacterTaskRecord) {
    let stages = record.stages();
    let settled = stages
        .iter()
        .filter(|stage| stage.state.is_settled())
        .count();
    let total = stages.len().max(1);
    let pending = stages
        .iter()
        .any(|stage| matches!(stage.state, StageState::Pending | StageState::Running));
    let failed = stages
        .iter()
        .any(|stage| stage.state == StageState::Failed);
    let unsupported = stages
        .iter()
        .filter(|stage| stage.state == StageState::Unsupported)
        .count();
    let has_mesh = record.mesh.is_some();

    let computed = ((settled * 100) / total) as u32;
    record.task.progress = if pending {
        // Keep the provider's own percentage when it is ahead of our count.
        record.task.progress.max(computed)
    } else {
        computed
    };

    record.task.status = if failed {
        CharacterStatus::Failed
    } else if pending {
        // A freshly created provider task stays PENDING until it is polled.
        match record.task.status {
            CharacterStatus::Pending | CharacterStatus::InProgress => record.task.status,
            _ => CharacterStatus::InProgress,
        }
    } else if unsupported > 0 && !has_mesh {
        CharacterStatus::Blocked
    } else {
        CharacterStatus::Succeeded
    };

    match record.task.status {
        CharacterStatus::Blocked => {
            let blocked: Vec<&str> = stages
                .iter()
                .filter(|stage| stage.state == StageState::Unsupported)
                .map(|stage| stage.id.id())
                .collect();
            record.task.error = Some(format!(
                "No mesh backend for: {}. Nothing was generated and nothing is claimed.",
                blocked.join(", ")
            ));
        }
        CharacterStatus::Failed => {
            if let Some(stage) = stages
                .iter()
                .find(|stage| stage.state == StageState::Failed)
            {
                let reason = stage
                    .notes
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "the stage failed without a reason".to_string());
                record.task.error = Some(format!("{} failed: {reason}", stage.id.id()));
            }
        }
        _ => {
            // Keep a provider error only while it still describes the run.
            let stale = record
                .task
                .error
                .as_deref()
                .map(|error| error.starts_with("No mesh backend for: "))
                .unwrap_or(false);
            if stale {
                record.task.error = None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Source 2 plan
// ---------------------------------------------------------------------------

fn build_source2_plan(record: &CharacterTaskRecord) -> Source2Plan {
    let hero = record
        .request
        .target_hero
        .clone()
        .unwrap_or_else(|| "<target-hero>".to_string());
    let hero_folder = format!("models/{hero}/{hero}");

    let artifacts = vec![
        Source2Artifact {
            kind: "mesh".to_string(),
            source: MODEL_FILE_NAME.to_string(),
            compiled: String::new(),
            shipped_by_mod: true,
            note: "Source mesh for the model compile. Not a game file on its own.".to_string(),
        },
        Source2Artifact {
            kind: "vmdl".to_string(),
            source: format!("{hero_folder}.vmdl"),
            compiled: format!("{hero_folder}.vmdl_c"),
            shipped_by_mod: true,
            note: "Points at the mesh, the hero skeleton, material groups and LODs.".to_string(),
        },
        Source2Artifact {
            kind: "vmat".to_string(),
            source: format!("materials/{hero_folder}_body.vmat"),
            compiled: format!("materials/{hero_folder}_body.vmat_c"),
            shipped_by_mod: true,
            note: "Source 2 shader parameters. A PBR PNG set does not map one to one.".to_string(),
        },
        Source2Artifact {
            kind: "vtex".to_string(),
            source: format!("materials/{hero_folder}_body_color.tga"),
            compiled: format!("materials/{hero_folder}_body_color.vtex_c"),
            shipped_by_mod: true,
            note: "Base colour. Compiled from a TGA or PNG source by resourcecompiler.".to_string(),
        },
        Source2Artifact {
            kind: "vtex".to_string(),
            source: format!("materials/{hero_folder}_body_normal.tga"),
            compiled: format!("materials/{hero_folder}_body_normal.vtex_c"),
            shipped_by_mod: true,
            note: "Normal map, with the Source 2 green channel convention.".to_string(),
        },
        Source2Artifact {
            kind: "vtex".to_string(),
            source: format!("materials/{hero_folder}_body_rough.tga"),
            compiled: format!("materials/{hero_folder}_body_rough.vtex_c"),
            shipped_by_mod: true,
            note: "Roughness. Packed maps are a material decision, not an export detail."
                .to_string(),
        },
        Source2Artifact {
            kind: "vtex".to_string(),
            source: format!("materials/{hero_folder}_body_metal.tga"),
            compiled: format!("materials/{hero_folder}_body_metal.vtex_c"),
            shipped_by_mod: true,
            note: "Metallic. AO may be packed here or into the shader, decided per material."
                .to_string(),
        },
        Source2Artifact {
            kind: "vnmskel_c".to_string(),
            source: String::new(),
            compiled: format!("{hero_folder}.vnmskel_c"),
            shipped_by_mod: false,
            note: "Comes from the game. A mod must not ship a skeleton: it breaks animation and \
                   other mods."
                .to_string(),
        },
        Source2Artifact {
            kind: "animgraph".to_string(),
            source: String::new(),
            compiled: String::new(),
            shipped_by_mod: false,
            note: "AnimGraph 2 binding. Reuse the hero graph; do not invent a new one."
                .to_string(),
        },
    ];

    let mut requirements = vec![
        "Retopologise onto the hero skeleton and transfer weights. A generated rig is not the \
         hero rig."
            .to_string(),
        "Author the .vmdl with the exact game path, material groups and LOD list.".to_string(),
        "Author .vmat materials and compile every texture to .vtex_c.".to_string(),
        "Bind the model in the hero AnimGraph 2 setup without changing the graph contract."
            .to_string(),
        "Build the LOD chain to the triangle ladder, LOD0 first.".to_string(),
        "Compile with CSDK 12 on Windows, or the backend recorded in ADR 0003.".to_string(),
        "Verify in game with a local mod, then decide whether the result is publishable."
            .to_string(),
    ];

    if let Some(hero) = &record.request.target_hero {
        requirements.push(format!(
            "Resolve every `{hero}` path through the game index. Hero folders move between \
             updates."
        ));
    } else {
        requirements.push(
            "No target hero was recorded. The `models/<target-hero>` paths in this plan are \
             placeholders."
                .to_string(),
        );
    }

    let mut notes = vec![
        "inGameReady is false by construction: this pipeline never compiles or launches."
            .to_string(),
        "source2Compile is `required`. No stage in this pipeline sets it to `done`.".to_string(),
        "publishAllowed is false. Nothing here is cleared for publish.".to_string(),
    ];

    if let Some(mesh) = &record.mesh {
        let fitting = mesh.fitting_lods();
        notes.push(format!(
            "The mesh has {} triangles and fits LODs {:?} of the budget ladder. Every LOD still \
             has to be authored.",
            mesh.triangles, fitting
        ));
    }

    Source2Plan {
        generated_at: now_iso(),
        hero: record.request.target_hero.clone(),
        in_game_ready: false,
        source2_compile: SOURCE2_COMPILE_REQUIRED.to_string(),
        publish_allowed: false,
        artifacts,
        skeleton: Source2SkeletonRequirements {
            skeleton: record
                .request
                .target_hero
                .clone()
                .map(|hero| format!("{hero} hero skeleton, from the game"))
                .unwrap_or_else(|| "hero skeleton, from the game".to_string()),
            ship_skeleton: false,
            weights: "Authored against the hero skeleton, not the generated rig. Skin weights \
                      from a generated rig do not transfer to Source 2 bone names."
                .to_string(),
            note: "A mod that ships a .vnmskel_c breaks animation and other mods. Never do it."
                .to_string(),
        },
        anim_graph: Source2AnimGraphRequirements {
            graph: "hero AnimGraph 2 subgraph".to_string(),
            bindings: vec![
                "model reference in the hero graph".to_string(),
                "attachment and bone names matched to the hero skeleton".to_string(),
                "state machine inputs left unchanged".to_string(),
            ],
            note: "Meshy emits no AnimGraph data. This work is ours, not an export option."
                .to_string(),
        },
        requirements,
        notes,
    }
}

// ---------------------------------------------------------------------------
// Status and import
// ---------------------------------------------------------------------------

/// Read recent task records under `generated/characters`, newest first.
pub fn character_status(workspace: &Path) -> Result<Vec<CharacterTaskRecord>> {
    let root = characters_dir(workspace);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut records = Vec::new();

    for entry in entries {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }

        let task_file = path.join(TASK_FILE_NAME);
        if !task_file.is_file() {
            continue;
        }

        match read_task_record_at(&task_file) {
            Ok(record) => records.push(record),
            Err(error) => {
                return Err(Error::msg(format!(
                    "Character task {} could not be read: {error}",
                    task_file.display()
                )));
            }
        }
    }

    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.slug.cmp(&right.slug))
    });

    Ok(records)
}

/// Validate the mesh, write `character.json` and keep the Source 2 work marked
/// as required.
pub fn import_character(workspace: &Path, slug: &str) -> Result<CharacterImport> {
    validate_slug(slug)?;

    let record = read_task_record(workspace, slug)?;

    if record.task.status == CharacterStatus::Blocked {
        let blocked: Vec<String> = record
            .unsupported_stages()
            .into_iter()
            .map(|stage| stage.id().to_string())
            .collect();
        return Err(Error::msg(format!(
            "Task {} is BLOCKED: no backend can run {}. Configure {MESHGEN_COMMAND_ENV} or \
             {MESHGEN_URL_ENV}, or re-run with --provider meshy, then refresh.",
            record.task.id,
            blocked.join(", ")
        )));
    }

    if record.task.status != CharacterStatus::Succeeded {
        return Err(Error::msg(format!(
            "Task {} is {}. Run `dah character status --refresh --slug {slug}` until it succeeds.",
            record.task.id, record.task.status
        )));
    }

    let path = model_path(workspace, slug);
    if !path.is_file() {
        return Err(Error::msg(format!(
            "{} does not exist. Run `dah character status --refresh --slug {slug}` to download or \
             generate the mesh.",
            path.display()
        )));
    }

    let asset = read_asset(&path, slug)?;
    let bytes = fs::read(&path)?;
    let mesh = parse_glb(&bytes, "glb")?;
    let mut session = record.clone();
    if session.asset.as_ref() != Some(&asset) {
        session.asset = Some(asset.clone());
        session.updated_at = now_iso();
        write_task_record(workspace, &session)?;
    }

    let mut notes = vec![
        "The GLB is a generated mesh, not a Deadlock hero model. It has no Source 2 skeleton, \
         vmdl, material or AnimGraph 2 binding."
            .to_string(),
        "Source 2 compile is required. Retopology, skin weights, LODs, materials and AnimGraph 2 \
         work happen outside this pipeline."
            .to_string(),
        "Not approved for publish. A likeness mod needs a separate approval that names the \
         reference and the permission basis."
            .to_string(),
    ];

    if !mesh.problems.is_empty() {
        notes.push(format!(
            "The mesh parser reported {} structural problem(s): {}",
            mesh.problems.len(),
            mesh.problems.join("; ")
        ));
    }

    if record.rights.reference_uploaded {
        notes.push(
            "The reference image was uploaded to the remote provider with explicit consent."
                .to_string(),
        );
    } else if record.rights.reference_image.is_some() {
        notes.push(
            "The reference image stayed on this machine and was not uploaded to the provider."
                .to_string(),
        );
    }

    if record.task.stage == STAGE_TEXT_TO_3D {
        notes.push(
            "This was a text-to-3D preview, which Meshy documents as untextured. A separate refine \
             task is required for textures and is not created here."
                .to_string(),
        );
    }

    let source2_plan = read_source2_plan(workspace, slug).ok();

    let import = CharacterImport {
        schema: CHARACTER_SCHEMA,
        slug: slug.to_string(),
        created_at: record.created_at.clone(),
        imported_at: now_iso(),
        prompt: record.request.prompt.clone(),
        style: record.request.style.clone(),
        provider: record.request.provider.as_str().to_string(),
        backend: record.backend.clone(),
        target_hero: record.request.target_hero.clone(),
        task_id: record.task.id.clone(),
        asset,
        rights: record.rights.clone(),
        source2_compile: SOURCE2_COMPILE_REQUIRED.to_string(),
        in_game_ready: false,
        stages: record.stages(),
        mesh: Some(mesh),
        source2_plan,
        compile_requirements: vec![
            "Retopologise the mesh onto a Deadlock hero skeleton and transfer weights".to_string(),
            "Author a .vmdl source that points at the mesh and the skeleton".to_string(),
            "Author .vmat materials and compile textures to .vtex_c".to_string(),
            "Bind the model in the hero AnimGraph 2 setup".to_string(),
            "Build LODs for the model".to_string(),
            "Compile with CSDK 12 on Windows, or the backend recorded in ADR 0003".to_string(),
        ],
        notes,
    };

    let mut text = serde_json::to_string_pretty(&import)?;
    text.push('\n');
    write_atomic(&character_manifest_path(workspace, slug), text.as_bytes())?;

    Ok(import)
}

pub fn read_character_import(workspace: &Path, slug: &str) -> Result<CharacterImport> {
    validate_slug(slug)?;
    let path = character_manifest_path(workspace, slug);
    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "{} does not exist. Run `dah character import {slug}` first.",
                path.display()
            ))
        } else {
            Error::from(error)
        }
    })?;

    let import: CharacterImport = serde_json::from_str(&text)?;
    check_schema(&path, import.schema)?;
    Ok(import)
}

pub fn read_source2_plan(workspace: &Path, slug: &str) -> Result<Source2Plan> {
    validate_slug(slug)?;
    let path = source2_plan_path(workspace, slug);
    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!(
                "{} does not exist. Run `dah character create` or refresh until source2_map runs.",
                path.display()
            ))
        } else {
            Error::from(error)
        }
    })?;

    Ok(serde_json::from_str(&text)?)
}

pub fn read_task_record(workspace: &Path, slug: &str) -> Result<CharacterTaskRecord> {
    validate_slug(slug)?;
    let path = task_path(workspace, slug);
    read_task_record_at(&path).map_err(|error| {
        Error::msg(format!(
            "Character task {} could not be read: {error}. Run `dah character create` first.",
            path.display()
        ))
    })
}

fn read_task_record_at(path: &Path) -> Result<CharacterTaskRecord> {
    let text = fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::msg(format!("{} does not exist.", path.display()))
        } else {
            Error::from(error)
        }
    })?;

    let mut record: CharacterTaskRecord = serde_json::from_str(&text)?;
    check_schema(path, record.schema)?;

    if record.stages.is_empty() {
        let backend = if record.backend.is_empty() {
            "none".to_string()
        } else {
            record.backend.clone()
        };
        record.stages = CharacterStage::ALL
            .into_iter()
            .map(|stage| StageRecord::new(stage, backend.clone()))
            .collect();
    }

    Ok(record)
}

fn check_schema(path: &Path, schema: u32) -> Result<()> {
    if schema > CHARACTER_SCHEMA {
        return Err(Error::msg(format!(
            "{} uses schema {schema}. This build reads schema {CHARACTER_SCHEMA}.",
            path.display()
        )));
    }

    if schema < MIN_READABLE_SCHEMA {
        return Err(Error::msg(format!(
            "{} uses schema {schema}, older than the readable minimum {MIN_READABLE_SCHEMA}.",
            path.display()
        )));
    }

    Ok(())
}

fn write_task_record(workspace: &Path, record: &CharacterTaskRecord) -> Result<()> {
    let mut text = serde_json::to_string_pretty(record)?;
    text.push('\n');
    write_atomic(&task_path(workspace, &record.slug), text.as_bytes())
}

/// Validate a character slug: 1 to 64 lowercase letters, digits and hyphens,
/// starting with a letter or digit.
pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(Error::msg("Character slug must not be empty."));
    }

    if slug.len() > 64 {
        return Err(Error::msg("Character slug must be 64 characters or fewer."));
    }

    let mut characters = slug.chars();
    let first = characters.next().unwrap_or(' ');

    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(Error::msg(
            "Character slug must start with a lowercase letter or a digit.",
        ));
    }

    for character in characters {
        let allowed =
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-';
        if !allowed {
            return Err(Error::msg(format!(
                "Character slug contains {character:?}. Use lowercase letters, digits and hyphens."
            )));
        }
    }

    Ok(())
}

/// Slug base: the prompt when there is one, otherwise the reference file name,
/// so a reference-only task does not land in `character`.
fn slug_source(request: &CharacterRequest) -> String {
    if !request.prompt.trim().is_empty() {
        return slugify(&request.prompt);
    }

    request
        .reference_image
        .as_ref()
        .and_then(|path| path.file_stem())
        .and_then(|value| value.to_str())
        .map(slugify)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| slugify(&request.prompt))
}

pub fn slugify(prompt: &str) -> String {
    let mut output = String::new();
    let mut pending_dash = false;

    for character in prompt.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !output.is_empty() {
                output.push('-');
            }
            pending_dash = false;
            output.push(character.to_ascii_lowercase());
        } else {
            pending_dash = !output.is_empty();
        }

        if output.len() >= MAX_SLUG_LENGTH {
            break;
        }
    }

    let trimmed = output.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "character".to_string()
    } else {
        trimmed
    }
}

fn allocate_slug(workspace: &Path, base: &str) -> Result<String> {
    let root = characters_dir(workspace);
    let base = base.trim_matches('-');

    if base.is_empty() {
        return Err(Error::msg("Character prompt produced an empty slug."));
    }

    let mut candidate = base.to_string();
    let mut suffix = 2;

    while root.join(&candidate).exists() {
        candidate = format!("{base}-{suffix}");
        suffix += 1;

        if suffix > 1000 {
            return Err(Error::msg(format!(
                "Could not allocate a character folder for slug {base:?} under {}.",
                root.display()
            )));
        }
    }

    validate_slug(&candidate)?;
    Ok(candidate)
}

fn validate_reference_file(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path).map_err(|error| {
        Error::msg(format!(
            "Reference image {} could not be read: {error}",
            path.display()
        ))
    })?;

    if !metadata.is_file() {
        return Err(Error::msg(format!(
            "Reference image {} is not a file.",
            path.display()
        )));
    }

    if metadata.len() == 0 {
        return Err(Error::msg(format!(
            "Reference image {} is empty.",
            path.display()
        )));
    }

    if metadata.len() > MAX_REFERENCE_IMAGE_BYTES {
        return Err(Error::msg(format!(
            "Reference image {} is {} bytes. The pipeline accepts at most {} bytes.",
            path.display(),
            metadata.len(),
            MAX_REFERENCE_IMAGE_BYTES
        )));
    }

    reference_mime(path).map(|_| ())
}

fn reference_mime(path: &Path) -> Result<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        other => Err(Error::msg(format!(
            "Reference image {} has extension {other:?}. Use JPG, JPEG, PNG or WebP.",
            path.display()
        ))),
    }
}

fn validate_task_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));

    if valid {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "Provider task id {id:?} is not a plain identifier. Refusing to build a request URL."
        )))
    }
}

fn read_asset(path: &Path, slug: &str) -> Result<CharacterAsset> {
    let bytes = fs::read(path)?;
    let version = validate_glb_bytes(&bytes)?;

    Ok(CharacterAsset {
        path: format!("{}/{}", slug, MODEL_FILE_NAME),
        bytes: bytes.len() as u64,
        crc32: format!("{:08x}", crc32(&bytes)),
        format: "glb".to_string(),
        glb_version: version,
    })
}

/// Validate a binary glTF header. The game and the import both need real
/// bytes, so an HTML error page saved as `model.glb` must fail here.
fn validate_glb_bytes(bytes: &[u8]) -> Result<u32> {
    if bytes.len() < 12 {
        return Err(Error::msg(format!(
            "GLB is {} bytes, too short for a binary glTF header.",
            bytes.len()
        )));
    }

    if &bytes[0..4] != b"glTF" {
        return Err(Error::msg(
            "Downloaded model is not a GLB: the file does not start with the glTF magic.",
        ));
    }

    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 2 {
        return Err(Error::msg(format!(
            "Downloaded GLB uses glTF version {version}. This pipeline stores version 2 only."
        )));
    }

    let declared = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if declared != bytes.len() {
        return Err(Error::msg(format!(
            "Downloaded GLB declares {declared} bytes but the file is {} bytes.",
            bytes.len()
        )));
    }

    Ok(version)
}

// ---------------------------------------------------------------------------
// Reference image analysis
// ---------------------------------------------------------------------------

struct ImageHeader {
    format: &'static str,
    width: u32,
    height: u32,
    has_alpha_channel: bool,
    /// PNG colour type, when the file is a PNG.
    png_color_type: Option<u8>,
    png_bit_depth: Option<u8>,
}

/// Read the format, dimensions and alpha-channel presence from the file header.
/// No pixels are touched here.
fn read_image_header(bytes: &[u8]) -> Result<ImageHeader> {
    if bytes.len() >= 8 && bytes[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        if bytes.len() < 33 {
            return Err(Error::msg("PNG header is truncated."));
        }
        let chunk_type = &bytes[12..16];
        if chunk_type != b"IHDR" {
            return Err(Error::msg("PNG is missing the IHDR chunk."));
        }
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        let bit_depth = bytes[24];
        let color_type = bytes[25];
        let has_alpha_channel = matches!(color_type, 4 | 6) || color_type == 3;

        return Ok(ImageHeader {
            format: "png",
            width,
            height,
            has_alpha_channel,
            png_color_type: Some(color_type),
            png_bit_depth: Some(bit_depth),
        });
    }

    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut index = 2;
        while index + 9 < bytes.len() {
            if bytes[index] != 0xFF {
                index += 1;
                continue;
            }
            let marker = bytes[index + 1];
            index += 2;

            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                continue;
            }

            if index + 2 > bytes.len() {
                break;
            }

            let length = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
            let is_sof = (0xC0..=0xCF).contains(&marker)
                && !matches!(marker, 0xC4 | 0xC8 | 0xCC);

            if is_sof {
                if index + 7 > bytes.len() {
                    break;
                }
                let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
                let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
                return Ok(ImageHeader {
                    format: "jpeg",
                    width,
                    height,
                    has_alpha_channel: false,
                    png_color_type: None,
                    png_bit_depth: None,
                });
            }

            if length < 2 {
                break;
            }
            index += length;
        }

        return Err(Error::msg("JPEG has no readable SOF marker."));
    }

    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let chunk = &bytes[12..16];

        if chunk == b"VP8X" {
            let flags = bytes[20];
            let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            return Ok(ImageHeader {
                format: "webp",
                width,
                height,
                has_alpha_channel: flags & 0x10 != 0,
                png_color_type: None,
                png_bit_depth: None,
            });
        }

        if chunk == b"VP8L" {
            if bytes.len() < 25 {
                return Err(Error::msg("WebP VP8L header is truncated."));
            }
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            let width = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            let has_alpha = (bits >> 28) & 1 == 1;
            return Ok(ImageHeader {
                format: "webp",
                width,
                height,
                has_alpha_channel: has_alpha,
                png_color_type: None,
                png_bit_depth: None,
            });
        }

        if chunk == b"VP8 " {
            if bytes.len() < 30 {
                return Err(Error::msg("WebP VP8 header is truncated."));
            }
            let width = u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF;
            let height = u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF;
            return Ok(ImageHeader {
                format: "webp",
                width: width as u32,
                height: height as u32,
                has_alpha_channel: false,
                png_color_type: None,
                png_bit_depth: None,
            });
        }

        return Err(Error::msg("WebP uses an unsupported chunk layout."));
    }

    Err(Error::msg(
        "Reference image is not PNG, JPEG or WebP. The first bytes match no known header.",
    ))
}

/// Decoded alpha plane, only the numbers the checks need.
struct AlphaPlane {
    coverage: f64,
    box_xywh: [u32; 4],
    edge_quality: Option<f64>,
}

/// Decode an 8-bit PNG alpha channel. Returns `None` when the file is not a
/// PNG, not 8-bit, or has no alpha channel.
fn decode_png_alpha(bytes: &[u8], header: &ImageHeader) -> Result<Option<AlphaPlane>> {
    if header.format != "png" {
        return Ok(None);
    }

    let color_type = header.png_color_type.unwrap_or(0);
    let bit_depth = header.png_bit_depth.unwrap_or(0);
    if bit_depth != 8 || !matches!(color_type, 4 | 6) {
        return Ok(None);
    }

    let channels = if color_type == 6 { 4 } else { 2 };
    let width = header.width as usize;
    let height = header.height as usize;
    if width == 0 || height == 0 {
        return Err(Error::msg("PNG declares a zero dimension."));
    }

    // Collect IDAT payloads. The IHDR is 8 bytes of signature plus 25 bytes of
    // chunk header and data, so scanning starts after it.
    let mut index = 8;
    let mut compressed: Vec<u8> = Vec::new();

    while index + 8 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[index],
            bytes[index + 1],
            bytes[index + 2],
            bytes[index + 3],
        ]) as usize;
        let kind = &bytes[index + 4..index + 8];
        let data_start = index + 8;
        let data_end = data_start + length;

        if data_end > bytes.len() {
            break;
        }

        match kind {
            b"IDAT" => compressed.extend_from_slice(&bytes[data_start..data_end]),
            b"IEND" => break,
            _ => {}
        }

        index = data_end + 4;
    }

    if compressed.is_empty() {
        return Err(Error::msg("PNG has no IDAT data."));
    }

    let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
    let mut raw = Vec::new();
    decoder
        .read_to_end(&mut raw)
        .map_err(|error| Error::msg(format!("PNG zlib stream is unreadable: {error}")))?;

    let stride = width * channels;
    let expected = height * (stride + 1);
    if raw.len() < expected {
        return Err(Error::msg(format!(
            "PNG scanlines are {} bytes, expected {expected}.",
            raw.len()
        )));
    }

    let mut previous = vec![0u8; stride];
    let mut current = vec![0u8; stride];
    let mut alpha = vec![0u8; width * height];

    for row in 0..height {
        let start = row * (stride + 1);
        let filter = raw[start];
        current.copy_from_slice(&raw[start + 1..start + 1 + stride]);
        unfilter(filter, &mut current, &previous, channels)?;

        for column in 0..width {
            alpha[row * width + column] = current[column * channels + channels - 1];
        }

        std::mem::swap(&mut previous, &mut current);
    }

    Ok(Some(alpha_stats(&alpha, width, height)))
}

fn unfilter(filter: u8, row: &mut [u8], previous: &[u8], bpp: usize) -> Result<()> {
    match filter {
        0 => Ok(()),
        1 => {
            for index in bpp..row.len() {
                row[index] = row[index].wrapping_add(row[index - bpp]);
            }
            Ok(())
        }
        2 => {
            for index in 0..row.len() {
                row[index] = row[index].wrapping_add(previous[index]);
            }
            Ok(())
        }
        3 => {
            for index in 0..row.len() {
                let left = if index >= bpp { row[index - bpp] as u16 } else { 0 };
                let up = previous[index] as u16;
                row[index] = row[index].wrapping_add(((left + up) / 2) as u8);
            }
            Ok(())
        }
        4 => {
            for index in 0..row.len() {
                let left = if index >= bpp { row[index - bpp] as i16 } else { 0 };
                let up = previous[index] as i16;
                let up_left = if index >= bpp {
                    previous[index - bpp] as i16
                } else {
                    0
                };
                let p = left + up - up_left;
                let pa = (p - left).abs();
                let pb = (p - up).abs();
                let pc = (p - up_left).abs();
                let predictor = if pa <= pb && pa <= pc {
                    left
                } else if pb <= pc {
                    up
                } else {
                    up_left
                };
                row[index] = row[index].wrapping_add(predictor as u8);
            }
            Ok(())
        }
        other => Err(Error::msg(format!("PNG uses unknown filter {other}."))),
    }
}

fn alpha_stats(alpha: &[u8], width: usize, height: usize) -> AlphaPlane {
    let total = (width * height) as f64;
    let mut opaque = 0usize;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;

    for row in 0..height {
        for column in 0..width {
            let value = alpha[row * width + column];
            if value >= ALPHA_THRESHOLD {
                opaque += 1;
                min_x = min_x.min(column);
                min_y = min_y.min(row);
                max_x = max_x.max(column);
                max_y = max_y.max(row);
            }
        }
    }

    let coverage = opaque as f64 / total.max(1.0);

    if opaque == 0 {
        return AlphaPlane {
            coverage: 0.0,
            box_xywh: [0, 0, 0, 0],
            edge_quality: None,
        };
    }

    let box_xywh = [
        min_x as u32,
        min_y as u32,
        (max_x - min_x + 1) as u32,
        (max_y - min_y + 1) as u32,
    ];

    let mut edges = 0usize;
    let mut soft = 0usize;

    for row in 0..height {
        for column in 0..width {
            let value = alpha[row * width + column] as i16;
            let mut is_edge = false;

            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let neighbour_x = column as i32 + dx;
                let neighbour_y = row as i32 + dy;
                if neighbour_x < 0
                    || neighbour_y < 0
                    || neighbour_x >= width as i32
                    || neighbour_y >= height as i32
                {
                    continue;
                }
                let neighbour =
                    alpha[neighbour_y as usize * width + neighbour_x as usize] as i16;
                if (neighbour - value).abs() > 96 {
                    is_edge = true;
                    break;
                }
            }

            if is_edge {
                edges += 1;
                let value = value.clamp(0, 255) as u8;
                if (SOFT_EDGE_MIN..=SOFT_EDGE_MAX).contains(&value) {
                    soft += 1;
                }
            }
        }
    }

    let edge_quality = if edges == 0 {
        None
    } else {
        Some(soft as f64 / edges as f64)
    };

    AlphaPlane {
        coverage,
        box_xywh,
        edge_quality,
    }
}

fn analyse_reference(path: &Path) -> Result<ReferenceReport> {
    let metadata = fs::metadata(path)?;
    let bytes = fs::read(path)?;
    let header = read_image_header(&bytes)?;

    let mut problems = Vec::new();
    let mut notes = Vec::new();

    if header.width < MIN_REFERENCE_SIDE || header.height < MIN_REFERENCE_SIDE {
        problems.push(format!(
            "fatal: {}x{} is below the {MIN_REFERENCE_SIDE} pixel floor on a side.",
            header.width, header.height
        ));
    }

    let pixels = header.width as u64 * header.height as u64;
    if pixels > MAX_REFERENCE_PIXELS {
        problems.push(format!(
            "fatal: {pixels} pixels is over the documented {MAX_REFERENCE_PIXELS} pixel ceiling."
        ));
    }

    let aspect = header.width as f64 / header.height.max(1) as f64;
    if !(0.25..=4.0).contains(&aspect) {
        notes.push(format!(
            "Aspect ratio {aspect:.2} is unusual for a character reference. A square or portrait \
             crop generates a better mesh."
        ));
    }

    let (alpha_coverage, subject_box, centered, edge_quality) =
        if pixels > MAX_ANALYSED_PIXELS {
            notes.push(format!(
                "{pixels} pixels is above the {MAX_ANALYSED_PIXELS} pixel analysis cap, so the \
                 matte was not decoded. Header facts are still recorded."
            ));
            (None, None, None, None)
        } else {
            match decode_png_alpha(&bytes, &header) {
                Ok(Some(plane)) => {
                    let centered = subject_centered(plane.box_xywh, header.width, header.height);
                    (Some(plane.coverage), Some(plane.box_xywh), Some(centered), plane.edge_quality)
                }
                Ok(None) => {
                    notes.push(
                        "Pixel analysis covers 8-bit PNG with an alpha channel only. This file's \
                         matte was not decoded."
                            .to_string(),
                    );
                    (None, None, None, None)
                }
                Err(error) => {
                    problems.push(format!("matte decode failed: {error}"));
                    (None, None, None, None)
                }
            }
        };

    if let Some(coverage) = alpha_coverage {
        if coverage >= 0.999 {
            notes.push(
                "The alpha channel is fully opaque. There is no usable matte.".to_string(),
            );
        } else if coverage <= 0.01 {
            problems.push(
                "The alpha channel hides almost the whole image, so the subject is missing."
                    .to_string(),
            );
        }
    }

    if let Some(quality) = edge_quality
        && quality < 0.15
    {
        notes.push(format!(
            "Alpha edge quality {quality:.2} is close to a hard threshold. Expect jagged \
             silhouette detail in the generated mesh."
        ));
    }

    Ok(ReferenceReport {
        path: path.to_string_lossy().into_owned(),
        format: header.format.to_string(),
        width: header.width,
        height: header.height,
        bytes: metadata.len(),
        has_alpha_channel: header.has_alpha_channel,
        alpha_coverage,
        subject_box,
        centered,
        edge_quality,
        problems,
        notes,
    })
}

fn subject_centered(box_xywh: [u32; 4], width: u32, height: u32) -> bool {
    let [x, y, w, h] = box_xywh;
    if w == 0 || h == 0 {
        return false;
    }

    let subject_x = x as f64 + w as f64 / 2.0;
    let subject_y = y as f64 + h as f64 / 2.0;
    let image_x = width as f64 / 2.0;
    let image_y = height as f64 / 2.0;

    let tolerance_x = width as f64 * 0.08;
    let tolerance_y = height as f64 * 0.08;

    (subject_x - image_x).abs() <= tolerance_x && (subject_y - image_y).abs() <= tolerance_y
}

// ---------------------------------------------------------------------------
// GLB parsing
// ---------------------------------------------------------------------------

const GLB_JSON_CHUNK: u32 = 0x4E4F_534A;
const GLB_BIN_CHUNK: u32 = 0x004E_4942;
const GLTF_TRIANGLES: u64 = 4;

/// Parse a GLB far enough to say whether it is structurally sound and what it
/// contains. Accessor bounds are checked against the binary chunk.
pub fn parse_glb(bytes: &[u8], format: &str) -> Result<MeshReport> {
    let glb_version = validate_glb_bytes(bytes)?;

    let mut offset = 12usize;
    let mut json: Option<Value> = None;
    let mut bin_length = 0usize;

    while offset + 8 <= bytes.len() {
        let length = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let kind = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let start = offset + 8;
        let end = start + length;

        if end > bytes.len() {
            return Err(Error::msg(format!(
                "GLB chunk at offset {offset} declares {length} bytes, past the end of the file."
            )));
        }

        match kind {
            GLB_JSON_CHUNK => {
                if json.is_none() {
                    let text = std::str::from_utf8(&bytes[start..end]).map_err(|error| {
                        Error::msg(format!("GLB JSON chunk is not UTF-8: {error}"))
                    })?;
                    let value: Value = serde_json::from_str(text.trim_end_matches(['\0', ' ']))
                        .map_err(|error| {
                            Error::msg(format!("GLB JSON chunk is invalid JSON: {error}"))
                        })?;
                    json = Some(value);
                }
            }
            GLB_BIN_CHUNK => bin_length = bin_length.saturating_add(length),
            _ => {}
        }

        offset = end;
    }

    let json = json.ok_or_else(|| {
        Error::msg("GLB has no JSON chunk. It is not a readable glTF binary.")
    })?;

    let mut problems = Vec::new();

    let asset_version = json
        .pointer("/asset/version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if asset_version != "2.0" {
        problems.push(format!(
            "asset.version is {asset_version:?}, not \"2.0\"."
        ));
    }

    let generator = json
        .pointer("/asset/generator")
        .and_then(Value::as_str)
        .map(str::to_string);

    let scenes = array_len(&json, "scenes") as u32;
    let nodes = array_len(&json, "nodes") as u32;
    let materials = array_len(&json, "materials") as u32;
    let textures = array_len(&json, "textures") as u32;
    let images = array_len(&json, "images") as u32;
    let skins = array_len(&json, "skins") as u32;

    if scenes == 0 {
        problems.push("no scene is declared.".to_string());
    }

    let buffers = json.get("buffers").and_then(Value::as_array);
    let buffer_length = buffers
        .and_then(|buffers| buffers.first())
        .and_then(|buffer| buffer.get("byteLength"))
        .and_then(Value::as_u64);

    if let Some(declared) = buffer_length
        && bin_length > 0
        && declared as usize > bin_length
    {
        problems.push(format!(
            "buffer 0 declares {declared} bytes but the BIN chunk holds {bin_length}."
        ));
    }

    let mut triangles = 0u64;
    let mut vertices = 0u64;
    let mut primitives = 0u32;
    let mut meshes = 0u32;
    let mut has_normals = false;
    let mut has_uvs = false;
    let mut has_tangents = false;
    let mut bounds_min: Option<[f64; 3]> = None;
    let mut bounds_max: Option<[f64; 3]> = None;
    let mut accessors_checked = 0u32;

    let accessors = json.get("accessors").and_then(Value::as_array);
    let buffer_views = json.get("bufferViews").and_then(Value::as_array);

    if let Some(mesh_array) = json.get("meshes").and_then(Value::as_array) {
        meshes = mesh_array.len() as u32;

        for mesh in mesh_array {
            let Some(primitive_array) = mesh.get("primitives").and_then(Value::as_array) else {
                problems.push("a mesh has no primitives array.".to_string());
                continue;
            };

            for primitive in primitive_array {
                primitives += 1;
                let mode = primitive
                    .get("mode")
                    .and_then(Value::as_u64)
                    .unwrap_or(GLTF_TRIANGLES);

                let attributes = primitive.get("attributes").and_then(Value::as_object);
                let position_index = attributes
                    .and_then(|attributes| attributes.get("POSITION"))
                    .and_then(Value::as_u64);

                has_normals |= attributes
                    .map(|attributes| attributes.contains_key("NORMAL"))
                    .unwrap_or(false);
                has_uvs |= attributes
                    .map(|attributes| attributes.contains_key("TEXCOORD_0"))
                    .unwrap_or(false);
                has_tangents |= attributes
                    .map(|attributes| attributes.contains_key("TANGENT"))
                    .unwrap_or(false);

                // Validate every attribute accessor, not just POSITION. A bad
                // normal or UV buffer breaks the mesh just as hard.
                if let Some(attributes) = attributes {
                    for (name, index) in attributes {
                        if name == "POSITION" {
                            continue;
                        }
                        if let Some(accessor) = index.as_u64().and_then(|index| accessor(accessors, index)) {
                            check_accessor(accessor, buffer_views, bin_length, &mut problems)?;
                            accessors_checked += 1;
                        } else {
                            problems.push(format!(
                                "attribute {name} points at an accessor that does not exist."
                            ));
                        }
                    }
                }

                let position = position_index
                    .and_then(|index| accessor(accessors, index))
                    .map(|accessor| check_accessor(accessor, buffer_views, bin_length, &mut problems))
                    .transpose()?;

                if let Some(report) = &position {
                    accessors_checked += 1;
                    vertices += report.count;
                    if let (Some(min), Some(max)) = (report.min, report.max) {
                        bounds_min = Some(merge_min(bounds_min, min));
                        bounds_max = Some(merge_max(bounds_max, max));
                    }
                    if report.min.is_none() || report.max.is_none() {
                        problems.push(
                            "a POSITION accessor has no min/max. Bounds cannot be verified."
                                .to_string(),
                        );
                    }
                } else {
                    problems.push(
                        "a primitive has no POSITION attribute. It cannot be rendered."
                            .to_string(),
                    );
                }

                let index_accessor = primitive
                    .get("indices")
                    .and_then(Value::as_u64)
                    .and_then(|index| accessor(accessors, index));

                if mode != GLTF_TRIANGLES {
                    problems.push(format!(
                        "a primitive uses mode {mode}, not TRIANGLES. Triangle counts are \
                         unavailable."
                    ));
                    continue;
                }

                if let Some(index_accessor) = index_accessor {
                    let report =
                        check_accessor(index_accessor, buffer_views, bin_length, &mut problems)?;
                    accessors_checked += 1;
                    if report.count % 3 != 0 {
                        problems.push(format!(
                            "an index accessor has {} indices, which is not a multiple of 3.",
                            report.count
                        ));
                    }
                    triangles += report.count / 3;
                } else if let Some(report) = &position {
                    if report.count % 3 != 0 {
                        problems.push(format!(
                            "a non-indexed primitive has {} vertices, which is not a multiple of \
                             3.",
                            report.count
                        ));
                    }
                    triangles += report.count / 3;
                }
            }
        }
    }

    if meshes == 0 {
        problems.push("no meshes. The file declares geometry it does not contain.".to_string());
    }

    if images > textures && textures == 0 {
        problems.push(format!(
            "{images} image(s) but no textures. Materials cannot reference them."
        ));
    }

    if bin_length == 0 && triangles > 0 {
        problems.push("triangles are declared but the BIN chunk is empty.".to_string());
    }

    Ok(MeshReport {
        format: format.to_string(),
        bytes: bytes.len() as u64,
        glb_version,
        generator,
        scenes,
        nodes,
        meshes,
        primitives,
        triangles,
        vertices,
        materials,
        textures,
        has_normals,
        has_uvs,
        has_tangents,
        has_skin: skins > 0,
        bounds_min,
        bounds_max,
        accessors_checked,
        problems,
    })
}

fn array_len(json: &Value, key: &str) -> usize {
    json.get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn accessor(accessors: Option<&Vec<Value>>, index: u64) -> Option<&Value> {
    accessors
        .and_then(|accessors| accessors.get(index as usize))
}

struct AccessorReport {
    count: u64,
    min: Option<[f64; 3]>,
    max: Option<[f64; 3]>,
}

fn check_accessor(
    accessor: &Value,
    buffer_views: Option<&Vec<Value>>,
    bin_length: usize,
    problems: &mut Vec<String>,
) -> Result<AccessorReport> {
    let count = accessor
        .get("count")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::msg("an accessor has no count."))?;

    if count == 0 {
        problems.push("an accessor declares zero elements.".to_string());
    }

    let component_type = accessor
        .get("componentType")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if !matches!(component_type, 5120..=5126) {
        problems.push(format!(
            "an accessor uses componentType {component_type}, which is not a glTF component type."
        ));
    }

    let kind = accessor.get("type").and_then(Value::as_str).unwrap_or("");
    let components = match kind {
        "SCALAR" => 1usize,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT2" => 4,
        "MAT3" => 9,
        "MAT4" => 16,
        other => {
            problems.push(format!("an accessor uses type {other:?}, which is not glTF."));
            0
        }
    };

    let min = accessor
        .get("min")
        .and_then(Value::as_array)
        .map(|values| parse_vec3(values));
    let max = accessor
        .get("max")
        .and_then(Value::as_array)
        .map(|values| parse_vec3(values));

    if let (Some(min_values), Some(max_values)) = (min, max) {
        let min_len = accessor
            .get("min")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let max_len = accessor
            .get("max")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if min_len != max_len || min_len != components {
            problems.push(format!(
                "an accessor min/max has {min_len}/{max_len} components but type {kind} needs \
                 {components}."
            ));
        }
        for index in 0..3 {
            if min_values[index] > max_values[index] {
                problems.push(
                    "an accessor has min greater than max on an axis. Bounds are invalid."
                        .to_string(),
                );
                break;
            }
        }
    }

    if let Some(view_index) = accessor.get("bufferView").and_then(Value::as_u64) {
        let view = buffer_views
            .and_then(|views| views.get(view_index as usize))
            .ok_or_else(|| {
                Error::msg(format!(
                    "an accessor points at bufferView {view_index}, which does not exist."
                ))
            })?;

        let view_length = view
            .get("byteLength")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let view_offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let accessor_offset = accessor
            .get("byteOffset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;

        let component_size = match component_type {
            5120 | 5121 => 1usize,
            5122 | 5123 => 2,
            _ => 4,
        };
        let stride = view
            .get("byteStride")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or_else(|| component_size * components.max(1));
        let needed = accessor_offset
            + if count > 0 {
                (count as usize - 1) * stride + component_size * components.max(1)
            } else {
                0
            };

        if needed > view_length {
            problems.push(format!(
                "an accessor needs {needed} bytes but bufferView {view_index} declares \
                 {view_length}."
            ));
        }

        if bin_length > 0 && view_offset + view_length > bin_length {
            problems.push(format!(
                "bufferView {view_index} ends at {} but the BIN chunk is {bin_length} bytes.",
                view_offset + view_length
            ));
        }
    }

    Ok(AccessorReport {
        count,
        min: min.filter(|values| values[0].is_finite()),
        max: max.filter(|values| values[0].is_finite()),
    })
}

fn parse_vec3(values: &[Value]) -> [f64; 3] {
    let mut output = [0.0f64; 3];
    for (index, slot) in output.iter_mut().enumerate() {
        *slot = values
            .get(index)
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN);
    }
    output
}

fn merge_min(current: Option<[f64; 3]>, next: [f64; 3]) -> [f64; 3] {
    match current {
        Some(current) => [
            current[0].min(next[0]),
            current[1].min(next[1]),
            current[2].min(next[2]),
        ],
        None => next,
    }
}

fn merge_max(current: Option<[f64; 3]>, next: [f64; 3]) -> [f64; 3] {
    match current {
        Some(current) => [
            current[0].max(next[0]),
            current[1].max(next[1]),
            current[2].max(next[2]),
        ],
        None => next,
    }
}

impl MeshReport {
    fn default_report() -> Self {
        Self {
            format: "glb".to_string(),
            bytes: 0,
            glb_version: 2,
            generator: None,
            scenes: 0,
            nodes: 0,
            meshes: 0,
            primitives: 0,
            triangles: 0,
            vertices: 0,
            materials: 0,
            textures: 0,
            has_normals: false,
            has_uvs: false,
            has_tangents: false,
            has_skin: false,
            bounds_min: None,
            bounds_max: None,
            accessors_checked: 0,
            problems: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

fn http_agent(timeout_seconds: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_seconds)))
        // Read the provider error body instead of losing it to a status error.
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn finish_json(
    action: &str,
    request: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<(Value, Option<u64>)> {
    let mut response =
        request.map_err(|error| Error::Network(format!("{action} failed: {error}")))?;
    let status = response.status().as_u16();
    let retry_after = retry_after_seconds(response.headers());
    let body = response.body_mut().read_to_string().map_err(|error| {
        Error::Network(format!(
            "{action} returned HTTP {status} with an unreadable body: {error}"
        ))
    })?;

    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "{action} returned HTTP {status}: {}",
            api_message(&body)
        )));
    }

    let value: Value = serde_json::from_str(&body).map_err(|error| {
        Error::Network(format!(
            "{action} returned invalid JSON ({error}): {}",
            snippet(&body)
        ))
    })?;

    Ok((value, retry_after))
}

fn finish_bytes(
    action: &str,
    request: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Vec<u8>> {
    let mut response =
        request.map_err(|error| Error::Network(format!("{action} failed: {error}")))?;
    let status = response.status().as_u16();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_MODEL_BYTES + 1)
        .read_to_vec()
        .map_err(|error| {
            Error::Network(format!(
                "{action} returned HTTP {status} with an unreadable body: {error}"
            ))
        })?;

    if !(200..300).contains(&status) {
        let text = String::from_utf8_lossy(&bytes);
        return Err(Error::Network(format!(
            "{action} returned HTTP {status}: {}",
            api_message(&text)
        )));
    }

    if bytes.len() as u64 > MAX_MODEL_BYTES {
        return Err(Error::Network(format!(
            "{action} returned more than {MAX_MODEL_BYTES} bytes. The file was not stored."
        )));
    }

    Ok(bytes)
}

fn retry_after_seconds(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(1, 300))
}

/// Meshy error responses carry `message`. Fall back to a short body excerpt.
fn api_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body)
        && let Some(message) = value.get("message").and_then(Value::as_str)
        && !message.trim().is_empty()
    {
        return message.trim().to_string();
    }

    snippet(body)
}

fn snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "the response body was empty".to_string();
    }

    let mut text: String = trimmed.chars().take(BODY_SNIPPET_CHARS).collect();
    if trimmed.chars().count() > BODY_SNIPPET_CHARS {
        text.push_str("...");
    }
    text
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);

        if chunk.len() > 1 {
            output.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            output.push('=');
        }

        if chunk.len() > 2 {
            output.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }

    output
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    let mut values = Vec::with_capacity(input.len());
    for character in input.chars() {
        match character {
            'A'..='Z' => values.push(character as u8 - b'A'),
            'a'..='z' => values.push(character as u8 - b'a' + 26),
            '0'..='9' => values.push(character as u8 - b'0' + 52),
            '+' => values.push(62),
            '/' => values.push(63),
            '=' | '\n' | '\r' | ' ' | '\t' => {}
            other => {
                return Err(Error::msg(format!(
                    "Backend base64 payload contains {other:?}."
                )));
            }
        }
    }

    if values.len() % 4 == 1 {
        return Err(Error::msg(
            "Backend base64 payload has a trailing single character.",
        ));
    }

    let mut output = Vec::with_capacity(values.len() / 4 * 3);
    for chunk in values.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        output.push((chunk[0] << 2) | (chunk[1] >> 4));
        if chunk.len() > 2 {
            output.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        if chunk.len() > 3 {
            output.push((chunk[2] << 6) | chunk[3]);
        }
    }

    Ok(output)
}

/// A minimal valid GLB with an empty scene. The mock provider stores this so
/// the import path validates real glTF bytes instead of trusting a filename.
fn minimal_glb_bytes() -> Vec<u8> {
    let mut json =
        br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[]}],"nodes":[]}"#.to_vec();

    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }

    let total = 12 + 8 + json.len();
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json);

    output
}

#[cfg(test)]
mod tests;
