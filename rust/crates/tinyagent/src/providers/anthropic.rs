use super::{
    client, endpoint, index, key, merge_extra, parse, provider_error, send, string, Event, Headers,
    Parser, ProviderOptions, Usage,
};
use crate::{
    Error, Message, Provider, ProviderRequest, ProviderStream, ProviderTurn, Result, ToolCall,
    MAX_JSONL_LINE_BYTES,
};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const ANTHROPIC_BASE: &str = "https://api.anthropic.com/v1";

pub struct AnthropicProvider {
    pub client: Client,
    pub options: ProviderOptions,
    pub max_tokens: u32,
}

#[derive(Serialize)]
struct InputMessage {
    role: &'static str,
    content: Vec<Value>,
}

impl AnthropicProvider {
    pub fn new() -> Result<Self> {
        Self::with_options(ProviderOptions::new())
    }

    pub fn with_options(options: ProviderOptions) -> Result<Self> {
        Self::with_max_tokens(options, 8192)
    }

    pub fn with_max_tokens(options: ProviderOptions, max_tokens: u32) -> Result<Self> {
        if max_tokens == 0 {
            return Err(Error::Configuration(
                "max_tokens must be a positive integer".into(),
            ));
        }
        Ok(Self {
            client: client()?,
            options,
            max_tokens,
        })
    }

