//! Host-owned plugin agent sessions: in-flight claims, stop signals and run records.
use super::*;

/// Where a task's transcript lives inside a plugin agent session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginSessionLink {
    pub(crate) session_id: String,
    /// The author message that started the task.
    pub(crate) user_entry_id: String,
    /// The assistant entry holding the run record.
    pub(crate) entry_id: String,
}

impl PluginSessionLink {
    /// The current turn is sent as the user prompt, not replayed as history.
    pub(super) fn excluded(&self) -> [&str; 2] {
        [self.user_entry_id.as_str(), self.entry_id.as_str()]
    }

    pub(super) fn in_flight_key(&self, plugin_id: &str) -> String {
        session_in_flight_key(plugin_id, &self.session_id)
    }
}

pub(crate) fn session_in_flight_key(plugin_id: &str, session_id: &str) -> String {
    format!("{plugin_id}/{session_id}")
}

/// Sessions with a task segment executing, each with the signal that stops it.
pub(crate) type InFlightSessions = HashMap<String, Arc<Notify>>;

/// The failure recorded on a run the author stopped; the run keeps its checkpoint.
pub(crate) const PLUGIN_TASK_STOPPED: &str = "Stopped before it finished.";

/// A running segment's hold on its session. Dropping it releases the session
/// even when the segment never recorded its end (a panic, a dropped future).
pub(super) struct SessionClaim<'a> {
    pub(super) state: &'a AgentStore,
    pub(super) key: String,
    pub(super) stop: Arc<Notify>,
}

impl<'a> SessionClaim<'a> {
    /// The claim `begin_session_run` / `resume_session_run` just took for `link`.
    pub(super) fn current(
        state: &'a AgentStore,
        plugin_id: &str,
        link: &PluginSessionLink,
    ) -> Option<Self> {
        let key = link.in_flight_key(plugin_id);
        let stop = state.plugin_agent_sessions.lock().ok()?.get(&key)?.clone();
        Some(Self { state, key, stop })
    }
}

impl Drop for SessionClaim<'_> {
    fn drop(&mut self) {
        let mut in_flight = match self.state.plugin_agent_sessions.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // A later segment may already hold the session under a new signal.
        if in_flight
            .get(&self.key)
            .is_some_and(|stop| Arc::ptr_eq(stop, &self.stop))
        {
            in_flight.remove(&self.key);
        }
    }
}

/// Wakes the task segment running in `session_id`; returns whether one was running.
pub(crate) fn stop_session_task(
    in_flight: &InFlightSessions,
    plugin_id: &str,
    session_id: &str,
) -> bool {
    match in_flight.get(&session_in_flight_key(plugin_id, session_id)) {
        Some(stop) => {
            // `notify_one` keeps the wake-up if the segment is not waiting yet.
            stop.notify_one();
            true
        }
        None => false,
    }
}

/// Runs one task segment until it ends or the author stops the session's task.
pub(super) async fn until_stopped(
    claim: Option<&SessionClaim<'_>>,
    segment: impl Future<Output = Result<PluginAgentTaskFlow, PluginAgentTaskFlowError>>,
) -> Result<PluginAgentTaskFlow, PluginAgentTaskFlowError> {
    let Some(claim) = claim else {
        return segment.await;
    };
    tokio::select! {
        outcome = segment => outcome,
        () = claim.stop.notified() => Err(PLUGIN_TASK_STOPPED.to_string().into()),
    }
}

