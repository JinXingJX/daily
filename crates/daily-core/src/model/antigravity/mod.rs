//! Unofficial Antigravity 2.x adapter: reuses the official client's macOS Keychain login.
//! Not affiliated with or endorsed by Google; it may violate the service's terms and can stop
//! working at any time. See `docs/subscription-adapters.md`.
mod auth;
mod response;
#[cfg(test)]
mod test_support;

use super::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelMessageRole,
    ModelProviderKind, ModelStreamChunk, ModelToolCall, ModelToolDefinition,
    DEFAULT_ANTIGRAVITY_URL,
};
use async_trait::async_trait;
pub use auth::{
    antigravity_auth_status, start_antigravity_login, AntigravityAuthStatusView,
    AntigravityLoginStartView,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::{Client, Response, StatusCode};
use response::{read_response, Recovery};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// The client's updater; its manifest names the newest released version.
const VERSION_MANIFEST_URL: &str = "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-arm64-mac.yml";
const FALLBACK_VERSION: &str = "2.23.1";
const VERSION_TTL: Duration = Duration::from_secs(6 * 60 * 60);
// Tool calls are not streamed: headers and the whole call arrive only after the model
// finishes generating it, which takes minutes for a full document rewrite.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CONNECT_RETRIES: u32 = 2;
const CONNECT_RETRY_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(1)
};
static LATEST_VERSION: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();

pub struct AntigravityLoginAdapter;

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RefreshAntigravityModelsInput {
    pub auth_config_id: String,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AntigravityModelCatalog {
    pub models: Vec<String>,
    pub default_model: String,
}

async fn send_with_auth(
    http: &Client,
    method: &str,
    body: &Value,
    stream: bool,
) -> Result<Response, String> {
    let user_agent = user_agent(&client_version(http).await);
    send_with_auth_using(http, DEFAULT_ANTIGRAVITY_URL, &user_agent, method, body, stream, |rejected: Option<String>| async move {
        auth::credentials(http, rejected.as_deref()).await
    }).await
}

async fn send_with_auth_using<F, Fut>(
    http: &Client,
    base_url: &str,
    user_agent: &str,
    method: &str,
    body: &Value,
    stream: bool,
    mut credentials_for: F,
) -> Result<Response, String>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<auth::AuthSnapshot, String>>,
{
    let mut credentials = credentials_for(None).await?;
    let mut refreshed = false;
    let mut resent = false;
    let mut connect_retries = 0;
    loop {
        let suffix = if stream { "?alt=sse" } else { "" };
        let response = http
            .post(format!("{base_url}:{method}{suffix}"))
            .bearer_auth(&credentials.access_token)
            .header("User-Agent", user_agent)
            .header("Client-Metadata", "{\"ideType\":\"ANTIGRAVITY\"}")
            .header(
                "Accept",
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .json(body)
            .send();
        let started = Instant::now();
        // Bound only the wait for response headers: a total request timeout would also
        // cut off long SSE bodies. Stream idleness is bounded in `read_response`.
        let response = match tokio::time::timeout(RESPONSE_TIMEOUT, response)
            .await
            .map_err(|_| "The Antigravity service timed out. Try again.")?
        {
            Ok(response) => response,
            Err(error) => {
                let waited = started.elapsed();
                eprintln!("Antigravity {method} send failed after {waited:?}: {error:?}");
                // A failed connect (such as a proxy dropping the tunnel mid TLS handshake)
                // never reached the service, so it is safe to retry after a short pause.
                if error.is_connect() && connect_retries < CONNECT_RETRIES {
                    connect_retries += 1;
                    tokio::time::sleep(CONNECT_RETRY_DELAY * connect_retries).await;
                    continue;
                }
                // A connection dropped after it was established (an idle proxy cutting a
                // long wait, or a stale pooled one) may have been served, so resend once.
                if !error.is_connect() && !resent {
                    resent = true;
                    continue;
                }
                return Err(send_error(&error, waited));
            }
        };
        if response.status() == StatusCode::UNAUTHORIZED && !refreshed {
            refreshed = true;
            credentials = credentials_for(Some(credentials.access_token.clone())).await?;
            continue;
        }
        if !response.status().is_success() {
            return Err(service_error(response.status()));
        }
        return Ok(response);
    }
}

fn send_error(error: &reqwest::Error, waited: Duration) -> String {
    let mut cause: &dyn std::error::Error = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    if error.is_connect() {
        format!("Could not reach the Antigravity service. Check the network or proxy. ({cause})")
    } else {
        let waited = waited.as_secs();
        format!("The Antigravity connection dropped after waiting {waited} seconds for a response. Try again; if it keeps happening, check your proxy's idle timeout. ({cause})")
    }
}

// Same shape as the official client: Go-style platform names after the version.
fn user_agent(version: &str) -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    };
    format!("antigravity/hub/{version} {os}/{arch}")
}

