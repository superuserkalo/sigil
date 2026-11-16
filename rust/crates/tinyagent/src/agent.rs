//! The agent loop: one model request at a time, the tool calls the model asks
//! for, and a run that can pause, resume and end itself.
//!
//! The loop has no storage of its own. A [`Session`](crate::Session) journals the
//! run through [`RunHooks`], which is also how an interrupted run comes back in.

use crate::{
    storage::RecordBody,
    tool::{
        AfterTool, AfterToolOutcome, BeforeTool, Tool, ToolContext, ToolDecision, ToolResult,
        ToolReturn,
    },
    types::{
        no_events, AgentEvent, Error, EventError, EventFn, EventKind, Limit, Message, PendingPause,
        Provider, ProviderEvent, ProviderRequest, ProviderTurn, Result, ResumeAnswer, RunFailure,
        RunLimits, RunOptions, RunOutcome, TokenUsage, ToolCall, ToolDefinition, Usage,
        MAX_TOOL_OUTPUT_BYTES, PROTOCOL_VERSION,
    },
};
use futures_util::{future::BoxFuture, StreamExt};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// A model request is sent at most this many times.
const MAX_ATTEMPTS: u32 = 4;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
/// A run ends after this many model turns in a row whose tool calls all failed.
const MAX_ERROR_TURNS: u32 = 3;
/// The tool the model must call to finish a run that has an `outputSchema`.
pub const OUTPUT_TOOL: &str = "final_output";
const INTERRUPTED: &str =
    "This tool call was interrupted and may have partly run. Check its effects before continuing.";

/// The agent's own settings. The provider is the only generic part.
pub struct AgentConfig<P> {
    pub model: String,
    pub instructions: String,
    pub provider: P,
    pub tools: Vec<Tool>,
}

/// Journals one record: returns its sequence, or `None` when the run has no session.
pub type RecordFn =
    Arc<dyn Fn(RecordBody) -> BoxFuture<'static, Result<Option<u64>>> + Send + Sync>;

/// Emits the events a compaction produces, in order, from inside the loop.
pub type CompactionEmitter = Arc<dyn Fn(EventKind) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// What a compaction hook is given, once per model request.
pub struct PrepareRequest {
    pub history: Vec<Message>,
    pub cancellation: CancellationToken,
    pub emit: CompactionEmitter,
}

/// What a compaction hook hands back: the context to send, and what the summary cost.
pub struct Prepared {
    pub history: Vec<Message>,
    pub usage: Option<Usage>,
}

/// Compact before a request if the context has grown too large.
pub type PrepareFn =
    Arc<dyn Fn(PrepareRequest) -> BoxFuture<'static, Result<Prepared>> + Send + Sync>;

/// A tool call of the last model turn that has no result yet.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingCall {
    /// The sequence of its `tool_intent` record, which is also its execution ID.
    pub sequence: u64,
    /// True when the call may run again: it is idempotent, and the journal said so too.
    pub rerun: bool,
    /// The result or error a paused run was resumed with.
    pub answer: Option<ResumeAnswer>,
}

/// The unfinished part of an interrupted run, as its session read it from the journal.
#[derive(Debug, Clone, Default)]
pub struct ResumeState {
    /// The run's journaled history items, starting with its prompt.
    pub items: Vec<Message>,
    /// Tool calls of the last model turn that have no result yet.
    pub calls: Vec<ToolCall>,
    /// Set when the first of `calls` was started and never finished.
    pub pending: Option<PendingCall>,
    /// True when nothing is left but to commit: the final reply, or the typed
    /// output, is already journaled.
    pub finishing: bool,
    /// The typed output a `final_output` call had delivered before the interruption.
    pub value: Option<Value>,
}

/// How a session makes a run durable. A bare agent works without it.
#[derive(Clone, Default)]
pub struct RunHooks {
    pub session_id: Option<String>,
    /// Append one journal record durably and return its sequence.
    pub record: Option<RecordFn>,
    pub prepare: Option<PrepareFn>,
    pub resume: Option<ResumeState>,
}

impl RunHooks {
    pub fn new(session_id: impl Into<String>, record: RecordFn) -> Self {
        Self {
            session_id: Some(session_id.into()),
            record: Some(record),
            prepare: None,
            resume: None,
        }
    }
}

