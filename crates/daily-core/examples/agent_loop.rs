//! Embedding the Daily agent loop: a custom model provider, an async tool, and a run.
//!
//! Runs offline: the "model" below is a script. To use a real provider, drop the
//! `model_adapter` line and point `ModelConfig::for_provider` at it, for example
//! `ModelProviderKind::AnthropicMessages`, `https://api.anthropic.com/v1`, a model id and
//! your API key.
//!
//! ```sh
//! cargo run -p daily-core --example agent_loop
//! ```
use daily_core::sdk::model::{
    ModelAdapter, ModelCompletionRequest, ModelCompletionResponse, ModelConfig, ModelError,
    ModelMessageRole, ModelProviderKind, ModelStopReason, ModelStreamChunk, ModelToolCall,
    ModelUsage,
};
use daily_core::sdk::{
    run_agent_task, AgentTask, AgentTaskEvent, AgentTool, AgentToolCall, AgentToolExecution,
    AgentToolResult,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// A provider Daily does not ship. A real one would send `request.messages` to its API and
/// map the answer back; this one asks for the weather once, then answers.
struct ScriptedModel;

#[async_trait::async_trait]
impl ModelAdapter for ScriptedModel {
    fn kind(&self) -> ModelProviderKind {
        ModelProviderKind::OpenAiCompatible
    }

    async fn complete(
        &self,
        _http: &reqwest::Client,
        _config: &ModelConfig,
        request: &ModelCompletionRequest,
        _on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, ModelError> {
        // Tool results come back as their own messages, after the assistant turn that
        // called the tools.
        let weather = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == ModelMessageRole::Tool);
        let usage = Some(ModelUsage {
            input_tokens: 40,
            output_tokens: 12,
            ..ModelUsage::default()
        });
        Ok(match weather {
            None => ModelCompletionResponse {
                raw_output: "Let me check.".into(),
                tool_calls: vec![ModelToolCall {
                    call_id: Some("call_1".into()),
                    tool_id: "weather.lookup".into(),
                    arguments: json!({ "city": "Paris" }),
                    ..ModelToolCall::default()
                }],
                stop_reason: ModelStopReason::ToolCalls,
                usage,
                ..ModelCompletionResponse::default()
            },
            Some(result) => ModelCompletionResponse {
                raw_output: format!("In Paris it is {}.", result.content),
                usage,
                ..ModelCompletionResponse::default()
            },
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // An async tool: awaiting is fine, and stopping the run cancels it. `parallel_safe` lets
    // several calls of one model response run at once; set it only for tools that just read.
    let weather = AgentTool::new_async(
        "weather.lookup",
        "Look up weather",
        "Returns the current weather of a city.",
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
        |call: AgentToolCall| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let city = call.input["city"].as_str().unwrap_or("there");
            Ok(AgentToolExecution::Completed(AgentToolResult::new(
                format!("18°C and sunny in {city}"),
            )))
        },
    )
    .parallel_safe()
    .with_timeout(Duration::from_secs(10));

    let model = ModelConfig::for_provider(
        ModelProviderKind::OpenAiCompatible,
        "http://unused.invalid/v1",
        "scripted",
        "",
    );
    let mut task = AgentTask::new("example", model, "What is the weather in Paris?");
    task.system_prompt = "You are a concise assistant.".into();
    task.tools = vec![weather];
    task.model_adapter = Some(Arc::new(ScriptedModel));

    let outcome = run_agent_task(&reqwest::Client::new(), task, |event| {
        match event {
            AgentTaskEvent::ToolSelected { tool_id, input, .. } => println!("→ {tool_id} {input}"),
            AgentTaskEvent::ToolResultProduced { observation, .. } => println!("← {observation}"),
            _ => {}
        }
        // Returning an error here stops the run.
        Ok(())
    })
    .await?;

    println!("{}", outcome.final_text);
    println!(
        "{} input and {} output tokens",
        outcome.usage.input_tokens, outcome.usage.output_tokens
    );
    Ok(())
}
