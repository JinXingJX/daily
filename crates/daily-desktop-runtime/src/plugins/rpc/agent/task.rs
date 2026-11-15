//! `agent.task.*`: structured plugin tasks with checkpoints, progress streaming and sub-agents.
use super::*;

pub(crate) async fn plugin_agent_task_start(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
    progress: Option<PluginProgressSink>,
) -> Result<Value, String> {
    let input = serde_json::from_value::<PluginAgentTaskStartInput>(payload)
        .map_err(|error| format!("Failed to parse agent.task.start payload: {error}"))?;
    validate_plugin_agent_task_input(&input)?;

    let (http, model) = plugin_task_model(state, &input)?;
    let (link, defined_agents) = match input.session_id.as_deref() {
        Some(session_id) => {
            let (link, agents) = begin_session_run(state, plugin_id, session_id, &input)?;
            (Some(link), agents)
        }
        None => (None, Vec::new()),
    };
    let claim = link
        .as_ref()
        .and_then(|link| SessionClaim::current(state, plugin_id, link));
    // A failed first request is kept as a checkpoint so the plugin can retry the same turn.
    let checkpoint = PendingPluginAgentTaskRun {
        runtime_run_id: None,
        run_id: format!("plugin-agent-task-{}", unique_stamp()),
        plugin_id: plugin_id.to_string(),
        task: input,
        task_id: format!("plugin-task-{}", unique_stamp()),
        pending_tool: None,
        observations: Vec::new(),
        next_ply: 1,
        failed_resume: None,
        last_error: None,
        session: link,
        defined_agents,
        sub_agent: None,
        created_at: unix_timestamp(),
    };
    let mut outcome = until_stopped(claim.as_ref(), async {
        let (conversation, compaction_view) = match checkpoint.session.as_ref() {
            Some(link) => {
                prepare_session_conversation(state, plugin_id, link, &http, &model).await?
            }
            None => (Vec::new(), None),
        };
        let task = build_plugin_agent_task(
            state,
            &checkpoint,
            model.clone(),
            Vec::new(),
            conversation,
            1,
            None,
        )?;
        let views = compaction_view.into_iter().collect();
        run_plugin_agent_task_flow(http.clone(), task, &checkpoint.task_id, views, progress).await
    })
    .await;
    retain_interrupted_checkpoint(state, &checkpoint, &mut outcome).await;
    let persist_on_failure = checkpoint.session.is_some();
    complete_plugin_agent_task(
        state,
        TaskSegment {
            checkpoint,
            persist_on_failure,
            continues_run: false,
            base_iterations: 0,
            resumed_calls: Vec::new(),
        },
        outcome,
    )
}

pub(crate) async fn plugin_agent_task_resume(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
    progress: Option<PluginProgressSink>,
) -> Result<Value, String> {
    let input = serde_json::from_value::<PluginAgentTaskResumeInput>(payload)
        .map_err(|error| format!("Failed to parse agent.task.resume payload: {error}"))?;
    let pending = load_task_run(state, plugin_id, &input.run_id)?;
    resume_task_run(state, plugin_id, pending, input, progress).await
}

/// Continues a task from its checkpoint: replays a failed resume, re-sends a
/// failed model request, or hands back a tool call the plugin never answered.
pub(crate) async fn plugin_agent_task_retry(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
    progress: Option<PluginProgressSink>,
) -> Result<Value, String> {
    let input = serde_json::from_value::<PluginAgentTaskRetryInput>(payload)
        .map_err(|error| format!("Failed to parse agent.task.retry payload: {error}"))?;
    let pending = load_task_run(state, plugin_id, &input.run_id)?;
    if let Some(resume) = pending.failed_resume.clone() {
        return resume_task_run(state, plugin_id, pending, resume, progress).await;
    }
    if let Some(tool) = pending.pending_tool.clone() {
        if let Some(link) = pending.session.as_ref() {
            reopen_waiting_run(state, plugin_id, link, &pending.run_id, &tool)?;
        }
        let (max_iterations, limit_notice) = effective_max_iterations(&pending.task);
        let response = plugin_agent_task_pending_response(&pending, Vec::new());
        return Ok(with_task_response_fields(
            response,
            tool.ply,
            max_iterations,
            limit_notice,
            pending.session.as_ref(),
        ));
    }

    let (http, model) = plugin_task_model(state, &pending.task)?;
    if let Some(link) = pending.session.as_ref() {
        resume_session_run(state, plugin_id, link, &pending.run_id)?;
    }
    let claim = pending
        .session
        .as_ref()
        .and_then(|link| SessionClaim::current(state, plugin_id, link));
    let start_ply = pending.next_ply.max(1);
    let task_id = pending.task_id();
    let mut outcome = until_stopped(claim.as_ref(), async {
        let conversation = session_replay(state, plugin_id, pending.session.as_ref())?;
        let task = build_plugin_agent_task(
            state,
            &pending,
            model,
            pending.observations.clone(),
            conversation,
            start_ply,
            None,
        )?;
        run_plugin_agent_task_flow(http, task, &task_id, Vec::new(), progress).await
    })
    .await;
    retain_interrupted_checkpoint(state, &pending, &mut outcome).await;
    complete_plugin_agent_task(
        state,
        TaskSegment {
            checkpoint: PendingPluginAgentTaskRun { task_id, ..pending },
            persist_on_failure: true,
            continues_run: true,
            base_iterations: start_ply - 1,
            resumed_calls: Vec::new(),
        },
        outcome,
    )
}

