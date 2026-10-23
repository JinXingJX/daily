use super::config::AgentConfig;
use super::events::AgentEvent;
use super::sdk::{
    agent_tool_from_dynamic, run_agent_task_until_pause, AgentContextBlock, AgentTask,
    AgentTaskEvent, AgentTaskRunResult, AgentTaskTraceStep, AgentTool, AgentToolCall,
    AgentToolExecution, AgentToolObservation, DynamicAgentToolDefinition,
};
use super::tools::{recent_working_memory, WorkingMemoryItem};
use super::types::{DialogueMessage, MemoryEntry, TurnCheckpoint};
use super::workspace_tools::workspace_tools;
use crate::model::{ModelConfig, ModelInlineImage, ModelMessage, ModelMessageRole};
use reqwest::Client;
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub struct TurnRuntimeInput {
    pub agent: AgentConfig,
    pub model: ModelConfig,
    /// Binds the read-only workspace tools; `None` offers no workspace tools.
    pub workspace_root: Option<std::path::PathBuf>,
    pub dialogue: Vec<DialogueMessage>,
    pub memory: Vec<MemoryEntry>,
    pub request_images: Vec<ModelInlineImage>,
    pub prior_tool_observations: Vec<AgentToolObservation>,
    pub dynamic_tools: Vec<DynamicAgentToolDefinition>,
    /// The first ply to run; later than 1 when continuing a turn.
    pub start_ply: usize,
}

pub enum DriveTurnResult {
    Completed,
    Paused,
    Failed,
}

pub async fn drive_turn<F>(
    http: &Client,
    session_id: u64,
    turn_id: u64,
    prompt: &str,
    input: TurnRuntimeInput,
    mut emit: F,
) -> Result<DriveTurnResult, String>
where
    F: FnMut(AgentEvent) -> Result<(), String> + Send,
{
    emit(AgentEvent::TurnStarted {
        session_id,
        turn_id,
    })?;

    let mut working_memory =
        recent_working_memory(&input.memory, input.agent.memory_window as usize);
    working_memory.push(WorkingMemoryItem::new(
        "user_message",
        "Current user request",
        prompt,
    ));
    let dialogue = recent_dialogue(&input.dialogue, input.agent.dialogue_window as usize);
    let memory_state = Arc::new(Mutex::new(working_memory.clone()));
    let mut tools = chat_sdk_tools(input.workspace_root.as_deref(), memory_state);
    tools.extend(
        input
            .dynamic_tools
            .clone()
            .into_iter()
            .map(agent_tool_from_dynamic),
    );
    let task = AgentTask {
        name: "chat.turn".into(),
        session_id: Some(session_id),
        turn_id: Some(turn_id),
        model: input.model.clone(),
        system_prompt: input.agent.system_prompt.clone(),
        user_prompt: format!(
            "Current user request:\n{}\n\nIf you need a tool, call one with type=\"tool\". Otherwise return type=\"final\".",
            prompt.trim()
        ),
        final_instructions: "For a chat turn, final output must be a plain string ready to show to the user.".into(),
        context: vec![AgentContextBlock::new(
            "Working memory",
            format_memory_for_prompt(&working_memory),
        )],
        conversation: dialogue_to_model_messages(&dialogue),
        prior_tool_observations: input.prior_tool_observations.clone(),
        tools,
        max_iterations: input.agent.max_iterations.clamp(1, 6) as usize,
        start_ply: input.start_ply.max(1),
        observation_budget: None,
        max_output_tokens: input.model.max_output_tokens,
        temperature: input.model.temperature,
        inline_images: input.request_images.clone(),
        stream: true,
        require_tool_before_final: false,
        continue_on_tool_error: false,
        sub_agents: Default::default(),
        resume_sub_agent: None,
    };

    // Mirrors the SDK's observation list so every tool result can be checkpointed.
    let mut observations = input.prior_tool_observations.clone();
    let mut selected = std::collections::HashMap::<String, (String, Value)>::new();
    let result = run_agent_task_until_pause(http, task, |event| match event {
        AgentTaskEvent::ModelRequestStarted {
            ply,
            provider_kind,
            model,
        } => emit(AgentEvent::ModelRequestStarted {
            session_id,
            turn_id,
            ply,
            provider_kind,
            model,
        }),
        AgentTaskEvent::ModelStreamChunk { ply, text } => emit(AgentEvent::ModelStreamChunk {
            session_id,
            turn_id,
            ply,
            text,
        }),
        AgentTaskEvent::ModelResponseReceived { ply, raw_output } => {
            emit(AgentEvent::ModelResponseReceived {
                session_id,
                turn_id,
                ply,
                raw_output,
            })
        }
        AgentTaskEvent::ToolSelected {
            ply,
            rationale,
            raw_output,
            tool_id,
            tool_name,
            input,
        } => {
            let tool_input = input_to_tool_text(&input);
            selected.insert(tool_id.clone(), (rationale.clone(), input));
            emit(AgentEvent::ToolSelected {
                session_id,
                turn_id,
                ply,
                rationale,
                raw_output,
                tool_id,
                tool_name,
                tool_input,
            })
        }
        AgentTaskEvent::ToolResultProduced {
            ply,
            tool_id,
            tool_name,
            observation,
            metadata,
        } => {
            let (rationale, input) = selected.remove(&tool_id).unwrap_or_default();
            observations.push(AgentToolObservation {
                tool_id: tool_id.clone(),
                tool_name: tool_name.clone(),
                rationale,
                input,
                pending_message: None,
                observation: observation.clone(),
                metadata,
            });
            emit(AgentEvent::ToolResultProduced {
                session_id,
                turn_id,
                ply,
                tool_id,
                tool_name,
                observation,
                checkpoint: TurnCheckpoint {
                    observations: observations.clone(),
                    next_ply: ply + 1,
                },
            })
        }
        AgentTaskEvent::AsyncToolPending {
            ply,
            tool_id,
            tool_name,
            rationale,
            input,
            pending_message,
            metadata,
            observations,
        } => emit(AgentEvent::AsyncToolPending {
            session_id,
            turn_id,
            ply,
            tool_id,
            tool_name,
            rationale,
            input,
            pending_message,
            metadata,
            observations,
        }),
        AgentTaskEvent::FinalSelected {
            ply,
            rationale,
            raw_output,
            output_text,
            ..
        } => emit(AgentEvent::FinalSelected {
            session_id,
            turn_id,
            ply,
            rationale,
            raw_output,
            response: output_text,
        }),
        AgentTaskEvent::ContextCompacted { .. }
        | AgentTaskEvent::AgentDefined { .. }
        | AgentTaskEvent::SubAgent { .. } => Ok(()),
    })
    .await;

    match result {
        Ok(AgentTaskRunResult::Completed(outcome)) => {
            let final_ply = outcome
                .trace
                .steps
                .iter()
                .find_map(|step| match step {
                    AgentTaskTraceStep::Final { ply, .. } => Some(*ply),
                    _ => None,
                })
                .unwrap_or(1);
            emit(AgentEvent::TurnCompleted {
                session_id,
                turn_id,
                strategy: format!("Event flow completed after {} step(s).", final_ply),
            })?;
            Ok(DriveTurnResult::Completed)
        }
        Ok(AgentTaskRunResult::Paused(_pause)) => Ok(DriveTurnResult::Paused),
        Err(error) => {
            emit(AgentEvent::TurnFailed {
                session_id,
                turn_id,
                error,
                raw_output: None,
            })?;
            Ok(DriveTurnResult::Failed)
        }
    }
}

