use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelMessageRole,
    ModelProviderKind, ModelStreamChunk,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct AnthropicMessagesAdapter;

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[async_trait]
impl ModelAdapter for AnthropicMessagesAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::AnthropicMessages
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        _on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, String> {
        let endpoint = format!("{}/messages", config.base_url.trim_end_matches('/'));
        let system = request
            .messages
            .iter()
            .filter(|message| message.role == ModelMessageRole::System)
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages = request
            .messages
            .iter()
            .filter(|message| message.role != ModelMessageRole::System)
            .map(|message| {
                json!({
                    "role": message.role.as_str(),
                    "content": message.content,
                })
            })
            .collect::<Vec<Value>>();

        let body = json!({
            "model": config.model,
            "temperature": request.temperature,
            "max_tokens": request.max_output_tokens.unwrap_or(1024),
            "system": system,
            "messages": messages,
        });

        let mut request_builder = http
            .post(endpoint)
            .header("anthropic-version", "2023-06-01")
            .json(&body);

        if !config.api_key.is_empty() {
            request_builder = request_builder.header("x-api-key", &config.api_key);
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| format!("Anthropic request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read Anthropic response: {error}"))?;

        if !status.is_success() {
            return Err(format!(
                "Anthropic request failed with status {}: {}",
                status, body
            ));
        }

        let parsed: AnthropicResponse = serde_json::from_str(&body)
            .map_err(|error| format!("Failed to parse Anthropic response: {error}"))?;
        let content = parsed
            .content
            .into_iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n");

        if content.trim().is_empty() {
            return Err("Anthropic adapter returned empty content.".into());
        }

        Ok(ModelCompletionResponse {
            raw_output: content,
            tool_calls: Vec::new(),
        })
    }
}
