//! Gemini API (`generateContent`): native function calling, streaming and thought signatures.
use super::wire::{self, SseEvents};
use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelError,
    ModelErrorKind, ModelMessage, ModelMessageRole, ModelProviderKind, ModelStopReason,
    ModelStreamChunk, ModelToolCall, ModelToolChoice, ModelUsage, PROVIDER_GEMINI,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

const LABEL: &str = "Gemini";

pub struct GeminiApiAdapter;

#[async_trait]
impl ModelAdapter for GeminiApiAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::GeminiApi
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
        if config.api_key.trim().is_empty() {
            return Err(ModelError::new(
                ModelErrorKind::Auth,
                "Gemini API key is missing. Set GEMINI_API_KEY or GOOGLE_API_KEY, or add apiKey or apiKeyEnv to this provider in settings.",
            ));
        }
        let stream = on_chunk.is_some();
        let endpoint = format!(
            "{}/models/{}:{}",
            config.base_url.trim_end_matches('/'),
            config.model,
            if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            }
        );
        let mut body = request_body(request);
        let send = |body: &Value| {
            let builder = http
                .post(&endpoint)
                .json(body)
                .header("x-goog-api-key", &config.api_key);
            wire::send(LABEL, builder, stream)
        };
        let response = match send(&body).await {
            // The configured output budget can exceed what this model allows.
            Err(error) => {
                let requested = body["generationConfig"]["maxOutputTokens"]
                    .as_u64()
                    .unwrap_or(0) as u32;
                let config = body["generationConfig"]
                    .as_object_mut()
                    .expect("generationConfig is an object");
                match wire::lower_output_limit(&error, requested) {
                    Ok(Some(allowed)) => config.insert("maxOutputTokens".into(), json!(allowed)),
                    Ok(None) => config.remove("maxOutputTokens"),
                    Err(()) => return Err(error),
                };
                send(&body).await?
            }
            Ok(response) => response,
        };

        let names = wire::tool_name_map(&request.tools);
        let mut message = AssistantMessage::default();
        if let Some(on_chunk) = on_chunk.as_mut() {
            let mut events = SseEvents::new(LABEL, response);
            while let Some(data) = events.next().await? {
                let payload: Value = serde_json::from_str(&data).map_err(|error| {
                    ModelError::protocol(format!("Failed to parse {LABEL} stream chunk: {error}"))
                })?;
                let text = message.apply(&payload, &names);
                if !text.is_empty() {
                    on_chunk(ModelStreamChunk { text }).map_err(ModelError::aborted)?;
                }
            }
        } else {
            let payload: Value = response.json().await.map_err(|error| {
                ModelError::protocol(format!("Failed to parse {LABEL} response: {error}"))
            })?;
            message.apply(&payload, &names);
        }
        message.finish()
    }
}

fn request_body(request: &ModelCompletionRequest) -> Value {
    let system_instruction = request
        .messages
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut body = json!({
        "contents": contents(&request.messages),
        "generationConfig": {
            "temperature": request.temperature,
            "maxOutputTokens": request.max_output_tokens.unwrap_or(8192),
        }
    });
    if !system_instruction.is_empty() {
        body["system_instruction"] = json!({ "parts": [{ "text": system_instruction }] });
    }
    if !request.tools.is_empty() {
        body["tools"] = json!([{
            "functionDeclarations": request.tools.iter().map(|tool| json!({
                "name": wire::api_tool_name(&tool.id),
                "description": wire::tool_description(tool),
                "parameters": function_schema(&tool.input_schema),
            })).collect::<Vec<_>>(),
        }]);
        if request.tool_choice == ModelToolChoice::None {
            body["toolConfig"] = json!({ "functionCallingConfig": { "mode": "NONE" } });
        }
    }
    body
}

/// The conversation as alternating `user` and `model` contents. Function responses go in the
/// user content right after the model content that called them, all of one round together.
fn contents(messages: &[ModelMessage]) -> Vec<Value> {
    let mut contents: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for message in messages {
        let (role, parts) = match message.role {
            ModelMessageRole::System => continue,
            ModelMessageRole::User => ("user", user_parts(message)),
            ModelMessageRole::Tool => ("user", vec![function_response_part(message)]),
            ModelMessageRole::Assistant => ("model", model_parts(message)),
        };
        if parts.is_empty() {
            continue;
        }
        match contents.last_mut() {
            Some((last_role, last_parts)) if *last_role == role => last_parts.extend(parts),
            _ => contents.push((role, parts)),
        }
    }
    contents
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect()
}

