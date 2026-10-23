use super::*;
use crate::runtime::DailyRuntime;

fn task_input(session_id: &str, max_iterations: Option<usize>) -> PluginAgentTaskStartInput {
    serde_json::from_value(json!({
        "systemPrompt": "s",
        "userPrompt": "【作者的要求】继续",
        "displayMessage": "继续",
        "messageMeta": { "kind": "text" },
        "sessionId": session_id,
        "maxIterations": max_iterations,
    }))
    .unwrap()
}

#[test]
fn iteration_cap_defaults_and_reports_clamping() {
    assert_eq!(effective_max_iterations(&task_input("s", None)), (8, None));
    assert_eq!(
        effective_max_iterations(&task_input("s", Some(1000))),
        (1000, None)
    );
    let (effective, notice) = effective_max_iterations(&task_input("s", Some(5000)));
    assert_eq!(effective, PLUGIN_AGENT_MAX_ITERATIONS);
    assert!(notice.unwrap().contains("5000"));
}

#[tokio::test]
async fn stopping_a_session_ends_its_running_segment() {
    let runtime = DailyRuntime::test_runtime();
    let state = runtime.store();
    let plugin = "daily.writer";
    let session = with_sessions(state, plugin, |store, _| store.create("project-1", None)).unwrap();
    let input = task_input(&session.id, Some(20));
    let link = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    let claim = SessionClaim::current(state, plugin, &link).unwrap();
    let stop = |state: &AgentStore| {
        with_sessions(state, plugin, |_, in_flight| {
            Ok(stop_session_task(in_flight, plugin, &session.id))
        })
        .unwrap()
    };

    // The stop arrives before the segment waits on it; the wake-up is kept.
    assert!(stop(state));
    let outcome = until_stopped(Some(&claim), std::future::pending()).await;
    assert_eq!(outcome.err().unwrap().error, PLUGIN_TASK_STOPPED);

    // A segment that never records its end still frees the session when its claim drops.
    drop(claim);
    assert!(!stop(state));

    // An older claim dropped late leaves the newer segment's hold alone.
    let next = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    let stale = SessionClaim {
        state,
        key: next.in_flight_key(plugin),
        stop: Arc::default(),
    };
    drop(stale);
    assert!(stop(state));
}

#[test]
fn session_run_lifecycle_records_tool_calls_and_replays_them() {
    let runtime = DailyRuntime::test_runtime();
    let state = runtime.store();
    let plugin = "daily.writer";
    let session = with_sessions(state, plugin, |store, _| store.create("project-1", None)).unwrap();
    let input = task_input(&session.id, Some(20));

    let link = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    assert!(begin_session_run(state, plugin, &session.id, &input)
        .unwrap_err()
        .contains("already running"));

    // Pause on a plugin tool: the run waits and the session stays claimed by that run.
    let run_id = "plugin-agent-task-test";
    AGENT_TASK_RUNS
        .persist(&state.project_root, run_id, &json!({ "runId": run_id }))
        .unwrap();
    record_session_run(state, plugin, &link, |run, _| {
        run.status = SessionRunStatus::Waiting;
        run.run_id = Some(run_id.into());
        Some(link.entry_id.clone())
    })
    .unwrap();
    assert!(begin_session_run(state, plugin, &session.id, &input)
        .unwrap_err()
        .contains("waiting"));
    assert!(resume_session_run(state, plugin, &link, "other-run").is_err());
    resume_session_run(state, plugin, &link, run_id).unwrap();
    AGENT_TASK_RUNS.remove(&state.project_root, run_id).unwrap();

    record_session_run(state, plugin, &link, |run, content| {
        run.tool_calls.push(SessionToolCall {
            ply: 1,
            tool_id: "writer.readDocument".into(),
            tool_name: "readDocument".into(),
            rationale: String::new(),
            input: json!({ "doc": "worldview" }),
            ok: true,
            result: "世界观全文".into(),
        });
        run.status = SessionRunStatus::Completed;
        run.run_id = None;
        *content = "已整理".into();
        None
    })
    .unwrap();

    let next = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    let stored = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    assert_eq!(stored.entries.len(), 4);
    assert_eq!(stored.entries[0].content, "继续");
    assert_eq!(stored.entries[0].meta, json!({ "kind": "text" }));
    assert_eq!(
        stored.active_entry_id.as_deref(),
        Some(next.entry_id.as_str())
    );
    let conversation = session_conversation(&stored, &next.excluded());
    assert_eq!(conversation.len(), 2);
    assert!(conversation[1].content.contains("世界观全文"));
    assert!(conversation[1].content.ends_with("已整理"));

    let _ = std::fs::remove_dir_all(runtime.project_root());
}

