use crate::plugins::{canonical_plugin_id, PluginManifestView};
use crate::{agent, model};

pub(crate) fn effective_agent_config(
    base: &agent::AgentConfig,
    agent_app: Option<&agent::AgentAppConfig>,
    manifests: &[PluginManifestView],
) -> agent::AgentConfig {
    let mut config = base.clone();
    if let Some(agent_app) = agent_app {
        if !agent_app.system_prompt.trim().is_empty() {
            config.system_prompt = agent_app.system_prompt.trim().to_string();
        }
        let plugin_prompt = plugin_protocol_prompt(agent_app, manifests);
        if !plugin_prompt.is_empty() {
            config.system_prompt = format!("{}\n\n{}", config.system_prompt, plugin_prompt);
        }
    }
    config
}

pub(crate) fn effective_model_config(
    base: &model::ModelConfig,
    agent_app: Option<&agent::AgentAppConfig>,
) -> model::ModelConfig {
    let mut config = base.clone();
    if let Some(agent_app) = agent_app {
        let requested_auth_config_id = agent_app.auth_config_id.trim();
        if !requested_auth_config_id.is_empty()
            && config
                .auth_configs
                .iter()
                .any(|auth_config| auth_config.id == requested_auth_config_id)
        {
            config.auth_config_id = requested_auth_config_id.to_string();
        }
    }

    let auth_config = config.selected_auth_config();
    config.apply_auth_config(&auth_config);
    if config.model.trim().is_empty() || !auth_config.models.iter().any(|m| m == &config.model) {
        config.model = auth_config.default_model.clone();
    } else {
        config.model = model::normalize_provider_model(auth_config.provider_kind, &config.model);
    }

    if let Some(agent_app) = agent_app {
        if !agent_app.model.trim().is_empty() {
            let requested =
                model::normalize_provider_model(auth_config.provider_kind, &agent_app.model);
            config.model = if auth_config.provider_kind
                == model::ModelProviderKind::AntigravityLogin
                && !auth_config.models.contains(&requested)
            {
                auth_config.default_model.clone()
            } else {
                requested
            };
        }
        config.temperature = agent_app.temperature;
    }
    config
}

pub(crate) fn plugin_protocol_prompt(
    agent_app: &agent::AgentAppConfig,
    manifests: &[PluginManifestView],
) -> String {
    let available = manifests
        .iter()
        .filter(|manifest| {
            manifest.enabled
                && !manifest.contributes.renderers.is_empty()
                && agent_app
                    .mounted_plugin_ids
                    .iter()
                    .any(|identifier| canonical_plugin_id(identifier) == manifest.identifier)
        })
        .collect::<Vec<_>>();
    if available.is_empty() {
        return String::new();
    }

    let catalog = available
        .iter()
        .flat_map(|manifest| {
            manifest.contributes.renderers.iter().map(|renderer| {
                format!(
                    "- pluginId: {}, renderer: {}. {} JSON example: {}",
                    manifest.identifier, renderer.id, renderer.description, renderer.payload_hint
                )
            })
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Mounted UI plugins:\n{catalog}\n\nIf a visual block would help the user, return it as a fenced code block whose info string is exactly `daily-plugin`. The body must be valid JSON with this shape: {{\"pluginId\":\"<plugin id>\",\"renderer\":\"<renderer id>\",\"payload\":{{...}}}}. Use only mounted plugin ids and renderer ids from the catalog."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_auth_binding_selects_provider_without_inheriting_codex_model() {
        let mut base = model::ModelConfig::seeded();
        let mut agy = model::ModelAuthConfig::antigravity_default();
        agy.models = vec!["gemini-pro".into(), "claude-sonnet".into()];
        agy.default_model = "gemini-pro".into();
        base.auth_configs.push(agy.clone());
        let mut state = crate::runtime::DailyRuntime::test_runtime()
            .store()
            .state
            .lock()
            .unwrap()
            .clone();
        let mut agent = state.agent_apps.remove(0);
        agent.auth_config_id = agy.id;
        agent.model = "claude-sonnet".into();
        let config = effective_model_config(&base, Some(&agent));
        assert!(config.provider_kind == model::ModelProviderKind::AntigravityLogin);
        assert_eq!(config.model, "claude-sonnet");
        assert!(config.codex_home.is_none());
        agent.model = "gpt-5.5".into();
        assert_eq!(
            effective_model_config(&base, Some(&agent)).model,
            "gemini-pro"
        );
        agent.auth_config_id = base.auth_config_id.clone();
        agent.model = "gpt-5.5".into();
        assert!(
            effective_model_config(&base, Some(&agent)).provider_kind
                == model::ModelProviderKind::CodexLogin
        );
    }
}
