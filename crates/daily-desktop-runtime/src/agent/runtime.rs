use super::compaction::estimate_tokens;
use super::config::AgentConfig;
use super::events::AgentEvent;
use super::sdk::{
    agent_tool_from_dynamic, AgentContextBlock, AgentTask, AgentTaskEvent, AgentTaskRunResult,
    AgentTaskTraceStep, AgentToolObservation, DynamicAgentToolDefinition, ObservationBudget,
};
use super::tools::{recent_working_memory, WorkingMemoryItem};
use super::types::{ChatHistorySummary, DialogueMessage, MemoryEntry, MessageRole, TurnCheckpoint};
use super::workspace_tools::workspace_tools;
use crate::limits::{
    CHAT_HISTORY_BUDGET_TOKENS, CHAT_KEEP_RECENT_OBSERVATIONS, CHAT_MAX_ITERATIONS,
    CHAT_OBSERVATION_BUDGET_TOKENS,
};
use crate::model::{ModelConfig, ModelInlineImage, ModelMessage, ModelStopReason};
use reqwest::Client;
use serde_json::Value;

pub struct TurnRuntimeInput {
    pub execution: Option<daily_agent_core::RunContext>,
    pub runtime_checkpoint: Option<String>,
    pub agent: AgentConfig,
    pub model: ModelConfig,
    /// Binds the read-only workspace tools; `None` offers no workspace tools.
    pub workspace_root: Option<std::path::PathBuf>,
    pub dialogue: Vec<DialogueMessage>,
    /// Summary of the dialogue up to one of its messages; those are not replayed.
    pub history_summary: Option<ChatHistorySummary>,
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
    let mut tools = input
        .workspace_root
        .as_deref()
        .map(workspace_tools)
        .unwrap_or_default();
    tools.extend(
        input
            .dynamic_tools
            .clone()
            .into_iter()
            .map(agent_tool_from_dynamic),
    );
    let task = AgentTask {
        checkpoint_ref: input.runtime_checkpoint.clone(),
        execution: input.execution,
        checkpoint: None,
        resume_action: daily_agent_core::checkpoint::ResumeAction::Continue,
        agent_definitions: Default::default(),
        model_bindings: Default::default(),
        name: "chat.turn".into(),
        session_id: Some(session_id),
        turn_id: Some(turn_id),
        model: input.model.clone(),
        model_adapter: None,
        system_prompt: input.agent.system_prompt.clone(),
        user_prompt: prompt.trim().to_string(),
        final_instructions:
            "For a chat turn, the final answer is plain text ready to show to the user.".into(),
        context: vec![AgentContextBlock::new(
            "Working memory",
            format_memory_for_prompt(&working_memory),
        )],
        conversation: chat_history(
            &input.dialogue,
            input.history_summary.as_ref(),
            turn_id,
            input.agent.dialogue_window as usize,
            CHAT_HISTORY_BUDGET_TOKENS,
        ),
        prior_tool_observations: input.prior_tool_observations.clone(),
        tools,
        max_iterations: (input.agent.max_iterations as usize).clamp(1, CHAT_MAX_ITERATIONS),
        start_ply: input.start_ply.max(1),
        observation_budget: Some(ObservationBudget {
            max_tokens: CHAT_OBSERVATION_BUDGET_TOKENS,
            keep_recent: CHAT_KEEP_RECENT_OBSERVATIONS,
        }),
        max_output_tokens: input.model.max_output_tokens,
        temperature: input.model.temperature,
        inline_images: input.request_images.clone(),
        stream: true,
        require_tool_before_final: false,
        // A failed or mistaken tool call goes back to the model to correct.
        continue_on_tool_error: true,
        sub_agents: Default::default(),
        allow_handoff: false,
        resume_sub_agent: None,
    };

