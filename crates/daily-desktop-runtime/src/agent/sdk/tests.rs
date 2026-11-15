use super::run_loop::{build_task_messages, transcript_messages};
use super::*;
use crate::model::test_support::{scripted_server, Recorded, Reply};
use crate::model::{
    ModelCompletionRequest, ModelCompletionResponse, ModelError, ModelMessageRole,
    ModelStreamChunk, ModelToolCall,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[tokio::test]
async fn persisted_desktop_run_rebinds_credentials_and_keeps_completed_tool_results() {
    use daily_agent_core::checkpoint::CheckpointStore;
    struct Adapter {
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl ModelAdapter for Adapter {
        fn kind(&self) -> ModelProviderKind {
            ModelProviderKind::OpenAiCompatible
        }
        async fn complete(
            &self,
            _: &Client,
            _: &ModelConfig,
            request: &ModelCompletionRequest,
            _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
        ) -> Result<ModelCompletionResponse, ModelError> {
            match self.calls.fetch_add(1, Ordering::SeqCst) {
                0 => Ok(ModelCompletionResponse {
                    tool_calls: vec![ModelToolCall {
                        call_id: Some("read-1".into()),
                        tool_id: "read".into(),
                        ..Default::default()
                    }],
                    stop_reason: ModelStopReason::ToolCalls,
                    ..Default::default()
                }),
                1 => Err(ModelError::new(
                    crate::model::ModelErrorKind::Auth,
                    "credential expired",
                )),
                _ => {
                    assert!(request.messages.iter().any(|m| m.content == "saved result"));
                    Ok(ModelCompletionResponse {
                        raw_output: "recovered".into(),
                        ..Default::default()
                    })
                }
            }
        }
    }
    let runtime = crate::runtime::DailyRuntime::test_runtime();
    let store = runtime.store().agent_checkpoints.clone();
    let adapter = Arc::new(Adapter {
        calls: AtomicUsize::new(0),
    });
    let tool_count = Arc::new(AtomicUsize::new(0));
    let count = tool_count.clone();
    let tool = AgentTool::new_with_execution("read", "read", "read", json!({}), move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(AgentToolExecution::Completed(AgentToolResult::new(
            "saved result",
        )))
    });
    let mut config = ModelConfig::seeded();
    config.api_key = "credential-must-never-be-persisted".into();
    config.base_url = "https://fixture.invalid/v1?private=endpoint-secret".into();
    let mut task = AgentTask::new("persisted", config.clone(), "work");
    task.model_adapter = Some(adapter.clone());
    task.tools = vec![tool.clone()];
    task.execution = Some(daily_agent_core::RunContext::new_with_store(
        "persisted",
        Default::default(),
        store.clone(),
    ));
    let id = task.execution.as_ref().unwrap().identity.run_id.clone();
    assert!(run_agent_task(&http(), task, |_| Ok(())).await.is_err());
    let checkpoint = store.load(&id).await.unwrap().unwrap();
    let raw = serde_json::to_string(&checkpoint).unwrap();
    assert!(!raw.contains("credential-must-never-be-persisted"));
    assert!(!raw.contains("endpoint-secret"));
    config.api_key = "replacement-credential".into();
    let mut resumed = AgentTask::new("persisted", config.clone(), "work");
    resumed.model_adapter = Some(adapter.clone());
    resumed.tools = vec![tool.clone()];
    resumed.checkpoint_ref = Some(id.clone());
    resumed.execution = Some(daily_agent_core::RunContext::new_with_store(
        "persisted",
        Default::default(),
        store.clone(),
    ));
    let prepared = resumed.prepare(&http()).await.unwrap();
    let context = prepared.execution();
    assert!(matches!(
        prepared.run_until_pause(|_| Ok(())).await.unwrap(),
        AgentTaskRunResult::Completed(_)
    ));
    assert_eq!(tool_count.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 3);
    assert_eq!(context.budget().model_requests, 3);
    // Simulate losing the desktop commit after the SDK had completed.
    let mut recovered = AgentTask::new("persisted", config, "work");
    recovered.model_adapter = Some(adapter.clone());
    recovered.tools = vec![tool];
    let mut execution =
        daily_agent_core::RunContext::new_with_store("persisted", Default::default(), store);
    execution.identity.run_id = id;
    recovered.execution = Some(execution);
    assert_eq!(
        run_agent_task(&http(), recovered, |_| Ok(()))
            .await
            .unwrap()
            .final_text,
        "recovered"
    );
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 3);
    assert_eq!(tool_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn desktop_correlation_ids_are_kept_off_compaction_requests() {
    struct RecordingAdapter(Mutex<Vec<ModelCompletionRequest>>);
    #[async_trait::async_trait]
    impl ModelAdapter for RecordingAdapter {
        fn kind(&self) -> ModelProviderKind {
            ModelProviderKind::OpenAiCompatible
        }
        async fn complete(
            &self,
            _: &Client,
            _: &ModelConfig,
            request: &ModelCompletionRequest,
            _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
        ) -> Result<ModelCompletionResponse, ModelError> {
            self.0.lock().unwrap().push(request.clone());
            Ok(ModelCompletionResponse {
                raw_output: "summary or answer".into(),
                ..Default::default()
            })
        }
    }
    let adapter = Arc::new(RecordingAdapter(Mutex::new(Vec::new())));
    let mut task = AgentTask::new("chat.turn", ModelConfig::seeded(), "Hello");
    task.session_id = Some(41);
    task.turn_id = Some(7);
    task.model_adapter = Some(adapter.clone());
    task.prior_tool_observations = vec![
        AgentToolObservation::runtime_message("earlier context"),
        AgentToolObservation::runtime_message("recent context"),
    ];
    task.observation_budget = Some(ObservationBudget {
        max_tokens: 1,
        keep_recent: 1,
    });
    run_agent_task(&http(), task, |_| Ok(())).await.unwrap();
    let requests = adapter.0.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].request_name.as_deref(),
        Some(daily_agent_core::compaction::SUMMARY_REQUEST_NAME)
    );
    assert_eq!((requests[0].session_id, requests[0].turn_id), (None, None));
    assert_eq!(
        (requests[1].session_id, requests[1].turn_id),
        (Some(41), Some(7))
    );
}

