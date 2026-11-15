//! The agent loop: a model, tools, and the transcript that connects them.
//!
//! A caller describes a run with an [`AgentTask`] and drives it with
//! [`run_agent_task_until_pause`]. The loop sends the conversation to the model, executes the
//! tools it calls, and sends each round back in the provider's native tool-call format until
//! the model answers, the step budget runs out, or a tool needs outside work and the run
//! pauses. It knows nothing about chat sessions, plugins or UI.
pub mod checkpoint;
pub mod compaction;
pub mod definition;
pub mod execution;
pub mod supervisor;
pub mod workflow;
pub use definition::{AgentDefinition, Bindings};
pub use execution::{
    BudgetSnapshot, Cancellation, RunContext, RunEvent, RunEventEnvelope, RunIdentity, RunLimits,
    RunStatus,
};
pub mod model;
use model::{
    Model, ModelCompletionRequest, ModelError, ModelInlineImage, ModelMessage, ModelStopReason,
    ModelStreamChunk, ModelToolCall, ModelToolChoice, ModelToolDefinition, ModelUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fmt, future::Future, pin::Pin, sync::Arc};

#[cfg(test)]
mod framework_tests;
mod run_loop;
mod sub_agents;
#[cfg(test)]
mod tests;
mod tools;

pub use run_loop::{run_agent_task, run_agent_task_until_pause};
/// Build and inspect the provider-neutral conversation sent to a model.
pub mod transcript {
    pub use crate::run_loop::{build_task_messages, transcript_messages};
}
pub use sub_agents::{
    upsert_agent_tool_spec, validate_agent_tool_spec, AgentToolSpec, SubAgentConfig, SubAgentFrame,
    AGENT_CREATE_TOOL_ID, AGENT_HANDOFF_TOOL_ID, MAX_SUB_AGENTS, MAX_SUB_AGENT_INSTRUCTIONS_BYTES,
    SUB_AGENT_TOOL_PREFIX,
};
pub use tools::{
    agent_tool_from_dynamic, AgentAsyncToolPending, AgentTool, AgentToolCall, AgentToolCallRecord,
    AgentToolExecution, AgentToolObservation, AgentToolResult, DynamicAgentToolDefinition,
    DynamicAgentToolExecution, ToolCancel, ToolHandler, RUNTIME_TOOL_ID,
};

pub(crate) const SDK_LOOP_PROMPT: &str = r#"You are running inside the Daily agent runtime.

Rules:
- Call a tool only when it is needed; otherwise answer.
- Treat tool results as authoritative runtime state.
- Do not claim a tool changed state unless its result says it did.
- When done, answer normally and follow the task's final instructions."#;

pub(crate) const FORCE_FINAL_NOTE: &str =
    "The step budget for this request is used up. Give the final answer now, without calling tools.";

#[derive(Clone)]
pub struct AgentTask {
    pub execution: RunContext,
    pub bindings: Bindings,
    pub agent_definitions: std::collections::BTreeMap<String, AgentDefinition>,
    pub pending_round: Option<checkpoint::PendingRound>,
    pub name: String,
    pub model: Arc<dyn Model>,
    pub system_prompt: String,
    pub user_prompt: String,
    pub final_instructions: String,
    pub context: Vec<AgentContextBlock>,
    pub conversation: Vec<ModelMessage>,
    pub prior_tool_observations: Vec<AgentToolObservation>,
    pub tools: Vec<AgentTool>,
    pub max_iterations: usize,
    /// First ply of this segment; resumed tasks continue the iteration count
    /// instead of restarting it.
    pub start_ply: usize,
    /// When set, older observations are folded into an LLM summary once the
    /// list outgrows the budget.
    pub observation_budget: Option<ObservationBudget>,
    pub max_output_tokens: u32,
    pub temperature: f32,
    pub inline_images: Vec<ModelInlineImage>,
    pub stream: bool,
    pub require_tool_before_final: bool,
    pub continue_on_tool_error: bool,
    /// Agents the model can call as tools, and whether it may create more.
    pub sub_agents: SubAgentConfig,
    pub allow_handoff: bool,
    /// Set when the run paused inside a sub-agent: that sub-agent continues first.
    pub resume_sub_agent: Option<SubAgentFrame>,
}

