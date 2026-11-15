//! Bind a provider to the transport-neutral agent SDK.
use crate::{adapter_for, complete_with_retry, ModelAdapter, ModelCompletionRequest, ModelConfig};
use daily_agent_core::model::{Model, ModelCompletionResponse, ModelError, ModelIdentity};
use std::sync::Arc;

/// A configured provider implementing the SDK's `Model` contract.
///
/// Credentials and HTTP configuration stay here, outside agent state and checkpoints.
pub struct ProviderModel {
    http: reqwest::Client,
    config: ModelConfig,
    adapter: Arc<dyn ModelAdapter>,
    session_id: Option<u64>,
    turn_id: Option<u64>,
}

impl ProviderModel {
    pub fn new(http: reqwest::Client, config: ModelConfig) -> Self {
        let adapter = Arc::from(adapter_for(config.provider_kind));
        Self::with_adapter(http, config, adapter)
    }

    pub fn with_adapter(
        http: reqwest::Client,
        config: ModelConfig,
        adapter: Arc<dyn ModelAdapter>,
    ) -> Self {
        Self {
            http,
            config,
            adapter,
            session_id: None,
            turn_id: None,
        }
    }

    /// Optional legacy conversation identifiers for provider request correlation.
    pub fn with_request_context(mut self, session_id: Option<u64>, turn_id: Option<u64>) -> Self {
        self.session_id = session_id;
        self.turn_id = turn_id;
        self
    }
}

#[async_trait::async_trait]
impl Model for ProviderModel {
    fn identity(&self) -> ModelIdentity {
        crate::model_identity(&self.config)
    }

    fn supports_images(&self) -> bool {
        self.adapter.supports_images()
    }
    fn supports_streaming(&self) -> bool {
        self.adapter.supports_streaming()
    }

    async fn complete(
        &self,
        request: &daily_agent_core::model::ModelCompletionRequest,
        on_chunk: Option<
            &mut (dyn FnMut(daily_agent_core::model::ModelStreamChunk) -> Result<(), String>
                      + Send),
        >,
    ) -> Result<ModelCompletionResponse, ModelError> {
        let (session_id, turn_id) = match request.purpose {
            daily_agent_core::model::ModelRequestPurpose::Generation => {
                (self.session_id, self.turn_id)
            }
            daily_agent_core::model::ModelRequestPurpose::ContextCompaction => (None, None),
        };
        complete_with_retry(
            self.adapter.as_ref(),
            &self.http,
            &self.config,
            &ModelCompletionRequest {
                messages: request.messages.clone(),
                temperature: request.temperature,
                max_output_tokens: request.max_output_tokens,
                tools: request.tools.clone(),
                tool_choice: request.tool_choice,
                session_id,
                turn_id,
                request_name: request.request_name.clone(),
            },
            on_chunk,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_support::{scripted_server, Reply},
        ModelProviderKind,
    };
    use daily_agent_core::{
        run_agent_task, AgentTask, AgentTool, AgentToolExecution, AgentToolResult,
    };
    use serde_json::json;

    #[tokio::test]
    async fn sdk_runs_through_the_bound_http_provider_and_replays_native_calls() {
        let (url, requests) = scripted_server(vec![
            Reply::json(json!({"choices":[{"message":{"content":null,"tool_calls":[{"id":"lookup-1","type":"function","function":{"name":"fixture_lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})),
            Reply::json(json!({"choices":[{"message":{"content":"Found it"},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":3}})),
        ]).await;
        let provider = ProviderModel::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            ModelConfig::for_provider(ModelProviderKind::OpenAiCompatible, url, "fixture", "dummy"),
        );
        let mut task = AgentTask::new("provider-test", Arc::new(provider), "Look it up");
        task.stream = false;
        task.tools = vec![AgentTool::new_with_execution(
            "fixture.lookup",
            "Lookup",
            "Read a value",
            json!({"type":"object"}),
            |_| Ok(AgentToolExecution::Completed(AgentToolResult::new("42"))),
        )];
        let result = run_agent_task(task, |_| Ok(())).await.unwrap();
        assert_eq!(result.final_text, "Found it");
        assert_eq!(result.usage.input_tokens, 20);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].head.contains("authorization: bearer dummy"));
        let messages = requests[1].body["messages"].as_array().unwrap();
        let tool = messages.last().unwrap();
        assert_eq!(tool["role"], "tool");
        assert_eq!(tool["tool_call_id"], "lookup-1");
        assert_eq!(tool["content"], "42");
    }

    #[tokio::test]
    async fn bound_provider_keeps_transport_retry_policy() {
        let (url, requests) = scripted_server(vec![
            Reply::error(429, "slow down"),
            Reply::json(
                json!({"choices":[{"message":{"content":"Recovered"},"finish_reason":"stop"}]}),
            ),
        ])
        .await;
        let provider = ProviderModel::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            ModelConfig::for_provider(ModelProviderKind::OpenAiCompatible, url, "fixture", "dummy"),
        );
        let mut task = AgentTask::new("retry-test", Arc::new(provider), "Hello");
        task.stream = false;
        assert_eq!(
            run_agent_task(task, |_| Ok(())).await.unwrap().final_text,
            "Recovered"
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[cfg(not(feature = "login"))]
    #[tokio::test]
    async fn login_adapters_require_an_explicit_feature() {
        let provider = ProviderModel::new(reqwest::Client::new(), ModelConfig::seeded());
        let result = run_agent_task(
            AgentTask::new("login-disabled", Arc::new(provider), "Hello"),
            |_| Ok(()),
        )
        .await;
        assert!(
            matches!(result, Err(daily_agent_core::AgentError::Model(error)) if error.kind == crate::ModelErrorKind::InvalidRequest)
        );
    }
}