fn http() -> Client {
    Client::builder().no_proxy().build().unwrap()
}

fn model(base_url: String) -> ModelConfig {
    ModelConfig {
        provider_kind: ModelProviderKind::OpenAiCompatible,
        base_url,
        model: "test".into(),
        api_key: String::new(),
        ..ModelConfig::seeded()
    }
}

/// An OpenAI-style response calling the given `(function name, arguments)` tools.
fn tool_calls(text: Option<&str>, calls: &[(&str, Value)]) -> Reply {
    Reply::json(json!({ "choices": [{ "message": {
        "content": text,
        "tool_calls": calls.iter().enumerate().map(|(index, (name, arguments))| json!({
            "id": format!("call-{name}-{index}"),
            "type": "function",
            "function": { "name": name, "arguments": arguments.to_string() },
        })).collect::<Vec<_>>(),
    }, "finish_reason": "tool_calls" }],
    "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }))
}

fn tool_call(name: &str, arguments: Value) -> Reply {
    tool_calls(None, &[(name, arguments)])
}

fn text(content: &str) -> Reply {
    Reply::json(json!({
        "choices": [{ "message": { "content": content }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 120, "completion_tokens": 5 },
    }))
}

fn tools() -> Vec<AgentTool> {
    vec![
        agent_tool_from_dynamic(DynamicAgentToolDefinition {
            id: "t.lookup".into(),
            name: "Lookup".into(),
            description: "Looks something up".into(),
            input_schema: json!({ "type": "object" }),
            execution: DynamicAgentToolExecution::Sync {
                observation: "lookup result".into(),
                metadata: Value::Null,
            },
        }),
        agent_tool_from_dynamic(DynamicAgentToolDefinition {
            id: "t.write".into(),
            name: "Write".into(),
            description: "Writes, run by the host".into(),
            input_schema: json!({ "type": "object" }),
            execution: DynamicAgentToolExecution::Async {
                pending_message: "writing".into(),
                metadata: Value::Null,
            },
        }),
    ]
}

fn task(base_url: String, agents: Vec<AgentToolSpec>) -> AgentTask {
    AgentTask {
        system_prompt: "parent system".into(),
        final_instructions: "final".into(),
        context: vec![AgentContextBlock::new("Samples", "sample text")],
        tools: tools(),
        max_iterations: 8,
        observation_budget: None,
        max_output_tokens: 256,
        stream: false,
        sub_agents: SubAgentConfig {
            allow_create: true,
            agents,
        },
        ..AgentTask::new("test", model(base_url), "do the job")
    }
}

fn messages(request: &Recorded) -> &Vec<Value> {
    request.body["messages"].as_array().unwrap()
}

fn tool_names(request: &Recorded) -> Vec<String> {
    request.body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| tool["function"]["name"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn completed(task: AgentTask) -> AgentTaskOutcome {
    match run_agent_task_until_pause(&http(), task, |_| Ok(()))
        .await
        .unwrap()
    {
        AgentTaskRunResult::Completed(outcome) => outcome,
        AgentTaskRunResult::Paused(_) => panic!("expected completion"),
    }
}

#[tokio::test]
async fn tool_rounds_go_back_as_native_messages_after_an_unchanged_prefix() {
    let (url, requests) = scripted_server(vec![
        tool_calls(
            Some("Checking two things."),
            &[
                ("t_lookup", json!({ "q": "a" })),
                ("t_lookup", json!({ "q": "b" })),
            ],
        ),
        text("all done"),
    ])
    .await;
    let outcome = completed(task(url, Vec::new())).await;
    assert_eq!(outcome.final_text, "all done");
    assert_eq!(outcome.stop_reason, ModelStopReason::Complete);
    // Usage of both requests is summed.
    assert_eq!(outcome.usage.input_tokens, 220);
    assert_eq!(outcome.usage.output_tokens, 15);

    let requests = requests.lock().unwrap();
    let first = messages(&requests[0]);
    let second = messages(&requests[1]);
    // The second request starts with the first one, byte for byte.
    assert_eq!(&second[..first.len()], &first[..]);
    assert_eq!(first.last().unwrap()["content"], "do the job");
    // Then the model's own turn with both calls, and one tool message per result.
    let round = &second[first.len()..];
    assert_eq!(round.len(), 3);
    assert_eq!(round[0]["role"], "assistant");
    assert_eq!(round[0]["content"], "Checking two things.");
    assert_eq!(round[0]["tool_calls"][0]["id"], "call-t_lookup-0");
    assert_eq!(
        round[0]["tool_calls"][1]["function"]["arguments"],
        "{\"q\":\"b\"}"
    );
    assert_eq!(round[1]["role"], "tool");
    assert_eq!(round[1]["tool_call_id"], "call-t_lookup-0");
    assert_eq!(round[1]["content"], "lookup result");
    assert_eq!(round[2]["tool_call_id"], "call-t_lookup-1");
    assert_eq!(requests[0].body["tools"], requests[1].body["tools"]);
}

#[tokio::test]
async fn the_last_step_keeps_the_tools_declared_but_forbids_calling_them() {
    let (url, requests) =
        scripted_server(vec![tool_call("t_lookup", json!({})), text("answer")]).await;
    let mut task = task(url, Vec::new());
    task.max_iterations = 2;
    assert_eq!(completed(task).await.final_text, "answer");

    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].body["tool_choice"], "auto");
    assert_eq!(requests[1].body["tool_choice"], "none");
    assert_eq!(requests[1].body["tools"], requests[0].body["tools"]);
    let last = messages(&requests[1]).last().unwrap().clone();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"], FORCE_FINAL_NOTE);
}

#[tokio::test]
async fn a_made_up_tool_is_reported_to_the_model_instead_of_failing_the_run() {
    let (url, requests) =
        scripted_server(vec![tool_call("t_delete", json!({})), text("sorry")]).await;
    let mut task = task(url, Vec::new());
    task.continue_on_tool_error = false;
    assert_eq!(completed(task).await.final_text, "sorry");
    let requests = requests.lock().unwrap();
    let result = messages(&requests[1]).last().unwrap().clone();
    assert_eq!(result["role"], "tool");
    let content = result["content"].as_str().unwrap();
    assert!(content.contains("Unknown tool t_delete"), "{content}");
    assert!(content.contains("t.lookup"), "{content}");
}

#[tokio::test]
async fn side_effect_free_tools_of_one_response_run_at_the_same_time() {
    let (url, requests) = scripted_server(vec![
        tool_calls(
            None,
            &[
                ("t_slow", json!({ "n": 1 })),
                ("t_slow", json!({ "n": 2 })),
                ("t_slow", json!({ "n": 3 })),
            ],
        ),
        text("done"),
    ])
    .await;
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (running_in, peak_in) = (running.clone(), peak.clone());
    let mut task = task(url, Vec::new());
    task.tools = vec![AgentTool::new_async(
        "t.slow",
        "Slow",
        "Waits",
        json!({ "type": "object" }),
        move |call: AgentToolCall| {
            let (running, peak) = (running_in.clone(), peak_in.clone());
            async move {
                peak.fetch_max(running.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(150)).await;
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(AgentToolExecution::Completed(AgentToolResult::new(
                    format!("slow {}", call.input["n"]),
                )))
            }
        },
    )
    .parallel_safe()];
    completed(task).await;
    // All three were inside the tool at once.
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    // Results keep the order of the calls.
    let requests = requests.lock().unwrap();
    let results = messages(&requests[1])
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| message["content"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(results, vec!["slow 1", "slow 2", "slow 3"]);
}

#[tokio::test]
async fn a_tool_that_exceeds_its_timeout_fails_and_a_blocking_tool_is_told_to_stop() {
    let (url, requests) =
        scripted_server(vec![tool_call("t_stuck", json!({})), text("gave up")]).await;
    let saw_cancel = Arc::new(Mutex::new(false));
    let saw = saw_cancel.clone();
    let mut task = task(url, Vec::new());
    task.tools = vec![AgentTool::blocking(
        "t.stuck",
        "Stuck",
        "Never returns in time",
        json!({ "type": "object" }),
        move |_call, cancel| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(10));
            }
            *saw.lock().unwrap() = cancel.is_cancelled();
            Ok(AgentToolExecution::Completed(AgentToolResult::new("late")))
        },
    )
    .with_timeout(Duration::from_millis(80))];
    assert_eq!(completed(task).await.final_text, "gave up");
    let content = messages(&requests.lock().unwrap()[1]).last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(content.contains("timed out"), "{content}");
    // The worker thread sees the cancellation shortly after the timeout.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !*saw_cancel.lock().unwrap() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(*saw_cancel.lock().unwrap());
}

#[tokio::test]
async fn a_pause_in_the_middle_of_a_round_resumes_with_every_call_answered() {
    let (url, requests) = scripted_server(vec![
        tool_calls(
            None,
            &[
                ("t_lookup", json!({})),
                ("t_write", json!({ "text": "draft" })),
                ("t_lookup", json!({ "again": true })),
            ],
        ),
        text("finished"),
    ])
    .await;
    let first = run_agent_task_until_pause(&http(), task(url.clone(), Vec::new()), |_| Ok(()))
        .await
        .unwrap();
    let AgentTaskRunResult::Paused(pause) = first else {
        panic!("expected a pause")
    };
    assert_eq!(pause.tool_id, "t.write");
    assert_eq!(pause.ply, 1);
    let call = pause.call.clone().expect("the pending call is recorded");
    assert_eq!((call.ply, call.index), (1, 1));
    // The first call ran; the third is answered as skipped so the round is complete.
    assert_eq!(pause.observations.len(), 2);
    assert_eq!(pause.observations[0].observation, "lookup result");
    assert!(pause.observations[1].observation.starts_with("Not run"));
    assert_eq!(pause.usage.input_tokens, 100);

    let mut resumed = task(url, Vec::new());
    resumed.prior_tool_observations = pause.observations;
    resumed.prior_tool_observations.push(
        AgentToolObservation::resumed(
            pause.tool_id,
            pause.tool_name,
            pause.rationale,
            pause.input,
            pause.pending_message,
            "written",
            json!({}),
        )
        .with_call(pause.call),
    );
    resumed.start_ply = pause.ply + 1;
    assert_eq!(completed(resumed).await.final_text, "finished");

    let requests = requests.lock().unwrap();
    let sent = messages(&requests[1]);
    let assistant = sent
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["tool_calls"].as_array().unwrap().len(), 3);
    // Results are in call order, although the resumed one was appended last.
    let results = sent
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| {
            (
                message["tool_call_id"].as_str().unwrap().to_string(),
                message["content"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results[0],
        ("call-t_lookup-0".into(), "lookup result".into())
    );
    assert_eq!(results[1], ("call-t_write-1".into(), "written".into()));
    assert!(results[2].1.starts_with("Not run"));
}

#[tokio::test]
async fn failures_carry_a_machine_readable_code() {
    let (url, _) = scripted_server(vec![
        Reply::error(401, "{\"error\":\"bad key\"}"),
        Reply::json(
            json!({ "choices": [{ "message": { "content": "cut" }, "finish_reason": "length" }] }),
        ),
        tool_call("t_lookup", json!({})),
    ])
    .await;
    let run =
        |task: AgentTask| async { run_agent_task_until_pause(&http(), task, |_| Ok(())).await };

    let auth = run(task(url.clone(), Vec::new())).await.err().unwrap();
    assert_eq!(auth.code(), "model_auth");
    assert!(auth.to_string().contains("401"));

    // A cut-off answer is returned, flagged with its stop reason.
    let Ok(AgentTaskRunResult::Completed(cut)) = run(task(url.clone(), Vec::new())).await else {
        panic!("expected completion")
    };
    assert_eq!(cut.stop_reason, ModelStopReason::MaxTokens);

    let mut one_step = task(url.clone(), Vec::new());
    one_step.max_iterations = 1;
    assert_eq!(
        run(one_step).await.err().unwrap().code(),
        "step_budget_exhausted"
    );

    let stopped = run_agent_task_until_pause(&http(), task(url, Vec::new()), |_| {
        Err("Stopped.".to_string())
    })
    .await
    .err()
    .unwrap();
    assert_eq!(stopped.code(), "stopped");
}

/// A provider that is not built in: answers from a script and records what it was sent.
struct ScriptedAdapter {
    replies: Mutex<Vec<ModelCompletionResponse>>,
    seen: Arc<Mutex<Vec<Vec<ModelMessage>>>>,
}

#[async_trait::async_trait]
impl ModelAdapter for ScriptedAdapter {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::OpenAiCompatible
    }

    async fn complete(
        &self,
        _http: &Client,
        _config: &ModelConfig,
        request: &ModelCompletionRequest,
        _on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        self.seen.lock().unwrap().push(request.messages.clone());
        Ok(self.replies.lock().unwrap().remove(0))
    }
}

#[tokio::test]
async fn a_task_can_run_on_a_custom_adapter_and_replays_its_opaque_items() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = ScriptedAdapter {
        replies: Mutex::new(vec![
            ModelCompletionResponse {
                raw_output: "thinking aloud".into(),
                tool_calls: vec![ModelToolCall {
                    call_id: None,
                    tool_id: "t.lookup".into(),
                    arguments: json!({ "q": "x" }),
                    provider_meta: json!({ "signature": "s" }),
                }],
                stop_reason: ModelStopReason::ToolCalls,
                provider: "custom".into(),
                provider_items: vec![json!({ "reasoning": "opaque" })],
                ..ModelCompletionResponse::default()
            },
            ModelCompletionResponse {
                raw_output: "{\"ok\":true}".into(),
                ..ModelCompletionResponse::default()
            },
        ]),
        seen: seen.clone(),
    };
    let mut task = task("http://unused".into(), Vec::new());
    task.model_adapter = Some(Arc::new(adapter));
    let outcome = completed(task).await;
    // JSON answers are also available parsed.
    assert_eq!(outcome.final_value, json!({ "ok": true }));

    let seen = seen.lock().unwrap();
    let assistant = seen[1]
        .iter()
        .find(|message| message.role == ModelMessageRole::Assistant)
        .unwrap();
    assert_eq!(assistant.content, "thinking aloud");
    assert_eq!(assistant.provider, "custom");
    assert_eq!(
        assistant.provider_items,
        vec![json!({ "reasoning": "opaque" })]
    );
    assert_eq!(assistant.tool_calls[0].provider_meta["signature"], "s");
    // A call without a provider id gets a generated one, shared with its result.
    let call_id = assistant.tool_calls[0].call_id.clone().unwrap();
    assert_eq!(call_id, "call_1_0");
    let result = seen[1].last().unwrap();
    assert_eq!(result.role, ModelMessageRole::Tool);
    assert_eq!(result.tool_result.as_ref().unwrap().call_id, call_id);
}

#[test]
fn observations_without_a_call_record_are_shown_as_notes() {
    let legacy = AgentToolObservation::resumed(
        "writer.draftScene",
        "Draft",
        "call 1",
        json!({ "sceneIndex": 1 }),
        "drafting",
        "scene 2 written",
        json!({}),
    );
    let runtime = AgentToolObservation::runtime_message("Call a tool first.");
    let messages = transcript_messages(&[legacy, runtime]);
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|m| m.role == ModelMessageRole::User));
    assert!(messages[0].content.contains("writer.draftScene"));
    assert!(messages[0].content.contains("scene 2 written"));
    assert!(messages[1].content.starts_with("[Runtime note]"));
}

