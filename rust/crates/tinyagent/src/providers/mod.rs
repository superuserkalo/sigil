mod anthropic;
mod chat;
mod openai;
mod sse;

pub use anthropic::AnthropicProvider;
pub use chat::ChatCompletionsProvider;
pub use openai::OpenAIProvider;

use crate::{Error, ProviderTurn, Result, TokenUsage};
use futures_util::{
    stream,
    stream::BoxStream,
    StreamExt,
};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt, env,
    time::Duration,
};

/// The core provider stream. Wrappers around an arbitrary transport use it too.
pub(crate) type Bytes = BoxStream<'static, Result<Vec<u8>>>;

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_ERROR_BODY_BYTES: usize = 8_192;
const MAX_ERROR_CHARS: usize = 500;

/// Token counts for one model request, when the API reported them.
pub type Usage = TokenUsage;

/// What a provider parser yields. Reasoning deltas are visible model reasoning,
/// separate from answer text so a host can show or hide them on their own.
#[derive(Debug)]
pub(crate) enum Event {
    TextDelta(String),
    ReasoningDelta(String),
    Turn {
        turn: ProviderTurn,
        usage: Option<Usage>,
    },
}

/// The provider configuration shared by the three shipped adapters. It mirrors
/// the TypeScript `HttpRequestOptions` plus the API key.
#[derive(Debug, Clone, Default)]
pub struct ProviderOptions {
    /// The complete endpoint URL. Wins over `base_url`.
    pub endpoint: Option<String>,
    /// The API root, such as `https://gateway.example/v1`. The adapter appends its path.
    pub base_url: Option<String>,
    /// An explicit key. Otherwise the adapter reads its environment variable.
    pub api_key: Option<String>,
    /// Extra request headers. A header named like a default one replaces it.
    pub headers: BTreeMap<String, String>,
    /// A JSON object merged into every request body after the adapter's own fields.
    pub extra: Option<Value>,
    /// Per-request timeout. Defaults to 180 seconds.
    pub timeout: Option<Duration>,
}

impl ProviderOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

pub(crate) fn client() -> Result<Client> {
    Client::builder()
        .build()
        .map_err(|error| Error::Configuration(error.to_string()))
}

/// A provider failure with the metadata the TypeScript `ProviderError` carries.
/// The core error type has no retryable flag, so the message says when a request
/// could be sent again and the caller still sees a plain `Error::Provider`.
pub(crate) struct Failure {
    pub message: String,
    pub retryable: bool,
    pub retry_after: Option<u64>,
}

impl Failure {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            retry_after: None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        if failure.retryable {
            return Error::ProviderRetry {
                message: failure.message,
                retry_after: failure.retry_after.map(Duration::from_secs),
            };
        }
        Error::Provider(failure.message)
    }
}

/// Case-insensitive request headers. User headers replace a default of the same name.
pub(crate) struct Headers {
    values: Vec<(String, String)>,
}

impl Headers {
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) -> Result<()> {
        let name = name.into();
        let value = value.into();
        check_header(&name, &value)?;
        let lower = name.to_ascii_lowercase();
        self.values.retain(|(existing, _)| *existing != lower);
        self.values.push((lower, value));
        Ok(())
    }

    pub fn merge(&mut self, extra: &BTreeMap<String, String>) -> Result<()> {
        for (name, value) in extra {
            self.set(name.clone(), value.clone())?;
        }
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        self.values.iter().any(|(existing, _)| *existing == lower)
    }

    fn apply(self, builder: RequestBuilder) -> RequestBuilder {
        self.values
            .into_iter()
            .fold(builder, |builder, (name, value)| builder.header(name, value))
    }
}

/// A control character in a header value is a mistake, usually a key read with
/// its newline. The value itself never appears in the error.
fn check_header(name: &str, value: &str) -> Result<()> {
    if value
        .chars()
        .any(|character| character <= '\u{1f}' || character == '\u{7f}')
    {
        return Err(Error::Configuration(format!(
            "request header {name} contains a control character"
        )));
    }
    Ok(())
}

pub(crate) fn endpoint(
    options: &ProviderOptions,
    default_base: &str,
    path: &str,
) -> Result<String> {
    let url = match &options.endpoint {
        Some(endpoint) => endpoint.clone(),
        None => {
            let base = options
                .base_url
                .as_deref()
                .unwrap_or(default_base)
                .trim_end_matches('/');
            format!("{base}{path}")
        }
    };
    validate_endpoint(&url)?;
    Ok(url)
}

fn validate_endpoint(url: &str) -> Result<()> {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return Err(Error::Configuration(
            "provider endpoint must be an HTTP or HTTPS URL".into(),
        ));
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || url.contains('#') {
        return Err(Error::Configuration(
            "provider endpoint must be an HTTP or HTTPS URL".into(),
        ));
    }
    Ok(())
}