fn user_parts(message: &ModelMessage) -> Vec<Value> {
    let mut parts = Vec::new();
    if !message.content.trim().is_empty() {
        parts.push(json!({ "text": message.content }));
    }
    parts.extend(message.images.iter().map(|image| {
        json!({ "inline_data": { "mime_type": image.mime_type, "data": image.data_base64 } })
    }));
    parts
}

fn model_parts(message: &ModelMessage) -> Vec<Value> {
    let from_gemini = message.provider == PROVIDER_GEMINI;
    let mut parts = Vec::new();
    if !message.content.trim().is_empty() {
        let mut part = json!({ "text": message.content });
        // A signature on the text part travels in the message's provider items.
        if let Some(signature) = message
            .provider_items
            .first()
            .filter(|_| from_gemini)
            .and_then(|item| item.get("thoughtSignature"))
        {
            part["thoughtSignature"] = signature.clone();
        }
        parts.push(part);
    }
    for call in &message.tool_calls {
        let mut function_call = json!({
            "name": wire::api_tool_name(&call.tool_id),
            "args": if call.arguments.is_object() { call.arguments.clone() } else { json!({}) },
        });
        let mut part = json!({});
        if from_gemini {
            if let Some(id) = call.provider_meta.get("id") {
                function_call["id"] = id.clone();
            }
            // Thinking models reject a replayed function call without the signature it
            // was returned with.
            if let Some(signature) = call.provider_meta.get("thoughtSignature") {
                part["thoughtSignature"] = signature.clone();
            }
        }
        part["functionCall"] = function_call;
        parts.push(part);
    }
    parts
}

fn function_response_part(message: &ModelMessage) -> Value {
    let result = message.tool_result.as_ref();
    let key = if result.is_some_and(|result| result.is_error) {
        "error"
    } else {
        "result"
    };
    json!({
        "functionResponse": {
            "name": wire::api_tool_name(result.map(|result| result.tool_id.as_str()).unwrap_or_default()),
            "response": { key: message.content },
        }
    })
}

/// Function parameters accept only a subset of JSON Schema; unknown keywords are an error.
fn function_schema(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return json!({ "type": "object" });
    };
    let mut result = serde_json::Map::new();
    for key in ["type", "description", "enum", "format", "nullable"] {
        if let Some(value) = object.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        result.insert(
            "properties".into(),
            Value::Object(
                properties
                    .iter()
                    .map(|(key, value)| (key.clone(), function_schema(value)))
                    .collect(),
            ),
        );
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            result.insert(
                "required".into(),
                json!(required
                    .iter()
                    .filter(|value| value
                        .as_str()
                        .is_some_and(|key| properties.contains_key(key)))
                    .collect::<Vec<_>>()),
            );
        }
    }
    if let Some(items) = object.get("items") {
        result.insert("items".into(), function_schema(items));
    }
    if !result.contains_key("type") {
        result.insert("type".into(), json!("object"));
    }
    Value::Object(result)
}

/// The model's message, assembled from a whole response or from stream chunks.
#[derive(Default)]
struct AssistantMessage {
    text: String,
    text_signature: Option<Value>,
    tool_calls: Vec<ModelToolCall>,
    finish_reason: Option<String>,
    block_reason: Option<String>,
    usage: Option<ModelUsage>,
}