    pub(super) fn request(&self, request: ProviderRequest) -> Result<reqwest::RequestBuilder> {
        let extra = request.extra.clone();
        let key = key(&self.options.api_key, "ANTHROPIC_API_KEY");
        let mut headers = Headers::new();
        headers.set("Content-Type", "application/json")?;
        headers.set("Accept", "text/event-stream")?;
        headers.set("anthropic-version", "2023-06-01")?;
        if let Some(key) = &key {
            headers.set("x-api-key", key.clone())?;
        }
        headers.merge(&self.options.headers)?;
        if !headers.contains("x-api-key") {
            return Err(Error::Configuration(
                "ANTHROPIC_API_KEY is not set".into(),
            ));
        }
        if self.max_tokens == 0 {
            return Err(Error::Configuration(
                "max_tokens must be positive".into(),
            ));
        }
        let mut messages: Vec<InputMessage> = Vec::new();
        let mut previous_result = false;
        for message in request.history {
            let is_result = matches!(message, Message::ToolResult { .. });
            match message {
                Message::User { text } => messages.push(InputMessage {
                    role: "user",
                    content: vec![json!({"type":"text", "text":text})],
                }),
                Message::Assistant {
                    text,
                    tool_calls,
                    replay,
                } => {
                    let mut content = Vec::new();
                    if !replay.is_empty() {
                        if replay.len() != 1 {
                            return Err(Error::Provider(
                                "Anthropic replay must contain one message".into(),
                            ));
                        }
                        content = replay[0]
                            .get("content")
                            .and_then(Value::as_array)
                            .cloned()
                            .ok_or_else(|| {
                                Error::Provider(
                                    "Anthropic replay message is missing content".into(),
                                )
                            })?;
                    } else {
                        if !text.is_empty() {
                            content.push(json!({"type":"text", "text":text}));
                        }
                        content.extend(tool_calls.into_iter().map(|call| json!({"type":"tool_use", "id":call.call_id, "name":call.name, "input":call.arguments})));
                    }
                    // The API rejects a message without content.
                    if !content.is_empty() {
                        messages.push(InputMessage {
                            role: "assistant",
                            content,
                        });
                    }
                }
                Message::ToolResult {
                    call_id,
                    output,
                    is_error,
                    ..
                } => {
                    let block = json!({"type":"tool_result", "tool_use_id":call_id, "content":output, "is_error":is_error});
                    if previous_result {
                        messages
                            .last_mut()
                            .expect("previous tool result created a message")
                            .content
                            .push(block);
                    } else {
                        messages.push(InputMessage {
                            role: "user",
                            content: vec![block],
                        });
                    }
                }
            }
            previous_result = is_result;
        }
        let tools: Vec<_> = request.tools.into_iter().map(|tool| json!({"name":tool.name,"description":tool.description,"input_schema":tool.parameters})).collect();
        let mut body = json!({"model":request.model,
                "messages":messages,"max_tokens":self.max_tokens,"stream":true});
        if !request.instructions.is_empty() {
            body["system"] = Value::String(request.instructions);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        merge_extra(&mut body, &self.options.extra)?;
        merge_extra(&mut body, &extra)?;
        let url = endpoint(&self.options, ANTHROPIC_BASE, "/messages")?;
        Ok(headers.apply(self.client.post(url).json(&body)))
    }
}

impl Provider for AnthropicProvider {
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

struct Block {
    value: Value,
    arguments: String,
    open: bool,
}

#[derive(Default)]
pub(super) struct Response {
    message: Option<Map<String, Value>>,
    usage: Map<String, Value>,
    blocks: BTreeMap<usize, Block>,
}

impl Parser for Response {
    fn event(&mut self, event: Value) -> Result<Option<Event>> {
        match string(&event, "type")? {
            "message_start" => {
                if self.message.is_some() {
                    return Err(Error::Provider("Repeated Anthropic message_start".into()));
                }
                let message = event
                    .get("message")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| Error::Provider("Missing Anthropic message".into()))?;
                if let Some(usage) = message.get("usage") {
                    self.usage = usage
                        .as_object()
                        .cloned()
                        .ok_or_else(|| Error::Provider("Invalid Anthropic usage".into()))?;
                }
                self.message = Some(message);
            }
            "content_block_start" => {
                let index = index(&event, "index")?;
                if self.message.is_none() || self.blocks.contains_key(&index) {
                    return Err(Error::Provider("Invalid Anthropic block order".into()));
                }
                let value = event
                    .get("content_block")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or_else(|| Error::Provider("Missing Anthropic content block".into()))?;
                string(&value, "type")?;
                self.blocks.insert(
                    index,
                    Block {
                        value,
                        arguments: String::new(),
                        open: true,
                    },
                );
            }
            "content_block_delta" => {
                let block = self
                    .blocks
                    .get_mut(&index(&event, "index")?)
                    .filter(|block| block.open)
                    .ok_or_else(|| {
                        Error::Provider("Anthropic delta referenced an unopened block".into())
                    })?;
                let delta = event
                    .get("delta")
                    .ok_or_else(|| Error::Provider("Missing Anthropic delta".into()))?;
                let kind = string(delta, "type")?;
                let (field, expected) = match kind {
                    "text_delta" => ("text", "text"),
                    "thinking_delta" => ("thinking", "thinking"),
                    "signature_delta" => ("signature", "thinking"),
                    "input_json_delta" => ("partial_json", "tool_use"),
                    _ => return Ok(None),
                };
                if string(&block.value, "type")? != expected {
                    return Err(Error::Provider(
                        "Anthropic delta does not match its block".into(),
                    ));
                }
                let value = string(delta, field)?;
                if kind == "input_json_delta" {
                    if block.arguments.len() + value.len() > MAX_JSONL_LINE_BYTES {
                        return Err(Error::Provider(
                            "Anthropic tool arguments exceed the byte limit".into(),
                        ));
                    }
                    block.arguments.push_str(value);
                } else {
                    let target = block
                        .value
                        .as_object_mut()
                        .expect("block checked at start")
                        .entry(field)
                        .or_insert_with(|| Value::String(String::new()));
                    match target {
                        Value::String(text) => text.push_str(value),
                        _ => {
                            return Err(Error::Provider(format!(
                                "Anthropic {field} is not a string"
                            )))
                        }
                    }
                    if !value.is_empty() {
                        return Ok(Some(match kind {
                            "text_delta" => Event::TextDelta(value.into()),
                            _ => Event::ReasoningDelta(value.into()),
                        }));
                    }
                }
            }
            "content_block_stop" => {
                let block = self
                    .blocks
                    .get_mut(&index(&event, "index")?)
                    .filter(|block| block.open)
                    .ok_or_else(|| Error::Provider("Anthropic stopped an unopened block".into()))?;
                block.open = false;
                if string(&block.value, "type")? == "tool_use" {
                    if !block.arguments.is_empty() {
                        block.value["input"] =
                            serde_json::from_str(&block.arguments).map_err(provider_error)?;
                    }
                    if !block.value.get("input").is_some_and(Value::is_object) {
                        return Err(Error::Provider(
                            "Anthropic tool arguments must be an object".into(),
                        ));
                    }
                }
            }
            "message_delta" => {
                let message = self.message.as_mut().ok_or_else(|| {
                    Error::Provider("Anthropic delta preceded message_start".into())
                })?;
                let delta = event
                    .get("delta")
                    .and_then(Value::as_object)
                    .ok_or_else(|| Error::Provider("Invalid Anthropic message delta".into()))?;
                message.extend(delta.clone());
                if let Some(usage) = event.get("usage") {
                    self.usage.extend(
                        usage
                            .as_object()
                            .ok_or_else(|| Error::Provider("Invalid Anthropic usage delta".into()))?
                            .clone(),
                    );
                }
            }
            "message_stop" => {
                let (turn, usage) = self.complete()?;
                return Ok(Some(Event::Turn { turn, usage }));
            }
            "error" => return Err(Error::Provider(anthropic_error_message(&event))),
            _ => {}
        }
        Ok(None)
    }
}

impl Response {
    fn complete(&self) -> Result<(ProviderTurn, Option<Usage>)> {
        let mut message = self
            .message
            .clone()
            .ok_or_else(|| Error::Provider("Anthropic stream stopped without a message".into()))?;
        match message.get("stop_reason").and_then(Value::as_str) {
            Some("end_turn" | "tool_use" | "stop_sequence" | "refusal") => {}
            reason => {
                return Err(Error::Provider(format!(
                    "Anthropic response is incomplete: {reason:?}"
                )))
            }
        }
        let mut output = Vec::new();
        let mut calls = Vec::new();
        let mut ids = BTreeSet::new();
        let mut text = String::new();
        for block in self.blocks.values() {
            if block.open {
                return Err(Error::Provider(
                    "Anthropic stream stopped with an unfinished block".into(),
                ));
            }
            match string(&block.value, "type")? {
                "text" => text.push_str(string(&block.value, "text")?),
                "tool_use" => {
                    let call_id = string(&block.value, "id")?.to_owned();
                    let name = string(&block.value, "name")?.to_owned();
                    if call_id.is_empty() || name.is_empty() || !ids.insert(call_id.clone()) {
                        return Err(Error::Provider("Invalid Anthropic tool call".into()));
                    }
                    calls.push(ToolCall {
                        call_id,
                        name,
                        arguments: block.value["input"].clone(),
                    });
                }
                _ => {}
            }
            output.push(block.value.clone());
        }
        message.insert("content".into(), Value::Array(output.clone()));
        message.insert("usage".into(), Value::Object(self.usage.clone()));
        let usage = match (
            self.usage.get("input_tokens").and_then(Value::as_u64),
            self.usage.get("output_tokens").and_then(Value::as_u64),
        ) {
            (Some(input_tokens), Some(output_tokens)) => Some(Usage {
                input_tokens,
                output_tokens,
            }),
            _ => None,
        };
        Ok((
            ProviderTurn {
                text,
                tool_calls: calls,
                output,
                replay: vec![Value::Object(message)],
            },
            usage,
        ))
    }
}

fn anthropic_error_message(event: &Value) -> String {
    event
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .or_else(|| error.get("type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("Anthropic stream error")
        .to_owned()
}