impl AgentTask {
    /// A task with the defaults most runs want: streaming on, 24 steps, tool errors fed back
    /// to the model. Set the public fields to change them.
    pub fn new(
        name: impl Into<String>,
        model: Arc<dyn Model>,
        user_prompt: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            execution: RunContext::new(name.clone(), RunLimits::default()),
            bindings: Bindings::default(),
            agent_definitions: Default::default(),
            pending_round: None,
            name,
            max_output_tokens: 32_000,
            temperature: 0.2,
            model,
            system_prompt: String::new(),
            user_prompt: user_prompt.into(),
            final_instructions: "Answer in plain text.".into(),
            context: Vec::new(),
            conversation: Vec::new(),
            prior_tool_observations: Vec::new(),
            tools: Vec::new(),
            max_iterations: 24,
            start_ply: 1,
            observation_budget: Some(ObservationBudget {
                max_tokens: 48_000,
                keep_recent: 6,
            }),
            inline_images: Vec::new(),
            stream: true,
            require_tool_before_final: false,
            continue_on_tool_error: true,
            sub_agents: SubAgentConfig::default(),
            allow_handoff: false,
            resume_sub_agent: None,
        }
    }
}

/// When the tool results of a run are compacted: once they are estimated to exceed
/// `max_tokens`, all but the `keep_recent` newest are folded into a summary.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct ObservationBudget {
    pub max_tokens: usize,
    pub keep_recent: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AgentContextBlock {
    pub title: String,
    pub content: String,
}

impl AgentContextBlock {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            content: content.into(),
        }
    }
}

/// Why a run failed.
#[derive(Clone, Debug)]
pub enum AgentError {
    /// The model request failed; the kind says whether retrying or shrinking the context helps.
    Model(ModelError),
    /// A tool failed and the task does not feed tool errors back to the model.
    Tool { tool_id: String, message: String },
    /// The step budget ran out without a final answer.
    Budget(String),
    /// The model's output could not be used as an answer.
    Output(String),
    /// The caller's event callback asked to stop.
    Stopped(String),
}

