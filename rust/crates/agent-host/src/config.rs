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
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Chat => "chat",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "chat" => Some(Self::Chat),
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
                    "DAH_AGENT_PROVIDER must be openai, anthropic or chat; received \"{value}\"."
                )
            })?,
        };

        let model = non_empty(env::var("DAH_AGENT_MODEL").ok()).ok_or_else(|| {
            "DAH_AGENT_MODEL is required. Set it to the exact model id for the configured provider. The host does not choose a default model.".to_string()
        })?;

        let native_key = match provider {
            ProviderKind::Anthropic => env::var("ANTHROPIC_API_KEY").ok(),
            _ => env::var("OPENAI_API_KEY").ok(),
        };
        let api_key = non_empty(env::var("DAH_AGENT_API_KEY").ok()).or(non_empty(native_key));

        if api_key.is_none() && provider != ProviderKind::Chat {
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
        let workspace_dir = match non_empty(env::var("DAH_WORKSPACE_DIR").ok()) {
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

/// Matches the data leaf the native CLI uses, so both processes share one
/// directory.
fn app_data_leaf() -> &'static str {
    if cfg!(target_os = "linux") {
        "deadlock-agent-harness"
    } else {
        "DeadlockAgentHarness"
    }
}

fn data_dir() -> Result<PathBuf, String> {
    if let Some(configured) = non_empty(env::var("DAH_DATA_DIR").ok()) {
        return Ok(expand_home(&configured));
    }
    let base = dirs::data_dir().ok_or_else(|| {
        "Could not locate the OS application data directory. Set DAH_DATA_DIR.".to_string()
    })?;
    Ok(base.join(app_data_leaf()))
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
