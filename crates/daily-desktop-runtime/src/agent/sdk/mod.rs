//! Desktop compatibility facade over `daily-agent-core`.
//! New embedders should depend on `daily-agent-core` and `daily-agent-providers` directly.
//!
//! A caller describes a run with an [`AgentTask`] and drives it with
//! [`run_agent_task_until_pause`]. The loop sends the conversation to the model, executes the
//! tools it calls, and sends each round back in the provider's native tool-call format until
//! the model answers, the step budget runs out, or a tool needs outside work and the run
//! pauses. It knows nothing about chat sessions, plugins or UI.
use crate::model::{
    ModelAdapter, ModelConfig, ModelInlineImage, ModelMessage, ModelProviderKind, ModelStopReason,
    ModelUsage,
};
use reqwest::Client;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

/// Model types and providers the loop runs on. Implement [`model::ModelAdapter`] to add a
/// provider and pass it as [`AgentTask::model_adapter`].
pub mod model {
    pub use crate::model::{
        adapter_for, complete_model, complete_with_retry, ModelAdapter, ModelCompletionRequest,
        ModelCompletionResponse, ModelConfig, ModelError, ModelErrorKind, ModelInlineImage,
        ModelMessage, ModelMessageRole, ModelProviderKind, ModelStopReason, ModelStreamChunk,
        ModelToolCall, ModelToolChoice, ModelToolDefinition, ModelToolResultRef, ModelUsage,
        StreamCallback,
    };
}

pub(crate) mod run_loop;
#[cfg(test)]
mod tests;

pub use daily_agent_core::{
    agent_tool_from_dynamic, AgentAsyncToolPending, AgentTool, AgentToolCall, AgentToolCallRecord,
    AgentToolExecution, AgentToolObservation, AgentToolResult, DynamicAgentToolDefinition,
    DynamicAgentToolExecution, ToolCancel, ToolHandler, RUNTIME_TOOL_ID,
};
pub use daily_agent_core::{
    upsert_agent_tool_spec, validate_agent_tool_spec, AgentToolSpec, SubAgentConfig, SubAgentFrame,
    AGENT_CREATE_TOOL_ID, MAX_SUB_AGENTS, MAX_SUB_AGENT_INSTRUCTIONS_BYTES, SUB_AGENT_TOOL_PREFIX,
};
pub use run_loop::{run_agent_task, run_agent_task_until_pause, PreparedTask};

#[cfg(test)]
pub(super) const FORCE_FINAL_NOTE: &str =
    "The step budget for this request is used up. Give the final answer now, without calling tools.";

pub struct AgentTask {
    pub checkpoint_ref: Option<String>,
    pub execution: Option<daily_agent_core::RunContext>,
    pub checkpoint: Option<daily_agent_core::checkpoint::RunCheckpoint>,
    pub resume_action: daily_agent_core::checkpoint::ResumeAction,
    pub agent_definitions: std::collections::BTreeMap<String, daily_agent_core::AgentDefinition>,
    pub model_bindings: std::collections::BTreeMap<String, Arc<dyn daily_agent_core::model::Model>>,
    pub name: String,
    pub session_id: Option<u64>,
    pub turn_id: Option<u64>,
    pub model: ModelConfig,
    /// Runs the task on this adapter instead of the built-in one for `model.provider_kind`.
    pub model_adapter: Option<Arc<dyn ModelAdapter>>,
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
        model: ModelConfig,
        user_prompt: impl Into<String>,
    ) -> Self {
        Self {
            checkpoint_ref: None,
            execution: None,
            checkpoint: None,
            resume_action: daily_agent_core::checkpoint::ResumeAction::Continue,
            agent_definitions: Default::default(),
            model_bindings: Default::default(),
            name: name.into(),
            session_id: None,
            turn_id: None,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
            model,
            model_adapter: None,
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

pub use daily_agent_core::{AgentContextBlock, ObservationBudget};

pub use daily_agent_core::{
    AgentError, AgentTaskOutcome, AgentTaskPause, AgentTaskRunResult, AgentTaskTrace,
    AgentTaskTraceStep,
};

pub enum AgentTaskEvent {
    ModelRequestStarted {
        ply: usize,
        provider_kind: ModelProviderKind,
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

impl AgentTaskEvent {
    fn from_core(
        event: daily_agent_core::AgentTaskEvent,
        provider_kind: ModelProviderKind,
    ) -> Self {
        match event {
            daily_agent_core::AgentTaskEvent::ModelRequestStarted {
                ply,
                model,
                provider,
            } => Self::ModelRequestStarted {
                ply,
                provider_kind: provider_kind_from_identity(&provider, provider_kind),
                model,
            },
            daily_agent_core::AgentTaskEvent::ModelStreamChunk { ply, text } => {
                Self::ModelStreamChunk { ply, text }
            }
            daily_agent_core::AgentTaskEvent::ModelResponseReceived {
                ply,
                raw_output,
                stop_reason,
                usage,
            } => Self::ModelResponseReceived {
                ply,
                raw_output,
                stop_reason,
                usage,
            },
            daily_agent_core::AgentTaskEvent::ToolSelected {
                ply,
                rationale,
                raw_output,
                tool_id,
                tool_name,
                input,
            } => Self::ToolSelected {
                ply,
                rationale,
                raw_output,
                tool_id,
                tool_name,
                input,
            },
            daily_agent_core::AgentTaskEvent::ToolResultProduced {
                ply,
                tool_id,
                tool_name,
                observation,
                metadata,
                call,
            } => Self::ToolResultProduced {
                ply,
                tool_id,
                tool_name,
                observation,
                metadata,
                call,
            },
            daily_agent_core::AgentTaskEvent::AsyncToolPending {
                ply,
                tool_id,
                tool_name,
                rationale,
                input,
                pending_message,
                metadata,
                observations,
                call,
            } => Self::AsyncToolPending {
                ply,
                tool_id,
                tool_name,
                rationale,
                input,
                pending_message,
                metadata,
                observations,
                call,
            },
            daily_agent_core::AgentTaskEvent::FinalSelected {
                ply,
                rationale,
                raw_output,
                output_text,
                output_value,
                stop_reason,
            } => Self::FinalSelected {
                ply,
                rationale,
                raw_output,
                output_text,
                output_value,
                stop_reason,
            },
            daily_agent_core::AgentTaskEvent::ContextCompacted {
                ply,
                folded,
                summary_chars,
                error,
            } => Self::ContextCompacted {
                ply,
                folded,
                summary_chars,
                error,
            },
            daily_agent_core::AgentTaskEvent::AgentDefined { ply, spec } => {
                Self::AgentDefined { ply, spec }
            }
            daily_agent_core::AgentTaskEvent::SubAgent {
                agent_id,
                agent_name,
                parent_ply,
                event,
            } => Self::SubAgent {
                agent_id,
                agent_name,
                parent_ply,
                event: Box::new(Self::from_core(*event, provider_kind)),
            },
        }
    }
}

fn provider_kind_from_identity(identity: &str, fallback: ModelProviderKind) -> ModelProviderKind {
    [
        ModelProviderKind::OpenAiCompatible,
        ModelProviderKind::AnthropicMessages,
        ModelProviderKind::GeminiApi,
        ModelProviderKind::CodexApiKey,
        ModelProviderKind::CodexLogin,
        ModelProviderKind::AntigravityLogin,
    ]
    .into_iter()
    .find(|kind| kind.label() == identity.split('@').next().unwrap_or(identity))
    .unwrap_or(fallback)
}