#[test]
fn the_request_prefix_does_not_depend_on_progress() {
    let mut task = task("http://unused".into(), Vec::new());
    task.require_tool_before_final = true;
    task.conversation = vec![ModelMessage::assistant("earlier turn")];
    let observation = AgentToolObservation::completed(
        "t.lookup",
        "Lookup",
        "",
        json!({}),
        AgentToolResult::new("found"),
    )
    .with_call(Some(AgentToolCallRecord {
        call_id: "c1".into(),
        ply: 1,
        ..AgentToolCallRecord::default()
    }));

    let before = build_task_messages(&task, &[], false);
    let after = build_task_messages(&task, &[observation], true);
    // System prompt, context, history and the request are the same messages in both.
    for (left, right) in before.iter().zip(&after) {
        assert_eq!(left.role, right.role);
        assert_eq!(left.content, right.content);
    }
    assert!(before[0]
        .content
        .contains("requires at least one successful tool call"));
    assert_eq!(before.last().unwrap().content, "do the job");
    assert_eq!(after.last().unwrap().content, FORCE_FINAL_NOTE);
    assert!(after
        .iter()
        .filter(|message| message.role == ModelMessageRole::System)
        .all(|message| !message.content.contains("found")));
}

#[tokio::test]
async fn dynamic_async_tool_returns_pending_execution() {
    let tool = agent_tool_from_dynamic(DynamicAgentToolDefinition {
        id: "dynamic.ask".into(),
        name: "Ask".into(),
        description: "Ask user".into(),
        input_schema: json!({ "type": "object" }),
        execution: DynamicAgentToolExecution::Async {
            pending_message: "正在执行异步工具".into(),
            metadata: json!({ "interaction": { "type": "ask_user" } }),
        },
    });

    let execution = tool
        .execute(AgentToolCall {
            tool_id: "dynamic.ask".into(),
            rationale: "Need user input".into(),
            input: json!({ "question": "A or B?" }),
        })
        .await
        .expect("dynamic tool should execute");

    match execution {
        AgentToolExecution::Pending(pending) => {
            assert_eq!(pending.pending_message, "正在执行异步工具");
            assert_eq!(pending.metadata["input"]["question"], "A or B?");
        }
        AgentToolExecution::Completed(_) => panic!("expected pending execution"),
    }
}

