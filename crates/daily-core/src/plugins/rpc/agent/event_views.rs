//! Event views a plugin task renders while it runs.
use super::*;

/// Folds one task event into the step list and returns the resulting changes.
pub(crate) fn apply_plugin_task_event_view(
    event_views: &mut Vec<agent::AgentEventView>,
    task_id: &str,
    event: &AgentTaskEvent,
) -> Vec<agent::AgentEventViewPatch> {
    match event {
        AgentTaskEvent::ModelRequestStarted {
            ply,
            provider_kind,
            model,
        } => vec![upsert_plugin_task_event_view(
            event_views,
            agent::AgentEventView::new(
                format!("{task_id}-ply-{ply}-thought"),
                agent::AgentEventViewKind::LlmThought,
                agent::AgentEventViewStatus::InProgress,
                "Agent thinking",
                "",
            )
            .with_ply(*ply)
            .with_metadata(json!({
                "request": format!("Requesting {} / {}.", provider_kind.label(), model),
            })),
            false,
        )],
        AgentTaskEvent::ModelStreamChunk { ply, text } => vec![upsert_plugin_task_event_view(
            event_views,
            agent::AgentEventView::new(
                format!("{task_id}-ply-{ply}-thought"),
                agent::AgentEventViewKind::LlmThought,
                agent::AgentEventViewStatus::InProgress,
                "Agent thinking",
                text.clone(),
            )
            .with_ply(*ply),
            true,
        )],
        AgentTaskEvent::ModelResponseReceived { ply, raw_output } => {
            vec![upsert_plugin_task_event_view(
                event_views,
                agent::AgentEventView::new(
                    format!("{task_id}-ply-{ply}-thought"),
                    agent::AgentEventViewKind::LlmThought,
                    agent::AgentEventViewStatus::Completed,
                    "Agent thinking",
                    "",
                )
                .with_ply(*ply)
                .with_metadata(json!({ "rawOutput": raw_output })),
                false,
            )]
        }
        AgentTaskEvent::ToolSelected {
            ply,
            tool_id,
            tool_name,
            input,
            rationale,
            ..
        } => vec![upsert_plugin_task_event_view(
            event_views,
            agent::AgentEventView::new(
                plugin_task_tool_view_id(task_id, *ply, tool_id),
                agent::AgentEventViewKind::ToolCall,
                agent::AgentEventViewStatus::InProgress,
                format!("Tool call: {tool_name}"),
                serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string()),
            )
            .with_ply(*ply)
            .with_tool(tool_id.clone(), tool_name.clone())
            .with_metadata(json!({ "rationale": rationale })),
            false,
        )],
        AgentTaskEvent::ToolResultProduced {
            ply,
            tool_id,
            tool_name,
            observation,
            metadata,
        } => {
            let failed = metadata.get("error").is_some();
            vec![
                upsert_plugin_task_event_view(
                    event_views,
                    agent::AgentEventView::new(
                        plugin_task_tool_view_id(task_id, *ply, tool_id),
                        agent::AgentEventViewKind::ToolCall,
                        if failed {
                            agent::AgentEventViewStatus::Failed
                        } else {
                            agent::AgentEventViewStatus::Completed
                        },
                        if failed { format!("Tool failed: {tool_name}") } else { format!("Tool done: {tool_name}") },
                        "",
                    )
                    .with_ply(*ply)
                    .with_tool(tool_id.clone(), tool_name.clone()),
                    false,
                ),
                upsert_plugin_task_event_view(
                    event_views,
                    agent::AgentEventView::new(
                        plugin_task_tool_result_view_id(task_id, *ply, tool_id),
                        agent::AgentEventViewKind::ToolResult,
                        agent::AgentEventViewStatus::Completed,
                        format!("Tool result: {tool_name}"),
                        observation.clone(),
                    )
                    .with_ply(*ply)
                    .with_tool(tool_id.clone(), tool_name.clone())
                    .with_metadata(metadata.clone()),
                    false,
                ),
            ]
        }
        AgentTaskEvent::AsyncToolPending {
            ply,
            tool_id,
            tool_name,
            rationale,
            pending_message,
            metadata,
            ..
        } => vec![upsert_plugin_task_event_view(
            event_views,
            // Keeps the tool input as content; the plugin runs the tool next.
            agent::AgentEventView::new(
                plugin_task_tool_view_id(task_id, *ply, tool_id),
                agent::AgentEventViewKind::ToolCall,
                agent::AgentEventViewStatus::Waiting,
                format!("Running: {tool_name}"),
                "",
            )
            .with_ply(*ply)
            .with_tool(tool_id.clone(), tool_name.clone())
            .with_metadata(json!({
                "rationale": rationale,
                "pendingMessage": pending_message,
                "tool": metadata,
            })),
            false,
        )],
        AgentTaskEvent::FinalSelected {
            ply,
            output_text,
            output_value,
            ..
        } => vec![upsert_plugin_task_event_view(
            event_views,
            agent::AgentEventView::new(
                format!("{task_id}-final"),
                agent::AgentEventViewKind::AssistantText,
                agent::AgentEventViewStatus::Completed,
                "Final result",
                output_text.clone(),
            )
            .with_role(agent::MessageRole::Assistant)
            .with_ply(*ply)
            .with_metadata(json!({ "output": output_value })),
            false,
        )],
        AgentTaskEvent::AgentDefined { ply, spec } => vec![upsert_plugin_task_event_view(
            event_views,
            agent::AgentEventView::new(
                format!("{task_id}-ply-{ply}-agent-{}", spec.id),
                agent::AgentEventViewKind::Runtime,
                agent::AgentEventViewStatus::Completed,
                format!("Created sub-agent: {}", spec.name),
                spec.instructions.clone(),
            )
            .with_ply(*ply)
            .with_metadata(json!({ "agent": spec })),
            false,
        )],
        AgentTaskEvent::SubAgent {
            agent_id,
            agent_name,
            parent_ply,
            event,
        } => {
            let frame_id = format!("{task_id}-ply-{parent_ply}-sub-{agent_id}");
            let patches = apply_plugin_task_event_view(event_views, &frame_id, event);
            let tag = json!({ "subAgent": { "id": agent_id, "name": agent_name, "parentPly": parent_ply } });
            patches
                .into_iter()
                .map(|mut patch| {
                    if let Some(view) = event_views.iter_mut().find(|view| view.id == patch.view.id)
                    {
                        view.metadata = merge_metadata(&view.metadata, tag.clone());
                        // A sub-agent's answer goes back to the parent; it is not the task's reply.
                        if view.kind == agent::AgentEventViewKind::AssistantText {
                            view.kind = agent::AgentEventViewKind::ToolResult;
                            view.role = None;
                            view.title = format!("Result from {agent_name}");
                        }
                        if matches!(patch.merge, agent::AgentEventViewMergeStrategy::Replace) {
                            patch.view = view.clone();
                        } else {
                            patch.view.metadata = merge_metadata(&patch.view.metadata, tag.clone());
                        }
                    }
                    patch
                })
                .collect()
        }
        AgentTaskEvent::ContextCompacted {
            ply,
            folded,
            summary_chars,
            error,
        } => vec![upsert_plugin_task_event_view(
            event_views,
            match error {
                None => agent::AgentEventView::new(
                    format!("{task_id}-ply-{ply}-compaction"),
                    agent::AgentEventViewKind::Runtime,
                    agent::AgentEventViewStatus::Completed,
                    "Compacted tool history",
                    format!("Folded {folded} earlier tool results into a {summary_chars}-character summary."),
                ),
                Some(error) => agent::AgentEventView::new(
                    format!("{task_id}-ply-{ply}-compaction"),
                    agent::AgentEventViewKind::Error,
                    agent::AgentEventViewStatus::Failed,
                    "Tool history compaction failed",
                    format!("Summary failed; truncated earlier tool results instead: {error}"),
                ),
            }
            .with_ply(*ply),
            false,
        )],
    }
}

