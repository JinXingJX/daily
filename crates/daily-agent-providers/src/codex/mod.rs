//! Codex Responses adapters. API keys use the public Responses API; the unofficial login adapter
//! reuses the login stored by the local Codex CLI (`auth.json`) to call
//! the ChatGPT Codex backend. Not affiliated with or endorsed by OpenAI; it may violate the
//! service's terms and can stop working at any time. See `docs/subscription-adapters.md`.
use super::{
    codex_model_slug, ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig,
    ModelError, ModelErrorKind, ModelInlineImage, ModelMessage, ModelMessageRole,
    ModelProviderKind, ModelStopReason, ModelStreamChunk, ModelToolCall, ModelToolChoice,
    ModelToolDefinition, ModelUsage, DEFAULT_CODEX_MODEL, PROVIDER_OPENAI_RESPONSES,
};
#[cfg(feature = "login")]
use crate::fs_utils::read_limited_utf8;
use crate::net::describe_error_chain;
use crate::util::stable_hash64;
use async_trait::async_trait;
#[cfg(feature = "login")]
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[cfg(feature = "login")]
use base64::Engine;
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use reqwest::header::ACCEPT;
#[cfg(feature = "login")]
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
#[cfg(feature = "login")]
use reqwest::StatusCode;
use reqwest::{Client, Response};
#[cfg(feature = "login")]
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[cfg(feature = "login")]
use std::collections::HashMap;
use std::error::Error as StdError;
#[cfg(feature = "login")]
use std::fs::{self, OpenOptions};
#[cfg(feature = "login")]
use std::io::Write;
#[cfg(all(unix, feature = "login"))]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(feature = "login")]
use std::path::PathBuf;
#[cfg(feature = "login")]
use std::process::{Command, Stdio};
#[cfg(feature = "login")]
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, timeout};

mod api;
#[cfg(feature = "login")]
mod auth;
#[cfg(feature = "login")]
mod catalog;
mod stream;

pub use api::CodexApiKeyAdapter;
#[cfg(feature = "login")]
use auth::*;
#[cfg(feature = "login")]
pub use auth::{codex_auth_status, start_codex_login};
#[cfg(feature = "login")]
pub use catalog::{load_codex_model_catalog, CodexModelCatalog};
use stream::*;

#[cfg(feature = "login")]
const MAX_CODEX_AUTH_FILE_BYTES: usize = 2 * 1024 * 1024;

#[cfg(feature = "login")]
const CODEX_AUTH_FILE: &str = "auth.json";
#[cfg(feature = "login")]
const CODEX_REFRESH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
#[cfg(feature = "login")]
const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
#[cfg(feature = "login")]
const CODEX_DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
#[cfg(feature = "login")]
const CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";
const CODEX_DEFAULT_REASONING_EFFORT: &str = "medium";
const CODEX_MAX_REQUEST_ATTEMPTS: usize = 5;
const CODEX_MAX_STREAM_ATTEMPTS: usize = 3;
const CODEX_RETRY_BASE_DELAY_MS: u64 = 200;
const CODEX_STREAM_IDLE_TIMEOUT_MS: u64 = 300_000;
const X_CODEX_INSTALLATION_ID_HEADER: &str = "x-codex-installation-id";
#[cfg(feature = "login")]
const X_CODEX_TURN_STATE_HEADER: &str = "x-codex-turn-state";
const X_CODEX_TURN_METADATA_HEADER: &str = "x-codex-turn-metadata";
const X_CODEX_WINDOW_ID_HEADER: &str = "x-codex-window-id";

#[cfg(feature = "login")]
static CODEX_TURN_STATE_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
#[cfg(feature = "login")]
static CODEX_CLIENT_VERSION: OnceLock<String> = OnceLock::new();

struct ResponsesAdapter;

#[cfg(feature = "login")]
pub struct CodexLoginAdapter;

