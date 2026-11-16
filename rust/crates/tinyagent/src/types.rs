//! Protocol v2 types: items, providers, run options, events and errors.
//!
//! The crate follows upstream TinyAgent protocol v2. A conversation is a list of
//! [`Message`] items; runs are journaled record by record; a run can pause for an
//! approval and resume later. Names kept from protocol v1 are aliases, so an
//! adapter that has not been ported yet still compiles where the shape allows.

use futures_util::{future::BoxFuture, stream::BoxStream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fmt, future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// The protocol this crate implements.
pub const PROTOCOL_VERSION: u32 = 2;
/// The journal format this crate writes. Bump it when a record changes shape.
pub const JOURNAL_VERSION: u32 = 1;
pub const MAX_JSONL_LINE_BYTES: usize = 1_048_576;
pub const MAX_HTTP_BODY_BYTES: usize = 1_048_576;
pub const MAX_QUEUED_OUTPUT_BYTES: usize = 4_194_304;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 131_072;
/// A longer tool timeout would overflow the timer and fire at once.
pub const MAX_TIMEOUT_SECONDS: u64 = 86_400;
/// How long host code that ignores cancellation may keep a run waiting.
pub const CANCEL_GRACE: Duration = Duration::from_secs(1);

pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can stop a run or a session operation.
///
/// [`Error::Tool`] is the one variant whose message is written for the model:
/// it travels into the journal as the tool result. Every other variant is for
/// the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Configuration(String),
    /// The provider failed. Retrying may or may not help; see [`Error::ProviderRetry`].
    Provider(String),
    /// Text a tool chose to give the model (`ToolError` upstream).
    Tool(String),
    /// A stream or sink the run writes to failed.
    Consumer(String),
    /// The journal could not be read or written. Reopen the session.
    Storage(String),
    /// Another writer appended first. Reopen the session.
    Fenced(String),
    /// Another operation already owns the session.
    SessionBusy,
    /// The session is closed or failed. Reopen it.
    SessionClosed,
    /// A run is open; resume or abandon it before starting another.
    ResumeRequired(String),
    /// There is no interrupted run to continue.
    NothingToResume(String),
    /// The interrupted tool call must be reconciled before the run continues.
    RecoveryRequired(String),
    /// The provider failed before any response arrived, so the request may be sent again.
    ProviderRetry {
        message: String,
        retry_after: Option<Duration>,
    },
    /// The run ended itself: a limit, repeated tool errors, a hook failure or missing output.
    Run(RunFailure),
    /// Deliberately not implemented. See the crate README.
    Unsupported(String),
}

impl Error {
    pub fn provider_retry(message: impl Into<String>) -> Self {
        Self::ProviderRetry {
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn retry_after(message: impl Into<String>, retry_after: Duration) -> Self {
        Self::ProviderRetry {
            message: message.into(),
            retry_after: Some(retry_after),
        }
    }

    /// The name a non-tool failure is labelled with in a tool result.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Cancelled => "Cancelled",
            Self::Configuration(_) => "ConfigurationError",
            Self::Provider(_) | Self::ProviderRetry { .. } => "ProviderError",
            Self::Tool(_) => "ToolError",
            Self::Consumer(_) => "ConsumerError",
            Self::Storage(_) => "StorageError",
            Self::Fenced(_) => "FencedError",
            Self::SessionBusy => "SessionBusyError",
            Self::SessionClosed => "SessionFailedError",
            Self::ResumeRequired(_) => "ResumeRequiredError",
            Self::NothingToResume(_) => "NothingToResumeError",
            Self::RecoveryRequired(_) => "RecoveryRequiredError",
            Self::Run(_) => "RunError",
            Self::Unsupported(_) => "UnsupportedError",
        }
    }