pub(crate) fn key(explicit: &Option<String>, variable: &str) -> Option<String> {
    explicit
        .clone()
        .or_else(|| env::var(variable).ok())
        .filter(|key| !key.is_empty())
}

pub(crate) fn merge_extra(body: &mut Value, extra: &Option<Value>) -> Result<()> {
    match extra {
        None => Ok(()),
        Some(Value::Object(extra)) => {
            let body = body
                .as_object_mut()
                .ok_or_else(|| Error::Configuration("provider body must be an object".into()))?;
            for (name, value) in extra {
                body.insert(name.clone(), value.clone());
            }
            Ok(())
        }
        Some(_) => Err(Error::Configuration(
            "provider extra must be a JSON object".into(),
        )),
    }
}

pub(crate) fn send(
    client: &Client,
    options: &ProviderOptions,
    request: RequestBuilder,
) -> impl std::future::Future<Output = Result<Bytes>> + Send {
    let timeout = options.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let client = client.clone();
    async move {
        let request = request.timeout(timeout).build().map_err(connect_error)?;
        let mut response = client.execute(request).await.map_err(connect_error)?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok());
            let mut body = Vec::new();
            while body.len() < MAX_ERROR_BODY_BYTES {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) | Err(_) => break,
                };
                let take = chunk.len().min(MAX_ERROR_BODY_BYTES - body.len());
                body.extend_from_slice(&chunk[..take]);
            }
            let detail = truncate(&String::from_utf8_lossy(&body));
            let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
            return Err(Error::from(Failure {
                message: format!("API error {status}: {detail}"),
                retryable,
                retry_after,
            }));
        }
        Ok(response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()).map_err(connect_error))
            .boxed())
    }
}

/// Nothing of the response arrived, so sending the request again is safe.
fn connect_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        return Error::Provider("provider request timed out".into());
    }
    Error::from(Failure::retryable(format!(
        "provider connection failed: {error}"
    )))
}

fn truncate(detail: &str) -> String {
    if detail.chars().count() <= MAX_ERROR_CHARS {
        detail.to_owned()
    } else {
        detail.chars().take(MAX_ERROR_CHARS).collect()
    }
}

pub(crate) trait Parser: Send + 'static {
    fn event(&mut self, event: Value) -> Result<Option<Event>>;

    /// Some APIs, notably Chat Completions, send the terminal information
    /// before the transport closes. `finish` may only return a turn.
    fn finish(&mut self) -> Result<Option<Event>> {
        Ok(None)
    }
}

/// A parser stream adapted to the core provider trait. A turn's usage is emitted
/// as its own event first, because the core sums token counts separately.
pub(crate) fn parse(bytes: Bytes, parser: impl Parser) -> crate::ProviderStream {
    parse_events(bytes, parser)
        .map(|result| match result {
            Ok(event) => stream::iter(convert(event)),
            Err(error) => stream::iter(vec![Err(error)]),
        })
        .flatten()
        .boxed()
}

fn convert(event: Event) -> Vec<Result<crate::ProviderEvent>> {
    match event {
        Event::TextDelta(delta) => vec![Ok(crate::ProviderEvent::TextDelta(delta))],
        Event::ReasoningDelta(delta) => vec![Ok(crate::ProviderEvent::ReasoningDelta(delta))],
        Event::Turn { turn, usage } => {
            let mut events: Vec<Result<crate::ProviderEvent>> = Vec::with_capacity(2);
            if let Some(usage) = usage {
                events.push(Ok(crate::ProviderEvent::Usage(usage)));
            }
            events.push(Ok(crate::ProviderEvent::Turn(turn)));
            events
        }
    }
}

/// The unadapted stream: text deltas, reasoning deltas and the terminal turn.
pub(crate) fn parse_events(
    bytes: Bytes,
    parser: impl Parser,
) -> BoxStream<'static, Result<Event>> {
    stream::try_unfold(
        (sse::Reader::new(bytes), parser, false),
        |(mut reader, mut parser, finished)| async move {
            if finished {
                return Ok(None);
            }
            loop {
                match reader.next().await? {
                    Some(value) => {
                        if let Some(event) = parser.event(value)? {
                            let finished = matches!(event, Event::Turn { .. });
                            return Ok(Some((event, (reader, parser, finished))));
                        }
                    }
                    None => {
                        return match parser.finish()? {
                            Some(event) => Ok(Some((event, (reader, parser, true)))),
                            None => Err(Error::Provider(
                                "Provider stream ended before its terminal event".into(),
                            )),
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

pub(crate) fn provider_error(error: impl std::fmt::Display) -> Error {
    Error::Provider(error.to_string())
}

pub(crate) fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Provider(format!("Provider field {field} must be a string")))
}

pub(crate) fn index(value: &Value, field: &str) -> Result<usize> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| {
            Error::Provider(format!(
                "Provider field {field} must be a nonnegative index"
            ))
        })
}

#[cfg(test)]
mod tests;