#[cfg(feature = "login")]
#[async_trait]
impl ModelAdapter for CodexLoginAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::CodexLogin
    }
    fn supports_images(&self) -> bool {
        true
    }
    fn supports_streaming(&self) -> bool {
        true
    }
    fn retries_internally(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        ResponsesAdapter
            .complete(http, config, request, on_chunk)
            .await
    }
}

#[cfg(feature = "login")]
#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CodexAuthStatusView {
    pub provider: String,
    pub codex_home: String,
    pub auth_path: String,
    pub authenticated: bool,
    pub auth_mode: Option<String>,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub is_fedramp_account: bool,
    pub codex_cli_available: bool,
    pub codex_cli_path: Option<String>,
    pub error: Option<String>,
}

#[cfg(feature = "login")]
#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StartCodexLoginInput {
    pub codex_home: Option<String>,
}

#[cfg(feature = "login")]
#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginStartView {
    pub pid: u32,
    pub command: String,
    pub status: CodexAuthStatusView,
}

#[async_trait]
impl ModelAdapter for ResponsesAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::CodexApiKey
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn retries_internally(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        mut on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        // The ChatGPT Codex responses endpoint requires streaming transport even
        // when the caller only wants the final aggregated text.
        let stream = true;
        let identity = CodexRequestIdentity::new(config, request);
        let body = if config.provider_kind == ModelProviderKind::CodexApiKey {
            api::build_api_request_body(config, request, &identity)
        } else {
            build_codex_request_body(config, request, stream, &identity)
        };

        let mut attempt = 0;
        loop {
            let response = send_codex_request_with_refresh(http, config, &body, stream, &identity)
                .await
                .map_err(|message| ModelError::new(ModelErrorKind::Network, message))?;
            let status = response.status();

            if !status.is_success() {
                let body = response.text().await.map_err(|error| {
                    ModelError::new(
                        ModelErrorKind::Network,
                        format!("Failed to read Codex error response: {error}"),
                    )
                })?;
                return Err(ModelError::from_status(
                    "Codex",
                    status.as_u16(),
                    &body,
                    None,
                ));
            }

            let result = match on_chunk.as_mut() {
                Some(on_chunk) => {
                    read_codex_stream(response, Some(&mut **on_chunk), &request.tools).await
                }
                None => read_codex_stream(response, None, &request.tools).await,
            };
            match result {
                Ok(response) => return Ok(response),
                // The connection can drop mid-body (proxies, flaky networks). Replaying the
                // request is only safe while nothing has been streamed to the caller yet.
                Err(error) if error.retryable && attempt + 1 < CODEX_MAX_STREAM_ATTEMPTS => {
                    sleep(codex_retry_delay(attempt)).await;
                    attempt += 1;
                }
                Err(error) if error.retryable => {
                    return Err(ModelError::new(
                        ModelErrorKind::Network,
                        format!(
                            "{} (gave up after {} attempts)",
                            error.message, CODEX_MAX_STREAM_ATTEMPTS
                        ),
                    ));
                }
                Err(error) => return Err(ModelError::from(error.message)),
            }
        }
    }
}

struct CodexStreamError {
    message: String,
    /// True when the transport failed before any chunk reached the caller, so the whole
    /// request can be replayed without duplicating streamed output.
    retryable: bool,
}

impl From<String> for CodexStreamError {
    fn from(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }
}

