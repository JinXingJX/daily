use crate::agent::AgentStore;
use crate::limits::{
    MAX_OUTPUT_TOKENS, MAX_PLUGIN_PROMPT_BYTES, MAX_PLUGIN_SYSTEM_PROMPT_BYTES, MAX_TURN_IMAGES,
};
use crate::model::{
    complete_model, normalize_provider_model, ModelCompletionRequest, ModelConfig,
    ModelInlineImage, ModelMessage, ModelProviderKind, ModelToolChoice,
};
use crate::plugins::rpc::agent::validate_plugin_prompt_text;
use crate::plugins::rpc::image::load_model_image;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A plugin's per-call choice of provider (auth config) and model; empty fields keep the default.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginModelSelection {
    #[serde(default)]
    pub(crate) auth_config_id: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

/// Lists the configured providers and their models, without credentials or endpoints.
pub(crate) fn plugin_model_list(state: &AgentStore) -> Result<Value, String> {
    let app_state = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?;
    Ok(model_choices(&app_state.model_config))
}

fn model_choices(config: &ModelConfig) -> Value {
    json!({
        "default": {
            "authConfigId": config.auth_config_id,
            "model": config.model,
        },
        "providers": config.auth_configs.iter().map(|auth_config| {
            json!({
                "id": auth_config.id,
                "name": auth_config.name,
                "providerKind": auth_config.provider_kind,
                "models": auth_config.models,
                "defaultModel": auth_config.default_model,
                "acceptsImages": accepts_images(auth_config.provider_kind),
            })
        }).collect::<Vec<_>>(),
    })
}

/// Whether `model.invoke` can send images to this kind of provider.
fn accepts_images(kind: ModelProviderKind) -> bool {
    matches!(
        kind,
        ModelProviderKind::OpenAiCompatible
            | ModelProviderKind::AnthropicMessages
            | ModelProviderKind::GeminiApi
            | ModelProviderKind::CodexLogin
            | ModelProviderKind::CodexApiKey
    )
}

/// Points `config` at the selected provider and model. Unknown providers and models are rejected
/// rather than silently replaced by the default, so a plugin never runs on a model it did not pick.
pub(crate) fn apply_model_selection(
    config: &mut ModelConfig,
    selection: &PluginModelSelection,
) -> Result<(), String> {
    let auth_config_id = non_empty(selection.auth_config_id.as_deref());
    let model = non_empty(selection.model.as_deref());
    if auth_config_id.is_none() && model.is_none() {
        return Ok(());
    }
    let auth_config = match auth_config_id {
        Some(id) => config
            .auth_configs
            .iter()
            .find(|auth_config| auth_config.id == id)
            .cloned()
            .map(|auth_config| auth_config.normalized())
            .ok_or_else(|| format!("Unknown model provider \"{id}\"."))?,
        None => config.selected_auth_config(),
    };
    let model = match model {
        Some(model) => normalize_provider_model(auth_config.provider_kind, model),
        None => auth_config.default_model.clone(),
    };
    if model.is_empty() || !auth_config.models.contains(&model) {
        return Err(format!(
            "Model \"{model}\" is not available for provider \"{}\".",
            auth_config.name
        ));
    }
    config.apply_auth_config(&auth_config);
    config.model = model;
    Ok(())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) async fn plugin_model_invoke(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let (http, mut model_config) = {
        let app_state = state
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        (state.http.clone(), app_state.model_config.clone())
    };
    let prompt = payload
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "model.invoke requires payload.prompt.".to_string())?;
    let system = payload
        .get("system")
        .and_then(Value::as_str)
        .unwrap_or("You are a helpful plugin assistant inside Daily.");
    validate_plugin_prompt_text(prompt, MAX_PLUGIN_PROMPT_BYTES, "model.invoke prompt")?;
    validate_plugin_prompt_text(
        system,
        MAX_PLUGIN_SYSTEM_PROMPT_BYTES,
        "model.invoke system prompt",
    )?;
    let temperature = payload
        .get("temperature")
        .and_then(Value::as_f64)
        .map(|value| value.clamp(0.0, 2.0) as f32)
        .unwrap_or(model_config.temperature);
    let max_output_tokens = payload
        .get("maxOutputTokens")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, MAX_OUTPUT_TOKENS as u64) as u32)
        .unwrap_or(model_config.max_output_tokens);

    if let Some(selection) = payload.get("model").filter(|value| !value.is_null()) {
        let selection = serde_json::from_value::<PluginModelSelection>(selection.clone())
            .map_err(|error| format!("model.invoke model is invalid: {error}"))?;
        apply_model_selection(&mut model_config, &selection)?;
    }
    if let Some(effort) = payload.get("reasoningEffort").and_then(Value::as_str) {
        apply_reasoning_effort(&mut model_config.model, model_config.provider_kind, effort)?;
    }

    let inline_images = plugin_model_images(state, plugin_id, &payload, &model_config)?;
    let mut user = ModelMessage::user(prompt);
    user.images = inline_images;
    let response = complete_model(
        &http,
        &model_config,
        &ModelCompletionRequest {
            messages: vec![ModelMessage::system(system), user],
            temperature,
            max_output_tokens: Some(max_output_tokens),
            tools: Vec::new(),
            tool_choice: ModelToolChoice::Auto,
            session_id: None,
            turn_id: None,
            request_name: Some("plugin.model.invoke".into()),
        },
        None,
    )
    .await?;
    Ok(json!({
        "text": response.raw_output,
        "stopReason": response.stop_reason,
        "usage": response.usage,
    }))
}