    /// The failure code reported for a run that ended with this error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Run(failure) => match failure.code.as_str() {
                "limit_exceeded" => "limit_exceeded",
                "repeated_tool_errors" => "repeated_tool_errors",
                "hook_error" => "hook_error",
                "output_missing" => "output_missing",
                _ => "provider_error",
            },
            Self::Fenced(_) => "session_fenced",
            Self::Storage(_) => "storage_error",
            Self::Unsupported(_) => "unsupported",
            Self::Cancelled => "aborted",
            _ => "provider_error",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::ProviderRetry { .. })
    }

    /// True when a journal append failed or a concurrent writer won. The session object is done.
    pub fn is_storage(&self) -> bool {
        matches!(self, Self::Storage(_) | Self::Fenced(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("Run cancelled"),
            Self::Configuration(message)
            | Self::Provider(message)
            | Self::Tool(message)
            | Self::Consumer(message)
            | Self::Storage(message)
            | Self::Fenced(message)
            | Self::ResumeRequired(message)
            | Self::NothingToResume(message)
            | Self::RecoveryRequired(message)
            | Self::Unsupported(message) => formatter.write_str(message),
            Self::ProviderRetry { message, .. } => formatter.write_str(message),
            Self::SessionBusy => formatter.write_str("Session is busy with another operation"),
            Self::SessionClosed => formatter.write_str("Session is closed or failed; reopen it"),
            Self::Run(failure) => formatter.write_str(&failure.message),
        }
    }
}

impl std::error::Error for Error {}

/// The limit a run exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    Steps,
    Seconds,
    Tokens,
}

impl Limit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steps => "steps",
            Self::Seconds => "seconds",
            Self::Tokens => "tokens",
        }
    }
}

/// Why a run failed, as a client sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<Limit>,
}

/// The protocol v1 name for [`RunFailure`].
pub type RunError = RunFailure;

impl RunFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            limit: None,
        }
    }

    pub fn limit(limit: Limit, message: impl Into<String>) -> Self {
        Self {
            code: "limit_exceeded".into(),
            message: message.into(),
            retryable: false,
            limit: Some(limit),
        }
    }

    /// The failure a run reports for any error that reached it.
    pub fn of(error: &Error) -> Self {
        match error {
            Error::Run(failure) => failure.clone(),
            Error::Cancelled => Self::new("aborted", "run aborted"),
            other => Self {
                code: other.code().into(),
                message: other.to_string(),
                retryable: other.retryable(),
                limit: None,
            },
        }
    }
}

/// Token counts for one model request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Tokens and model requests, summed over one run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.requests += other.requests;
    }

    pub fn add_tokens(&mut self, other: TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }

    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// One journaled history item.
///
/// The tag is the provider role. `Assistant.replay` holds the provider's own
/// output blocks, so a request can be rebuilt exactly as the model wrote it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "role",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Message {
    User {
        text: String,
    },
    Assistant {
        text: String,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
        #[serde(default)]
        replay: Vec<Value>,
    },
    ToolResult {
        call_id: String,
        name: String,
        output: String,
        is_error: bool,
    },
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self::User { text: text.into() }
    }

    pub fn assistant(
        text: impl Into<String>,
        tool_calls: Vec<ToolCall>,
        replay: Vec<Value>,
    ) -> Self {
        Self::Assistant {
            text: text.into(),
            tool_calls,
            replay,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        output: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            name: name.into(),
            output: output.into(),
            is_error,
        }
    }

    pub fn role(&self) -> &'static str {
        match self {
            Self::User { .. } => "user",
            Self::Assistant { .. } => "assistant",
            Self::ToolResult { .. } => "tool_result",
        }
    }

    /// The text of an assistant message, whichever shape it was stored in.
    pub fn text(&self) -> &str {
        match self {
            Self::User { text } => text,
            Self::Assistant { text, .. } => text,
            Self::ToolResult { output, .. } => output,
        }
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        match self {
            Self::Assistant { tool_calls, .. } => tool_calls,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// What a provider is told about the tools a request may call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub instructions: String,
    pub history: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    /// Fires when the run is cancelled. Dropping the stream must still release its I/O.
    pub signal: CancellationToken,
    /// Merged into the request body after everything else, when the provider supports it.
    pub extra: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ProviderTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    /// The provider's raw assistant message, journaled and replayed verbatim.
    pub replay: Vec<Value>,
    /// The provider's raw output items for this turn.
    pub output: Vec<Value>,
}

#[derive(Debug)]
pub enum ProviderEvent {
    TextDelta(String),
    /// The model's visible reasoning, where the API streams it.
    ReasoningDelta(String),
    /// Tokens this request used, when the API reported them. The last one wins.
    Usage(TokenUsage),
    Turn(ProviderTurn),
}

pub type ProviderStream = BoxStream<'static, Result<ProviderEvent>>;

/// Providers yield deltas and usage followed by one complete Turn. Dropping the
/// returned stream must release its I/O; this is how the core cancels providers.
pub trait Provider: Send + Sync {
    fn stream(
        &self,
        request: ProviderRequest,
    ) -> impl Future<Output = Result<ProviderStream>> + Send;
}

/// Every check before a model request. `None` is unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RunLimits {
    /// Model requests per run.
    pub max_steps: Option<u32>,
    /// Wall-clock seconds per run.
    pub max_seconds: Option<f64>,
    /// Input plus output tokens per run.
    pub max_tokens: Option<u64>,
}