    let prepared = task.prepare(http).await;
    let execution = prepared.as_ref().ok().map(|task| task.execution());
    // Mirrors the SDK's observation list so every tool result can be checkpointed.
    let mut observations = input.prior_tool_observations.clone();
    let mut selected = std::collections::HashMap::<String, (String, Value)>::new();
    let mut on_event = |event| match event {
        AgentTaskEvent::ModelRequestStarted {
            ply,
            provider_kind,
            model,
        } => emit(AgentEvent::ModelRequestStarted {
            checkpoint: execution
                .as_ref()
                .and_then(|ctx| ctx.checkpoint())
                .map(|cp| TurnCheckpoint {
                    observations: cp.task.observations.clone(),
                    next_ply: cp.task.next_ply,
                    runtime: Some(super::types::AgentRunRef {
                        run_id: cp.run.run_id,
                    }),
                }),
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
        AgentTaskEvent::ModelResponseReceived {
            ply,
            raw_output,
            usage,
            ..
        } => emit(AgentEvent::ModelResponseReceived {
            session_id,
            turn_id,
            ply,
            raw_output,
            usage,
        }),
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
            call,
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
                call,
            });
            emit(AgentEvent::ToolResultProduced {
                session_id,
                turn_id,
                ply,
                tool_id,
                tool_name,
                observation,
                checkpoint: TurnCheckpoint {
                    runtime: execution
                        .as_ref()
                        .and_then(|ctx| ctx.checkpoint())
                        .map(|cp| super::types::AgentRunRef {
                            run_id: cp.run.run_id,
                        }),
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
            call,
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
            call,
        }),
        AgentTaskEvent::FinalSelected {
            ply,
            rationale,
            raw_output,
            output_text,
            stop_reason,
            ..
        } => emit(AgentEvent::FinalSelected {
            session_id,
            turn_id,
            ply,
            rationale,
            raw_output,
            response: output_text,
            truncated: stop_reason == ModelStopReason::MaxTokens,
        }),
        AgentTaskEvent::ContextCompacted { .. }
        | AgentTaskEvent::AgentDefined { .. }
        | AgentTaskEvent::SubAgent { .. } => Ok(()),
    };
    let result = match prepared {
        Ok(task) => task.run_until_pause(&mut on_event).await,
        Err(error) => Err(error),
    };

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
            let mut strategy = format!("Event flow completed after {} step(s).", final_ply);
            if outcome.stop_reason == ModelStopReason::MaxTokens {
                strategy.push_str(" The answer was cut off at the max output token limit.");
            }
            emit(AgentEvent::TurnCompleted {
                session_id,
                turn_id,
                strategy,
            })?;
            Ok(DriveTurnResult::Completed)
        }
        Ok(AgentTaskRunResult::Paused(_pause)) => Ok(DriveTurnResult::Paused),
        Err(error) => {
            emit(AgentEvent::TurnFailed {
                session_id,
                turn_id,
                code: Some(error.code().to_string()),
                error: error.to_string(),
                raw_output: None,
            })?;
            Ok(DriveTurnResult::Failed)
        }
    }
}

/// The conversation before this turn as the model should see it: the summary of what was
/// folded away, then the newest messages that fit `budget_tokens`, always including the last
/// `keep_recent`. Messages of the running turn itself are left out: its request is sent as the
/// current prompt, and an earlier failed attempt's error text is not something the agent said.
pub(crate) fn chat_history(
    dialogue: &[DialogueMessage],
    summary: Option<&ChatHistorySummary>,
    turn_id: u64,
    keep_recent: usize,
    budget_tokens: usize,
) -> Vec<ModelMessage> {
    let through = summary
        .map(|summary| summary.through_message_id)
        .unwrap_or(0);
    let candidates = dialogue
        .iter()
        .filter(|message| message.id > through && message.turn_id != Some(turn_id))
        .collect::<Vec<_>>();
    let mut used = summary
        .map(|summary| estimate_tokens(&summary.content))
        .unwrap_or(0);
    let mut start = candidates.len();
    while start > 0 {
        let cost = estimate_tokens(&candidates[start - 1].content);
        if candidates.len() - start >= keep_recent.max(1) && used + cost > budget_tokens {
            break;
        }
        used += cost;
        start -= 1;
    }

    let mut messages = Vec::new();
    if let Some(summary) = summary {
        messages.push(ModelMessage::system(format!(
            "Summary of the earlier conversation in this session:\n{}",
            summary.content
        )));
    }
    if start > 0 {
        messages.push(ModelMessage::system(format!(
            "[{start} earlier message(s) of this session are not shown.]"
        )));
    }
    messages.extend(
        candidates[start..]
            .iter()
            .map(|message| match message.role {
                MessageRole::User => ModelMessage::user(message.content.clone()),
                MessageRole::Assistant => ModelMessage::assistant(message.content.clone()),
            }),
    );
    messages
}

