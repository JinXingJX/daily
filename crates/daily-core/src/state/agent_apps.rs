use crate::agent::types::{
    default_auth_config_id, AgentAppConfig, AgentHookConfig, AgentHookMode, CreateAgentAppInput,
    UpdateAgentAppInput,
};
use crate::model::{normalize_provider_model, ModelConfig};
use crate::state::{
    normalize_name, validate_name, AppState, MAX_AGENT_APPS, MAX_AGENT_REFERENCES,
    MAX_SYSTEM_PROMPT_BYTES,
};
use crate::util::unix_timestamp;

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_agent_input(
    name: &str,
    system_prompt: &str,
    model: &str,
    mounted_plugin_ids: &[String],
    context_roots: &[String],
    on_start_hook: &AgentHookConfig,
    on_finish_hook: &AgentHookConfig,
) -> Result<(), String> {
    validate_name(name, "Agent name")?;
    validate_name(model, "Model id")?;
    if system_prompt.len() > MAX_SYSTEM_PROMPT_BYTES {
        return Err(format!(
            "Agent system prompt exceeds the {MAX_SYSTEM_PROMPT_BYTES} byte limit."
        ));
    }
    if mounted_plugin_ids.len() > MAX_AGENT_REFERENCES
        || context_roots.len() > MAX_AGENT_REFERENCES
        || on_start_hook.agent_app_ids.len() > MAX_AGENT_REFERENCES
        || on_finish_hook.agent_app_ids.len() > MAX_AGENT_REFERENCES
    {
        return Err(format!(
            "Agent plugin, context, and hook lists support at most {MAX_AGENT_REFERENCES} items each."
        ));
    }
    Ok(())
}

pub(crate) fn normalize_auth_config_id(input: Option<&str>, model_config: &ModelConfig) -> String {
    if let Some(candidate) = input.map(str::trim).filter(|value| !value.is_empty()) {
        if model_config
            .auth_configs
            .iter()
            .any(|auth_config| auth_config.id == candidate)
        {
            return candidate.to_string();
        }
    }
    if model_config
        .auth_configs
        .iter()
        .any(|auth_config| auth_config.id == model_config.auth_config_id)
    {
        return model_config.auth_config_id.clone();
    }
    model_config
        .auth_configs
        .first()
        .map(|auth_config| auth_config.id.clone())
        .unwrap_or_else(default_auth_config_id)
}

pub(crate) fn auth_config_for_id(
    model_config: &ModelConfig,
    auth_config_id: &str,
) -> crate::model::ModelAuthConfig {
    model_config
        .auth_configs
        .iter()
        .find(|auth_config| auth_config.id == auth_config_id)
        .cloned()
        .unwrap_or_else(|| model_config.selected_auth_config())
        .normalized()
}