/// One agent loop with its settings.
pub struct Agent<P> {
    model: String,
    instructions: String,
    provider: P,
    tools: BTreeMap<String, Arc<Tool>>,
    definitions: Vec<ToolDefinition>,
    limits: RunLimits,
    before_tool: Option<BeforeTool>,
    after_tool: Option<AfterTool>,
    trace: bool,
    new_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl<P: Provider> Agent<P> {
    pub fn new(config: AgentConfig<P>) -> Result<Self> {
        Self::builder(config).build()
    }

    pub fn builder(config: AgentConfig<P>) -> AgentBuilder<P> {
        AgentBuilder {
            config,
            limits: RunLimits::default(),
            before_tool: None,
            after_tool: None,
            trace: false,
            new_id: None,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn limits(&self) -> RunLimits {
        self.limits
    }

    pub fn trace(&self) -> bool {
        self.trace
    }

    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name).map(Arc::as_ref)
    }

    pub fn tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values().map(Arc::as_ref)
    }

    pub fn before_tool(&self) -> Option<&BeforeTool> {
        self.before_tool.as_ref()
    }

    pub fn after_tool(&self) -> Option<&AfterTool> {
        self.after_tool.as_ref()
    }

    /// A fresh random ID, for a run or for a session.
    pub fn new_id(&self) -> String {
        match &self.new_id {
            Some(new_id) => new_id(),
            None => crate::types::random_id(),
        }
    }

