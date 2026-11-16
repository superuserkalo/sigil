//! Reference-image and prompt character generation on top of a hosted
//! image-to-3D provider.
//!
//! The pipeline is deliberately narrow. It creates a provider task, records it
//! under `<workspace>/generated/characters/<slug>/`, polls it, downloads the
//! GLB and imports metadata. It never claims the result is a Deadlock hero
//! model: a generated GLB has no Source 2 skeleton, `vmdl`, material or
//! AnimGraph 2 binding, so `character.json` stays at
//! `source2Compile: "required"` and `inGameReady: false`.
//!
//! Reference images are inputs, not assets. A reference image is only sent to a
//! remote provider when the request carries explicit consent, and no generated
//! character is cleared for publish. See
//! `docs/decisions/0005-reference-images-and-likeness.md`.
//!
//! Meshy endpoints, fields and limits follow the public API reference read on
//! 2026-11-16. See `docs/research/character-generation.md`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};
use crate::vpk::crc32;

/// Schema version for `task.json` and `character.json`.
pub const CHARACTER_SCHEMA: u32 = 1;
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
/// Meshy caps an input image at 20 MB.
pub const MAX_REFERENCE_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Local cap for one downloaded model file.
pub const MAX_MODEL_BYTES: u64 = 512 * 1024 * 1024;

pub const TASK_FILE_NAME: &str = "task.json";
pub const MODEL_FILE_NAME: &str = "model.glb";
pub const CHARACTER_FILE_NAME: &str = "character.json";

/// The generated mesh still has to be retopologised, rigged and compiled.
pub const SOURCE2_COMPILE_REQUIRED: &str = "required";

/// Stage recorded with a task so a later poll knows which endpoint to read.
pub const STAGE_IMAGE_TO_3D: &str = "image-to-3d";
pub const STAGE_TEXT_TO_3D: &str = "text-to-3d";

/// Recorded when the caller does not supply a rights basis.
pub const DEFAULT_RIGHTS_NOTE: &str = "Reference image and likeness rights are not confirmed. \
Generated assets stay local and are not cleared for publish.";

const CREATE_TIMEOUT_SECONDS: u64 = 60;
const POLL_TIMEOUT_SECONDS: u64 = 30;
const DOWNLOAD_TIMEOUT_SECONDS: u64 = 300;
const BODY_SNIPPET_CHARS: usize = 240;
const MAX_SLUG_LENGTH: usize = 48;

/// Which backend runs the generation task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CharacterProviderKind {
    Meshy,
    Mock,
}

impl CharacterProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meshy => "meshy",
            Self::Mock => "mock",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "meshy" => Some(Self::Meshy),
            "mock" => Some(Self::Mock),
            _ => None,
        }
    }
}

/// Provider task state. The wire values match the Meshy status strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CharacterStatus {
    Pending,
    InProgress,
    Succeeded,
    Failed,
    Canceled,
}

impl CharacterStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::InProgress => "IN_PROGRESS",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Canceled => "CANCELED",
        }
    }

    pub fn is_final(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "PENDING" => Some(Self::Pending),
            "IN_PROGRESS" => Some(Self::InProgress),
            "SUCCEEDED" => Some(Self::Succeeded),
            "FAILED" => Some(Self::Failed),
            "CANCELED" | "CANCELLED" => Some(Self::Canceled),
            _ => None,
        }
    }
}

impl std::fmt::Display for CharacterStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One character request. `style` maps to the provider texture prompt, and
/// `target_hero` is recorded for the later Source 2 work; it is not sent to the
/// provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRequest {
    pub prompt: String,
    /// Local reference image. Goes to the provider only with consent.
    pub reference_image: Option<PathBuf>,
    /// Deadlock hero codename this character is intended for, for example
    /// `billy`. Recorded, never uploaded.
    pub target_hero: Option<String>,
    /// Visual style, sent as the Meshy texture prompt.
    pub style: String,
    pub provider: CharacterProviderKind,
    /// Rights basis recorded with the task. See ADR 0005.
    pub rights_note: String,
    /// Explicit consent to upload the reference image to a remote provider.
    pub allow_reference_upload: bool,
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
        }
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

