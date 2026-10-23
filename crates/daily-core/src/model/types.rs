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
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ModelProviderKind {
    OpenAiCompatible,
    AnthropicMessages,
    GeminiApi,
    CodexLogin,
    AntigravityLogin,
}

impl ModelProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI Compatible",
            Self::AnthropicMessages => "Anthropic Messages",
            Self::GeminiApi => "Gemini API",
            Self::CodexLogin => "Codex Login",
            Self::AntigravityLogin => "Antigravity Login",
        }
    }

    /// Key-based providers read credentials from settings or the environment;
    /// login providers use a local CLI or desktop client session instead.
    pub fn uses_api_key(self) -> bool {
        matches!(
            self,
            Self::OpenAiCompatible | Self::AnthropicMessages | Self::GeminiApi
        )
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => DEFAULT_OPENAI_URL,
            Self::AnthropicMessages => DEFAULT_ANTHROPIC_URL,
            Self::GeminiApi => DEFAULT_GEMINI_URL,
            Self::CodexLogin => DEFAULT_CODEX_URL,
            Self::AntigravityLogin => DEFAULT_ANTIGRAVITY_URL,
        }
    }

    /// Environment variables checked, in order, when a key-based config has no key of its own.
    fn default_api_key_envs(self) -> &'static [&'static str] {
        match self {
            Self::OpenAiCompatible => &["OPENAI_API_KEY"],
            Self::AnthropicMessages => &["ANTHROPIC_API_KEY"],
            Self::GeminiApi => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            Self::CodexLogin | Self::AntigravityLogin => &[],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelAuthConfig {
    pub id: String,
    pub name: String,
    pub provider_kind: ModelProviderKind,
    pub base_url: String,
    pub codex_home: Option<String>,
    pub models: Vec<String>,
    pub default_model: String,
    /// Take `models` and `default_model` from the local Codex CLI state when available.
    #[serde(default = "default_follow_codex")]
    pub follow_codex: bool,
    /// API key for key-based providers. Only read from user and local settings.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// Environment variable holding the API key; takes precedence over `api_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

fn default_follow_codex() -> bool {
    true
}

impl ModelAuthConfig {
    pub fn antigravity_default() -> Self {
        Self {
            id: DEFAULT_ANTIGRAVITY_AUTH_CONFIG_ID.into(),
            name: "Antigravity".into(),
            provider_kind: ModelProviderKind::AntigravityLogin,
            base_url: DEFAULT_ANTIGRAVITY_URL.into(),
            codex_home: None,
            models: Vec::new(),
            default_model: String::new(),
            follow_codex: false,
            api_key: String::new(),
            api_key_env: None,
        }
    }

    pub fn codex_default() -> Self {
        Self {
            id: DEFAULT_CODEX_AUTH_CONFIG_ID.into(),
            name: "Codex".into(),
            provider_kind: ModelProviderKind::CodexLogin,
            base_url: DEFAULT_CODEX_URL.into(),
            codex_home: None,
            models: default_codex_models(),
            default_model: DEFAULT_CODEX_MODEL.into(),
            follow_codex: true,
            api_key: String::new(),
            api_key_env: None,
        }
    }

    /// The key sent to a key-based provider: the configured environment variable, then the
    /// stored key, then the provider's conventional environment variables.
    pub fn resolved_api_key(&self) -> String {
        if !self.provider_kind.uses_api_key() {
            return String::new();
        }
        let from_env = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        self.api_key_env
            .as_deref()
            .and_then(from_env)
            .or_else(|| (!self.api_key.trim().is_empty()).then(|| self.api_key.trim().to_owned()))
            .or_else(|| {
                self.provider_kind
                    .default_api_key_envs()
                    .iter()
                    .find_map(|name| from_env(name))
            })
            .unwrap_or_default()
    }

    /// Replaces the model list and default with the Codex catalog when this
    /// config follows Codex and the catalog could be loaded.
    pub fn with_codex_catalog(mut self) -> Self {
        if self.provider_kind != ModelProviderKind::CodexLogin || !self.follow_codex {
            return self;
        }
        if let Some(catalog) = super::codex::load_codex_model_catalog(self.codex_home.as_deref()) {
            self.models = catalog.models;
            if let Some(default_model) = catalog.default_model {
                self.default_model = default_model;
            }
        }
        self.normalized()
    }

    pub fn normalized(mut self) -> Self {
        match self.provider_kind {
            ModelProviderKind::AntigravityLogin => self.normalized_antigravity(),
            ModelProviderKind::CodexLogin => self.normalized_codex(),
            kind => {
                self.id = normalized_config_string(&self.id, &kind_slug(kind));
                self.name = normalized_config_string(&self.name, kind.label());
                self.base_url = normalized_config_string(&self.base_url, kind.default_base_url());
                self.codex_home = None;
                self.follow_codex = false;
                self.api_key = self.api_key.trim().to_owned();
                self.api_key_env = self
                    .api_key_env
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned);
                self.models = dedupe_models(
                    self.models
                        .into_iter()
                        .map(|model| model.trim().to_owned())
                        .collect(),
                );
                self.default_model = self.default_model.trim().to_owned();
                if self.default_model.is_empty() {
                    self.default_model = self.models.first().cloned().unwrap_or_default();
                } else if !self.models.contains(&self.default_model) {
                    self.models.insert(0, self.default_model.clone());
                }
                self
            }
        }
    }

    fn normalized_antigravity(mut self) -> Self {
        let defaults = Self::antigravity_default();
        self.id = normalized_config_string(&self.id, &defaults.id);
        self.name = normalized_config_string(&self.name, &defaults.name);
        // Subscription credentials must only be sent to the Antigravity gateway.
        self.base_url = DEFAULT_ANTIGRAVITY_URL.into();
        self.codex_home = None;
        self.follow_codex = false;
        self.api_key.clear();
        self.api_key_env = None;
        self.models = dedupe_models(
            self.models
                .into_iter()
                .map(|m| m.trim().to_owned())
                .collect(),
        );
        self.default_model = self.default_model.trim().to_owned();
        if !self.models.contains(&self.default_model) {
            self.default_model = self.models.first().cloned().unwrap_or_default();
        }
        self
    }

    fn normalized_codex(mut self) -> Self {
        self.id = normalized_config_string(&self.id, DEFAULT_CODEX_AUTH_CONFIG_ID);
        self.name = normalized_config_string(&self.name, "Codex");
        self.base_url = normalized_config_string(&self.base_url, DEFAULT_CODEX_URL);
        self.codex_home = self
            .codex_home
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        self.api_key.clear();
        self.api_key_env = None;
        self.models = dedupe_models(
            self.models
                .into_iter()
                .map(|model| normalize_codex_model(&model))
                .collect(),
        );
        self.default_model = normalize_codex_model(&self.default_model);
        if self.models.is_empty() {
            self.models = default_codex_models();
        }
        if !self.models.iter().any(|model| model == &self.default_model) {
            self.models.insert(0, self.default_model.clone());
        }
        self
    }
}

fn kind_slug(kind: ModelProviderKind) -> String {
    match kind {
        ModelProviderKind::OpenAiCompatible => "openai",
        ModelProviderKind::AnthropicMessages => "anthropic",
        ModelProviderKind::GeminiApi => "gemini",
        ModelProviderKind::CodexLogin => DEFAULT_CODEX_AUTH_CONFIG_ID,
        ModelProviderKind::AntigravityLogin => DEFAULT_ANTIGRAVITY_AUTH_CONFIG_ID,
    }
    .to_string()
}

fn normalized_config_string(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
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

fn dedupe_models(models: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    for model in models {
        if !model.is_empty() && !result.iter().any(|existing| existing == &model) {
            result.push(model);
        }
    }
    result
}

#[derive(Clone)]
pub struct ModelConfig {
    pub auth_config_id: String,
    pub auth_configs: Vec<ModelAuthConfig>,
    pub provider_kind: ModelProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub codex_home: Option<String>,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigView {
    pub auth_config_id: String,
    pub auth_configs: Vec<ModelAuthConfig>,
    pub provider_kind: ModelProviderKind,
    pub base_url: String,
    pub model: String,
    pub codex_home: Option<String>,
    pub temperature: f32,
    pub max_output_tokens: u32,
    pub api_key_set: bool,
}

impl ModelConfig {
    pub fn seeded() -> Self {
        let auth_config = ModelAuthConfig::codex_default();
        Self {
            auth_config_id: auth_config.id.clone(),
            auth_configs: vec![auth_config.clone()],
            provider_kind: ModelProviderKind::CodexLogin,
            base_url: auth_config.base_url,
            model: auth_config.default_model,
            api_key: String::new(),
            codex_home: auth_config.codex_home,
            temperature: 0.2,
            max_output_tokens: 1024,
        }
    }

    pub fn selected_auth_config(&self) -> ModelAuthConfig {
        self.auth_configs
            .iter()
            .find(|auth_config| auth_config.id == self.auth_config_id)
            .or_else(|| self.auth_configs.first())
            .cloned()
            .unwrap_or_else(ModelAuthConfig::codex_default)
            .normalized()
    }

    /// Points this config at `auth_config`: provider, endpoint, Codex home and resolved key.
    pub fn apply_auth_config(&mut self, auth_config: &ModelAuthConfig) {
        self.auth_config_id = auth_config.id.clone();
        self.provider_kind = auth_config.provider_kind;
        self.base_url = auth_config.base_url.clone();
        self.codex_home = auth_config.codex_home.clone();
        self.api_key = auth_config.resolved_api_key();
    }

    pub fn view(&self) -> ModelConfigView {
        ModelConfigView {
            auth_config_id: self.auth_config_id.clone(),
            auth_configs: self.auth_configs.clone(),
            provider_kind: self.provider_kind,
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            codex_home: self.codex_home.clone(),
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
            api_key_set: !self.api_key.is_empty(),
        }
    }
}

#[derive(Clone)]
pub struct ModelCompletionRequest {
    pub messages: Vec<ModelMessage>,
    pub temperature: f32,
    pub max_output_tokens: Option<u32>,
    pub inline_images: Vec<ModelInlineImage>,
    pub tools: Vec<ModelToolDefinition>,
    pub session_id: Option<u64>,
    pub turn_id: Option<u64>,
    pub request_name: Option<String>,
}

pub struct ModelCompletionResponse {
    pub raw_output: String,
    pub tool_calls: Vec<ModelToolCall>,
}

pub struct ModelStreamChunk {
    pub text: String,
}

#[derive(Clone)]
pub struct ModelToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Clone)]
pub struct ModelToolCall {
    pub call_id: Option<String>,
    pub tool_id: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone)]
pub struct ModelMessage {
    pub role: ModelMessageRole,
    pub content: String,
}

#[derive(Clone)]
pub struct ModelInlineImage {
    pub name: Option<String>,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModelMessageRole {
    System,
    User,
    Assistant,
}

impl ModelMessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}
