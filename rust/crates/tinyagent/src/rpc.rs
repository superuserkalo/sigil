use crate::{
    agent::Agent,
    lines::{LineError, Lines},
    session::{EventQuery, Session, SessionOptions},
    storage::{MemoryStorage, Storage},
    types::{
        AgentEvent, Error, EventFn, EventKind, Provider, Result, RunOptions, MAX_JSONL_LINE_BYTES,
    },
};
use serde_json::{json, Map, Value};
use std::{future::pending,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod output;

use output::{Receipt, Writer};

const DEFAULT_EVENT_LIMIT: u64 = 100;
const DEFAULT_HISTORY_LIMIT: u64 = 20;
const MAX_PAGE_LIMIT: u64 = 1_000;

fn protocol_error(code: &str, message: &str) -> Value {
    json!({"type":"protocol_error","error":{"code":code,"message":message}})
}

fn command_error(id: Option<&str>, command: &str, code: &str, message: &str) -> Value {
    match id {
        Some(id) => {
            json!({"id":id,"type":"response","command":command,"success":false,"error":{"code":code,"message":message}})
        }
        None => protocol_error(code, message),
    }
}

fn response(id: Option<&str>, command: &str, data: Value) -> Value {
    let mut response = json!({"type":"response","command":command,"success":true,"data":data});
    if let Some(id) = id {
        response["id"] = Value::String(id.into());
    }
    response
}

/// The command error code a session failure maps to.
fn error_code(error: &Error) -> &'static str {
    match error {
        Error::SessionBusy => "busy",
        Error::SessionClosed => "session_closed",
        Error::NothingToResume(_) => "nothing_to_resume",
        Error::Storage(_) => "storage_error",
        Error::Fenced(_) => "session_fenced",
        Error::Cancelled => "aborted",
        _ => "session_error",
    }
}

fn bounded_message(error: &Error) -> String {
    let message = error.to_string();
    if message.chars().count() <= 500 {
        message
    } else {
        message.chars().take(500).collect()
    }
}

/// A response whose serialized form exceeded the line limit is replaced by a
/// protocol error; the run it belongs to is cancelled.
fn enqueue(writer: &Writer, cancellation: &CancellationToken, value: Value) -> Result<Receipt> {
    let receipt = writer.enqueue(value)?;
    if receipt.oversized {
        cancellation.cancel();
    }
    Ok(receipt)
}

/// An optional non-negative integer command field.
fn number(command: &Map<String, Value>, field: &str, default: u64) -> Option<u64> {
    match command.get(field) {
        None => Some(default),
        Some(value) => value.as_u64(),
    }
}

fn page_limit(value: u64) -> Option<usize> {
    if (1..=MAX_PAGE_LIMIT).contains(&value) {
        usize::try_from(value).ok()
    } else {
        None
    }
}

fn busy_reply(
    writer: &Writer,
    cancellation: &CancellationToken,
    id: Option<&str>,
    command: &str,
) -> Result<Receipt> {
    enqueue(
        writer,
        cancellation,
        command_error(id, command, "busy", "A run is already active"),
    )
}

/// What the first event of a run answers: the command response, then the event.
#[derive(Clone)]
struct RunContext {
    writer: Arc<Writer>,
    cancellation: CancellationToken,
    command: &'static str,
    id: Option<String>,
    /// The response already answered the command; later events only stream.
    answered: Arc<Mutex<bool>>,
    /// The run ID the first event reported, for `get_state` and abort.
    run_id: Arc<Mutex<Option<String>>>,
    /// `prompt` reports `repeated`; `resume_run` and `compact` do not.
    report_repeated: bool,
}

impl RunContext {
    fn new(
        writer: Arc<Writer>,
        cancellation: CancellationToken,
        command: &'static str,
        id: Option<&str>,
        report_repeated: bool,
    ) -> Self {
        Self {
            writer,
            cancellation,
            command,
            id: id.map(str::to_owned),
            answered: Arc::new(Mutex::new(false)),
            run_id: Arc::new(Mutex::new(None)),
            report_repeated,
        }
    }