#[test]
fn stale_active_runs_are_marked_failed_on_next_start() {
    let runtime = DailyRuntime::test_runtime();
    let state = runtime.store();
    let plugin = "daily.writer";
    let session = with_sessions(state, plugin, |store, _| store.create("", None)).unwrap();
    let input = task_input(&session.id, None);
    let link = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    // Simulate an app restart: the in-flight claim is gone, the session still says running.
    state.plugin_agent_sessions.lock().unwrap().clear();

    begin_session_run(state, plugin, &session.id, &input).unwrap();
    let stored = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    let old = stored
        .entries
        .iter()
        .find(|entry| entry.id == link.entry_id)
        .unwrap();
    assert_eq!(old.run.as_ref().unwrap().status, SessionRunStatus::Failed);

    let _ = std::fs::remove_dir_all(runtime.project_root());
}

fn pending_tool(ply: usize) -> PendingPluginAgentTool {
    PendingPluginAgentTool {
        ply,
        tool_id: "writer.readDocument".into(),
        tool_name: "读取文档".into(),
        rationale: "先看世界观".into(),
        input: json!({ "doc": "worldview" }),
        pending_message: "读取中".into(),
        metadata: Value::Null,
        observations: Vec::new(),
    }
}

fn checkpoint(
    plugin: &str,
    input: PluginAgentTaskStartInput,
    link: &PluginSessionLink,
) -> PendingPluginAgentTaskRun {
    PendingPluginAgentTaskRun {
        run_id: format!("plugin-agent-task-{}", unique_stamp()),
        plugin_id: plugin.into(),
        task: input,
        task_id: "plugin-task-1".into(),
        pending_tool: None,
        observations: Vec::new(),
        next_ply: 1,
        failed_resume: None,
        last_error: None,
        session: Some(link.clone()),
        defined_agents: Vec::new(),
        sub_agent: None,
        created_at: unix_timestamp(),
    }
}

fn failure(error: &str) -> PluginAgentTaskFlowError {
    let mut event_views = Vec::new();
    mark_segment_failed(&mut event_views, "plugin-task-1", 1, error);
    PluginAgentTaskFlowError {
        error: error.into(),
        event_views,
        iterations_used: 1,
    }
}

#[test]
fn created_agents_are_kept_in_the_session_and_a_sub_agent_pause_names_its_agent() {
    let runtime = DailyRuntime::test_runtime();
    let state = runtime.store();
    let plugin = "daily.writer";
    let session = with_sessions(state, plugin, |store, _| store.create("", None)).unwrap();
    let mut input = task_input(&session.id, None);
    input.tools = serde_json::from_value(json!([{
        "id": "writer.write", "name": "Write", "description": "d",
        "inputSchema": { "type": "object" }, "execution": { "type": "async", "pendingMessage": "writing" }
    }]))
    .unwrap();
    input.sub_agents.allow_create = true;
    let link = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    let spec = AgentToolSpec {
        id: "drafter".into(),
        name: "执笔".into(),
        description: String::new(),
        instructions: "write".into(),
        tools: vec!["writer.write".into(), "writer.gone".into()],
        max_iterations: None,
    };
    let frame = SubAgentFrame {
        agent_id: "drafter".into(),
        parent_ply: 2,
        parent_rationale: String::new(),
        parent_input: json!({ "task": "draft" }),
        observations: Vec::new(),
        next_ply: 2,
    };
    let response = complete_plugin_agent_task(
        state,
        TaskSegment {
            checkpoint: checkpoint(plugin, input.clone(), &link),
            persist_on_failure: true,
            continues_run: false,
            base_iterations: 0,
            resumed_calls: Vec::new(),
        },
        Ok(PluginAgentTaskFlow {
            result: PluginAgentTaskFlowResult::Paused {
                pending_tool: PendingPluginAgentTool {
                    ply: 1,
                    tool_id: "writer.write".into(),
                    tool_name: "Write".into(),
                    rationale: String::new(),
                    input: json!({}),
                    pending_message: "writing".into(),
                    metadata: Value::Null,
                    observations: Vec::new(),
                },
                sub_agent: Some(frame),
            },
            event_views: Vec::new(),
            iterations_used: 2,
            tool_calls: Vec::new(),
            defined_agents: vec![spec.clone()],
        }),
    )
    .unwrap();
    assert_eq!(response["subAgent"]["name"], "执笔");
    let run_id = response["runId"].as_str().unwrap().to_string();
    let pending = AGENT_TASK_RUNS
        .load::<PendingPluginAgentTaskRun>(&state.project_root, &run_id)
        .unwrap();
    assert_eq!(pending.sub_agent.as_ref().unwrap().parent_ply, 2);
    assert_eq!(pending.defined_agents, vec![spec.clone()]);
    // Tools the task no longer offers are dropped when the agent is offered again.
    assert_eq!(
        run_agents(&pending)[0].tools,
        vec!["writer.write".to_string()]
    );

    let saved = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    assert_eq!(saved.agent_tools, vec![spec]);
}

