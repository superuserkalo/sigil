//! Tools: a JSON Schema, a handler, an idempotency flag, an optional timeout and
//! an optional per-tool `beforeTool` hook.
//!
//! Validate, then call. A handler that honours its cancellation token reports
//! what it has within a grace period after the timeout fires; one that does not
//! is abandoned and the call fails with a tool error the model can read.

use crate::{
    types::{Error, Result, ToolCall, ToolDefinition},
    MAX_TIMEOUT_SECONDS,
};
use futures_util::future::BoxFuture;
use jsonschema::{Draft, Retrieve, Uri, Validator};
use serde_json::Value;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// A handler that honours its signal gets this long to report what it has.
pub const TOOL_TIMEOUT_GRACE: Duration = Duration::from_secs(1);

/// What a tool handler and hook are told about the call they serve.
#[derive(Clone)]
pub struct ToolContext {
    /// Fires when the run is cancelled or the tool's timeout expires.
    pub cancellation: CancellationToken,
    /// Stable across a rerun of the same call: `"<session id>:<intent sequence>"`,
    /// or the call ID outside a session.
    pub execution_id: String,
    /// The host value passed as the run's `context` option. Never journaled or sent to the model.
    pub context: Value,
    timed_out: Arc<AtomicBool>,
}

impl ToolContext {
    pub fn new(
        execution_id: impl Into<String>,
        cancellation: CancellationToken,
        context: Value,
    ) -> Self {
        Self {
            cancellation,
            execution_id: execution_id.into(),
            context,
            timed_out: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A context for code that calls a tool directly, outside a run.
    pub fn standalone(cancellation: CancellationToken) -> Self {
        Self::new(String::new(), cancellation, Value::Null)
    }

    /// True once the tool's timeout expired: the token fired for that, not for a cancelled run.
    pub fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolContext")
            .field("execution_id", &self.execution_id)
            .field("timed_out", &self.timed_out())
            .finish_non_exhaustive()
    }
}

/// What a tool handler returns: model-facing output, or a pause.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolReturn {
    /// Text or JSON. A plain string stays a string; anything else is serialized as JSON.
    Output(String),
    /// End the run until its session is resumed with this call's result.
    Pause(Value),
}

impl From<String> for ToolReturn {
    fn from(text: String) -> Self {
        Self::Output(text)
    }
}

impl From<&str> for ToolReturn {
    fn from(text: &str) -> Self {
        Self::Output(text.to_string())
    }
}

impl From<Value> for ToolReturn {
    fn from(value: Value) -> Self {
        match value {
            Value::String(text) => Self::Output(text),
            other => Self::Output(other.to_string()),
        }
    }
}

/// A hook's verdict on a tool call: run it, refuse it with a reason, run it with
/// other arguments, or pause.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolDecision {
    Allow,
    Deny(String),
    Replace(Value),
    Pause(Value),
}

impl ToolDecision {
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny(reason.into())
    }

    pub fn replace(arguments: Value) -> Self {
        Self::Replace(arguments)
    }

    pub fn pause(payload: Value) -> Self {
        Self::Pause(payload)
    }
}

/// Runs before a tool call, after its arguments are validated.
pub type BeforeTool =
    Arc<dyn Fn(ToolCall, ToolContext) -> BoxFuture<'static, Result<ToolDecision>> + Send + Sync>;

/// A settled tool call as the model will see it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

/// What `afterTool` wants done with a settled result.
#[derive(Debug, Clone, PartialEq)]
pub enum AfterToolOutcome {
    Keep,
    Replace {
        output: String,
        is_error: Option<bool>,
    },
}

/// Sees every tool call that ran, whether it succeeded or failed.
pub type AfterTool = Arc<
    dyn Fn(ToolCall, ToolResult, ToolContext) -> BoxFuture<'static, Result<AfterToolOutcome>>
        + Send
        + Sync,
>;

