//! OpenAI-compatible Chat Completions: OpenAI, local servers and gateways.
use super::wire::{self, SseEvents};
use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelError,
    ModelErrorKind, ModelMessage, ModelMessageRole, ModelProviderKind, ModelStopReason,
    ModelStreamChunk, ModelToolCall, ModelToolChoice, ModelToolDefinition, ModelUsage,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

const LABEL: &str = "OpenAI-compatible";

pub struct OpenAiCompatibleAdapter;

#[async_trait]
impl ModelAdapter for OpenAiCompatibleAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::OpenAiCompatible
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
        let endpoint = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
        let stream = on_chunk.is_some();
        let mut body = request_body(config, request, stream);
        let names = wire::tool_name_map(&request.tools);
        let send = |body: &Value| {
            let builder = http.post(&endpoint).json(body);
            let builder = if config.api_key.is_empty() {
                builder
            } else {
                builder.bearer_auth(&config.api_key)
            };
            wire::send(LABEL, builder, stream)
        };
        // A rejected request is adjusted and sent again, once per kind of rejection.
        let mut adjusted_stream_options = false;
        let mut adjusted_limit = false;
        let response = loop {
            let error = match send(&body).await {
                Ok(response) => break response,
                Err(error) => error,
            };
            let object = body.as_object_mut().expect("request body is an object");
            // Usage in the stream is an OpenAI extension some compatible servers reject.
            if !adjusted_stream_options
                && error.kind == ModelErrorKind::InvalidRequest
                && error.message.contains("stream_options")
                && object.remove("stream_options").is_some()
            {
                adjusted_stream_options = true;
                continue;
            }
            // The configured output budget can exceed what this model allows.
            let limit_key = output_limit_key(config);
            let requested = object.get(limit_key).and_then(Value::as_u64);
            match (adjusted_limit, requested) {
                (false, Some(requested)) => {
                    match wire::lower_output_limit(&error, requested as u32) {
                        Ok(Some(allowed)) => object.insert(limit_key.into(), json!(allowed)),
                        Ok(None) => object.remove(limit_key),
                        Err(()) => return Err(error),
                    };
                    adjusted_limit = true;
                }
                _ => return Err(error),
            }
        };

        let mut message = AssistantMessage::default();
        if let Some(on_chunk) = on_chunk.as_mut() {
            let mut events = SseEvents::new(LABEL, response);
            while let Some(data) = events.next().await? {
                let chunk: Value = serde_json::from_str(&data).map_err(|error| {
                    ModelError::protocol(format!("Failed to parse {LABEL} stream chunk: {error}"))
                })?;
                if let Some(error) = chunk.get("error") {
                    return Err(ModelError::new(
                        ModelErrorKind::Server,
                        format!("{LABEL} stream failed: {error}"),
                    ));
                }
                let text = message.apply_chunk(&chunk);
                if !text.is_empty() {
                    on_chunk(ModelStreamChunk { text }).map_err(ModelError::aborted)?;
                }
            }
        } else {
            let payload: Value = response.json().await.map_err(|error| {
                ModelError::protocol(format!("Failed to parse {LABEL} response: {error}"))
            })?;
            message.apply_response(&payload)?;
        }
        message.finish(&names)
    }
}

fn request_body(config: &ModelConfig, request: &ModelCompletionRequest, stream: bool) -> Value {
    let mut body = json!({
        "model": config.model,
        "temperature": request.temperature,
        "messages": request.messages.iter().map(chat_message).collect::<Vec<_>>(),
    });
    if let Some(max_tokens) = request.max_output_tokens {
        body[output_limit_key(config)] = json!(max_tokens);
    }
    if !request.tools.is_empty() {
        body["tools"] = json!(request
            .tools
            .iter()
            .map(tool_definition)
            .collect::<Vec<_>>());
        match request.tool_choice {
            ModelToolChoice::Auto => {
                body["tool_choice"] = json!("auto");
                body["parallel_tool_calls"] = json!(true);
            }
            ModelToolChoice::None => body["tool_choice"] = json!("none"),
        }
    }
    if stream {
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
    }
    body
}

