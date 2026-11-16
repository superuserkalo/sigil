//! TinyAgent protocol version 2, in Rust.
//!
//! One hand-written agent loop, used by the SDK surface here, the JSONL RPC mode
//! and the HTTP server. A [`Session`] journals every run to a [`Storage`] backend,
//! so a run survives a crash, and a tool call can pause a run until a host
//! answers it with `resume`.
//!
//! ```no_run
//! # use std::{sync::Arc, time::Duration};
//! # use tiny_agent::{Agent, AgentConfig, MemoryStorage, RunOptions, Session, SessionOptions};
//! # async fn example<P: tiny_agent::Provider + 'static>(provider: P) -> tiny_agent::Result<()> {
//! let agent = Arc::new(Agent::new(AgentConfig {
//!     model: "gpt-5.6".into(),
//!     instructions: "You are a coding agent.".into(),
//!     provider,
//!     tools: vec![],
//! })?);
//! let session = Session::new(
//!     agent,
//!     Arc::new(MemoryStorage::new()),
//!     SessionOptions::create(),
//! )
//! .await?;
//! let result = session.run("hello", RunOptions::default()).await?;
//! println!("{}", result.text);
//! # Ok(())
//! # }
//! ```
//!
//! The `rpc`, `http` and `cli` adapters are being brought to the same protocol;
//! see the crate README for what is ported.

mod agent;
mod bash;
pub mod cli;
mod file_storage;
mod http;
mod lines;
mod providers;
mod rpc;
mod session;
mod storage;
mod tool;
mod types;

pub use agent::{
    Agent, AgentBuilder, AgentConfig, PendingCall, PrepareRequest, Prepared, RecordFn, ResumeState,
    RunHooks, OUTPUT_TOOL,
};
pub use bash::{bash_tool, denied_reason, run_bash};
pub use file_storage::{valid_session_id, FileStorage};
pub use http::{serve, HttpOptions, HttpServer};
pub use providers::{AnthropicProvider, ChatCompletionsProvider, OpenAIProvider, ProviderOptions};
pub use rpc::run_rpc;
pub use session::{ContextBudget, EventPage, EventQuery, Session, SessionOptions, UncertainPolicy};
pub use storage::{check_storage, fenced, MemoryStorage, Record, RecordBody, RunStatus, Storage};
pub use tokio_util::sync::CancellationToken;
pub use tool::{
    after_tool_fn, before_tool_fn, AfterTool, AfterToolOutcome, BeforeTool, Tool, ToolBuilder,
    ToolContext, ToolDecision, ToolResult, ToolReturn,
};
pub use types::*;
