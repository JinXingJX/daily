use crate::agent::turn::prompt::{effective_agent_config, effective_model_config};
use crate::agent::{
    apply_event_delta, drive_turn, AgentEvent, AgentHookConfig, AgentHookMode, AgentHookPhase,
    AgentStore, DialogueMessage, DriveTurnResult, MemoryEntry, TurnDelta, TurnPatch,
    TurnRuntimeInput,
};
use crate::model::ModelInlineImage;
use crate::plugins::{PluginManifestView, BUILTIN_AGENT_HOOKS_ID};
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::{build_turn_delta, emit_turn_delta};
use crate::{agent, model};
use reqwest::Client;
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentHooksPayload {
    pub(crate) title: String,
    pub(crate) phase: String,
    pub(crate) mode: String,
    pub(crate) append_previous_output: bool,
    pub(crate) agents: Vec<HookAgentCard>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HookAgentCard {
    pub(crate) agent_id: u64,
    pub(crate) agent_name: String,
    pub(crate) status: String,
    pub(crate) model: String,
    pub(crate) output: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) struct HookExecutionReport {
    pub(crate) payload: AgentHooksPayload,
    pub(crate) context_blocks: Vec<String>,
}

pub(crate) fn delay_turn_completion_for_hooks(
    state: &mut agent::AppState,
    session_id: u64,
    turn_id: u64,
    strategy: String,
) -> TurnDelta {
    let next_strategy = format!("{strategy} Running on_finish hooks before finalizing.");
    if let Some(turn) = state.turn_mut(session_id, turn_id) {
        turn.status = agent::TurnStatus::Running;
        turn.strategy = next_strategy.clone();
    }
    if let Some(session) = state.session_mut(session_id) {
        session.running_turn_id = Some(turn_id);
    }
    state.runtime_status = "Running on_finish hooks".into();
    build_turn_delta(
        state,
        session_id,
        turn_id,
        TurnPatch {
            status: Some(agent::TurnStatus::Running),
            strategy: Some(next_strategy),
            ..TurnPatch::default()
        },
        Vec::new(),
    )
}

pub(crate) fn hook_enabled(config: &AgentHookConfig) -> bool {
    config.mode.is_some() && !config.agent_app_ids.is_empty()
}

pub(crate) fn set_hook_runtime_state(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
    turn_id: u64,
    strategy: &str,
) -> Result<(), String> {
    let delta = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let strategy_text = strategy.to_string();
        if let Some(turn) = state.turn_mut(session_id, turn_id) {
            turn.status = agent::TurnStatus::Running;
            turn.strategy = strategy_text.clone();
        }
        if let Some(session) = state.session_mut(session_id) {
            session.running_turn_id = Some(turn_id);
        }
        state.runtime_status = strategy_text.clone();
        state.persist_session(&store.project_root, session_id)?;
        build_turn_delta(
            &state,
            session_id,
            turn_id,
            TurnPatch {
                status: Some(agent::TurnStatus::Running),
                strategy: Some(strategy_text),
                ..TurnPatch::default()
            },
            Vec::new(),
        )
    };
    emit_turn_delta(app, delta)
}

pub(crate) fn append_hook_report(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
    turn_id: u64,
    report: &HookExecutionReport,
) -> Result<(), String> {
    let block = build_hook_plugin_block(&report.payload)?;
    let delta = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let message_id = state
            .push_message(
                session_id,
                agent::MessageRole::Assistant,
                &block,
                Some(turn_id),
            )
            .unwrap_or(0);
        state.push_memory(
            session_id,
            turn_id,
            agent::MemoryKind::AssistantMessage,
            format!("{} hook results", report.payload.phase),
            &block,
        );
        let mut steps = None;
        if let Some(turn) = state.turn_mut(session_id, turn_id) {
            turn.steps.push(agent::DecisionStep {
                ply: turn.steps.len() + 1,
                selected_action: format!("{} hooks", report.payload.phase),
                selected_kind: "hook".into(),
                rationale: format!(
                    "{} hook agents completed in {} mode.",
                    report.payload.phase, report.payload.mode
                ),
                model_output: String::new(),
                tool_name: None,
                tool_input: None,
                observation: Some(block.clone()),
            });
            steps = Some(turn.steps.clone());
        }
        state.persist_session(&store.project_root, session_id)?;
        build_turn_delta(
            &state,
            session_id,
            turn_id,
            TurnPatch {
                steps,
                ..TurnPatch::default()
            },
            vec![DialogueMessage {
                id: message_id,
                turn_id: Some(turn_id),
                role: agent::MessageRole::Assistant,
                content: block,
            }],
        )
    };
    emit_turn_delta(app, delta)
}