#[test]
fn sub_agent_steps_are_scoped_under_the_parent_call() {
    let mut views = Vec::new();
    let event = AgentTaskEvent::SubAgent {
        agent_id: "drafter".into(),
        agent_name: "执笔".into(),
        parent_ply: 3,
        event: Box::new(AgentTaskEvent::FinalSelected {
            ply: 1,
            rationale: String::new(),
            raw_output: String::new(),
            output_text: "draft ready".into(),
            output_value: Value::Null,
        }),
    };
    apply_plugin_task_event_view(&mut views, "task", &event);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].id, "task-ply-3-sub-drafter-final");
    assert_eq!(views[0].metadata["subAgent"]["name"], "执笔");
    assert!(views[0].kind == agent::AgentEventViewKind::ToolResult);
}

#[test]
fn failed_segments_keep_a_checkpoint_until_a_new_message_drops_it() {
    let runtime = DailyRuntime::test_runtime();
    let state = runtime.store();
    let plugin = "daily.writer";
    let session = with_sessions(state, plugin, |store, _| store.create("", None)).unwrap();
    let input = task_input(&session.id, None);
    let link = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    let run = checkpoint(plugin, input.clone(), &link);
    let run_id = run.run_id.clone();

    let error = complete_plugin_agent_task(
        state,
        TaskSegment {
            checkpoint: run,
            persist_on_failure: true,
            continues_run: false,
            base_iterations: 0,
            resumed_calls: Vec::new(),
        },
        Err(failure("Transport error")),
    )
    .unwrap_err();
    assert_eq!(error, "Transport error");

    let stored = AGENT_TASK_RUNS
        .load::<PendingPluginAgentTaskRun>(&state.project_root, &run_id)
        .unwrap();
    assert!(stored.pending_tool.is_none());
    assert_eq!(stored.last_error.as_deref(), Some("Transport error"));
    let saved = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    let record = saved.entries[1].run.as_ref().unwrap();
    assert_eq!(record.status, SessionRunStatus::Failed);
    assert_eq!(record.run_id.as_deref(), Some(run_id.as_str()));
    assert!(record
        .event_views
        .iter()
        .any(|view| view.id == "plugin-task-1-ply-1-error"));
    // The session is free: continuing claims it again and clears the failure note.
    resume_session_run(state, plugin, &link, &run_id).unwrap();
    let saved = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    assert!(saved.entries[1].content.is_empty());
    assert_eq!(
        saved.entries[1].run.as_ref().unwrap().status,
        SessionRunStatus::Running
    );
    state.plugin_agent_sessions.lock().unwrap().clear();
    record_session_run(state, plugin, &link, |run, _| {
        run.status = SessionRunStatus::Failed;
        None
    })
    .unwrap();

    begin_session_run(state, plugin, &session.id, &input).unwrap();
    assert!(!AGENT_TASK_RUNS.path(&state.project_root, &run_id).exists());
    let saved = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    assert!(saved.entries[1].run.as_ref().unwrap().run_id.is_none());

    let _ = std::fs::remove_dir_all(runtime.project_root());
}

