use super::event_views::apply_event_view_parsers;
use super::types::{
    DecisionStep, DialogueMessage, MemoryKind, MessageRole, PendingAsyncToolState, TurnCheckpoint,
    TurnDelta, TurnPatch, TurnStatus,
};
use crate::agent::AgentConfig;
use crate::model::{ModelConfig, ModelProviderKind};
use crate::settings::SettingsView;
use crate::state::AppState;

pub enum AgentEvent {
    SettingsApplied {
        agent_config: AgentConfig,
        model_config: Box<ModelConfig>,
        settings: Box<SettingsView>,
    },
    TurnStarted {
        session_id: u64,
        turn_id: u64,
    },
    ModelRequestStarted {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        provider_kind: ModelProviderKind,
        model: String,
    },
    ModelStreamChunk {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        text: String,
    },
    ModelResponseReceived {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        raw_output: String,
    },
    ToolSelected {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        rationale: String,
        raw_output: String,
        tool_id: String,
        tool_name: String,
        tool_input: String,
    },
    ToolResultProduced {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        tool_id: String,
        tool_name: String,
        observation: String,
        /// The turn's progress including this result, saved so a failure can continue from here.
        checkpoint: TurnCheckpoint,
    },
    AsyncToolPending {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        tool_id: String,
        tool_name: String,
        rationale: String,
        input: serde_json::Value,
        pending_message: String,
        metadata: serde_json::Value,
        observations: Vec<crate::agent::sdk::AgentToolObservation>,
    },
    FinalSelected {
        session_id: u64,
        turn_id: u64,
        ply: usize,
        rationale: String,
        raw_output: String,
        response: String,
    },
    TurnCompleted {
        session_id: u64,
        turn_id: u64,
        strategy: String,
    },
    TurnFailed {
        session_id: u64,
        turn_id: u64,
        error: String,
        raw_output: Option<String>,
    },
}

pub fn apply_event(state: &mut AppState, event: AgentEvent) {
    let _ = apply_event_delta(state, event);
}

