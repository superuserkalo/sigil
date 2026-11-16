//! One TinyAgent session at a time, one active run at a time. The host is the
//! only writer of its session directory. The protocol is in `README.md`.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::{Map, Value, json};
use tiny_agent::{
    Agent, AgentEvent, CancellationToken, Error, EventKind, PendingPause, ResumeAnswer, RunOptions,
    Session, SessionOptions, Usage,
};
use tokio::task::JoinHandle;

use crate::{Writer, error_envelope, log, provider::AnyProvider};

/// How long a cancelled run is given to journal its settlement.
const STOP_GRACE: Duration = Duration::from_secs(10);
const MAX_MESSAGE_CHARS: usize = 4_000;

pub struct Host {
    writer: Writer,
    agent: Arc<Agent<AnyProvider>>,
    session_dir: PathBuf,
    current: Option<Arc<Session<AnyProvider>>>,
    active: Option<ActiveRun>,
    closed: bool,
}

struct ActiveRun {
    cancellation: CancellationToken,
    handle: Option<JoinHandle<()>>,
    finished: Arc<AtomicBool>,
    fatal: Arc<AtomicBool>,
    run_id: Arc<StdMutex<Option<String>>>,
}

enum RunKind {
    Prompt(String),
    Resume {
        result: Option<Value>,
        error: Option<String>,
    },
}

struct HostError {
    code: String,
    message: String,
}

impl HostError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl Host {
    pub fn new(writer: Writer, agent: Arc<Agent<AnyProvider>>, session_dir: PathBuf) -> Self {
        Self {
            writer,
            agent,
            session_dir,
            current: None,
            active: None,
            closed: false,
        }
    }

