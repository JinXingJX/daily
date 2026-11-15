use super::events::AgentEvent;
use super::types::{
    AgentEventView, AgentEventViewKind, AgentEventViewMergeStrategy, AgentEventViewPatch,
    AgentEventViewStatus, MessageRole,
};
use crate::plugins::PluginEventViewParserContribution;
use crate::state::AppState;
use serde_json::{json, Value};

pub trait AgentEventViewParser {
    fn id(&self) -> String;
    fn supports(&self, event: &AgentEvent) -> bool;
    fn parse(&self, event: &AgentEvent) -> Vec<AgentEventViewPatch>;
}

pub struct EventViewParserRegistry {
    parsers: Vec<Box<dyn AgentEventViewParser + Send + Sync>>,
}

impl Default for EventViewParserRegistry {
    fn default() -> Self {
        Self {
            parsers: vec![Box::new(DefaultAgentEventViewParser)],
        }
    }
}

impl EventViewParserRegistry {
    pub fn with_parser(mut self, parser: Box<dyn AgentEventViewParser + Send + Sync>) -> Self {
        self.parsers.push(parser);
        self
    }

    fn parse(&self, event: &AgentEvent) -> Vec<AgentEventViewPatch> {
        self.parsers
            .iter()
            .filter(|parser| parser.supports(event))
            .flat_map(|parser| {
                parser.parse(event).into_iter().map(|mut patch| {
                    patch.view.parser_id = parser.id().to_string();
                    patch
                })
            })
            .collect()
    }
}

pub fn apply_event_view_parsers(
    state: &mut AppState,
    event: &AgentEvent,
) -> Vec<AgentEventViewPatch> {
    let Some((session_id, turn_id)) = event_session_turn(event) else {
        return Vec::new();
    };
    let mut registry = EventViewParserRegistry::default();
    for (plugin_id, contribution) in plugin_event_view_parsers_for_turn(state, session_id, turn_id)
    {
        registry = registry.with_parser(Box::new(ManifestAgentEventViewParser {
            plugin_id,
            contribution,
        }));
    }
    let patches = registry.parse(event);
    for patch in patches.iter().cloned() {
        state.apply_event_view_patch(session_id, turn_id, patch);
    }
    patches
}

fn plugin_event_view_parsers_for_turn(
    state: &AppState,
    session_id: u64,
    turn_id: u64,
) -> Vec<(String, PluginEventViewParserContribution)> {
    let agent_app_id = state.session(session_id).and_then(|session| {
        session
            .turns
            .iter()
            .find(|turn| turn.id == turn_id)
            .and_then(|turn| turn.agent_app_id)
            .or(session.agent_app_id)
    });
    let Some(agent_app_id) = agent_app_id else {
        return Vec::new();
    };
    let Some(agent) = state
        .agent_apps
        .iter()
        .find(|agent| agent.id == agent_app_id)
    else {
        return Vec::new();
    };
    state.plugins.event_view_parsers(&agent.mounted_plugin_ids)
}