/// Runs `f` with the plugin's session store while holding the session write lock.
pub(crate) fn with_sessions<T>(
    state: &AgentStore,
    plugin_id: &str,
    f: impl FnOnce(&SessionStore<'_>, &mut InFlightSessions) -> Result<T, String>,
) -> Result<T, String> {
    let mut in_flight = state
        .plugin_agent_sessions
        .lock()
        .map_err(|_| "Agent sessions are unavailable.".to_string())?;
    f(
        &SessionStore::new(&state.project_root, plugin_id),
        &mut in_flight,
    )
}

/// Appends the author message and a running assistant entry, and claims the session.
pub(super) fn begin_session_run(
    state: &AgentStore,
    plugin_id: &str,
    session_id: &str,
    input: &PluginAgentTaskStartInput,
) -> Result<(PluginSessionLink, Vec<AgentToolSpec>), String> {
    let project_root = state.project_root.clone();
    with_sessions(state, plugin_id, |store, in_flight| {
        let mut session = store.load(session_id)?;
        let key = session_in_flight_key(plugin_id, session_id);
        if in_flight.contains_key(&key) {
            return Err("This agent session is already running a task.".into());
        }
        // A new message gives up on continuing earlier failed turns.
        for entry in session.entries.iter_mut() {
            if let Some(run) = entry.run.as_mut() {
                if run.status == SessionRunStatus::Failed {
                    if let Some(run_id) = run.run_id.take() {
                        AGENT_TASK_RUNS.remove(&project_root, &run_id)?;
                    }
                }
            }
        }
        if let Some(active) = session.active_entry_id.clone() {
            if let Ok(run) = session.run_mut(&active) {
                let waiting = run
                    .run_id
                    .as_deref()
                    .is_some_and(|run_id| AGENT_TASK_RUNS.path(&project_root, run_id).exists());
                if run.status == SessionRunStatus::Waiting && waiting {
                    return Err(
                        "This agent session is waiting for a tool result; resume or cancel that task first."
                            .into(),
                    );
                }
                if matches!(
                    run.status,
                    SessionRunStatus::Running | SessionRunStatus::Waiting
                ) {
                    run.status = SessionRunStatus::Failed;
                    run.error = Some("Interrupted before it finished.".into());
                    if let Some(run_id) = run.run_id.take() {
                        AGENT_TASK_RUNS.remove(&project_root, &run_id)?;
                    }
                }
            }
            session.active_entry_id = None;
        }
        let user = SessionEntry::new(
            SessionRole::User,
            input
                .display_message
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| input.user_prompt.clone()),
            input.message_meta.clone(),
        );
        let mut assistant = SessionEntry::new(SessionRole::Assistant, "", Value::Null);
        assistant.run = Some(SessionRunRecord::running(effective_max_iterations(input).0));
        let link = PluginSessionLink {
            session_id: session.id.clone(),
            user_entry_id: user.id.clone(),
            entry_id: assistant.id.clone(),
        };
        session.push(user);
        session.push(assistant);
        session.active_entry_id = Some(link.entry_id.clone());
        store.save(&mut session)?;
        in_flight.insert(key, Arc::default());
        Ok((link, session.agent_tools.clone()))
    })
}

/// Keeps the agents the model created in the session, for later tasks.
pub(super) fn save_session_agents(
    state: &AgentStore,
    plugin_id: &str,
    link: &PluginSessionLink,
    agents: &[AgentToolSpec],
) -> Result<(), String> {
    with_sessions(state, plugin_id, |store, _| {
        let mut session = store.load(&link.session_id)?;
        for spec in agents {
            upsert_agent_tool_spec(&mut session.agent_tools, spec.clone());
        }
        store.save(&mut session)
    })
}

pub(super) fn resume_session_run(
    state: &AgentStore,
    plugin_id: &str,
    link: &PluginSessionLink,
    run_id: &str,
) -> Result<(), String> {
    with_sessions(state, plugin_id, |store, in_flight| {
        let key = link.in_flight_key(plugin_id);
        if in_flight.contains_key(&key) {
            return Err("This agent session is already running a task.".into());
        }
        let mut session = store.load(&link.session_id)?;
        let entry = session.entry_mut(&link.entry_id)?;
        let run = entry
            .run
            .as_mut()
            .ok_or_else(|| "This agent session entry has no run.".to_string())?;
        if run.run_id.as_deref() != Some(run_id) {
            return Err("This agent task is no longer the session's active run.".into());
        }
        if run.status == SessionRunStatus::Failed {
            // Continuing a failed run: its failure note no longer applies.
            run.error = None;
            entry.content.clear();
        }
        run.status = SessionRunStatus::Running;
        session.active_entry_id = Some(link.entry_id.clone());
        store.save(&mut session)?;
        in_flight.insert(key, Arc::default());
        Ok(())
    })
}

/// Puts a run whose plugin tool never got its result back into the waiting state.
pub(super) fn reopen_waiting_run(
    state: &AgentStore,
    plugin_id: &str,
    link: &PluginSessionLink,
    run_id: &str,
    tool: &PendingPluginAgentTool,
) -> Result<(), String> {
    with_sessions(state, plugin_id, |store, in_flight| {
        if in_flight.contains_key(&link.in_flight_key(plugin_id)) {
            return Err("This agent session is already running a task.".into());
        }
        let mut session = store.load(&link.session_id)?;
        let entry = session.entry_mut(&link.entry_id)?;
        let run = entry
            .run
            .as_mut()
            .ok_or_else(|| "This agent session entry has no run.".to_string())?;
        if run.run_id.as_deref() != Some(run_id) {
            return Err("This agent task is no longer the session's active run.".into());
        }
        if run.status == SessionRunStatus::Failed {
            entry.content.clear();
        }
        run.status = SessionRunStatus::Waiting;
        run.error = None;
        run.pending_message = Some(tool.pending_message.clone());
        run.pending_tool_id = Some(tool.tool_id.clone());
        session.active_entry_id = Some(link.entry_id.clone());
        store.save(&mut session)
    })
}