/// Downloaded model metadata recorded after the GLB lands in the workspace.
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

/// `<workspace>/generated/characters/<slug>/task.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterTaskRecord {
    pub schema: u32,
    pub slug: String,
    pub created_at: String,
    pub updated_at: String,
    pub request: CharacterRequest,
    pub task: CharacterTask,
    pub rights: CharacterRights,
    #[serde(default)]
    pub asset: Option<CharacterAsset>,
}

/// `<workspace>/generated/characters/<slug>/character.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterImport {
    pub schema: u32,
    pub slug: String,
    pub created_at: String,
    pub imported_at: String,
    pub prompt: String,
    pub style: String,
    pub provider: String,
    pub target_hero: Option<String>,
    pub task_id: String,
    pub asset: CharacterAsset,
    pub rights: CharacterRights,
    /// `required`. Never `done` from this pipeline.
    pub source2_compile: String,
    /// Always false until a mesh is compiled and verified in game.
    pub in_game_ready: bool,
    pub compile_requirements: Vec<String>,
    pub notes: Vec<String>,
}

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

// ---------------------------------------------------------------------------
// Meshy provider
// ---------------------------------------------------------------------------

/// Meshy image-to-3D and text-to-3D client.
///
/// Image path: `POST /openapi/v1/image-to-3d`, `GET /openapi/v1/image-to-3d/:id`.
/// Text path: `POST /openapi/v2/text-to-3d` with `mode: "preview"`,
/// `GET /openapi/v2/text-to-3d/:id`. The text preview is untextured by design;
/// Meshy textures it in a second `mode: "refine"` task, which this pipeline
/// does not create because it spends credits.
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
                    "Meshy needs an API key. Set {MESHY_API_KEY_ENV} to a Meshy API key, or use \
                     `--provider mock` to run the local scripted provider."
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
        if prompt.is_empty() {
            return Err(Error::msg("Character prompt must not be empty."));
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
                    "pose_mode": "a-pose",
                    "target_formats": ["glb"],
                });
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
                    "pose_mode": "a-pose",
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

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

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

    Ok(CharacterTask {
        id: id.to_string(),
        status,
        progress,
        model_urls,
        thumbnail_url,
        error,
        retry_after_seconds: None,
        stage: stage.to_string(),
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
        if request.prompt.trim().is_empty() {
            return Err(Error::msg("Character prompt must not be empty."));
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
            CharacterStatus::Succeeded | CharacterStatus::Failed | CharacterStatus::Canceled => {}
        }

        Ok(next)
    }

    fn download_asset(&self, url: &str, destination: &Path) -> Result<u64> {
        if !url.starts_with("mock://") {
            return Err(Error::msg(format!(
                "The mock provider only serves mock:// URLs, got {url:?}."
            )));
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        let bytes = minimal_glb_bytes();
        write_atomic(destination, &bytes)?;
        Ok(bytes.len() as u64)
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Create a generation task and record it in the workspace.
pub fn create_character(
    workspace: &Path,
    request: &CharacterRequest,
) -> Result<CharacterTaskRecord> {
    let provider = provider_for_request(request)?;
    create_character_with_provider(workspace, request, provider.as_ref())
}

/// `create_character` with an injected provider. Used by tests and by a caller
/// that already holds a configured provider.
pub fn create_character_with_provider(
    workspace: &Path,
    request: &CharacterRequest,
    provider: &dyn CharacterProvider,
) -> Result<CharacterTaskRecord> {
    if request.prompt.trim().is_empty() {
        return Err(Error::msg("Character prompt must not be empty."));
    }

    if request.prompt.chars().count() > MESHY_PROMPT_LIMIT {
        return Err(Error::msg(format!(
            "Character prompt is {} characters. The provider accepts at most {MESHY_PROMPT_LIMIT}.",
            request.prompt.chars().count()
        )));
    }

    if request.reference_image.is_some() {
        validate_reference_file(request.reference_image.as_deref().unwrap_or(Path::new("")))?;

        // Enforce consent here as well as in the Meshy client, so a future
        // provider cannot upload a reference image by forgetting the check.
        if !request.allow_reference_upload && provider.name() != "mock" {
            return Err(Error::msg(format!(
                "Refusing to send {} to {} without consent. Pass --allow-reference-upload after \
                 checking the rights to the image.",
                request
                    .reference_image
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                provider.name()
            )));
        }
    }

    if request.rights_note.trim().is_empty() {
        return Err(Error::msg(
            "Character tasks need a rights note. The default note or a caller supplied note is \
             required by ADR 0005.",
        ));
    }

    let slug = allocate_slug(workspace, &slugify(&request.prompt))?;
    let directory = character_dir(workspace, &slug);

    // Create the folder before the remote task so a permission problem cannot
    // spend provider credits on a task the workspace cannot record.
    fs::create_dir_all(&directory)?;

    let task = match provider.create_task(request) {
        Ok(task) => task,
        Err(error) => {
            let _ = fs::remove_dir(&directory);
            return Err(error);
        }
    };

    let uploaded = request.reference_image.is_some()
        && request.allow_reference_upload
        && provider.name() != "mock";

    let timestamp = now_iso();
    let record = CharacterTaskRecord {
        schema: CHARACTER_SCHEMA,
        slug: slug.clone(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
        request: request.clone(),
        task,
        rights: CharacterRights::for_request(request, uploaded),
        asset: None,
    };

    write_task_record(workspace, &record)?;
    Ok(record)
}

/// Poll a task, update `task.json`, and download the GLB once it succeeds.
pub fn refresh_character(workspace: &Path, slug: &str) -> Result<CharacterTaskRecord> {
    let record = read_task_record(workspace, slug)?;
    let provider = provider_for_request(&record.request)?;
    refresh_character_with_provider(workspace, slug, provider.as_ref())
}

/// `refresh_character` with an injected provider.
pub fn refresh_character_with_provider(
    workspace: &Path,
    slug: &str,
    provider: &dyn CharacterProvider,
) -> Result<CharacterTaskRecord> {
    let mut record = read_task_record(workspace, slug)?;

    if record.task.status.is_final() {
        let missing_file = !model_path(workspace, slug).is_file();
        if record.task.status == CharacterStatus::Succeeded
            && (record.asset.is_none() || missing_file)
        {
            download_model(workspace, &mut record, provider)?;
            record.updated_at = now_iso();
            write_task_record(workspace, &record)?;
        }
        return Ok(record);
    }

    let polled = provider.poll_task(&record.task)?;
    record.task = polled;
    record.updated_at = now_iso();

    if record.task.status == CharacterStatus::Succeeded {
        download_model(workspace, &mut record, provider)?;
    }

    write_task_record(workspace, &record)?;
    Ok(record)
}

fn download_model(
    workspace: &Path,
    record: &mut CharacterTaskRecord,
    provider: &dyn CharacterProvider,
) -> Result<()> {
    let url = record.task.glb_url().ok_or_else(|| {
        Error::msg(format!(
            "Task {} succeeded without a GLB URL. The provider output cannot be imported.",
            record.task.id
        ))
    })?;

    let slug = record.slug.clone();
    let destination = model_path(workspace, &slug);
    provider.download_asset(url, &destination)?;
    record.asset = Some(read_asset(&destination, &slug)?);
    Ok(())
}

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

/// Validate the downloaded GLB and write `character.json` with the Source 2
/// work still marked as required.
pub fn import_character(workspace: &Path, slug: &str) -> Result<CharacterImport> {
    validate_slug(slug)?;

    let record = read_task_record(workspace, slug)?;

    if record.task.status != CharacterStatus::Succeeded {
        return Err(Error::msg(format!(
            "Task {} is {}. Run `dah character status --refresh --slug {slug}` until it succeeds.",
            record.task.id, record.task.status
        )));
    }

    let path = model_path(workspace, slug);
    if !path.is_file() {
        return Err(Error::msg(format!(
            "{} does not exist. Run `dah character status --refresh --slug {slug}` to download the \
             GLB from the recorded task.",
            path.display()
        )));
    }

    let asset = read_asset(&path, slug)?;
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
            "This was a text-to-3D preview, which the provider documents as untextured. A \
             separate refine task is required for textures and is not created here."
                .to_string(),
        );
    }

    let import = CharacterImport {
        schema: CHARACTER_SCHEMA,
        slug: slug.to_string(),
        created_at: record.created_at.clone(),
        imported_at: now_iso(),
        prompt: record.request.prompt.clone(),
        style: record.request.style.clone(),
        provider: record.request.provider.as_str().to_string(),
        target_hero: record.request.target_hero.clone(),
        task_id: record.task.id.clone(),
        asset,
        rights: record.rights.clone(),
        source2_compile: SOURCE2_COMPILE_REQUIRED.to_string(),
        in_game_ready: false,
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
    if import.schema != CHARACTER_SCHEMA {
        return Err(Error::msg(format!(
            "{} uses schema {}. This build reads schema {CHARACTER_SCHEMA}.",
            path.display(),
            import.schema
        )));
    }

    Ok(import)
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

    let record: CharacterTaskRecord = serde_json::from_str(&text)?;
    if record.schema != CHARACTER_SCHEMA {
        return Err(Error::msg(format!(
            "{} uses schema {}. This build reads schema {CHARACTER_SCHEMA}.",
            path.display(),
            record.schema
        )));
    }

    Ok(record)
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

fn provider_for_request(request: &CharacterRequest) -> Result<Box<dyn CharacterProvider>> {
    match request.provider {
        CharacterProviderKind::Meshy => Ok(Box::new(MeshyProvider::from_env()?)),
        CharacterProviderKind::Mock => Ok(Box::new(MockProvider::new())),
    }
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
            "Reference image {} is {} bytes. The provider accepts at most {} bytes.",
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
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_workspace(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = env::temp_dir().join(format!(
            "dah-character-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn provider_debug_redacts_the_key() {
        let provider =
            MeshyProvider::with_base_url("secret-key-value", "https://api.meshy.ai").unwrap();
        let text = format!("{provider:?}");
        assert!(!text.contains("secret-key-value"));
        assert!(text.contains("redacted"));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn slugify_is_stable() {
        assert_eq!(slugify("A Red Hooded Character"), "a-red-hooded-character");
        assert_eq!(slugify("  --  "), "character");
        assert_eq!(slugify("!!!!"), "character");
    }

    #[test]
    fn minimal_glb_passes_validation() {
        let bytes = minimal_glb_bytes();
        let version = validate_glb_bytes(&bytes).unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn glb_validation_rejects_html_and_truncated_files() {
        assert!(validate_glb_bytes(b"<html>not a model</html>").is_err());
        assert!(validate_glb_bytes(b"glTF").is_err());

        let mut wrong_length = minimal_glb_bytes();
        wrong_length.truncate(wrong_length.len() - 4);
        assert!(validate_glb_bytes(&wrong_length).is_err());
    }

    #[test]
    fn mock_pipeline_records_task_then_imports() {
        let workspace = temp_workspace("mock-pipeline");
        let mut request =
            CharacterRequest::new("a slim pilot in a green coat", CharacterProviderKind::Mock);
        request.target_hero = Some("billy".to_string());

        let record = create_character(&workspace, &request).unwrap();
        assert_eq!(record.task.status, CharacterStatus::Pending);
        assert!(task_path(&workspace, &record.slug).is_file());
        assert!(!model_path(&workspace, &record.slug).exists());

        let first = refresh_character(&workspace, &record.slug).unwrap();
        assert_eq!(first.task.status, CharacterStatus::InProgress);
        assert_eq!(first.task.progress, 50);

        let second = refresh_character(&workspace, &record.slug).unwrap();
        assert_eq!(second.task.status, CharacterStatus::Succeeded);
        assert!(model_path(&workspace, &record.slug).is_file());
        assert!(second.asset.is_some());

        let import = import_character(&workspace, &record.slug).unwrap();
        assert_eq!(import.source2_compile, SOURCE2_COMPILE_REQUIRED);
        assert!(!import.in_game_ready);
        assert_eq!(import.target_hero.as_deref(), Some("billy"));
        assert_eq!(import.provider, "mock");
        assert!(!import.rights.publish_allowed);
        assert!(import.notes.iter().any(|note| note.contains("untextured")));
        assert!(character_manifest_path(&workspace, &record.slug).is_file());

        let status = character_status(&workspace).unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].slug, record.slug);

        // A deleted GLB is restored from the recorded task instead of leaving
        // a succeeded task without an asset.
        fs::remove_file(model_path(&workspace, &record.slug)).unwrap();
        let restored = refresh_character(&workspace, &record.slug).unwrap();
        assert!(restored.asset.is_some());
        assert!(model_path(&workspace, &record.slug).is_file());

        cleanup(&workspace);
    }

    #[test]
    fn import_refuses_an_unfinished_task() {
        let workspace = temp_workspace("unfinished");
        let request = CharacterRequest::new("a test character", CharacterProviderKind::Mock);
        let record = create_character(&workspace, &request).unwrap();

        let error = import_character(&workspace, &record.slug).unwrap_err();
        assert!(error.to_string().contains("PENDING"));

        cleanup(&workspace);
    }

    #[test]
    fn reference_image_needs_consent_for_a_remote_provider() {
        let workspace = temp_workspace("consent");
        let image = workspace.join("reference.png");
        fs::write(&image, b"not really a png").unwrap();

        let mut request = CharacterRequest::new("a test character", CharacterProviderKind::Meshy);
        request.reference_image = Some(image);

        let provider = MeshyProvider::with_base_url("test-key", "http://127.0.0.1:1").unwrap();
        let error = create_character_with_provider(&workspace, &request, &provider).unwrap_err();
        assert!(error.to_string().contains("consent"));

        cleanup(&workspace);
    }

    #[test]
    fn meshy_flow_against_a_local_scripted_server() {
        let workspace = temp_workspace("http");
        let image = workspace.join("reference.png");
        fs::write(&image, b"fake png bytes").unwrap();

        let server = ScriptedMeshyServer::start();

        let mut request = CharacterRequest::new(
            "a tall mechanic with a red scarf",
            CharacterProviderKind::Meshy,
        );
        request.style = "stylized painted game character".to_string();
        request.reference_image = Some(image);
        request.allow_reference_upload = true;

        let provider = MeshyProvider::with_base_url("test-key", server.base_url()).unwrap();

        let record = create_character_with_provider(&workspace, &request, &provider).unwrap();
        assert_eq!(record.task.status, CharacterStatus::Pending);
        assert_eq!(record.task.id, "img-task-1");
        assert_eq!(record.task.stage, STAGE_IMAGE_TO_3D);
        assert!(record.rights.reference_uploaded);

        let refreshed =
            refresh_character_with_provider(&workspace, &record.slug, &provider).unwrap();
        assert_eq!(refreshed.task.status, CharacterStatus::Succeeded);
        assert_eq!(refreshed.task.progress, 100);
        assert_eq!(refreshed.task.retry_after_seconds, Some(2));
        assert!(refreshed.asset.is_some());

        let import = import_character(&workspace, &record.slug).unwrap();
        assert_eq!(import.source2_compile, SOURCE2_COMPILE_REQUIRED);
        assert!(!import.in_game_ready);
        assert_eq!(import.task_id, "img-task-1");
        assert_eq!(import.asset.glb_version, 2);
        assert_eq!(import.asset.format, "glb");
        assert!(import.asset.bytes > 0);
        assert!(!import.rights.publish_allowed);

        server.join();
        cleanup(&workspace);
    }

    struct ScriptedMeshyServer {
        base_url: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl ScriptedMeshyServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");

            let server_url = base_url.clone();
            let handle = thread::spawn(move || {
                for _ in 0..3 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    let (status, content_type, body) = route(&request, &server_url);
                    write_response(&mut stream, status, &content_type, &body);
                }
            });

            Self {
                base_url,
                handle: Some(handle),
            }
        }

        fn base_url(&self) -> &str {
            &self.base_url
        }

        fn join(mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join().unwrap();
            }
        }
    }

    fn route(request: &ScriptedRequest, base_url: &str) -> (u16, String, Vec<u8>) {
        let needs_auth = request.path.starts_with("/openapi/");
        let authorized = !needs_auth
            || request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization") && value == "Bearer test-key"
            });

        if !authorized {
            return (
                401,
                "application/json".to_string(),
                br#"{"message":"invalid api key"}"#.to_vec(),
            );
        }

        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/openapi/v1/image-to-3d") => {
                let body = String::from_utf8_lossy(&request.body);
                if !body.contains("data:image/png;base64,") {
                    return (
                        400,
                        "application/json".to_string(),
                        br#"{"message":"image_url is missing a data URI"}"#.to_vec(),
                    );
                }
                if !body.contains("texture_prompt") {
                    return (
                        400,
                        "application/json".to_string(),
                        br#"{"message":"texture_prompt missing"}"#.to_vec(),
                    );
                }
                (
                    202,
                    "application/json".to_string(),
                    br#"{"result":"img-task-1"}"#.to_vec(),
                )
            }
            ("GET", "/openapi/v1/image-to-3d/img-task-1") => {
                let glb = format!("{base_url}/files/model.glb");
                let thumbnail = format!("{base_url}/files/preview.png");
                let body = json!({
                    "id": "img-task-1",
                    "type": "image-to-3d",
                    "status": "SUCCEEDED",
                    "progress": 100,
                    "model_urls": { "glb": glb },
                    "thumbnail_url": thumbnail,
                    "task_error": null,
                    "consumed_credits": 30,
                });
                (
                    200,
                    "application/json".to_string(),
                    body.to_string().into_bytes(),
                )
            }
            ("GET", "/files/model.glb") => {
                (200, "model/gltf-binary".to_string(), minimal_glb_bytes())
            }
            (method, path) => (
                404,
                "application/json".to_string(),
                format!(r#"{{"message":"no route for {method} {path}"}}"#).into_bytes(),
            ),
        }
    }

    struct ScriptedRequest {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    fn read_request(stream: &mut TcpStream) -> ScriptedRequest {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];

        let header_end = loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                panic!("client closed before sending headers");
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(index) = find_headers_end(&buffer) {
                break index;
            }
        };

        let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();

        let mut headers = Vec::new();
        let mut content_length = 0usize;
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_string();
                let value = value.trim().to_string();
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.parse().unwrap_or(0);
                }
                headers.push((name, value));
            }
        }

        let body_start = header_end + 4;
        while buffer.len() < body_start + content_length {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                panic!("client closed before sending the body");
            }
            buffer.extend_from_slice(&chunk[..read]);
        }

        ScriptedRequest {
            method,
            path,
            headers,
            body: buffer[body_start..body_start + content_length].to_vec(),
        }
    }

    fn find_headers_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
        let reason = match status {
            200 => "OK",
            202 => "Accepted",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            _ => "Error",
        };
        let retry = if status == 200 {
            "Retry-After: 2\r\n"
        } else {
            ""
        };
        let headers = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{retry}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    }
}