#[test]
fn dynamic_async_tool_deserializes_camel_case_pending_message() {
    let definition: DynamicAgentToolDefinition = serde_json::from_value(json!({
        "id": "dynamic.ask",
        "name": "Ask",
        "description": "Ask user",
        "inputSchema": { "type": "object" },
        "execution": {
            "type": "async",
            "pendingMessage": "正在等待用户回答问题",
            "metadata": { "interaction": { "type": "ask_user" } }
        }
    }))
    .expect("camelCase dynamic async tool should deserialize");

    match definition.execution {
        DynamicAgentToolExecution::Async {
            pending_message,
            metadata,
        } => {
            assert_eq!(pending_message, "正在等待用户回答问题");
            assert_eq!(metadata["interaction"]["type"], "ask_user");
        }
        DynamicAgentToolExecution::Sync { .. } => panic!("expected async execution"),
    }
}

#[test]
fn observations_saved_by_older_versions_still_load() {
    let observation: AgentToolObservation = serde_json::from_value(json!({
        "tool_id": "t.lookup", "tool_name": "Lookup", "rationale": "", "input": {},
        "pending_message": null, "observation": "old", "metadata": null
    }))
    .unwrap();
    assert!(observation.call.is_none());
    // And a record round-trips, leaving empty fields out.
    let linked = observation.with_call(Some(AgentToolCallRecord {
        call_id: "c".into(),
        ply: 2,
        ..AgentToolCallRecord::default()
    }));
    let saved = serde_json::to_value(&linked).unwrap();
    assert_eq!(
        saved["call"],
        json!({ "call_id": "c", "ply": 2, "index": 0 })
    );
    let loaded: AgentToolObservation = serde_json::from_value(saved).unwrap();
    assert_eq!(loaded.call, linked.call);
}

