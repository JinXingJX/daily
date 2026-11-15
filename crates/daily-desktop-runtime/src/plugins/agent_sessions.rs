//! Durable, per-plugin agent sessions.
//!
//! A session is the transcript a plugin's agent works in: author messages,
//! final replies, every tool call with its result, and the event views shown
//! while the task ran. Core replays it to the model on the next
//! `agent.task.start` so plugins don't rebuild history themselves.
//!
//! Layout: `.daily/plugin-agent-sessions/<pluginId>/<sessionId>.json` plus an
//! `index.json` per plugin. Callers serialize writes with
//! `AgentStore::plugin_agent_sessions`.

use crate::agent::compaction::{estimate_tokens, truncate_middle};
use crate::agent::sdk::AgentToolObservation;
use crate::agent::AgentEventView;
use crate::fs_utils::atomic_write;
use crate::fs_utils::sandbox::read_limited_text;
use crate::limits::{MAX_PLUGIN_AGENT_SESSIONS, MAX_PLUGIN_AGENT_SESSION_BYTES};
use crate::model::{ModelMessage, ModelMessageRole};
use crate::plugins::rpc::storage::sanitize_plugin_file_name;
use crate::state::sessions::summarize_title;
use crate::util::{unique_stamp, unix_timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SESSIONS_DIR_NAME: &str = "plugin-agent-sessions";
const INDEX_FILE_NAME: &str = "index.json";
const DEFAULT_TITLE: &str = "New conversation";
/// Tool inputs are replayed to the model in full up to this size.
const REPLAY_INPUT_CHARS: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionRunStatus {
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionToolCall {
    pub(crate) ply: usize,
    pub(crate) tool_id: String,
    pub(crate) tool_name: String,
    #[serde(default)]
    pub(crate) rationale: String,
    #[serde(default)]
    pub(crate) input: Value,
    #[serde(default)]
    pub(crate) ok: bool,
    #[serde(default)]
    pub(crate) result: String,
}

impl SessionToolCall {
    pub(crate) fn from_observation(
        ply: usize,
        ok: bool,
        observation: &AgentToolObservation,
    ) -> Self {
        Self {
            ply,
            tool_id: observation.tool_id.clone(),
            tool_name: observation.tool_name.clone(),
            rationale: observation.rationale.clone(),
            input: observation.input.clone(),
            ok,
            result: observation.observation.clone(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionRunRecord {
    /// The pending run to resume, while `status` is `waiting`.
    #[serde(default)]
    pub(crate) run_id: Option<String>,
    pub(crate) status: SessionRunStatus,
    #[serde(default)]
    pub(crate) tool_calls: Vec<SessionToolCall>,
    #[serde(default)]
    pub(crate) event_views: Vec<AgentEventView>,
    #[serde(default)]
    pub(crate) pending_message: Option<String>,
    #[serde(default)]
    pub(crate) pending_tool_id: Option<String>,
    #[serde(default)]
    pub(crate) final_value: Value,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) iterations_used: usize,
    #[serde(default)]
    pub(crate) max_iterations: usize,
}

impl SessionRunRecord {
    pub(crate) fn running(max_iterations: usize) -> Self {
        Self {
            run_id: None,
            status: SessionRunStatus::Running,
            tool_calls: Vec::new(),
            event_views: Vec::new(),
            pending_message: None,
            pending_tool_id: None,
            final_value: Value::Null,
            error: None,
            iterations_used: 0,
            max_iterations,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) idempotency_key: Option<String>,
    pub(crate) id: String,
    pub(crate) role: SessionRole,
    pub(crate) content: String,
    /// Plugin-owned data (e.g. message kind, linked proposal); core never reads it.
    #[serde(default)]
    pub(crate) meta: Value,
    pub(crate) created_at: u64,
    #[serde(default)]
    pub(crate) run: Option<SessionRunRecord>,
}

impl SessionEntry {
    pub(crate) fn new(role: SessionRole, content: impl Into<String>, meta: Value) -> Self {
        Self {
            idempotency_key: None,
            id: fresh_id("entry"),
            role,
            content: content.into(),
            meta,
            created_at: unix_timestamp(),
            run: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionSummary {
    pub(crate) content: String,
    /// Entries up to and including this one are covered by `content`.
    pub(crate) through_entry_id: String,
    pub(crate) created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAgentSession {
    pub(crate) id: String,
    pub(crate) plugin_id: String,
    #[serde(default)]
    pub(crate) scope: String,
    pub(crate) title: String,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
    /// The assistant entry whose run is executing or waiting on a tool.
    #[serde(default)]
    pub(crate) active_entry_id: Option<String>,
    #[serde(default)]
    pub(crate) summary: Option<SessionSummary>,
    /// Agents-as-tools created in this session; later tasks can call them again.
    #[serde(default)]
    pub(crate) agent_tools: Vec<crate::agent::sdk::AgentToolSpec>,
    #[serde(default)]
    pub(crate) entries: Vec<SessionEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionIndexEntry {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) scope: String,
    pub(crate) title: String,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
    #[serde(default)]
    pub(crate) entry_count: usize,
}

#[derive(Default, Serialize, Deserialize)]
struct SessionIndex {
    #[serde(default)]
    sessions: Vec<SessionIndexEntry>,
}

impl PluginAgentSession {
    pub(crate) fn entry_mut(&mut self, entry_id: &str) -> Result<&mut SessionEntry, String> {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("Session entry {entry_id} does not exist."))
    }

    pub(crate) fn run_mut(&mut self, entry_id: &str) -> Result<&mut SessionRunRecord, String> {
        self.entry_mut(entry_id)?
            .run
            .as_mut()
            .ok_or_else(|| format!("Session entry {entry_id} has no agent run."))
    }

    pub(crate) fn push(&mut self, entry: SessionEntry) {
        if self.entries.is_empty() && self.title == DEFAULT_TITLE && entry.role == SessionRole::User
        {
            self.title = summarize_title(&entry.content);
        }
        self.entries.push(entry);
    }

    fn index_entry(&self) -> SessionIndexEntry {
        SessionIndexEntry {
            id: self.id.clone(),
            scope: self.scope.clone(),
            title: self.title.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            entry_count: self.entries.len(),
        }
    }

    /// Entries the model has not seen in the summary yet.
    pub(crate) fn unsummarized_entries(&self) -> &[SessionEntry] {
        let start = self
            .summary
            .as_ref()
            .and_then(|summary| {
                self.entries
                    .iter()
                    .position(|entry| entry.id == summary.through_entry_id)
            })
            .map(|index| index + 1)
            .unwrap_or(0);
        &self.entries[start..]
    }
}

/// Sessions of one plugin.
pub(crate) struct SessionStore<'a> {
    root: &'a Path,
    plugin_id: &'a str,
}

impl<'a> SessionStore<'a> {
    pub(crate) fn new(root: &'a Path, plugin_id: &'a str) -> Self {
        Self { root, plugin_id }
    }

    fn dir(&self) -> PathBuf {
        self.root
            .join(".daily")
            .join(SESSIONS_DIR_NAME)
            .join(sanitize_plugin_file_name(self.plugin_id))
    }

    fn path(&self, session_id: &str) -> Result<PathBuf, String> {
        validate_session_id(session_id)?;
        Ok(self.dir().join(format!("{session_id}.json")))
    }

    pub(crate) fn create(
        &self,
        scope: &str,
        title: Option<&str>,
    ) -> Result<PluginAgentSession, String> {
        let index = self.read_index()?;
        if index.sessions.len() >= MAX_PLUGIN_AGENT_SESSIONS {
            return Err(format!(
                "A plugin can keep at most {MAX_PLUGIN_AGENT_SESSIONS} agent sessions; delete old ones first."
            ));
        }
        let now = unix_timestamp();
        let mut session = PluginAgentSession {
            id: fresh_id("session"),
            plugin_id: self.plugin_id.to_string(),
            scope: scope.to_string(),
            title: title
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_TITLE)
                .chars()
                .take(120)
                .collect(),
            created_at: now,
            updated_at: now,
            active_entry_id: None,
            summary: None,
            agent_tools: Vec::new(),
            entries: Vec::new(),
        };
        self.save(&mut session)?;
        Ok(session)
    }

    pub(crate) fn load(&self, session_id: &str) -> Result<PluginAgentSession, String> {
        let path = self.path(session_id)?;
        if !path.exists() {
            return Err(format!("Agent session {session_id} does not exist."));
        }
        let raw = read_limited_text(&path, MAX_PLUGIN_AGENT_SESSION_BYTES, "agent session")?;
        let session = serde_json::from_str::<PluginAgentSession>(&raw)
            .map_err(|error| format!("Failed to parse agent session {session_id}: {error}"))?;
        if session.plugin_id != self.plugin_id || session.id != session_id {
            return Err("Agent session belongs to a different plugin.".into());
        }
        Ok(session)
    }

    pub(crate) fn save(&self, session: &mut PluginAgentSession) -> Result<(), String> {
        if session.plugin_id != self.plugin_id {
            return Err("Agent session belongs to a different plugin.".into());
        }
        session.updated_at = unix_timestamp();
        let payload = fit_session_payload(session)?;
        let path = self.path(&session.id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create agent session directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        atomic_write(&path, format!("{payload}\n")).map_err(|error| {
            format!("Failed to write agent session {}: {error}", path.display())
        })?;
        let mut index = self.read_index()?;
        index.sessions.retain(|item| item.id != session.id);
        index.sessions.push(session.index_entry());
        self.write_index(&index)
    }

    pub(crate) fn delete(&self, session_id: &str) -> Result<(), String> {
        let path = self.path(session_id)?;
        if path.exists() {
            // Loading first enforces plugin ownership.
            self.load(session_id)?;
            fs::remove_file(&path).map_err(|error| {
                format!("Failed to remove agent session {}: {error}", path.display())
            })?;
        }
        let mut index = self.read_index()?;
        index.sessions.retain(|item| item.id != session_id);
        self.write_index(&index)
    }

    pub(crate) fn list(&self, scope: Option<&str>) -> Result<Vec<SessionIndexEntry>, String> {
        let mut sessions = self
            .read_index()?
            .sessions
            .into_iter()
            .filter(|item| scope.is_none_or(|scope| item.scope == scope))
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Ok(sessions)
    }

    fn read_index(&self) -> Result<SessionIndex, String> {
        let path = self.dir().join(INDEX_FILE_NAME);
        if !path.exists() {
            return Ok(SessionIndex::default());
        }
        let raw = read_limited_text(&path, MAX_PLUGIN_AGENT_SESSION_BYTES, "agent session index")?;
        serde_json::from_str(&raw)
            .map_err(|error| format!("Failed to parse agent session index: {error}"))
    }

    fn write_index(&self, index: &SessionIndex) -> Result<(), String> {
        let path = self.dir().join(INDEX_FILE_NAME);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create agent session directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let payload = serde_json::to_string(index)
            .map_err(|error| format!("Failed to serialize agent session index: {error}"))?;
        atomic_write(&path, payload)
            .map_err(|error| format!("Failed to write agent session index: {error}"))
    }
}

/// Clock stamps alone can repeat within one batch, so ids also carry a process-wide counter.
fn fresh_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}-{}-{}",
        unique_stamp(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || session_id.len() > 128
        || sanitize_plugin_file_name(session_id) != session_id
        || session_id.starts_with('.')
    {
        return Err("Invalid agent session id.".into());
    }
    Ok(())
}

/// Serializes the session, shedding the least useful data when it grows past
/// the size cap: event views of the oldest runs first, then old tool results.
fn fit_session_payload(session: &mut PluginAgentSession) -> Result<String, String> {
    let serialize = |session: &PluginAgentSession| {
        serde_json::to_string(session)
            .map_err(|error| format!("Failed to serialize agent session: {error}"))
    };
    let mut payload = serialize(session)?;
    let last = session.entries.len().saturating_sub(1);
    for index in 0..last {
        if payload.len() <= MAX_PLUGIN_AGENT_SESSION_BYTES {
            return Ok(payload);
        }
        if let Some(run) = session.entries[index].run.as_mut() {
            if !run.event_views.is_empty() {
                run.event_views.clear();
                payload = serialize(session)?;
            }
        }
    }
    for index in 0..last {
        if payload.len() <= MAX_PLUGIN_AGENT_SESSION_BYTES {
            return Ok(payload);
        }
        if let Some(run) = session.entries[index].run.as_mut() {
            for call in &mut run.tool_calls {
                call.result = truncate_middle(&call.result, 2_000);
            }
            payload = serialize(session)?;
        }
    }
    if payload.len() > MAX_PLUGIN_AGENT_SESSION_BYTES {
        return Err(format!(
            "Agent session exceeds the {MAX_PLUGIN_AGENT_SESSION_BYTES} byte limit; start a new session."
        ));
    }
    Ok(payload)
}

/// Renders one entry the way the model should see it on later turns.
pub(crate) fn render_entry(entry: &SessionEntry) -> ModelMessage {
    let role = match entry.role {
        SessionRole::User => ModelMessageRole::User,
        SessionRole::Assistant => ModelMessageRole::Assistant,
    };
    let Some(run) = entry.run.as_ref() else {
        return ModelMessage::text(role, entry.content.clone());
    };
    let mut content = String::new();
    if !run.tool_calls.is_empty() {
        content.push_str(&format!(
            "[Tool calls made in this turn: {}]\n",
            run.tool_calls.len()
        ));
        for (index, call) in run.tool_calls.iter().enumerate() {
            content.push_str(&format!(
                "{}. {} ({}){}\n   input: {}\n   result{}: {}\n",
                index + 1,
                call.tool_name,
                call.tool_id,
                if call.rationale.trim().is_empty() {
                    String::new()
                } else {
                    format!(" — {}", call.rationale.trim())
                },
                truncate_middle(&call.input.to_string(), REPLAY_INPUT_CHARS),
                if call.ok { "" } else { " (failed)" },
                call.result
            ));
        }
        content.push('\n');
    }
    match run.status {
        SessionRunStatus::Completed => {}
        SessionRunStatus::Failed => content.push_str(&format!(
            "[This turn failed: {}]\n",
            run.error.as_deref().unwrap_or("unknown error")
        )),
        SessionRunStatus::Cancelled => content.push_str("[The user cancelled this turn.]\n"),
        SessionRunStatus::Running | SessionRunStatus::Waiting => {
            content.push_str("[This turn did not finish.]\n")
        }
    }
    if !entry.content.trim().is_empty() {
        if !content.is_empty() {
            content.push_str("[Reply]\n");
        }
        content.push_str(&entry.content);
    }
    ModelMessage::text(role, content)
}

pub(crate) fn rendered_tokens(message: &ModelMessage) -> usize {
    estimate_tokens(&message.content)
}

/// Model conversation for the next turn: summary (if any) plus unsummarized entries.
pub(crate) fn session_conversation(
    session: &PluginAgentSession,
    exclude: &[&str],
) -> Vec<ModelMessage> {
    let mut messages = Vec::new();
    if let Some(summary) = session.summary.as_ref() {
        messages.push(ModelMessage::system(format!(
            "Summary of the earlier conversation in this session:\n{}",
            summary.content
        )));
    }
    messages.extend(
        session
            .unsummarized_entries()
            .iter()
            .filter(|entry| !exclude.contains(&entry.id.as_str()))
            .map(render_entry),
    );
    messages
}

/// Entries to fold into the summary so the replayed conversation fits
/// `budget_tokens` (estimated), keeping at least the last `keep_recent` entries verbatim.
/// Returns the ids of the entries to fold, oldest first.
pub(crate) fn plan_session_compaction(
    session: &PluginAgentSession,
    exclude: &[&str],
    budget_tokens: usize,
    keep_recent: usize,
) -> Vec<String> {
    let entries = session
        .unsummarized_entries()
        .iter()
        .filter(|entry| !exclude.contains(&entry.id.as_str()))
        .collect::<Vec<_>>();
    let sizes = entries
        .iter()
        .map(|entry| rendered_tokens(&render_entry(entry)))
        .collect::<Vec<_>>();
    let summary_tokens = session
        .summary
        .as_ref()
        .map(|summary| estimate_tokens(&summary.content))
        .unwrap_or(0);
    let mut total = summary_tokens + sizes.iter().sum::<usize>();
    let foldable = entries.len().saturating_sub(keep_recent);
    let mut fold = 0;
    while total > budget_tokens && fold < foldable {
        total -= sizes[fold];
        fold += 1;
    }
    entries[..fold]
        .iter()
        .map(|entry| entry.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("daily-agent-sessions-{}", unique_stamp()))
    }

    #[test]
    fn sessions_round_trip_and_stay_isolated_per_plugin() {
        let root = temp_root();
        let writer = SessionStore::new(&root, "daily.writer");
        let other = SessionStore::new(&root, "other.plugin");

        let mut session = writer.create("project-1", None).unwrap();
        session.push(SessionEntry::new(
            SessionRole::User,
            "帮我续写第三章",
            json!({ "kind": "text" }),
        ));
        writer.save(&mut session).unwrap();

        let loaded = writer.load(&session.id).unwrap();
        assert_eq!(loaded.title, "帮我续写第三章");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(writer.list(Some("project-1")).unwrap().len(), 1);
        assert!(writer.list(Some("project-2")).unwrap().is_empty());

        assert!(other.load(&session.id).is_err());
        assert!(other.list(None).unwrap().is_empty());
        assert!(writer.load("../escape").is_err());

        writer.delete(&session.id).unwrap();
        assert!(writer.load(&session.id).is_err());
        assert!(writer.list(None).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    fn run_entry(content: &str, result: &str) -> SessionEntry {
        let mut entry = SessionEntry::new(SessionRole::Assistant, content, Value::Null);
        let mut run = SessionRunRecord::running(8);
        run.status = SessionRunStatus::Completed;
        run.tool_calls.push(SessionToolCall {
            ply: 1,
            tool_id: "writer.readDocument".into(),
            tool_name: "readDocument".into(),
            rationale: "need the text".into(),
            input: json!({ "doc": "worldview" }),
            ok: true,
            result: result.into(),
        });
        entry.run = Some(run);
        entry
    }

    #[test]
    fn rendered_assistant_entries_include_tool_calls_and_results() {
        let message = render_entry(&run_entry("已完成", "世界观原文"));
        assert!(message.content.contains("readDocument"));
        assert!(message.content.contains("世界观原文"));
        assert!(message.content.contains("\"doc\":\"worldview\""));
        assert!(message.content.ends_with("已完成"));
    }

    #[test]
    fn compaction_plan_folds_oldest_entries_and_respects_summary_and_keep_recent() {
        let mut session = PluginAgentSession {
            id: "session-1".into(),
            plugin_id: "p".into(),
            scope: String::new(),
            title: "t".into(),
            created_at: 0,
            updated_at: 0,
            active_entry_id: None,
            summary: None,
            agent_tools: Vec::new(),
            entries: Vec::new(),
        };
        for index in 0..4 {
            session.push(SessionEntry::new(
                SessionRole::User,
                format!("ask {index}"),
                Value::Null,
            ));
            session.push(run_entry("ok", &"r".repeat(1_000)));
        }
        assert!(plan_session_compaction(&session, &[], 1_000_000, 2).is_empty());
        // Each run entry is about 270 estimated tokens.
        let fold = plan_session_compaction(&session, &[], 600, 2);
        assert!(!fold.is_empty());
        assert_eq!(fold[0], session.entries[0].id);
        assert!(fold.len() <= 6);

        session.summary = Some(SessionSummary {
            content: "earlier".into(),
            through_entry_id: session.entries[3].id.clone(),
            created_at: 0,
        });
        let conversation = session_conversation(&session, &[]);
        assert_eq!(conversation.len(), 5);
        assert!(conversation[0].content.contains("earlier"));
        let fold = plan_session_compaction(&session, &[], 10, 2);
        assert_eq!(
            fold,
            vec![session.entries[4].id.clone(), session.entries[5].id.clone()]
        );
    }

    #[test]
    fn oversized_sessions_shed_old_event_views_before_failing() {
        let mut session = PluginAgentSession {
            id: "session-2".into(),
            plugin_id: "p".into(),
            scope: String::new(),
            title: "t".into(),
            created_at: 0,
            updated_at: 0,
            active_entry_id: None,
            summary: None,
            agent_tools: Vec::new(),
            entries: Vec::new(),
        };
        let mut old = run_entry("old", "small");
        let view = AgentEventView::new(
            "v",
            crate::agent::AgentEventViewKind::ToolResult,
            crate::agent::AgentEventViewStatus::Completed,
            "t",
            "z".repeat(MAX_PLUGIN_AGENT_SESSION_BYTES),
        );
        old.run.as_mut().unwrap().event_views.push(view);
        session.entries.push(old);
        session.entries.push(run_entry("new", "small"));
        let payload = fit_session_payload(&mut session).unwrap();
        assert!(payload.len() <= MAX_PLUGIN_AGENT_SESSION_BYTES);
        assert!(session.entries[0]
            .run
            .as_ref()
            .unwrap()
            .event_views
            .is_empty());
        assert_eq!(session.entries[0].run.as_ref().unwrap().tool_calls.len(), 1);
    }
}
