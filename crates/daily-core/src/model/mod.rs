pub mod anthropic;
pub mod antigravity;
pub mod codex;
pub mod gemini;
pub mod openai;
pub mod types;

use async_trait::async_trait;
use reqwest::Client;
pub use types::{
    codex_model_slug, default_codex_models, normalize_codex_model, normalize_provider_model,
    ModelAuthConfig, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelConfigView,
    ModelInlineImage, ModelMessage, ModelMessageRole, ModelProviderKind, ModelStreamChunk,
    ModelToolCall, ModelToolDefinition, DEFAULT_ANTIGRAVITY_URL, DEFAULT_CODEX_AUTH_CONFIG_ID,
    DEFAULT_CODEX_MODEL, DEFAULT_CODEX_URL,
};

#[async_trait]
pub trait ModelAdapter: Send + Sync {
    fn kind(&self) -> ModelProviderKind;

    fn supports_images(&self) -> bool {
        false
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    async fn complete(
        &self,
        http: &Client,
        config: &ModelConfig,
        request: &ModelCompletionRequest,
        on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, String>;
}

pub fn adapter_for(kind: ModelProviderKind) -> Box<dyn ModelAdapter> {
    match kind {
        ModelProviderKind::OpenAiCompatible => Box::new(openai::OpenAiCompatibleAdapter),
        ModelProviderKind::AnthropicMessages => Box::new(anthropic::AnthropicMessagesAdapter),
        ModelProviderKind::GeminiApi => Box::new(gemini::GeminiApiAdapter),
        ModelProviderKind::CodexLogin => Box::new(codex::CodexLoginAdapter),
        ModelProviderKind::AntigravityLogin => Box::new(antigravity::AntigravityLoginAdapter),
    }
}