fn event_session_turn(event: &AgentEvent) -> Option<(u64, u64)> {
    match event {
        AgentEvent::SettingsApplied { .. } => None,
        AgentEvent::TurnStarted {
            session_id,
            turn_id,
        }
        | AgentEvent::ModelRequestStarted {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::ModelStreamChunk {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::ModelResponseReceived {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::ToolSelected {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::ToolResultProduced {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::AsyncToolPending {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::FinalSelected {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::TurnCompleted {
            session_id,
            turn_id,
            ..
        }
        | AgentEvent::TurnFailed {
            session_id,
            turn_id,
            ..
        } => Some((*session_id, *turn_id)),
    }
}

struct ManifestAgentEventViewParser {
    plugin_id: String,
    contribution: PluginEventViewParserContribution,
}

impl AgentEventViewParser for ManifestAgentEventViewParser {
    fn id(&self) -> String {
        format!("{}:{}", self.plugin_id, self.contribution.id)
    }

    fn supports(&self, event: &AgentEvent) -> bool {
        let Some(probe) = event_view_probe(event) else {
            return false;
        };
        self.contribution
            .support
            .supports(probe.kind, probe.tool_id, probe.status)
    }

    fn parse(&self, event: &AgentEvent) -> Vec<AgentEventViewPatch> {
        let Some((session_id, turn_id)) = event_session_turn(event) else {
            return Vec::new();
        };
        let Some(probe) = event_view_probe(event) else {
            return Vec::new();
        };
        let (id_suffix, title, content, merge) = manifest_event_view_content(event);
        let mut view = AgentEventView::new(
            format!(
                "turn-{turn_id}-plugin-{}-{}-{id_suffix}",
                self.plugin_id, self.contribution.id
            ),
            probe.view_kind,
            probe.view_status,
            title,
            content,
        )
        .with_metadata(json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "pluginId": self.plugin_id,
            "parserId": self.contribution.id,
            "renderer": self.contribution.renderer,
        }));
        if let Some(ply) = probe.ply {
            view = view.with_ply(ply);
        }
        if let (Some(tool_id), Some(tool_name)) = (probe.tool_id, probe.tool_name) {
            view = view.with_tool(tool_id, tool_name);
        }
        if matches!(probe.view_kind, AgentEventViewKind::AssistantText) {
            view = view.with_role(MessageRole::Assistant);
        }
        vec![AgentEventViewPatch::new(view, merge)]
    }
}

struct EventViewProbe<'a> {
    kind: &'static str,
    status: &'static str,
    tool_id: Option<&'a str>,
    tool_name: Option<&'a str>,
    ply: Option<usize>,
    view_kind: AgentEventViewKind,
    view_status: AgentEventViewStatus,
}

fn event_view_probe(event: &AgentEvent) -> Option<EventViewProbe<'_>> {
    match event {
        AgentEvent::SettingsApplied { .. } => None,
        AgentEvent::TurnStarted { .. } => Some(EventViewProbe {
            kind: "runtime",
            status: "in_progress",
            tool_id: None,
            tool_name: None,
            ply: None,
            view_kind: AgentEventViewKind::Runtime,
            view_status: AgentEventViewStatus::InProgress,
        }),
        AgentEvent::TurnCompleted { .. } => Some(EventViewProbe {
            kind: "runtime",
            status: "completed",
            tool_id: None,
            tool_name: None,
            ply: None,
            view_kind: AgentEventViewKind::Runtime,
            view_status: AgentEventViewStatus::Completed,
        }),
        AgentEvent::ModelRequestStarted { ply, .. } | AgentEvent::ModelStreamChunk { ply, .. } => {
            Some(EventViewProbe {
                kind: "llm_thought",
                status: "in_progress",
                tool_id: None,
                tool_name: None,
                ply: Some(*ply),
                view_kind: AgentEventViewKind::LlmThought,
                view_status: AgentEventViewStatus::InProgress,
            })
        }
        AgentEvent::ModelResponseReceived { ply, .. } => Some(EventViewProbe {
            kind: "llm_thought",
            status: "completed",
            tool_id: None,
            tool_name: None,
            ply: Some(*ply),
            view_kind: AgentEventViewKind::LlmThought,
            view_status: AgentEventViewStatus::Completed,
        }),
        AgentEvent::ToolSelected {
            ply,
            tool_id,
            tool_name,
            ..
        } => Some(EventViewProbe {
            kind: "tool_call",
            status: "in_progress",
            tool_id: Some(tool_id),
            tool_name: Some(tool_name),
            ply: Some(*ply),
            view_kind: AgentEventViewKind::ToolCall,
            view_status: AgentEventViewStatus::InProgress,
        }),
        AgentEvent::ToolResultProduced {
            ply,
            tool_id,
            tool_name,
            ..
        } => Some(EventViewProbe {
            kind: "tool_result",
            status: "completed",
            tool_id: Some(tool_id),
            tool_name: Some(tool_name),
            ply: Some(*ply),
            view_kind: AgentEventViewKind::ToolResult,
            view_status: AgentEventViewStatus::Completed,
        }),
        AgentEvent::AsyncToolPending {
            ply,
            tool_id,
            tool_name,
            ..
        } => Some(EventViewProbe {
            kind: "tool_call",
            status: "waiting",
            tool_id: Some(tool_id),
            tool_name: Some(tool_name),
            ply: Some(*ply),
            view_kind: AgentEventViewKind::ToolCall,
            view_status: AgentEventViewStatus::Waiting,
        }),
        AgentEvent::FinalSelected { ply, .. } => Some(EventViewProbe {
            kind: "assistant_text",
            status: "completed",
            tool_id: None,
            tool_name: None,
            ply: Some(*ply),
            view_kind: AgentEventViewKind::AssistantText,
            view_status: AgentEventViewStatus::Completed,
        }),
        AgentEvent::TurnFailed { .. } => Some(EventViewProbe {
            kind: "error",
            status: "failed",
            tool_id: None,
            tool_name: None,
            ply: None,
            view_kind: AgentEventViewKind::Error,
            view_status: AgentEventViewStatus::Failed,
        }),
    }
}

fn manifest_event_view_content(
    event: &AgentEvent,
) -> (String, String, String, AgentEventViewMergeStrategy) {
    match event {
        AgentEvent::TurnStarted { .. } => (
            "runtime-start".into(),
            "Run started".into(),
            "Agent started working.".into(),
            AgentEventViewMergeStrategy::Update,
        ),
        AgentEvent::TurnCompleted { strategy, .. } => (
            "runtime-complete".into(),
            "Run completed".into(),
            strategy.clone(),
            AgentEventViewMergeStrategy::Replace,
        ),
        AgentEvent::ModelRequestStarted {
            ply,
            provider_kind,
            model,
            ..
        } => (
            format!("ply-{ply}-thought"),
            "Agent thinking".into(),
            format!("Requesting {} / {}.", provider_kind.label(), model),
            AgentEventViewMergeStrategy::Update,
        ),
        AgentEvent::ModelStreamChunk { ply, text, .. } => (
            format!("ply-{ply}-thought"),
            "Agent thinking".into(),
            text.clone(),
            AgentEventViewMergeStrategy::AppendContent,
        ),
        AgentEvent::ModelResponseReceived { ply, .. } => (
            format!("ply-{ply}-thought"),
            "Agent thinking".into(),
            String::new(),
            AgentEventViewMergeStrategy::Update,
        ),
        AgentEvent::ToolSelected {
            ply,
            tool_id,
            tool_name,
            tool_input,
            ..
        } => (
            format!("ply-{ply}-tool-{tool_id}-call"),
            format!("Tool call: {tool_name}"),
            tool_input.clone(),
            AgentEventViewMergeStrategy::Replace,
        ),
        AgentEvent::ToolResultProduced {
            ply,
            tool_id,
            tool_name,
            observation,
            ..
        } => (
            format!("ply-{ply}-tool-{tool_id}-result"),
            format!("Tool result: {tool_name}"),
            observation.clone(),
            AgentEventViewMergeStrategy::Replace,
        ),
        AgentEvent::AsyncToolPending {
            ply,
            tool_id,
            tool_name,
            pending_message,
            ..
        } => (
            format!("ply-{ply}-tool-{tool_id}-pending"),
            format!("Waiting for tool: {tool_name}"),
            pending_message.clone(),
            AgentEventViewMergeStrategy::Update,
        ),
        AgentEvent::FinalSelected { response, .. } => (
            "final".into(),
            "Final reply".into(),
            response.clone(),
            AgentEventViewMergeStrategy::Replace,
        ),
        AgentEvent::TurnFailed { error, .. } => (
            "error".into(),
            "Run failed".into(),
            error.clone(),
            AgentEventViewMergeStrategy::Replace,
        ),
        AgentEvent::SettingsApplied { .. } => (
            "settings".into(),
            String::new(),
            String::new(),
            AgentEventViewMergeStrategy::Replace,
        ),
    }
}

struct DefaultAgentEventViewParser;

impl AgentEventViewParser for DefaultAgentEventViewParser {
    fn id(&self) -> String {
        "builtin.default".into()
    }

    fn supports(&self, event: &AgentEvent) -> bool {
        !matches!(event, AgentEvent::SettingsApplied { .. })
    }

    fn parse(&self, event: &AgentEvent) -> Vec<AgentEventViewPatch> {
        match event {
            AgentEvent::TurnStarted { turn_id, .. } => vec![AgentEventViewPatch::new(
                runtime_view(
                    format!("turn-{turn_id}-runtime-start"),
                    "Run started",
                    "Agent received the request and started.",
                    AgentEventViewStatus::InProgress,
                    None,
                    Value::Null,
                ),
                AgentEventViewMergeStrategy::Update,
            )],
            AgentEvent::ModelRequestStarted {
                turn_id,
                ply,
                provider_kind,
                model,
                ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    format!("turn-{turn_id}-ply-{ply}-thought"),
                    AgentEventViewKind::LlmThought,
                    AgentEventViewStatus::InProgress,
                    "Agent thinking",
                    format!("Requesting {} / {}.", provider_kind.label(), model),
                )
                .with_ply(*ply)
                .with_metadata(json!({
                    "providerKind": provider_kind,
                    "model": model,
                })),
                AgentEventViewMergeStrategy::Update,
            )],
            AgentEvent::ModelStreamChunk {
                turn_id, ply, text, ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    format!("turn-{turn_id}-ply-{ply}-thought"),
                    AgentEventViewKind::LlmThought,
                    AgentEventViewStatus::InProgress,
                    "Agent thinking",
                    text,
                )
                .with_ply(*ply),
                AgentEventViewMergeStrategy::AppendContent,
            )],
            AgentEvent::ModelResponseReceived {
                turn_id,
                ply,
                raw_output,
                ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    format!("turn-{turn_id}-ply-{ply}-thought"),
                    AgentEventViewKind::LlmThought,
                    AgentEventViewStatus::Completed,
                    "Agent thinking",
                    "",
                )
                .with_ply(*ply)
                .with_metadata(json!({
                    "rawOutput": raw_output,
                })),
                AgentEventViewMergeStrategy::Update,
            )],
            AgentEvent::ToolSelected {
                turn_id,
                ply,
                rationale,
                raw_output,
                tool_id,
                tool_name,
                tool_input,
                ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    tool_call_view_id(*turn_id, *ply, tool_id),
                    AgentEventViewKind::ToolCall,
                    AgentEventViewStatus::InProgress,
                    format!("Tool call: {tool_name}"),
                    tool_input,
                )
                .with_ply(*ply)
                .with_tool(tool_id, tool_name)
                .with_metadata(json!({
                    "rationale": rationale,
                    "rawOutput": raw_output,
                })),
                AgentEventViewMergeStrategy::Replace,
            )],
            AgentEvent::ToolResultProduced {
                turn_id,
                ply,
                tool_id,
                tool_name,
                observation,
                ..
            } => vec![
                AgentEventViewPatch::new(
                    AgentEventView::new(
                        tool_call_view_id(*turn_id, *ply, tool_id),
                        AgentEventViewKind::ToolCall,
                        AgentEventViewStatus::Completed,
                        format!("Tool done: {tool_name}"),
                        "",
                    )
                    .with_ply(*ply)
                    .with_tool(tool_id, tool_name),
                    AgentEventViewMergeStrategy::Update,
                ),
                AgentEventViewPatch::new(
                    AgentEventView::new(
                        format!("turn-{turn_id}-ply-{ply}-tool-{tool_id}-result"),
                        AgentEventViewKind::ToolResult,
                        AgentEventViewStatus::Completed,
                        format!("Tool result: {tool_name}"),
                        observation,
                    )
                    .with_ply(*ply)
                    .with_tool(tool_id, tool_name),
                    AgentEventViewMergeStrategy::Replace,
                ),
            ],
            AgentEvent::AsyncToolPending {
                turn_id,
                ply,
                tool_id,
                tool_name,
                rationale,
                input,
                pending_message,
                metadata,
                ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    tool_call_view_id(*turn_id, *ply, tool_id),
                    AgentEventViewKind::ToolCall,
                    AgentEventViewStatus::Waiting,
                    format!("Waiting for tool: {tool_name}"),
                    pending_message,
                )
                .with_ply(*ply)
                .with_tool(tool_id, tool_name)
                .with_metadata(json!({
                    "rationale": rationale,
                    "input": input,
                    "metadata": metadata,
                })),
                AgentEventViewMergeStrategy::Update,
            )],
            AgentEvent::FinalSelected {
                turn_id,
                ply,
                rationale,
                raw_output,
                response,
                ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    format!("turn-{turn_id}-final"),
                    AgentEventViewKind::AssistantText,
                    AgentEventViewStatus::Completed,
                    "Final reply",
                    response,
                )
                .with_role(MessageRole::Assistant)
                .with_ply(*ply)
                .with_metadata(json!({
                    "rationale": rationale,
                    "rawOutput": raw_output,
                })),
                AgentEventViewMergeStrategy::Replace,
            )],
            AgentEvent::TurnCompleted {
                turn_id, strategy, ..
            } => vec![AgentEventViewPatch::new(
                runtime_view(
                    format!("turn-{turn_id}-runtime-complete"),
                    "Run completed",
                    strategy,
                    AgentEventViewStatus::Completed,
                    None,
                    Value::Null,
                ),
                AgentEventViewMergeStrategy::Replace,
            )],
            AgentEvent::TurnFailed {
                turn_id,
                error,
                raw_output,
                ..
            } => vec![AgentEventViewPatch::new(
                AgentEventView::new(
                    format!("turn-{turn_id}-error"),
                    AgentEventViewKind::Error,
                    AgentEventViewStatus::Failed,
                    "Run failed",
                    error,
                )
                .with_metadata(json!({
                    "rawOutput": raw_output,
                })),
                AgentEventViewMergeStrategy::Replace,
            )],
            AgentEvent::SettingsApplied { .. } => Vec::new(),
        }
    }
}

fn runtime_view(
    id: String,
    title: impl Into<String>,
    content: impl Into<String>,
    status: AgentEventViewStatus,
    ply: Option<usize>,
    metadata: Value,
) -> AgentEventView {
    let mut view = AgentEventView::new(id, AgentEventViewKind::Runtime, status, title, content)
        .with_metadata(metadata);
    view.ply = ply;
    view
}

fn tool_call_view_id(turn_id: u64, ply: usize, tool_id: &str) -> String {
    format!("turn-{turn_id}-ply-{ply}-tool-{tool_id}-call")
}