    /// The digest of the configuration that shapes a request: what a session
    /// journals to explain why a continuation used a different model or tool set.
    pub fn configuration(&self) -> String {
        let tools: Vec<Value> = self
            .definitions
            .iter()
            .map(|definition| serde_json::to_value(definition).unwrap_or(Value::Null))
            .collect();
        crate::types::canonical_digest(&serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "model": self.model,
            "instructions": self.instructions,
            "tools": tools,
        }))
    }

    /// Run one prompt and drop the events.
    pub async fn run(&self, prompt: &str, options: RunOptions) -> Result<crate::RunResult> {
        let emit = options.on_event.clone().unwrap_or_else(no_events);
        let history = options.history.clone();
        self.stream(prompt, &history, options, RunHooks::default(), emit)
            .await
    }

    /// Run one prompt, handing each event to `on_event`. The callback applies
    /// backpressure; its error stops the run.
    pub async fn run_with_events<F, Fut>(
        &self,
        prompt: &str,
        options: RunOptions,
        on_event: F,
    ) -> Result<crate::RunResult>
    where
        F: FnMut(AgentEvent) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let history = options.history.clone();
        let emit = crate::types::event_sink(on_event);
        self.stream(prompt, &history, options, RunHooks::default(), emit)
            .await
    }

    /// The loop itself. `prompt` is only read when the run is not resuming;
    /// `history` is what the session has committed so far.
    pub async fn stream(
        &self,
        prompt: &str,
        history: &[Message],
        options: RunOptions,
        hooks: RunHooks,
        emit: EventFn,
    ) -> Result<crate::RunResult> {
        options.validate()?;
        let resume = hooks.resume.clone();
        let mut transcript: Vec<Message> = history.to_vec();
        match &resume {
            Some(state) => transcript.extend(state.items.clone()),
            None => transcript.push(Message::User {
                text: prompt.to_string(),
            }),
        }
        let run_id = options
            .run_id
            .clone()
            .unwrap_or_else(|| format!("r_{}", self.new_id()));
        // With an outputSchema the run finishes through a tool whose parameters are that schema.
        let mut tools = self.tools.clone();
        if let Some(schema) = options.output_schema.clone() {
            if tools.contains_key(OUTPUT_TOOL) {
                return Err(Error::Configuration(format!(
                    "outputSchema needs the tool name {OUTPUT_TOOL}, which is already taken"
                )));
            }
            tools.insert(OUTPUT_TOOL.into(), Arc::new(output_tool(&schema)?));
        }
        let definitions: Vec<ToolDefinition> =
            tools.values().map(|tool| tool.definition()).collect();
        let reply = resume.as_ref().and_then(|state| {
            if !state.finishing {
                return None;
            }
            Some(Reply {
                text: match &state.value {
                    Some(_) => String::new(),
                    None => state
                        .items
                        .last()
                        .map(|item| item.text().to_string())
                        .unwrap_or_default(),
                },
                output: Vec::new(),
                value: state.value.clone(),
            })
        });
        let mut run = Run {
            emitter: Emitter {
                run_id,
                sequence: Arc::new(AtomicU64::new(0)),
                emit,
            },
            cancellation: options.cancellation.clone(),
            usage: Usage::default(),
            steps: 0,
            started: Instant::now(),
            transcript,
            calls: resume
                .as_ref()
                .map(|state| state.calls.clone())
                .unwrap_or_default(),
            pending: resume.as_ref().and_then(|state| state.pending.clone()),
            reply,
            error_turns: 0,
            nudged: false,
            model: options.model.clone().unwrap_or_else(|| self.model.clone()),
            instructions: options
                .instructions
                .clone()
                .unwrap_or_else(|| self.instructions.clone()),
            extra: options.extra.clone(),
            output_schema: options.output_schema.clone(),
            max_steps: options.max_steps.or(self.limits.max_steps),
            max_seconds: options.max_seconds.or(self.limits.max_seconds),
            max_tokens: options.max_tokens.or(self.limits.max_tokens),
            trace: options.trace.unwrap_or(self.trace),
            context: options.context.clone(),
            tools,
            definitions,
            record: hooks.record.clone(),
            prepare: hooks.prepare.clone(),
            session_id: hooks.session_id.clone(),
            terminal: false,
        };
        let outcome = self.drive(&mut run, resume.is_some()).await;
        match outcome {
            Ok(result) => Ok(result),
            Err(error) => {
                if matches!(error, Error::Cancelled) {
                    run.emitter
                        .send(
                            EventKind::AgentEnd {
                                outcome: RunOutcome::Aborted { usage: run.usage },
                            },
                            None,
                        )
                        .await
                        .ok();
                    return Err(Error::Cancelled);
                }
                if run.terminal {
                    return Err(error);
                }
                run.emitter
                    .send(
                        EventKind::AgentEnd {
                            outcome: RunOutcome::Failed {
                                error: RunFailure::of(&error),
                                usage: run.usage,
                            },
                        },
                        None,
                    )
                    .await?;
                Err(error)
            }
        }
    }

    async fn drive(&self, run: &mut Run, resumed: bool) -> Result<crate::RunResult> {
        run.emitter
            .send(
                EventKind::AgentStart {
                    resumed: resumed.then_some(true),
                    repeated: None,
                },
                None,
            )
            .await?;
        loop {
            run.check_cancelled()?;
            if run.calls.is_empty() && run.reply.is_none() {
                run.check_limits()?;
                if let Some(prepare) = run.prepare.clone() {
                    let prepared = prepare(PrepareRequest {
                        history: run.transcript.clone(),
                        cancellation: run.cancellation.clone(),
                        emit: run.emitter.compaction_sink(),
                    })
                    .await?;
                    run.transcript = prepared.history;
                    if let Some(usage) = prepared.usage {
                        // The summary request is part of what this run cost.
                        run.usage.add(usage);
                    }
                    run.check_cancelled()?;
                }
                run.steps += 1;
                let turn = self.request(run).await?;
                let calls = turn.tool_calls.clone();
                let assistant = Message::Assistant {
                    text: turn.text.clone(),
                    tool_calls: calls.clone(),
                    replay: turn.replay,
                };
                run.transcript.push(assistant.clone());
                run.record(RecordBody::Item { item: assistant }).await?;
                run.calls = calls;
                if run.calls.is_empty() && run.output_schema.is_some() {
                    // The model answered in text. It is told once what is expected.
                    if run.nudged {
                        return Err(Error::Run(RunFailure::new(
                            "output_missing",
                            format!("the model did not call {OUTPUT_TOOL}"),
                        )));
                    }
                    run.nudged = true;
                    let reminder = Message::user(format!(
                        "Call the {OUTPUT_TOOL} tool with your final answer."
                    ));
                    run.transcript.push(reminder.clone());
                    run.record(RecordBody::Item { item: reminder }).await?;
                    continue;
                }
                if run.calls.is_empty() {
                    run.reply = Some(Reply {
                        text: turn.text,
                        output: turn.output,
                        value: None,
                    });
                }
            }
            if let Some(reply) = run.reply.clone() {
                let result = crate::RunResult {
                    text: reply.text.clone(),
                    history: run.transcript.clone(),
                    output: reply.output,
                    usage: run.usage,
                    value: reply.value.clone(),
                    pause: None,
                    repeated: None,
                };
                run.terminal = true;
                run.emitter
                    .send(
                        EventKind::AgentEnd {
                            outcome: RunOutcome::Completed {
                                text: reply.text,
                                usage: run.usage,
                                value: reply.value,
                                repeated: None,
                            },
                        },
                        None,
                    )
                    .await?;
                run.check_cancelled()?;
                return Ok(result);
            }
            let finishing = run.output_schema.is_some()
                && run.calls.iter().any(|call| call.name == OUTPUT_TOOL);
            let done = self.run_calls(run, finishing).await?;
            let accepted = match done {
                CallsDone::Continue { failures, accepted } => {
                    run.error_turns = if failures as usize == run.calls.len() {
                        run.error_turns + 1
                    } else {
                        0
                    };
                    accepted
                }
                CallsDone::Paused(pause) => {
                    let result = crate::RunResult {
                        text: String::new(),
                        history: run.transcript.clone(),
                        output: Vec::new(),
                        usage: run.usage,
                        value: None,
                        pause: Some(pause.clone()),
                        repeated: None,
                    };
                    run.terminal = true;
                    run.emitter
                        .send(
                            EventKind::AgentEnd {
                                outcome: RunOutcome::Paused {
                                    pause,
                                    usage: run.usage,
                                },
                            },
                            None,
                        )
                        .await?;
                    return Ok(result);
                }
            };
            run.calls.clear();
            if let Some(value) = accepted {
                run.reply = Some(Reply {
                    text: String::new(),
                    output: Vec::new(),
                    value: Some(value),
                });
            } else if run.error_turns >= MAX_ERROR_TURNS {
                return Err(Error::Run(RunFailure::new(
                    "repeated_tool_errors",
                    format!("every tool call failed in {MAX_ERROR_TURNS} model turns in a row"),
                )));
            }
        }
    }

    /// One model request: its deltas, then the complete turn.
    async fn request(&self, run: &mut Run) -> Result<ProviderTurn> {
        let cancellation = run.cancellation.clone();
        let mut attempt = 0;
        loop {
            attempt += 1;
            if run.trace {
                run.emitter
                    .send(EventKind::ModelRequestStart { attempt }, None)
                    .await?;
            }
            let request = ProviderRequest {
                model: run.model.clone(),
                instructions: run.instructions.clone(),
                history: run.transcript.clone(),
                tools: run.definitions.clone(),
                signal: cancellation.clone(),
                extra: run.extra.clone(),
            };
            let usage: Option<TokenUsage>;
            let opened = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(Error::Cancelled),
                opened = self.provider.stream(request) => opened,
            };
            let stream = match opened {
                Ok(stream) => stream,
                Err(error) => {
                    self.retry(run, error, false, attempt).await?;
                    continue;
                }
            };
            match Self::consume(stream, &run.emitter, run.trace, &cancellation).await {
                Ok((turn, reported, streamed)) => {
                    usage = reported;
                    if run.trace {
                        run.emitter
                            .send(
                                EventKind::ModelRequestEnd {
                                    usage: Some(usage.unwrap_or_default()),
                                    error: None,
                                    will_retry: None,
                                },
                                None,
                            )
                            .await?;
                    }
                    let _ = streamed;
                    run.usage.requests += 1;
                    run.usage.add_tokens(usage.unwrap_or_default());
                    return Ok(turn);
                }
                Err((error, streamed)) => self.retry(run, error, streamed, attempt).await?,
            }
        }
    }

    /// Read one provider stream to its turn, emitting deltas as they arrive.
    /// The error carries whether any of the response had arrived.
    async fn consume(
        mut stream: crate::ProviderStream,
        emitter: &Emitter,
        trace: bool,
        cancellation: &CancellationToken,
    ) -> std::result::Result<(ProviderTurn, Option<TokenUsage>, bool), (Error, bool)> {
        let mut streamed = false;
        let mut usage: Option<TokenUsage> = None;
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err((Error::Cancelled, streamed)),
                next = stream.next() => next,
            };
            match next {
                Some(Ok(ProviderEvent::TextDelta(delta))) => {
                    streamed = true;
                    if let Err(error) = emitter.send(EventKind::TextDelta { delta }, None).await {
                        return Err((error, streamed));
                    }
                }
                Some(Ok(ProviderEvent::ReasoningDelta(delta))) => {
                    streamed = true;
                    if trace {
                        if let Err(error) = emitter
                            .send(EventKind::ReasoningDelta { delta }, None)
                            .await
                        {
                            return Err((error, streamed));
                        }
                    }
                }
                Some(Ok(ProviderEvent::Usage(reported))) => usage = Some(reported),
                Some(Ok(ProviderEvent::Turn(turn))) => return Ok((turn, usage, streamed)),
                Some(Err(error)) => return Err((error, streamed)),
                None => {
                    return Err((
                        Error::Provider(
                            "the provider ended its stream without a complete turn".into(),
                        ),
                        streamed,
                    ))
                }
            }
        }
    }

    /// Sleep and let the caller send the request again, or answer with the error.
    async fn retry(&self, run: &mut Run, error: Error, streamed: bool, attempt: u32) -> Result<()> {
        let cancelled = run.cancellation.is_cancelled();
        let will_retry = matches!(error, Error::ProviderRetry { .. })
            && !streamed
            && attempt < MAX_ATTEMPTS
            && !cancelled;
        if run.trace {
            run.emitter
                .send(
                    EventKind::ModelRequestEnd {
                        usage: None,
                        error: Some(EventError {
                            code: if cancelled {
                                "aborted".into()
                            } else {
                                error.code().into()
                            },
                            message: if cancelled {
                                "run aborted".into()
                            } else {
                                error.to_string()
                            },
                        }),
                        will_retry: Some(will_retry),
                    },
                    None,
                )
                .await?;
        }
        if !will_retry {
            return Err(error);
        }
        let delay = match &error {
            Error::ProviderRetry {
                retry_after: Some(after),
                ..
            } => (*after).min(MAX_RETRY_DELAY),
            _ => Duration::from_millis(500 * u64::from(attempt)).min(MAX_RETRY_DELAY),
        };
        let cancellation = run.cancellation.clone();
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(Error::Cancelled),
            _ = tokio::time::sleep(delay) => {}
        }
        run.check_cancelled()
    }

    /// Run the tool calls of one model turn.
    async fn run_calls(&self, run: &mut Run, finishing: bool) -> Result<CallsDone> {
        let calls = run.calls.clone();
        let mut failures = 0;
        let mut accepted: Option<Value> = None;
        for call in calls {
            let tool = run.tools.get(&call.name).cloned();
            let interrupted = run.pending.take();
            let answer = interrupted
                .as_ref()
                .and_then(|pending| pending.answer.clone());
            let rerun = interrupted.as_ref().is_some_and(|pending| pending.rerun);
            let was_interrupted = interrupted.is_some();
            let intent = match &interrupted {
                Some(pending) => Some(pending.sequence),
                None => {
                    run.record(RecordBody::ToolIntent {
                        call_id: call.call_id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        idempotent: tool.as_ref().is_some_and(|tool| tool.idempotent()),
                    })
                    .await?
                }
            };
            // Rerunning a call, or answering a paused one, is journaled first, so a
            // writer that has been overtaken stops here.
            let again = if was_interrupted {
                if rerun {
                    run.record(RecordBody::ToolRerun {
                        call_id: call.call_id.clone(),
                        tool_name: call.name.clone(),
                    })
                    .await?
                } else if answer.is_some() {
                    run.record(RecordBody::ToolResumed {
                        call_id: call.call_id.clone(),
                        tool_name: call.name.clone(),
                    })
                    .await?
                } else {
                    None
                }
            } else {
                None
            };
            let execution_id = match (intent, &run.session_id) {
                (Some(sequence), Some(session_id)) => format!("{session_id}:{sequence}"),
                (Some(sequence), None) => sequence.to_string(),
                (None, _) => call.call_id.clone(),
            };
            let context = ToolContext::new(
                execution_id.clone(),
                run.cancellation.clone(),
                run.context.clone(),
            );
            run.emitter
                .send(
                    EventKind::ToolExecutionStart {
                        tool_call_id: call.call_id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        execution_id,
                        rerun: rerun.then_some(true),
                        interrupted: (was_interrupted && !rerun && answer.is_none())
                            .then_some(true),
                        resumed: answer.is_some().then_some(true),
                    },
                    again.or(intent),
                )
                .await?;
            let outcome = self
                .attempt(CallInput {
                    call: &call,
                    tool: tool.as_ref(),
                    answer: answer.as_ref(),
                    interrupted: was_interrupted,
                    rerun,
                    finishing,
                    context: &context,
                })
                .await?;
            if outcome.is_error {
                failures += 1;
            }
            let observation = Message::tool_result(
                call.call_id.clone(),
                call.name.clone(),
                outcome.output.clone(),
                outcome.is_error,
            );
            let value =
                if call.name == OUTPUT_TOOL && run.output_schema.is_some() && !outcome.is_error {
                    Some(outcome.arguments.clone())
                } else {
                    None
                };
            let written = if let Some(payload) = &outcome.paused {
                run.record(RecordBody::ToolPaused {
                    call_id: call.call_id.clone(),
                    tool_name: call.name.clone(),
                    payload: payload.clone(),
                })
                .await
            } else {
                run.record(RecordBody::ToolResult {
                    item: observation.clone(),
                    value: value.clone(),
                    interrupted: false,
                })
                .await
            };
            // The end event is owed to whoever saw the start, also when the
            // journal refuses the result.
            let cursor = match written {
                Ok(written) => written,
                Err(error) => {
                    run.emitter
                        .send(
                            EventKind::ToolExecutionEnd {
                                tool_call_id: call.call_id.clone(),
                                tool_name: call.name.clone(),
                                output: outcome.output.clone(),
                                is_error: outcome.is_error,
                                paused: outcome.paused.is_some().then_some(true),
                            },
                            None,
                        )
                        .await?;
                    return Err(outcome.fatal.unwrap_or(error));
                }
            };
            run.emitter
                .send(
                    EventKind::ToolExecutionEnd {
                        tool_call_id: call.call_id.clone(),
                        tool_name: call.name.clone(),
                        output: outcome.output.clone(),
                        is_error: outcome.is_error,
                        paused: outcome.paused.is_some().then_some(true),
                    },
                    cursor,
                )
                .await?;
            if let Some(error) = outcome.fatal {
                return Err(error);
            }
            if let Some(payload) = outcome.paused {
                return Ok(CallsDone::Paused(PendingPause {
                    tool_call_id: call.call_id.clone(),
                    tool_name: call.name.clone(),
                    payload,
                }));
            }
            if value.is_some() {
                accepted = value;
            }
            run.transcript.push(observation);
        }
        Ok(CallsDone::Continue { failures, accepted })
    }

    /// One tool call: hooks, then the handler, then `afterTool`.
    async fn attempt(&self, input: CallInput<'_>) -> Result<CallOutcome> {
        let call = input.call;
        let context = input.context;
        let mut outcome = CallOutcome {
            output: String::new(),
            arguments: call.arguments.clone(),
            is_error: false,
            paused: None,
            fatal: None,
        };
        if context.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if let Some(ResumeAnswer::Error(message)) = input.answer {
            return Ok(outcome.failed(Error::Tool(message.clone())));
        }
        if input.interrupted && !input.rerun && input.answer.is_none() {
            return Ok(outcome.failed(Error::Tool(INTERRUPTED.into())));
        }
        if input.finishing && call.name != OUTPUT_TOOL {
            return Ok(outcome.failed(Error::Tool("not run: the run finished".into())));
        }
        if let Some(answer) = input.answer {
            let ResumeAnswer::Result(value) = answer else {
                unreachable!("an error answer is handled above");
            };
            outcome.output = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
        } else {
            let Some(tool) = input.tool else {
                return Ok(outcome.failed(Error::Tool(format!("Unknown tool: {}", call.name))));
            };
            if let Err(error) = tool.check(&call.arguments) {
                return Ok(outcome.failed(error));
            }
            // A paused call that has been answered is settled: its hooks do not decide again.
            for hook in [tool.before_tool(), self.before_tool.as_ref()]
                .into_iter()
                .flatten()
            {
                let decision = match hook(
                    ToolCall {
                        call_id: call.call_id.clone(),
                        name: call.name.clone(),
                        arguments: outcome.arguments.clone(),
                    },
                    context.clone(),
                )
                .await
                {
                    Ok(decision) => decision,
                    Err(_) if context.cancellation.is_cancelled() => return Err(Error::Cancelled),
                    Err(error) => return Ok(outcome.failed(hook_error(error))),
                };
                match decision {
                    ToolDecision::Allow => {}
                    ToolDecision::Deny(reason) => {
                        return Ok(outcome.failed(Error::Tool(reason)));
                    }
                    ToolDecision::Replace(arguments) => outcome.arguments = arguments,
                    ToolDecision::Pause(payload) => {
                        outcome.paused = Some(payload);
                        break;
                    }
                }
            }
            if outcome.paused.is_none() {
                let decided = ToolCall {
                    call_id: call.call_id.clone(),
                    name: call.name.clone(),
                    arguments: outcome.arguments.clone(),
                };
                match tool.execute(&decided, context.clone()).await {
                    Ok(ToolReturn::Output(text)) => outcome.output = text,
                    Ok(ToolReturn::Pause(payload)) => outcome.paused = Some(payload),
                    Err(Error::Cancelled) if context.cancellation.is_cancelled() => {
                        return Err(Error::Cancelled)
                    }
                    Err(error) => return Ok(outcome.failed(error)),
                }
            }
        }
        if outcome.paused.is_some() {
            outcome.output = String::new();
        } else if output_too_large(&outcome.output) {
            // The model is told, rather than handed a truncated result it cannot trust.
            outcome.output = format!(
                "ToolOutputLimitError: tool output exceeds {MAX_TOOL_OUTPUT_BYTES} UTF-8 bytes"
            );
            outcome.is_error = true;
        }
        if outcome.paused.is_none() && outcome.fatal.is_none() {
            if let Some(after) = self.after_tool.as_ref() {
                let decided = ToolCall {
                    call_id: call.call_id.clone(),
                    name: call.name.clone(),
                    arguments: outcome.arguments.clone(),
                };
                let result = ToolResult {
                    output: outcome.output.clone(),
                    is_error: outcome.is_error,
                };
                match after(decided, result, context.clone()).await {
                    Ok(AfterToolOutcome::Keep) => {}
                    Ok(AfterToolOutcome::Replace { output, is_error }) => {
                        if output_too_large(&output) {
                            return Ok(outcome.failed(hook_error(Error::Run(
                                RunFailure::new(
                                    "hook_error",
                                    format!(
                                        "afterTool must return an output within {MAX_TOOL_OUTPUT_BYTES} UTF-8 bytes"
                                    ),
                                ),
                            ))));
                        }
                        outcome.output = output;
                        outcome.is_error = is_error.unwrap_or(outcome.is_error);
                    }
                    Err(_) if context.cancellation.is_cancelled() => return Err(Error::Cancelled),
                    Err(error) => {
                        outcome.output = error.to_string();
                        outcome.is_error = true;
                        outcome.fatal = Some(hook_error(error));
                    }
                }
            }
        }
        Ok(outcome)
    }
}