async fn read_codex_stream(
    response: Response,
    mut on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    tools: &[ModelToolDefinition],
) -> Result<ModelCompletionResponse, CodexStreamError> {
    let response_details = describe_response_transport(&response);
    let tool_name_map = tool_name_map(tools);
    let mut aggregated = String::new();
    let mut tool_calls = Vec::new();
    let mut reasoning_items = Vec::new();
    let mut usage = None;
    let idle_timeout = codex_stream_idle_timeout();
    let mut stream = response.bytes_stream().eventsource();

    loop {
        match timeout(idle_timeout, stream.next()).await {
            Ok(Some(Ok(event))) => {
                let payload = event.data.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }

                let parsed: Value = serde_json::from_str(payload)
                    .map_err(|error| format!("Failed to parse Codex stream event: {error}"))?;
                append_unique_tool_calls(
                    &mut tool_calls,
                    codex_tool_calls_from_stream_event(&parsed, &tool_name_map)?,
                );
                collect_reasoning_items(&mut reasoning_items, &parsed);
                if let Some(reported) = codex_usage_from_stream_event(&parsed) {
                    usage = Some(reported);
                }
                let text = codex_text_from_stream_event(&parsed, aggregated.trim().is_empty())?;
                if text.is_empty() {
                    continue;
                }

                aggregated.push_str(&text);
                if let Some(on_chunk) = on_chunk.as_mut() {
                    on_chunk(ModelStreamChunk { text })?;
                }
            }
            Ok(Some(Err(error))) => {
                return Err(CodexStreamError {
                    message: format!(
                        "Failed to read Codex stream event: {}; {response_details}",
                        describe_error_chain(&error)
                    ),
                    retryable: aggregated.is_empty()
                        && matches!(error, EventStreamError::Transport(_)),
                });
            }
            Ok(None) => break,
            Err(_) => {
                return Err(format!(
                    "Codex stream timed out after {}s without events; {response_details}",
                    idle_timeout.as_secs()
                )
                .into());
            }
        }
    }

    if aggregated.trim().is_empty() && tool_calls.is_empty() {
        return Err(String::from("Codex stream returned empty content.").into());
    }

    let stop_reason = if tool_calls.is_empty() {
        ModelStopReason::Complete
    } else {
        ModelStopReason::ToolCalls
    };
    Ok(ModelCompletionResponse {
        raw_output: aggregated,
        // Reasoning only matters to the tool round it belongs to.
        provider_items: if tool_calls.is_empty() {
            Vec::new()
        } else {
            reasoning_items
        },
        tool_calls,
        stop_reason,
        usage,
        provider: PROVIDER_OPENAI_RESPONSES.into(),
    })
}

fn describe_response_transport(response: &Response) -> String {
    let headers = response.headers();
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<none>")
    };

    format!(
        "status={}; version={:?}; content-type={}; content-encoding={}; transfer-encoding={}",
        response.status(),
        response.version(),
        header("content-type"),
        header("content-encoding"),
        header("transfer-encoding")
    )
}

#[derive(Clone)]
struct CodexRequestIdentity {
    installation_id: String,
    session_id: String,
    thread_id: String,
    turn_id: String,
    window_id: String,
    request_name: String,
    turn_started_at_unix_ms: i64,
}

impl CodexRequestIdentity {
    fn new(config: &ModelConfig, request: &ModelCompletionRequest) -> Self {
        let now = unix_timestamp_millis();
        let request_name = request
            .request_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("model.complete")
            .to_string();
        let codex_home = config.codex_home.as_deref().unwrap_or("default-codex-home");
        let request_seed = format!(
            "{}:{}:{}:{}",
            request_name,
            config.model,
            now,
            request
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );

        let session_seed = request
            .session_id
            .map(|id| format!("daily-session-{id}"))
            .unwrap_or_else(|| format!("daily-session-{request_seed}"));
        let thread_seed = request
            .session_id
            .map(|id| format!("daily-thread-{id}"))
            .unwrap_or_else(|| format!("daily-thread-{request_seed}"));
        let turn_seed = match (request.session_id, request.turn_id) {
            (Some(session_id), Some(turn_id)) => format!("daily-turn-{session_id}-{turn_id}"),
            _ => format!("daily-turn-{request_seed}"),
        };

        Self {
            installation_id: pseudo_uuid("daily-installation", codex_home),
            session_id: pseudo_uuid("daily-session", &session_seed),
            thread_id: pseudo_uuid("daily-thread", &thread_seed),
            turn_id: pseudo_uuid("daily-turn", &turn_seed),
            window_id: pseudo_uuid("daily-window", codex_home),
            request_name,
            turn_started_at_unix_ms: now,
        }
    }