pub(crate) fn append_hook_error_message(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
    turn_id: u64,
    message: String,
) -> Result<(), String> {
    let delta = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let message_id = state
            .push_message(
                session_id,
                agent::MessageRole::Assistant,
                &message,
                Some(turn_id),
            )
            .unwrap_or(0);
        state.push_memory(
            session_id,
            turn_id,
            agent::MemoryKind::Error,
            "Hook execution failed",
            &message,
        );
        let mut steps = None;
        if let Some(turn) = state.turn_mut(session_id, turn_id) {
            turn.steps.push(agent::DecisionStep {
                ply: turn.steps.len() + 1,
                selected_action: "Hook failure".into(),
                selected_kind: "hook_error".into(),
                rationale: "A configured hook agent failed.".into(),
                model_output: String::new(),
                tool_name: None,
                tool_input: None,
                observation: Some(message.clone()),
            });
            steps = Some(turn.steps.clone());
        }
        state.persist_session(&store.project_root, session_id)?;
        build_turn_delta(
            &state,
            session_id,
            turn_id,
            TurnPatch {
                steps,
                ..TurnPatch::default()
            },
            vec![DialogueMessage {
                id: message_id,
                turn_id: Some(turn_id),
                role: agent::MessageRole::Assistant,
                content: message,
            }],
        )
    };
    emit_turn_delta(app, delta)
}

pub(crate) fn finalize_turn_after_hooks(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
    turn_id: u64,
) -> Result<(), String> {
    let delta = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let mut strategy = None;
        if let Some(turn) = state.turn_mut(session_id, turn_id) {
            turn.status = agent::TurnStatus::Completed;
            if !turn.strategy.contains("hook") {
                turn.strategy = "Completed with lifecycle hooks.".into();
            }
            strategy = Some(turn.strategy.clone());
        }
        state.complete_turn(session_id);
        state.persist_session(&store.project_root, session_id)?;
        build_turn_delta(
            &state,
            session_id,
            turn_id,
            TurnPatch {
                status: Some(agent::TurnStatus::Completed),
                strategy,
                ..TurnPatch::default()
            },
            Vec::new(),
        )
    };
    emit_turn_delta(app, delta)
}

pub(crate) fn fail_turn_with_message(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
    turn_id: u64,
    error: String,
) -> Result<(), String> {
    let delta = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let delta = apply_event_delta(
            &mut state,
            AgentEvent::TurnFailed {
                session_id,
                turn_id,
                error,
                raw_output: None,
            },
        );
        state.persist_session(&store.project_root, session_id)?;
        delta
    };
    if let Some(delta) = delta {
        emit_turn_delta(app, delta)?;
    }
    Ok(())
}

/// Immutable workspace snapshot that hook agents run against. Captured under the
/// state lock when a turn starts so hooks never observe concurrent mutations.
#[derive(Clone)]
pub(crate) struct HookContext {
    pub(crate) dialogue: Vec<DialogueMessage>,
    pub(crate) memory: Vec<MemoryEntry>,
    pub(crate) workspace_root: Option<std::path::PathBuf>,
    pub(crate) agent_apps: Vec<agent::AgentAppConfig>,
    pub(crate) manifests: Vec<PluginManifestView>,
    pub(crate) base_agent_config: agent::AgentConfig,
    pub(crate) base_model_config: model::ModelConfig,
}