fn hook_error(error: Error) -> Error {
    match error {
        Error::Run(_) => error,
        other => Error::Run(RunFailure::new(
            "hook_error",
            format!("tool hook failed: {other}"),
        )),
    }
}

fn output_too_large(output: &str) -> bool {
    output.len() > MAX_TOOL_OUTPUT_BYTES
}

fn output_tool(schema: &Value) -> Result<Tool> {
    Tool::builder(ToolDefinition {
        name: OUTPUT_TOOL.into(),
        description: "Give the final answer. Call this exactly once, when the task is complete."
            .into(),
        parameters: schema.clone(),
    })
    .handler(|_arguments, _context| async { Ok(ToolReturn::Output("Output accepted.".into())) })
}

enum CallsDone {
    Continue {
        failures: u32,
        accepted: Option<Value>,
    },
    Paused(PendingPause),
}

#[derive(Clone)]
struct Reply {
    text: String,
    output: Vec<Value>,
    value: Option<Value>,
}

struct Run {
    emitter: Emitter,
    cancellation: CancellationToken,
    usage: Usage,
    steps: u32,
    started: Instant,
    transcript: Vec<Message>,
    calls: Vec<ToolCall>,
    pending: Option<PendingCall>,
    reply: Option<Reply>,
    error_turns: u32,
    nudged: bool,
    model: String,
    instructions: String,
    extra: Option<Value>,
    output_schema: Option<Value>,
    max_steps: Option<u32>,
    max_seconds: Option<f64>,
    max_tokens: Option<u64>,
    trace: bool,
    context: Value,
    tools: BTreeMap<String, Arc<Tool>>,
    definitions: Vec<ToolDefinition>,
    record: Option<RecordFn>,
    prepare: Option<PrepareFn>,
    session_id: Option<String>,
    terminal: bool,
}