/// The read-only workspace tools, recording each result in the turn's working memory.
fn chat_sdk_tools(
    workspace_root: Option<&std::path::Path>,
    memory_state: Arc<Mutex<Vec<WorkingMemoryItem>>>,
) -> Vec<AgentTool> {
    let Some(root) = workspace_root else {
        return Vec::new();
    };
    workspace_tools(root)
        .into_iter()
        .map(|tool| {
            let memory_state = Arc::clone(&memory_state);
            let (id, name, description, input_schema) = (
                tool.id.clone(),
                tool.name.clone(),
                tool.description.clone(),
                tool.input_schema.clone(),
            );
            AgentTool::new_with_execution(
                id,
                name,
                description,
                input_schema,
                move |call: AgentToolCall| {
                    let execution = tool.execute(call.clone())?;
                    if let AgentToolExecution::Completed(result) = &execution {
                        let mut memory = memory_state
                            .lock()
                            .map_err(|_| "Working memory is unavailable.".to_string())?;
                        memory.push(WorkingMemoryItem::new(
                            "llm_decision",
                            format!("Tool request {}", call.tool_id),
                            format!("Rationale: {}\nTool input: {}", call.rationale, call.input),
                        ));
                        memory.push(WorkingMemoryItem::new(
                            "tool_result",
                            format!("Tool result: {}", call.tool_id),
                            result.observation.clone(),
                        ));
                    }
                    Ok(execution)
                },
            )
        })
        .collect()
}

fn recent_dialogue(messages: &[DialogueMessage], limit: usize) -> Vec<DialogueMessage> {
    messages
        .iter()
        .rev()
        .take(limit.max(1))
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn dialogue_to_model_messages(dialogue: &[DialogueMessage]) -> Vec<ModelMessage> {
    dialogue
        .iter()
        .map(|message| ModelMessage {
            role: match message.role {
                super::types::MessageRole::User => ModelMessageRole::User,
                super::types::MessageRole::Assistant => ModelMessageRole::Assistant,
            },
            content: message.content.clone(),
        })
        .collect()
}

fn input_to_tool_text(input: &Value) -> String {
    match input {
        Value::String(text) => text.trim().to_string(),
        Value::Object(map) => map
            .get("toolInput")
            .or_else(|| map.get("input"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| input.to_string()),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn format_memory_for_prompt(memory: &[WorkingMemoryItem]) -> String {
    if memory.is_empty() {
        return "No prior memory.".into();
    }

    memory
        .iter()
        .map(|item| {
            format!(
                "[{}] {}: {}",
                item.kind,
                item.title,
                compact_text(&item.content, 320)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_text(input: &str, limit: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }

    let snippet = trimmed.chars().take(limit).collect::<String>();
    format!("{snippet}...")
}