pub(super) fn load_task_run(
    state: &AgentStore,
    plugin_id: &str,
    run_id: &str,
) -> Result<PendingPluginAgentTaskRun, String> {
    let pending = AGENT_TASK_RUNS.load::<PendingPluginAgentTaskRun>(&state.project_root, run_id)?;
    if pending.plugin_id != plugin_id {
        return Err("Pending agent task belongs to a different plugin.".into());
    }
    Ok(pending)
}

pub(super) fn session_replay(
    state: &AgentStore,
    plugin_id: &str,
    link: Option<&PluginSessionLink>,
) -> Result<Vec<ModelMessage>, String> {
    match link {
        Some(link) => with_sessions(state, plugin_id, |store, _| {
            let session = store.load(&link.session_id)?;
            Ok(session_conversation(&session, &link.excluded()))
        }),
        None => Ok(Vec::new()),
    }
}

pub(super) async fn resume_task_run(
    state: &AgentStore,
    plugin_id: &str,
    pending: PendingPluginAgentTaskRun,
    input: PluginAgentTaskResumeInput,
    progress: Option<PluginProgressSink>,
) -> Result<Value, String> {
    let Some(tool) = pending.pending_tool.clone() else {
        return Err(
            "This agent task is not waiting for a tool result; continue it with agent.task.retry."
                .into(),
        );
    };
    let observation_text = match input.observation.as_deref() {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        _ => plugin_resume_observation(input.ok, &input.result)?,
    };
    let observation = AgentToolObservation::resumed(
        tool.tool_id.clone(),
        tool.tool_name.clone(),
        tool.rationale.clone(),
        tool.input.clone(),
        tool.pending_message.clone(),
        observation_text,
        json!({
            "ok": input.ok,
            "result": input.result,
        }),
    )
    .with_call(tool.call.clone());
    let observation = if input.ok {
        observation
    } else {
        observation.failed()
    };
    let task_id = pending.task_id();
    // A sub-agent's tool result goes back to that sub-agent; the session records only the parent's calls.
    let runtime_resume =
        tool.call.as_ref().map(
            |call| daily_agent_core::checkpoint::ResumeAction::ToolResult {
                call_id: call.call_id.clone(),
                result: daily_agent_core::AgentToolResult::with_metadata(
                    observation.observation.clone(),
                    observation.metadata.clone(),
                ),
                is_error: !input.ok,
            },
        );
    let (resumed_calls, resumed_views, observations, sub_agent, next_ply) =
        match pending.sub_agent.clone() {
            Some(mut frame) => {
                let views = sub_agent_views(
                    resumed_tool_views(
                        &sub_agent_task_id(&task_id, &frame),
                        &tool,
                        &observation.observation,
                        input.ok,
                    ),
                    &frame,
                    &pending.defined_agents,
                    &pending.task.sub_agents.agents,
                );
                frame.resume_observation(observation, tool.ply + 1);
                let parent_ply = frame.parent_ply;
                (
                    Vec::new(),
                    views,
                    pending.observations.clone(),
                    Some(frame),
                    parent_ply,
                )
            }
            None => {
                let resumed_call =
                    SessionToolCall::from_observation(tool.ply, input.ok, &observation);
                let views = resumed_tool_views(&task_id, &tool, &observation.observation, input.ok);
                let mut observations = pending.observations.clone();
                observations.push(observation);
                (vec![resumed_call], views, observations, None, tool.ply + 1)
            }
        };

    let (http, model) = plugin_task_model(state, &pending.task)?;
    if let Some(link) = pending.session.as_ref() {
        resume_session_run(state, plugin_id, link, &pending.run_id)?;
    }
    let claim = pending
        .session
        .as_ref()
        .and_then(|link| SessionClaim::current(state, plugin_id, link));
    let mut outcome = until_stopped(claim.as_ref(), async {
        let conversation = session_replay(state, plugin_id, pending.session.as_ref())?;
        let mut task = build_plugin_agent_task(
            state,
            &pending,
            model,
            observations,
            conversation,
            next_ply,
            sub_agent,
        )?;
        if task.checkpoint.is_some() || task.checkpoint_ref.is_some() {
            if let Some(action) = runtime_resume.clone() {
                task.resume_action = action;
            }
        }
        run_plugin_agent_task_flow(http, task, &task_id, resumed_views, progress).await
    })
    .await;
    retain_interrupted_checkpoint(state, &pending, &mut outcome).await;
    complete_plugin_agent_task(
        state,
        TaskSegment {
            checkpoint: PendingPluginAgentTaskRun {
                task_id,
                failed_resume: Some(input),
                ..pending
            },
            persist_on_failure: true,
            continues_run: true,
            base_iterations: next_ply.saturating_sub(1),
            resumed_calls,
        },
        outcome,
    )
}

