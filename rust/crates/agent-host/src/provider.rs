//! Provider construction and the agent's global hooks.

use std::sync::Arc;

use serde_json::json;
use tiny_agent::{
    AfterToolOutcome, Agent, AgentConfig, AnthropicProvider, ChatCompletionsProvider,
    OpenAIProvider, Provider, ProviderOptions, ProviderRequest, ProviderStream, Result, ToolCall,
    ToolContext, ToolDecision,
};

use crate::{
    config::{Config, ProviderKind},
    dah_rs::DahRsCli,
    prompt::SYSTEM_PROMPT,
    redact,
};

/// The three shipped providers behind one concrete type, so the host does not
/// need a generic parameter in every function.
pub enum AnyProvider {
    OpenAi(OpenAIProvider),
    Anthropic(AnthropicProvider),
    Chat(ChatCompletionsProvider),
}

impl Provider for AnyProvider {
    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream> {
        match self {
            Self::OpenAi(provider) => provider.stream(request).await,
            Self::Anthropic(provider) => provider.stream(request).await,
            Self::Chat(provider) => provider.stream(request).await,
        }
    }
}

pub fn build_agent(config: &Config, cli: Arc<DahRsCli>) -> Result<Arc<Agent<AnyProvider>>> {
    let provider = build_provider(config)?;
    let tools = crate::tools::deadlock_tools(cli)?;
    let agent = Agent::builder(AgentConfig {
        model: config.model.clone(),
        instructions: SYSTEM_PROMPT.to_string(),
        provider,
        tools,
    })
    .before_tool(before_tool())
    .after_tool(after_tool())
    .build()?;
    Ok(Arc::new(agent))
}

fn build_provider(config: &Config) -> Result<AnyProvider> {
    let mut options = ProviderOptions::new();
    if let Some(base_url) = &config.base_url {
        options = options.with_base_url(base_url);
    }
    if let Some(api_key) = &config.api_key {
        options = options.with_api_key(api_key);
    }
    match config.provider {
        ProviderKind::OpenAi => Ok(AnyProvider::OpenAi(OpenAIProvider::with_options(options)?)),
        ProviderKind::Anthropic => Ok(AnyProvider::Anthropic(AnthropicProvider::with_options(
            options,
        )?)),
        ProviderKind::Chat => Ok(AnyProvider::Chat(ChatCompletionsProvider::with_options(
            options,
        )?)),
    }
}

/// Exact tool names that perform an app action outside the workspace. The
/// suffix rules catch a future apply or upload tool without pausing reads such
/// as `deploy_plan` or `project_migrate`.
const DANGEROUS_TOOLS: [&str; 5] = [
    "deploy_apply",
    "publish_upload",
    "game_launch",
    "gameinfo_repair",
    "install_addon",
];
const DANGEROUS_SUFFIXES: [&str; 2] = ["_apply", "_upload"];

fn needs_approval(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    DANGEROUS_TOOLS.contains(&lowered.as_str())
        || DANGEROUS_SUFFIXES
            .iter()
            .any(|suffix| lowered.ends_with(suffix))
}

fn before_tool() -> tiny_agent::BeforeTool {
    tiny_agent::before_tool_fn(|call: ToolCall, _context: ToolContext| async move {
        if !needs_approval(&call.name) {
            return Ok(ToolDecision::Allow);
        }
        Ok(ToolDecision::Pause(json!({
            "kind": "approval",
            "tool": call.name,
            "arguments": call.arguments,
        })))
    })
}

fn after_tool() -> tiny_agent::AfterTool {
    tiny_agent::after_tool_fn(
        |_call: ToolCall, result: tiny_agent::ToolResult, _context: ToolContext| async move {
            let redacted = redact::secrets(&result.output);
            if redacted == result.output {
                Ok(AfterToolOutcome::Keep)
            } else {
                Ok(AfterToolOutcome::Replace {
                    output: redacted,
                    is_error: None,
                })
            }
        },
    )
}
