use super::*;
use model::{ModelCompletionResponse, ModelIdentity, ModelMessageRole};
use std::{collections::VecDeque, sync::Mutex};

struct ScriptedModel {
    replies: Mutex<VecDeque<ModelCompletionResponse>>,
    requests: Mutex<Vec<ModelCompletionRequest>>,
}

impl ScriptedModel {
    fn new(replies: Vec<ModelCompletionResponse>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl Model for ScriptedModel {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity {
            provider: "custom.offline".into(),
            model: "scripted".into(),
        }
    }
    async fn complete(
        &self,
        request: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(crate::model::ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        self.requests.lock().unwrap().push(request.clone());
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

fn call(id: &str, tool: &str, input: Value) -> ModelCompletionResponse {
    ModelCompletionResponse {
        tool_calls: vec![ModelToolCall {
            call_id: Some(id.into()),
            tool_id: tool.into(),
            arguments: input,
            ..Default::default()
        }],
        stop_reason: ModelStopReason::ToolCalls,
        provider: "custom.offline".into(),
        provider_items: vec![json!({ "opaque": id })],
        ..Default::default()
    }
}

fn child() -> AgentToolSpec {
    AgentToolSpec {
        id: "reviewer".into(),
        name: "Reviewer".into(),
        description: "Review a draft".into(),
        instructions: "Review precisely".into(),
        tools: vec!["draft.write".into()],
        max_iterations: Some(4),
    }
}

fn task(model: Arc<dyn Model>) -> AgentTask {
    let mut task = AgentTask::new("offline", model, "Review this draft");
    task.observation_budget = None;
    task.sub_agents.agents = vec![child()];
    task.tools = vec![agent_tool_from_dynamic(DynamicAgentToolDefinition {
        id: "draft.write".into(),
        name: "Write".into(),
        description: "Host writes a draft".into(),
        input_schema: json!({"type":"object"}),
        execution: DynamicAgentToolExecution::Async {
            pending_message: "Waiting for host".into(),
            metadata: Value::Null,
        },
    })];
    task
}

#[tokio::test]
async fn custom_model_delegates_and_preserves_provider_items_without_desktop_types() {
    let model = ScriptedModel::new(vec![
        call("parent-call", "subagent.reviewer", json!({"task":"review"})),
        answer("Reviewed"),
        answer("Complete"),
    ]);
    let mut child_events = 0;
    let result = run_agent_task(task(model.clone()), |event| {
        if let AgentTaskEvent::ModelRequestStarted { provider, .. } = &event {
            assert_eq!(provider, "custom.offline");
        }
        if matches!(event, AgentTaskEvent::SubAgent { .. }) {
            child_events += 1;
        }
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(result.final_text, "Complete");
    assert_eq!(result.usage.input_tokens, 20);
    assert!(child_events > 0);
    let requests = model.requests.lock().unwrap();
    assert!(requests[1].messages[0].content.contains("Review precisely"));
    let parent = &requests[2].messages;
    let round = parent.iter().find(|m| !m.tool_calls.is_empty()).unwrap();
    assert_eq!(round.provider_items, vec![json!({"opaque":"parent-call"})]);
    assert_eq!(parent.last().unwrap().role, ModelMessageRole::Tool);
    assert_eq!(parent.last().unwrap().content, "Reviewed");
}

#[tokio::test]
async fn serialized_child_pause_resumes_without_replaying_the_parent_call() {
    let model = ScriptedModel::new(vec![
        call("delegate", "subagent.reviewer", json!({"task":"write"})),
        call("write", "draft.write", json!({"text":"draft"})),
        answer("Written"),
        answer("Done"),
    ]);
    let AgentTaskRunResult::Paused(pause) =
        run_agent_task_until_pause(task(model.clone()), |_| Ok(()))
            .await
            .unwrap()
    else {
        panic!("expected pause")
    };
    let mut frame: SubAgentFrame =
        serde_json::from_value(serde_json::to_value(pause.sub_agent.unwrap()).unwrap()).unwrap();
    let observation = AgentToolObservation::resumed(
        pause.tool_id,
        pause.tool_name,
        pause.rationale,
        pause.input,
        pause.pending_message,
        "saved",
        Value::Null,
    )
    .with_call(pause.call);
    frame.observations.push(observation);
    let mut resumed = task(model.clone());
    resumed.prior_tool_observations = pause.observations;
    resumed.resume_sub_agent = Some(frame);
    assert_eq!(
        run_agent_task(resumed, |_| Ok(()))
            .await
            .unwrap()
            .final_text,
        "Done"
    );
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[2].messages.last().unwrap().content, "saved");
    assert_eq!(
        requests[3]
            .messages
            .last()
            .unwrap()
            .tool_result
            .as_ref()
            .unwrap()
            .call_id,
        "delegate"
    );
}

#[tokio::test]
async fn cancellation_before_request_never_calls_the_model() {
    let model = ScriptedModel::new(vec![answer("unused")]);
    let result = run_agent_task(task(model.clone()), |_| Err("stop".into())).await;
    assert!(matches!(result, Err(AgentError::Stopped(message)) if message == "stop"));
    assert!(model.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_errors_keep_their_kind_across_the_core_boundary() {
    let model = ScriptedModel::new(Vec::new());
    let result = run_agent_task(task(model), |_| Ok(())).await;
    assert!(
        matches!(result, Err(AgentError::Model(error)) if error.kind == model::ModelErrorKind::Protocol)
    );
}
