//! A complete multi-agent program without a desktop app, HTTP, or credentials.
//! Run: cargo run -p daily-agent-core --example multi_agent
use daily_agent_core::{
    model::{
        Model, ModelCompletionRequest, ModelCompletionResponse, ModelError, ModelIdentity,
        ModelStopReason, ModelStreamChunk, ModelToolCall,
    },
    run_agent_task, AgentTask, AgentTaskEvent, AgentToolSpec,
};
use serde_json::json;
use std::sync::Arc;

struct OfflineModel;

#[async_trait::async_trait]
impl Model for OfflineModel {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity {
            provider: "example.offline".into(),
            model: "scripted".into(),
        }
    }

    async fn complete(
        &self,
        request: &ModelCompletionRequest,
        _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        if request.request_name.as_deref() == Some("demo.reviewer") {
            return Ok(ModelCompletionResponse {
                raw_output: "Reviewed: the implementation should preserve desktop behavior.".into(),
                ..Default::default()
            });
        }
        if let Some(result) = request
            .messages
            .iter()
            .find(|message| message.tool_result.is_some())
        {
            return Ok(ModelCompletionResponse {
                raw_output: format!("Review complete. {}", result.content),
                ..Default::default()
            });
        }
        Ok(ModelCompletionResponse {
            tool_calls: vec![ModelToolCall {
                call_id: Some("review-1".into()),
                tool_id: "subagent.reviewer".into(),
                arguments: json!({"task":"Review the SDK extraction plan"}),
                ..Default::default()
            }],
            stop_reason: ModelStopReason::ToolCalls,
            ..Default::default()
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut task = AgentTask::new(
        "demo",
        Arc::new(OfflineModel),
        "Ask the reviewer to check the plan",
    );
    task.sub_agents.agents.push(AgentToolSpec {
        id: "reviewer".into(),
        name: "Reviewer".into(),
        description: "Review implementation plans".into(),
        instructions: "Review the plan and return concrete findings.".into(),
        tools: Vec::new(),
        max_iterations: Some(4),
    });
    let result = run_agent_task(task, |event| {
        if let AgentTaskEvent::SubAgent { agent_name, .. } = event {
            println!("{agent_name}: progress");
        }
        Ok(())
    })
    .await?;
    println!("{}", result.final_text);
    Ok(())
}