pub fn apply_event_delta(state: &mut AppState, event: AgentEvent) -> Option<TurnDelta> {
    let event_view_patches = apply_event_view_parsers(state, &event);
    match event {
        AgentEvent::SettingsApplied {
            agent_config,
            model_config,
            settings,
        } => {
            state.update_settings(agent_config, *model_config, *settings);
            state.runtime_status = "Settings updated".into();
            None
        }
        AgentEvent::TurnStarted {
            session_id,
            turn_id,
        } => {
            let strategy = "The event-driven agent flow is running.".to_string();
            state.mark_running(session_id, turn_id, strategy.clone());
            state.runtime_status = "Streaming response".into();
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::Running),
                    strategy: Some(strategy),
                    clear_pending_async_tool: true,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::ModelRequestStarted {
            session_id,
            turn_id,
            ply,
            provider_kind,
            model,
        } => {
            let strategy = format!(
                "Waiting for model response on ply {} via {} / {}.",
                ply,
                provider_kind.label(),
                model
            );
            state.mark_running(session_id, turn_id, strategy.clone());
            state.runtime_status = format!("Ply {ply} running");
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::Running),
                    strategy: Some(strategy),
                    clear_pending_async_tool: true,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::ModelStreamChunk {
            session_id,
            turn_id,
            ply,
            text,
        } => {
            let strategy = format!("Streaming model output on ply {}.", ply);
            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.partial_response.push_str(&text);
                turn.status = TurnStatus::Running;
                turn.strategy = strategy.clone();
            }
            state.runtime_status = "Streaming response".into();
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::Running),
                    strategy: Some(strategy),
                    partial_response_append: Some(text),
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::ModelResponseReceived {
            session_id,
            turn_id,
            ply,
            raw_output,
        } => {
            let mut response = None;
            let strategy = format!("Model response received on ply {}.", ply);
            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.status = TurnStatus::Running;
                turn.strategy = strategy.clone();
                if turn.response.is_empty() {
                    let compact = compact_text(&raw_output, 240);
                    turn.response = compact.clone();
                    response = Some(compact);
                }
            }
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::Running),
                    strategy: Some(strategy),
                    response,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::ToolSelected {
            session_id,
            turn_id,
            ply,
            rationale,
            raw_output,
            tool_id,
            tool_name,
            tool_input,
        } => {
            let step = DecisionStep {
                ply,
                selected_action: tool_name.clone(),
                selected_kind: "tool".into(),
                rationale: rationale.clone(),
                model_output: raw_output,
                tool_name: Some(tool_name.clone()),
                tool_input: Some(tool_input.clone()),
                observation: None,
            };
            state.push_memory(
                session_id,
                turn_id,
                MemoryKind::LlmDecision,
                format!("LLM selected tool {tool_id}"),
                format!("Rationale: {rationale}\nTool input: {tool_input}"),
            );

            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.steps.push(step.clone());
            }
            state.runtime_status = format!("Tool selected on ply {ply}");
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    steps_append: vec![step],
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::ToolResultProduced {
            session_id,
            turn_id,
            ply,
            tool_id: _,
            tool_name,
            observation,
            checkpoint,
        } => {
            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.checkpoint = Some(checkpoint);
            }
            state.push_memory(
                session_id,
                turn_id,
                MemoryKind::ToolResult,
                format!("Tool result: {tool_name}"),
                &observation,
            );

            let mut steps = None;
            if let Some(step) = state.latest_step_mut(session_id, turn_id, ply) {
                step.observation = Some(observation);
            }
            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                steps = Some(turn.steps.clone());
            }
            state.runtime_status = format!("Tool result collected on ply {ply}");
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    steps,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::AsyncToolPending {
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
        } => {
            let pending_tool_name = tool_name.clone();
            let pending_message_text = pending_message.clone();
            state.push_memory(
                session_id,
                turn_id,
                MemoryKind::ToolResult,
                format!("Async tool pending: {tool_name}"),
                &pending_message,
            );

            if let Some(step) = state.latest_step_mut(session_id, turn_id, ply) {
                step.observation = Some(pending_message.clone());
            } else if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.steps.push(DecisionStep {
                    ply,
                    selected_action: tool_name.clone(),
                    selected_kind: "async_tool_pending".into(),
                    rationale: format!("Async tool {tool_id} is still running."),
                    model_output: String::new(),
                    tool_name: Some(tool_name.clone()),
                    tool_input: None,
                    observation: Some(pending_message.clone()),
                });
            }

            state.pause_turn_for_async_tool(
                session_id,
                turn_id,
                PendingAsyncToolState {
                    ply,
                    tool_id,
                    tool_name,
                    rationale,
                    input,
                    pending_message,
                    metadata,
                    observations,
                },
            );
            let (steps, pending_async_tool) = state
                .turn_mut(session_id, turn_id)
                .map(|turn| (Some(turn.steps.clone()), turn.pending_async_tool.clone()))
                .unwrap_or((None, None));
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::WaitingAsyncTool),
                    strategy: Some(format!("Waiting for async tool {}.", pending_tool_name)),
                    response: Some(pending_message_text),
                    partial_response: Some(String::new()),
                    clear_partial_response: true,
                    pending_async_tool,
                    steps,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::FinalSelected {
            session_id,
            turn_id,
            ply,
            rationale,
            raw_output,
            response,
        } => {
            let response_text = response.clone();
            let message = DialogueMessage {
                id: state
                    .push_message(session_id, MessageRole::Assistant, &response, Some(turn_id))
                    .unwrap_or(0),
                turn_id: Some(turn_id),
                role: MessageRole::Assistant,
                content: response.clone(),
            };
            state.push_memory(
                session_id,
                turn_id,
                MemoryKind::LlmDecision,
                "LLM selected final answer",
                &rationale,
            );
            state.push_memory(
                session_id,
                turn_id,
                MemoryKind::AssistantMessage,
                "Assistant reply",
                &response,
            );

            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.response = response.clone();
                turn.partial_response.clear();
                turn.steps.push(DecisionStep {
                    ply,
                    selected_action: "Final answer".into(),
                    selected_kind: "final".into(),
                    rationale,
                    model_output: raw_output,
                    tool_name: None,
                    tool_input: None,
                    observation: Some(response),
                });
            }
            state.runtime_status = "Final answer selected".into();
            let steps = state
                .turn_mut(session_id, turn_id)
                .map(|turn| turn.steps.clone());
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    response: Some(response_text),
                    partial_response: Some(String::new()),
                    clear_partial_response: true,
                    steps,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                vec![message],
            ))
        }
        AgentEvent::TurnCompleted {
            session_id,
            turn_id,
            strategy,
        } => {
            let strategy_text = strategy.clone();
            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.status = TurnStatus::Completed;
                turn.strategy = strategy;
            }
            state.complete_turn(session_id);
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::Completed),
                    strategy: Some(strategy_text),
                    event_view_patches,
                    ..TurnPatch::default()
                },
                Vec::new(),
            ))
        }
        AgentEvent::TurnFailed {
            session_id,
            turn_id,
            error,
            raw_output,
        } => {
            let error_text = error.clone();
            let message = DialogueMessage {
                id: state
                    .push_message(session_id, MessageRole::Assistant, &error, Some(turn_id))
                    .unwrap_or(0),
                turn_id: Some(turn_id),
                role: MessageRole::Assistant,
                content: error.clone(),
            };
            state.push_memory(
                session_id,
                turn_id,
                MemoryKind::Error,
                "Turn failed",
                &error,
            );

            if let Some(turn) = state.turn_mut(session_id, turn_id) {
                turn.status = TurnStatus::Failed;
                turn.strategy = error.clone();
                turn.partial_response.clear();
                if turn.response.is_empty() {
                    turn.response = error.clone();
                }
                turn.steps.push(DecisionStep {
                    ply: turn.steps.len() + 1,
                    selected_action: "Failure".into(),
                    selected_kind: "error".into(),
                    rationale: "The event-driven loop stopped with an error.".into(),
                    model_output: raw_output.unwrap_or_default(),
                    tool_name: None,
                    tool_input: None,
                    observation: Some(error.clone()),
                });
            }
            state.complete_turn(session_id);
            state.runtime_status = "Turn failed".into();
            let steps = state
                .turn_mut(session_id, turn_id)
                .map(|turn| turn.steps.clone());
            Some(turn_delta(
                state,
                session_id,
                turn_id,
                TurnPatch {
                    status: Some(TurnStatus::Failed),
                    strategy: Some(error_text.clone()),
                    response: Some(error_text),
                    partial_response: Some(String::new()),
                    clear_partial_response: true,
                    steps,
                    event_view_patches,
                    ..TurnPatch::default()
                },
                vec![message],
            ))
        }
    }
}

