use crate::{checkpoint::*, model::*, supervisor::RunSupervisor, workflow::*, *};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Script {
    id: String,
    replies: Mutex<VecDeque<ModelCompletionResponse>>,
    seen: Mutex<Vec<ModelCompletionRequest>>,
}
impl Script {
    fn new(id: &str, replies: Vec<ModelCompletionResponse>) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            replies: Mutex::new(replies.into()),
            seen: Mutex::new(Vec::new()),
        })
    }
}
#[async_trait::async_trait]
impl Model for Script {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity {
            provider: "offline".into(),
            model: self.id.clone(),
        }
    }
    async fn complete(
        &self,
        request: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        self.seen.lock().unwrap().push(request.clone());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ModelError::protocol("Unexpected request"))
    }
}
fn answer(text: &str) -> ModelCompletionResponse {
    ModelCompletionResponse {
        raw_output: text.into(),
        usage: Some(ModelUsage {
            input_tokens: 10,
            output_tokens: 2,
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn calls(ids: &[&str]) -> ModelCompletionResponse {
    ModelCompletionResponse {
        tool_calls: ids
            .iter()
            .enumerate()
            .map(|(i, id)| ModelToolCall {
                call_id: Some(format!("call-{i}")),
                tool_id: (*id).into(),
                arguments: json!({"task":"work"}),
                ..Default::default()
            })
            .collect(),
        stop_reason: ModelStopReason::ToolCalls,
        provider: "offline".into(),
        provider_items: vec![json!({"opaque":"reasoning"})],
        ..Default::default()
    }
}
fn definition(id: &str, tools: &[&str]) -> AgentDefinition {
    AgentDefinition::new(AgentToolSpec {
        id: id.into(),
        name: id.into(),
        description: id.into(),
        instructions: format!("Work as {id}"),
        tools: tools.iter().map(|id| (*id).into()).collect(),
        max_iterations: Some(8),
    })
}
fn pending_tool() -> AgentTool {
    agent_tool_from_dynamic(DynamicAgentToolDefinition {
        id: "save".into(),
        name: "Save".into(),
        description: "Wait for host".into(),
        input_schema: json!({"type":"object"}),
        execution: DynamicAgentToolExecution::Async {
            pending_message: "waiting".into(),
            metadata: Value::Null,
        },
    })
}

#[tokio::test]
async fn nested_agents_use_distinct_models_and_resume_the_entire_stack() {
    let parent = Script::new(
        "parent",
        vec![calls(&["subagent.editor"]), answer("parent done")],
    );
    let editor = Script::new(
        "editor",
        vec![calls(&["subagent.reviewer"]), answer("editor done")],
    );
    let reviewer = Script::new("reviewer", vec![calls(&["save"]), answer("reviewer done")]);
    let mut task = AgentTask::new("root", parent.clone(), "work");
    task.tools = vec![pending_tool()];
    task.bindings.models.insert("editor".into(), editor.clone());
    task.bindings
        .models
        .insert("reviewer".into(), reviewer.clone());
    task.register_agent(
        definition("editor", &["save"])
            .with_model("editor")
            .delegate("reviewer"),
    );
    task.agent_definitions.insert(
        "reviewer".into(),
        definition("reviewer", &["save"]).with_model("reviewer"),
    );
    let bindings = task.bindings();
    let context = task.execution.clone();
    assert!(matches!(
        run_agent_task_until_pause(task, |_| Ok(())).await.unwrap(),
        AgentTaskRunResult::Paused(_)
    ));
    let checkpoint: RunCheckpoint =
        serde_json::from_str(&serde_json::to_string(&context.checkpoint().unwrap()).unwrap())
            .unwrap();
    let frame = checkpoint.task.resume_sub_agent.as_ref().unwrap();
    assert!(frame.child.is_some());
    let resumed = checkpoint
        .restore(
            &bindings,
            ResumeAction::ToolResult {
                call_id: checkpoint
                    .pending
                    .as_ref()
                    .unwrap()
                    .call
                    .as_ref()
                    .unwrap()
                    .call_id
                    .clone(),
                result: AgentToolResult::new("saved once"),
                is_error: false,
            },
            None,
        )
        .unwrap();
    let resumed_context = resumed.execution.clone();
    assert_eq!(
        run_agent_task(resumed, |_| Ok(()))
            .await
            .unwrap()
            .final_text,
        "parent done"
    );
    assert_eq!(resumed_context.budget().model_requests, 6);
    assert_eq!(
        reviewer.seen.lock().unwrap()[1]
            .messages
            .last()
            .unwrap()
            .content,
        "saved once"
    );
    let events = resumed_context.events_after(0);
    assert!(events
        .events
        .iter()
        .any(|e| e.run.depth == 2 && e.run.agent_id == "reviewer"));
    assert!(events
        .events
        .windows(2)
        .all(|p| p[0].sequence < p[1].sequence));
}

#[tokio::test]
async fn aggregate_budget_stops_delegation_and_recursive_cycles_have_a_depth_limit() {
    let model = Script::new(
        "shared",
        vec![calls(&["subagent.child"]), answer("should not run")],
    );
    let mut task = AgentTask::new("budget", model.clone(), "work");
    task.register_agent(definition("child", &[]));
    task.execution = RunContext::new(
        "budget",
        RunLimits {
            max_model_requests: 1,
            ..Default::default()
        },
    );
    assert!(matches!(
        run_agent_task(task, |_| Ok(())).await,
        Err(AgentError::Budget(_))
    ));
    assert_eq!(model.seen.lock().unwrap().len(), 1);
    let model = Script::new(
        "recursive",
        vec![
            calls(&["subagent.child"]),
            calls(&["subagent.child"]),
            calls(&["subagent.child"]),
        ],
    );
    let mut task = AgentTask::new("depth", model.clone(), "work");
    task.register_agent(definition("child", &[]).delegate("child"));
    task.execution = RunContext::new(
        "depth",
        RunLimits {
            max_depth: 2,
            ..Default::default()
        },
    );
    assert!(matches!(
        run_agent_task(task, |_| Ok(())).await,
        Err(AgentError::Budget(_))
    ));
    assert_eq!(model.seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn cancelled_external_tool_requires_resolution_and_completed_calls_are_never_replayed() {
    let model = Script::new(
        "durable",
        vec![calls(&["first", "external", "last"]), answer("finished")],
    );
    let first = Arc::new(AtomicUsize::new(0));
    let count = first.clone();
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = entered.clone();
    let last = Arc::new(AtomicUsize::new(0));
    let last_count = last.clone();
    let mut task = AgentTask::new("durable", model.clone(), "work");
    task.tools = vec![
        AgentTool::new_with_execution("first", "first", "first", json!({}), move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(AgentToolExecution::Completed(AgentToolResult::new(
                "first result",
            )))
        }),
        AgentTool::new_async("external", "external", "external", json!({}), move |_| {
            let signal = signal.clone();
            async move {
                signal.notify_one();
                std::future::pending().await
            }
        }),
        AgentTool::new_with_execution("last", "last", "last", json!({}), move |_| {
            last_count.fetch_add(1, Ordering::SeqCst);
            Ok(AgentToolExecution::Completed(AgentToolResult::new(
                "last result",
            )))
        }),
    ];
    let bindings = task.bindings();
    let store = Arc::new(MemoryCheckpointStore::default());
    let supervisor = RunSupervisor::with_store(store.clone());
    let handle = supervisor.start(task).unwrap();
    let id = handle.context.identity.run_id.clone();
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    handle.cancel();
    assert!(matches!(handle.wait().await, Err(AgentError::Stopped(_))));
    let checkpoint = store.load(&id).await.unwrap().unwrap();
    assert!(matches!(
        checkpoint.phase,
        CheckpointPhase::ToolInFlight { .. }
    ));
    assert!(checkpoint
        .restore(&bindings, ResumeAction::Continue, None)
        .is_err());
    let resumed = supervisor
        .resume(
            &id,
            &bindings,
            ResumeAction::ToolResult {
                call_id: "call-1".into(),
                result: AgentToolResult::new("external outcome reconciled"),
                is_error: false,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        resumed.wait().await.unwrap(),
        AgentTaskRunResult::Completed(_)
    ));
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(last.load(Ordering::SeqCst), 1);
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let results: Vec<_> = seen[1]
        .messages
        .iter()
        .filter(|m| m.tool_result.is_some())
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        results,
        ["first result", "external outcome reconciled", "last result"]
    );
}

struct SlowModel {
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl Model for SlowModel {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity {
            provider: "offline".into(),
            model: "slow".into(),
        }
    }
    async fn complete(
        &self,
        _: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        self.peak.fetch_max(
            self.active.fetch_add(1, Ordering::SeqCst) + 1,
            Ordering::SeqCst,
        );
        let _active = Active(self.active.clone());
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(answer("done"))
    }
}

#[tokio::test]
async fn workflows_bound_concurrency_join_dependencies_and_share_usage() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(SlowModel {
        active: active.clone(),
        peak: peak.clone(),
    });
    let mut bindings = Bindings::default();
    bindings.models.insert("default".into(), model);
    let workflow = Workflow {
        agents: BTreeMap::from([("worker".into(), definition("worker", &[]))]),
        nodes: (0..7)
            .map(|i| WorkflowNode {
                id: format!("node-{i}"),
                agent_id: "worker".into(),
                prompt: "work".into(),
                dependencies: if i == 6 {
                    vec!["node-0".into(), "node-5".into()]
                } else {
                    vec![]
                },
            })
            .collect(),
        failure_policy: FailurePolicy::FailFast,
    };
    let context = RunContext::new(
        "workflow",
        RunLimits {
            max_concurrency: 3,
            ..Default::default()
        },
    );
    let state = workflow
        .run(&bindings, context.clone(), None)
        .await
        .unwrap();
    assert_eq!(state.results.len(), 7);
    assert_eq!(state.budget.model_requests, 7);
    assert_eq!(state.budget.usage.input_tokens, 70);
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(context.status(), RunStatus::Completed);
    let mut invalid = workflow;
    invalid.nodes[0].dependencies = vec!["node-6".into()];
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn cancelling_a_background_run_drops_active_model_work() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(SlowModel {
        active: active.clone(),
        peak: peak.clone(),
    });
    let task = AgentTask::new("cancel", model, "work");
    let supervisor = RunSupervisor::default();
    let handle = supervisor.start(task).unwrap();
    while active.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    handle.cancel();
    assert!(matches!(handle.wait().await, Err(AgentError::Stopped(_))));
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unsupported_checkpoint_versions_and_stale_writers_are_rejected() {
    let model = Script::new("pause", vec![calls(&["save"])]);
    let mut task = AgentTask::new("pause", model, "work");
    task.tools = vec![pending_tool()];
    let bindings = task.bindings();
    let context = task.execution.clone();
    run_agent_task_until_pause(task, |_| Ok(())).await.unwrap();
    let mut checkpoint = context.checkpoint().unwrap();
    checkpoint.version = 999;
    assert!(checkpoint
        .restore(&bindings, ResumeAction::Continue, None)
        .is_err());
    checkpoint.version = CHECKPOINT_VERSION;
    let store = MemoryCheckpointStore::default();
    checkpoint.revision = 1;
    store.save(&checkpoint, None).await.unwrap();
    assert!(store.save(&checkpoint, None).await.is_err());
}

#[tokio::test]
async fn handoff_answers_without_returning_to_the_parent_even_after_a_pause() {
    let mut handoff = calls(&[AGENT_HANDOFF_TOOL_ID]);
    handoff.tool_calls[0].arguments = json!({"agent":"worker","task":"finish"});
    let parent = Script::new("handoff-parent", vec![handoff]);
    let worker = Script::new(
        "handoff-worker",
        vec![calls(&["save"]), answer("worker final answer")],
    );
    let mut task = AgentTask::new("handoff", parent.clone(), "work");
    task.allow_handoff = true;
    task.tools = vec![pending_tool()];
    task.bindings.models.insert("worker".into(), worker);
    task.register_agent(definition("worker", &["save"]).with_model("worker"));
    let bindings = task.bindings();
    let context = task.execution.clone();
    run_agent_task_until_pause(task, |_| Ok(())).await.unwrap();
    let checkpoint = context.checkpoint().unwrap();
    let task = checkpoint
        .restore(
            &bindings,
            ResumeAction::ToolResult {
                call_id: "call-0".into(),
                result: AgentToolResult::new("saved"),
                is_error: false,
            },
            None,
        )
        .unwrap();
    assert_eq!(
        run_agent_task(task, |_| Ok(())).await.unwrap().final_text,
        "worker final answer"
    );
    assert_eq!(parent.seen.lock().unwrap().len(), 1);
}

struct FailOnce(AtomicUsize);
#[async_trait::async_trait]
impl Model for FailOnce {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity {
            provider: "offline".into(),
            model: "fail-once".into(),
        }
    }
    async fn complete(
        &self,
        _: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(ModelError::new(
                ModelErrorKind::Network,
                "interrupted request",
            ))
        } else {
            Ok(answer("recovered child"))
        }
    }
}

#[tokio::test]
async fn interrupted_child_model_is_reconciled_from_its_own_checkpoint() {
    let parent = Script::new(
        "tree-parent",
        vec![calls(&["subagent.child"]), answer("tree done")],
    );
    let child = Arc::new(FailOnce(AtomicUsize::new(0)));
    let mut task = AgentTask::new("tree", parent.clone(), "work");
    task.continue_on_tool_error = false;
    task.bindings.models.insert("child".into(), child.clone());
    task.register_agent(definition("child", &[]).with_model("child"));
    let bindings = task.bindings();
    let store = Arc::new(MemoryCheckpointStore::default());
    let supervisor = RunSupervisor::with_store(store.clone());
    let handle = supervisor.start(task).unwrap();
    let id = handle.context.identity.run_id.clone();
    assert!(handle.wait().await.is_err());
    assert!(matches!(
        store.load(&id).await.unwrap().unwrap().phase,
        CheckpointPhase::Delegating
    ));
    let resumed = supervisor
        .resume(&id, &bindings, ResumeAction::RetryModel)
        .await
        .unwrap();
    let context = resumed.context.clone();
    let AgentTaskRunResult::Completed(result) = resumed.wait().await.unwrap() else {
        panic!("expected completion")
    };
    assert_eq!(result.final_text, "tree done");
    assert_eq!(parent.seen.lock().unwrap().len(), 2);
    assert_eq!(child.0.load(Ordering::SeqCst), 2);
    assert_eq!(context.budget().model_requests, 4);
}

#[tokio::test]
async fn workflow_pause_and_resume_keeps_completed_nodes_and_dependency_results() {
    let model = Script::new(
        "workflow-resume",
        vec![
            answer("first"),
            calls(&["save"]),
            answer("second"),
            answer("third"),
        ],
    );
    let mut bindings = Bindings::default();
    bindings.models.insert("default".into(), model.clone());
    bindings.tools.insert("save".into(), pending_tool());
    let workflow = Workflow::serial(
        BTreeMap::from([("worker".into(), definition("worker", &["save"]))]),
        (0..3)
            .map(|i| WorkflowNode {
                id: i.to_string(),
                agent_id: "worker".into(),
                prompt: "work".into(),
                dependencies: vec![],
            })
            .collect(),
    );
    let store = Arc::new(MemoryCheckpointStore::default());
    let mut state = workflow
        .run(
            &bindings,
            RunContext::new_with_store("workflow", Default::default(), store.clone()),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(state.results["0"], NodeResult::Completed { .. }));
    assert!(matches!(
        state.results["1"],
        NodeResult::WaitingInput { .. }
    ));
    assert!(!state.results.contains_key("2"));
    state
        .resume_node(
            "1",
            &bindings,
            ResumeAction::ToolResult {
                call_id: "call-0".into(),
                result: AgentToolResult::new("saved"),
                is_error: false,
            },
            Some(store.clone()),
        )
        .await
        .unwrap();
    let context = state.execution(Some(store.clone()));
    let state = workflow.run(&bindings, context, Some(state)).await.unwrap();
    assert_eq!(state.results.len(), 3);
    assert_eq!(state.budget.model_requests, 4);
    let requests = model.seen.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[3]
        .messages
        .iter()
        .any(|m| m.content.contains("Dependency 1") && m.content.contains("second")));
}

#[tokio::test]
async fn dropping_a_wait_future_cancels_the_worker_and_supervisor_bounds_root_runs() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(SlowModel {
        active: active.clone(),
        peak: peak.clone(),
    });
    let supervisor = RunSupervisor::with_capacity(1).unwrap();
    let first = supervisor
        .start(AgentTask::new("first", model.clone(), "work"))
        .unwrap();
    let second = supervisor
        .start(AgentTask::new("second", model, "work"))
        .unwrap();
    let second_context = second.context.clone();
    second.cancel();
    assert!(second.wait().await.is_err());
    assert_eq!(second_context.status(), RunStatus::Cancelled);
    let context = first.context.clone();
    assert!(tokio::time::timeout(Duration::from_millis(1), first.wait())
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(context.status(), RunStatus::Cancelled);
    assert!(peak.load(Ordering::SeqCst) <= 1);
}

#[tokio::test]
async fn deadlines_cancel_model_io_and_budget_counts_context_compaction() {
    let active = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(SlowModel {
        active: active.clone(),
        peak: Arc::new(AtomicUsize::new(0)),
    });
    let mut task = AgentTask::new("deadline", model, "work");
    task.execution = RunContext::new(
        "deadline",
        RunLimits::default().with_timeout(Duration::from_millis(1)),
    );
    assert!(matches!(
        run_agent_task(task, |_| Ok(())).await,
        Err(AgentError::Budget(_))
    ));
    assert_eq!(active.load(Ordering::SeqCst), 0);
    let model = Script::new(
        "summary-budget",
        vec![answer("summary"), answer("should not run")],
    );
    let mut task = AgentTask::new("summary", model.clone(), "work");
    task.prior_tool_observations = vec![
        AgentToolObservation::runtime_message("old"),
        AgentToolObservation::runtime_message("recent"),
    ];
    task.observation_budget = Some(ObservationBudget {
        max_tokens: 1,
        keep_recent: 1,
    });
    task.execution = RunContext::new(
        "summary",
        RunLimits {
            max_model_requests: 1,
            ..Default::default()
        },
    );
    assert!(matches!(
        run_agent_task(task, |_| Ok(())).await,
        Err(AgentError::Budget(_))
    ));
    assert_eq!(model.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn persisted_workflow_recovers_a_completed_child_after_graph_write_failure() {
    struct InterruptedGraph {
        inner: MemoryCheckpointStore,
        fail: std::sync::atomic::AtomicBool,
    }
    #[async_trait::async_trait]
    impl CheckpointStore for InterruptedGraph {
        async fn load(&self, id: &str) -> Result<Option<RunCheckpoint>, AgentError> {
            self.inner.load(id).await
        }
        async fn save(&self, cp: &RunCheckpoint, expected: Option<u64>) -> Result<(), AgentError> {
            self.inner.save(cp, expected).await
        }
        async fn load_tree_budget(&self, id: &str) -> Result<BudgetSnapshot, AgentError> {
            self.inner.load_tree_budget(id).await
        }
        async fn load_workflow(&self, id: &str) -> Result<Option<WorkflowState>, AgentError> {
            self.inner.load_workflow(id).await
        }
        async fn save_workflow(
            &self,
            state: &WorkflowState,
            expected: Option<u64>,
        ) -> Result<(), AgentError> {
            if state
                .results
                .values()
                .any(|r| matches!(r, NodeResult::Completed { .. }))
                && self.fail.swap(false, Ordering::SeqCst)
            {
                return Err(AgentError::Output("injected graph storage failure".into()));
            }
            self.inner.save_workflow(state, expected).await
        }
    }
    let store = Arc::new(InterruptedGraph {
        inner: Default::default(),
        fail: std::sync::atomic::AtomicBool::new(true),
    });
    let model = Script::new("durable-graph", vec![answer("first"), answer("second")]);
    let mut bindings = Bindings::default();
    bindings.models.insert("default".into(), model.clone());
    let workflow = Workflow::serial(
        BTreeMap::from([("worker".into(), definition("worker", &[]))]),
        (0..2)
            .map(|i| WorkflowNode {
                id: i.to_string(),
                agent_id: "worker".into(),
                prompt: "work".into(),
                dependencies: vec![],
            })
            .collect(),
    );
    let context = RunContext::new_with_store("durable-graph", Default::default(), store.clone());
    let id = context.identity.run_id.clone();
    assert!(workflow.run(&bindings, context, None).await.is_err());
    assert_eq!(model.seen.lock().unwrap().len(), 1);
    let state = store.load_workflow(&id).await.unwrap().unwrap();
    assert!(matches!(state.results["0"], NodeResult::Running { .. }));
    let stale = state.clone();
    let context = state.execution(Some(store.clone()));
    let state = workflow.run(&bindings, context, Some(state)).await.unwrap();
    assert!(state
        .results
        .values()
        .all(|r| matches!(r, NodeResult::Completed { .. })));
    assert_eq!(state.budget.model_requests, 2);
    assert_eq!(model.seen.lock().unwrap().len(), 2);
    let context = stale.execution(Some(store.clone()));
    assert!(workflow.run(&bindings, context, Some(stale)).await.is_err());
    assert_eq!(model.seen.lock().unwrap().len(), 2);
    let checkpoint = store.load(&format!("{id}:node:0")).await.unwrap().unwrap();
    let (context, outcome) = checkpoint.completed_result(Some(store.clone())).unwrap();
    assert_eq!(outcome.final_text, "first");
    assert_eq!(context.status(), RunStatus::Completed);
    let mut duplicate = AgentTask::new("duplicate", model.clone(), "work");
    duplicate.execution = RunContext::new_with_store("duplicate", Default::default(), store);
    duplicate.execution.identity.run_id = checkpoint.run.run_id;
    assert!(run_agent_task(duplicate, |_| Ok(())).await.is_err());
    assert_eq!(model.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn supervisor_fences_concurrent_resume() {
    let model = Script::new("supervised-resume", vec![calls(&["save"]), answer("done")]);
    let mut task = AgentTask::new("worker", model.clone(), "work");
    task.tools.push(pending_tool());
    let bindings = task.bindings();
    let store = Arc::new(MemoryCheckpointStore::default());
    let supervisor = RunSupervisor::with_store(store);
    let handle = supervisor.start(task).unwrap();
    let id = handle.context.identity.run_id.clone();
    assert!(matches!(
        handle.wait().await.unwrap(),
        AgentTaskRunResult::Paused(_)
    ));
    let action = ResumeAction::ToolResult {
        call_id: "call-0".into(),
        result: AgentToolResult::new("saved"),
        is_error: false,
    };
    let (first, second) = tokio::join!(
        supervisor.resume(&id, &bindings, action.clone()),
        supervisor.resume(&id, &bindings, action)
    );
    assert!(first.is_ok());
    assert!(second.is_err());
    let handle = first.unwrap();
    let context = handle.context.clone();
    tokio::time::timeout(Duration::from_secs(1), async {
        while context.status() != RunStatus::Completed {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    handle.wait().await.unwrap();
    assert_eq!(context.status(), RunStatus::Completed);
    assert_eq!(model.seen.lock().unwrap().len(), 2);
}
