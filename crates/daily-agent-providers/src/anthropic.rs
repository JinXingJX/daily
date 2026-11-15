//! Anthropic Messages API: native tool use, streaming, prompt caching and thinking replay.
use super::wire::{self, SseEvents};
use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelError,
    ModelErrorKind, ModelMessage, ModelMessageRole, ModelProviderKind, ModelStopReason,
    ModelStreamChunk, ModelToolCall, ModelToolChoice, ModelUsage, PROVIDER_ANTHROPIC,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const LABEL: &str = "Anthropic";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicMessagesAdapter;

#[async_trait]
impl ModelAdapter for AnthropicMessagesAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::AnthropicMessages
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        mut on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        let endpoint = format!("{}/messages", config.base_url.trim_end_matches('/'));
        let stream = on_chunk.is_some();
        let names = wire::tool_name_map(&request.tools);
        let requested = request
            .max_output_tokens
            .unwrap_or(config.max_output_tokens);
        let send = |replay_thinking: bool, max_tokens: u32| {
            let mut body = request_body(config, request, stream, replay_thinking);
            body["max_tokens"] = json!(max_tokens);
            let builder = http
                .post(&endpoint)
                .header("anthropic-version", API_VERSION)
                .json(&body);
            let builder = if config.api_key.is_empty() {
                builder
            } else {
                builder.header("x-api-key", &config.api_key)
            };
            wire::send(LABEL, builder, stream)
        };
        // A rejected request is adjusted and sent again, once per kind of rejection.
        let mut replay_thinking = true;
        let mut max_tokens = requested;
        let response = loop {
            let error = match send(replay_thinking, max_tokens).await {
                Ok(response) => break response,
                Err(error) => error,
            };
            // A thinking block is signed together with the conversation before it. When the
            // history changed (a compaction, a changed tool set, another model), the API
            // rejects the replayed blocks; the documented recovery is to send the same
            // history once more without any thinking block.
            if replay_thinking && is_thinking_signature_error(&error) {
                replay_thinking = false;
                continue;
            }
            // The configured output budget can exceed what this model allows; the error
            // names the model's maximum.
            match wire::lower_output_limit(&error, max_tokens) {
                Ok(Some(allowed)) if max_tokens == requested => max_tokens = allowed,
                _ => return Err(error),
            }
        };

        let mut message = AssistantMessage::default();
        if let Some(on_chunk) = on_chunk.as_mut() {
            let mut events = SseEvents::new(LABEL, response);
            while let Some(data) = events.next().await? {
                let event: Value = serde_json::from_str(&data).map_err(|error| {
                    ModelError::protocol(format!("Failed to parse {LABEL} stream event: {error}"))
                })?;
                let text = message.apply_event(&event)?;
                if !text.is_empty() {
                    on_chunk(ModelStreamChunk { text }).map_err(ModelError::aborted)?;
                }
            }
        } else {
            let payload: Value = response.json().await.map_err(|error| {
                ModelError::protocol(format!("Failed to parse {LABEL} response: {error}"))
            })?;
            message.apply_message(&payload);
            message.content = payload
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
        }
        message.finish(&names)
    }
}

fn is_thinking_signature_error(error: &ModelError) -> bool {
    error.kind == ModelErrorKind::InvalidRequest
        && error.message.contains("signature")
        && error.message.contains("thinking")
}

fn request_body(
    config: &ModelConfig,
    request: &ModelCompletionRequest,
    stream: bool,
    replay_thinking: bool,
) -> Value {
    let system = request
        .messages
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut messages = api_messages(&request.messages, replay_thinking);
    // Everything up to the newest message is identical on the next request of a tool loop,
    // so cache it; the system prompt (with the tools, which render before it) gets its own
    // read point that survives changes later in the conversation.
    if let Some(block) = messages
        .last_mut()
        .and_then(|message| message["content"].as_array_mut())
        .and_then(|content| content.last_mut())
    {
        block["cache_control"] = json!({ "type": "ephemeral" });
    }

    // Sampling parameters are rejected by current Claude models, so none are sent.
    let mut body = json!({
        "model": config.model,
        "max_tokens": request.max_output_tokens.unwrap_or(config.max_output_tokens),
        "messages": messages,
    });
    if !system.is_empty() {
        body["system"] = json!([{
            "type": "text",
            "text": system,
            "cache_control": { "type": "ephemeral" },
        }]);
    }
    if !request.tools.is_empty() {
        body["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| json!({
                "name": wire::api_tool_name(&tool.id),
                "description": wire::tool_description(tool),
                "input_schema": tool.input_schema,
            }))
            .collect::<Vec<_>>());
        if request.tool_choice == ModelToolChoice::None {
            body["tool_choice"] = json!({ "type": "none" });
        }
    }
    if stream {
        body["stream"] = json!(true);
    }
    body
}

