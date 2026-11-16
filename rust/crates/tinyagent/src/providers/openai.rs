use super::{
    client, endpoint, index, key, merge_extra, parse, provider_error, send, string, Event, Headers,
    Parser, ProviderOptions, Usage,
};
use crate::{
    Error, Message, Provider, ProviderRequest, ProviderStream, ProviderTurn, Result, ToolCall,
    MAX_JSONL_LINE_BYTES,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const OPENAI_BASE: &str = "https://api.openai.com/v1";

pub struct OpenAIProvider {
    pub client: Client,
    pub options: ProviderOptions,
}

impl OpenAIProvider {
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
        if !headers.contains("authorization") {
            return Err(Error::Configuration("OPENAI_API_KEY is not set".into()));
        }
        let mut input = Vec::new();
        for message in request.history {
            match message {
                Message::User { text } => input.push(json!({"role":"user", "content":text})),
                Message::Assistant {
                    text,
                    tool_calls,
                    replay,
                } => {
                    if !replay.is_empty() {
                        input.extend(replay);
                        continue;
                    }
                    if !text.is_empty() {
                        input.push(json!({"role":"assistant", "content":text}));
                    }
                    for call in tool_calls {
                        input.push(json!({"type":"function_call", "call_id":call.call_id, "name":call.name, "arguments":call.arguments.to_string()}));
                    }
                }
                Message::ToolResult {
                    call_id, output, ..
                } => input.push(
                    json!({"type":"function_call_output", "call_id":call_id, "output":output}),
                ),
            }
        }
        let tools: Vec<_> = request
            .tools
            .into_iter()
            .map(|tool| {
                json!({"type":"function", "name":tool.name,
            "description":tool.description, "parameters":tool.parameters, "strict":true})
            })
            .collect();
        let mut body = json!({
            "model":request.model, "instructions":request.instructions, "input":input, "tools":tools,
            "parallel_tool_calls":false, "store":false, "stream":true,
            "include":["reasoning.encrypted_content"],
        });
        merge_extra(&mut body, &self.options.extra)?;
        merge_extra(&mut body, &extra)?;
        let url = endpoint(&self.options, OPENAI_BASE, "/responses")?;
        Ok(headers.apply(self.client.post(url).json(&body)))
    }
}

