//! Tools offered to a task: executable tools, dynamic plugin tools, calls, results and observations.
use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Runs one tool call. Implement this for tools that need full control; the constructors on
/// [`AgentTool`] cover closures.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, call: AgentToolCall) -> Result<AgentToolExecution, String>;
}

/// Tells a blocking tool that its result is no longer wanted (the run was stopped or the
/// tool timed out), so long loops can end early.
#[derive(Clone, Default)]
pub struct ToolCancel(Arc<AtomicBool>);

impl ToolCancel {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct CancelOnDrop(ToolCancel);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0 .0.store(true, Ordering::Relaxed);
    }
}

type SyncExecutor = dyn Fn(AgentToolCall) -> Result<AgentToolExecution, String> + Send + Sync;
type BlockingExecutor =
    dyn Fn(AgentToolCall, &ToolCancel) -> Result<AgentToolExecution, String> + Send + Sync;
type ToolFuture = Pin<Box<dyn Future<Output = Result<AgentToolExecution, String>> + Send>>;

struct InlineHandler(Arc<SyncExecutor>);

#[async_trait]
impl ToolHandler for InlineHandler {
    async fn call(&self, call: AgentToolCall) -> Result<AgentToolExecution, String> {
        (self.0)(call)
    }
}

struct BlockingHandler(Arc<BlockingExecutor>);

#[async_trait]
impl ToolHandler for BlockingHandler {
    async fn call(&self, call: AgentToolCall) -> Result<AgentToolExecution, String> {
        let executor = Arc::clone(&self.0);
        let cancel = ToolCancel::default();
        // Dropping this future (a stop or a timeout) raises the flag for the worker thread.
        let _guard = CancelOnDrop(cancel.clone());
        tokio::task::spawn_blocking(move || executor(call, &cancel))
            .await
            .map_err(|error| format!("Tool crashed: {error}"))?
    }
}

struct AsyncHandler<F>(F);

#[async_trait]
impl<F> ToolHandler for AsyncHandler<F>
where
    F: Fn(AgentToolCall) -> ToolFuture + Send + Sync,
{
    async fn call(&self, call: AgentToolCall) -> Result<AgentToolExecution, String> {
        (self.0)(call).await
    }
}

#[derive(Clone)]
pub struct AgentTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// The call fails with a timeout error after this long.
    pub timeout: Option<Duration>,
    /// Whether calls to this tool may run at the same time as other such calls of one model
    /// response. Set it only for tools without side effects.
    pub parallel_safe: bool,
    handler: Arc<dyn ToolHandler>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DynamicAgentToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub execution: DynamicAgentToolExecution,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// A tool run by a custom [`ToolHandler`].
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        handler: impl ToolHandler + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            input_schema,
            timeout: None,
            parallel_safe: false,
            handler: Arc::new(handler),
        }
    }

    /// A tool whose closure returns immediately (no I/O): it runs inline on the async runtime.
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
        Self::new(
            id,
            name,
            description,
            input_schema,
            InlineHandler(Arc::new(executor)),
        )
    }

    /// A tool that does blocking work (file system, CPU): it runs on the blocking thread pool
    /// and should check `cancel` in long loops.
    pub fn blocking<F>(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        executor: F,
    ) -> Self
    where
        F: Fn(AgentToolCall, &ToolCancel) -> Result<AgentToolExecution, String>
            + Send
            + Sync
            + 'static,
    {
        Self::new(
            id,
            name,
            description,
            input_schema,
            BlockingHandler(Arc::new(executor)),
        )
    }

    /// A tool that awaits (network, subprocesses). Dropping the run cancels the future.
    pub fn new_async<F, Fut>(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        executor: F,
    ) -> Self
    where
        F: Fn(AgentToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AgentToolExecution, String>> + Send + 'static,
    {
        Self::new(
            id,
            name,
            description,
            input_schema,
            AsyncHandler(move |call| Box::pin(executor(call)) as ToolFuture),
        )
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn parallel_safe(mut self) -> Self {
        self.parallel_safe = true;
        self
    }

    pub async fn execute(&self, call: AgentToolCall) -> Result<AgentToolExecution, String> {
        match self.timeout {
            Some(limit) => tokio::time::timeout(limit, self.handler.call(call))
                .await
                .unwrap_or_else(|_| {
                    Err(format!(
                        "{} timed out after {}s.",
                        self.name,
                        limit.as_secs().max(1)
                    ))
                }),
            None => self.handler.call(call).await,
        }
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

#[derive(Clone, Serialize, Deserialize)]
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

/// How one tool call appeared in the model's response, kept with its result so the round can
/// be sent back to the model in the provider's native tool-call format.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentToolCallRecord {
    /// The provider's id of the call, or a generated one.
    pub call_id: String,
    /// The model request that made the call; calls of one response share it.
    pub ply: usize,
    /// Position of the call among the calls of its response.
    #[serde(default)]
    pub index: usize,
    /// Text the model wrote alongside the calls. Stored on the first call of the response.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub assistant_text: String,
    /// Wire format that produced the provider fields.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    /// Opaque items of the response (reasoning, thinking) the provider wants back unchanged.
    /// Stored on the first call of the response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_items: Vec<Value>,
    /// Opaque data of this call (e.g. a thought signature).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub provider_meta: Value,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentToolObservation {
    pub tool_id: String,
    pub tool_name: String,
    pub rationale: String,
    pub input: Value,
    pub pending_message: Option<String>,
    pub observation: String,
    pub metadata: Value,
    /// Absent on runtime notes, summaries and observations saved by older versions; those are
    /// shown to the model as plain text instead of a native tool round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub call: Option<AgentToolCallRecord>,
}

impl AgentToolObservation {
    pub fn runtime_message(content: impl Into<String>) -> Self {
        Self {
            tool_id: RUNTIME_TOOL_ID.into(),
            tool_name: "Daily runtime".into(),
            rationale: "Runtime control message.".into(),
            input: Value::Null,
            pending_message: None,
            observation: content.into(),
            metadata: json!({ "source": "runtime" }),
            call: None,
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
            call: None,
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
            call: None,
        }
    }

    /// Links the observation to the model's call, so it replays as a native tool result.
    pub fn with_call(mut self, call: Option<AgentToolCallRecord>) -> Self {
        self.call = call;
        self
    }

    /// Marks the result as a failure for the model.
    pub fn failed(mut self) -> Self {
        if let Some(call) = self.call.as_mut() {
            call.is_error = true;
        }
        self
    }
}

/// Tool id of observations written by the runtime itself rather than a tool.
pub const RUNTIME_TOOL_ID: &str = "runtime";