impl AssistantMessage {
    /// Folds one response payload in and returns the visible text it added.
    fn apply(&mut self, payload: &Value, names: &HashMap<String, String>) -> String {
        if let Some(reason) = payload
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(reason.to_string());
        }
        if let Some(reason) = payload
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            self.block_reason = Some(reason.to_string());
        }
        if let Some(usage) = payload.get("usageMetadata") {
            let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            let thoughts = count("thoughtsTokenCount");
            self.usage = Some(ModelUsage {
                input_tokens: count("promptTokenCount"),
                output_tokens: count("candidatesTokenCount") + thoughts,
                cached_input_tokens: count("cachedContentTokenCount"),
                reasoning_tokens: thoughts,
            });
        }
        let mut added = String::new();
        for part in payload
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(call) = part.get("functionCall") {
                let mut provider_meta = serde_json::Map::new();
                if let Some(id) = call.get("id").filter(|id| id.is_string()) {
                    provider_meta.insert("id".into(), id.clone());
                }
                if let Some(signature) = part.get("thoughtSignature") {
                    provider_meta.insert("thoughtSignature".into(), signature.clone());
                }
                self.tool_calls.push(ModelToolCall {
                    call_id: call.get("id").and_then(Value::as_str).map(str::to_string),
                    tool_id: wire::tool_id_for(
                        names,
                        call.get("name").and_then(Value::as_str).unwrap_or_default(),
                    ),
                    arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
                    provider_meta: if provider_meta.is_empty() {
                        Value::Null
                    } else {
                        Value::Object(provider_meta)
                    },
                });
                continue;
            }
            // Thought summaries are not part of the answer.
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                added.push_str(text);
                if let Some(signature) = part.get("thoughtSignature") {
                    self.text_signature = Some(signature.clone());
                }
            }
        }
        self.text.push_str(&added);
        added
    }

    fn finish(self) -> Result<ModelCompletionResponse, ModelError> {
        let finish = self.finish_reason.as_deref();
        if self.text.trim().is_empty() && self.tool_calls.is_empty() {
            let message = match (self.block_reason.as_deref(), finish) {
                (Some(reason), _) => {
                    return Err(ModelError::new(
                        ModelErrorKind::Refused,
                        format!("{LABEL} blocked this request ({reason})."),
                    ))
                }
                (None, Some("MAX_TOKENS")) => format!("{LABEL} output hit the token limit before any text or tool call. Raise the max output tokens."),
                (None, Some("MALFORMED_FUNCTION_CALL")) => format!("{LABEL} produced an invalid tool call (MALFORMED_FUNCTION_CALL). Try again or switch models."),
                (None, Some(reason @ ("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII"))) => {
                    return Err(ModelError::new(
                        ModelErrorKind::Refused,
                        format!("{LABEL} blocked the generated content ({reason})."),
                    ))
                }
                _ => format!("{LABEL} adapter returned empty content."),
            };
            return Err(ModelError::protocol(message));
        }
        let stop_reason = match finish {
            _ if !self.tool_calls.is_empty() => ModelStopReason::ToolCalls,
            Some("MAX_TOKENS") => ModelStopReason::MaxTokens,
            Some("STOP") | None => ModelStopReason::Complete,
            Some(_) => ModelStopReason::Other,
        };
        Ok(ModelCompletionResponse {
            raw_output: self.text,
            tool_calls: self.tool_calls,
            stop_reason,
            usage: self.usage,
            provider: PROVIDER_GEMINI.into(),
            provider_items: self
                .text_signature
                .map(|signature| vec![json!({ "thoughtSignature": signature })])
                .unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{scripted_server, Reply};
    use crate::{ModelInlineImage, ModelToolDefinition};

    fn config(base_url: String) -> ModelConfig {
        ModelConfig {
            provider_kind: ModelProviderKind::GeminiApi,
            base_url,
            model: "gemini-test".into(),
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
                input_schema: json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": { "path": { "type": "string", "minLength": 1 } },
                    "required": ["path", "ghost"],
                }),
            }],
            tool_choice: ModelToolChoice::Auto,
            session_id: None,
            turn_id: None,
            request_name: None,
        }
    }

    #[test]
    fn requests_use_function_calls_responses_and_signatures() {
        let mut user = ModelMessage::user("read both");
        user.images.push(ModelInlineImage {
            name: None,
            mime_type: "image/png".into(),
            data_base64: "QUJD".into(),
        });
        let mut assistant = ModelMessage::assistant("");
        assistant.provider = PROVIDER_GEMINI.into();
        for (path, signature) in [("a.md", Some("sig-a")), ("b.md", None)] {
            assistant.tool_calls.push(ModelToolCall {
                call_id: Some(format!("call_{path}")),
                tool_id: "workspace.read".into(),
                arguments: json!({ "path": path }),
                provider_meta: signature
                    .map(|signature| json!({ "thoughtSignature": signature }))
                    .unwrap_or(Value::Null),
            });
        }
        let mut request = request(vec![
            ModelMessage::system("be brief"),
            user,
            assistant,
            ModelMessage::tool_result("call_a.md", "workspace.read", "alpha", false),
            ModelMessage::tool_result("call_b.md", "workspace.read", "missing", true),
        ]);
        let body = request_body(&request);
        assert_eq!(body["system_instruction"]["parts"][0]["text"], "be brief");
        let declaration = &body["tools"][0]["functionDeclarations"][0];
        assert_eq!(declaration["name"], "workspace_read");
        // Unsupported schema keywords and unknown required keys are dropped.
        assert!(declaration["parameters"]
            .get("additionalProperties")
            .is_none());
        assert!(declaration["parameters"]["properties"]["path"]
            .get("minLength")
            .is_none());
        assert_eq!(declaration["parameters"]["required"], json!(["path"]));
        assert!(body.get("toolConfig").is_none());

        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(
            contents[0]["parts"][1]["inline_data"]["mime_type"],
            "image/png"
        );
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(
            contents[1]["parts"][0]["functionCall"]["name"],
            "workspace_read"
        );
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "sig-a");
        assert!(contents[1]["parts"][1].get("thoughtSignature").is_none());
        // Both results answer in one user content, in call order.
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(
            contents[2]["parts"][0]["functionResponse"]["response"]["result"],
            "alpha"
        );
        assert_eq!(
            contents[2]["parts"][1]["functionResponse"]["response"]["error"],
            "missing"
        );

        request.tool_choice = ModelToolChoice::None;
        assert_eq!(
            request_body(&request)["toolConfig"]["functionCallingConfig"]["mode"],
            "NONE"
        );
    }

    #[tokio::test]
    async fn streams_text_function_calls_and_usage() {
        let (url, requests) = scripted_server(vec![Reply::sse(&[
            json!({ "candidates": [{ "content": { "parts": [{ "text": "hidden", "thought": true }, { "text": "Look" }] } }] }),
            json!({ "candidates": [{ "content": { "parts": [{ "text": "ing" }] } }] }),
            json!({ "candidates": [{ "content": { "parts": [
                { "functionCall": { "name": "workspace_read", "args": { "path": "a.md" } }, "thoughtSignature": "sig" }
            ] }, "finishReason": "STOP" }],
              "usageMetadata": { "promptTokenCount": 50, "candidatesTokenCount": 8, "thoughtsTokenCount": 4, "cachedContentTokenCount": 30 } }),
        ])])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let mut chunks = Vec::new();
        let response = GeminiApiAdapter
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
        assert_eq!(
            response.tool_calls[0].provider_meta["thoughtSignature"],
            "sig"
        );
        let usage = response.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
                usage.reasoning_tokens
            ),
            (50, 12, 30, 4)
        );
        let recorded = requests.lock().unwrap();
        assert!(recorded[0].head.contains("streamgeneratecontent?alt=sse"));
        assert!(recorded[0].head.contains("x-goog-api-key: key"));
    }

    #[tokio::test]
    async fn empty_and_blocked_responses_are_typed() {
        let (url, _) = scripted_server(vec![
            Reply::json(json!({ "candidates": [{ "content": { "parts": [] }, "finishReason": "MAX_TOKENS" }] })),
            Reply::json(json!({ "promptFeedback": { "blockReason": "SAFETY" } })),
            Reply::json(json!({ "candidates": [{ "content": { "parts": [{ "text": "cut" }] }, "finishReason": "MAX_TOKENS" }] })),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let request = request(vec![ModelMessage::user("go")]);
        let config = config(url);
        let run = || GeminiApiAdapter.complete(&http, &config, &request, None);
        assert!(run().await.err().unwrap().message.contains("token limit"));
        assert_eq!(run().await.err().unwrap().kind, ModelErrorKind::Refused);
        assert_eq!(run().await.unwrap().stop_reason, ModelStopReason::MaxTokens);
    }
}