    fn client_metadata(&self) -> Map<String, Value> {
        let turn_metadata = json!({
            "installation_id": self.installation_id.clone(),
            "session_id": self.session_id.clone(),
            "thread_id": self.thread_id.clone(),
            "turn_id": self.turn_id.clone(),
            "window_id": self.window_id.clone(),
            "request_kind": "turn",
            "turn_started_at_unix_ms": self.turn_started_at_unix_ms,
            "extra": {
                "client": "daily",
                "request_name": self.request_name.clone(),
            },
        });
        let turn_metadata_json =
            serde_json::to_string(&turn_metadata).unwrap_or_else(|_| "{}".to_string());

        let mut metadata = Map::new();
        metadata.insert(
            X_CODEX_INSTALLATION_ID_HEADER.into(),
            Value::String(self.installation_id.clone()),
        );
        metadata.insert("session_id".into(), Value::String(self.session_id.clone()));
        metadata.insert("thread_id".into(), Value::String(self.thread_id.clone()));
        metadata.insert("turn_id".into(), Value::String(self.turn_id.clone()));
        metadata.insert(
            X_CODEX_WINDOW_ID_HEADER.into(),
            Value::String(self.window_id.clone()),
        );
        metadata.insert(
            X_CODEX_TURN_METADATA_HEADER.into(),
            Value::String(turn_metadata_json),
        );
        metadata
    }

    #[cfg(feature = "login")]
    fn turn_metadata_header(&self) -> Option<String> {
        self.client_metadata()
            .remove(X_CODEX_TURN_METADATA_HEADER)
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }
}

fn build_codex_request_body(
    config: &ModelConfig,
    request: &ModelCompletionRequest,
    stream: bool,
    identity: &CodexRequestIdentity,
) -> Value {
    let instructions = request
        .messages
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let input = request
        .messages
        .iter()
        .filter(|message| message.role != ModelMessageRole::System)
        .flat_map(codex_input_items_from_message)
        .collect::<Vec<_>>();

    let (model, reasoning_effort) = codex_model_and_reasoning(&config.model);

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    if !instructions.is_empty() {
        body.insert("instructions".into(), json!(instructions));
    }
    body.insert("input".into(), Value::Array(input));
    body.insert(
        "tools".into(),
        Value::Array(
            request
                .tools
                .iter()
                .map(codex_tool_definition)
                .collect::<Vec<_>>(),
        ),
    );
    body.insert(
        "tool_choice".into(),
        json!(match request.tool_choice {
            ModelToolChoice::Auto => "auto",
            ModelToolChoice::None => "none",
        }),
    );
    body.insert("parallel_tool_calls".into(), json!(true));
    let reasoning_effort = reasoning_effort.unwrap_or(CODEX_DEFAULT_REASONING_EFFORT);
    body.insert(
        "reasoning".into(),
        json!({
            "effort": reasoning_effort,
            "summary": "auto",
        }),
    );
    body.insert("store".into(), json!(false));
    body.insert("stream".into(), json!(stream));
    body.insert("include".into(), json!(["reasoning.encrypted_content"]));
    body.insert("prompt_cache_key".into(), json!(identity.thread_id));
    if let Some(service_tier) = codex_service_tier() {
        body.insert("service_tier".into(), json!(service_tier));
    }
    body.insert(
        "client_metadata".into(),
        Value::Object(identity.client_metadata()),
    );
    Value::Object(body)
}

fn codex_model_and_reasoning(model: &str) -> (String, Option<&str>) {
    let model = model.trim();
    if model.is_empty() {
        return (
            DEFAULT_CODEX_MODEL.to_string(),
            Some(CODEX_DEFAULT_REASONING_EFFORT),
        );
    }
    for effort in ["xhigh", "medium", "high", "low"] {
        if let Some(base_model) = model.strip_suffix(&format!("-{effort}")) {
            let base_model = base_model.trim();
            if !base_model.is_empty() {
                return (base_model.to_string(), Some(effort));
            }
        }
    }
    (
        codex_model_slug(model).unwrap_or(model).to_string(),
        (model.is_empty()).then_some(CODEX_DEFAULT_REASONING_EFFORT),
    )
}