/// Updates the run record of `link`. `update` returns the entry that stays
/// active (waiting), or `None` when the run is finished. Always releases the
/// in-flight claim.
pub(super) fn record_session_run(
    state: &AgentStore,
    plugin_id: &str,
    link: &PluginSessionLink,
    update: impl FnOnce(&mut SessionRunRecord, &mut String) -> Option<String>,
) -> Result<(), String> {
    with_sessions(state, plugin_id, |store, in_flight| {
        in_flight.remove(&link.in_flight_key(plugin_id));
        let mut session = store.load(&link.session_id)?;
        let entry = session.entry_mut(&link.entry_id)?;
        let mut content = std::mem::take(&mut entry.content);
        let run = entry
            .run
            .get_or_insert_with(|| SessionRunRecord::running(0));
        let active = update(run, &mut content);
        entry.content = content;
        session.active_entry_id = active;
        store.save(&mut session)
    })
}

pub(super) fn merge_event_views(
    target: &mut Vec<agent::AgentEventView>,
    views: &[agent::AgentEventView],
) {
    for view in views {
        match target.iter_mut().find(|item| item.id == view.id) {
            Some(existing) => *existing = view.clone(),
            None => target.push(view.clone()),
        }
    }
}

/// Builds the replayed conversation, folding old entries into an LLM summary
/// when the history outgrows the budget.
pub(super) async fn prepare_session_conversation(
    state: &AgentStore,
    plugin_id: &str,
    link: &PluginSessionLink,
    http: &Client,
    model: &ModelConfig,
) -> Result<(Vec<ModelMessage>, Option<agent::AgentEventView>), String> {
    let excluded = link.excluded();
    let session = with_sessions(state, plugin_id, |store, _| store.load(&link.session_id))?;
    let fold = plan_session_compaction(
        &session,
        &excluded,
        PLUGIN_SESSION_CONTEXT_BUDGET_CHARS,
        PLUGIN_SESSION_KEEP_RECENT_ENTRIES,
    );
    let Some(through_entry_id) = fold.last().cloned() else {
        return Ok((session_conversation(&session, &excluded), None));
    };
    let history = session
        .entries
        .iter()
        .filter(|entry| fold.contains(&entry.id))
        .map(|entry| {
            let message = render_entry(entry);
            let role = match entry.role {
                SessionRole::User => "User",
                SessionRole::Assistant => "Assistant",
            };
            format!("{role}:\n{}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let previous = session
        .summary
        .as_ref()
        .map(|summary| summary.content.clone());
    let view_id = format!("{}-compaction", link.entry_id);
    match compaction::summarize(http, model, previous.as_deref(), &history).await {
        Ok(content) => {
            let session = with_sessions(state, plugin_id, |store, _| {
                let mut session = store.load(&link.session_id)?;
                session.summary = Some(SessionSummary {
                    content,
                    through_entry_id,
                    created_at: unix_timestamp(),
                });
                store.save(&mut session)?;
                Ok(session)
            })?;
            let view = agent::AgentEventView::new(
                view_id,
                agent::AgentEventViewKind::Runtime,
                agent::AgentEventViewStatus::Completed,
                "Compacted context",
                format!(
                    "Folded {} earlier conversation entries into a summary.",
                    fold.len()
                ),
            );
            Ok((session_conversation(&session, &excluded), Some(view)))
        }
        Err(error) => {
            // Keep going without the summary: replay a truncated copy of the old turns.
            let mut conversation = Vec::new();
            if let Some(previous) = previous {
                conversation.push(ModelMessage {
                    role: ModelMessageRole::System,
                    content: format!(
                        "Summary of the earlier conversation in this session:\n{previous}"
                    ),
                });
            }
            conversation.push(ModelMessage {
                role: ModelMessageRole::System,
                content: format!(
                    "Earlier turns (truncated):\n{}",
                    compaction::truncate_middle(&history, PLUGIN_SESSION_CONTEXT_BUDGET_CHARS / 3)
                ),
            });
            conversation.extend(
                session
                    .unsummarized_entries()
                    .iter()
                    .filter(|entry| {
                        !fold.contains(&entry.id) && !excluded.contains(&entry.id.as_str())
                    })
                    .map(render_entry),
            );
            let view = agent::AgentEventView::new(
                view_id,
                agent::AgentEventViewKind::Error,
                agent::AgentEventViewStatus::Failed,
                "Context compaction failed",
                format!("Summary failed; truncated earlier entries instead: {error}"),
            );
            Ok((conversation, Some(view)))
        }
    }
}