/// The older part of the dialogue that no longer fits `budget_tokens` and should be folded
/// into the session summary: its text for the summarizer and the id of its newest message.
/// `None` while everything fits or nothing but the `keep_recent` newest messages is left.
pub(crate) fn plan_history_fold(
    dialogue: &[DialogueMessage],
    summary: Option<&ChatHistorySummary>,
    turn_id: u64,
    keep_recent: usize,
    budget_tokens: usize,
) -> Option<(String, u64)> {
    let through = summary
        .map(|summary| summary.through_message_id)
        .unwrap_or(0);
    let candidates = dialogue
        .iter()
        .filter(|message| message.id > through && message.turn_id != Some(turn_id))
        .collect::<Vec<_>>();
    let sizes = candidates
        .iter()
        .map(|message| estimate_tokens(&message.content))
        .collect::<Vec<_>>();
    let mut total = summary
        .map(|summary| estimate_tokens(&summary.content))
        .unwrap_or(0)
        + sizes.iter().sum::<usize>();
    let foldable = candidates.len().saturating_sub(keep_recent.max(1));
    let mut fold = 0;
    while total > budget_tokens && fold < foldable {
        total -= sizes[fold];
        fold += 1;
    }
    let last = candidates[..fold].last()?;
    let history = candidates[..fold]
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
            };
            format!("{role}:\n{}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Some((history, last.id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelMessageRole;

    fn message(id: u64, turn_id: u64, role: MessageRole, content: &str) -> DialogueMessage {
        DialogueMessage {
            id,
            turn_id: Some(turn_id),
            role,
            content: content.into(),
        }
    }

    #[test]
    fn history_skips_the_running_turn_and_respects_the_budget_and_summary() {
        let dialogue = vec![
            message(1, 1, MessageRole::User, &"a".repeat(400)),
            message(2, 1, MessageRole::Assistant, &"b".repeat(400)),
            message(3, 2, MessageRole::User, "second question"),
            message(4, 2, MessageRole::Assistant, "second answer"),
            message(5, 3, MessageRole::User, "current request"),
            message(6, 3, MessageRole::Assistant, "Rate limited."),
        ];

        // Everything fits: both earlier turns, nothing of turn 3.
        let all = chat_history(&dialogue, None, 3, 2, 10_000);
        assert_eq!(all.len(), 4);
        assert!(
            all.iter()
                .all(|m| !m.content.contains("current request")
                    && !m.content.contains("Rate limited"))
        );

        // A tight budget keeps the newest two and says how many are missing.
        let tight = chat_history(&dialogue, None, 3, 2, 20);
        assert_eq!(tight.len(), 3);
        assert_eq!(tight[0].role, ModelMessageRole::System);
        assert!(tight[0].content.contains("2 earlier message(s)"));
        assert_eq!(tight[1].content, "second question");
        assert_eq!(tight[2].role, ModelMessageRole::Assistant);

        // Summarized messages are replaced by the summary.
        let summary = ChatHistorySummary {
            content: "They discussed a and b.".into(),
            through_message_id: 2,
        };
        let summarized = chat_history(&dialogue, Some(&summary), 3, 2, 10_000);
        assert_eq!(summarized.len(), 3);
        assert!(summarized[0].content.contains("They discussed a and b."));
        assert_eq!(summarized[1].content, "second question");

        // Folding takes the oldest messages that do not fit, never the newest ones or
        // the running turn, and continues after an existing summary.
        assert!(plan_history_fold(&dialogue, None, 3, 2, 10_000).is_none());
        let (history, through) = plan_history_fold(&dialogue, None, 3, 2, 20).unwrap();
        assert_eq!(through, 2);
        assert!(history.starts_with("User:\naaaa"));
        assert!(history.contains("Assistant:\nbbbb"));
        assert!(!history.contains("second question"));
        assert!(plan_history_fold(&dialogue, Some(&summary), 3, 2, 1).is_none());
    }
}