fn turn_delta(
    state: &AppState,
    session_id: u64,
    turn_id: u64,
    turn_patch: TurnPatch,
    message_appends: Vec<DialogueMessage>,
) -> TurnDelta {
    TurnDelta {
        session_id,
        turn_id,
        active_session_id: state.active_session_id,
        runtime: state.runtime_view(),
        session_updated_at: state.session(session_id).map(|session| session.updated_at),
        message_appends,
        turn_patch,
    }
}

fn compact_text(input: &str, limit: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }

    let snippet = trimmed.chars().take(limit).collect::<String>();
    format!("{snippet}...")
}

#[cfg(test)]
mod tests {
    use super::super::types::AgentEventViewMergeStrategy;
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_state() -> AppState {
        let root = unique_test_root();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should be created");
        let loaded =
            crate::settings::default_settings(&root).expect("default settings should load");
        AppState::seeded(&root, loaded.agent, loaded.model, loaded.view).expect("state should seed")
    }

    fn unique_test_root() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("daily-event-delta-test-{stamp}"))
    }

    #[test]
    fn stream_chunk_returns_append_delta() {
        let mut state = test_state();
        let turn_id = state
            .queue_turn(1, "hello", Vec::new())
            .expect("turn should queue");

        let delta = apply_event_delta(
            &mut state,
            AgentEvent::ModelStreamChunk {
                session_id: 1,
                turn_id,
                ply: 1,
                text: "partial".into(),
            },
        )
        .expect("stream chunks should produce delta");

        assert_eq!(delta.session_id, 1);
        assert_eq!(delta.turn_id, turn_id);
        assert_eq!(
            delta.turn_patch.partial_response_append.as_deref(),
            Some("partial")
        );
        assert!(delta
            .turn_patch
            .event_view_patches
            .iter()
            .any(|patch| matches!(patch.merge, AgentEventViewMergeStrategy::AppendContent)));
        assert_eq!(
            state
                .turn_mut(1, turn_id)
                .expect("turn should exist")
                .partial_response,
            "partial"
        );
    }

    #[test]
    fn final_selected_returns_message_append_and_clears_partial() {
        let mut state = test_state();
        let turn_id = state
            .queue_turn(1, "hello", Vec::new())
            .expect("turn should queue");
        if let Some(turn) = state.turn_mut(1, turn_id) {
            turn.partial_response = "partial".into();
        }

        let delta = apply_event_delta(
            &mut state,
            AgentEvent::FinalSelected {
                session_id: 1,
                turn_id,
                ply: 1,
                rationale: "done".into(),
                raw_output: "done".into(),
                response: "final answer".into(),
            },
        )
        .expect("final selection should produce delta");

        assert!(delta.turn_patch.clear_partial_response);
        assert_eq!(delta.turn_patch.response.as_deref(), Some("final answer"));
        assert_eq!(delta.message_appends.len(), 1);
        assert_eq!(delta.message_appends[0].content, "final answer");
        let turn = state.turn_mut(1, turn_id).expect("turn should exist");
        assert_eq!(turn.partial_response, "");
        assert_eq!(turn.response, "final answer");
    }
}