    fn answered(&self) -> bool {
        *self.answered.lock().unwrap()
    }

    fn run_id(&self) -> Option<String> {
        self.run_id.lock().unwrap().clone()
    }

    fn sink(&self) -> EventFn {
        let context = self.clone();
        Arc::new(move |event: AgentEvent| {
            let context = context.clone();
            Box::pin(async move {
                if let Ok(mut run_id) = context.run_id.lock() {
                    run_id.get_or_insert_with(|| event.run_id.clone());
                }
                let first = {
                    let mut answered = context.answered.lock().unwrap();
                    let first = !*answered;
                    *answered = true;
                    first
                };
                if first {
                    let repeated = context.report_repeated
                        && matches!(
                            &event.kind,
                            EventKind::AgentStart {
                                repeated: Some(true),
                                ..
                            }
                        );
                    let data = if context.report_repeated {
                        json!({"runId":event.run_id, "repeated":repeated})
                    } else {
                        json!({"runId":event.run_id})
                    };
                    let receipt = enqueue(
                        &context.writer,
                        &context.cancellation,
                        response(context.id.as_deref(), context.command, data),
                    )?;
                    context.writer.wait(receipt, &context.cancellation).await?;
                }
                let receipt = context.writer.enqueue(event)?;
                context.writer.wait(receipt, &context.cancellation).await
            })
        })
    }
}

/// Report a run that ended before it produced a first event.
async fn report_start_failure(
    context: &RunContext,
    fallback_run_id: Option<String>,
    result: &Result<()>,
) {
    if context.answered() {
        return;
    }
    let value = match result {
        Ok(_) => {
            let run_id = context
                .run_id()
                .or(fallback_run_id)
                .unwrap_or_else(|| format!("r_{}", Uuid::new_v4().simple()));
            let data = if context.report_repeated {
                json!({"runId":run_id, "repeated":false})
            } else {
                json!({"runId":run_id})
            };
            response(context.id.as_deref(), context.command, data)
        }
        Err(error) => command_error(
            context.id.as_deref(),
            context.command,
            error_code(error),
            &bounded_message(error),
        ),
    };
    // The run is over; a cancelled token must not stop this final answer.
    let _ = enqueue(&context.writer, &CancellationToken::new(), value);
}