/// The waiting step of `tool`, completed with the result the plugin delivered.
pub(super) fn resumed_tool_views(
    task_id: &str,
    tool: &PendingPluginAgentTool,
    observation: &str,
    ok: bool,
) -> Vec<agent::AgentEventView> {
    let status = if ok {
        agent::AgentEventViewStatus::Completed
    } else {
        agent::AgentEventViewStatus::Failed
    };
    vec![
        agent::AgentEventView::new(
            plugin_task_tool_view_id(task_id, tool.ply, &tool.tool_id),
            agent::AgentEventViewKind::ToolCall,
            status,
            if ok {
                format!("Tool done: {}", tool.tool_name)
            } else {
                format!("Tool failed: {}", tool.tool_name)
            },
            serde_json::to_string_pretty(&tool.input).unwrap_or_else(|_| tool.input.to_string()),
        )
        .with_ply(tool.ply)
        .with_tool(tool.tool_id.clone(), tool.tool_name.clone())
        .with_metadata(json!({ "rationale": tool.rationale })),
        agent::AgentEventView::new(
            plugin_task_tool_result_view_id(task_id, tool.ply, &tool.tool_id),
            agent::AgentEventViewKind::ToolResult,
            status,
            format!("Tool result: {}", tool.tool_name),
            observation,
        )
        .with_ply(tool.ply)
        .with_tool(tool.tool_id.clone(), tool.tool_name.clone()),
    ]
}