impl AgentError {
    /// Stable machine-readable name of the failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Model(error) => error.code(),
            Self::Tool { .. } => "tool_failed",
            Self::Budget(_) => "step_budget_exhausted",
            Self::Output(_) => "invalid_model_output",
            Self::Stopped(_) => "stopped",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Model(error) => &error.message,
            Self::Tool { message, .. }
            | Self::Budget(message)
            | Self::Output(message)
            | Self::Stopped(message) => message,
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for AgentError {}

impl From<AgentError> for String {
    fn from(error: AgentError) -> Self {
        error.message().to_string()
    }
}

impl From<ModelError> for AgentError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AgentTaskOutcome {
    pub final_text: String,
    /// `final_text` parsed as JSON when it is JSON, otherwise a JSON string.
    pub final_value: Value,
    /// `MaxTokens` when the answer was cut off at the output limit.
    pub stop_reason: ModelStopReason,
    /// Tokens used by this segment of the run, sub-agents included.
    pub usage: ModelUsage,
    pub trace: AgentTaskTrace,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AgentTaskTrace {
    pub steps: Vec<AgentTaskTraceStep>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum AgentTaskTraceStep {
    Model {
        ply: usize,
        raw_output: String,
    },
    Tool {
        ply: usize,
        tool_id: String,
        tool_name: String,
        input: Value,
        observation: String,
        metadata: Value,
    },
    AsyncToolPending {
        ply: usize,
        tool_id: String,
        tool_name: String,
        input: Value,
        pending_message: String,
        metadata: Value,
    },
    Final {
        ply: usize,
        rationale: String,
        output: Value,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub enum AgentTaskEvent {
    ModelRequestStarted {
        ply: usize,
        provider: String,
        model: String,
    },
    ModelStreamChunk {
        ply: usize,
        text: String,
    },
    ModelResponseReceived {
        ply: usize,
        raw_output: String,
        stop_reason: ModelStopReason,
        /// Tokens of this request, when the provider reports them.
        usage: Option<ModelUsage>,
    },
    ToolSelected {
        ply: usize,
        rationale: String,
        raw_output: String,
        tool_id: String,
        tool_name: String,
        input: Value,
    },
    ToolResultProduced {
        ply: usize,
        tool_id: String,
        tool_name: String,
        observation: String,
        metadata: Value,
        /// The model's call this result answers.
        call: Option<AgentToolCallRecord>,
    },
    AsyncToolPending {
        ply: usize,
        tool_id: String,
        tool_name: String,
        rationale: String,
        input: Value,
        pending_message: String,
        metadata: Value,
        observations: Vec<AgentToolObservation>,
        call: Option<AgentToolCallRecord>,
    },
    FinalSelected {
        ply: usize,
        rationale: String,
        raw_output: String,
        output_text: String,
        output_value: Value,
        stop_reason: ModelStopReason,
    },
    ContextCompacted {
        ply: usize,
        folded: usize,
        summary_chars: usize,
        /// Set when the summarizer failed and results were truncated instead.
        error: Option<String>,
    },
    /// The model created (or replaced) an agent-as-tool.
    AgentDefined {
        ply: usize,
        spec: AgentToolSpec,
    },
    /// An event of a sub-agent the parent called at `parent_ply`; its plies count on their own.
    SubAgent {
        agent_id: String,
        agent_name: String,
        parent_ply: usize,
        event: Box<AgentTaskEvent>,
    },
}

#[allow(clippy::large_enum_variant)]
pub enum AgentTaskRunResult {
    Completed(AgentTaskOutcome),
    Paused(AgentTaskPause),
}

/// A run waiting for the result of a tool that is executed outside the loop. Continue it with
/// a new task whose `prior_tool_observations` are `observations` plus the tool's result
/// (`AgentToolObservation::resumed(..).with_call(call)`) and whose `start_ply` is `ply + 1`.
#[derive(Clone, Serialize, Deserialize)]
pub struct AgentTaskPause {
    pub ply: usize,
    pub tool_id: String,
    pub tool_name: String,
    pub rationale: String,
    pub input: Value,
    pub pending_message: String,
    pub metadata: Value,
    pub observations: Vec<AgentToolObservation>,
    /// The model's call the pending tool answers; attach it to the resumed observation.
    pub call: Option<AgentToolCallRecord>,
    /// Tokens used by this segment before it paused.
    pub usage: ModelUsage,
    pub trace: AgentTaskTrace,
    /// Set when the pending tool belongs to a sub-agent: `ply`, `call` and the tool fields are
    /// the sub-agent's, `observations` the parent's, and the frame holds the sub-agent's progress.
    pub sub_agent: Option<SubAgentFrame>,
}

impl AgentTask {
    pub fn register_agent(&mut self, definition: AgentDefinition) {
        upsert_agent_tool_spec(&mut self.sub_agents.agents, definition.spec.clone());
        self.agent_definitions
            .insert(definition.spec.id.clone(), definition);
    }
    pub fn bindings(&self) -> Bindings {
        let mut bindings = self.bindings.clone();
        if !bindings
            .models
            .values()
            .any(|model| Arc::ptr_eq(model, &self.model))
            && !bindings.models.contains_key("default")
        {
            bindings.models.insert("default".into(), self.model.clone());
        }
        bindings.tools.extend(
            self.tools
                .iter()
                .map(|tool| (tool.id.clone(), tool.clone())),
        );
        bindings.agents.extend(self.agent_definitions.clone());
        if !bindings
            .models
            .values()
            .any(|model| Arc::ptr_eq(model, &self.model))
        {
            bindings.models.insert(
                format!("run:{}", self.execution.identity.run_id),
                self.model.clone(),
            );
        }
        bindings
    }
}