/// OpenAI's own endpoint replaced `max_tokens`, and its reasoning models reject the old name;
/// other servers mostly know only the old one.
fn output_limit_key(config: &ModelConfig) -> &'static str {
    if config.base_url.contains("api.openai.com") {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

fn chat_message(message: &ModelMessage) -> Value {
    match message.role {
        ModelMessageRole::System => json!({ "role": "system", "content": message.content }),
        ModelMessageRole::User if message.images.is_empty() => {
            json!({ "role": "user", "content": message.content })
        }
        ModelMessageRole::User => {
            let mut parts = vec![json!({ "type": "text", "text": message.content })];
            parts.extend(message.images.iter().map(|image| {
                json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{};base64,{}", image.mime_type, image.data_base64) },
                })
            }));
            json!({ "role": "user", "content": parts })
        }
        ModelMessageRole::Assistant if message.tool_calls.is_empty() => {
            json!({ "role": "assistant", "content": message.content })
        }
        ModelMessageRole::Assistant => json!({
            "role": "assistant",
            "content": if message.content.is_empty() { Value::Null } else { json!(message.content) },
            "tool_calls": message.tool_calls.iter().enumerate().map(|(index, call)| json!({
                "id": call.call_id.clone().unwrap_or_else(|| format!("call_{index}")),
                "type": "function",
                "function": {
                    "name": wire::api_tool_name(&call.tool_id),
                    "arguments": call.arguments.to_string(),
                },
            })).collect::<Vec<_>>(),
        }),
        ModelMessageRole::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_result.as_ref().map(|result| result.call_id.as_str()).unwrap_or_default(),
            "content": message.content,
        }),
    }
}

fn tool_definition(tool: &ModelToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": wire::api_tool_name(&tool.id),
            "description": wire::tool_description(tool),
            "parameters": tool.input_schema,
        }
    })
}

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

/// The assistant message, assembled from a whole response or from stream chunks.
#[derive(Default)]
struct AssistantMessage {
    text: String,
    calls: Vec<PartialToolCall>,
    finish_reason: Option<String>,
    usage: Option<ModelUsage>,
}

