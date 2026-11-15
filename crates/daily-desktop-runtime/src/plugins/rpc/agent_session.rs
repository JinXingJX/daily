//! `agent.session.*`: plugin-facing CRUD over core-owned agent sessions.
//! Domain logic lives in `plugins::agent_sessions`; these handlers only parse,
//! check permissions and lock.

use crate::agent::AgentStore;
use crate::limits::MAX_PLUGIN_PROMPT_BYTES;
use crate::plugins::agent_runs::AGENT_TASK_RUNS;
use crate::plugins::agent_sessions::{SessionEntry, SessionRole};
use crate::plugins::rpc::agent::{
    session_in_flight_key, stop_session_task, validate_plugin_prompt_text, with_sessions,
};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_APPEND_MESSAGES: usize = 500;
const MAX_SCOPE_BYTES: usize = 256;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateInput {
    #[serde(default)]
    scope: String,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListInput {
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdInput {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppendMessage {
    #[serde(default)]
    idempotency_key: Option<String>,
    role: SessionRole,
    content: String,
    #[serde(default)]
    meta: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppendInput {
    session_id: String,
    messages: Vec<AppendMessage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateEntryInput {
    session_id: String,
    entry_id: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    meta: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveAgentToolInput {
    session_id: String,
    agent_id: String,
}

fn parse<T: for<'de> Deserialize<'de>>(payload: Value, method: &str) -> Result<T, String> {
    serde_json::from_value(payload)
        .map_err(|error| format!("Failed to parse {method} payload: {error}"))
}

pub(crate) fn plugin_agent_session_create(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: CreateInput = parse(payload, "agent.session.create")?;
    if input.scope.len() > MAX_SCOPE_BYTES {
        return Err(format!("Session scope exceeds {MAX_SCOPE_BYTES} bytes."));
    }
    let session = with_sessions(state, plugin_id, |store, _| {
        store.create(&input.scope, input.title.as_deref())
    })?;
    Ok(json!({ "session": session }))
}

pub(crate) fn plugin_agent_session_list(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: ListInput = parse(payload, "agent.session.list")?;
    let sessions = with_sessions(state, plugin_id, |store, _| {
        store.list(input.scope.as_deref())
    })?;
    Ok(json!({ "sessions": sessions }))
}

pub(crate) fn plugin_agent_session_get(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: SessionIdInput = parse(payload, "agent.session.get")?;
    let (session, running) = with_sessions(state, plugin_id, |store, in_flight| {
        let running = in_flight.contains_key(&session_in_flight_key(plugin_id, &input.session_id));
        Ok((store.load(&input.session_id)?, running))
    })?;
    let mut session = serde_json::to_value(session).map_err(|error| error.to_string())?;
    // Whether a task is executing in the session right now (not just recorded as running).
    session["running"] = json!(running);
    Ok(json!({ "session": session }))
}

/// Stops the task executing in a session. The run is recorded as failed with
/// its checkpoint kept, so the plugin can continue it with `agent.task.retry`.
pub(crate) fn plugin_agent_session_stop(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: SessionIdInput = parse(payload, "agent.session.stop")?;
    let stopping = with_sessions(state, plugin_id, |_, in_flight| {
        Ok(stop_session_task(in_flight, plugin_id, &input.session_id))
    })?;
    Ok(json!({ "stopping": stopping }))
}

pub(crate) fn plugin_agent_session_delete(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: SessionIdInput = parse(payload, "agent.session.delete")?;
    let project_root = state.project_root.clone();
    with_sessions(state, plugin_id, |store, in_flight| {
        if in_flight.contains_key(&session_in_flight_key(plugin_id, &input.session_id)) {
            return Err("This agent session is running a task; wait for it to finish.".into());
        }
        if let Ok(session) = store.load(&input.session_id) {
            let waiting = session
                .entries
                .iter()
                .filter_map(|entry| entry.run.as_ref()?.run_id.clone());
            for run_id in waiting {
                AGENT_TASK_RUNS.remove(&project_root, &run_id)?;
            }
        }
        store.delete(&input.session_id)
    })?;
    Ok(json!({ "deleted": true }))
}

pub(crate) fn plugin_agent_session_append_messages(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: AppendInput = parse(payload, "agent.session.appendMessages")?;
    if input.messages.is_empty() || input.messages.len() > MAX_APPEND_MESSAGES {
        return Err(format!(
            "agent.session.appendMessages accepts 1-{MAX_APPEND_MESSAGES} messages."
        ));
    }
    for message in &input.messages {
        if message
            .idempotency_key
            .as_ref()
            .is_some_and(|key| key.is_empty() || key.len() > 256)
        {
            return Err("Message idempotency key must contain 1-256 bytes.".into());
        }
        validate_plugin_prompt_text(
            &message.content,
            MAX_PLUGIN_PROMPT_BYTES,
            "agent.session.appendMessages content",
        )?;
    }
    let entries = with_sessions(state, plugin_id, |store, _| {
        let mut session = store.load(&input.session_id)?;
        let mut entries = Vec::new();
        for message in input.messages {
            if let Some(existing) = message.idempotency_key.as_ref().and_then(|key| {
                session
                    .entries
                    .iter()
                    .find(|entry| entry.idempotency_key.as_ref() == Some(key))
            }) {
                // An assistant placeholder may have been finalized since the initial append.
                if existing.role != message.role
                    || (existing.content != message.content
                        && !(message.role == SessionRole::Assistant && message.content.is_empty()))
                {
                    return Err(
                        "Message idempotency key was reused with different parameters.".into(),
                    );
                }
                entries.push(existing.clone());
            } else {
                let mut entry = SessionEntry::new(message.role, message.content, message.meta);
                entry.idempotency_key = message.idempotency_key;
                session.push(entry.clone());
                entries.push(entry);
            }
        }
        store.save(&mut session)?;
        Ok(entries)
    })?;
    Ok(json!({ "entries": entries }))
}

pub(crate) fn plugin_agent_session_update_entry(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: UpdateEntryInput = parse(payload, "agent.session.updateEntry")?;
    if let Some(content) = input.content.as_deref() {
        validate_plugin_prompt_text(
            content,
            MAX_PLUGIN_PROMPT_BYTES,
            "agent.session.updateEntry content",
        )?;
    }
    let entry = with_sessions(state, plugin_id, |store, _| {
        let mut session = store.load(&input.session_id)?;
        let entry = session.entry_mut(&input.entry_id)?;
        if let Some(content) = input.content {
            entry.content = content;
        }
        if let Some(meta) = input.meta {
            entry.meta = meta;
        }
        let entry = entry.clone();
        store.save(&mut session)?;
        Ok(entry)
    })?;
    Ok(json!({ "entry": entry }))
}

/// Drops an agent-as-tool the model created in this session.
pub(crate) fn plugin_agent_session_remove_agent_tool(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input: RemoveAgentToolInput = parse(payload, "agent.session.removeAgentTool")?;
    let removed = with_sessions(state, plugin_id, |store, _| {
        let mut session = store.load(&input.session_id)?;
        let before = session.agent_tools.len();
        session.agent_tools.retain(|spec| spec.id != input.agent_id);
        let removed = session.agent_tools.len() != before;
        if removed {
            store.save(&mut session)?;
        }
        Ok(removed)
    })?;
    Ok(json!({ "removed": removed }))
}