/// The installed client's version, else the newest one its updater offers.
async fn client_version(http: &Client) -> String {
    if let Some(version) = auth::app_version() {
        return version;
    }
    let cache = LATEST_VERSION.get_or_init(|| Mutex::new(None));
    let cached = cache.lock().ok().and_then(|guard| guard.clone());
    if let Some((version, at)) = &cached {
        if at.elapsed() < VERSION_TTL {
            return version.clone();
        }
    }
    let fetched = fetch_latest_version(http).await;
    // A failed lookup still stamps the cache so it is retried only after the TTL.
    let version = fetched
        .or_else(|| cached.map(|(version, _)| version))
        .unwrap_or_else(|| FALLBACK_VERSION.into());
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((version.clone(), Instant::now()));
    }
    version
}

async fn fetch_latest_version(http: &Client) -> Option<String> {
    let response = http
        .get(VERSION_MANIFEST_URL)
        .header("User-Agent", "electron-builder")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_manifest_version(&response.text().await.ok()?)
}

fn parse_manifest_version(manifest: &str) -> Option<String> {
    let version = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version:"))?
        .trim()
        .trim_matches(|c| c == '\'' || c == '"');
    (!version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .then(|| version.to_owned())
}

fn request_id() -> String {
    let seed = crate::util::unique_stamp().to_string();
    let mut bytes = [0u8; 12];
    bytes[..8].copy_from_slice(&crate::util::stable_hash64(&format!("{seed}:a")).to_be_bytes());
    bytes[8..]
        .copy_from_slice(&crate::util::stable_hash64(&format!("{seed}:b")).to_be_bytes()[..4]);
    format!("agent-{}", URL_SAFE_NO_PAD.encode(bytes))
}

// Named from the first user message, as the official client does, so every
// turn of one conversation shares it.
fn session_id(request: &ModelCompletionRequest) -> String {
    let first = request
        .messages
        .iter()
        .find(|m| m.role == ModelMessageRole::User && !m.content.is_empty())
        .map(|m| m.content.clone())
        .unwrap_or_else(|| crate::util::unique_stamp().to_string());
    format!(
        "-{}",
        crate::util::stable_hash64(&first) & 0x7fff_ffff_ffff_ffff
    )
}

fn service_error(status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED => "The Antigravity authorization is no longer valid. Sign in again in the official client.".into(),
        StatusCode::FORBIDDEN => {
            "The Antigravity account or selected model is unavailable. Check the account in the official client.".into()
        }
        StatusCode::TOO_MANY_REQUESTS => {
            "Antigravity quota exhausted or rate limited. Try again later or pick another model.".into()
        }
        _ => format!("Antigravity request failed (HTTP {status})."),
    }
}

async fn discover_project(http: &Client) -> Result<Option<String>, String> {
    let response = send_with_auth(
        http,
        "loadCodeAssist",
        &json!({"metadata":{"ideType":"ANTIGRAVITY"}}),
        false,
    )
    .await?;
    let payload: Value = response
        .json()
        .await
        .map_err(|_| "Antigravity returned an invalid account setup response.")?;
    let project = payload
        .get("cloudaicompanionProject")
        .and_then(|v| v.as_str().or_else(|| v.get("id").and_then(Value::as_str)))
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    Ok(project)
}

pub async fn fetch_model_catalog(
    http: &Client,
    current_model: &str,
) -> Result<AntigravityModelCatalog, String> {
    let project = discover_project(http).await?;
    let body = project
        .map(|project| json!({"project":project}))
        .unwrap_or_else(|| json!({}));
    let response = send_with_auth(http, "fetchAvailableModels", &body, false).await?;
    let payload: Value = response
        .json()
        .await
        .map_err(|_| "Antigravity returned an invalid model catalog.")?;
    let catalog = parse_catalog(&payload, current_model)?;
    // User info is cosmetic; catalog access remains useful when it is unavailable.
    let credentials = auth::credentials(http, None).await?;
    if credentials.email.is_none() {
        if let Ok(response) = http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(&credentials.access_token)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(user) = response.json::<Value>().await {
                    auth::remember_email(
                        &credentials.access_token,
                        user.get("email")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    )
                    .await;
                }
            }
        }
    }
    Ok(catalog)
}

fn parse_catalog(payload: &Value, current_model: &str) -> Result<AntigravityModelCatalog, String> {
    let entries = payload
        .get("models")
        .and_then(Value::as_object)
        .ok_or("The Antigravity model catalog has no models.")?;
    let mut models = entries
        .keys()
        .filter(|id| {
            (id.starts_with("gemini") || id.starts_with("claude")) && !id.contains("image")
        })
        .cloned()
        .collect::<Vec<_>>();
    models.sort();
    let default_model = models
        .iter()
        .find(|m| m.as_str() == current_model)
        .or_else(|| {
            models.iter().find(|m| {
                m.starts_with("gemini")
                    && entries[*m].get("recommended").and_then(Value::as_bool) == Some(true)
            })
        })
        .or_else(|| models.iter().find(|m| m.starts_with("gemini")))
        .or_else(|| models.first())
        .cloned()
        .ok_or("Antigravity returned no usable Gemini or Claude text model.")?;
    Ok(AntigravityModelCatalog {
        models,
        default_model,
    })
}

