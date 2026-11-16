use super::{
    client, endpoint, key, merge_extra, parse, send, Event, Headers, Parser, ProviderOptions, Usage,
};
use crate::{
    Error, Message, Provider, ProviderRequest, ProviderStream, ProviderTurn, Result, ToolCall,
    MAX_JSONL_LINE_BYTES,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const CHAT_BASE: &str = "https://api.openai.com/v1";

/// The OpenAI-compatible `/chat/completions` streaming API. Most gateways and
/// self-hosted model servers offer only this shape. A key is optional: an empty
/// result means no `Authorization` header is sent.
pub struct ChatCompletionsProvider {
    pub client: Client,
    pub options: ProviderOptions,
}

impl ChatCompletionsProvider {
    pub fn new() -> Result<Self> {
        Self::with_options(ProviderOptions::new())
    }

    pub fn with_options(options: ProviderOptions) -> Result<Self> {
        Ok(Self {
            client: client()?,
            options,
        })
    }

    pub(super) fn request(&self, request: ProviderRequest) -> Result<reqwest::RequestBuilder> {
        let extra = request.extra.clone();
        let key = key(&self.options.api_key, "OPENAI_API_KEY");
        let mut headers = Headers::new();
        headers.set("Content-Type", "application/json")?;
        headers.set("Accept", "text/event-stream")?;
        if let Some(key) = &key {
            headers.set("Authorization", format!("Bearer {key}"))?;
        }
        headers.merge(&self.options.headers)?;
        let mut messages = Vec::new();
        if !request.instructions.is_empty() {
            messages.push(json!({"role":"system", "content":request.instructions}));
        }
        messages.extend(chat_messages(request.history));
        let tools: Vec<_> = request
            .tools
            .into_iter()
            .map(|tool| {
                json!({"type":"function", "function":{
                    "name":tool.name, "description":tool.description, "parameters":tool.parameters}})
            })
            .collect();
        let mut body = json!({
            "model":request.model,
            "messages":messages,
            "stream":true,
            "stream_options":{"include_usage":true},
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        merge_extra(&mut body, &self.options.extra)?;
        merge_extra(&mut body, &extra)?;
        let url = endpoint(&self.options, CHAT_BASE, "/chat/completions")?;
        Ok(headers.apply(self.client.post(url).json(&body)))
    }
}

impl Provider for ChatCompletionsProvider {
    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream> {
        if request.signal.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let request = self.request(request)?;
        Ok(parse(
            send(&self.client, &self.options, request).await?,
            Response::default(),
        ))
    }
}

/// Provider-neutral history as Chat Completions messages. A turn with neither
/// text nor tool calls is omitted because the API rejects an empty assistant.
fn chat_messages(history: Vec<Message>) -> Vec<Value> {
    let mut messages = Vec::new();
    for message in history {
        match message {
            Message::User { text } => messages.push(json!({"role":"user", "content":text})),
            Message::Assistant {
                text, tool_calls, ..
            } => {
                if text.is_empty() && tool_calls.is_empty() {
                    continue;
                }
                let calls: Vec<_> = tool_calls
                    .into_iter()
                    .map(|call| {
                        json!({"id":call.call_id, "type":"function", "function":{
                            "name":call.name, "arguments":call.arguments.to_string()}})
                    })
                    .collect();
                let mut message = json!({
                    "role":"assistant",
                    "content": if text.is_empty() { Value::Null } else { Value::String(text) },
                });
                if !calls.is_empty() {
                    message["tool_calls"] = Value::Array(calls);
                }
                messages.push(message);
            }
            Message::ToolResult {
                call_id, output, ..
            } => messages.push(
                json!({"role":"tool", "tool_call_id":call_id, "content":output}),
            ),
        }
    }
    messages
}

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub(super) struct Response {
    text: String,
    calls: BTreeMap<usize, Call>,
    finished: Option<String>,
    usage: Option<Usage>,
}

impl Parser for Response {
    fn event(&mut self, event: Value) -> Result<Option<Event>> {
        if let Some(error) = event.get("error") {
            return Err(Error::Provider(chat_error_message(error)));
        }
        if let Some(usage) = event.get("usage").filter(|usage| usage.is_object()) {
            if let (Some(input), Some(output)) = (
                usage.get("prompt_tokens").and_then(Value::as_u64),
                usage.get("completion_tokens").and_then(Value::as_u64),
            ) {
                self.usage = Some(Usage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
        }
        let Some(choice) = event
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .filter(|choice| choice.is_object())
        else {
            return Ok(None);
        };
        let empty = Value::Object(Default::default());
        let delta = choice
            .get("delta")
            .filter(|delta| delta.is_object())
            .unwrap_or(&empty);
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                self.text.push_str(content);
                return Ok(Some(Event::TextDelta(content.into())));
            }
        }
        let reasoning = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .or_else(|| delta.get("reasoning").and_then(Value::as_str));
        if let Some(reasoning) = reasoning.filter(|reasoning| !reasoning.is_empty()) {
            return Ok(Some(Event::ReasoningDelta(reasoning.into())));
        }
        if let Some(parts) = delta.get("tool_calls").and_then(Value::as_array) {
            for part in parts {
                if !part.is_object() {
                    return Err(Error::Provider(
                        "Chat Completions tool call must be an object".into(),
                    ));
                }
                let index = part
                    .get("index")
                    .and_then(Value::as_u64)
                    .map_or(0, |index| index as usize);
                let call = self.calls.entry(index).or_default();
                if let Some(id) = part.get("id").and_then(Value::as_str) {
                    if !id.is_empty() {
                        call.id = id.into();
                    }
                }
                if let Some(function) = part.get("function").filter(|value| value.is_object()) {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        if !name.is_empty() {
                            call.name = name.into();
                        }
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        if call.arguments.len() + arguments.len() > MAX_JSONL_LINE_BYTES {
                            return Err(Error::Provider(
                                "Chat Completions tool arguments exceed the byte limit".into(),
                            ));
                        }
                        call.arguments.push_str(arguments);
                    }
                }
            }
        }
        if let Some(finished) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finished = Some(finished.into());
        }
        Ok(None)
    }

    fn finish(&mut self) -> Result<Option<Event>> {
        let finished = self.finished.clone().ok_or_else(|| {
            Error::Provider("Chat Completions stream ended before a finish_reason".into())
        })?;
        if !matches!(finished.as_str(), "stop" | "tool_calls" | "function_call") {
            return Err(Error::Provider(format!(
                "Chat Completions response is incomplete: {finished}"
            )));
        }
        let mut calls = Vec::new();
        let mut ids = BTreeSet::new();
        for (_, call) in std::mem::take(&mut self.calls) {
            if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.clone()) {
                return Err(Error::Provider(
                    "invalid Chat Completions tool call".into(),
                ));
            }
            let raw = if call.arguments.is_empty() {
                "{}"
            } else {
                &call.arguments
            };
            let arguments: Value = serde_json::from_str(raw).map_err(|error| {
                Error::Provider(format!(
                    "invalid arguments for tool {}: {error}",
                    call.name
                ))
            })?;
            if !arguments.is_object() {
                return Err(Error::Provider(format!(
                    "invalid arguments for tool {}: expected an object",
                    call.name
                )));
            }
            calls.push(ToolCall {
                call_id: call.id,
                name: call.name,
                arguments,
            });
        }
        let text = std::mem::take(&mut self.text);
        Ok(Some(Event::Turn {
            turn: ProviderTurn {
                text,
                tool_calls: calls,
                replay: Vec::new(),
                output: Vec::new(),
            },
            usage: self.usage,
        }))
    }
}

fn chat_error_message(error: &Value) -> String {
    match error {
        Value::Object(_) => error
            .get("message")
            .or_else(|| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("Chat Completions stream error")
            .to_owned(),
        Value::String(message) => message.clone(),
        _ => "Chat Completions stream error".into(),
    }
}