/// A `beforeTool` hook from a plain closure.
pub fn before_tool_fn<F, Fut>(hook: F) -> BeforeTool
where
    F: Fn(ToolCall, ToolContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<ToolDecision>> + Send + 'static,
{
    Arc::new(move |call, context| Box::pin(hook(call, context)))
}

/// An `afterTool` hook from a plain closure.
pub fn after_tool_fn<F, Fut>(hook: F) -> AfterTool
where
    F: Fn(ToolCall, ToolResult, ToolContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<AfterToolOutcome>> + Send + 'static,
{
    Arc::new(move |call, result, context| Box::pin(hook(call, result, context)))
}

type Handler = dyn Fn(Value, ToolContext) -> BoxFuture<'static, Result<ToolReturn>> + Send + Sync;

pub struct Tool {
    definition: ToolDefinition,
    validator: Validator,
    handler: Arc<Handler>,
    idempotent: bool,
    timeout: Option<Duration>,
    before_tool: Option<BeforeTool>,
}

impl Tool {
    /// A tool whose handler sees its arguments and its cancellation token.
    ///
    /// This is the protocol v1 shape, kept so existing tools build unchanged.
    /// Use [`Tool::with_context`] or [`Tool::builder`] for the full context,
    /// idempotency and timeouts.
    pub fn new<F, Fut>(definition: ToolDefinition, handler: F) -> Result<Self>
    where
        F: Fn(Value, CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        Self::builder(definition).handler(move |arguments, context| {
            let handler = handler.clone();
            let cancellation = context.cancellation.clone();
            async move {
                handler(arguments, cancellation)
                    .await
                    .map(ToolReturn::Output)
            }
        })
    }

    /// A tool whose handler sees the full [`ToolContext`].
    pub fn with_context<F, Fut>(definition: ToolDefinition, handler: F) -> Result<Self>
    where
        F: Fn(Value, ToolContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolReturn>> + Send + 'static,
    {
        Self::builder(definition).handler(handler)
    }

    pub fn builder(definition: ToolDefinition) -> ToolBuilder {
        ToolBuilder {
            definition,
            idempotent: false,
            timeout: None,
            before_tool: None,
        }
    }

    pub fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    pub fn name(&self) -> &str {
        &self.definition.name
    }

    /// True when running the same call twice is harmless. An interrupted
    /// idempotent call is rerun on resume.
    pub fn idempotent(&self) -> bool {
        self.idempotent
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    pub fn before_tool(&self) -> Option<&BeforeTool> {
        self.before_tool.as_ref()
    }

    /// Check the arguments against the tool's schema, naming the first mismatch.
    pub fn check(&self, arguments: &Value) -> Result<()> {
        if !arguments.is_object() {
            return Err(Error::Tool(format!(
                "Invalid arguments for {}: arguments must be a JSON object",
                self.definition.name
            )));
        }
        match self.validator.validate(arguments) {
            Ok(()) => Ok(()),
            Err(error) => {
                let path = error.instance_path.clone();
                Err(Error::Tool(format!(
                    "Invalid arguments for {}{path}: {error}",
                    self.definition.name
                )))
            }
        }
    }

    /// Validate, then call the handler, honouring the timeout and the run's cancellation.
    pub async fn execute(&self, call: &ToolCall, context: ToolContext) -> Result<ToolReturn> {
        self.check(&call.arguments)?;
        let run = context.cancellation.clone();
        let scoped = run.child_token();
        let timed_out = context.timed_out.clone();
        let mut handler_context = context;
        handler_context.cancellation = scoped.clone();
        let mut work = (self.handler)(call.arguments.clone(), handler_context);
        let timeout = self.timeout;
        let deadline = timeout.map(|timeout| {
            tokio::time::Instant::now() + timeout.min(Duration::from_secs(MAX_TIMEOUT_SECONDS))
        });
        let wait = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = run.cancelled() => {
                // The run is cancelled: give the handler its grace, then abandon it.
                scoped.cancel();
                let _ = tokio::time::timeout(TOOL_TIMEOUT_GRACE, &mut work).await;
                Err(Error::Cancelled)
            }
            _ = &mut wait => {
                timed_out.store(true, Ordering::SeqCst);
                scoped.cancel();
                let expired = Error::Tool(format!(
                    "timed out after {} s",
                    timeout.unwrap_or_default().as_secs()
                ));
                if run.is_cancelled() {
                    let _ = tokio::time::timeout(TOOL_TIMEOUT_GRACE, &mut work).await;
                    return Err(Error::Cancelled);
                }
                match tokio::time::timeout(TOOL_TIMEOUT_GRACE, &mut work).await {
                    // A handler that stopped by itself may explain why.
                    Ok(Err(Error::Tool(message))) => Err(Error::Tool(message)),
                    Ok(Ok(value)) => Ok(value),
                    // Anything else from an expired call is the timeout.
                    Ok(Err(_)) | Err(_) => Err(expired),
                }
            }
            result = &mut work => result,
        }
    }
}

/// Builds a [`Tool`] with the options an agent cares about.
pub struct ToolBuilder {
    definition: ToolDefinition,
    idempotent: bool,
    timeout: Option<Duration>,
    before_tool: Option<BeforeTool>,
}

impl ToolBuilder {
    pub fn idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = idempotent;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Fire the handler's signal and report a tool error after this many seconds.
    pub fn timeout_seconds(mut self, seconds: u64) -> Self {
        self.timeout = Some(Duration::from_secs(seconds));
        self
    }

    /// A `beforeTool` hook for this tool alone. It runs before the agent's.
    pub fn before_tool(mut self, hook: BeforeTool) -> Self {
        self.before_tool = Some(hook);
        self
    }

    pub fn handler<F, Fut>(self, handler: F) -> Result<Tool>
    where
        F: Fn(Value, ToolContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolReturn>> + Send + 'static,
    {
        if self.definition.name.is_empty() {
            return Err(Error::Configuration("Tool name is required".into()));
        }
        if self.definition.parameters.get("$async").is_some() {
            return Err(Error::Configuration(
                "Tool schemas must use synchronous validation".into(),
            ));
        }
        if let Some(timeout) = self.timeout {
            if timeout.is_zero() || timeout > Duration::from_secs(MAX_TIMEOUT_SECONDS) {
                return Err(Error::Configuration(format!(
                    "timeout must be greater than zero and at most {MAX_TIMEOUT_SECONDS} s"
                )));
            }
        }
        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(false)
            .with_retriever(NoExternalSchemas)
            .build(&self.definition.parameters)
            .map_err(|error| Error::Configuration(error.to_string()))?;
        Ok(Tool {
            definition: self.definition,
            validator,
            handler: Arc::new(move |arguments, context| Box::pin(handler(arguments, context))),
            idempotent: self.idempotent,
            timeout: self.timeout,
            before_tool: self.before_tool,
        })
    }
}

struct NoExternalSchemas;

impl Retrieve for NoExternalSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("External schemas are disabled: {uri}").into())
    }
}
