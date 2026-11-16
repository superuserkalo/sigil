mod body;
mod server;

pub use server::{serve, HttpOptions, HttpServer};

use crate::{
    session::{Session, SessionOptions},
    storage::{MemoryStorage, Storage},
    Agent, CancellationToken, Error, EventFn, Provider, RunFailure, RunOptions, MAX_HTTP_BODY_BYTES,
    PROTOCOL_VERSION,
};
use body::{send_event, Body, ConnectionContext};
use http_body_util::BodyExt;
use hyper::{
    body::Incoming,
    header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, WWW_AUTHENTICATE},
    Request, Response, StatusCode,
};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

struct HttpState<P: Provider> {
    agent: Arc<Agent<P>>,
    storage: Arc<dyn Storage>,
    authorization: Option<String>,
    sessions: Mutex<Sessions<P>>,
    /// The active run of each session, so another request can cancel it.
    runs: Mutex<BTreeMap<String, ActiveRun>>,
    cancellation: CancellationToken,
    tasks: TaskTracker,
}

#[derive(Clone)]
struct ActiveRun {
    token: CancellationToken,
    run_id: String,
}

struct Sessions<P: Provider> {
    values: BTreeMap<String, Arc<Session<P>>>,
    closing: bool,
}

impl<P: Provider> Default for Sessions<P> {
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            closing: false,
        }
    }
}

struct HttpError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl HttpError {
    fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    fn response(self) -> Response<Body> {
        let mut response = json_response(
            self.status,
            json!({"error":RunFailure {
                code: self.code.into(), message: self.message.into(),
                retryable: self.status == StatusCode::CONFLICT || self.status.is_server_error(),
                limit: None,
            }}),
        );
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, "Bearer".parse().unwrap());
        }
        response
    }
}

impl<P: Provider + 'static> HttpState<P> {
    fn new(agent: Arc<Agent<P>>, token: String) -> Self {
        Self {
            agent,
            storage: Arc::new(MemoryStorage::new()),
            authorization: if token.is_empty() {
                None
            } else {
                Some(format!("Bearer {token}"))
            },
            sessions: Mutex::new(Sessions::default()),
            runs: Mutex::new(BTreeMap::new()),
            cancellation: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }
    }

    async fn handle(
        self: Arc<Self>,
        request: Request<Incoming>,
        context: ConnectionContext,
    ) -> Response<Body> {
        self.dispatch(request, context)
            .await
            .unwrap_or_else(HttpError::response)
    }

    async fn dispatch(
        self: &Arc<Self>,
        request: Request<Incoming>,
        context: ConnectionContext,
    ) -> std::result::Result<Response<Body>, HttpError> {
        if let Some(expected) = &self.authorization {
            let mut values = request.headers().get_all(AUTHORIZATION).iter();
            let actual = values.next().map_or(&b""[..], |value| value.as_bytes());
            if values.next().is_some() || !bool::from(actual.ct_eq(expected.as_bytes())) {
                return Err(HttpError::new(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "Missing or invalid bearer token",
                ));
            }
        }
        if self.sessions.lock().unwrap().closing {
            return Err(HttpError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_closed",
                "Server is shutting down",
            ));
        }
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        if method == hyper::Method::GET && path == "/health" {
            return Ok(json_response(
                StatusCode::OK,
                json!({"status":"ok","protocolVersion":PROTOCOL_VERSION}),
            ));
        }
        if method == hyper::Method::POST && path == "/sessions" {
            read_object(request, true).await?;
            let session = Session::new(
                self.agent.clone(),
                self.storage.clone(),
                SessionOptions::create(),
            )
            .await
            .map_err(|_| {
                HttpError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session_error",
                    "Could not create a session",
                )
            })?;
            let id = session.id().to_owned();
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.closing {
                return Err(HttpError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "server_closed",
                    "Server is shutting down",
                ));
            }
            sessions.values.insert(id.clone(), Arc::new(session));
            return Ok(json_response(StatusCode::CREATED, json!({"sessionId":id})));
        }
        let parts = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
        if !(2..=3).contains(&parts.len()) || parts[0] != "sessions" {
            return Err(HttpError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "Unknown route",
            ));
        }
        let session = self
            .sessions
            .lock()
            .unwrap()
            .values
            .get(parts[1])
            .cloned()
            .ok_or_else(|| {
                HttpError::new(
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    "Unknown session",
                )
            })?;
        let session_id = parts[1].to_owned();
        if method == hyper::Method::DELETE && parts.len() == 2 {
            let removed = self.sessions.lock().unwrap().values.remove(&session_id);
            let run = self.runs.lock().unwrap().remove(&session_id);
            if let Some(run) = &run {
                run.token.cancel();
            }
            // Remove ownership before awaiting cleanup so a disconnected
            // deletion request cannot leave a closed session in the map.
            if removed.is_none() {
                return Err(HttpError::new(
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    "Unknown session",
                ));
            }
            let _ = session.close().await;
            return Ok(Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::Fixed(None))
                .unwrap());
        }
        if method == hyper::Method::POST && parts.len() == 3 {
            match parts[2] {
                "cancel" => {
                    read_object(request, true).await?;
                    let run = self.runs.lock().unwrap().get(&session_id).cloned();
                    if let Some(run) = &run {
                        run.token.cancel();
                    }
                    return Ok(json_response(
                        StatusCode::OK,
                        json!({
                            "accepted":run.is_some(),
                            "runId":run.as_ref().map(|run| run.run_id.clone()),
                        }),
                    ));
                }
                "prompts" => {
                    return self
                        .prompt(request, session_id, session, context)
                        .await
                }
                _ => {}
            }
        }
        Err(HttpError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Unknown route",
        ))
    }

    async fn prompt(
        self: &Arc<Self>,
        request: Request<Incoming>,
        session_id: String,
        session: Arc<Session<P>>,
        context: ConnectionContext,
    ) -> std::result::Result<Response<Body>, HttpError> {
        let object = read_object(request, false).await?;
        let message = object
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .ok_or_else(|| {
                HttpError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "message must be a non-empty string",
                )
            })?
            .to_owned();
        let token = context.cancellation.child_token();
        let run_id = format!("r_{}", Uuid::new_v4().simple());
        {
            if session.busy() {
                return Err(HttpError::new(
                    StatusCode::CONFLICT,
                    "session_busy",
                    "The session already has an active run",
                ));
            }
            let mut runs = self.runs.lock().unwrap();
            if runs.contains_key(&session_id) {
                return Err(HttpError::new(
                    StatusCode::CONFLICT,
                    "session_busy",
                    "The session already has an active run",
                ));
            }
            runs.insert(
                session_id.clone(),
                ActiveRun {
                    token: token.clone(),
                    run_id: run_id.clone(),
                },
            );
        }
        let connection_cancellation = context.cancellation.clone();
        let context = ConnectionContext {
            cancellation: token.clone(),
            written: context.written.clone(),
        };
        // One queued event plus one in-flight event stays below the shared
        // 4 MiB queue limit. The producer waits when the peer stops reading.
        let (sender, receiver) = mpsc::channel(1);
        let failure_sender = sender.clone();
        let failure_context = context.clone();
        let emitted = Arc::new(AtomicBool::new(false));
        let emit: EventFn = {
            let sender = sender.clone();
            let context = context.clone();
            let emitted = emitted.clone();
            Arc::new(move |event| {
                emitted.store(true, Ordering::SeqCst);
                let sender = sender.clone();
                let context = context.clone();
                Box::pin(async move { send_event(event, sender, context).await })
            })
        };
        let state = self.clone();
        self.tasks.spawn(async move {
            let options = RunOptions {
                run_id: Some(run_id.clone()),
                cancellation: token.clone(),
                on_event: Some(emit),
                ..RunOptions::default()
            };
            let result = session.run(&message, options).await;
            if let Err(error) = &result {
                if !emitted.load(Ordering::SeqCst) {
                    let event = crate::types::AgentEvent {
                        run_id: run_id.clone(),
                        sequence: 1,
                        cursor: None,
                        kind: crate::types::EventKind::AgentEnd {
                            outcome: crate::types::RunOutcome::Failed {
                                error: RunFailure::of(error),
                                usage: Default::default(),
                            },
                        },
                    };
                    let _ = send_event(event, failure_sender, failure_context).await;
                }
                if matches!(error, Error::Consumer(_)) {
                    connection_cancellation.cancel();
                }
            }
            state.runs.lock().unwrap().remove(&session_id);
        });
        Ok(Response::builder()
            .header(CONTENT_TYPE, "application/jsonl; charset=utf-8")
            .header(CACHE_CONTROL, "no-store")
            .body(Body::Stream {
                receiver,
                ended: false,
            })
            .unwrap())
    }

    async fn shutdown(&self) {
        self.cancellation.cancel();
        let sessions = {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.closing = true;
            std::mem::take(&mut sessions.values)
        };
        let runs = std::mem::take(&mut *self.runs.lock().unwrap());
        for run in runs.values() {
            run.token.cancel();
        }
        for session in sessions.values() {
            let _ = session.close().await;
        }
        self.tasks.close();
        self.tasks.wait().await;
    }
}