pub(crate) fn plugin_agent_task_cancel(
    state: &AgentStore,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let run_id = payload
        .get("runId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "agent.task.cancel requires runId.".to_string())?;
    let project_root = state.project_root.clone();
    let Ok(pending) = AGENT_TASK_RUNS.load::<PendingPluginAgentTaskRun>(&project_root, run_id)
    else {
        return Ok(json!({ "cancelled": false }));
    };
    if pending.plugin_id != plugin_id {
        return Err("Pending agent task belongs to a different plugin.".into());
    }
    AGENT_TASK_RUNS.remove(&project_root, run_id)?;
    if let Some(link) = pending.session.as_ref() {
        with_sessions(state, plugin_id, |store, _| {
            let mut session = store.load(&link.session_id)?;
            let run = session.run_mut(&link.entry_id)?;
            // A failed run stays failed; dropping its checkpoint only ends "continue".
            if run.status != SessionRunStatus::Failed {
                run.status = SessionRunStatus::Cancelled;
            }
            run.run_id = None;
            run.pending_message = None;
            run.pending_tool_id = None;
            if session.active_entry_id.as_deref() == Some(link.entry_id.as_str()) {
                session.active_entry_id = None;
            }
            store.save(&mut session)
        })?;
    }
    Ok(json!({ "cancelled": true }))
}

/// The iteration cap actually applied to a task, and a notice when the plugin asked for more.
pub(crate) fn effective_max_iterations(
    input: &PluginAgentTaskStartInput,
) -> (usize, Option<String>) {
    let requested = input
        .max_iterations
        .unwrap_or(PLUGIN_AGENT_DEFAULT_ITERATIONS);
    let effective = requested.clamp(1, PLUGIN_AGENT_MAX_ITERATIONS);
    let notice = (requested > PLUGIN_AGENT_MAX_ITERATIONS).then(|| {
        format!("maxIterations {requested} exceeds the host limit; using {PLUGIN_AGENT_MAX_ITERATIONS}.")
    });
    (effective, notice)
}

pub(super) fn plugin_task_model(
    state: &AgentStore,
    task: &PluginAgentTaskStartInput,
) -> Result<(Client, ModelConfig), String> {
    let app_state = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?;
    let agent_app = task.agent_app_id.and_then(|agent_id| {
        app_state
            .agent_apps
            .iter()
            .find(|agent| agent.id == agent_id)
    });
    let mut model = effective_model_config(&app_state.model_config, agent_app);
    if let Some(selection) = &task.model {
        apply_model_selection(&mut model, selection)?;
    }
    Ok((state.http.clone(), model))
}

pub(crate) fn build_plugin_agent_task(
    state: &AgentStore,
    run: &PendingPluginAgentTaskRun,
    model_config: ModelConfig,
    observations: Vec<AgentToolObservation>,
    conversation: Vec<ModelMessage>,
    start_ply: usize,
    resume_sub_agent: Option<SubAgentFrame>,
) -> Result<AgentTask, String> {
    let input = &run.task;
    let max_output_tokens = input
        .max_output_tokens
        .unwrap_or(model_config.max_output_tokens)
        .clamp(1, crate::limits::MAX_OUTPUT_TOKENS);
    let temperature = input
        .temperature
        .unwrap_or(model_config.temperature)
        .clamp(0.0, 2.0);
    let mut execution = daily_agent_core::RunContext::new_with_store(
        "plugin.agent_task",
        input.run_limits.clone().unwrap_or_default(),
        state.agent_checkpoints.clone(),
    );
    execution.identity.run_id = run
        .runtime_run_id
        .clone()
        .unwrap_or_else(|| run.run_id.clone());
    Ok(AgentTask {
        checkpoint_ref: run.runtime_run_id.clone(),
        execution: Some(execution),
        checkpoint: None,
        resume_action: daily_agent_core::checkpoint::ResumeAction::Continue,
        agent_definitions: input
            .definitions
            .iter()
            .map(|d| (d.spec.id.clone(), d.clone()))
            .collect(),
        model_bindings: {
            let mut bindings = std::collections::BTreeMap::new();
            for (key, selection) in &input.model_bindings {
                let mut config = model_config.clone();
                apply_model_selection(&mut config, selection)?;
                let model: Arc<dyn daily_agent_core::model::Model> =
                    Arc::new(daily_agent_providers::ProviderModel::new(
                        state.http.clone(),
                        config.provider_config(),
                    ));
                bindings.insert(key.clone(), model);
            }
            bindings
        },
        name: "plugin.agent_task".into(),
        session_id: None,
        turn_id: None,
        model: model_config,
        model_adapter: None,
        system_prompt: input.system_prompt.clone(),
        user_prompt: input.user_prompt.clone(),
        final_instructions: input.final_instructions.clone(),
        context: input
            .context
            .iter()
            .map(|item| AgentContextBlock::new(item.title.clone(), item.content.clone()))
            .collect(),
        conversation,
        prior_tool_observations: observations,
        tools: input
            .tools
            .clone()
            .into_iter()
            .map(agent_tool_from_dynamic)
            .collect(),
        max_iterations: effective_max_iterations(input).0,
        start_ply,
        observation_budget: Some(ObservationBudget {
            max_tokens: PLUGIN_RUN_OBSERVATION_BUDGET_TOKENS,
            keep_recent: PLUGIN_RUN_KEEP_RECENT_OBSERVATIONS,
        }),
        max_output_tokens,
        temperature,
        inline_images: Vec::new(),
        stream: true,
        require_tool_before_final: input.require_tool_before_final,
        continue_on_tool_error: input.continue_on_tool_error,
        sub_agents: SubAgentConfig {
            allow_create: input.sub_agents.allow_create,
            agents: run_agents(run),
        },
        allow_handoff: input.allow_handoff,
        resume_sub_agent,
    })
}
/// The plugin's agents, then those the model created (which replace a plugin agent of the same id).
pub(super) fn run_agents(run: &PendingPluginAgentTaskRun) -> Vec<AgentToolSpec> {
    let tool_ids = run
        .task
        .tools
        .iter()
        .map(|tool| tool.id.clone())
        .collect::<Vec<_>>();
    let mut agents = run.task.sub_agents.agents.clone();
    for spec in &run.defined_agents {
        // An agent saved in the session may name tools this task no longer offers.
        let mut spec = spec.clone();
        spec.tools.retain(|tool| tool_ids.contains(tool));
        upsert_agent_tool_spec(&mut agents, spec);
    }
    agents
}

/// Event view ids of a sub-agent's steps, scoped under the parent call.
pub(crate) fn sub_agent_task_id(task_id: &str, frame: &SubAgentFrame) -> String {
    format!("{task_id}-ply-{}-sub-{}", frame.parent_ply, frame.agent_id)
}

/// Tags views produced for a sub-agent's steps so the plugin can show them under the parent call.
pub(super) fn sub_agent_views(
    views: Vec<agent::AgentEventView>,
    frame: &SubAgentFrame,
    defined: &[AgentToolSpec],
    declared: &[AgentToolSpec],
) -> Vec<agent::AgentEventView> {
    let name = defined
        .iter()
        .chain(declared)
        .find(|spec| spec.id == frame.agent_id)
        .map(|spec| spec.name.clone())
        .unwrap_or_else(|| frame.agent_id.clone());
    views
        .into_iter()
        .map(|view| {
            let tag = json!({ "subAgent": { "id": frame.agent_id, "name": name, "parentPly": frame.parent_ply } });
            let metadata = merge_metadata(&view.metadata, tag);
            view.with_metadata(metadata)
        })
        .collect()
}

/// One task segment (start, resume or retry) about to be recorded.
pub(super) struct TaskSegment {
    /// The run state to keep when this segment fails.
    pub(super) checkpoint: PendingPluginAgentTaskRun,
    pub(super) persist_on_failure: bool,
    /// The segment continued `checkpoint.run_id`, whose file is replaced on success.
    pub(super) continues_run: bool,
    /// Plies already used before this segment.
    pub(super) base_iterations: usize,
    /// Plugin tool results this segment delivered.
    pub(super) resumed_calls: Vec<SessionToolCall>,
}

/// Persists the outcome of one task segment and builds the RPC response.
pub(super) fn complete_plugin_agent_task(
    state: &AgentStore,
    segment: TaskSegment,
    outcome: Result<PluginAgentTaskFlow, PluginAgentTaskFlowError>,
) -> Result<Value, String> {
    let TaskSegment {
        mut checkpoint,
        persist_on_failure,
        continues_run,
        base_iterations,
        resumed_calls,
    } = segment;
    let plugin_id = checkpoint.plugin_id.clone();
    let plugin_id = plugin_id.as_str();
    let link = checkpoint.session.clone();
    let (max_iterations, limit_notice) = effective_max_iterations(&checkpoint.task);
    let flow = match outcome {
        Ok(flow) => flow,
        Err(failure) => {
            checkpoint.last_error = Some(failure.error.clone());
            if let Some(runtime) = failure.runtime_checkpoint {
                if !matches!(
                    runtime.phase,
                    daily_agent_core::checkpoint::CheckpointPhase::WaitingInput
                ) {
                    checkpoint.pending_tool = None;
                    checkpoint.failed_resume = None;
                }
                checkpoint.runtime_run_id = Some(runtime.run.run_id);
            }
            let kept = persist_on_failure
                && AGENT_TASK_RUNS
                    .persist(&state.project_root, &checkpoint.run_id, &checkpoint)
                    .is_ok();
            if let Some(link) = link.as_ref() {
                record_session_run(state, plugin_id, link, |run, entry_content| {
                    merge_event_views(&mut run.event_views, &failure.event_views);
                    run.error = Some(failure.error.clone());
                    run.status = SessionRunStatus::Failed;
                    // A kept checkpoint lets the plugin continue this run with agent.task.retry.
                    run.run_id = kept.then(|| checkpoint.run_id.clone());
                    run.pending_message = None;
                    run.pending_tool_id = None;
                    run.iterations_used = run.iterations_used.max(failure.iterations_used);
                    if entry_content.is_empty() {
                        *entry_content = format!("Task failed: {}", failure.error);
                    }
                    None
                })?;
            }
            return Err(failure.error);
        }
    };
    if continues_run {
        AGENT_TASK_RUNS.remove(&state.project_root, &checkpoint.run_id)?;
    }
    if !flow.defined_agents.is_empty() {
        for spec in &flow.defined_agents {
            upsert_agent_tool_spec(&mut checkpoint.defined_agents, spec.clone());
        }
        if let Some(link) = link.as_ref() {
            save_session_agents(state, plugin_id, link, &flow.defined_agents)?;
        }
    }
    let iterations_used = flow.iterations_used.max(base_iterations);
    let mut tool_calls = resumed_calls;
    tool_calls.extend(flow.tool_calls);

    let response = match flow.result {
        PluginAgentTaskFlowResult::Completed { text, value } => {
            if let Some(link) = link.as_ref() {
                record_session_run(state, plugin_id, link, |run, entry_content| {
                    run.tool_calls.extend(tool_calls);
                    merge_event_views(&mut run.event_views, &flow.event_views);
                    run.status = SessionRunStatus::Completed;
                    run.run_id = None;
                    run.pending_message = None;
                    run.pending_tool_id = None;
                    run.error = None;
                    run.final_value = value.clone();
                    run.iterations_used = iterations_used;
                    *entry_content = text.clone();
                    None
                })?;
            }
            json!({
                "status": "completed",
                "text": text,
                "value": value,
                "eventViews": flow.event_views,
            })
        }
        PluginAgentTaskFlowResult::Paused {
            pending_tool,
            sub_agent,
        } => {
            let next_ply = match sub_agent.as_ref() {
                Some(frame) => frame.parent_ply,
                None => pending_tool.ply + 1,
            };
            let pending = PendingPluginAgentTaskRun {
                runtime_run_id: flow
                    .runtime_checkpoint
                    .as_ref()
                    .map(|cp| cp.run.run_id.clone()),
                run_id: format!("plugin-agent-task-{}", unique_stamp()),
                plugin_id: plugin_id.to_string(),
                task: checkpoint.task,
                task_id: checkpoint.task_id,
                observations: pending_tool.observations.clone(),
                next_ply,
                pending_tool: Some(pending_tool),
                failed_resume: None,
                last_error: None,
                session: link.clone(),
                defined_agents: checkpoint.defined_agents,
                sub_agent,
                created_at: unix_timestamp(),
            };
            if let Err(error) =
                AGENT_TASK_RUNS.persist(&state.project_root, &pending.run_id, &pending)
            {
                if let Some(link) = link.as_ref() {
                    record_session_run(state, plugin_id, link, |run, _| {
                        run.status = SessionRunStatus::Failed;
                        run.run_id = None;
                        run.error = Some(error.clone());
                        None
                    })?;
                }
                return Err(error);
            }
            let tool = pending
                .pending_tool
                .as_ref()
                .expect("paused run has a tool");
            if let Some(link) = link.as_ref() {
                record_session_run(state, plugin_id, link, |run, _| {
                    run.tool_calls.extend(tool_calls);
                    merge_event_views(&mut run.event_views, &flow.event_views);
                    run.status = SessionRunStatus::Waiting;
                    run.run_id = Some(pending.run_id.clone());
                    run.pending_message = Some(tool.pending_message.clone());
                    run.pending_tool_id = Some(tool.tool_id.clone());
                    run.error = None;
                    run.iterations_used = iterations_used;
                    Some(link.entry_id.clone())
                })?;
            }
            plugin_agent_task_pending_response(&pending, flow.event_views)
        }
    };
    Ok(with_task_response_fields(
        response,
        iterations_used,
        max_iterations,
        limit_notice,
        link.as_ref(),
    ))
}

pub(super) fn with_task_response_fields(
    mut response: Value,
    iterations_used: usize,
    max_iterations: usize,
    limit_notice: Option<String>,
    link: Option<&PluginSessionLink>,
) -> Value {
    if let Value::Object(map) = &mut response {
        map.insert("iterationsUsed".into(), json!(iterations_used));
        map.insert("maxIterations".into(), json!(max_iterations));
        if let Some(notice) = limit_notice {
            map.insert("limitNotice".into(), json!(notice));
        }
        if let Some(link) = link {
            map.insert("sessionId".into(), json!(link.session_id));
            map.insert("entryId".into(), json!(link.entry_id));
        }
    }
    response
}

pub(crate) struct PluginAgentTaskFlow {
    pub(crate) runtime_checkpoint: Option<daily_agent_core::checkpoint::RunCheckpoint>,
    pub(crate) result: PluginAgentTaskFlowResult,
    pub(crate) event_views: Vec<agent::AgentEventView>,
    pub(crate) iterations_used: usize,
    /// Tools the core executed itself (sync dynamic tools) during this segment.
    pub(crate) tool_calls: Vec<SessionToolCall>,
    /// Agents the model created during this segment.
    pub(crate) defined_agents: Vec<AgentToolSpec>,
}

/// A failed segment, with the steps it got through before failing.
pub(crate) struct PluginAgentTaskFlowError {
    pub(crate) runtime_checkpoint: Option<daily_agent_core::checkpoint::RunCheckpoint>,
    pub(crate) error: String,
    pub(crate) event_views: Vec<agent::AgentEventView>,
    pub(crate) iterations_used: usize,
}

impl From<String> for PluginAgentTaskFlowError {
    fn from(error: String) -> Self {
        Self {
            error,
            runtime_checkpoint: None,
            event_views: Vec::new(),
            iterations_used: 0,
        }
    }
}

/// Live step updates for one plugin request, sent as `plugin://agent-progress`.
/// Streamed text is coalesced so the frame rate stays low.
pub(crate) struct PluginProgressSink {
    emit: Arc<EventCallback>,
    plugin_id: String,
    stream_id: String,
    buffer: Vec<agent::AgentEventViewPatch>,
    last_flush: Instant,
}

pub(crate) const PLUGIN_AGENT_PROGRESS_EVENT: &str = "plugin://agent-progress";
pub(super) const PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

impl PluginProgressSink {
    pub(crate) fn new(emit: Arc<EventCallback>, plugin_id: &str, stream_id: &str) -> Self {
        Self {
            emit,
            plugin_id: plugin_id.to_string(),
            stream_id: stream_id.to_string(),
            buffer: Vec::new(),
            last_flush: Instant::now(),
        }
    }

    pub(super) fn push(&mut self, patches: Vec<agent::AgentEventViewPatch>) {
        let mut urgent = false;
        for patch in patches {
            if matches!(
                patch.merge,
                agent::AgentEventViewMergeStrategy::AppendContent
            ) {
                if let Some(last) = self.buffer.last_mut() {
                    if matches!(
                        last.merge,
                        agent::AgentEventViewMergeStrategy::AppendContent
                    ) && last.view.id == patch.view.id
                    {
                        last.view.content.push_str(&patch.view.content);
                        continue;
                    }
                }
            } else {
                urgent = true;
            }
            self.buffer.push(patch);
        }
        if urgent || self.last_flush.elapsed() >= PROGRESS_FLUSH_INTERVAL {
            self.flush();
        }
    }

    pub(super) fn flush(&mut self) {
        self.last_flush = Instant::now();
        if self.buffer.is_empty() {
            return;
        }
        let patches = std::mem::take(&mut self.buffer);
        // Progress is best effort: the RPC response still carries every view.
        let _ = (self.emit)(
            PLUGIN_AGENT_PROGRESS_EVENT,
            json!({
                "pluginId": self.plugin_id,
                "streamId": self.stream_id,
                "patches": patches,
            }),
        );
    }
}

pub(super) fn replace_patches(views: &[agent::AgentEventView]) -> Vec<agent::AgentEventViewPatch> {
    views
        .iter()
        .map(|view| {
            agent::AgentEventViewPatch::new(
                view.clone(),
                agent::AgentEventViewMergeStrategy::Replace,
            )
        })
        .collect()
}

/// Runs one segment of a plugin task. `initial_views` are reported before the
/// model runs (e.g. the tool result this segment resumes with).
pub(crate) async fn run_plugin_agent_task_flow(
    http: Client,
    task: AgentTask,
    task_id: &str,
    initial_views: Vec<agent::AgentEventView>,
    mut progress: Option<PluginProgressSink>,
) -> Result<PluginAgentTaskFlow, PluginAgentTaskFlowError> {
    let mut event_views = vec![agent::AgentEventView::user_input(
        0,
        task.user_prompt.clone(),
    )];
    event_views.extend(initial_views);
    if let Some(sink) = progress.as_mut() {
        sink.push(replace_patches(&event_views));
    }
    let mut iterations_used = 0;
    let mut current_ply = task.start_ply;
    let mut tool_calls = Vec::new();
    let mut defined_agents = Vec::new();
    let mut selected = std::collections::HashMap::<String, (String, Value)>::new();
    let prepared = task.prepare(&http).await;
    let execution = prepared.as_ref().ok().map(|task| task.execution());
    let mut on_event = |event| {
        let patches = apply_plugin_task_event_view(&mut event_views, task_id, &event);
        if let Some(sink) = progress.as_mut() {
            sink.push(patches);
        }
        match event {
            AgentTaskEvent::ModelRequestStarted { ply, .. } => {
                iterations_used = iterations_used.max(ply);
                current_ply = ply;
            }
            AgentTaskEvent::ToolSelected {
                tool_id,
                rationale,
                input,
                ..
            } => {
                selected.insert(tool_id, (rationale, input));
            }
            AgentTaskEvent::ToolResultProduced {
                ply,
                tool_id,
                tool_name,
                observation,
                metadata,
                ..
            } => {
                let (rationale, input) = selected.remove(&tool_id).unwrap_or_default();
                tool_calls.push(SessionToolCall {
                    ply,
                    ok: metadata.get("error").is_none(),
                    tool_id,
                    tool_name,
                    rationale,
                    input,
                    result: observation,
                });
            }
            AgentTaskEvent::AgentDefined { spec, .. } => {
                upsert_agent_tool_spec(&mut defined_agents, spec);
            }
            _ => {}
        }
        Ok(())
    };
    let result = match prepared {
        Ok(task) => task.run_until_pause(&mut on_event).await,
        Err(error) => Err(error),
    };
    let runtime_checkpoint = execution.as_ref().and_then(|ctx| ctx.checkpoint());

    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let error = error.to_string();
            let failed = mark_segment_failed(&mut event_views, task_id, current_ply, &error);
            if let Some(sink) = progress.as_mut() {
                sink.push(replace_patches(&failed));
                sink.flush();
            }
            return Err(PluginAgentTaskFlowError {
                runtime_checkpoint,
                error,
                event_views,
                iterations_used,
            });
        }
    };
    if let Some(sink) = progress.as_mut() {
        sink.flush();
    }
    let result = match result {
        AgentTaskRunResult::Completed(outcome) => PluginAgentTaskFlowResult::Completed {
            text: outcome.final_text,
            value: outcome.final_value,
        },
        AgentTaskRunResult::Paused(pause) => PluginAgentTaskFlowResult::Paused {
            pending_tool: PendingPluginAgentTool {
                ply: pause.ply,
                tool_id: pause.tool_id,
                tool_name: pause.tool_name,
                rationale: pause.rationale,
                input: pause.input,
                pending_message: pause.pending_message,
                metadata: pause.metadata,
                observations: pause.observations,
                call: pause.call,
            },
            sub_agent: pause.sub_agent,
        },
    };
    Ok(PluginAgentTaskFlow {
        runtime_checkpoint,
        result,
        event_views,
        iterations_used,
        tool_calls,
        defined_agents,
    })
}