fn codex_service_tier() -> Option<String> {
    std::env::var("DAILY_CODEX_SERVICE_TIER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "default")
}

fn codex_stream_idle_timeout() -> Duration {
    std::env::var("DAILY_CODEX_STREAM_IDLE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(CODEX_STREAM_IDLE_TIMEOUT_MS))
}

#[cfg(feature = "login")]
fn codex_originator() -> String {
    std::env::var(CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CODEX_DEFAULT_ORIGINATOR.to_string())
}

#[cfg(feature = "login")]
fn codex_client_version() -> String {
    CODEX_CLIENT_VERSION
        .get_or_init(|| {
            std::env::var("DAILY_CODEX_CLIENT_VERSION")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(codex_cli_version)
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
        })
        .clone()
}

#[cfg(feature = "login")]
fn codex_cli_version() -> Option<String> {
    let codex_bin = resolve_codex_binary()?;
    let output = Command::new(codex_bin).arg("--version").output().ok()?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    parse_codex_version(&text)
}

#[cfg(any(feature = "login", test))]
fn parse_codex_version(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|ch: char| {
            !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '+')
        });
        let token = token.strip_prefix('v').unwrap_or(token);
        let mut parts = token.splitn(4, ['.', '-', '+']);
        let major = parts.next()?;
        let minor = parts.next()?;
        let patch = parts.next()?;
        if major.chars().all(|ch| ch.is_ascii_digit())
            && minor.chars().all(|ch| ch.is_ascii_digit())
            && patch.chars().all(|ch| ch.is_ascii_digit())
        {
            Some(token.to_string())
        } else {
            None
        }
    })
}

#[cfg(any(feature = "login", test))]
fn codex_user_agent(originator: &str, version: &str) -> String {
    format!(
        "{originator}/{version} ({}; {}) Daily",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn unix_timestamp_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn pseudo_uuid(namespace: &str, value: &str) -> String {
    let a = stable_hash64(&format!("{namespace}:{value}:a"));
    let b = stable_hash64(&format!("{namespace}:{value}:b"));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) as u32,
        ((a >> 16) & 0xffff) as u16,
        (a & 0x0fff) as u16,
        (0x8000 | ((b >> 48) & 0x3fff)) as u16,
        b & 0x0000_ffff_ffff_ffff
    )
}

/// One message as Responses API input items. An assistant turn that called tools becomes its
/// reasoning items (as returned, so the model keeps its chain of thought), its text, and one
/// `function_call` per call; each tool result is a `function_call_output` with the same call id.
fn codex_input_items_from_message(message: &ModelMessage) -> Vec<Value> {
    let text_item = |role: &str, content_type: &str| {
        json!({
            "type": "message",
            "role": role,
            "content": [{ "type": content_type, "text": message.content }],
        })
    };
    match message.role {
        ModelMessageRole::System | ModelMessageRole::User => {
            let mut item = text_item("user", "input_text");
            attach_inline_images(&mut item, &message.images);
            vec![item]
        }
        ModelMessageRole::Assistant => {
            let mut items = Vec::new();
            if message.provider == PROVIDER_OPENAI_RESPONSES {
                items.extend(message.provider_items.iter().cloned());
            }
            if !message.content.trim().is_empty() || message.tool_calls.is_empty() {
                items.push(text_item("assistant", "output_text"));
            }
            items.extend(message.tool_calls.iter().enumerate().map(|(index, call)| {
                json!({
                    "type": "function_call",
                    "call_id": call.call_id.clone().unwrap_or_else(|| format!("call_{index}")),
                    "name": api_tool_name(&call.tool_id),
                    "arguments": call.arguments.to_string(),
                })
            }));
            items
        }
        ModelMessageRole::Tool => vec![json!({
            "type": "function_call_output",
            "call_id": message.tool_result.as_ref().map(|result| result.call_id.as_str()).unwrap_or_default(),
            "output": message.content,
        })],
    }
}