async fn read_object(
    mut request: Request<Incoming>,
    optional: bool,
) -> std::result::Result<Map<String, Value>, HttpError> {
    let too_large = || {
        HttpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_too_large",
            "Request body exceeds its byte limit",
        )
    };
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_HTTP_BODY_BYTES as u64)
    {
        return Err(too_large());
    }
    let read = async {
        let mut bytes = Vec::new();
        while let Some(frame) = request.body_mut().frame().await {
            let frame = frame.map_err(|_| {
                HttpError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "Could not read request body",
                )
            })?;
            if let Ok(data) = frame.into_data() {
                if bytes.len() + data.len() > MAX_HTTP_BODY_BYTES {
                    return Err(too_large());
                }
                bytes.extend_from_slice(&data);
            }
        }
        Ok(bytes)
    };
    let bytes = tokio::time::timeout(Duration::from_secs(30), read)
        .await
        .map_err(|_| {
            HttpError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Request body timed out",
            )
        })??;
    if optional && bytes.is_empty() {
        return Ok(Map::new());
    }
    let media_type = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<mime::Mime>().ok());
    if !media_type.is_some_and(|value| {
        value.type_() == mime::APPLICATION
            && value.subtype() == mime::JSON
            && value
                .get_param(mime::CHARSET)
                .is_none_or(|charset| charset.as_str().eq_ignore_ascii_case("utf-8"))
    }) {
        return Err(HttpError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Expected application/json with UTF-8 encoding",
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        HttpError::new(
            StatusCode::BAD_REQUEST,
            "invalid_utf8",
            "Request body is not valid UTF-8",
        )
    })?;
    serde_json::from_str(text).map_err(|_| {
        HttpError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Request body must be a JSON object",
        )
    })
}

fn json_response(status: StatusCode, value: Value) -> Response<Body> {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::Fixed(Some(bytes.into())))
        .unwrap()
}

#[cfg(test)]
mod tests;