/// Marks unfinished steps failed and adds the error step; returns the changed views.
pub(super) fn mark_segment_failed(
    event_views: &mut Vec<agent::AgentEventView>,
    task_id: &str,
    ply: usize,
    error: &str,
) -> Vec<agent::AgentEventView> {
    let mut changed = Vec::new();
    for view in event_views.iter_mut() {
        if view.status == agent::AgentEventViewStatus::InProgress {
            view.status = agent::AgentEventViewStatus::Failed;
            changed.push(view.clone());
        }
    }
    let view = agent::AgentEventView::new(
        format!("{task_id}-ply-{ply}-error"),
        agent::AgentEventViewKind::Error,
        agent::AgentEventViewStatus::Failed,
        "Request failed",
        error,
    )
    .with_ply(ply);
    upsert_plugin_task_event_view(event_views, view.clone(), false);
    changed.push(view);
    changed
}

pub(crate) fn plugin_agent_task_pending_response(
    pending: &PendingPluginAgentTaskRun,
    event_views: Vec<agent::AgentEventView>,
) -> Value {
    let tool = pending.pending_tool.as_ref();
    json!({
        "status": "tool_call",
        "runId": pending.run_id,
        "toolId": tool.map(|tool| tool.tool_id.clone()),
        "toolName": tool.map(|tool| tool.tool_name.clone()),
        "pendingMessage": tool.map(|tool| tool.pending_message.clone()),
        "input": tool.map(|tool| tool.input.clone()),
        "metadata": tool.map(|tool| tool.metadata.clone()),
        "subAgent": pending.sub_agent.as_ref().map(|frame| {
            let name = run_agents(pending)
                .into_iter()
                .find(|spec| spec.id == frame.agent_id)
                .map(|spec| spec.name)
                .unwrap_or_else(|| frame.agent_id.clone());
            json!({ "id": frame.agent_id, "name": name })
        }),
        "eventViews": event_views,
    })
}

// ---- Session bookkeeping -------------------------------------------------

async fn retain_interrupted_checkpoint(
    state: &AgentStore,
    pending: &PendingPluginAgentTaskRun,
    outcome: &mut Result<PluginAgentTaskFlow, PluginAgentTaskFlowError>,
) {
    use daily_agent_core::checkpoint::CheckpointStore;
    if let Err(failure) = outcome {
        if failure.runtime_checkpoint.is_none() {
            let id = pending.runtime_run_id.as_deref().unwrap_or(&pending.run_id);
            if let Ok(Some(checkpoint)) = state.agent_checkpoints.load(id).await {
                failure.runtime_checkpoint = Some(checkpoint);
            }
        }
    }
}