impl AssistantMessage {
    fn apply_response(&mut self, payload: &Value) -> Result<(), ModelError> {
        let choice = payload
            .pointer("/choices/0")
            .ok_or_else(|| ModelError::protocol(format!("{LABEL} response has no choices.")))?;
        self.text = choice
            .pointer("/message/content")
            .map(content_text)
            .unwrap_or_default();
        for call in choice
            .pointer("/message/tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.calls.push(PartialToolCall {
                id: call.get("id").and_then(Value::as_str).map(str::to_string),
                name: string_at(call, "/function/name"),
                arguments: string_at(call, "/function/arguments"),
            });
        }
        self.finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.usage = payload.get("usage").and_then(usage);
        Ok(())
    }

    /// Folds one stream chunk in and returns the text it added.
    fn apply_chunk(&mut self, chunk: &Value) -> String {
        if let Some(usage) = chunk.get("usage").and_then(usage) {
            self.usage = Some(usage);
        }
        let Some(choice) = chunk.pointer("/choices/0") else {
            return String::new();
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        for delta in choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = delta
                .get("index")
                .and_then(Value::as_u64)
                .map(|index| index as usize)
                .unwrap_or(self.calls.len().saturating_sub(1));
            while self.calls.len() <= index {
                self.calls.push(PartialToolCall::default());
            }
            let call = &mut self.calls[index];
            if let Some(id) = delta.get("id").and_then(Value::as_str) {
                call.id = Some(id.to_string());
            }
            call.name.push_str(&string_at(delta, "/function/name"));
            call.arguments
                .push_str(&string_at(delta, "/function/arguments"));
        }
        let text = choice
            .pointer("/delta/content")
            .map(content_text)
            .unwrap_or_default();
        self.text.push_str(&text);
        text
    }

    fn finish(
        self,
        names: &HashMap<String, String>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        let truncated = self.finish_reason.as_deref() == Some("length");
        let tool_calls = self
            .calls
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .map(|call| {
                Ok(ModelToolCall {
                    arguments: wire::parse_tool_arguments(
                        LABEL,
                        &call.name,
                        &call.arguments,
                        truncated,
                    )?,
                    call_id: call.id,
                    tool_id: wire::tool_id_for(names, &call.name),
                    provider_meta: Value::Null,
                })
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        if self.text.trim().is_empty() && tool_calls.is_empty() {
            return Err(ModelError::protocol(if truncated {
                format!("{LABEL} output hit the token limit before any text or tool call. Raise the max output tokens.")
            } else {
                format!("{LABEL} adapter returned empty content.")
            }));
        }
        let stop_reason = match self.finish_reason.as_deref() {
            _ if !tool_calls.is_empty() => ModelStopReason::ToolCalls,
            Some("length") => ModelStopReason::MaxTokens,
            Some("content_filter") => ModelStopReason::Refusal,
            Some("stop") | None => ModelStopReason::Complete,
            Some(_) => ModelStopReason::Other,
        };
        Ok(ModelCompletionResponse {
            raw_output: self.text,
            tool_calls,
            stop_reason,
            usage: self.usage,
            ..ModelCompletionResponse::default()
        })
    }
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn usage(value: &Value) -> Option<ModelUsage> {
    let count = |pointer: &str| value.pointer(pointer).and_then(Value::as_u64);
    Some(ModelUsage {
        input_tokens: count("/prompt_tokens")?,
        output_tokens: count("/completion_tokens").unwrap_or(0),
        cached_input_tokens: count("/prompt_tokens_details/cached_tokens").unwrap_or(0),
        reasoning_tokens: count("/completion_tokens_details/reasoning_tokens").unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{scripted_server, Reply};
    use crate::ModelInlineImage;

    fn config(base_url: String) -> ModelConfig {
        ModelConfig {
            provider_kind: ModelProviderKind::OpenAiCompatible,
            base_url,
            model: "test".into(),
            api_key: "key".into(),
            ..ModelConfig::seeded()
        }
    }

    fn tools() -> Vec<ModelToolDefinition> {
        vec![ModelToolDefinition {
            id: "workspace.read".into(),
            name: "Read".into(),
            description: "Reads a file".into(),
            input_schema: json!({ "type": "object" }),
        }]
    }

    fn request(messages: Vec<ModelMessage>) -> ModelCompletionRequest {
        ModelCompletionRequest {
            messages,
            temperature: 0.2,
            max_output_tokens: Some(512),
            tools: tools(),
            tool_choice: ModelToolChoice::Auto,
            session_id: None,
            turn_id: None,
            request_name: None,
        }
    }

    #[test]
    fn tool_calls_and_results_use_the_native_message_shapes() {
        let mut user = ModelMessage::user("look");
        user.images.push(ModelInlineImage {
            name: None,
            mime_type: "image/png".into(),
            data_base64: "QUJD".into(),
        });
        let mut assistant = ModelMessage::assistant("");
        assistant.tool_calls.push(ModelToolCall {
            call_id: Some("call_1".into()),
            tool_id: "workspace.read".into(),
            arguments: json!({ "path": "a.md" }),
            provider_meta: Value::Null,
        });
        let mut request = request(vec![
            ModelMessage::system("s"),
            user,
            assistant,
            ModelMessage::tool_result("call_1", "workspace.read", "alpha", false),
        ]);
        let body = request_body(&config("http://local/v1".into()), &request, true);
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["tool_choice"], "auto");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages[1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,QUJD"
        );
        assert_eq!(messages[2]["content"], Value::Null);
        assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["name"],
            "workspace_read"
        );
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.md\"}"
        );
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        assert_eq!(messages[3]["content"], "alpha");

        request.tool_choice = ModelToolChoice::None;
        let body = request_body(&config("https://api.openai.com/v1".into()), &request, false);
        assert_eq!(body["tool_choice"], "none");
        assert_eq!(body["max_completion_tokens"], 512);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("stream").is_none());
    }

    #[tokio::test]
    async fn streams_text_tool_calls_usage_and_the_stop_reason() {
        let (url, requests) = scripted_server(vec![Reply::sse(&[
            json!({ "choices": [{ "delta": { "content": "Read" } }] }),
            json!({ "choices": [{ "delta": { "content": "ing" } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "call_9", "function": { "name": "workspace_read", "arguments": "{\"pa" } }] } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "th\":\"a.md\"}" } }] } }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
            json!({ "choices": [], "usage": { "prompt_tokens": 100, "completion_tokens": 20, "prompt_tokens_details": { "cached_tokens": 60 } } }),
        ])])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let mut chunks = Vec::new();
        let response = OpenAiCompatibleAdapter
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
        assert_eq!(chunks, vec!["Read", "ing"]);
        assert_eq!(response.raw_output, "Reading");
        assert_eq!(response.stop_reason, ModelStopReason::ToolCalls);
        assert_eq!(response.tool_calls[0].tool_id, "workspace.read");
        assert_eq!(response.tool_calls[0].call_id.as_deref(), Some("call_9"));
        assert_eq!(response.tool_calls[0].arguments, json!({ "path": "a.md" }));
        let usage = response.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens
            ),
            (100, 20, 60)
        );
        let recorded = requests.lock().unwrap();
        assert!(recorded[0].head.contains("authorization: bearer key"));
        assert_eq!(recorded[0].body["stream"], true);
    }

    #[tokio::test]
    async fn a_cut_off_answer_reports_max_tokens_and_a_cut_off_tool_call_fails() {
        let (url, _) = scripted_server(vec![
            Reply::json(json!({ "choices": [{ "message": { "content": "partial" }, "finish_reason": "length" }] })),
            Reply::json(json!({ "choices": [{ "message": { "content": null, "tool_calls": [
                { "id": "c", "function": { "name": "workspace_read", "arguments": "{\"path\":\"a" } }
            ] }, "finish_reason": "length" }] })),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let request = request(vec![ModelMessage::user("go")]);
        let cut = OpenAiCompatibleAdapter
            .complete(&http, &config(url.clone()), &request, None)
            .await
            .unwrap();
        assert_eq!(cut.stop_reason, ModelStopReason::MaxTokens);
        let error = OpenAiCompatibleAdapter
            .complete(&http, &config(url), &request, None)
            .await
            .err()
            .unwrap();
        assert!(error.message.contains("token limit"), "{}", error.message);
    }

    #[tokio::test]
    async fn an_output_limit_above_the_models_is_lowered_and_retried() {
        let (url, requests) = scripted_server(vec![
            Reply::error(400, "{\"error\":{\"message\":\"max_tokens is too large: 512. This model supports at most 256 completion tokens\"}}"),
            Reply::json(json!({ "choices": [{ "message": { "content": "ok" }, "finish_reason": "stop" }] })),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let response = OpenAiCompatibleAdapter
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
        assert_eq!(recorded[1].body["max_tokens"], 256);
    }

    #[tokio::test]
    async fn servers_without_stream_usage_get_a_second_request_without_it() {
        let (url, requests) = scripted_server(vec![
            Reply::error(400, "{\"error\":\"unknown field stream_options\"}"),
            Reply::sse(&[
                json!({ "choices": [{ "delta": { "content": "ok" }, "finish_reason": "stop" }] }),
            ]),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let response = OpenAiCompatibleAdapter
            .complete(
                &http,
                &config(url),
                &request(vec![ModelMessage::user("go")]),
                Some(&mut |_| Ok(())),
            )
            .await
            .unwrap();
        assert_eq!(response.raw_output, "ok");
        let recorded = requests.lock().unwrap();
        assert!(recorded[1].body.get("stream_options").is_none());
        assert_eq!(recorded[1].body["stream"], true);
    }
}