/// Run one native session over bounded UTF-8 JSONL. The caller must consume
/// output concurrently. EOF and cancellation wait for cooperative run cleanup.
pub async fn run_rpc<P, R, W>(
    agent: Arc<Agent<P>>,
    input: R,
    output: W,
    cancellation: CancellationToken,
) -> Result<()>
where
    P: Provider + 'static,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let cancellation = cancellation.child_token();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let (writer, output_task) = Writer::start(output, cancellation.clone());
    let mut lines = Lines::new(input);
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let mut session = Arc::new(
        Session::new(agent.clone(), storage.clone(), SessionOptions::create()).await?,
    );
    let mut task: Option<JoinHandle<()>> = None;
    // True from the spawn until the run task is joined, so a racing command sees busy.
    let mut active = false;
    let mut active_token: Option<CancellationToken> = None;
    let mut active_run_id: Option<String> = None;
    let result = async {
        loop {
            let line = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                result = async {
                    match &mut task {
                        Some(task) => task.await,
                        None => pending().await,
                    }
                } => {
                    task = None;
                    active = false;
                    active_token = None;
                    active_run_id = None;
                    result.map_err(|error| Error::Consumer(error.to_string()))?;
                    continue;
                }
                line = lines.next() => line,
            };
            let line = match line {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    let response = match error {
                        LineError::TooLarge => protocol_error(
                            "line_too_large",
                            &format!("RPC input exceeds {MAX_JSONL_LINE_BYTES} UTF-8 bytes"),
                        ),
                        LineError::InvalidUtf8 => {
                            protocol_error("invalid_utf8", "RPC input is not valid UTF-8")
                        }
                        LineError::Io(error) => return Err(Error::Consumer(error.to_string())),
                    };
                    enqueue(&writer, &cancellation, response)?;
                    break;
                }
            };
            let command: Value = match serde_json::from_str(&line) {
                Ok(command) => command,
                Err(_) => {
                    enqueue(
                        &writer,
                        &cancellation,
                        protocol_error("invalid_json", "RPC input is not valid JSON"),
                    )?;
                    continue;
                }
            };
            let Some(command) = command.as_object() else {
                enqueue(
                    &writer,
                    &cancellation,
                    protocol_error("invalid_command", "RPC command must be an object"),
                )?;
                continue;
            };
            let id = match command.get("id") {
                Some(Value::String(id)) => Some(id.as_str()),
                None => None,
                _ => {
                    enqueue(
                        &writer,
                        &cancellation,
                        protocol_error("invalid_command", "command.id must be a string"),
                    )?;
                    continue;
                }
            };
            let kind = command
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let busy = active || session.busy();
            match kind {
                "prompt" => {
                    let Some(message) = command
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|message| !message.is_empty())
                    else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "prompt.message must be a non-empty string",
                            ),
                        )?;
                        continue;
                    };
                    let message_id = match command.get("messageId") {
                        None => None,
                        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                        Some(_) => {
                            enqueue(
                                &writer,
                                &cancellation,
                                command_error(
                                    id,
                                    kind,
                                    "invalid_command",
                                    "messageId must be a non-empty string",
                                ),
                            )?;
                            continue;
                        }
                    };
                    let message = message.to_owned();
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    let run_id = format!("r_{}", Uuid::new_v4().simple());
                    let token = cancellation.child_token();
                    let context = RunContext::new(
                        writer.clone(),
                        token.clone(),
                        "prompt",
                        id,
                        true,
                    );
                    let options = RunOptions {
                        run_id: Some(run_id.clone()),
                        cancellation: token.clone(),
                        message_id,
                        on_event: Some(context.sink()),
                        ..RunOptions::default()
                    };
                    let session = session.clone();
                    active = true;
                    active_token = Some(token);
                    active_run_id = Some(run_id.clone());
                    task = Some(tokio::spawn(async move {
                        let result = session.run(&message, options).await.map(|_| ());
                        report_start_failure(&context, Some(run_id), &result).await;
                    }));
                }
                "resume_run" => {
                    if command.get("result").is_some() && command.get("error").is_some() {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "resume_run accepts a result or an error, not both",
                            ),
                        )?;
                        continue;
                    }
                    let error = match command.get("error") {
                        None => None,
                        Some(Value::String(message)) if !message.is_empty() => {
                            Some(message.clone())
                        }
                        Some(_) => {
                            enqueue(
                                &writer,
                                &cancellation,
                                command_error(
                                    id,
                                    kind,
                                    "invalid_command",
                                    "resume_run.error must be a non-empty string",
                                ),
                            )?;
                            continue;
                        }
                    };
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    if !session.resumable() && !session.recovery_required() {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "nothing_to_resume",
                                "session has no interrupted run",
                            ),
                        )?;
                        continue;
                    }
                    let token = cancellation.child_token();
                    let context =
                        RunContext::new(writer.clone(), token.clone(), "resume_run", id, false);
                    let options = RunOptions {
                        cancellation: token.clone(),
                        result: command.get("result").cloned(),
                        error,
                        on_event: Some(context.sink()),
                        ..RunOptions::default()
                    };
                    let session = session.clone();
                    active = true;
                    active_token = Some(token);
                    task = Some(tokio::spawn(async move {
                        let result = session.resume(options).await.map(|_| ());
                        report_start_failure(&context, None, &result).await;
                    }));
                }
                "abandon_run" => {
                    let reason = match command.get("reason") {
                        None => "abandoned".to_owned(),
                        Some(Value::String(reason)) => reason.clone(),
                        Some(_) => {
                            enqueue(
                                &writer,
                                &cancellation,
                                command_error(
                                    id,
                                    kind,
                                    "invalid_command",
                                    "abandon_run.reason must be a string",
                                ),
                            )?;
                            continue;
                        }
                    };
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    match session.abandon(&reason).await {
                        Ok(()) => enqueue(
                            &writer,
                            &cancellation,
                            response(id, kind, json!({"abandoned":true})),
                        )?,
                        Err(error) => enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                error_code(&error),
                                &bounded_message(&error),
                            ),
                        )?,
                    };
                }
                "abort" => {
                    let accepted = active && !session.failed();
                    if accepted {
                        if let Some(token) = &active_token {
                            token.cancel();
                        }
                    }
                    enqueue(
                        &writer,
                        &cancellation,
                        response(
                            id,
                            kind,
                            json!({"accepted":accepted,"runId":active_run_id}),
                        ),
                    )?;
                }
                "compact" => {
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    if session.budget().is_none() {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "session_error",
                                "Compaction requires a context budget, which this RPC adapter does not configure",
                            ),
                        )?;
                        continue;
                    }
                    let token = cancellation.child_token();
                    let context =
                        RunContext::new(writer.clone(), token.clone(), "compact", id, false);
                    let options = RunOptions {
                        cancellation: token.clone(),
                        on_event: Some(context.sink()),
                        ..RunOptions::default()
                    };
                    let session = session.clone();
                    active = true;
                    active_token = Some(token);
                    task = Some(tokio::spawn(async move {
                        let result = session.compact(options).await.map(|_| ());
                        report_start_failure(&context, None, &result).await;
                    }));
                }
                "new_session" => {
                    let aborted = active || session.busy();
                    if let Some(token) = &active_token {
                        token.cancel();
                    }
                    session.close().await?;
                    if let Some(previous) = task.take() {
                        previous
                            .await
                            .map_err(|error| Error::Consumer(error.to_string()))?;
                    }
                    active = false;
                    active_token = None;
                    active_run_id = None;
                    session = Arc::new(
                        Session::new(agent.clone(), storage.clone(), SessionOptions::create())
                            .await?,
                    );
                    enqueue(
                        &writer,
                        &cancellation,
                        response(id, kind, json!({"abortedRun":aborted})),
                    )?;
                }
                "get_state" => {
                    let status = if active || session.busy() {
                        "running"
                    } else {
                        "idle"
                    };
                    enqueue(
                        &writer,
                        &cancellation,
                        response(
                            id,
                            kind,
                            json!({
                                "protocolVersion":crate::types::PROTOCOL_VERSION,
                                "status":status,
                                "runId":active_run_id,
                                "messageCount":session.history().len(),
                                "resumable":session.resumable(),
                                "paused":session.paused(),
                                "usage":session.usage(),
                            }),
                        ),
                    )?;
                }
                "get_events" => {
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    let Some(after) = number(command, "after", 0) else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "get_events.after must be a nonnegative integer",
                            ),
                        )?;
                        continue;
                    };
                    let Some(limit) =
                        number(command, "limit", DEFAULT_EVENT_LIMIT).and_then(page_limit)
                    else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "get_events.limit must be an integer from 1 to 1000",
                            ),
                        )?;
                        continue;
                    };
                    let page = session.events(EventQuery { after, limit });
                    enqueue(
                        &writer,
                        &cancellation,
                        response(
                            id,
                            kind,
                            json!({"events":page.events,"nextAfter":page.next_after}),
                        ),
                    )?;
                }
                "get_history" => {
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    let Some(offset) = number(command, "offset", 0) else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "get_history.offset must be a nonnegative integer",
                            ),
                        )?;
                        continue;
                    };
                    let Some(limit) =
                        number(command, "limit", DEFAULT_HISTORY_LIMIT).and_then(page_limit)
                    else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "get_history.limit must be an integer from 1 to 1000",
                            ),
                        )?;
                        continue;
                    };
                    let records = match command.get("view") {
                        None => false,
                        Some(Value::String(view)) if view == "history" => false,
                        Some(Value::String(view)) if view == "records" => true,
                        Some(_) => {
                            enqueue(
                                &writer,
                                &cancellation,
                                command_error(
                                    id,
                                    kind,
                                    "invalid_command",
                                    "get_history.view must be history or records",
                                ),
                            )?;
                            continue;
                        }
                    };
                    let items = if records {
                        serde_json::to_value(session.records())
                            .map_err(|error| Error::Consumer(error.to_string()))?
                    } else {
                        serde_json::to_value(session.history())
                            .map_err(|error| Error::Consumer(error.to_string()))?
                    };
                    let array = items.as_array().cloned().unwrap_or_default();
                    let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                    let start = offset.min(array.len());
                    let end = array.len().min(start.saturating_add(limit));
                    let page: Vec<Value> = array[start..end].to_vec();
                    let next_offset = (end < array.len()).then_some(end);
                    let mut data = json!({
                        "sessionId":session.id(),
                        "recoveryRequired":session.recovery_required(),
                        "resumable":session.resumable(),
                        "paused":session.paused(),
                        "nextOffset":next_offset,
                        "recoveryNotes":session.recovery_notes(),
                    });
                    data[if records { "records" } else { "history" }] = Value::Array(page);
                    enqueue(&writer, &cancellation, response(id, kind, data))?;
                }
                "list_sessions" => {
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    match storage.list().await {
                        Ok(mut session_ids) => {
                            if !session_ids.iter().any(|id| id == session.id()) {
                                session_ids.push(session.id().to_owned());
                            }
                            session_ids.sort();
                            enqueue(
                                &writer,
                                &cancellation,
                                response(id, kind, json!({"sessionIds":session_ids})),
                            )?;
                        }
                        Err(error) => {
                            enqueue(
                                &writer,
                                &cancellation,
                                command_error(
                                    id,
                                    kind,
                                    error_code(&error),
                                    &bounded_message(&error),
                                ),
                            )?;
                        }
                    }
                }
                "resume_session" => {
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    let Some(session_id) = command.get("sessionId").and_then(Value::as_str) else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "resume_session.sessionId must be a string",
                            ),
                        )?;
                        continue;
                    };
                    if session_id == session.id() {
                        enqueue(
                            &writer,
                            &cancellation,
                            response(id, kind, json!({"sessionId":session.id()})),
                        )?;
                        continue;
                    }
                    match Session::open(agent.clone(), storage.clone(), session_id).await {
                        Ok(opened) => {
                            session.close().await?;
                            session = Arc::new(opened);
                            enqueue(
                                &writer,
                                &cancellation,
                                response(id, kind, json!({"sessionId":session.id()})),
                            )?;
                        }
                        Err(error) => {
                            enqueue(
                                &writer,
                                &cancellation,
                                command_error(
                                    id,
                                    kind,
                                    error_code(&error),
                                    &bounded_message(&error),
                                ),
                            )?;
                        }
                    }
                }
                "reconcile" => {
                    if busy {
                        busy_reply(&writer, &cancellation, id, kind)?;
                        continue;
                    }
                    let Some(note) = command.get("note").and_then(Value::as_str) else {
                        enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                "invalid_command",
                                "reconcile.note must be a string",
                            ),
                        )?;
                        continue;
                    };
                    match session.reconcile(note).await {
                        Ok(()) => enqueue(
                            &writer,
                            &cancellation,
                            response(
                                id,
                                kind,
                                json!({"sessionId":session.id(),"recoveryRequired":false}),
                            ),
                        )?,
                        Err(error) => enqueue(
                            &writer,
                            &cancellation,
                            command_error(
                                id,
                                kind,
                                error_code(&error),
                                &bounded_message(&error),
                            ),
                        )?,
                    };
                }
                _ => {
                    enqueue(
                        &writer,
                        &cancellation,
                        command_error(id, kind, "invalid_command", "Unknown command type"),
                    )?;
                }
            }
        }
        Ok(())
    }
    .await;
    if let Some(token) = &active_token {
        token.cancel();
    }
    let _ = session.close().await;
    if let Some(task) = task {
        task.await
            .map_err(|error| Error::Consumer(error.to_string()))?;
    }
    writer.close(output_task).await?;
    result
}

#[cfg(test)]
mod tests;