/// The conversation as alternating user and assistant turns: tool results travel in the user
/// turn that follows the assistant turn that called them.
fn api_messages(messages: &[ModelMessage], replay_thinking: bool) -> Vec<Value> {
    let mut turns: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for message in messages {
        let (role, blocks) = match message.role {
            ModelMessageRole::System => continue,
            ModelMessageRole::User => ("user", user_blocks(message)),
            ModelMessageRole::Tool => ("user", vec![tool_result_block(message)]),
            ModelMessageRole::Assistant => {
                ("assistant", assistant_blocks(message, replay_thinking))
            }
        };
        if blocks.is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => last_blocks.extend(blocks),
            _ => turns.push((role, blocks)),
        }
    }
    if turns.first().is_some_and(|(role, _)| *role == "assistant") {
        turns.insert(
            0,
            (
                "user",
                vec![json!({ "type": "text", "text": "(The conversation starts with a message from the assistant.)" })],
            ),
        );
    }
    if turns.last().is_none_or(|(role, _)| *role == "assistant") {
        turns.push(("user", vec![json!({ "type": "text", "text": "Continue." })]));
    }
    turns
        .into_iter()
        .map(|(role, content)| json!({ "role": role, "content": content }))
        .collect()
}

fn user_blocks(message: &ModelMessage) -> Vec<Value> {
    // Images go before the text that refers to them.
    let mut blocks = message
        .images
        .iter()
        .map(|image| {
            json!({
                "type": "image",
                "source": { "type": "base64", "media_type": image.mime_type, "data": image.data_base64 },
            })
        })
        .collect::<Vec<_>>();
    if !message.content.trim().is_empty() {
        blocks.push(json!({ "type": "text", "text": message.content }));
    }
    blocks
}

fn tool_result_block(message: &ModelMessage) -> Value {
    let mut block = json!({
        "type": "tool_result",
        "tool_use_id": message.tool_result.as_ref().map(|result| api_call_id(&result.call_id)).unwrap_or_default(),
        "content": if message.content.is_empty() { "(no output)" } else { message.content.as_str() },
    });
    if message
        .tool_result
        .as_ref()
        .is_some_and(|result| result.is_error)
    {
        block["is_error"] = json!(true);
    }
    block
}

fn assistant_blocks(message: &ModelMessage, replay_thinking: bool) -> Vec<Value> {
    let tool_use = |call: &ModelToolCall, index: usize| {
        json!({
            "type": "tool_use",
            "id": api_call_id(call.call_id.as_deref().unwrap_or(&format!("call_{index}"))),
            "name": wire::api_tool_name(&call.tool_id),
            "input": if call.arguments.is_object() { call.arguments.clone() } else { json!({}) },
        })
    };
    // The blocks this API returned go back exactly as they came, thinking included: that is
    // what keeps thinking signatures valid and the prompt cache warm. A tool call the loop
    // did not answer is left out, because every `tool_use` needs its `tool_result`.
    if replay_thinking
        && message.provider == PROVIDER_ANTHROPIC
        && !message.provider_items.is_empty()
    {
        let answered = |id: &str| {
            message
                .tool_calls
                .iter()
                .any(|call| call.call_id.as_deref().map(api_call_id).as_deref() == Some(id))
        };
        let mut blocks = message
            .provider_items
            .iter()
            .filter(|block| {
                block["type"] != "tool_use" || block["id"].as_str().is_some_and(answered)
            })
            .cloned()
            .collect::<Vec<_>>();
        for (index, call) in message.tool_calls.iter().enumerate() {
            let id = call.call_id.as_deref().map(api_call_id);
            if !blocks
                .iter()
                .any(|block| block["type"] == "tool_use" && block["id"].as_str() == id.as_deref())
            {
                blocks.push(tool_use(call, index));
            }
        }
        return blocks;
    }
    let mut blocks = Vec::new();
    if !message.content.trim().is_empty() {
        blocks.push(json!({ "type": "text", "text": message.content }));
    }
    blocks.extend(
        message
            .tool_calls
            .iter()
            .enumerate()
            .map(|(index, call)| tool_use(call, index)),
    );
    blocks
}