impl Run {
    fn check_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    fn check_limits(&self) -> Result<()> {
        if let Some(max) = self.max_steps {
            if self.steps >= max {
                return Err(Error::Run(RunFailure::limit(
                    Limit::Steps,
                    format!("run exceeded maxSteps ({max})"),
                )));
            }
        }
        if let Some(max) = self.max_seconds {
            if self.started.elapsed().as_secs_f64() >= max {
                return Err(Error::Run(RunFailure::limit(
                    Limit::Seconds,
                    format!("run exceeded maxSeconds ({max})"),
                )));
            }
        }
        if let Some(max) = self.max_tokens {
            if self.usage.tokens() >= max {
                return Err(Error::Run(RunFailure::limit(
                    Limit::Tokens,
                    format!("run exceeded maxTokens ({max})"),
                )));
            }
        }
        Ok(())
    }

    /// Journal one record. A cancellation that arrived while it was written is noticed here.
    async fn record(&mut self, body: RecordBody) -> Result<Option<u64>> {
        let Some(record) = self.record.clone() else {
            return Ok(None);
        };
        let written = record(body).await?;
        self.check_cancelled()?;
        Ok(written)
    }
}

/// A clonable event sink with the run's sequence counter.
#[derive(Clone)]
struct Emitter {
    run_id: String,
    sequence: Arc<AtomicU64>,
    emit: EventFn,
}

