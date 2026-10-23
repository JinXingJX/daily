use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelInlineImage,
    ModelMessage, ModelMessageRole, ModelProviderKind, ModelStreamChunk,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct GeminiApiAdapter;

#[derive(Deserialize)]
struct GeminiGenerateContentResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}

#[derive(Deserialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Deserialize)]
struct GeminiPart {
    text: Option<String>,
}

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
    ) -> Result<ModelCompletionResponse, String> {
        if config.api_key.trim().is_empty() {
            return Err(
                "Gemini API key is missing. Set GEMINI_API_KEY or GOOGLE_API_KEY, or add apiKey or apiKeyEnv to this provider in settings."
                    .into(),
            );
        }

        let endpoint = if on_chunk.is_some() {
            format!(
                "{}/models/{}:streamGenerateContent?alt=sse",
                config.base_url.trim_end_matches('/'),
                config.model
            )
        } else {
            format!(
                "{}/models/{}:generateContent",
                config.base_url.trim_end_matches('/'),
                config.model
            )
        };

        let body = build_gemini_request_body(request);
        let request_builder = http
            .post(endpoint)
            .json(&body)
            .header("x-goog-api-key", &config.api_key);

        let mut response = request_builder
            .send()
            .await
            .map_err(|error| format!("Gemini request failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|error| format!("Failed to read Gemini error response: {error}"))?;
            return Err(format!(
                "Gemini request failed with status {}: {}",
                status, body
            ));
        }

        if let Some(on_chunk) = on_chunk.as_mut() {
            let mut pending = String::new();
            let mut aggregated = String::new();

            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| format!("Failed to read Gemini stream chunk: {error}"))?
            {
                pending.push_str(
                    std::str::from_utf8(&chunk)
                        .map_err(|error| format!("Gemini stream was not valid UTF-8: {error}"))?,
                );

                while let Some(event) = take_sse_event(&mut pending) {
                    let payload = extract_sse_data(&event);
                    if payload.trim().is_empty() {
                        continue;
                    }

                    let parsed: GeminiGenerateContentResponse = serde_json::from_str(&payload)
                        .map_err(|error| format!("Failed to parse Gemini stream chunk: {error}"))?;
                    let text = extract_gemini_text(&parsed);
                    if text.is_empty() {
                        continue;
                    }

                    aggregated.push_str(&text);
                    on_chunk(ModelStreamChunk { text })?;
                }
            }

            if aggregated.trim().is_empty() {
                return Err("Gemini stream returned empty content.".into());
            }

            return Ok(ModelCompletionResponse {
                raw_output: aggregated,
                tool_calls: Vec::new(),
            });
        }

        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read Gemini response: {error}"))?;
        let parsed: GeminiGenerateContentResponse = serde_json::from_str(&body)
            .map_err(|error| format!("Failed to parse Gemini response: {error}"))?;
        let content = extract_gemini_text(&parsed);

        if content.trim().is_empty() {
            return Err("Gemini adapter returned empty content.".into());
        }

        Ok(ModelCompletionResponse {
            raw_output: content,
            tool_calls: Vec::new(),
        })
    }
}

fn build_gemini_request_body(request: &ModelCompletionRequest) -> Value {
    let system_instruction = request
        .messages
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut contents = request
        .messages
        .iter()
        .filter(|message| message.role != ModelMessageRole::System)
        .map(gemini_content_from_message)
        .collect::<Vec<_>>();

    if !request.inline_images.is_empty() {
        if let Some(last_user_content) = contents.iter_mut().rev().find(|content| {
            content
                .get("role")
                .and_then(Value::as_str)
                .map(|role| role == "user")
                .unwrap_or(false)
        }) {
            if let Some(parts) = last_user_content
                .get_mut("parts")
                .and_then(Value::as_array_mut)
            {
                parts.extend(request.inline_images.iter().map(gemini_part_from_image));
            }
        }
    }

    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "temperature": request.temperature,
            "maxOutputTokens": request.max_output_tokens.unwrap_or(1024),
        }
    });

    if !system_instruction.is_empty() {
        body["system_instruction"] = json!({
            "parts": [
                {
                    "text": system_instruction
                }
            ]
        });
    }

    body
}

fn gemini_content_from_message(message: &ModelMessage) -> Value {
    json!({
        "role": match message.role {
            ModelMessageRole::User => "user",
            ModelMessageRole::Assistant => "model",
            ModelMessageRole::System => "user",
        },
        "parts": [
            {
                "text": message.content
            }
        ]
    })
}

fn gemini_part_from_image(image: &ModelInlineImage) -> Value {
    json!({
        "inline_data": {
            "mime_type": image.mime_type,
            "data": image.data_base64
        }
    })
}

fn extract_gemini_text(response: &GeminiGenerateContentResponse) -> String {
    response
        .candidates
        .iter()
        .filter_map(|candidate| candidate.content.as_ref())
        .flat_map(|content| content.parts.iter())
        .filter_map(|part| part.text.as_deref())
        .collect::<Vec<_>>()
        .join("")
}

fn take_sse_event(pending: &mut String) -> Option<String> {
    let normalized = pending.replace("\r\n", "\n");
    if let Some(index) = normalized.find("\n\n") {
        let event = normalized[..index].to_string();
        *pending = normalized[index + 2..].to_string();
        return Some(event);
    }

    *pending = normalized;
    None
}

fn extract_sse_data(event: &str) -> String {
    event
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("data:"))
        .map(|value| value.trim_start())
        .collect::<Vec<_>>()
        .join("\n")
}