/// Tool-use ids must match `^[a-zA-Z0-9_-]+$`; ids from other providers may not.
fn api_call_id(call_id: &str) -> String {
    let id = call_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if id.is_empty() {
        "call".into()
    } else {
        id
    }
}

/// The assistant message, assembled from a whole response or from stream events.
#[derive(Default)]
struct AssistantMessage {
    content: Vec<Value>,
    /// JSON text of each `tool_use` input while it streams, by block index.
    partial_inputs: HashMap<usize, String>,
    stop_reason: Option<String>,
    refusal: Option<String>,
    usage: Usage,
}

#[derive(Default)]
struct Usage {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
    seen: bool,
}

impl Usage {
    fn observe(&mut self, usage: &Value) {
        let mut set = |target: &mut u64, key: &str| {
            if let Some(value) = usage.get(key).and_then(Value::as_u64) {
                *target = value;
                self.seen = true;
            }
        };
        set(&mut self.input, "input_tokens");
        set(&mut self.cache_creation, "cache_creation_input_tokens");
        set(&mut self.cache_read, "cache_read_input_tokens");
        set(&mut self.output, "output_tokens");
    }
}

impl AssistantMessage {
    /// Reads the fields shared by a whole response and the stream's `message_start`.
    fn apply_message(&mut self, message: &Value) {
        if let Some(usage) = message.get("usage") {
            self.usage.observe(usage);
        }
        self.observe_stop(message);
    }

    fn observe_stop(&mut self, value: &Value) {
        if let Some(reason) = value.get("stop_reason").and_then(Value::as_str) {
            self.stop_reason = Some(reason.to_string());
        }
        if let Some(details) = value
            .get("stop_details")
            .filter(|details| !details.is_null())
        {
            self.refusal = Some(
                details
                    .get("explanation")
                    .and_then(Value::as_str)
                    .or_else(|| details.get("category").and_then(Value::as_str))
                    .unwrap_or("no reason given")
                    .to_string(),
            );
        }
    }