mod sub_agents {
    use super::*;

    fn writer_spec() -> AgentToolSpec {
        AgentToolSpec {
            id: "drafter".into(),
            name: "Drafter".into(),
            description: String::new(),
            instructions: "drafter instructions".into(),
            tools: vec!["t.lookup".into(), "t.write".into()],
            max_iterations: None,
        }
    }

    #[tokio::test]
    async fn model_creates_an_agent_and_calls_it_as_a_tool() {
        let (url, requests) = scripted_server(vec![
            tool_call("agent_create", json!({ "id": "checker", "name": "Checker", "instructions": "checker instructions", "tools": ["t.lookup"] })),
            tool_call("subagent_checker", json!({ "task": "check the facts" })),
            tool_call("t_lookup", json!({})),
            text("child done"),
            text("parent done"),
        ])
        .await;
        let mut defined = Vec::new();
        let result = run_agent_task_until_pause(&http(), task(url, Vec::new()), |event| {
            if let AgentTaskEvent::AgentDefined { spec, .. } = event {
                defined.push(spec);
            }
            Ok(())
        })
        .await
        .unwrap();
        let AgentTaskRunResult::Completed(outcome) = result else {
            panic!("expected completion")
        };
        assert_eq!(outcome.final_text, "parent done");
        // The sub-agent's requests count towards the run's usage.
        assert_eq!(outcome.usage.input_tokens, 100 * 3 + 120 * 2);
        assert_eq!(defined.len(), 1);
        assert_eq!(defined[0].tools, vec!["t.lookup".to_string()]);

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(tool_names(&requests[1]).contains(&"subagent_checker".to_string()));
        // The sub-agent runs on its own prompt, the parent's context, only its tools and no agent tools.
        let child = &requests[2];
        let child_text = child.body["messages"].to_string();
        assert!(child_text.contains("checker instructions"));
        assert!(child_text.contains("sample text"));
        assert!(child_text.contains("check the facts"));
        assert!(!child_text.contains("parent system"));
        assert_eq!(tool_names(child), vec!["t_lookup".to_string()]);
        // Its answer comes back to the parent as the result of the parent's call.
        let answer = messages(&requests[4]).last().unwrap().clone();
        assert_eq!(answer["role"], "tool");
        assert_eq!(answer["tool_call_id"], "call-subagent_checker-0");
        assert_eq!(answer["content"], "child done");
    }

