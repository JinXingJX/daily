//! Offline workflow and durable-run examples, independent of any desktop host.
use daily_agent_core::{
    checkpoint::{AgentTaskState, ResumeAction},
    model::*,
    supervisor::RunSupervisor,
    workflow::*,
    *,
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

struct OfflineModel(&'static str);
#[async_trait::async_trait]
impl Model for OfflineModel {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity {
            provider: "offline".into(),
            model: self.0.into(),
        }
    }
    async fn complete(
        &self,
        request: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        if request.tools.iter().any(|tool| tool.id == "save")
            && !request.messages.iter().any(|m| m.tool_result.is_some())
        {
            return Ok(ModelCompletionResponse {
                tool_calls: vec![ModelToolCall {
                    call_id: Some("save-1".into()),
                    tool_id: "save".into(),
                    arguments: json!({"text":"reviewed draft"}),
                    ..Default::default()
                }],
                stop_reason: ModelStopReason::ToolCalls,
                ..Default::default()
            });
        }
        Ok(ModelCompletionResponse {
            raw_output: format!("{} completed its work", self.0),
            ..Default::default()
        })
    }
}
fn agent(id: &str, model: &str) -> AgentDefinition {
    AgentDefinition::new(AgentToolSpec {
        id: id.into(),
        name: id.into(),
        description: id.into(),
        instructions: format!("Work as {id}"),
        tools: vec![],
        max_iterations: Some(4),
    })
    .with_model(model)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bindings = Bindings::default();
    bindings
        .models
        .insert("research".into(), Arc::new(OfflineModel("research-model")));
    bindings
        .models
        .insert("review".into(), Arc::new(OfflineModel("review-model")));
    let workflow = Workflow {
        agents: BTreeMap::from([
            ("researcher".into(), agent("researcher", "research")),
            ("reviewer".into(), agent("reviewer", "review")),
        ]),
        nodes: vec![
            WorkflowNode {
                id: "research".into(),
                agent_id: "researcher".into(),
                prompt: "Research the design".into(),
                dependencies: vec![],
            },
            WorkflowNode {
                id: "check".into(),
                agent_id: "reviewer".into(),
                prompt: "Check constraints".into(),
                dependencies: vec![],
            },
            WorkflowNode {
                id: "synthesis".into(),
                agent_id: "reviewer".into(),
                prompt: "Combine the findings".into(),
                dependencies: vec!["research".into(), "check".into()],
            },
        ],
        failure_policy: FailurePolicy::FailFast,
    };
    let state = workflow
        .run(
            &bindings,
            RunContext::new("workflow", RunLimits::default()),
            None,
        )
        .await?;
    println!(
        "Workflow completed {} nodes with {} model requests",
        state.results.len(),
        state.budget.model_requests
    );

    let mut task = AgentTask::new("durable", bindings.model("review")?, "Save the draft");
    task.tools
        .push(agent_tool_from_dynamic(DynamicAgentToolDefinition {
            id: "save".into(),
            name: "Save".into(),
            description: "Host saves a draft".into(),
            input_schema: json!({"type":"object"}),
            execution: DynamicAgentToolExecution::Async {
                pending_message: "Waiting for host".into(),
                metadata: serde_json::Value::Null,
            },
        }));
    let bindings = task.bindings();
    let store = Arc::new(checkpoint::MemoryCheckpointStore::default());
    let supervisor = RunSupervisor::with_store(store);
    let handle = supervisor.start(task)?;
    let context = handle.context.clone();
    let AgentTaskRunResult::Paused(_) = handle.wait().await? else {
        return Err("Expected a pause".into());
    };
    let checkpoint = context.checkpoint().ok_or("Missing checkpoint")?;
    // State contains binding identities; executable models, tools and credentials stay outside it.
    let _: AgentTaskState = serde_json::from_str(&serde_json::to_string(&checkpoint.task)?)?;
    let resumed = supervisor
        .resume(
            &context.identity.run_id,
            &bindings,
            ResumeAction::ToolResult {
                call_id: "save-1".into(),
                result: AgentToolResult::new("saved"),
                is_error: false,
            },
        )
        .await?;
    let AgentTaskRunResult::Completed(result) = resumed.wait().await? else {
        return Err("Expected completion".into());
    };
    println!("Durable run resumed: {}", result.final_text);
    Ok(())
}
