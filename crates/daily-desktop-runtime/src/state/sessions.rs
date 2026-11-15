use crate::agent::types::{
    AgentEventView, AgentEventViewMergeStrategy, AgentEventViewPatch, AgentTurn, ChatSessionState,
    DecisionStep, DialogueMessage, MemoryEntry, MemoryKind, MessageRole, PendingAsyncToolState,
    TurnImageInput, TurnStatus,
};
use crate::state::{AppState, DEFAULT_SESSION_TITLE};
use crate::util::unix_timestamp;
use std::sync::Arc;

pub(crate) fn update_event_view_fields(
    existing: &mut AgentEventView,
    next: AgentEventView,
    preserve_existing_content: bool,
) {
    existing.parser_id = next.parser_id;
    existing.kind = next.kind;
    existing.status = next.status;
    existing.role = next.role;
    existing.title = next.title;
    if !preserve_existing_content || (existing.content.is_empty() && !next.content.is_empty()) {
        existing.content = next.content;
    }
    existing.ply = next.ply;
    existing.tool_id = next.tool_id;
    existing.tool_name = next.tool_name;
    existing.metadata = next.metadata;
    existing.updated_at = next.updated_at;
}

pub(crate) fn summarize_title(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return DEFAULT_SESSION_TITLE.into();
    }
    let one_line = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = one_line.chars().take(36).collect::<String>();
    if one_line.chars().count() > 36 {
        title.push('…');
    }
    title
}

impl AppState {
    pub fn active_session(&self) -> Option<&ChatSessionState> {
        self.active_session_id
            .and_then(|session_id| self.session(session_id))
    }