/// Loads `payload.images`, plugin-private paths; the bytes never cross the plugin bridge.
fn plugin_model_images(
    state: &AgentStore,
    plugin_id: &str,
    payload: &Value,
    config: &ModelConfig,
) -> Result<Vec<ModelInlineImage>, String> {
    let Some(paths) = payload.get("images").filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let paths = paths
        .as_array()
        .ok_or_else(|| "model.invoke images must be an array of paths.".to_string())?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if paths.len() > MAX_TURN_IMAGES {
        return Err(format!(
            "model.invoke accepts at most {MAX_TURN_IMAGES} images."
        ));
    }
    if !accepts_images(config.provider_kind) {
        return Err(format!(
            "Provider \"{}\" cannot read images through model.invoke.",
            config.provider_kind.label()
        ));
    }
    paths
        .iter()
        .map(|path| {
            let path = path
                .as_str()
                .ok_or_else(|| "model.invoke images must be paths.".to_string())?;
            load_model_image(state.project_root.as_path(), plugin_id, path)
        })
        .collect()
}

const REASONING_EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// Lets a plugin pick a lighter reasoning effort for a single call. Codex encodes the effort as a
/// model-name suffix (`<model>-<effort>`); other providers ignore the hint.
fn apply_reasoning_effort(
    model: &mut String,
    provider: ModelProviderKind,
    effort: &str,
) -> Result<(), String> {
    if !REASONING_EFFORTS.contains(&effort) {
        return Err(format!(
            "model.invoke reasoningEffort must be one of {}.",
            REASONING_EFFORTS.join(", ")
        ));
    }
    if !matches!(
        provider,
        ModelProviderKind::CodexLogin | ModelProviderKind::CodexApiKey
    ) || model.trim().is_empty()
    {
        return Ok(());
    }
    let base = REASONING_EFFORTS
        .iter()
        .find_map(|known| model.trim().strip_suffix(&format!("-{known}")))
        .unwrap_or(model.trim())
        .to_string();
    *model = format!("{base}-{effort}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelAuthConfig;

    fn two_provider_config() -> ModelConfig {
        let mut config = ModelConfig::seeded();
        config.api_key = "secret".into();
        let mut second = ModelAuthConfig::codex_default();
        second.id = "codex-work".into();
        second.name = "Work".into();
        second.codex_home = Some("/tmp/work-codex".into());
        second.models = vec!["gpt-5.4".into(), "gpt-5.4-mini".into()];
        second.default_model = "gpt-5.4".into();
        config.auth_configs.push(second);
        config
    }

    #[test]
    fn model_selection_switches_provider_and_model() {
        let mut config = two_provider_config();
        apply_model_selection(&mut config, &PluginModelSelection::default()).unwrap();
        assert_eq!(config.auth_config_id, "codex-default");
        assert_eq!(config.api_key, "secret");

        let selection = PluginModelSelection {
            auth_config_id: Some("codex-work".into()),
            model: Some(" gpt-5.4-mini ".into()),
        };
        apply_model_selection(&mut config, &selection).unwrap();
        assert_eq!(config.auth_config_id, "codex-work");
        assert_eq!(config.model, "gpt-5.4-mini");
        assert_eq!(config.codex_home.as_deref(), Some("/tmp/work-codex"));
        assert!(config.api_key.is_empty());

        let provider_only = PluginModelSelection {
            auth_config_id: Some("codex-work".into()),
            model: None,
        };
        let mut config = two_provider_config();
        apply_model_selection(&mut config, &provider_only).unwrap();
        assert_eq!(config.model, "gpt-5.4");

        let model_only = PluginModelSelection {
            auth_config_id: None,
            model: Some("gpt-5.4-low".into()),
        };
        let mut config = two_provider_config();
        apply_model_selection(&mut config, &model_only).unwrap();
        assert_eq!(config.auth_config_id, "codex-default");
        assert_eq!(config.model, "gpt-5.4-low");
    }

    #[test]
    fn model_list_exposes_choices_without_credentials() {
        let listed = model_choices(&two_provider_config());
        assert_eq!(listed["default"]["authConfigId"], "codex-default");
        assert_eq!(listed["providers"][1]["id"], "codex-work");
        assert_eq!(
            listed["providers"][1]["models"],
            json!(["gpt-5.4", "gpt-5.4-mini"])
        );
        let text = listed.to_string();
        assert!(
            !text.contains("secret") && !text.contains("baseUrl") && !text.contains("codexHome")
        );
    }

    #[test]
    fn model_selection_rejects_unknown_provider_or_model() {
        let mut config = two_provider_config();
        let unknown_provider = PluginModelSelection {
            auth_config_id: Some("missing".into()),
            model: None,
        };
        assert!(apply_model_selection(&mut config, &unknown_provider).is_err());
        let unknown_model = PluginModelSelection {
            auth_config_id: Some("codex-work".into()),
            model: Some("gpt-5.5".into()),
        };
        assert!(apply_model_selection(&mut config, &unknown_model).is_err());
        assert_eq!(config.auth_config_id, "codex-default");
    }

    #[test]
    fn reasoning_effort_replaces_codex_suffix_and_ignores_other_providers() {
        let mut model = "gpt-6.1-sol-xhigh".to_string();
        apply_reasoning_effort(&mut model, ModelProviderKind::CodexLogin, "low").unwrap();
        assert_eq!(model, "gpt-6.1-sol-low");

        let mut plain = "gpt-6.1-sol".to_string();
        apply_reasoning_effort(&mut plain, ModelProviderKind::CodexLogin, "medium").unwrap();
        assert_eq!(plain, "gpt-6.1-sol-medium");

        apply_reasoning_effort(&mut plain, ModelProviderKind::CodexApiKey, "low").unwrap();
        assert_eq!(plain, "gpt-6.1-sol-low");
        assert!(accepts_images(ModelProviderKind::CodexApiKey));

        let mut other = "claude-x".to_string();
        apply_reasoning_effort(&mut other, ModelProviderKind::AnthropicMessages, "low").unwrap();
        assert_eq!(other, "claude-x");

        assert!(apply_reasoning_effort(&mut model, ModelProviderKind::CodexLogin, "max").is_err());
    }
}