#[async_trait]
impl ModelAdapter for AntigravityLoginAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::AntigravityLogin
    }
    fn supports_streaming(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, String> {
        if config.model.trim().is_empty() {
            return Err(
                "Refresh the Antigravity model list in Settings → Model providers first.".into(),
            );
        }
        if !request.inline_images.is_empty() {
            return Err("Image input is not supported with Antigravity sign-in.".into());
        }
        let project = discover_project(http).await?;
        let body = request_body(config, request, project.as_deref());
        complete_using(body, &request.tools, on_chunk, |body, stream| async move {
            send_with_auth(
                http,
                if stream {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                },
                &body,
                stream,
            )
            .await
        })
        .await
    }
}

// Only a fully read response with no visible text or tool calls can be replayed.
// Transport errors and callback failures must keep the original checkpoint.
async fn complete_using<F, Fut>(
    mut body: Value,
    tools: &[ModelToolDefinition],
    mut on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    mut send: F,
) -> Result<ModelCompletionResponse, String>
where
    F: FnMut(Value, bool) -> Fut,
    Fut: std::future::Future<Output = Result<Response, String>>,
{
    const MAX_ATTEMPTS: usize = 3;
    const MAX_RECOVERY_TOKENS: u64 = 32_768;
    let mut stream = on_chunk.is_some();
    for attempt in 0..MAX_ATTEMPTS {
        let response = send(body.clone(), stream).await?;
        let result = if let Some(callback) = on_chunk.as_mut() {
            let mut forward = |chunk| callback(chunk);
            read_response(response, stream, tools, Some(&mut forward)).await
        } else {
            read_response(response, stream, tools, None).await
        };
        match result {
            Ok(result) => return Ok(result),
            Err(failure) => {
                let Some(recovery) = failure.recovery else {
                    return Err(failure.message);
                };
                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(failure.message);
                }
                match recovery {
                    Recovery::MoreTokens => {
                        let current = body["request"]["generationConfig"]["maxOutputTokens"]
                            .as_u64()
                            .unwrap_or(1024);
                        let increased = current.saturating_mul(2).clamp(8192, MAX_RECOVERY_TOKENS);
                        if increased <= current {
                            return Err(failure.message);
                        }
                        body["request"]["generationConfig"]["maxOutputTokens"] = json!(increased);
                    }
                    Recovery::Empty => {
                        // Some successful SSE responses contain only metadata. A JSON
                        // response is a useful fallback, with the same callback contract.
                        stream = false;
                    }
                    Recovery::MalformedToolCall => {
                        if tools.is_empty() {
                            stream = false;
                        } else {
                            let instruction = json!({"text":"The previous generation produced an invalid tool call. Use the exact declared function names and complete JSON arguments matching their schemas. Split large changes into smaller tool calls."});
                            if let Some(parts) =
                                body["request"]["systemInstruction"]["parts"].as_array_mut()
                            {
                                parts.push(instruction);
                            } else {
                                body["request"]["systemInstruction"] =
                                    json!({"parts":[instruction]});
                            }
                        }
                    }
                }
                body["requestId"] = json!(request_id());
                tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
            }
        }
    }
    unreachable!("the last attempt always returns")
}

fn tool_name(index: usize) -> String {
    format!("daily_tool_{index}")
}

fn tool_names(tools: &[ModelToolDefinition]) -> HashMap<String, String> {
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| (tool_name(i), t.id.clone()))
        .collect()
}

fn request_body(
    config: &ModelConfig,
    request: &ModelCompletionRequest,
    project: Option<&str>,
) -> Value {
    let system = request
        .messages
        .iter()
        .filter(|m| m.role == ModelMessageRole::System)
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let contents = request.messages.iter().filter(|m| m.role != ModelMessageRole::System)
        .map(|m| json!({"role":if m.role == ModelMessageRole::Assistant {"model"} else {"user"},"parts":[{"text":m.content}]})).collect::<Vec<_>>();
    let mut body = json!({"model":config.model,"userAgent":"antigravity","requestType":"agent",
        "requestId":request_id(),
        "request":{"contents":contents,"sessionId":session_id(request),"generationConfig":{"temperature":request.temperature,"maxOutputTokens":request.max_output_tokens.unwrap_or(1024)}}});
    if let Some(project) = project {
        body["project"] = json!(project);
    }
    let mut instructions = Vec::new();
    if !system.is_empty() {
        instructions.push(json!({"text":system}));
    }
    if !request.tools.is_empty() {
        let declarations = request.tools.iter().enumerate().map(|(i,t)| json!({"name":tool_name(i),"description":t.description,"parameters":clean_schema(&t.input_schema)})).collect::<Vec<_>>();
        body["request"]["tools"] = json!([{"functionDeclarations":declarations}]);
        // Prompts and observations use domain ids such as writer.readDocument.
        // Keep their relationship to API-safe aliases explicit for the model.
        let mapping = request
            .tools
            .iter()
            .enumerate()
            .map(|(i, tool)| format!("{} => {}", tool.id, tool_name(i)))
            .collect::<Vec<_>>()
            .join("\n");
        instructions.push(json!({"text":format!("Tool name mapping (use the declared API name on the right when calling a tool):\n{mapping}")}));
    }
    if !instructions.is_empty() {
        body["request"]["systemInstruction"] = json!({"parts":instructions});
    }
    body
}

