use serde::{Deserialize, Serialize};

pub const DEFAULT_CODEX_AUTH_CONFIG_ID: &str = "codex-default";
pub const DEFAULT_ANTIGRAVITY_AUTH_CONFIG_ID: &str = "antigravity-default";
pub const DEFAULT_ANTIGRAVITY_URL: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal";
pub const DEFAULT_CODEX_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_ANTHROPIC_URL: &str = "https://api.anthropic.com/v1";
pub const DEFAULT_GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";
pub const DEFAULT_CODEX_MODEL_SLUGS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.2",
    "codex-auto-review",
];
pub const DEFAULT_CODEX_REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh"];
pub const DEFAULT_CODEX_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.5-low",
    "gpt-5.5-medium",
    "gpt-5.5-high",
    "gpt-5.5-xhigh",
    "gpt-5.4",
    "gpt-5.4-low",
    "gpt-5.4-medium",
    "gpt-5.4-high",
    "gpt-5.4-xhigh",
    "gpt-5.4-mini",
    "gpt-5.4-mini-low",
    "gpt-5.4-mini-medium",
    "gpt-5.4-mini-high",
    "gpt-5.4-mini-xhigh",
    "gpt-5.3-codex",
    "gpt-5.2",
    "codex-auto-review",
];

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModelProviderKind {
    OpenAiCompatible,
    AnthropicMessages,
    GeminiApi,
    CodexApiKey,
    CodexLogin,
    AntigravityLogin,
}

impl ModelProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI Compatible",
            Self::AnthropicMessages => "Anthropic Messages",
            Self::GeminiApi => "Gemini API",
            Self::CodexApiKey => "Codex API (Responses)",
            Self::CodexLogin => "Codex Login",
            Self::AntigravityLogin => "Antigravity Login",
        }
    }

    /// Key-based providers read credentials from settings or the environment;
    /// login providers use a local CLI or desktop client session instead.
    pub fn uses_api_key(self) -> bool {
        matches!(
            self,
            Self::OpenAiCompatible | Self::AnthropicMessages | Self::GeminiApi | Self::CodexApiKey
        )
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAiCompatible | Self::CodexApiKey => DEFAULT_OPENAI_URL,
            Self::AnthropicMessages => DEFAULT_ANTHROPIC_URL,
            Self::GeminiApi => DEFAULT_GEMINI_URL,
            Self::CodexLogin => DEFAULT_CODEX_URL,
            Self::AntigravityLogin => DEFAULT_ANTIGRAVITY_URL,
        }
    }

    /// Environment variables checked, in order, when a key-based config has no key of its own.
    pub fn default_api_key_envs(self) -> &'static [&'static str] {
        match self {
            Self::OpenAiCompatible | Self::CodexApiKey => &["OPENAI_API_KEY"],
            Self::AnthropicMessages => &["ANTHROPIC_API_KEY"],
            Self::GeminiApi => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            Self::CodexLogin | Self::AntigravityLogin => &[],
        }
    }
}

pub fn default_codex_models() -> Vec<String> {
    DEFAULT_CODEX_MODELS
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

pub fn normalize_codex_model(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return DEFAULT_CODEX_MODEL.to_string();
    }
    value.to_string()
}

pub fn normalize_provider_model(kind: ModelProviderKind, value: &str) -> String {
    if kind == ModelProviderKind::CodexLogin {
        normalize_codex_model(value)
    } else {
        value.trim().to_owned()
    }
}

pub fn codex_model_slug(value: &str) -> Option<&str> {
    let value = value.trim();
    if DEFAULT_CODEX_MODEL_SLUGS.contains(&value) {
        return Some(value);
    }
    for effort in DEFAULT_CODEX_REASONING_EFFORTS {
        if let Some(base_model) = value.strip_suffix(&format!("-{effort}")) {
            let base_model = base_model.trim();
            if DEFAULT_CODEX_MODEL_SLUGS.contains(&base_model) {
                return Some(base_model);
            }
        }
    }
    None
}

#[derive(Clone)]
pub struct ModelConfig {
    pub provider_kind: ModelProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub codex_home: Option<String>,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

impl ModelConfig {
    pub fn for_provider(
        provider_kind: ModelProviderKind,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            provider_kind,
            base_url: base_url.into(),
            model: model.into(),
            api_key: api_key.into(),
            codex_home: None,
            temperature: 0.2,
            max_output_tokens: 32_000,
        }
    }

    /// Compatibility defaults for the optional Codex login adapter.
    pub fn seeded() -> Self {
        Self::for_provider(
            ModelProviderKind::CodexLogin,
            DEFAULT_CODEX_URL,
            DEFAULT_CODEX_MODEL,
            "",
        )
    }
}

#[derive(Clone)]
pub struct ModelCompletionRequest {
    pub messages: Vec<ModelMessage>,
    pub temperature: f32,
    pub max_output_tokens: Option<u32>,
    pub tools: Vec<ModelToolDefinition>,
    /// `None` keeps the tools declared, so the request prefix stays byte-stable for provider
    /// caches and reasoning replay, while forbidding the model from calling them.
    pub tool_choice: ModelToolChoice,
    pub session_id: Option<u64>,
    pub turn_id: Option<u64>,
    pub request_name: Option<String>,
}

pub use daily_agent_core::model::{
    ModelCompletionResponse, ModelInlineImage, ModelMessage, ModelMessageRole, ModelStopReason,
    ModelStreamChunk, ModelToolCall, ModelToolChoice, ModelToolDefinition, ModelToolResultRef,
    ModelUsage, PROVIDER_ANTHROPIC, PROVIDER_GEMINI, PROVIDER_OPENAI_RESPONSES,
};