impl Provider for OpenAIProvider {
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

#[derive(Default)]
pub(super) struct Response {
    items: BTreeMap<usize, Value>,
    arguments: BTreeMap<usize, String>,
    usage: Option<Usage>,
}

impl Parser for Response {
    fn event(&mut self, event: Value) -> Result<Option<Event>> {
        match string(&event, "type")? {
            "response.output_item.added" | "response.output_item.done" => {
                let index = index(&event, "output_index")?;
                let item = event
                    .get("item")
                    .filter(|item| item.is_object())
                    .ok_or_else(|| Error::Provider("Missing OpenAI output item".into()))?;
                if string(item, "type")? == "function_call" {
                    let arguments = string(item, "arguments")?;
                    if !arguments.is_empty() {
                        self.arguments.insert(index, arguments.into());
                    }
                }
                self.items.insert(index, item.clone());
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = string(&event, "delta")?;
                if !delta.is_empty() {
                    return Ok(Some(Event::ReasoningDelta(delta.into())));
                }
            }
            "response.output_text.delta" => {
                let delta = string(&event, "delta")?;
                if !delta.is_empty() {
                    return Ok(Some(Event::TextDelta(delta.into())));
                }
            }
            "response.function_call_arguments.delta" => {
                let index = index(&event, "output_index")?;
                let delta = string(&event, "delta")?;
                let arguments = self.arguments.entry(index).or_default();
                if arguments.len() + delta.len() > MAX_JSONL_LINE_BYTES {
                    return Err(Error::Provider(
                        "OpenAI tool arguments exceed the byte limit".into(),
                    ));
                }
                arguments.push_str(delta);
            }
            "response.function_call_arguments.done" => {
                self.arguments.insert(
                    index(&event, "output_index")?,
                    string(&event, "arguments")?.into(),
                );
            }
            "response.completed" => {
                let turn = self.complete(&event)?;
                return Ok(Some(Event::Turn {
                    turn,
                    usage: self.usage,
                }));
            }
            "response.failed" | "response.incomplete" | "error" => {
                return Err(Error::Provider(openai_error_message(&event)))
            }
            _ => {}
        }
        Ok(None)
    }
}

impl Response {
    fn complete(&mut self, event: &Value) -> Result<ProviderTurn> {
        let response = event
            .get("response")
            .filter(|value| value.is_object())
            .ok_or_else(|| Error::Provider("Missing completed OpenAI response".into()))?;
        if response
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "completed")
        {
            return Err(Error::Provider("OpenAI response is incomplete".into()));
        }
        if let Some(usage) = response.get("usage").filter(|usage| usage.is_object()) {
            if let (Some(input), Some(output)) = (
                usage.get("input_tokens").and_then(Value::as_u64),
                usage.get("output_tokens").and_then(Value::as_u64),
            ) {
                self.usage = Some(Usage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
        }
        let items: Vec<(usize, Value)> = match response.get("output") {
            Some(Value::Array(items)) => items.iter().cloned().enumerate().collect(),
            None => self
                .items
                .iter()
                .map(|(index, item)| (*index, item.clone()))
                .collect(),
            _ => return Err(Error::Provider("OpenAI output must be an array".into())),
        };
        let mut output = Vec::new();
        let mut calls = Vec::new();
        let mut text = String::new();
        let mut ids = BTreeSet::new();
        for (index, mut item) in items {
            match string(&item, "type")? {
                "function_call" => {
                    let raw = match item.get("arguments") {
                        Some(Value::String(raw)) if !raw.is_empty() => raw.clone(),
                        None | Some(Value::String(_)) => {
                            self.arguments.get(&index).cloned().unwrap_or_default()
                        }
                        _ => {
                            return Err(Error::Provider(
                                "OpenAI arguments must be a JSON string".into(),
                            ))
                        }
                    };
                    let arguments: Value = serde_json::from_str(&raw).map_err(provider_error)?;
                    let call_id = string(&item, "call_id")?.to_owned();
                    let name = string(&item, "name")?.to_owned();
                    if raw.len() > MAX_JSONL_LINE_BYTES
                        || !arguments.is_object()
                        || call_id.is_empty()
                        || name.is_empty()
                        || !ids.insert(call_id.clone())
                    {
                        return Err(Error::Provider("Invalid OpenAI function call".into()));
                    }
                    item["arguments"] = Value::String(raw);
                    calls.push(ToolCall {
                        call_id,
                        name,
                        arguments,
                    });
                }
                "message" => {
                    let content =
                        item.get("content")
                            .and_then(Value::as_array)
                            .ok_or_else(|| {
                                Error::Provider("OpenAI message is missing content".into())
                            })?;
                    for part in content {
                        if string(part, "type")? == "output_text" {
                            text.push_str(string(part, "text")?);
                        }
                    }
                }
                _ => {}
            }
            output.push(item);
        }
        Ok(ProviderTurn {
            text,
            tool_calls: calls,
            replay: output.clone(),
            output,
        })
    }
}

fn openai_error_message(event: &Value) -> String {
    match string(event, "type").unwrap_or("error") {
        "error" => event
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("OpenAI stream error")
            .to_owned(),
        kind => {
            if let Some(response) = event.get("response").filter(|value| value.is_object()) {
                if let Some(error) = response.get("error").filter(|value| value.is_object()) {
                    return error
                        .get("message")
                        .or_else(|| error.get("code"))
                        .and_then(Value::as_str)
                        .unwrap_or("OpenAI response failed")
                        .to_owned();
                }
                if let Some(reason) = response
                    .get("incomplete_details")
                    .and_then(|details| details.get("reason"))
                    .and_then(Value::as_str)
                {
                    return format!("OpenAI response incomplete: {reason}");
                }
            }
            format!("OpenAI stream failed: {kind}")
        }
    }
}