    #[tokio::test]
    async fn a_sub_agent_waiting_on_a_host_tool_pauses_and_resumes_the_parent() {
        let (url, requests) = scripted_server(vec![
            tool_call("subagent_drafter", json!({ "task": "draft it" })),
            tool_call("t_write", json!({ "text": "draft" })),
            text("drafted"),
            text("all done"),
        ])
        .await;
        let first =
            run_agent_task_until_pause(&http(), task(url.clone(), vec![writer_spec()]), |_| Ok(()))
                .await
                .unwrap();
        let AgentTaskRunResult::Paused(pause) = first else {
            panic!("expected a pause")
        };
        assert_eq!(pause.tool_id, "t.write");
        assert_eq!(pause.ply, 1);
        assert!(pause.observations.is_empty());
        let mut frame = pause.sub_agent.expect("paused inside the sub-agent");
        assert_eq!(frame.agent_id, "drafter");
        assert_eq!(frame.parent_ply, 1);
        assert_eq!(frame.next_ply, 2);
        assert!(frame.parent_call.is_some());

        frame.observations.push(
            AgentToolObservation::resumed(
                pause.tool_id,
                pause.tool_name,
                pause.rationale,
                pause.input,
                pause.pending_message,
                "written",
                json!({}),
            )
            .with_call(pause.call),
        );
        let mut resumed = task(url, vec![writer_spec()]);
        resumed.prior_tool_observations = pause.observations;
        resumed.resume_sub_agent = Some(frame);
        let mut sub_events = 0;
        let second = run_agent_task_until_pause(&http(), resumed, |event| {
            if matches!(event, AgentTaskEvent::SubAgent { .. }) {
                sub_events += 1;
            }
            Ok(())
        })
        .await
        .unwrap();
        let AgentTaskRunResult::Completed(outcome) = second else {
            panic!("expected completion")
        };
        assert_eq!(outcome.final_text, "all done");
        assert!(sub_events > 0);
        let requests = requests.lock().unwrap();
        let child_result = messages(&requests[2]).last().unwrap().clone();
        assert_eq!(child_result["role"], "tool");
        assert_eq!(child_result["content"], "written");
        let parent_final = requests[3].body["messages"].to_string();
        assert!(parent_final.contains("parent system"));
        let parent_result = messages(&requests[3]).last().unwrap().clone();
        assert_eq!(parent_result["tool_call_id"], "call-subagent_drafter-0");
        assert_eq!(parent_result["content"], "drafted");
    }

    #[test]
    fn agent_definitions_are_checked_against_the_available_tools() {
        let tools = vec!["t.lookup".to_string()];
        let mut spec = writer_spec();
        assert!(validate_agent_tool_spec(&spec, &tools, &[])
            .unwrap_err()
            .contains("t.write"));
        spec.tools = vec!["t.lookup".into()];
        assert!(validate_agent_tool_spec(&spec, &tools, &[]).is_ok());
        spec.id = "bad id".into();
        assert!(validate_agent_tool_spec(&spec, &tools, &[]).is_err());
        let full = (0..MAX_SUB_AGENTS)
            .map(|index| AgentToolSpec {
                id: format!("a{index}"),
                ..writer_spec()
            })
            .collect::<Vec<_>>();
        let extra = AgentToolSpec {
            id: "extra".into(),
            tools: Vec::new(),
            ..writer_spec()
        };
        assert!(validate_agent_tool_spec(&extra, &tools, &full).is_err());
        let replacing = AgentToolSpec {
            id: "a0".into(),
            tools: Vec::new(),
            ..writer_spec()
        };
        assert!(validate_agent_tool_spec(&replacing, &tools, &full).is_ok());
    }
}