/// Merges `next` into the step list. Empty content or `null` metadata keep
/// what the step already had. Returns the change to forward to listeners:
/// the appended text for streamed content, the merged step otherwise.
pub(crate) fn upsert_plugin_task_event_view(
    event_views: &mut Vec<agent::AgentEventView>,
    next: agent::AgentEventView,
    append_content: bool,
) -> agent::AgentEventViewPatch {
    let Some(existing) = event_views.iter_mut().find(|view| view.id == next.id) else {
        event_views.push(next.clone());
        return agent::AgentEventViewPatch::new(next, agent::AgentEventViewMergeStrategy::Replace);
    };
    let patch = append_content.then(|| {
        agent::AgentEventViewPatch::new(
            next.clone(),
            agent::AgentEventViewMergeStrategy::AppendContent,
        )
    });
    let content = if append_content {
        format!("{}{}", existing.content, next.content)
    } else if next.content.is_empty() {
        existing.content.clone()
    } else {
        next.content.clone()
    };
    existing.parser_id = next.parser_id;
    existing.kind = next.kind;
    existing.status = next.status;
    existing.role = next.role;
    existing.title = next.title;
    existing.content = content;
    existing.ply = next.ply;
    existing.tool_id = next.tool_id;
    existing.tool_name = next.tool_name;
    if !next.metadata.is_null() {
        existing.metadata = merge_metadata(&existing.metadata, next.metadata);
    }
    existing.updated_at = next.updated_at;
    patch.unwrap_or_else(|| {
        agent::AgentEventViewPatch::new(
            existing.clone(),
            agent::AgentEventViewMergeStrategy::Replace,
        )
    })
}

pub(super) fn merge_metadata(existing: &Value, next: Value) -> Value {
    match (existing, next) {
        (Value::Object(base), Value::Object(update)) => {
            let mut merged = base.clone();
            merged.extend(update);
            Value::Object(merged)
        }
        (_, next) => next,
    }
}

pub(crate) fn plugin_task_tool_view_id(task_id: &str, ply: usize, tool_id: &str) -> String {
    format!("{task_id}-ply-{ply}-tool-{tool_id}-call")
}

pub(crate) fn plugin_task_tool_result_view_id(task_id: &str, ply: usize, tool_id: &str) -> String {
    format!("{task_id}-ply-{ply}-tool-{tool_id}-result")
}