/// Adds the images to a user message item as Responses API `input_image` parts.
fn attach_inline_images(item: &mut Value, images: &[ModelInlineImage]) {
    let Some(content) = item["content"].as_array_mut() else {
        return;
    };
    content.extend(images.iter().map(|image| {
        json!({
            "type": "input_image",
            "image_url": format!("data:{};base64,{}", image.mime_type, image.data_base64),
        })
    }));
}

fn codex_tool_definition(tool: &ModelToolDefinition) -> Value {
    json!({
        "type": "function",
        "name": api_tool_name(&tool.id),
        "description": format!("{}: {} (Daily tool id: {})", tool.name, tool.description, tool.id),
        "parameters": tool.input_schema,
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

async fn send_codex_request_with_refresh(
    http: &Client,
    config: &ModelConfig,
    body: &Value,
    _stream: bool,
    _identity: &CodexRequestIdentity,
) -> Result<Response, String> {
    if config.provider_kind != ModelProviderKind::CodexApiKey {
        #[cfg(feature = "login")]
        return send_login_request_with_refresh(http, config, body, _stream, _identity).await;
        #[cfg(not(feature = "login"))]
        return Err("Enable the login feature to use Codex login.".into());
    }
    for attempt in 0..CODEX_MAX_REQUEST_ATTEMPTS {
        let result = api::send_api_request(http, config, body).await;
        let retry = match &result {
            Ok(response) => should_retry_codex_status(response.status()),
            Err(_) => true,
        };
        if retry && attempt + 1 < CODEX_MAX_REQUEST_ATTEMPTS {
            sleep(codex_retry_delay(attempt)).await;
        } else {
            return result;
        }
    }
    unreachable!("request retry budget is non-zero")
}

#[cfg(feature = "login")]
async fn send_login_request_with_refresh(
    http: &Client,
    config: &ModelConfig,
    body: &Value,
    stream: bool,
    identity: &CodexRequestIdentity,
) -> Result<Response, String> {
    // Key-based requests must never read or refresh the user's CLI credentials.
    let mut auth = if config.provider_kind == ModelProviderKind::CodexApiKey {
        None
    } else {
        Some(load_codex_auth(config.codex_home.as_deref())?)
    };
    let mut refreshed = false;
    let mut last_error = None;

    for attempt in 0..CODEX_MAX_REQUEST_ATTEMPTS {
        let result = match auth.as_ref() {
            Some(auth) => send_codex_request(http, config, body, auth, stream, identity).await,
            None => api::send_api_request(http, config, body).await,
        };
        match result {
            Ok(response)
                if response.status() == StatusCode::UNAUTHORIZED
                    && !refreshed
                    && auth.is_some() =>
            {
                auth = Some(refresh_codex_auth(http, auth.take().unwrap()).await?);
                refreshed = true;
                continue;
            }
            Ok(response)
                if should_retry_codex_status(response.status())
                    && attempt + 1 < CODEX_MAX_REQUEST_ATTEMPTS =>
            {
                sleep(codex_retry_delay(attempt)).await;
                continue;
            }
            Ok(response) => return Ok(response),
            Err(error) if attempt + 1 < CODEX_MAX_REQUEST_ATTEMPTS => {
                last_error = Some(error);
                sleep(codex_retry_delay(attempt)).await;
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| "Codex request failed after retries.".into()))
}

#[cfg(feature = "login")]
async fn send_codex_request(
    http: &Client,
    config: &ModelConfig,
    body: &Value,
    auth: &CodexAuthSnapshot,
    stream: bool,
    identity: &CodexRequestIdentity,
) -> Result<Response, String> {
    let endpoint = format!("{}/responses", config.base_url.trim_end_matches('/'));
    let originator = codex_originator();
    let client_version = codex_client_version();
    let user_agent = codex_user_agent(&originator, &client_version);
    let body = serde_json::to_vec(body)
        .map_err(|error| format!("Failed to encode Codex request body: {error}"))?;
    let mut request = http
        .post(endpoint)
        .bearer_auth(&auth.access_token)
        .header("originator", originator)
        .header(USER_AGENT, user_agent)
        .header("version", client_version)
        .header("session-id", &identity.session_id)
        .header("thread-id", &identity.thread_id)
        .header("x-client-request-id", &identity.thread_id)
        .header(X_CODEX_WINDOW_ID_HEADER, &identity.window_id)
        .header(CONTENT_TYPE, "application/json")
        .body(body);

    if let Some(turn_metadata) = identity.turn_metadata_header() {
        request = request.header(X_CODEX_TURN_METADATA_HEADER, turn_metadata);
    }
    if let Some(turn_state) = cached_codex_turn_state(&identity.thread_id) {
        request = request.header(X_CODEX_TURN_STATE_HEADER, turn_state);
    }
    if stream {
        request = request.header(ACCEPT, "text/event-stream");
    }
    if let Some(account_id) = auth.account_id.as_deref() {
        request = request.header("ChatGPT-Account-ID", account_id);
    }
    if auth.is_fedramp_account {
        request = request.header("X-OpenAI-Fedramp", "true");
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("Codex request failed: {}", describe_reqwest_error(&error)))?;
    record_codex_turn_state(&identity.thread_id, &response);
    Ok(response)
}

fn should_retry_codex_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
}

fn codex_retry_delay(attempt: usize) -> Duration {
    let shift = attempt.min(10) as u32;
    Duration::from_millis(CODEX_RETRY_BASE_DELAY_MS.saturating_mul(2_u64.saturating_pow(shift)))
}

#[cfg(feature = "login")]
fn cached_codex_turn_state(thread_id: &str) -> Option<String> {
    CODEX_TURN_STATE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(thread_id).cloned())
}

#[cfg(feature = "login")]
fn record_codex_turn_state(thread_id: &str, response: &Response) {
    let Some(turn_state) = response
        .headers()
        .get(X_CODEX_TURN_STATE_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
    else {
        return;
    };
    if let Ok(mut cache) = CODEX_TURN_STATE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        cache.insert(thread_id.to_string(), turn_state);
    }
}

fn describe_reqwest_error(error: &reqwest::Error) -> String {
    let mut details = vec![error.to_string()];
    if error.is_timeout() {
        details.push("kind=timeout".into());
    }
    if error.is_connect() {
        details.push("kind=connect".into());
        details.push("hint=Could not connect to the model endpoint; check the network, VPN/proxy, DNS, TLS certificates and system clock. Daily honors the system proxy and HTTP_PROXY/HTTPS_PROXY, and supports custom CAs via CODEX_CA_CERTIFICATE/SSL_CERT_FILE.".into());
        let env_details = codex_connectivity_env_details();
        if !env_details.is_empty() {
            details.push(format!("env={}", env_details.join(", ")));
        }
    }
    if error.is_request() {
        details.push("kind=request".into());
    }
    if let Some(status) = error.status() {
        details.push(format!("status={status}"));
    }
    if let Some(url) = error.url() {
        details.push(format!("url={url}"));
    }

    let mut source = error.source();
    while let Some(next) = source {
        details.push(format!("source={next}"));
        source = next.source();
    }
    details.join("; ")
}

fn codex_connectivity_env_details() -> Vec<String> {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
        "CODEX_CA_CERTIFICATE",
        "SSL_CERT_FILE",
    ]
    .into_iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| format!("{name}={}", redact_proxy_secret(&value)))
    })
    .collect()
}