pub(crate) async fn execute_hook_phase(
    http: &Client,
    prompt: &str,
    images: &[ModelInlineImage],
    context: &HookContext,
    config: &AgentHookConfig,
    phase: AgentHookPhase,
) -> Result<HookExecutionReport, String> {
    let HookContext {
        dialogue,
        memory,
        workspace_root,
        agent_apps,
        manifests,
        base_agent_config,
        base_model_config,
    } = context;
    let Some(mode) = config.mode else {
        return Err(format!("{} hook mode is missing.", phase.label()));
    };

    let selected_agents = config
        .agent_app_ids
        .iter()
        .map(|agent_id| {
            agent_apps
                .iter()
                .find(|agent| agent.id == *agent_id)
                .cloned()
                .ok_or_else(|| format!("Hook agent {} was not found.", agent_id))
        })
        .collect::<Result<Vec<_>, String>>()?;

    if selected_agents.is_empty() {
        return Err(format!("{} hook has no agents selected.", phase.label()));
    }

    let mut cards = Vec::new();
    match mode {
        AgentHookMode::Parallel => {
            let mut handles = Vec::new();
            for agent_app in selected_agents {
                handles.push(tokio::spawn(execute_hook_agent(
                    http.clone(),
                    prompt.to_string(),
                    images.to_vec(),
                    dialogue.to_vec(),
                    memory.to_vec(),
                    workspace_root.clone(),
                    agent_app,
                    manifests.to_vec(),
                    base_agent_config.clone(),
                    base_model_config.clone(),
                )));
            }
            for handle in handles {
                cards.push(
                    handle
                        .await
                        .map_err(|error| format!("Failed to join hook task: {error}"))?,
                );
            }
        }
        AgentHookMode::Serial => {
            let mut serial_prompt = prompt.to_string();
            for agent_app in selected_agents {
                let card = execute_hook_agent(
                    http.clone(),
                    serial_prompt.clone(),
                    images.to_vec(),
                    dialogue.to_vec(),
                    memory.to_vec(),
                    workspace_root.clone(),
                    agent_app,
                    manifests.to_vec(),
                    base_agent_config.clone(),
                    base_model_config.clone(),
                )
                .await;
                if config.append_previous_output {
                    if let Some(output) = &card.output {
                        serial_prompt = format!(
                            "{}\n\nPrevious hook output from {}:\n{}",
                            serial_prompt.trim(),
                            card.agent_name,
                            output.trim()
                        );
                    }
                }
                cards.push(card);
            }
        }
    }

    let context_blocks = cards
        .iter()
        .filter_map(|card| {
            card.output
                .as_ref()
                .map(|output| format!("- {} ({})\n{}", card.agent_name, card.model, output.trim()))
        })
        .collect::<Vec<_>>();

    Ok(HookExecutionReport {
        payload: AgentHooksPayload {
            title: match phase {
                AgentHookPhase::OnStart => "On Start Hooks".into(),
                AgentHookPhase::OnFinish => "On Finish Hooks".into(),
            },
            phase: phase.label().into(),
            mode: match mode {
                AgentHookMode::Parallel => "parallel".into(),
                AgentHookMode::Serial => "serial".into(),
            },
            append_previous_output: config.append_previous_output,
            agents: cards,
        },
        context_blocks,
    })
}

#[allow(clippy::too_many_arguments)] // Mirrors the immutable hook snapshot passed to each parallel agent.
pub(crate) async fn execute_hook_agent(
    http: Client,
    prompt: String,
    images: Vec<ModelInlineImage>,
    dialogue: Vec<DialogueMessage>,
    memory: Vec<MemoryEntry>,
    workspace_root: Option<std::path::PathBuf>,
    agent_app: agent::AgentAppConfig,
    manifests: Vec<PluginManifestView>,
    base_agent_config: agent::AgentConfig,
    base_model_config: model::ModelConfig,
) -> HookAgentCard {
    let model_config = effective_model_config(&base_model_config, Some(&agent_app));
    let runtime_input = TurnRuntimeInput {
        agent: effective_agent_config(&base_agent_config, Some(&agent_app), &manifests),
        model: model_config.clone(),
        workspace_root,
        dialogue,
        memory,
        request_images: images,
        prior_tool_observations: Vec::new(),
        dynamic_tools: Vec::new(),
        start_ply: 1,
    };
    match run_agent_flow(http, prompt, runtime_input).await {
        Ok(output) => HookAgentCard {
            agent_id: agent_app.id,
            agent_name: agent_app.name,
            status: "completed".into(),
            model: model_config.model,
            output: Some(output),
            error: None,
        },
        Err(error) => HookAgentCard {
            agent_id: agent_app.id,
            agent_name: agent_app.name,
            status: "failed".into(),
            model: model_config.model,
            output: None,
            error: Some(error),
        },
    }
}

pub(crate) async fn run_agent_flow(
    http: Client,
    prompt: String,
    runtime_input: TurnRuntimeInput,
) -> Result<String, String> {
    let mut final_output = None;
    let mut terminal_error = None;
    let result = drive_turn(&http, 0, 0, &prompt, runtime_input, |event| {
        match event {
            AgentEvent::FinalSelected { response, .. } => final_output = Some(response),
            AgentEvent::TurnFailed { error, .. } => terminal_error = Some(error),
            _ => {}
        }
        Ok(())
    })
    .await;

    let drive_result = result?;
    if matches!(drive_result, DriveTurnResult::Paused) {
        return Err(
            "The agent run paused for an async tool, but this caller has no resume handle.".into(),
        );
    }

    match (drive_result, final_output) {
        (DriveTurnResult::Completed, Some(output)) => Ok(output),
        (DriveTurnResult::Failed, _) => Err(terminal_error
            .unwrap_or_else(|| "The agent run failed before producing a final answer.".into())),
        _ => Err(terminal_error
            .unwrap_or_else(|| "The hook agent did not produce a final answer.".into())),
    }
}

pub(crate) fn build_hook_plugin_block(payload: &AgentHooksPayload) -> Result<String, String> {
    let envelope = serde_json::json!({
        "pluginId": BUILTIN_AGENT_HOOKS_ID,
        "renderer": "agent_hooks",
        "payload": payload,
    });
    let json = serde_json::to_string_pretty(&envelope)
        .map_err(|error| format!("Failed to serialize hook payload: {error}"))?;
    Ok(format!("```daily-plugin\n{json}\n```"))
}