impl Emitter {
    async fn send(&self, kind: EventKind, cursor: Option<u64>) -> Result<()> {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        (self.emit)(AgentEvent {
            run_id: self.run_id.clone(),
            sequence,
            cursor,
            kind,
        })
        .await
    }

    fn compaction_sink(&self) -> CompactionEmitter {
        let emitter = self.clone();
        Arc::new(move |kind| {
            let emitter = emitter.clone();
            Box::pin(async move { emitter.send(kind, None).await })
        })
    }
}

struct CallInput<'a> {
    call: &'a ToolCall,
    tool: Option<&'a Arc<Tool>>,
    answer: Option<&'a ResumeAnswer>,
    interrupted: bool,
    rerun: bool,
    finishing: bool,
    context: &'a ToolContext,
}

struct CallOutcome {
    output: String,
    arguments: Value,
    is_error: bool,
    paused: Option<Value>,
    fatal: Option<Error>,
}

impl CallOutcome {
    /// A call that failed in a way the model is told about.
    fn failed(mut self, error: Error) -> Self {
        self.output = match &error {
            Error::Tool(message) => message.clone(),
            other => format!("{}: {other}", other.kind_name()),
        };
        if output_too_large(&self.output) {
            self.output = format!(
                "ToolOutputLimitError: tool error exceeds {MAX_TOOL_OUTPUT_BYTES} UTF-8 bytes"
            );
        }
        self.is_error = true;
        self
    }
}

