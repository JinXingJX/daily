pub mod anthropic;
#[cfg(feature = "login")]
pub mod antigravity;
pub mod codex;
pub mod gemini;
pub mod openai;
pub mod types;
pub(crate) mod wire;

use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

pub use daily_agent_core::model::{ModelError, ModelErrorKind};
pub use types::{
    codex_model_slug, default_codex_models, normalize_codex_model, normalize_provider_model,
    ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelInlineImage, ModelMessage,
    ModelMessageRole, ModelProviderKind, ModelStopReason, ModelStreamChunk, ModelToolCall,
    ModelToolChoice, ModelToolDefinition, ModelToolResultRef, ModelUsage, DEFAULT_ANTIGRAVITY_URL,
    DEFAULT_CODEX_AUTH_CONFIG_ID, DEFAULT_CODEX_MODEL, DEFAULT_CODEX_URL, PROVIDER_ANTHROPIC,
    PROVIDER_GEMINI, PROVIDER_OPENAI_RESPONSES,
};

/// Receives streamed text; returning an error stops the request.
pub type StreamCallback<'a> = dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send + 'a;

/// One model provider. Implement this to plug in a provider Daily does not ship.
#[async_trait]
pub trait ModelAdapter: Send + Sync {
    fn kind(&self) -> ModelProviderKind;