    pub fn session(&self, session_id: u64) -> Option<&ChatSessionState> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| &**session)
    }

    pub fn session_mut(&mut self, session_id: u64) -> Option<&mut ChatSessionState> {
        self.sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .map(Arc::make_mut)
    }

    pub fn turn_mut(&mut self, session_id: u64, turn_id: u64) -> Option<&mut AgentTurn> {
        self.session_mut(session_id)?
            .turns
            .iter_mut()
            .find(|turn| turn.id == turn_id)
    }

    pub fn latest_step_mut(
        &mut self,
        session_id: u64,
        turn_id: u64,
        ply: usize,
    ) -> Option<&mut DecisionStep> {
        self.turn_mut(session_id, turn_id)?
            .steps
            .iter_mut()
            .find(|step| step.ply == ply)
    }

    pub fn apply_event_view_patch(
        &mut self,
        session_id: u64,
        turn_id: u64,
        mut patch: AgentEventViewPatch,
    ) {
        patch.view.updated_at = unix_timestamp();
        let Some(turn) = self.turn_mut(session_id, turn_id) else {
            return;
        };
        let Some(existing) = turn
            .event_views
            .iter_mut()
            .find(|view| view.id == patch.view.id)
        else {
            turn.event_views.push(patch.view);
            return;
        };

        let created_at = existing.created_at;
        match patch.merge {
            AgentEventViewMergeStrategy::Replace => {
                patch.view.created_at = created_at;
                *existing = patch.view;
            }
            AgentEventViewMergeStrategy::AppendContent => {
                if !patch.view.content.is_empty() {
                    existing.content.push_str(&patch.view.content);
                }
                update_event_view_fields(existing, patch.view, true);
            }
            AgentEventViewMergeStrategy::Update => {
                update_event_view_fields(existing, patch.view, true);
            }
        }
    }

    pub fn push_message(
        &mut self,
        session_id: u64,
        role: MessageRole,
        content: &str,
        turn_id: Option<u64>,
    ) -> Option<u64> {
        let session = self.session_mut(session_id)?;
        let id = session.next_message_id;
        session.next_message_id += 1;
        session.messages.push(DialogueMessage {
            id,
            turn_id,
            role,
            content: content.to_string(),
        });
        session.updated_at = unix_timestamp();
        Some(id)
    }

    pub fn push_memory(
        &mut self,
        session_id: u64,
        turn_id: u64,
        kind: MemoryKind,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Option<u64> {
        let session = self.session_mut(session_id)?;
        let id = session.next_memory_id;
        session.next_memory_id += 1;
        session.memory.push(MemoryEntry {
            id,
            turn_id,
            kind,
            title: title.into(),
            content: content.into(),
        });
        session.updated_at = unix_timestamp();
        Some(id)
    }

    pub fn queue_turn(
        &mut self,
        session_id: u64,
        prompt: &str,
        images: Vec<TurnImageInput>,
    ) -> Result<u64, String> {
        self.ensure_can_start_turn(session_id)?;
        if self
            .session(session_id)
            .map(|session| {
                session
                    .turns
                    .iter()
                    .any(|turn| matches!(turn.status, TurnStatus::WaitingAsyncTool))
            })
            .unwrap_or(false)
        {
            return Err("Resume the pending async tool before sending another message.".into());
        }
        let title_update = self
            .session(session_id)
            .map(|session| session.title == DEFAULT_SESSION_TITLE)
            .unwrap_or(false);
        let mut next_title = None;
        let turn_id = {
            let Some(session) = self.session_mut(session_id) else {
                return Err("Session was not found.".into());
            };
            let turn_id = session.next_turn_id;
            session.next_turn_id += 1;
            session.running_turn_id = Some(turn_id);
            session.turns.push(AgentTurn {
                id: turn_id,
                prompt: prompt.to_string(),
                response: String::new(),
                strategy: "Queued".into(),
                status: TurnStatus::Queued,
                partial_response: String::new(),
                steps: Vec::new(),
                event_views: vec![AgentEventView::user_input(turn_id, prompt)],
                images,
                agent_app_id: None,
                agent_name: None,
                pending_async_tool: None,
                checkpoint: None,
                usage: None,
                error_code: None,
            });
            session.updated_at = unix_timestamp();
            if title_update {
                let title = summarize_title(prompt);
                session.title = title.clone();
                next_title = Some(title);
            }
            turn_id
        };
        if let Some(title) = next_title {
            if let Some(node_id) = self.session_node_id(session_id) {
                if let Some(node) = self.node_mut(node_id) {
                    node.name = title;
                }
            }
        }
        self.active_session_id = Some(session_id);
        self.runtime_status = "Turn queued".into();
        self.push_message(session_id, MessageRole::User, prompt, Some(turn_id));
        self.push_memory(
            session_id,
            turn_id,
            MemoryKind::UserMessage,
            "User message",
            prompt,
        );
        Ok(turn_id)
    }

    pub fn retry_from_message(
        &mut self,
        session_id: u64,
        message_id: u64,
    ) -> Result<(String, Vec<TurnImageInput>), String> {
        self.ensure_can_start_turn(session_id)?;

        let (turn_id, prompt, images) = {
            let session = self
                .session(session_id)
                .ok_or_else(|| "Session was not found.".to_string())?;
            let message = session
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .ok_or_else(|| "Message was not found.".to_string())?;

            if !matches!(message.role, MessageRole::Assistant) {
                return Err("Only assistant messages can be retried.".into());
            }

            let turn_id = message.turn_id.ok_or_else(|| {
                "This assistant message is not associated with a retryable turn.".to_string()
            })?;
            let turn = session
                .turns
                .iter()
                .find(|turn| turn.id == turn_id)
                .ok_or_else(|| "The turn for this message was not found.".to_string())?;
            (turn_id, turn.prompt.clone(), turn.images.clone())
        };

        let Some(session) = self.session_mut(session_id) else {
            return Err("Session was not found.".into());
        };
        session.messages.retain(|message| match message.turn_id {
            Some(existing_turn_id) => existing_turn_id < turn_id,
            None => true,
        });
        session.turns.retain(|turn| turn.id < turn_id);
        session.memory.retain(|entry| entry.turn_id < turn_id);
        // A summary that covers removed messages describes a conversation that no longer exists.
        let newest_message = session.messages.iter().map(|message| message.id).max();
        if session
            .history_summary
            .as_ref()
            .is_some_and(|summary| Some(summary.through_message_id) > newest_message)
        {
            session.history_summary = None;
        }
        session.running_turn_id = None;
        session.updated_at = unix_timestamp();

        self.active_session_id = Some(session_id);

        Ok((prompt, images))
    }

    /// Reopens the session's latest turn after it failed or was stopped: drops the failure
    /// it recorded (reply, memory entry, step and step view) and marks it running again.
    pub fn continue_failed_turn(&mut self, session_id: u64, turn_id: u64) -> Result<(), String> {
        self.ensure_can_start_turn(session_id)?;
        let Some(session) = self.session_mut(session_id) else {
            return Err("Session was not found.".into());
        };
        let Some(turn) = session.turns.last_mut().filter(|turn| turn.id == turn_id) else {
            return Err("Only the latest turn can be continued.".into());
        };
        if !matches!(turn.status, TurnStatus::Failed) {
            return Err("Only a failed or stopped turn can be continued.".into());
        }
        turn.error_code = None;
        let error_view = format!("turn-{turn_id}-error");
        turn.event_views.retain(|view| view.id != error_view);
        turn.steps.retain(|step| step.selected_kind != "error");
        turn.status = TurnStatus::Running;
        turn.strategy = "Continuing from the last checkpoint.".into();
        turn.response.clear();
        turn.partial_response.clear();
        session.messages.retain(|message| {
            message.turn_id != Some(turn_id) || matches!(message.role, MessageRole::User)
        });
        session
            .memory
            .retain(|entry| entry.turn_id != turn_id || !matches!(entry.kind, MemoryKind::Error));
        session.running_turn_id = Some(turn_id);
        session.updated_at = unix_timestamp();
        self.active_session_id = Some(session_id);
        self.runtime_status = "Continuing turn".into();
        Ok(())
    }

    pub fn mark_running(&mut self, session_id: u64, turn_id: u64, message: impl Into<String>) {
        if let Some(turn) = self.turn_mut(session_id, turn_id) {
            turn.status = TurnStatus::Running;
            turn.strategy = message.into();
            turn.pending_async_tool = None;
        }
        if let Some(session) = self.session_mut(session_id) {
            session.updated_at = unix_timestamp();
        }
    }

    pub fn assign_turn_agent(
        &mut self,
        session_id: u64,
        turn_id: u64,
        agent_app_id: u64,
        agent_name: impl Into<String>,
    ) {
        let agent_name = agent_name.into();
        if let Some(turn) = self.turn_mut(session_id, turn_id) {
            turn.agent_app_id = Some(agent_app_id);
            turn.agent_name = Some(agent_name);
        }
        if let Some(session) = self.session_mut(session_id) {
            session.last_agent_app_id = Some(agent_app_id);
            session.updated_at = unix_timestamp();
        }
    }

    pub fn pause_turn_for_async_tool(
        &mut self,
        session_id: u64,
        turn_id: u64,
        pending_tool: PendingAsyncToolState,
    ) {
        if let Some(turn) = self.turn_mut(session_id, turn_id) {
            turn.status = TurnStatus::WaitingAsyncTool;
            turn.response = pending_tool.pending_message.clone();
            turn.partial_response.clear();
            turn.strategy = format!("Waiting for async tool {}.", pending_tool.tool_name);
            turn.pending_async_tool = Some(pending_tool);
        }
        if let Some(session) = self.session_mut(session_id) {
            session.running_turn_id = None;
            session.updated_at = unix_timestamp();
        }
        self.runtime_status = "Waiting for async tool".into();
    }

    pub fn complete_turn(&mut self, session_id: u64) {
        if let Some(session) = self.session_mut(session_id) {
            session.running_turn_id = None;
            session.updated_at = unix_timestamp();
        }
        if self.running_session_ids().is_empty() {
            self.runtime_status = "Ready".into();
        }
    }
}