// Cloud Code's protobuf Schema accepts only its documented subset of JSON Schema.
fn clean_schema(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return json!({"type":"object"});
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
                    .map(|(key, value)| (key.clone(), clean_schema(value)))
                    .collect(),
            ),
        );
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            result.insert(
                "required".into(),
                json!(required
                    .iter()
                    .filter(|v| v.as_str().is_some_and(|key| properties.contains_key(key)))
                    .collect::<Vec<_>>()),
            );
        }
    }
    if let Some(items) = object.get("items") {
        result.insert("items".into(), clean_schema(items));
    }
    if !result.contains_key("type") {
        result.insert("type".into(), json!("object"));
    }
    Value::Object(result)
}

fn append_response(
    payload: &Value,
    names: &HashMap<String, String>,
    result: &mut ModelCompletionResponse,
) -> Result<String, String> {
    let response = payload.get("response").unwrap_or(payload);
    if response.get("error").is_some() || payload.get("error").is_some() {
        return Err(
            "Antigravity returned a model error. Check the selected model and account access."
                .into(),
        );
    }
    let candidate = response
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|c| c.first());
    let mut text = String::new();
    if let Some(parts) = candidate
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
    {
        for part in parts {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                text.push_str(chunk);
            }
            if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("An Antigravity tool call has no name.")?;
                let tool_id = names
                    .get(name)
                    .ok_or("Antigravity called an undeclared tool.")?
                    .clone();
                let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
                let call_id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                if !result.tool_calls.iter().any(|existing| {
                    existing.call_id == call_id
                        && existing.tool_id == tool_id
                        && existing.arguments == arguments
                }) {
                    result.tool_calls.push(ModelToolCall {
                        call_id,
                        tool_id,
                        arguments,
                    });
                }
            }
        }
    }
    result.raw_output.push_str(&text);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_auth(token: &str) -> auth::AuthSnapshot {
        auth::AuthSnapshot {
            access_token: token.into(),
            refresh_token: "fixture-refresh".into(),
            expiry: auth::now() + 3600,
            expires_at: None,
            email: None,
        }
    }

    fn fixture_request(tokens: u32) -> ModelCompletionRequest {
        ModelCompletionRequest {
            messages: vec![super::super::ModelMessage {
                role: ModelMessageRole::User,
                content: "Read the project, then finish.".into(),
            }],
            temperature: 0.2,
            max_output_tokens: Some(tokens),
            inline_images: Vec::new(),
            tools: vec![ModelToolDefinition {
                id: "writer.readProjectIndex".into(),
                name: "Read project".into(),
                description: "Read the project index.".into(),
                input_schema: json!({"type":"object","properties":{}}),
            }],
            session_id: None,
            turn_id: None,
            request_name: None,
        }
    }

    fn sse(payloads: Vec<Value>) -> String {
        payloads
            .into_iter()
            .map(|payload| format!("data: {payload}\r\n\r\n"))
            .collect()
    }

    async fn complete_fixture(
        http: &Client,
        url: &str,
        request: &ModelCompletionRequest,
        on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, String> {
        let body = request_body(&ModelConfig::seeded(), request, None);
        complete_using(body, &request.tools, on_chunk, |body, stream| async move {
            send_with_auth_using(
                http,
                url,
                "antigravity/hub/0.0.0 test/test",
                if stream {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                },
                &body,
                stream,
                |_| std::future::ready(Ok(fixture_auth("dummy"))),
            )
            .await
        })
        .await
    }

    fn request_json(request: &str) -> Value {
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    #[tokio::test]
    async fn empty_thinking_response_increases_budget_without_replaying_a_tool() {
        let http = Client::builder().no_proxy().build().unwrap();
        let request = fixture_request(8192);
        let (url, server) = test_support::serve(vec![
            (200, "text/event-stream", sse(vec![
                json!({"response":{"candidates":[{"content":{"parts":[{"thought":true,"text":"hidden"}]}}]}}),
                json!({"response":{"candidates":[{"finishReason":"MAX_TOKENS"}]}}),
                json!({"response":{"usageMetadata":{"thoughtsTokenCount":8192}}}),
            ]), 0),
            (200, "text/event-stream", sse(vec![json!({"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"daily_tool_0","args":{}}}]},"finishReason":"STOP"}]}})]), 0),
        ]).await;
        let mut chunks = Vec::new();
        let result = complete_fixture(
            &http,
            &url,
            &request,
            Some(&mut |chunk| {
                chunks.push(chunk.text);
                Ok(())
            }),
        )
        .await
        .unwrap();
        assert!(chunks.is_empty());
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].tool_id, "writer.readProjectIndex");
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        let first = request_json(&requests[0]);
        let second = request_json(&requests[1]);
        assert_eq!(
            first["request"]["generationConfig"]["maxOutputTokens"],
            8192
        );
        assert_eq!(
            second["request"]["generationConfig"]["maxOutputTokens"],
            16384
        );
        assert_eq!(first["request"]["contents"], second["request"]["contents"]);
        assert_ne!(first["requestId"], second["requestId"]);
        assert!(first["request"]["systemInstruction"]
            .to_string()
            .contains("writer.readProjectIndex => daily_tool_0"));
    }

    #[tokio::test]
    async fn empty_sse_falls_back_to_json_and_emits_text_once() {
        let http = Client::builder().no_proxy().build().unwrap();
        let (url, server) = test_support::serve(vec![
            (200, "text/event-stream", sse(vec![json!({"response":{"candidates":[{"finishReason":"STOP"}]}})]), 0),
            (200, "application/json", json!({"response":{"candidates":[{"content":{"parts":[{"text":"已完成"}]},"finishReason":"STOP"}]}}).to_string(), 0),
        ]).await;
        let mut chunks = Vec::new();
        let result = complete_fixture(
            &http,
            &url,
            &fixture_request(8192),
            Some(&mut |chunk| {
                chunks.push(chunk.text);
                Ok(())
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.raw_output, "已完成");
        assert_eq!(chunks, vec!["已完成"]);
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("POST /v1internal:streamGenerateContent?alt=sse "));
        assert!(requests[1].starts_with("POST /v1internal:generateContent "));
    }

    #[tokio::test]
    async fn malformed_tool_calls_are_repaired_with_a_bounded_number_of_attempts() {
        let http = Client::builder().no_proxy().build().unwrap();
        let malformed = json!({"candidates":[{"finishReason":"MALFORMED_FUNCTION_CALL","finishMessage":"private prompt text"}]}).to_string();
        let (url, server) =
            test_support::serve(vec![(200, "application/json", malformed, 0); 3]).await;
        let error = complete_fixture(&http, &url, &fixture_request(8192), None)
            .await
            .err()
            .unwrap();
        assert!(error.contains("MALFORMED_FUNCTION_CALL"));
        assert!(!error.contains("private prompt text"));
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(request_json(&requests[1])["request"]["systemInstruction"]
            .to_string()
            .contains("exact declared function names"));
        for request in requests {
            assert_eq!(
                request_json(&request)["request"]["generationConfig"]["maxOutputTokens"],
                8192
            );
        }
    }

    #[tokio::test]
    async fn token_recovery_stops_at_the_cap_and_keeps_finish_metadata() {
        let http = Client::builder().no_proxy().build().unwrap();
        let exhausted = json!({"response":{"candidates":[{"finishReason":"MAX_TOKENS"}],"usageMetadata":{"thoughtsTokenCount":32768}}}).to_string();
        let (url, server) =
            test_support::serve(vec![(200, "application/json", exhausted, 0); 3]).await;
        let error = complete_fixture(&http, &url, &fixture_request(8192), None)
            .await
            .err()
            .unwrap();
        assert!(error.contains("MAX_TOKENS"));
        assert!(error.contains("32768"));
        let requests = server.await.unwrap();
        let budgets = requests
            .iter()
            .map(|r| {
                request_json(r)["request"]["generationConfig"]["maxOutputTokens"]
                    .as_u64()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(budgets, vec![8192, 16384, 32768]);

        let (url, server) = test_support::serve(vec![(
            200,
            "application/json",
            json!({"candidates":[{"finishReason":"MAX_TOKENS"}]}).to_string(),
            0,
        )])
        .await;
        assert!(complete_fixture(&http, &url, &fixture_request(32768), None)
            .await
            .is_err());
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn blocked_responses_and_unknown_tools_are_not_retried() {
        let http = Client::builder().no_proxy().build().unwrap();
        for payload in [
            json!({"promptFeedback":{"blockReason":"SAFETY"}}),
            json!({"candidates":[{"finishReason":"SAFETY"}]}),
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"undeclared","args":{}}}]}}]}),
            json!({"error":{"message":"private upstream text"}}),
        ] {
            let (url, server) =
                test_support::serve(vec![(200, "application/json", payload.to_string(), 0)]).await;
            let error = complete_fixture(&http, &url, &fixture_request(8192), None)
                .await
                .err()
                .unwrap();
            assert!(!error.contains("private upstream text"));
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn partial_streams_and_callback_failures_are_not_replayed() {
        let http = Client::builder().no_proxy().build().unwrap();
        for (text, extra, callback_fails) in
            [("你好", 100, false), ("你好", 0, true), (" ", 0, false)]
        {
            let body = sse(vec![
                json!({"response":{"candidates":[{"content":{"parts":[{"text":text}]}}]}}),
            ]);
            let (url, server) =
                test_support::serve(vec![(200, "text/event-stream", body, extra)]).await;
            let mut chunks = Vec::new();
            let error = complete_fixture(
                &http,
                &url,
                &fixture_request(8192),
                Some(&mut |chunk| {
                    chunks.push(chunk.text);
                    if callback_fails {
                        Err("fixture cancelled".into())
                    } else {
                        Ok(())
                    }
                }),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(chunks, vec![text]);
            assert!(!error.is_empty());
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn unauthorized_refreshes_once_and_other_errors_do_not_retry() {
        let http = Client::builder().no_proxy().build().unwrap();
        let (url, server) = test_support::serve(vec![
            (401, "application/json", "{}".into(), 0),
            (200, "application/json", "{}".into(), 0),
        ])
        .await;
        let mut calls = Vec::new();
        let result = send_with_auth_using(
            &http,
            &url,
            "antigravity/hub/0.0.0 test/test",
            "generateContent",
            &json!({}),
            false,
            |rejected| {
                calls.push(rejected.clone());
                std::future::ready(Ok(fixture_auth(if rejected.is_some() {
                    "new-token"
                } else {
                    "old-token"
                })))
            },
        )
        .await
        .unwrap();
        assert!(result.status().is_success());
        assert_eq!(calls, vec![None, Some("old-token".into())]);
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("Bearer new-token"));
        for status in [401, 403, 429] {
            let responses = if status == 401 {
                vec![(401, "application/json", "{}".into(), 0); 2]
            } else {
                vec![(status, "application/json", "{}".into(), 0)]
            };
            let (url, server) = test_support::serve(responses).await;
            assert!(send_with_auth_using(
                &http,
                &url,
                "antigravity/hub/0.0.0 test/test",
                "generateContent",
                &json!({}),
                false,
                |_| std::future::ready(Ok(fixture_auth("dummy")))
            )
            .await
            .is_err());
            assert_eq!(
                server.await.unwrap().len(),
                if status == 401 { 2 } else { 1 }
            );
        }
    }

    #[tokio::test]
    async fn dropped_connections_are_resent_once_and_failed_connects_are_retried() {
        let http = Client::builder().no_proxy().build().unwrap();
        let send = |url: String| {
            let http = http.clone();
            async move {
                send_with_auth_using(
                    &http,
                    &url,
                    "antigravity/hub/0.0.0 test/test",
                    "generateContent",
                    &json!({}),
                    false,
                    |_| std::future::ready(Ok(fixture_auth("dummy"))),
                )
                .await
            }
        };
        let (url, server) = test_support::serve(vec![
            (0, "", String::new(), 0),
            (200, "application/json", "{}".into(), 0),
        ])
        .await;
        assert!(send(url).await.unwrap().status().is_success());
        assert_eq!(server.await.unwrap().len(), 2);

        let (url, server) =
            test_support::serve(vec![(0, "", String::new(), 0), (0, "", String::new(), 0)]).await;
        let error = send(url).await.unwrap_err();
        assert!(error.contains("seconds for a response"), "{error}");
        assert_eq!(server.await.unwrap().len(), 2);

        // Refused on the first attempt; the service comes up before the retry.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/v1internal");
        drop(listener);
        let server = tokio::spawn(async move {
            tokio::time::sleep(CONNECT_RETRY_DELAY / 4).await;
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            test_support::serve_on(listener, vec![(200, "application/json", "{}".into(), 0)])
                .await
                .unwrap()
        });
        assert!(send(url).await.unwrap().status().is_success());
        assert_eq!(server.await.unwrap().len(), 1);

        // Still refused after every retry.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1internal", listener.local_addr().unwrap());
        drop(listener);
        let error = send(url).await.unwrap_err();
        assert!(
            error.starts_with("Could not reach the Antigravity service"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn sse_handles_fragmented_unicode_and_does_not_replay_after_disconnect() {
        let http = Client::builder().no_proxy().build().unwrap();
        let body = "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"你好\"}]}}]}}\n\ndata: [DONE]\n\n".to_owned();
        for extra in [0, 100] {
            let (url, server) =
                test_support::serve(vec![(200, "text/event-stream", body.clone(), extra)]).await;
            let response = http.get(url).send().await.unwrap();
            let mut chunks = Vec::new();
            let mut emit = |chunk: ModelStreamChunk| {
                chunks.push(chunk.text);
                Ok(())
            };
            let result = read_response(response, true, &[], Some(&mut emit)).await;
            assert_eq!(chunks, vec!["你好"]);
            assert_eq!(result.is_ok(), extra == 0);
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    #[ignore = "Requires a locally signed-in Antigravity 2.x client and network access"]
    async fn live_antigravity_high_thinking_tools_and_empty_recovery() {
        let http = crate::net::http::build_http_client().unwrap();
        let mut config = ModelConfig::seeded();
        config.provider_kind = ModelProviderKind::AntigravityLogin;
        config.model = "gemini-3.8-flash-high".into();
        let mut request = fixture_request(8192);
        request.messages[0].content =
            "Call writer.readProjectIndex once, with no arguments. Do not answer with text.".into();
        let result = AntigravityLoginAdapter
            .complete(&http, &config, &request, Some(&mut |_| Ok(())))
            .await
            .unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].tool_id, "writer.readProjectIndex");
        eprintln!("{}: streaming Writer tool call succeeded", config.model);

        request.tools.clear();
        request.messages[0].content = "Reply with exactly OK.".into();
        request.max_output_tokens = Some(1);
        let project = discover_project(&http).await.unwrap();
        let body = request_body(&config, &request, project.as_deref());
        let mut chunks = String::new();
        let mut attempts = 0;
        let result = complete_using(
            body,
            &[],
            Some(&mut |chunk| {
                chunks.push_str(&chunk.text);
                Ok(())
            }),
            |body, stream| {
                attempts += 1;
                eprintln!(
                    "{}: attempt {attempts}, budget {}",
                    config.model, body["request"]["generationConfig"]["maxOutputTokens"]
                );
                let http = &http;
                async move {
                    send_with_auth(
                        http,
                        if stream {
                            "streamGenerateContent"
                        } else {
                            "generateContent"
                        },
                        &body,
                        stream,
                    )
                    .await
                }
            },
        )
        .await
        .unwrap();
        assert!(!result.raw_output.trim().is_empty());
        assert_eq!(chunks, result.raw_output);
        assert!(
            attempts > 1,
            "the tiny budget should require empty-response recovery"
        );
        eprintln!("{}: empty-response recovery succeeded", config.model);
    }

    #[tokio::test]
    #[ignore = "Requires a locally signed-in Antigravity 2.x client and network access"]
    async fn live_antigravity_catalog_and_models() {
        let http = crate::net::http::build_http_client().unwrap();
        let status = antigravity_auth_status().await;
        assert!(status.authenticated, "{}", status.error.unwrap_or_default());
        let catalog = fetch_model_catalog(&http, "").await.unwrap();
        eprintln!("Antigravity models: {}", catalog.models.join(", "));
        for prefix in ["gemini", "claude"] {
            let Some(model) = catalog.models.iter().find(|m| m.starts_with(prefix)) else {
                continue;
            };
            let mut config = ModelConfig::seeded();
            config.provider_kind = ModelProviderKind::AntigravityLogin;
            config.model = model.clone();
            let request = ModelCompletionRequest {
                messages: vec![super::super::ModelMessage {
                    role: ModelMessageRole::User,
                    content: "Reply with exactly OK.".into(),
                }],
                temperature: 0.2,
                max_output_tokens: Some(1024),
                inline_images: Vec::new(),
                tools: Vec::new(),
                session_id: None,
                turn_id: None,
                request_name: None,
            };
            let result = AntigravityLoginAdapter
                .complete(&http, &config, &request, None)
                .await
                .unwrap();
            assert!(!result.raw_output.is_empty());
            eprintln!("{model}: text response succeeded");
            let mut streamed = String::new();
            let mut emit = |chunk: ModelStreamChunk| {
                streamed.push_str(&chunk.text);
                Ok(())
            };
            let result = AntigravityLoginAdapter
                .complete(&http, &config, &request, Some(&mut emit))
                .await
                .unwrap();
            assert_eq!(streamed, result.raw_output);
            assert!(!streamed.is_empty());
            eprintln!("{model}: streaming response succeeded");
        }

        use crate::agent::sdk::{
            run_agent_task, AgentTask, AgentTool, AgentToolExecution, AgentToolResult,
        };
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let tool = AgentTool::new_with_execution(
            "fixture.echo",
            "Echo",
            "Return the supplied text to verify a tool call.",
            json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}),
            move |call| {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(AgentToolExecution::Completed(AgentToolResult::new(
                    call.input.to_string(),
                )))
            },
        );
        let mut config = ModelConfig::seeded();
        config.provider_kind = ModelProviderKind::AntigravityLogin;
        config.model = catalog
            .models
            .iter()
            .find(|m| m.starts_with("gemini"))
            .unwrap()
            .clone();
        let task = AgentTask {
            name:"Antigravity smoke test".into(),session_id:None,turn_id:None,model:config,
            system_prompt:"Use the echo tool once. If tool observations already show success, answer OK without calling again.".into(),
            user_prompt:"Verify the tool using value=OK, then finish.".into(),final_instructions:"Answer OK after the tool succeeds.".into(),
            context:Vec::new(),conversation:Vec::new(),prior_tool_observations:Vec::new(),tools:vec![tool],max_iterations:3,start_ply:1,
            observation_budget:None,max_output_tokens:1024,temperature:0.2,inline_images:Vec::new(),stream:true,require_tool_before_final:true,continue_on_tool_error:false,sub_agents:Default::default(),resume_sub_agent:None,
        };
        run_agent_task(&http, task, |_| Ok(())).await.unwrap();
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 0);
        eprintln!("Antigravity Agent tool loop succeeded");
    }

    #[test]
    fn request_identity_matches_the_official_client_shape() {
        let mut request = fixture_request(1024);
        let body = request_body(&ModelConfig::seeded(), &request, Some("proj"));
        let id = body["requestId"].as_str().unwrap();
        assert!(
            id.starts_with("agent-") && id.len() == "agent-".len() + 16,
            "{id}"
        );
        assert!(!body.to_string().contains("daily-"));
        let session = body["request"]["sessionId"].as_str().unwrap().to_owned();
        assert!(session.starts_with('-') && session[1..].parse::<i64>().is_ok());

        // Later turns of the same conversation keep the session id.
        request.messages.push(super::super::ModelMessage {
            role: ModelMessageRole::Assistant,
            content: "Done.".into(),
        });
        let next = request_body(&ModelConfig::seeded(), &request, Some("proj"));
        assert_eq!(next["request"]["sessionId"], json!(session));
        assert_ne!(next["requestId"], body["requestId"]);
    }

    #[test]
    fn user_agent_and_manifest_version() {
        let ua = user_agent("2.9.1");
        assert!(ua.starts_with("antigravity/hub/2.9.1 "), "{ua}");
        assert!(!ua.contains("macos") && !ua.contains("aarch64"), "{ua}");
        assert_eq!(
            parse_manifest_version("version: 2.10.3\nfiles:\n").as_deref(),
            Some("2.10.3")
        );
        assert_eq!(
            parse_manifest_version("version: '2.11.0'\n").as_deref(),
            Some("2.11.0")
        );
        assert_eq!(parse_manifest_version("version: latest\n"), None);
        assert_eq!(parse_manifest_version("files: []\n"), None);
    }

    #[test]
    fn catalog_preserves_selection_and_prefers_recommended_gemini() {
        let payload = json!({"models":{"claude-sonnet":{"recommended":true},"gemini-flash":{},"gemini-pro":{"recommended":true},"gemini-image":{},"internal":{}}});
        let catalog = parse_catalog(&payload, "retired").unwrap();
        assert_eq!(catalog.default_model, "gemini-pro");
        assert_eq!(catalog.models.len(), 3);
        assert_eq!(
            parse_catalog(&payload, "claude-sonnet")
                .unwrap()
                .default_model,
            "claude-sonnet"
        );
        assert!(parse_catalog(&json!({"models":{}}), "").is_err());
    }

    #[test]
    fn response_ignores_thoughts_and_accepts_multiple_tools() {
        let tools = vec![ModelToolDefinition {
            id: "plugin.files.read".into(),
            name: "Read file".into(),
            description: String::new(),
            input_schema: json!({}),
        }];
        let names = tool_names(&tools);
        let mut result = ModelCompletionResponse {
            raw_output: String::new(),
            tool_calls: Vec::new(),
        };
        let payload = json!({"response":{"candidates":[{"content":{"parts":[{"thought":true,"text":"hidden"},{"text":"visible"},{"functionCall":{"name":"daily_tool_0","args":{"path":"one"}}},{"functionCall":{"name":"daily_tool_0","args":{"path":"two"}}}]}}]}});
        assert_eq!(
            append_response(&payload, &names, &mut result).unwrap(),
            "visible"
        );
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].tool_id, "plugin.files.read");
        assert!(
            append_response(&json!({"error":{"message":"secret"}}), &names, &mut result).is_err()
        );
    }

    #[test]
    fn tool_schema_removes_unsupported_keys_and_orphan_required() {
        let schema = clean_schema(
            &json!({"type":"object","additionalProperties":false,"$schema":"url","properties":{"path":{"type":"string","minLength":1}},"required":["path","missing"]}),
        );
        assert_eq!(schema["required"], json!(["path"]));
        assert!(schema.get("additionalProperties").is_none());
        assert!(schema["properties"]["path"].get("minLength").is_none());
    }
}