    fn supports_images(&self) -> bool {
        false
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    /// True when `complete` already retries transient failures, so callers must not.
    fn retries_internally(&self) -> bool {
        false
    }

    /// Runs one request. With `on_chunk`, text is delivered as it is generated.
    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError>;
}

pub fn adapter_for(kind: ModelProviderKind) -> Box<dyn ModelAdapter> {
    match kind {
        ModelProviderKind::OpenAiCompatible => Box::new(openai::OpenAiCompatibleAdapter),
        ModelProviderKind::AnthropicMessages => Box::new(anthropic::AnthropicMessagesAdapter),
        ModelProviderKind::GeminiApi => Box::new(gemini::GeminiApiAdapter),
        ModelProviderKind::CodexApiKey => Box::new(codex::CodexApiKeyAdapter),
        #[cfg(feature = "login")]
        ModelProviderKind::CodexLogin => Box::new(codex::CodexLoginAdapter),
        #[cfg(feature = "login")]
        ModelProviderKind::AntigravityLogin => Box::new(antigravity::AntigravityLoginAdapter),
        #[cfg(not(feature = "login"))]
        ModelProviderKind::CodexLogin | ModelProviderKind::AntigravityLogin => {
            Box::new(UnavailableLoginAdapter(kind))
        }
    }
}

const MAX_ATTEMPTS: usize = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(500);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Runs `request` on the configured provider, retrying transient failures.
pub async fn complete_model(
    http: &Client,
    config: &ModelConfig,
    request: &ModelCompletionRequest,
    on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
) -> Result<ModelCompletionResponse, ModelError> {
    let adapter = adapter_for(config.provider_kind);
    complete_with_retry(adapter.as_ref(), http, config, request, on_chunk).await
}

/// Runs `request` on `adapter`. Rate limits, overloads, timeouts, network and server errors
/// are retried with backoff, but only while nothing has been streamed to the caller: replaying
/// a request after partial output would duplicate it.
pub async fn complete_with_retry(
    adapter: &dyn ModelAdapter,
    http: &Client,
    config: &ModelConfig,
    request: &ModelCompletionRequest,
    mut on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
) -> Result<ModelCompletionResponse, ModelError> {
    if adapter.retries_internally() {
        return adapter.complete(http, config, request, on_chunk).await;
    }
    let mut attempt = 0;
    loop {
        let mut streamed = false;
        let result = match on_chunk.as_mut() {
            Some(callback) => {
                let mut forward = |chunk: ModelStreamChunk| {
                    streamed = true;
                    callback(chunk)
                };
                adapter
                    .complete(http, config, request, Some(&mut forward))
                    .await
            }
            None => adapter.complete(http, config, request, None).await,
        };
        let error = match result {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        let delay = error
            .retry_after
            .unwrap_or(RETRY_BASE_DELAY * 2u32.pow(attempt as u32));
        if !error.is_retryable()
            || streamed
            || attempt + 1 >= MAX_ATTEMPTS
            || delay > MAX_RETRY_DELAY
        {
            return Err(error);
        }
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A scripted HTTP server for adapter and agent-loop tests.
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Clone)]
    pub(crate) struct Reply {
        pub status: u16,
        pub content_type: &'static str,
        pub body: String,
    }

    impl Reply {
        pub(crate) fn json(body: Value) -> Self {
            Self {
                status: 200,
                content_type: "application/json",
                body: body.to_string(),
            }
        }

        /// A server-sent event stream with one `data:` event per value.
        pub(crate) fn sse(events: &[Value]) -> Self {
            Self {
                status: 200,
                content_type: "text/event-stream",
                body: events
                    .iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect(),
            }
        }

        pub(crate) fn error(status: u16, body: &str) -> Self {
            Self {
                status,
                content_type: "application/json",
                body: body.to_string(),
            }
        }
    }

    /// One recorded request: its lower-cased head and JSON body.
    #[derive(Clone)]
    pub(crate) struct Recorded {
        pub head: String,
        pub body: Value,
    }

    /// Serves `replies` in order and records every request.
    pub(crate) async fn scripted_server(
        replies: Vec<Reply>,
    ) -> (String, Arc<Mutex<Vec<Recorded>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&seen);
                let queue = Arc::clone(&queue);
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    loop {
                        let mut chunk = [0u8; 8192];
                        let Ok(read) = socket.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                            continue;
                        };
                        let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buffer.len() < end + 4 + length {
                            continue;
                        }
                        let body = buffer[end + 4..end + 4 + length].to_vec();
                        buffer.drain(..end + 4 + length);
                        seen.lock().unwrap().push(Recorded {
                            head,
                            body: serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
                        });
                        let reply = queue
                            .lock()
                            .unwrap()
                            .pop_front()
                            .expect("no scripted response left");
                        let response = format!(
                            "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\n\r\n{}",
                            reply.status,
                            reply.content_type,
                            reply.body.len(),
                            reply.body
                        );
                        if socket.write_all(response.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (format!("http://{address}"), requests)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{scripted_server, Reply};
    use super::*;
    use serde_json::json;

    fn config(base_url: String) -> ModelConfig {
        ModelConfig {
            provider_kind: ModelProviderKind::OpenAiCompatible,
            base_url,
            model: "test".into(),
            api_key: String::new(),
            ..ModelConfig::seeded()
        }
    }

    fn request() -> ModelCompletionRequest {
        ModelCompletionRequest {
            messages: vec![ModelMessage::user("hi")],
            temperature: 0.2,
            max_output_tokens: Some(64),
            tools: Vec::new(),
            tool_choice: ModelToolChoice::Auto,
            session_id: None,
            turn_id: None,
            request_name: None,
        }
    }

    #[tokio::test]
    async fn transient_failures_are_retried_and_permanent_ones_are_not() {
        let ok =
            json!({ "choices": [{ "message": { "content": "hello" }, "finish_reason": "stop" }] });
        let (url, requests) = scripted_server(vec![
            Reply::error(429, "{\"error\":\"slow down\"}"),
            Reply::json(ok),
            Reply::error(401, "{\"error\":\"bad key\"}"),
        ])
        .await;
        let http = Client::builder().no_proxy().build().unwrap();
        let response = complete_model(&http, &config(url.clone()), &request(), None)
            .await
            .unwrap();
        assert_eq!(response.raw_output, "hello");
        assert_eq!(requests.lock().unwrap().len(), 2);

        let error = complete_model(&http, &config(url), &request(), None)
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, ModelErrorKind::Auth);
        assert_eq!(requests.lock().unwrap().len(), 3);
    }
}

/// Maps transport failures without making the core depend on an HTTP client.
pub(crate) fn transport_error(label: &str, error: &reqwest::Error) -> ModelError {
    ModelError::new(
        if error.is_timeout() {
            ModelErrorKind::Timeout
        } else {
            ModelErrorKind::Network
        },
        format!(
            "{label} request failed: {}",
            crate::net::describe_error_chain(error)
        ),
    )
}

#[cfg(not(feature = "login"))]
struct UnavailableLoginAdapter(ModelProviderKind);
#[cfg(not(feature = "login"))]
#[async_trait]
impl ModelAdapter for UnavailableLoginAdapter {
    fn kind(&self) -> ModelProviderKind {
        self.0
    }
    async fn complete(
        &self,
        _: &Client,
        _: &ModelConfig,
        _: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        Err(ModelError::new(
            ModelErrorKind::InvalidRequest,
            "Enable the daily-agent-providers login feature to use login adapters.",
        ))
    }
}

mod bound;
pub use bound::ProviderModel;
#[cfg(feature = "login")]
mod fs_utils;
mod net;
mod util;

/// Stable endpoint identity without serializing URLs or credentials into agent checkpoints.
pub fn model_identity(config: &ModelConfig) -> daily_agent_core::model::ModelIdentity {
    daily_agent_core::model::ModelIdentity {
        provider: format!(
            "{}@{:016x}",
            config.provider_kind.label(),
            util::stable_hash64(config.base_url.trim_end_matches('/'))
        ),
        model: config.model.clone(),
    }
}
