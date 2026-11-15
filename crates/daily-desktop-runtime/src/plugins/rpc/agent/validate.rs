//! Bounds and shape checks for plugin agent inputs.
use super::*;

pub(super) fn validate_plugin_agent_task_input(
    input: &PluginAgentTaskStartInput,
) -> Result<(), String> {
    if input.system_prompt.trim().is_empty() {
        return Err("agent.task.start requires systemPrompt.".into());
    }
    if input.user_prompt.trim().is_empty() {
        return Err("agent.task.start requires userPrompt.".into());
    }
    validate_plugin_prompt_text(
        &input.system_prompt,
        MAX_PLUGIN_SYSTEM_PROMPT_BYTES,
        "agent.task.start systemPrompt",
    )?;
    validate_plugin_prompt_text(
        &input.user_prompt,
        MAX_PLUGIN_PROMPT_BYTES,
        "agent.task.start userPrompt",
    )?;
    if let Some(message) = input.display_message.as_deref() {
        validate_plugin_prompt_text(
            message,
            MAX_PLUGIN_PROMPT_BYTES,
            "agent.task.start displayMessage",
        )?;
    }
    validate_plugin_prompt_text(
        &input.final_instructions,
        MAX_PLUGIN_PROMPT_BYTES,
        "agent.task.start finalInstructions",
    )?;
    if input.context.len() > MAX_PLUGIN_CONTEXT_BLOCKS {
        return Err(format!(
            "agent.task.start supports at most {MAX_PLUGIN_CONTEXT_BLOCKS} context blocks."
        ));
    }
    for item in &input.context {
        validate_short_plugin_text(&item.title, "Context title")?;
        validate_plugin_prompt_text(
            &item.content,
            MAX_PLUGIN_PROMPT_BYTES,
            "agent.task.start context content",
        )?;
    }
    if input.definitions.len() > 64 || input.model_bindings.len() > 32 {
        return Err(
            "agent.task.start supports at most 64 definitions and 32 model bindings.".into(),
        );
    }
    if let Some(limits) = &input.run_limits {
        limits.validate().map_err(|e| e.to_string())?;
        if limits.max_concurrency > 4
            || limits.max_depth > 8
            || limits.max_model_requests > 1024
            || limits.max_tool_calls > 4096
        {
            return Err("Agent run limits exceed the desktop host bounds.".into());
        }
    }
    let mut definitions = std::collections::BTreeSet::new();
    for definition in &input.definitions {
        if !definitions.insert(definition.spec.id.clone()) {
            return Err("Agent definition ids must be unique.".into());
        }
        if definition.delegates.len() > 8 {
            return Err("An agent can delegate to at most eight agents.".into());
        }
        if definition
            .model_key
            .as_ref()
            .is_some_and(|key| !input.model_bindings.contains_key(key))
        {
            return Err("Agent definition references an unknown model binding.".into());
        }
        if definition
            .temperature
            .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
            || definition
                .max_output_tokens
                .is_some_and(|n| n == 0 || n > crate::limits::MAX_OUTPUT_TOKENS)
        {
            return Err("Invalid agent generation parameters.".into());
        }
        validate_agent_tool_spec(
            &definition.spec,
            &input.tools.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            &[],
        )?;
    }
    for definition in &input.definitions {
        if definition
            .delegates
            .iter()
            .any(|id| !definitions.contains(id))
        {
            return Err("Agent definition references an unknown delegate.".into());
        }
    }
    for key in input.model_bindings.keys() {
        validate_short_plugin_text(key, "Model binding key")?;
    }
    validate_dynamic_tools(&input.tools)?;
    let tool_ids = input
        .tools
        .iter()
        .map(|tool| tool.id.clone())
        .collect::<Vec<_>>();
    let mut agents = Vec::new();
    for spec in &input.sub_agents.agents {
        if agents
            .iter()
            .any(|agent: &AgentToolSpec| agent.id == spec.id)
        {
            return Err(format!("Agent id {} is duplicated.", spec.id));
        }
        validate_agent_tool_spec(spec, &tool_ids, &agents)?;
        agents.push(spec.clone());
    }
    Ok(())
}

pub(crate) fn parse_dynamic_tools(
    value: Option<&Value>,
) -> Result<Vec<DynamicAgentToolDefinition>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let tools = serde_json::from_value::<Vec<DynamicAgentToolDefinition>>(value.clone())
        .map_err(|error| format!("Failed to parse dynamicTools: {error}"))?;
    if tools.len() > 12 {
        return Err("dynamicTools cannot contain more than 12 tools.".into());
    }
    validate_dynamic_tools(&tools)?;
    Ok(tools)
}

pub(crate) fn validate_dynamic_tools(tools: &[DynamicAgentToolDefinition]) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    for tool in tools {
        validate_short_plugin_text(&tool.id, "Dynamic tool id")?;
        validate_short_plugin_text(&tool.name, "Dynamic tool name")?;
        if !ids.insert(tool.id.trim()) {
            return Err(format!("Dynamic tool id {} is duplicated.", tool.id));
        }
        if !tool.input_schema.is_object() {
            return Err(format!(
                "Dynamic tool {} inputSchema must be a JSON object.",
                tool.id
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_short_plugin_text(value: &str, label: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(format!(
            "{label} must be 1-256 bytes and contain no control characters."
        ));
    }
    Ok(())
}

pub(crate) fn validate_plugin_prompt_text(
    value: &str,
    limit: usize,
    label: &str,
) -> Result<(), String> {
    if value.len() > limit {
        return Err(format!("{label} exceeds the {limit} byte limit."));
    }
    Ok(())
}
