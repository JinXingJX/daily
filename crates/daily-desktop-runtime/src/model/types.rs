pub use daily_agent_providers::types::*;
use serde::{Deserialize, Serialize};

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
        ModelProviderKind::CodexApiKey => "codex-api",
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
    /// A configuration for one provider endpoint and model, outside of Daily's settings files.
    pub fn for_provider(
        provider_kind: ModelProviderKind,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            auth_config_id: String::new(),
            auth_configs: Vec::new(),
            provider_kind,
            base_url: base_url.into(),
            model: model.into(),
            api_key: api_key.into(),
            codex_home: None,
            temperature: 0.2,
            max_output_tokens: 32_000,
        }
    }

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
            max_output_tokens: 32_000,
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

impl ModelConfig {
    pub(crate) fn provider_config(&self) -> daily_agent_providers::ModelConfig {
        daily_agent_providers::ModelConfig {
            provider_kind: self.provider_kind,
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            codex_home: self.codex_home.clone(),
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
        }
    }
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