#[test]
fn unanswered_tool_calls_reopen_as_waiting() {
    let runtime = DailyRuntime::test_runtime();
    let state = runtime.store();
    let plugin = "daily.writer";
    let session = with_sessions(state, plugin, |store, _| store.create("", None)).unwrap();
    let input = task_input(&session.id, None);
    let link = begin_session_run(state, plugin, &session.id, &input)
        .unwrap()
        .0;
    let tool = pending_tool(2);
    record_session_run(state, plugin, &link, |run, content| {
        run.status = SessionRunStatus::Failed;
        run.run_id = Some("run-1".into());
        *content = "任务失败：x".into();
        None
    })
    .unwrap();

    reopen_waiting_run(state, plugin, &link, "run-1", &tool).unwrap();
    let saved = with_sessions(state, plugin, |store, _| store.load(&session.id)).unwrap();
    let entry = &saved.entries[1];
    let record = entry.run.as_ref().unwrap();
    assert_eq!(record.status, SessionRunStatus::Waiting);
    assert_eq!(
        record.pending_tool_id.as_deref(),
        Some("writer.readDocument")
    );
    assert!(record.error.is_none());
    assert!(entry.content.is_empty());
    assert_eq!(
        saved.active_entry_id.as_deref(),
        Some(link.entry_id.as_str())
    );
    assert!(reopen_waiting_run(state, plugin, &link, "other", &tool).is_err());

    let _ = std::fs::remove_dir_all(runtime.project_root());
}

#[test]
fn tool_steps_keep_their_input_and_complete_across_segments() {
    let mut views = Vec::new();
    let tool = pending_tool(1);
    apply_plugin_task_event_view(
        &mut views,
        "plugin-task-1",
        &AgentTaskEvent::ToolSelected {
            ply: 1,
            rationale: tool.rationale.clone(),
            raw_output: String::new(),
            tool_id: tool.tool_id.clone(),
            tool_name: tool.tool_name.clone(),
            input: tool.input.clone(),
        },
    );
    let patches = apply_plugin_task_event_view(
        &mut views,
        "plugin-task-1",
        &AgentTaskEvent::AsyncToolPending {
            ply: 1,
            tool_id: tool.tool_id.clone(),
            tool_name: tool.tool_name.clone(),
            rationale: tool.rationale.clone(),
            input: tool.input.clone(),
            pending_message: tool.pending_message.clone(),
            metadata: Value::Null,
            observations: Vec::new(),
        },
    );
    assert_eq!(views.len(), 1);
    assert!(views[0].content.contains("worldview"));
    assert_eq!(views[0].metadata["rationale"], "先看世界观");
    assert!(matches!(
        patches[0].merge,
        agent::AgentEventViewMergeStrategy::Replace
    ));

    // The next segment completes the same step and adds its result.
    let mut next = views.clone();
    merge_event_views(
        &mut next,
        &resumed_tool_views("plugin-task-1", &tool, "世界观全文", true),
    );
    assert_eq!(next.len(), 2);
    assert!(next[0].status == agent::AgentEventViewStatus::Completed);
    assert_eq!(next[0].title, "Tool done: 读取文档");
    assert_eq!(next[1].content, "世界观全文");

    let chunk = apply_plugin_task_event_view(
        &mut next,
        "plugin-task-1",
        &AgentTaskEvent::ModelStreamChunk {
            ply: 2,
            text: "思".into(),
        },
    );
    assert_eq!(chunk[0].view.content, "思");
    let chunk = apply_plugin_task_event_view(
        &mut next,
        "plugin-task-1",
        &AgentTaskEvent::ModelStreamChunk {
            ply: 2,
            text: "考".into(),
        },
    );
    assert!(matches!(
        chunk[0].merge,
        agent::AgentEventViewMergeStrategy::AppendContent
    ));
    assert_eq!(chunk[0].view.content, "考");
    assert_eq!(next.last().unwrap().content, "思考");
}

#[test]
fn progress_sink_coalesces_streamed_text() {
    let sent = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let sink_sent = sent.clone();
    let emit: Arc<EventCallback> = Arc::new(move |event: &str, payload: Value| {
        assert_eq!(event, PLUGIN_AGENT_PROGRESS_EVENT);
        sink_sent.lock().unwrap().push(payload);
        Ok(())
    });
    let mut sink = PluginProgressSink::new(emit, "daily.writer", "main", "stream-1");
    let view = |text: &str| {
        agent::AgentEventView::new(
            "t-ply-1-thought",
            agent::AgentEventViewKind::LlmThought,
            agent::AgentEventViewStatus::InProgress,
            "Agent thinking",
            text,
        )
    };
    let append = |text: &str| {
        agent::AgentEventViewPatch::new(
            view(text),
            agent::AgentEventViewMergeStrategy::AppendContent,
        )
    };
    sink.push(vec![append("a")]);
    sink.push(vec![append("b")]);
    assert!(sent.lock().unwrap().is_empty());
    sink.flush();
    let frames = sent.lock().unwrap().clone();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["streamId"], "stream-1");
    assert_eq!(frames[0]["patches"][0]["view"]["content"], "ab");
    assert_eq!(frames[0]["patches"][0]["merge"], "append_content");
}
