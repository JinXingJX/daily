use crate::agent::compaction;
use crate::model::{
    adapter_for, ModelCompletionRequest, ModelConfig, ModelInlineImage, ModelMessage,
    ModelMessageRole, ModelProviderKind, ModelStreamChunk, ModelToolDefinition,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod run_loop;
mod sub_agents;
#[cfg(test)]
mod tests;
mod tools;

#[allow(unused_imports)]
pub use run_loop::*;
#[allow(unused_imports)]
pub use sub_agents::*;
#[allow(unused_imports)]
pub use tools::*;

pub(super) const SDK_LOOP_PROMPT: &str = r#"You are running inside the Daily backend agent runtime.

Rules:
- Use model API tool calls when you need a tool.
- Treat tool observations as authoritative runtime state.
- Do not claim a tool changed state unless its observation says it did.
- When done, answer normally and match the task-specific final instructions."#;

pub struct AgentTask {
    pub name: String,
    pub session_id: Option<u64>,
    pub turn_id: Option<u64>,
    pub model: ModelConfig,
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
    /// Set when the run paused inside a sub-agent: that sub-agent continues first.
    pub resume_sub_agent: Option<SubAgentFrame>,
}

#[derive(Clone, Copy)]
pub struct ObservationBudget {
    pub max_chars: usize,
    pub keep_recent: usize,
}

#[derive(Clone)]
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

#[allow(dead_code)]
pub struct AgentTaskOutcome {
    pub final_text: String,
    pub final_value: Value,
    pub trace: AgentTaskTrace,
}

#[derive(Default)]
pub struct AgentTaskTrace {
    pub steps: Vec<AgentTaskTraceStep>,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
    },
    FinalSelected {
        ply: usize,
        rationale: String,
        raw_output: String,
        output_text: String,
        output_value: Value,
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

#[allow(dead_code)]
pub struct AgentTaskPause {
    pub ply: usize,
    pub tool_id: String,
    pub tool_name: String,
    pub rationale: String,
    pub input: Value,
    pub pending_message: String,
    pub metadata: Value,
    pub observations: Vec<AgentToolObservation>,
    pub trace: AgentTaskTrace,
    /// Set when the pending tool belongs to a sub-agent: `ply` and the tool fields are the
    /// sub-agent's, `observations` the parent's, and the frame holds the sub-agent's progress.
    pub sub_agent: Option<SubAgentFrame>,
}

pub(super) enum AgentDecision {
    Tools(Vec<AgentToolRequest>),
    Final {
        rationale: String,
        output: Value,
        output_text: String,
    },
}

pub(super) struct AgentToolRequest {
    rationale: String,
    tool_id: String,
    input: Value,
}