/// The tool call a paused run is waiting on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingPause {
    pub tool_call_id: String,
    pub tool_name: String,
    pub payload: Value,
}

/// What a paused run's waiting tool call returns.
#[derive(Debug, Clone, PartialEq)]
pub enum ResumeAnswer {
    Result(Value),
    Error(String),
}

/// One event of a run, in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub run_id: String,
    pub sequence: u64,
    /// The journal record this event stands for, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<u64>,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// The protocol v1 name for [`AgentEvent`].
pub type Event = AgentEvent;

impl AgentEvent {
    /// The event type name, for adapters that switch on a string.
    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStatus {
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EventKind {
    AgentStart {
        #[serde(skip_serializing_if = "Option::is_none")]
        resumed: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        repeated: Option<bool>,
    },
    ModelRequestStart {
        attempt: u32,
    },
    ModelRequestEnd {
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<EventError>,
        #[serde(skip_serializing_if = "Option::is_none")]
        will_retry: Option<bool>,
    },
    ReasoningDelta {
        delta: String,
    },
    TextDelta {
        delta: String,
    },
    CompactionStart,
    CompactionEnd {
        status: CompactionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        changed: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        rerun: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        interrupted: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resumed: Option<bool>,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        paused: Option<bool>,
    },
    AgentEnd {
        #[serde(flatten)]
        outcome: RunOutcome,
    },
}

impl EventKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::AgentStart { .. } => "agent_start",
            Self::ModelRequestStart { .. } => "model_request_start",
            Self::ModelRequestEnd { .. } => "model_request_end",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::TextDelta { .. } => "text_delta",
            Self::CompactionStart => "compaction_start",
            Self::CompactionEnd { .. } => "compaction_end",
            Self::ToolExecutionStart { .. } => "tool_execution_start",
            Self::ToolExecutionEnd { .. } => "tool_execution_end",
            Self::AgentEnd { .. } => "agent_end",
        }
    }
}

/// How a run ended. The tag is `status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunOutcome {
    Completed {
        text: String,
        usage: Usage,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        repeated: Option<bool>,
    },
    Paused {
        pause: PendingPause,
        usage: Usage,
    },
    Failed {
        error: RunFailure,
        usage: Usage,
    },
    Aborted {
        usage: Usage,
    },
}

/// A run sink. Events are delivered in order; an error stops the run.
pub type EventFn = Arc<dyn Fn(AgentEvent) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// A sink that drops every event, for a caller that only wants the result.
pub fn no_events() -> EventFn {
    Arc::new(|_event| Box::pin(std::future::ready(Ok(()))))
}

/// Run a `FnMut`-shaped callback as an [`EventFn`].
///
/// The core delivers one event at a time, so one lock is enough.
pub fn event_sink<F, Fut>(callback: F) -> EventFn
where
    F: FnMut(AgentEvent) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let callback = Arc::new(std::sync::Mutex::new(callback));
    Arc::new(move |event| {
        let callback = callback.clone();
        Box::pin(async move {
            let future = {
                let mut callback = callback
                    .lock()
                    .map_err(|_| Error::Consumer("event callback panicked".into()))?;
                callback(event)
            };
            future.await
        })
    })
}