    /// Handle one JSONL line. Returns false when the host must stop.
    pub async fn dispatch(&mut self, line: &str) -> bool {
        let parsed: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                self.send_error(None, "invalid_json", "Input line is not valid JSON.")
                    .await;
                return true;
            }
        };

        let Some(command) = parsed.as_object() else {
            self.send_error(None, "invalid_command", "A command must be a JSON object.")
                .await;
            return true;
        };

        let id = match command.get("id") {
            None => None,
            Some(Value::String(id)) => Some(id.clone()),
            Some(_) => {
                self.send_error(None, "invalid_command", "id must be a string.")
                    .await;
                return true;
            }
        };

        let kind = command
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        self.reap();

        match kind.as_str() {
            "prompt" => {
                self.prompt(id, command).await;
                true
            }
            "resume" => {
                self.resume(id, command).await;
                true
            }
            "abort" => {
                self.abort(id).await;
                true
            }
            "state" => {
                self.state(id).await;
                true
            }
            "list_sessions" => {
                self.list_sessions(id).await;
                true
            }
            "new_session" => {
                self.new_session(id).await;
                true
            }
            "shutdown" => {
                self.shutdown(id).await;
                false
            }
            other => {
                self.send_error(
                    id,
                    "invalid_command",
                    &format!("Unknown command type \"{other}\"."),
                )
                .await;
                true
            }
        }
    }

    async fn prompt(&mut self, id: Option<String>, command: &Map<String, Value>) {
        let text = match command.get("text").and_then(Value::as_str) {
            Some(text) if !text.trim().is_empty() => text.to_string(),
            _ => {
                self.send_error(
                    id,
                    "invalid_command",
                    "prompt needs a non-empty text string.",
                )
                .await;
                return;
            }
        };

        let requested = match requested_session(command.get("sessionId")) {
            Ok(requested) => requested,
            Err(error) => {
                self.send_error(id, &error.code, &error.message).await;
                return;
            }
        };

        if self.closed {
            self.send_error(id, "session_closed", "The host is shutting down.")
                .await;
            return;
        }
        if self.active.is_some() {
            self.send_error(id, "busy", "A run is already active. Send abort first.")
                .await;
            return;
        }

        let session = match self.open(requested.as_deref()).await {
            Ok(session) => session,
            Err(error) => {
                self.send_error(id, &error.code, &error.message).await;
                return;
            }
        };

        if let Some(error) = refuse_open(&session) {
            self.send_error(id, &error.code, &error.message).await;
            return;
        }

        self.start_run(id, session, RunKind::Prompt(text)).await;
    }

    async fn resume(&mut self, id: Option<String>, command: &Map<String, Value>) {
        let has_result = command.contains_key("result");
        let has_error = command.contains_key("error");

        if has_result && has_error {
            self.send_error(
                id,
                "invalid_command",
                "Send either result or error, not both.",
            )
            .await;
            return;
        }

        let error = match command.get("error") {
            None => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => {
                self.send_error(id, "invalid_command", "error must be a string.")
                    .await;
                return;
            }
        };

        let requested = match requested_session(command.get("sessionId")) {
            Ok(requested) => requested,
            Err(error) => {
                self.send_error(id, &error.code, &error.message).await;
                return;
            }
        };

        if self.closed {
            self.send_error(id, "session_closed", "The host is shutting down.")
                .await;
            return;
        }
        if self.active.is_some() {
            self.send_error(id, "busy", "A run is already active. Send abort first.")
                .await;
            return;
        }
        if requested.is_none() && self.current.is_none() {
            self.send_error(
                id,
                "nothing_to_resume",
                "The host has no open session to resume.",
            )
            .await;
            return;
        }

        let session = match self.open(requested.as_deref()).await {
            Ok(session) => session,
            Err(error) => {
                self.send_error(id, &error.code, &error.message).await;
                return;
            }
        };

        if session.recovery_required() {
            self.send_error(
                id,
                "recovery_required",
                "The session has an interrupted tool call that needs reconciliation. This host does not expose reconcile.",
            )
            .await;
            return;
        }

        if let Some(pause) = session.paused() {
            if !has_result && !has_error {
                self.send_error(
                    id,
                    "invalid_command",
                    &format!(
                        "The run is paused on {}. Provide result or error.",
                        pause.tool_name
                    ),
                )
                .await;
                return;
            }
        } else if session.resumable() {
            if has_result || has_error {
                self.send_error(
                    id,
                    "invalid_command",
                    "The session has no paused tool call, so result and error are not accepted.",
                )
                .await;
                return;
            }
        } else {
            self.send_error(
                id,
                "nothing_to_resume",
                "The session has no interrupted or paused run.",
            )
            .await;
            return;
        }

        let result = command.get("result").cloned();
        self.start_run(id, session, RunKind::Resume { result, error })
            .await;
    }

    async fn abort(&mut self, id: Option<String>) {
        let session_id = self
            .current
            .as_ref()
            .map(|session| session.id().to_string());
        let usage = self.current.as_ref().and_then(|session| session.usage());

        match &self.active {
            Some(active) => {
                let run_id = active.run_id.lock().ok().and_then(|slot| slot.clone());
                active.cancellation.cancel();
                self.writer
                    .send(&json!({
                        "id": id,
                        "ok": true,
                        "sessionId": session_id,
                        "text": "",
                        "pause": Value::Null,
                        "usage": usage,
                        "aborted": true,
                        "runId": run_id,
                    }))
                    .await;
            }
            None => {
                self.writer
                    .send(&json!({
                        "id": id,
                        "ok": true,
                        "sessionId": session_id,
                        "text": "",
                        "pause": Value::Null,
                        "usage": usage,
                        "aborted": false,
                        "runId": Value::Null,
                    }))
                    .await;
            }
        }
    }

    async fn state(&mut self, id: Option<String>) {
        let session = self.current.clone();
        let session_id = session.as_ref().map(|session| session.id().to_string());
        let usage: Option<Usage> = session.as_ref().and_then(|session| session.usage());
        let paused: Option<PendingPause> = session.as_ref().and_then(|session| session.paused());
        let pause_value = json!(paused);
        let message_count = session
            .as_ref()
            .map(|session| session.history().len())
            .unwrap_or(0);
        let resumable = session.as_ref().is_some_and(|session| session.resumable());
        let recovery_required = session
            .as_ref()
            .is_some_and(|session| session.recovery_required());
        let status = if self.active.is_some() {
            "running"
        } else {
            "idle"
        };
        let run_id = self
            .active
            .as_ref()
            .and_then(|active| active.run_id.lock().ok().and_then(|slot| slot.clone()));

        self.writer
            .send(&json!({
                "id": id,
                "ok": true,
                "sessionId": session_id,
                "text": "",
                "pause": pause_value.clone(),
                "usage": usage,
                "state": {
                    "status": status,
                    "runId": run_id,
                    "sessionId": session_id,
                    "messageCount": message_count,
                    "resumable": resumable,
                    "paused": pause_value,
                    "recoveryRequired": recovery_required,
                    "usage": usage,
                },
            }))
            .await;
    }

    async fn list_sessions(&mut self, id: Option<String>) {
        let session_id = self
            .current
            .as_ref()
            .map(|session| session.id().to_string());
        let usage = self.current.as_ref().and_then(|session| session.usage());

        match Session::<AnyProvider>::list(self.session_dir.clone()).await {
            Ok(session_ids) => {
                self.writer
                    .send(&json!({
                        "id": id,
                        "ok": true,
                        "sessionId": session_id,
                        "text": "",
                        "pause": Value::Null,
                        "usage": usage,
                        "sessionIds": session_ids,
                    }))
                    .await;
            }
            Err(error) => {
                let (code, message) = describe_error(&error);
                self.send_error(id, &code, &message).await;
            }
        }
    }

    async fn new_session(&mut self, id: Option<String>) {
        if self.closed {
            self.send_error(id, "session_closed", "The host is shutting down.")
                .await;
            return;
        }

        self.stop_run().await;
        let previous_session_id = self.close_current().await;

        let session = match Session::with_directory(
            self.agent.clone(),
            self.session_dir.clone(),
            SessionOptions::create(),
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                let (code, message) = describe_error(&error);
                self.send_error(id, &code, &message).await;
                return;
            }
        };

        let session_id = session.id().to_string();
        self.current = Some(Arc::new(session));
        self.writer
            .send(&json!({
                "id": id,
                "ok": true,
                "sessionId": session_id,
                "text": "",
                "pause": Value::Null,
                "usage": Value::Null,
                "previousSessionId": previous_session_id,
            }))
            .await;
    }

    async fn shutdown(&mut self, id: Option<String>) {
        let session_id = self
            .current
            .as_ref()
            .map(|session| session.id().to_string());
        self.stop_run().await;
        self.close_current().await;
        self.closed = true;
        self.writer
            .send(&json!({
                "id": id,
                "ok": true,
                "sessionId": session_id,
                "text": "",
                "pause": Value::Null,
                "usage": Value::Null,
                "stopped": true,
            }))
            .await;
    }

    /// Release the session after stdin closed, without an envelope.
    pub async fn shutdown_quiet(&mut self) {
        self.stop_run().await;
        self.close_current().await;
        self.closed = true;
    }

    /// A finished run releases its slot. A fatal journal error discards the
    /// session object so the next command starts clean.
    fn reap(&mut self) {
        let finished = self
            .active
            .as_ref()
            .is_some_and(|active| active.finished.load(Ordering::SeqCst));
        if !finished {
            return;
        }
        let Some(active) = self.active.take() else {
            return;
        };
        if active.fatal.load(Ordering::SeqCst)
            && let Some(session) = self.current.take()
        {
            log(&format!(
                "discarding session {} after a fatal journal error",
                session.id()
            ));
        }
    }

    async fn open(
        &mut self,
        requested: Option<&str>,
    ) -> Result<Arc<Session<AnyProvider>>, HostError> {
        if let Some(current) = &self.current
            && (requested.is_none() || requested == Some(current.id()))
        {
            return Ok(current.clone());
        }

        if self.current.is_some() {
            self.close_current().await;
        }

        let created = match requested {
            Some(session_id) => {
                Session::with_directory(
                    self.agent.clone(),
                    self.session_dir.clone(),
                    SessionOptions::resume(session_id),
                )
                .await
            }
            None => {
                Session::with_directory(
                    self.agent.clone(),
                    self.session_dir.clone(),
                    SessionOptions::create(),
                )
                .await
            }
        };

        match created {
            Ok(session) => {
                let session = Arc::new(session);
                self.current = Some(session.clone());
                Ok(session)
            }
            Err(Error::Storage(message)) if message.starts_with("unknown session") => {
                Err(HostError::new(
                    "session_not_found",
                    format!("No stored session {}.", requested.unwrap_or_default()),
                ))
            }
            Err(error) => {
                let (code, message) = describe_error(&error);
                Err(HostError { code, message })
            }
        }
    }

    async fn close_current(&mut self) -> Option<String> {
        let session = self.current.take()?;
        let session_id = session.id().to_string();
        if let Err(error) = session.close().await {
            log(&format!("could not close session {session_id}: {error}"));
        }
        Some(session_id)
    }

    async fn stop_run(&mut self) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        active.cancellation.cancel();
        if let Some(handle) = active.handle.take() {
            let stopped = tokio::time::timeout(STOP_GRACE, handle).await;
            if stopped.is_err() {
                log("a run did not stop within the grace period");
            }
        }
    }

    async fn start_run(
        &mut self,
        id: Option<String>,
        session: Arc<Session<AnyProvider>>,
        kind: RunKind,
    ) {
        let cancellation = CancellationToken::new();
        let finished = Arc::new(AtomicBool::new(false));
        let fatal = Arc::new(AtomicBool::new(false));
        let run_id = Arc::new(StdMutex::new(None));
        let writer = self.writer.clone();
        let session_id = session.id().to_string();
        let token = cancellation.clone();
        let flag = finished.clone();
        let fatal_flag = fatal.clone();
        let live = run_id.clone();

        let handle = tokio::spawn(async move {
            let sink = {
                let writer = writer.clone();
                let live = live.clone();
                tiny_agent::event_sink(move |event: AgentEvent| {
                    let writer = writer.clone();
                    let live = live.clone();
                    async move {
                        if matches!(&event.kind, EventKind::AgentStart { .. })
                            && let Ok(mut slot) = live.lock()
                        {
                            *slot = Some(event.run_id.clone());
                        }
                        writer.send(&json!({ "event": event })).await;
                        Ok(())
                    }
                })
            };

            let options = RunOptions {
                cancellation: token,
                on_event: Some(sink),
                ..RunOptions::default()
            };

            let outcome = match kind {
                RunKind::Prompt(text) => session.run(&text, options).await,
                RunKind::Resume { result, error } => {
                    let answer = match (error, result) {
                        (Some(message), _) => Some(ResumeAnswer::Error(message)),
                        (None, Some(value)) => Some(ResumeAnswer::Result(value)),
                        (None, None) => None,
                    };
                    match answer {
                        Some(answer) => session.resume_with(answer, options).await,
                        None => session.resume(options).await,
                    }
                }
            };

            let envelope = match outcome {
                Ok(result) => json!({
                    "id": id,
                    "ok": true,
                    "sessionId": session_id,
                    "text": result.text,
                    "pause": result.pause,
                    "usage": result.usage,
                }),
                Err(error) => {
                    let (code, message) = describe_error(&error);
                    if is_session_fatal(&code) {
                        fatal_flag.store(true, Ordering::SeqCst);
                    }
                    error_envelope(id.as_deref(), &code, &message)
                }
            };

            writer.send(&envelope).await;
            flag.store(true, Ordering::SeqCst);
        });

        self.active = Some(ActiveRun {
            cancellation,
            handle: Some(handle),
            finished,
            fatal,
            run_id,
        });
    }

    async fn send_error(&self, id: Option<String>, code: &str, message: &str) {
        let message = truncate(message, MAX_MESSAGE_CHARS);
        self.writer
            .send(&error_envelope(id.as_deref(), code, &message))
            .await;
    }
}