    /// Folds one stream event in and returns the text it added.
    fn apply_event(&mut self, event: &Value) -> Result<String, ModelError> {
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => self.apply_message(&event["message"]),
            "content_block_start" => {
                let index = block_index(event);
                while self.content.len() <= index {
                    self.content.push(Value::Null);
                }
                self.content[index] = event["content_block"].clone();
            }
            "content_block_delta" => {
                let index = block_index(event);
                let delta = &event["delta"];
                let Some(block) = self.content.get_mut(index).and_then(Value::as_object_mut) else {
                    return Ok(String::new());
                };
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or_default();
                        append(block, "text", text);
                        return Ok(text.to_string());
                    }
                    "thinking_delta" => append(
                        block,
                        "thinking",
                        delta["thinking"].as_str().unwrap_or_default(),
                    ),
                    "signature_delta" => append(
                        block,
                        "signature",
                        delta["signature"].as_str().unwrap_or_default(),
                    ),
                    "input_json_delta" => self
                        .partial_inputs
                        .entry(index)
                        .or_default()
                        .push_str(delta["partial_json"].as_str().unwrap_or_default()),
                    _ => {}
                }
            }
            "message_delta" => {
                self.observe_stop(&event["delta"]);
                if let Some(usage) = event.get("usage") {
                    self.usage.observe(usage);
                }
            }
            "error" => {
                let kind = match event.pointer("/error/type").and_then(Value::as_str) {
                    Some("overloaded_error") => ModelErrorKind::Overloaded,
                    Some("rate_limit_error") => ModelErrorKind::RateLimited,
                    Some("invalid_request_error") => ModelErrorKind::InvalidRequest,
                    _ => ModelErrorKind::Server,
                };
                return Err(ModelError::new(
                    kind,
                    format!(
                        "{LABEL} stream failed: {}",
                        event
                            .pointer("/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                    ),
                ));
            }
            _ => {}
        }
        Ok(String::new())
    }

    fn finish(
        mut self,
        names: &HashMap<String, String>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        let truncated = self.stop_reason.as_deref() == Some("max_tokens");
        for (index, raw) in std::mem::take(&mut self.partial_inputs) {
            let Some(block) = self.content.get_mut(index) else {
                continue;
            };
            let name = block["name"].as_str().unwrap_or_default().to_string();
            block["input"] = wire::parse_tool_arguments(LABEL, &name, &raw, truncated)?;
        }
        self.content.retain(|block| !block.is_null());
        if self.stop_reason.as_deref() == Some("refusal") {
            return Err(ModelError::new(
                ModelErrorKind::Refused,
                format!(
                    "Claude declined this request ({}).",
                    self.refusal.as_deref().unwrap_or("no reason given")
                ),
            ));
        }

        let text = self
            .content
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
        let tool_calls = self
            .content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .map(|block| ModelToolCall {
                call_id: block["id"].as_str().map(str::to_string),
                tool_id: wire::tool_id_for(names, block["name"].as_str().unwrap_or_default()),
                arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                provider_meta: Value::Null,
            })
            .collect::<Vec<_>>();
        if text.trim().is_empty() && tool_calls.is_empty() {
            return Err(ModelError::protocol(if truncated {
                format!("{LABEL} output hit the token limit before any text or tool call. Raise the max output tokens.")
            } else {
                format!("{LABEL} adapter returned empty content.")
            }));
        }
        let stop_reason = match self.stop_reason.as_deref() {
            _ if !tool_calls.is_empty() => ModelStopReason::ToolCalls,
            Some("max_tokens") => ModelStopReason::MaxTokens,
            Some("end_turn" | "stop_sequence") | None => ModelStopReason::Complete,
            Some(_) => ModelStopReason::Other,
        };
        let usage = self.usage.seen.then_some(ModelUsage {
            // `input_tokens` is only the part after the last cache breakpoint.
            input_tokens: self.usage.input + self.usage.cache_creation + self.usage.cache_read,
            output_tokens: self.usage.output,
            cached_input_tokens: self.usage.cache_read,
            reasoning_tokens: 0,
        });
        // Text-only answers end the tool round, so there is nothing to replay them into.
        let provider_items = if tool_calls.is_empty() {
            Vec::new()
        } else {
            self.content
        };
        Ok(ModelCompletionResponse {
            raw_output: text,
            tool_calls,
            stop_reason,
            usage,
            provider: PROVIDER_ANTHROPIC.into(),
            provider_items,
        })
    }
}

fn block_index(event: &Value) -> usize {
    event["index"].as_u64().unwrap_or(0) as usize
}

