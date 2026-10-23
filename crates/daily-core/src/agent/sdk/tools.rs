//! Tools offered to a task: executable tools, dynamic plugin tools, calls, results and observations.
use super::*;

pub(super) type ToolExecutor =
    dyn Fn(AgentToolCall) -> Result<AgentToolExecution, String> + Send + Sync;

#[derive(Clone)]
pub struct AgentTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    executor: Arc<ToolExecutor>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DynamicAgentToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub execution: DynamicAgentToolExecution,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicAgentToolExecution {
    Sync {
        observation: String,
        #[serde(default)]
        metadata: Value,
    },
    Async {
        #[serde(rename = "pendingMessage", alias = "pending_message")]
        pending_message: String,
        #[serde(default)]
        metadata: Value,
    },
}

pub fn agent_tool_from_dynamic(definition: DynamicAgentToolDefinition) -> AgentTool {
    let id = definition.id;
    let name = definition.name;
    let description = definition.description;
    let input_schema = definition.input_schema;
    let execution = definition.execution;
    AgentTool::new_with_execution(
        id,
        name,
        description,
        input_schema,
        move |call: AgentToolCall| match &execution {
            DynamicAgentToolExecution::Sync {
                observation,
                metadata,
            } => Ok(AgentToolExecution::Completed(
                AgentToolResult::with_metadata(
                    observation.clone(),
                    json!({
                        "dynamicTool": true,
                        "input": call.input,
                        "metadata": metadata,
                    }),
                ),
            )),
            DynamicAgentToolExecution::Async {
                pending_message,
                metadata,
            } => Ok(AgentToolExecution::Pending(
                AgentAsyncToolPending::with_metadata(
                    pending_message.clone(),
                    json!({
                        "dynamicTool": true,
                        "input": call.input,
                        "metadata": metadata,
                    }),
                ),
            )),
        },
    )
}

impl AgentTool {
    pub fn new_with_execution<F>(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        executor: F,
    ) -> Self
    where
        F: Fn(AgentToolCall) -> Result<AgentToolExecution, String> + Send + Sync + 'static,
    {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            input_schema,
            executor: Arc::new(executor),
        }
    }

    pub fn execute(&self, call: AgentToolCall) -> Result<AgentToolExecution, String> {
        (self.executor)(call)
    }
}

#[derive(Clone)]
pub struct AgentToolCall {
    pub tool_id: String,
    pub rationale: String,
    pub input: Value,
}

pub enum AgentToolExecution {
    Completed(AgentToolResult),
    Pending(AgentAsyncToolPending),
}

pub struct AgentAsyncToolPending {
    pub pending_message: String,
    pub metadata: Value,
}

impl AgentAsyncToolPending {
    pub fn with_metadata(pending_message: impl Into<String>, metadata: Value) -> Self {
        Self {
            pending_message: pending_message.into(),
            metadata,
        }
    }
}

pub struct AgentToolResult {
    pub observation: String,
    pub metadata: Value,
}

impl AgentToolResult {
    pub fn new(observation: impl Into<String>) -> Self {
        Self {
            observation: observation.into(),
            metadata: Value::Null,
        }
    }

    pub fn with_metadata(observation: impl Into<String>, metadata: Value) -> Self {
        Self {
            observation: observation.into(),
            metadata,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AgentToolObservation {
    pub tool_id: String,
    pub tool_name: String,
    pub rationale: String,
    pub input: Value,
    pub pending_message: Option<String>,
    pub observation: String,
    pub metadata: Value,
}

impl AgentToolObservation {
    pub fn runtime_message(content: impl Into<String>) -> Self {
        Self {
            tool_id: "runtime".into(),
            tool_name: "Daily runtime".into(),
            rationale: "Runtime control message.".into(),
            input: Value::Null,
            pending_message: None,
            observation: content.into(),
            metadata: json!({ "source": "runtime" }),
        }
    }

    pub fn completed(
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        rationale: impl Into<String>,
        input: Value,
        result: AgentToolResult,
    ) -> Self {
        Self {
            tool_id: tool_id.into(),
            tool_name: tool_name.into(),
            rationale: rationale.into(),
            input,
            pending_message: None,
            observation: result.observation,
            metadata: result.metadata,
        }
    }

    pub fn resumed(
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        rationale: impl Into<String>,
        input: Value,
        pending_message: impl Into<String>,
        observation: impl Into<String>,
        metadata: Value,
    ) -> Self {
        Self {
            tool_id: tool_id.into(),
            tool_name: tool_name.into(),
            rationale: rationale.into(),
            input,
            pending_message: Some(pending_message.into()),
            observation: observation.into(),
            metadata,
        }
    }
}