fn refuse_open(session: &Session<AnyProvider>) -> Option<HostError> {
    if session.recovery_required() {
        return Some(HostError::new(
            "recovery_required",
            "The session has an interrupted tool call that needs reconciliation. This host does not expose reconcile.",
        ));
    }
    if let Some(pause) = session.paused() {
        return Some(HostError::new(
            "resume_required",
            format!(
                "Session {} is paused on {}. Send resume with a result or error.",
                session.id(),
                pause.tool_name
            ),
        ));
    }
    if session.resumable() {
        return Some(HostError::new(
            "resume_required",
            format!(
                "Session {} has an interrupted run. Send resume or new_session.",
                session.id()
            ),
        ));
    }
    None
}

fn requested_session(value: Option<&Value>) -> Result<Option<String>, HostError> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) if is_session_id(text) => Ok(Some(text.clone())),
        Some(_) => Err(HostError::new(
            "invalid_command",
            "sessionId must look like s_ followed by letters, digits, _ or -.",
        )),
    }
}

fn is_session_id(value: &str) -> bool {
    value.strip_prefix("s_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 100
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

fn describe_error(error: &Error) -> (String, String) {
    let message = truncate(&error.to_string(), MAX_MESSAGE_CHARS);
    let code = match error {
        Error::Cancelled => "aborted",
        Error::SessionBusy => "busy",
        Error::SessionClosed => "session_failed",
        Error::Storage(text) if text.starts_with("unknown session") => "session_not_found",
        Error::Storage(_) => "storage_error",
        Error::Fenced(_) => "session_fenced",
        Error::NothingToResume(_) => "nothing_to_resume",
        Error::RecoveryRequired(_) => "recovery_required",
        Error::ResumeRequired(_) => "resume_required",
        Error::Unsupported(_) => "unsupported",
        Error::Configuration(_) => "configuration_error",
        Error::Provider(_) | Error::ProviderRetry { .. } => "provider_error",
        Error::Tool(_) => "tool_error",
        Error::Consumer(_) => "internal_error",
        Error::Run(failure) => failure.code.as_str(),
    };
    (code.to_string(), message)
}

fn is_session_fatal(code: &str) -> bool {
    matches!(
        code,
        "session_failed" | "storage_error" | "session_fenced" | "session_corrupt"
    )
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        text.chars().take(limit).collect()
    }
}