fn append(block: &mut Map<String, Value>, key: &str, text: &str) {
    let mut current = block
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    current.push_str(text);
    block.insert(key.into(), Value::String(current));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{scripted_server, Reply};
    use crate::{ModelInlineImage, ModelToolDefinition};

    fn config(base_url: String) -> ModelConfig {
        ModelConfig {
            provider_kind: ModelProviderKind::AnthropicMessages,
            base_url,
            model: "claude-test".into(),
            api_key: "key".into(),
            ..ModelConfig::seeded()
        }
    }

    fn request(messages: Vec<ModelMessage>) -> ModelCompletionRequest {
        ModelCompletionRequest {
            messages,
            temperature: 0.2,
            max_output_tokens: Some(512),
            tools: vec![ModelToolDefinition {
                id: "workspace.read".into(),
                name: "Read".into(),
                description: "Reads a file".into(),
                input_schema: json!({ "type": "object" }),
            }],
            tool_choice: ModelToolChoice::Auto,
            session_id: None,
            turn_id: None,
            request_name: None,
        }
    }

    fn tool_round() -> Vec<ModelMessage> {
        let mut user = ModelMessage::user("read a.md");
        user.images.push(ModelInlineImage {
            name: None,
            mime_type: "image/webp".into(),
            data_base64: "QUJD".into(),
        });
        let mut assistant = ModelMessage::assistant("Let me look.");
        assistant.provider = PROVIDER_ANTHROPIC.into();
        assistant.provider_items = vec![
            json!({ "type": "thinking", "thinking": "", "signature": "sig" }),
            json!({ "type": "text", "text": "Let me look." }),
            json!({ "type": "tool_use", "id": "toolu_1", "name": "workspace_read", "input": { "path": "a.md" } }),
            json!({ "type": "tool_use", "id": "toolu_2", "name": "workspace_read", "input": { "path": "b.md" } }),
        ];
        assistant.tool_calls.push(ModelToolCall {
            call_id: Some("toolu_1".into()),
            tool_id: "workspace.read".into(),
            arguments: json!({ "path": "a.md" }),
            provider_meta: Value::Null,
        });
        vec![
            ModelMessage::system("be brief"),
            ModelMessage::assistant("Hi, I am Daily."),
            user,
            assistant,
            ModelMessage::tool_result("toolu_1", "workspace.read", "alpha", false),
            ModelMessage::user("Tool budget reached."),
        ]
    }

    #[test]
    fn requests_replay_returned_blocks_and_group_tool_results_into_the_user_turn() {
        let body = request_body(
            &config("http://x".into()),
            &request(tool_round()),
            true,
            true,
        );
        assert!(body.get("temperature").is_none());
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"][0]["text"], "be brief");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][0]["name"], "workspace_read");
        let messages = body["messages"].as_array().unwrap();
        // A leading assistant message gets a user turn in front of it.
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["content"][0]["text"], "Hi, I am Daily.");
        assert_eq!(messages[2]["content"][0]["type"], "image");
        assert_eq!(messages[2]["content"][1]["text"], "read a.md");
        // Thinking and text come back verbatim; the unanswered tool_use is dropped.
        let assistant = messages[3]["content"].as_array().unwrap();
        assert_eq!(assistant.len(), 3);
        assert_eq!(assistant[0]["signature"], "sig");
        assert_eq!(assistant[2]["id"], "toolu_1");
        // The tool result and the note after it share one user turn, result first.
        let results = messages[4]["content"].as_array().unwrap();
        assert_eq!(results[0]["type"], "tool_result");
        assert_eq!(results[0]["tool_use_id"], "toolu_1");
        assert_eq!(results[1]["text"], "Tool budget reached.");
        assert_eq!(results[1]["cache_control"]["type"], "ephemeral");
        assert_eq!(messages.len(), 5);

        // Without thinking replay the turn is rebuilt from text and calls.
        let body = request_body(
            &config("http://x".into()),
            &request(tool_round()),
            false,
            false,
        );
        let assistant = body["messages"][3]["content"].as_array().unwrap();
        assert_eq!(assistant.len(), 2);
        assert_eq!(assistant[0]["type"], "text");
        assert_eq!(assistant[1]["name"], "workspace_read");
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn tool_choice_none_keeps_the_tools_declared() {
        let mut request = request(vec![ModelMessage::user("answer now")]);
        request.tool_choice = ModelToolChoice::None;
        let body = request_body(&config("http://x".into()), &request, false, true);
        assert_eq!(body["tool_choice"]["type"], "none");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn streams_text_thinking_and_tool_use_with_usage() {
        let (url, requests) = scripted_server(vec![Reply::sse(&[
            json!({ "type": "message_start", "message": { "usage": { "input_tokens": 10, "cache_read_input_tokens": 90, "cache_creation_input_tokens": 5, "output_tokens": 1 } } }),
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "", "signature": "" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "sig" } }),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text", "text": "" } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "Look" } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "ing" } }),
            json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "toolu_1", "name": "workspace_read", "input": {} } }),
            json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"path\":" } }),
            json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "\"a.md\"}" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 42 } }),
            json!({ "type": "message_stop" }),
        ])])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let mut chunks = Vec::new();
        let response = AnthropicMessagesAdapter
            .complete(
                &http,
                &config(url),
                &request(vec![ModelMessage::user("read a.md")]),
                Some(&mut |chunk| {
                    chunks.push(chunk.text);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert_eq!(chunks, vec!["Look", "ing"]);
        assert_eq!(response.raw_output, "Looking");
        assert_eq!(response.stop_reason, ModelStopReason::ToolCalls);
        assert_eq!(response.tool_calls[0].tool_id, "workspace.read");
        assert_eq!(response.tool_calls[0].arguments, json!({ "path": "a.md" }));
        let usage = response.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.output_tokens
            ),
            (105, 90, 42)
        );
        // The whole content array is kept for replay, with the streamed pieces assembled.
        assert_eq!(response.provider, PROVIDER_ANTHROPIC);
        assert_eq!(response.provider_items[0]["signature"], "sig");
        assert_eq!(response.provider_items[2]["input"]["path"], "a.md");
        let recorded = requests.lock().unwrap();
        assert!(recorded[0].head.contains("x-api-key: key"));
        assert!(recorded[0].head.contains("anthropic-version: 2023-06-01"));
    }

    #[tokio::test]
    async fn a_rejected_thinking_signature_is_retried_once_without_thinking() {
        let (url, requests) = scripted_server(vec![
            Reply::error(400, "{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"messages.3.content.0: Invalid `signature` in `thinking` block. The block is bound to a different conversation.\"}}"),
            Reply::json(json!({ "content": [{ "type": "text", "text": "done" }], "stop_reason": "end_turn", "usage": { "input_tokens": 7, "output_tokens": 2 } })),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let response = AnthropicMessagesAdapter
            .complete(&http, &config(url), &request(tool_round()), None)
            .await
            .unwrap();
        assert_eq!(response.raw_output, "done");
        assert_eq!(response.stop_reason, ModelStopReason::Complete);
        assert!(response.provider_items.is_empty());
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert!(recorded[0].body["messages"]
            .to_string()
            .contains("thinking"));
        assert!(!recorded[1].body["messages"]
            .to_string()
            .contains("thinking"));
    }

    #[tokio::test]
    async fn an_output_limit_above_the_models_is_lowered_and_retried() {
        let (url, requests) = scripted_server(vec![
            Reply::error(400, "{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"max_tokens: 512 > 300, which is the maximum allowed number of output tokens for claude-test\"}}"),
            Reply::json(json!({ "content": [{ "type": "text", "text": "ok" }], "stop_reason": "end_turn" })),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let response = AnthropicMessagesAdapter
            .complete(
                &http,
                &config(url),
                &request(vec![ModelMessage::user("go")]),
                None,
            )
            .await
            .unwrap();
        assert_eq!(response.raw_output, "ok");
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded[0].body["max_tokens"], 512);
        assert_eq!(recorded[1].body["max_tokens"], 300);
    }

    #[tokio::test]
    async fn refusals_and_overloads_are_typed() {
        let (url, _) = scripted_server(vec![
            Reply::json(json!({ "content": [], "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": "cyber" } })),
            Reply::error(529, "{\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}"),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let request = request(vec![ModelMessage::user("go")]);
        let refused = AnthropicMessagesAdapter
            .complete(&http, &config(url.clone()), &request, None)
            .await
            .err()
            .unwrap();
        assert_eq!(refused.kind, ModelErrorKind::Refused);
        assert!(refused.message.contains("cyber"));
        let overloaded = AnthropicMessagesAdapter
            .complete(&http, &config(url), &request, None)
            .await
            .err()
            .unwrap();
        assert_eq!(overloaded.kind, ModelErrorKind::Overloaded);
        assert!(overloaded.is_retryable());
    }
}