/// A model request that may be sent like it is.
#[derive(Clone, Default)]
pub struct RunOptions {
    pub run_id: Option<String>,
    /// Only `Agent::run` reads this. A session supplies its own history.
    pub history: Vec<Message>,
    /// Cancels the run when fired.
    pub cancellation: CancellationToken,
    /// A host value for tool handlers and hooks. Never journaled or sent to the model.
    pub context: Value,
    /// Use this model for the run instead of the agent's.
    pub model: Option<String>,
    /// Use these instructions for the run instead of the agent's.
    pub instructions: Option<String>,
    /// Merged into each request body after the provider's own settings.
    pub extra: Option<Value>,
    /// A JSON Schema for an object. The model finishes by calling `final_output`.
    pub output_schema: Option<Value>,
    pub max_steps: Option<u32>,
    pub max_seconds: Option<f64>,
    pub max_tokens: Option<u64>,
    /// Also emit `model_request_start`, `model_request_end` and `reasoning_delta`. Overrides the agent.
    pub trace: Option<bool>,
    /// A caller-chosen ID that makes a repeated prompt safe.
    pub message_id: Option<String>,
    /// `Session::resume` only: the result of the tool call a paused run waits on.
    pub result: Option<Value>,
    /// `Session::resume` only: fail that tool call with this message instead.
    pub error: Option<String>,
    /// Where this run's events go. `None` drops them.
    pub on_event: Option<EventFn>,
}

impl RunOptions {
    /// Checked before anything starts, so a wrong option leaves the session as it was.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("maxSteps", self.max_steps.map(f64::from)),
            ("maxSeconds", self.max_seconds),
            ("maxTokens", self.max_tokens.map(|value| value as f64)),
        ] {
            if let Some(value) = value {
                if !value.is_finite() || value < 0.0 {
                    return Err(Error::Configuration(format!(
                        "{name} must be a finite number that is not negative"
                    )));
                }
            }
        }
        if self
            .output_schema
            .as_ref()
            .is_some_and(|schema| !schema.is_object())
        {
            return Err(Error::Configuration(
                "outputSchema must be an object".into(),
            ));
        }
        if self.extra.as_ref().is_some_and(|extra| !extra.is_object()) {
            return Err(Error::Configuration("extra must be an object".into()));
        }
        if self.error.as_deref().is_some_and(str::is_empty) {
            return Err(Error::Configuration(
                "error must be a non-empty string".into(),
            ));
        }
        Ok(())
    }

    /// The options that belong to the run as a whole, journaled and reused on resume.
    pub fn stored(&self) -> StoredOptions {
        StoredOptions {
            model: self.model.clone(),
            instructions: self.instructions.clone(),
            extra: self.extra.clone(),
            output_schema: self.output_schema.clone(),
            max_steps: self.max_steps,
            max_seconds: self.max_seconds,
            max_tokens: self.max_tokens,
        }
    }
}

/// The run options a `run_start` record keeps, so a resumed run continues with them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

impl StoredOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The run options a resumed run continues with, before this call's own overrides.
    pub fn apply_to(&self, options: &mut RunOptions) {
        if options.model.is_none() {
            options.model = self.model.clone();
        }
        if options.instructions.is_none() {
            options.instructions = self.instructions.clone();
        }
        if options.extra.is_none() {
            options.extra = self.extra.clone();
        }
        if options.output_schema.is_none() {
            options.output_schema = self.output_schema.clone();
        }
        if options.max_steps.is_none() {
            options.max_steps = self.max_steps;
        }
        if options.max_seconds.is_none() {
            options.max_seconds = self.max_seconds;
        }
        if options.max_tokens.is_none() {
            options.max_tokens = self.max_tokens;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub text: String,
    pub history: Vec<Message>,
    /// The provider's raw output items for the final turn. Empty for a resumed or repeated run.
    pub output: Vec<Value>,
    pub usage: Usage,
    /// With an `output_schema`: the validated object the model finished with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Set when the run stopped at a pause. Nothing was committed; resume the session to continue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause: Option<PendingPause>,
    /// True when a prompt's message ID had already committed and nothing ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeated: Option<bool>,
}

impl RunResult {
    pub fn repeated(
        text: String,
        history: Vec<Message>,
        usage: Usage,
        value: Option<Value>,
    ) -> Self {
        Self {
            text,
            history,
            output: Vec::new(),
            usage,
            value,
            pause: None,
            repeated: Some(true),
        }
    }
}

/// 32 random hex digits: a version 4 UUID without its dashes.
pub fn random_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The hex SHA-256 of a string: the journal checksum and the configuration digest.
pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A digest of a JSON value with its object keys sorted, so equal values match.
pub fn canonical_digest(value: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), canonical(&map[key])))
                        .collect(),
                )
            }
            other => other.clone(),
        }
    }
    sha256_hex(&canonical(value).to_string())
}
