//! Host configuration, all from the environment. A key is read here and
//! handed to the provider; it is never written to an envelope, a log line or
//! the journal.

use std::{
    env,
    path::{Path, PathBuf},
};

/// The three provider shapes this host can construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Chat,
    /// ChatGPT subscription through the Codex endpoint.
    Codex,
    /// Grok subscription through the CLI proxy endpoint.
    Grok,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Chat => "chat",
            Self::Codex => "codex",
            Self::Grok => "grok",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "chat" => Some(Self::Chat),
            "codex" => Some(Self::Codex),
            "grok" => Some(Self::Grok),
            _ => None,
        }
    }
}

pub struct Config {
    pub provider: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub data_dir: PathBuf,
    pub session_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub dah_rs_bin: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let provider = match non_empty(env::var("DAH_AGENT_PROVIDER").ok()) {
            None => ProviderKind::OpenAi,
            Some(value) => ProviderKind::parse(&value).ok_or_else(|| {
                format!(
                    "DAH_AGENT_PROVIDER must be openai, anthropic, chat, codex or grok; received \"{value}\"."
                )
            })?,
        };

        let model = non_empty(env::var("DAH_AGENT_MODEL").ok()).ok_or_else(|| {
            "DAH_AGENT_MODEL is required. Set it to the exact model id for the configured provider. The host does not choose a default model.".to_string()
        })?;

        let subscription = matches!(provider, ProviderKind::Codex | ProviderKind::Grok);
        let native_key = match provider {
            ProviderKind::Anthropic => env::var("ANTHROPIC_API_KEY").ok(),
            ProviderKind::Codex | ProviderKind::Grok => None,
            _ => env::var("OPENAI_API_KEY").ok(),
        };
        let api_key = if subscription {
            None
        } else {
            non_empty(env::var("DAH_AGENT_API_KEY").ok()).or(non_empty(native_key))
        };

        if api_key.is_none()
            && !matches!(
                provider,
                ProviderKind::Chat | ProviderKind::Codex | ProviderKind::Grok
            )
        {
            let fallback = if provider == ProviderKind::Anthropic {
                "ANTHROPIC_API_KEY"
            } else {
                "OPENAI_API_KEY"
            };
            return Err(format!(
                "No API key for the {} provider. Set DAH_AGENT_API_KEY or {fallback} in the host environment.",
                provider.as_str()
            ));
        }

        let data_dir = data_dir()?;
        let session_dir = data_dir.join("agent-sessions");
        let workspace_dir = match non_empty(env::var("SIGIL_WORKSPACE_DIR").ok())
            .or_else(|| non_empty(env::var("DAH_WORKSPACE_DIR").ok()))
        {
            Some(value) => expand_home(&value),
            None => data_dir.join("workspace"),
        };

        Ok(Self {
            provider,
            model,
            base_url: non_empty(env::var("DAH_AGENT_BASE_URL").ok()),
            api_key,
            data_dir,
            session_dir,
            workspace_dir,
            dah_rs_bin: non_empty(env::var("DAH_RS_BIN").ok()),
        })
    }

    /// Create the workspace and the journal directory before anything runs.
    pub fn prepare(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.workspace_dir).map_err(|error| {
            format!(
                "Could not create the mod workspace {}. Set DAH_WORKSPACE_DIR to a writable folder: {error}",
                self.workspace_dir.display()
            )
        })?;
        std::fs::create_dir_all(&self.session_dir).map_err(|error| {
            format!(
                "Could not create the session directory {}: {error}",
                self.session_dir.display()
            )
        })
    }
}

/// Sigil's data folder, shared with the app and the CLI (`deadlock_core::app_data`):
/// `SIGIL_DATA_DIR`, then `DAH_DATA_DIR`, else the platform folder named `Sigil`.
fn data_dir() -> Result<PathBuf, String> {
    if let Some(configured) = deadlock_core::app_data::override_dir() {
        return Ok(expand_home(configured.to_string_lossy().trim()));
    }
    if deadlock_core::app_data::platform_base().is_none() {
        return Err(
            "Could not locate the OS application data directory. Set SIGIL_DATA_DIR.".to_string(),
        );
    }
    Ok(deadlock_core::app_data::data_dir())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    Path::new(value).to_path_buf()
}
