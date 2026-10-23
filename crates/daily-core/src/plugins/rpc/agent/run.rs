//! `agent.list` and `agent.run`: compact prompt/context runs on a configured agent, resumable after async tools.
use super::*;

pub(crate) fn plugin_agent_list(state: &AgentStore) -> Result<Value, String> {
    let app_state = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?;
    Ok(json!({
        "agents": app_state.agent_apps.iter().map(|agent| {
            json!({
                "id": agent.id,
                "name": agent.name,
                "providerKind": agent.provider_kind,
                "authConfigId": agent.auth_config_id,
                "model": agent.model,
            })
        }).collect::<Vec<_>>()
    }))
}
pub(crate) async fn plugin_agent_run(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    if plugin_agent_resume_run_id(&payload).is_some() {
        return plugin_agent_resume(state, plugin_id, payload).await;
    }

    let prompt = payload
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "agent.run requires payload.prompt.".to_string())?
        .to_string();
    let context = payload
        .get("context")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    validate_plugin_prompt_text(&prompt, MAX_PLUGIN_PROMPT_BYTES, "agent.run prompt")?;
    if let Some(context) = context.as_deref() {
        validate_plugin_prompt_text(context, MAX_PLUGIN_PROMPT_BYTES, "agent.run context")?;
    }
    let requested_agent_id = payload.get("agentAppId").and_then(Value::as_u64);
    let dynamic_tools =
        parse_dynamic_tools(payload.get("dynamicTools").or_else(|| payload.get("tools")))?;

    let (http, project_root, runtime_input, agent_name, agent_app_id) = {
        let app_state = state
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let agent_app = requested_agent_id
            .and_then(|agent_id| {
                app_state
                    .agent_apps
                    .iter()
                    .find(|agent| agent.id == agent_id)
            })
            .or_else(|| app_state.agent_apps.first())
            .cloned()
            .ok_or_else(|| "No agent app is available.".to_string())?;
        let manifests = app_state
            .plugins
            .prompt_manifests(&agent_app.mounted_plugin_ids);
        let model_config = effective_model_config(&app_state.model_config, Some(&agent_app));
        let runtime_input = TurnRuntimeInput {
            agent: effective_agent_config(&app_state.agent_config, Some(&agent_app), &manifests),
            model: model_config,
            workspace_root: app_state.workspace_root.clone(),
            dialogue: Vec::new(),
            memory: Vec::new(),
            request_images: Vec::new(),
            prior_tool_observations: Vec::new(),
            dynamic_tools: dynamic_tools.clone(),
            start_ply: 1,
        };
        (
            state.http.clone(),
            state.project_root.clone(),
            runtime_input,
            agent_app.name,
            agent_app.id,
        )
    };

    let full_prompt = if let Some(context) = context {
        format!("Context:\n{}\n\nPlugin request:\n{}", context, prompt)
    } else {
        prompt
    };
    match run_plugin_agent_flow(http, full_prompt.clone(), runtime_input).await? {
        PluginAgentFlowResult::Completed(text) => Ok(json!({
            "status": "completed",
            "agentName": agent_name,
            "text": text,
        })),
        PluginAgentFlowResult::Paused(pending_tool) => {
            let run_id = format!("plugin-agent-run-{}", unique_stamp());
            let pending = PendingPluginAgentRun {
                run_id: run_id.clone(),
                plugin_id: plugin_id.to_string(),
                agent_app_id,
                agent_name: agent_name.clone(),
                prompt: full_prompt,
                dynamic_tools,
                observations: pending_tool.observations.clone(),
                pending_tool,
                created_at: unix_timestamp(),
            };
            AGENT_RUNS.persist(&project_root, &pending.run_id, &pending)?;
            Ok(plugin_agent_pending_response(&pending))
        }
    }
}