pub(crate) fn normalize_context_roots(input: Vec<String>) -> Vec<String> {
    let mut roots = input
        .into_iter()
        .map(|value| value.trim().replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

pub(crate) fn normalize_hook_config(input: AgentHookConfig, owner_id: u64) -> AgentHookConfig {
    let mut seen = std::collections::BTreeSet::new();
    let agent_app_ids = input
        .agent_app_ids
        .into_iter()
        .filter(|agent_id| *agent_id != owner_id)
        .filter(|agent_id| seen.insert(*agent_id))
        .collect::<Vec<_>>();

    AgentHookConfig {
        mode: if agent_app_ids.is_empty() {
            None
        } else {
            Some(input.mode.unwrap_or(AgentHookMode::Serial))
        },
        append_previous_output: input.append_previous_output,
        agent_app_ids,
    }
}

pub(crate) fn validate_hook_graph(agent_apps: &[AgentAppConfig]) -> Result<(), String> {
    let agents_by_id = agent_apps
        .iter()
        .map(|agent| (agent.id, agent))
        .collect::<std::collections::BTreeMap<_, _>>();
    let adjacency = agent_apps
        .iter()
        .map(|agent| {
            let targets = agent
                .on_start_hook
                .agent_app_ids
                .iter()
                .chain(agent.on_finish_hook.agent_app_ids.iter())
                .copied()
                .collect::<Vec<_>>();
            (agent.id, targets)
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    for (agent_id, targets) in &adjacency {
        for target_id in targets {
            if !agents_by_id.contains_key(target_id) {
                let owner_name = agents_by_id
                    .get(agent_id)
                    .map(|agent| agent.name.as_str())
                    .unwrap_or("Unknown Agent");
                return Err(format!(
                    "Agent hook target {} referenced by \"{}\" was not found.",
                    target_id, owner_name
                ));
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum VisitState {
        Visiting,
        Visited,
    }

    fn walk(
        node_id: u64,
        adjacency: &std::collections::BTreeMap<u64, Vec<u64>>,
        agents_by_id: &std::collections::BTreeMap<u64, &AgentAppConfig>,
        states: &mut std::collections::BTreeMap<u64, VisitState>,
        stack: &mut Vec<u64>,
    ) -> Result<(), String> {
        if let Some(state) = states.get(&node_id) {
            if *state == VisitState::Visited {
                return Ok(());
            }
            if *state == VisitState::Visiting {
                if let Some(index) = stack.iter().position(|stack_id| *stack_id == node_id) {
                    let cycle = stack[index..]
                        .iter()
                        .chain(std::iter::once(&node_id))
                        .map(|agent_id| {
                            agents_by_id
                                .get(agent_id)
                                .map(|agent| agent.name.clone())
                                .unwrap_or_else(|| format!("Agent {}", agent_id))
                        })
                        .collect::<Vec<_>>()
                        .join(" -> ");
                    return Err(format!(
                        "Agent hook configuration cannot contain circular dependencies: {}.",
                        cycle
                    ));
                }
                return Err(
                    "Agent hook configuration cannot contain circular dependencies.".into(),
                );
            }
        }

        states.insert(node_id, VisitState::Visiting);
        stack.push(node_id);
        if let Some(targets) = adjacency.get(&node_id) {
            for target_id in targets {
                walk(*target_id, adjacency, agents_by_id, states, stack)?;
            }
        }
        stack.pop();
        states.insert(node_id, VisitState::Visited);
        Ok(())
    }

    let mut states = std::collections::BTreeMap::new();
    let mut stack = Vec::new();
    for agent in agent_apps {
        walk(agent.id, &adjacency, &agents_by_id, &mut states, &mut stack)?;
    }

    Ok(())
}

impl AppState {
    pub fn create_agent_app(
        &mut self,
        input: CreateAgentAppInput,
    ) -> Result<AgentAppConfig, String> {
        if self.agent_apps.len() >= MAX_AGENT_APPS {
            return Err(format!(
                "A workspace can contain at most {MAX_AGENT_APPS} agents."
            ));
        }
        validate_agent_input(
            &input.name,
            &input.system_prompt,
            &input.model,
            &input.mounted_plugin_ids,
            &input.context_roots,
            &input.on_start_hook,
            &input.on_finish_hook,
        )?;
        let agent_id = self.next_agent_app_id;
        let auth_config_id =
            normalize_auth_config_id(input.auth_config_id.as_deref(), &self.model_config);
        let auth_config = auth_config_for_id(&self.model_config, &auth_config_id);
        let agent = AgentAppConfig {
            id: agent_id,
            name: normalize_name(&input.name, "Agent"),
            system_prompt: input.system_prompt.trim().to_string(),
            provider_kind: auth_config.provider_kind,
            auth_config_id,
            model: normalize_provider_model(
                auth_config.provider_kind,
                &normalize_name(&input.model, auth_config.default_model.as_str()),
            ),
            temperature: input.temperature.clamp(0.0, 2.0),
            mounted_plugin_ids: self
                .plugins
                .canonicalize_agent_mounts(input.mounted_plugin_ids),
            context_roots: normalize_context_roots(input.context_roots),
            on_start_hook: normalize_hook_config(input.on_start_hook, agent_id),
            on_finish_hook: normalize_hook_config(input.on_finish_hook, agent_id),
        };
        let mut next_agent_apps = self.agent_apps.clone();
        next_agent_apps.push(agent.clone());
        validate_hook_graph(&next_agent_apps)?;

        self.next_agent_app_id += 1;
        self.agent_apps = next_agent_apps;
        Ok(agent)
    }

    pub fn update_agent_app(&mut self, input: UpdateAgentAppInput) -> Result<(), String> {
        validate_agent_input(
            &input.name,
            &input.system_prompt,
            &input.model,
            &input.mounted_plugin_ids,
            &input.context_roots,
            &input.on_start_hook,
            &input.on_finish_hook,
        )?;
        let Some(agent_index) = self
            .agent_apps
            .iter()
            .position(|agent| agent.id == input.id)
        else {
            return Err("Agent app was not found.".into());
        };
        let mut next_agent_apps = self.agent_apps.clone();
        let agent = &mut next_agent_apps[agent_index];
        let auth_config_id =
            normalize_auth_config_id(input.auth_config_id.as_deref(), &self.model_config);
        let auth_config = auth_config_for_id(&self.model_config, &auth_config_id);
        agent.name = normalize_name(&input.name, "Agent");
        agent.system_prompt = input.system_prompt.trim().to_string();
        agent.provider_kind = auth_config.provider_kind;
        agent.auth_config_id = auth_config_id;
        agent.model = normalize_provider_model(
            auth_config.provider_kind,
            &normalize_name(&input.model, auth_config.default_model.as_str()),
        );
        agent.temperature = input.temperature.clamp(0.0, 2.0);
        agent.mounted_plugin_ids = self
            .plugins
            .canonicalize_agent_mounts(input.mounted_plugin_ids);
        agent.context_roots = normalize_context_roots(input.context_roots);
        agent.on_start_hook = normalize_hook_config(input.on_start_hook, input.id);
        agent.on_finish_hook = normalize_hook_config(input.on_finish_hook, input.id);
        validate_hook_graph(&next_agent_apps)?;
        self.agent_apps = next_agent_apps;
        Ok(())
    }

    pub fn delete_agent_app(&mut self, id: u64) -> Result<(), String> {
        if self.agent_apps.len() <= 1 {
            return Err("At least one agent app must remain available.".into());
        }
        let removed = self.agent_apps.iter().any(|agent| agent.id == id);
        if !removed {
            return Err("Agent app was not found.".into());
        }
        self.agent_apps.retain(|agent| agent.id != id);
        let fallback_id = self.agent_apps.first().map(|agent| agent.id);
        for session in &mut self.sessions {
            if session.agent_app_id == Some(id) {
                std::sync::Arc::make_mut(session).agent_app_id = fallback_id;
            }
        }
        for agent in &mut self.agent_apps {
            agent.on_start_hook = normalize_hook_config(agent.on_start_hook.clone(), agent.id);
            agent
                .on_start_hook
                .agent_app_ids
                .retain(|agent_id| *agent_id != id);
            if agent.on_start_hook.agent_app_ids.is_empty() {
                agent.on_start_hook.mode = None;
            }

            agent.on_finish_hook = normalize_hook_config(agent.on_finish_hook.clone(), agent.id);
            agent
                .on_finish_hook
                .agent_app_ids
                .retain(|agent_id| *agent_id != id);
            if agent.on_finish_hook.agent_app_ids.is_empty() {
                agent.on_finish_hook.mode = None;
            }
        }
        Ok(())
    }

    pub fn bind_session_agent(
        &mut self,
        session_id: u64,
        agent_app_id: Option<u64>,
    ) -> Result<(), String> {
        if let Some(agent_app_id) = agent_app_id {
            if !self.agent_apps.iter().any(|agent| agent.id == agent_app_id) {
                return Err("Agent app was not found.".into());
            }
        }
        let Some(session) = self.session_mut(session_id) else {
            return Err("Session was not found.".into());
        };
        session.agent_app_id = agent_app_id;
        session.updated_at = unix_timestamp();
        Ok(())
    }

    #[allow(dead_code)]
    pub fn active_agent_for_session(&self, session_id: u64) -> Option<&AgentAppConfig> {
        let agent_id = self.session(session_id)?.agent_app_id?;
        self.agent_apps.iter().find(|agent| agent.id == agent_id)
    }

    pub fn find_agent_to_run(&self, session_id: u64, prompt: &str) -> Option<AgentAppConfig> {
        let session = self.session(session_id)?;
        if let Some(agent) = self.find_agent_mentioned_in_prompt(prompt) {
            return Some(agent.clone());
        }

        session
            .turns
            .iter()
            .rev()
            .find_map(|turn| turn.agent_app_id)
            .or(session.last_agent_app_id)
            .or(session.agent_app_id)
            .and_then(|agent_id| self.agent_apps.iter().find(|agent| agent.id == agent_id))
            .cloned()
            .or_else(|| self.agent_apps.first().cloned())
    }

    /// The agent named by an explicit `@Name` mention; the longest matching name wins so
    /// `@Research Agent` is not taken for `@Research`. A bare name in prose is not a mention.
    pub(crate) fn find_agent_mentioned_in_prompt(&self, prompt: &str) -> Option<&AgentAppConfig> {
        let prompt = prompt.to_lowercase();
        self.agent_apps
            .iter()
            .filter(|agent| {
                let name = agent.name.trim().to_lowercase();
                !name.is_empty() && mentions_agent(&prompt, &name)
            })
            .max_by_key(|agent| agent.name.trim().chars().count())
    }
}

/// True when `prompt` contains `@name` as a whole mention: not glued to a preceding word
/// (so emails do not match) and, for names ending in a Latin letter or digit, not followed by
/// one (so `@Writer` does not match `@Writers`). Both inputs are already lowercased.
fn mentions_agent(prompt: &str, name: &str) -> bool {
    let needle = format!("@{name}");
    let needs_end_boundary = name
        .chars()
        .next_back()
        .is_some_and(|last| last.is_ascii_alphanumeric());
    prompt.match_indices(&needle).any(|(index, _)| {
        let starts_word = prompt[..index]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric());
        let ends_word = !needs_end_boundary
            || prompt[index + needle.len()..]
                .chars()
                .next()
                .is_none_or(|after| !(after.is_ascii_alphanumeric() || after == '_'));
        starts_word && ends_word
    })
}
