use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelProviderKind,
    ModelStreamChunk, ModelToolCall, ModelToolDefinition,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct OpenAiCompatibleAdapter;

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionMessage,
}

#[derive(Deserialize)]
struct ChatCompletionMessage {
    content: Option<Value>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    id: Option<String>,
    function: OpenAiFunctionCall,
}

#[derive(Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

fn extract_chat_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or_else(|| item.as_str().map(ToOwned::to_owned))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[async_trait]
impl ModelAdapter for OpenAiCompatibleAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::OpenAiCompatible
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        _on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, String> {
        let endpoint = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
        let body = json!({
            "model": config.model,
            "temperature": request.temperature,
            "messages": request.messages.iter().map(|message| {
                json!({
                    "role": message.role.as_str(),
                    "content": message.content
                })
            }).collect::<Vec<_>>(),
        });
        let mut body = body;
        let tool_name_map = tool_name_map(&request.tools);
        if !request.tools.is_empty() {
            body["tools"] = json!(request
                .tools
                .iter()
                .map(openai_tool_definition)
                .collect::<Vec<_>>());
            body["tool_choice"] = json!("auto");
            body["parallel_tool_calls"] = json!(true);
        }

        let request_builder = http.post(endpoint).json(&body);
        let request_builder = if config.api_key.is_empty() {
            request_builder
        } else {
            request_builder.bearer_auth(&config.api_key)
        };

        let response = request_builder
            .send()
            .await
            .map_err(|error| format!("OpenAI-compatible request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read OpenAI-compatible response: {error}"))?;

        if !status.is_success() {
            return Err(format!(
                "OpenAI-compatible request failed with status {}: {}",
                status, body
            ));
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&body)
            .map_err(|error| format!("Failed to parse OpenAI-compatible response: {error}"))?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| "OpenAI-compatible adapter returned no choices.".to_string())?;
        let content = choice
            .message
            .content
            .as_ref()
            .map(extract_chat_text)
            .unwrap_or_default();
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|call| openai_tool_call(call, &tool_name_map))
            .collect::<Result<Vec<_>, String>>()?;

        if content.trim().is_empty() && tool_calls.is_empty() {
            return Err("OpenAI-compatible adapter returned empty content.".into());
        }

        Ok(ModelCompletionResponse {
            raw_output: content,
            tool_calls,
        })
    }
}

fn openai_tool_definition(tool: &ModelToolDefinition) -> Value {
    let name = api_tool_name(&tool.id);
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": format!("{}: {} (Daily tool id: {})", tool.name, tool.description, tool.id),
            "parameters": tool.input_schema
        }
    })
}

fn openai_tool_call(
    call: OpenAiToolCall,
    tool_name_map: &std::collections::HashMap<String, String>,
) -> Result<ModelToolCall, String> {
    let arguments = if call.function.arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&call.function.arguments).map_err(|error| {
            format!(
                "Failed to parse OpenAI-compatible tool call arguments for {}: {error}",
                call.function.name
            )
        })?
    };
    Ok(ModelToolCall {
        call_id: call.id,
        tool_id: tool_name_map
            .get(&call.function.name)
            .cloned()
            .unwrap_or(call.function.name),
        arguments,
    })
}

fn tool_name_map(tools: &[ModelToolDefinition]) -> std::collections::HashMap<String, String> {
    tools
        .iter()
        .map(|tool| (api_tool_name(&tool.id), tool.id.clone()))
        .collect()
}

fn api_tool_name(tool_id: &str) -> String {
    let mut name = tool_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name.is_empty() || !name.chars().next().unwrap_or('_').is_ascii_alphabetic() {
        name = format!("tool_{name}");
    }
    name.chars().take(64).collect()
}