pub(crate) async fn plugin_agent_resume(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let run_id = plugin_agent_resume_run_id(&payload)
        .ok_or_else(|| "agent.run resume requires runId.".to_string())?;
    let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(true);
    let result = payload
        .get("result")
        .or_else(|| payload.get("resumeResult"))
        .cloned()
        .unwrap_or(Value::Null);

    let project_root = state.project_root.clone();
    let pending = AGENT_RUNS.load::<PendingPluginAgentRun>(&project_root, &run_id)?;
    if pending.plugin_id != plugin_id {
        return Err("Pending agent run belongs to a different plugin.".into());
    }

    let (http, runtime_input, agent_name, prompt) = {
        let app_state = state
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let agent_app = app_state
            .agent_apps
            .iter()
            .find(|agent| agent.id == pending.agent_app_id)
            .cloned()
            .ok_or_else(|| "Pending agent run references an unavailable agent app.".to_string())?;
        let manifests = app_state
            .plugins
            .prompt_manifests(&agent_app.mounted_plugin_ids);
        let observation = AgentToolObservation::resumed(
            pending.pending_tool.tool_id.clone(),
            pending.pending_tool.tool_name.clone(),
            pending.pending_tool.rationale.clone(),
            pending.pending_tool.input.clone(),
            pending.pending_tool.pending_message.clone(),
            plugin_resume_observation(ok, &result)?,
            json!({
                "ok": ok,
                "result": result,
                "metadata": payload.get("metadata").cloned().unwrap_or(Value::Null),
            }),
        );
        let mut observations = pending.observations.clone();
        observations.push(observation);
        let runtime_input = TurnRuntimeInput {
            agent: effective_agent_config(&app_state.agent_config, Some(&agent_app), &manifests),
            model: effective_model_config(&app_state.model_config, Some(&agent_app)),
            workspace_root: app_state.workspace_root.clone(),
            dialogue: Vec::new(),
            memory: Vec::new(),
            request_images: Vec::new(),
            prior_tool_observations: observations,
            dynamic_tools: pending.dynamic_tools.clone(),
            start_ply: pending.pending_tool.ply + 1,
        };
        (
            state.http.clone(),
            runtime_input,
            agent_app.name,
            pending.prompt.clone(),
        )
    };

    match run_plugin_agent_flow(http, prompt, runtime_input).await? {
        PluginAgentFlowResult::Completed(text) => {
            AGENT_RUNS.remove(&project_root, &run_id)?;
            Ok(json!({
                "status": "completed",
                "agentName": agent_name,
                "text": text,
            }))
        }
        PluginAgentFlowResult::Paused(pending_tool) => {
            AGENT_RUNS.remove(&project_root, &run_id)?;
            let next_run = PendingPluginAgentRun {
                run_id: format!("plugin-agent-run-{}", unique_stamp()),
                plugin_id: pending.plugin_id,
                agent_app_id: pending.agent_app_id,
                agent_name,
                prompt: pending.prompt,
                dynamic_tools: pending.dynamic_tools,
                observations: pending_tool.observations.clone(),
                pending_tool,
                created_at: unix_timestamp(),
            };
            AGENT_RUNS.persist(&project_root, &next_run.run_id, &next_run)?;
            Ok(plugin_agent_pending_response(&next_run))
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingPluginAgentRun {
    pub(crate) run_id: String,
    pub(crate) plugin_id: String,
    pub(crate) agent_app_id: u64,
    pub(crate) agent_name: String,
    pub(crate) prompt: String,
    pub(crate) dynamic_tools: Vec<DynamicAgentToolDefinition>,
    pub(crate) pending_tool: PendingPluginAgentTool,
    #[serde(default)]
    pub(crate) observations: Vec<AgentToolObservation>,
    pub(crate) created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingPluginAgentTool {
    pub(crate) ply: usize,
    pub(crate) tool_id: String,
    pub(crate) tool_name: String,
    pub(crate) rationale: String,
    pub(crate) input: Value,
    pub(crate) pending_message: String,
    pub(crate) metadata: Value,
    #[serde(default)]
    pub(crate) observations: Vec<AgentToolObservation>,
}

pub(crate) enum PluginAgentFlowResult {
    Completed(String),
    Paused(PendingPluginAgentTool),
}

pub(crate) fn plugin_agent_resume_run_id(payload: &Value) -> Option<String> {
    payload
        .get("runId")
        .or_else(|| payload.get("resumeRunId"))
        .or_else(|| payload.pointer("/resume/runId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn plugin_agent_pending_response(pending: &PendingPluginAgentRun) -> Value {
    json!({
        "status": "pending",
        "agentName": pending.agent_name,
        "runId": pending.run_id,
        "toolId": pending.pending_tool.tool_id,
        "toolName": pending.pending_tool.tool_name,
        "pendingMessage": pending.pending_tool.pending_message,
        "toolInput": pending
            .pending_tool
            .metadata
            .get("input")
            .cloned()
            .unwrap_or_else(|| pending.pending_tool.input.clone()),
        "interaction": plugin_agent_pending_interaction(&pending.pending_tool),
        "metadata": pending.pending_tool.metadata,
    })
}

pub(crate) fn plugin_agent_pending_interaction(pending_tool: &PendingPluginAgentTool) -> Value {
    pending_tool
        .metadata
        .pointer("/metadata/interaction")
        .or_else(|| pending_tool.metadata.get("interaction"))
        .cloned()
        .unwrap_or(Value::Null)
}

pub(crate) fn plugin_resume_observation(ok: bool, result: &Value) -> Result<String, String> {
    let payload = serde_json::to_string_pretty(result)
        .map_err(|error| format!("Failed to serialize plugin agent resume result: {error}"))?;
    if ok {
        Ok(format!("The async tool completed. Result:\n{payload}"))
    } else {
        Ok(format!(
            "The async tool did not complete successfully. Feedback:\n{payload}"
        ))
    }
}

pub(crate) async fn run_plugin_agent_flow(
    http: Client,
    prompt: String,
    runtime_input: TurnRuntimeInput,
) -> Result<PluginAgentFlowResult, String> {
    let mut final_output = None;
    let mut terminal_error = None;
    let mut pending_tool = None;
    let result = drive_turn(&http, 0, 0, &prompt, runtime_input, |event| {
        match event {
            AgentEvent::FinalSelected { response, .. } => final_output = Some(response),
            AgentEvent::TurnFailed { error, .. } => terminal_error = Some(error),
            AgentEvent::AsyncToolPending {
                ply,
                tool_id,
                tool_name,
                rationale,
                input,
                pending_message,
                metadata,
                observations,
                ..
            } => {
                pending_tool = Some(PendingPluginAgentTool {
                    ply,
                    tool_id,
                    tool_name,
                    rationale,
                    input,
                    pending_message,
                    metadata,
                    observations,
                });
            }
            _ => {}
        }
        Ok(())
    })
    .await;

    let drive_result = result?;
    match drive_result {
        DriveTurnResult::Completed => final_output
            .map(PluginAgentFlowResult::Completed)
            .ok_or_else(|| "The plugin agent run completed without a final answer.".to_string()),
        DriveTurnResult::Paused => pending_tool
            .map(PluginAgentFlowResult::Paused)
            .ok_or_else(|| "The plugin agent run paused without pending tool state.".to_string()),
        DriveTurnResult::Failed => Err(terminal_error.unwrap_or_else(|| {
            "The plugin agent run failed before producing a final answer.".into()
        })),
    }
}