/// Builds an [`Agent`] with the limits, hooks and tracing a host wants.
pub struct AgentBuilder<P> {
    config: AgentConfig<P>,
    limits: RunLimits,
    before_tool: Option<BeforeTool>,
    after_tool: Option<AfterTool>,
    trace: bool,
    new_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl<P: Provider> AgentBuilder<P> {
    pub fn limits(mut self, limits: RunLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn max_steps(mut self, steps: u32) -> Self {
        self.limits.max_steps = Some(steps);
        self
    }

    pub fn max_seconds(mut self, seconds: f64) -> Self {
        self.limits.max_seconds = Some(seconds);
        self
    }

    pub fn max_tokens(mut self, tokens: u64) -> Self {
        self.limits.max_tokens = Some(tokens);
        self
    }

    /// Runs before every tool call, after the tool's own hook and its argument check.
    pub fn before_tool(mut self, hook: BeforeTool) -> Self {
        self.before_tool = Some(hook);
        self
    }

    /// Runs after every tool call that ran, and may replace its result.
    pub fn after_tool(mut self, hook: AfterTool) -> Self {
        self.after_tool = Some(hook);
        self
    }

    pub fn trace(mut self, trace: bool) -> Self {
        self.trace = trace;
        self
    }

    /// The random part of run and session IDs. Default: a UUID without dashes.
    pub fn new_id(mut self, new_id: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.new_id = Some(Arc::new(new_id));
        self
    }

    pub fn build(self) -> Result<Agent<P>> {
        if self.config.model.is_empty() {
            return Err(Error::Configuration("Model is required".into()));
        }
        let mut tools = BTreeMap::new();
        let mut definitions = Vec::new();
        for tool in self.config.tools {
            let definition = tool.definition();
            if tools
                .insert(definition.name.clone(), Arc::new(tool))
                .is_some()
            {
                return Err(Error::Configuration(format!(
                    "Duplicate tool: {}",
                    definition.name
                )));
            }
            definitions.push(definition);
        }
        Ok(Agent {
            model: self.config.model,
            instructions: self.config.instructions,
            provider: self.config.provider,
            tools,
            definitions,
            limits: self.limits,
            before_tool: self.before_tool,
            after_tool: self.after_tool,
            trace: self.trace,
            new_id: self.new_id,
        })
    }
}