fn redact_proxy_secret(value: &str) -> String {
    match value.find("://") {
        Some(scheme_end) => {
            let auth_start = scheme_end + 3;
            if let Some(at_offset) = value[auth_start..].find('@') {
                let at = auth_start + at_offset;
                format!("{}***{}", &value[..auth_start], &value[at..])
            } else {
                value.to_string()
            }
        }
        None => value.to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_retry_delay_uses_official_base_delay() {
        assert_eq!(
            codex_retry_delay(0),
            Duration::from_millis(CODEX_RETRY_BASE_DELAY_MS)
        );
        assert_eq!(
            codex_retry_delay(1),
            Duration::from_millis(CODEX_RETRY_BASE_DELAY_MS * 2)
        );
    }

    #[test]
    fn codex_user_agent_includes_originator_and_version() {
        let user_agent = codex_user_agent("codex_cli_rs", "0.142.0");

        assert!(user_agent.starts_with("codex_cli_rs/0.142.0"));
        assert!(user_agent.contains("Daily"));
    }

    #[test]
    fn parse_codex_version_reads_cli_output() {
        assert_eq!(
            parse_codex_version("codex-cli 0.142.0\n").as_deref(),
            Some("0.142.0")
        );
        assert_eq!(
            parse_codex_version(
                "WARNING: proceeding, even though we could not create PATH aliases\ncodex 1.2.3\n"
            )
            .as_deref(),
            Some("1.2.3")
        );
    }
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn images_join_their_user_message_as_input_images() {
        let mut user = ModelMessage::user("look");
        user.images.push(ModelInlineImage {
            name: None,
            mime_type: "image/png".into(),
            data_base64: "QUJD".into(),
        });
        let items = codex_input_items_from_message(&user);
        assert_eq!(items[0]["content"][1]["type"], "input_image");
        assert_eq!(
            items[0]["content"][1]["image_url"],
            "data:image/png;base64,QUJD"
        );
        let reply = codex_input_items_from_message(&ModelMessage::assistant("ok"));
        assert_eq!(reply[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(reply[0]["content"][0]["type"], "output_text");
    }

    #[test]
    fn tool_rounds_replay_reasoning_calls_and_outputs_as_native_items() {
        let mut assistant = ModelMessage::assistant("");
        assistant.provider = PROVIDER_OPENAI_RESPONSES.into();
        assistant.provider_items =
            vec![json!({ "type": "reasoning", "summary": [], "encrypted_content": "enc" })];
        assistant.tool_calls.push(ModelToolCall {
            call_id: Some("call_1".into()),
            tool_id: "daily.lookup".into(),
            arguments: json!({ "query": "x" }),
            provider_meta: Value::Null,
        });
        let items = codex_input_items_from_message(&assistant);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["encrypted_content"], "enc");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["call_id"], "call_1");
        assert_eq!(items[1]["name"], "daily_lookup");
        assert_eq!(items[1]["arguments"], "{\"query\":\"x\"}");

        // Reasoning from another provider is not sent.
        assistant.provider = "anthropic".into();
        assert_eq!(codex_input_items_from_message(&assistant).len(), 1);

        let output = codex_input_items_from_message(&ModelMessage::tool_result(
            "call_1",
            "daily.lookup",
            "found",
            false,
        ));
        assert_eq!(
            output[0],
            json!({ "type": "function_call_output", "call_id": "call_1", "output": "found" })
        );
    }

    #[test]
    fn stream_events_yield_reasoning_items_and_usage() {
        let mut items = Vec::new();
        collect_reasoning_items(
            &mut items,
            &json!({ "type": "response.output_item.done", "item": {
                "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "s" }],
                "encrypted_content": "enc", "status": "completed"
            } }),
        );
        // Items without encrypted content cannot be replayed when nothing is stored server-side.
        collect_reasoning_items(
            &mut items,
            &json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_2", "summary": [] } }),
        );
        assert_eq!(items.len(), 1);
        assert!(items[0].get("id").is_none());
        assert_eq!(items[0]["encrypted_content"], "enc");
        assert_eq!(items[0]["summary"][0]["text"], "s");

        let usage = codex_usage_from_stream_event(
            &json!({ "type": "response.completed", "response": { "usage": {
            "input_tokens": 120, "output_tokens": 30,
            "input_tokens_details": { "cached_tokens": 100 },
            "output_tokens_details": { "reasoning_tokens": 12 }
        } } }),
        )
        .unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
                usage.reasoning_tokens
            ),
            (120, 30, 100, 12)
        );
    }
}
