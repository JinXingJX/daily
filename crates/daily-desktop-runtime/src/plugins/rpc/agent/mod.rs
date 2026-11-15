use crate::agent;
use crate::agent::compaction;
use crate::agent::sdk::{
    agent_tool_from_dynamic, upsert_agent_tool_spec, validate_agent_tool_spec, AgentContextBlock,
    AgentTask, AgentTaskEvent, AgentTaskRunResult, AgentToolCallRecord, AgentToolObservation,
    AgentToolSpec, DynamicAgentToolDefinition, ObservationBudget, SubAgentConfig, SubAgentFrame,
};
use crate::agent::turn::prompt::{effective_agent_config, effective_model_config};
use crate::agent::{drive_turn, AgentEvent, AgentStore, DriveTurnResult, TurnRuntimeInput};
use crate::limits::{
    MAX_PLUGIN_CONTEXT_BLOCKS, MAX_PLUGIN_PROMPT_BYTES, MAX_PLUGIN_SYSTEM_PROMPT_BYTES,
    PLUGIN_AGENT_DEFAULT_ITERATIONS, PLUGIN_AGENT_MAX_ITERATIONS,
    PLUGIN_RUN_KEEP_RECENT_OBSERVATIONS, PLUGIN_RUN_OBSERVATION_BUDGET_TOKENS,
    PLUGIN_SESSION_CONTEXT_BUDGET_TOKENS, PLUGIN_SESSION_KEEP_RECENT_ENTRIES,
};
use crate::model::{adapter_for, ModelConfig, ModelMessage};
use crate::plugins::agent_runs::{AGENT_RUNS, AGENT_TASK_RUNS};
use crate::plugins::agent_sessions::{
    plan_session_compaction, render_entry, session_conversation, SessionEntry, SessionRole,
    SessionRunRecord, SessionRunStatus, SessionStore, SessionSummary, SessionToolCall,
};
use crate::plugins::rpc::model::{apply_model_selection, PluginModelSelection};
use crate::runtime::context::EventCallback;
use crate::util::{unique_stamp, unix_timestamp};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

mod event_views;
mod run;
mod session_link;
#[cfg(test)]
mod session_task_tests;
mod task;
mod validate;

#[allow(unused_imports)]
pub(crate) use event_views::*;
#[allow(unused_imports)]
pub(crate) use run::*;
#[allow(unused_imports)]
pub(crate) use session_link::*;
#[allow(unused_imports)]
pub(crate) use task::*;
#[allow(unused_imports)]
pub(crate) use validate::*;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAgentTaskContextInput {
    pub(crate) title: String,
    pub(crate) content: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAgentTaskStartInput {
    #[serde(default)]
    pub(crate) allow_handoff: bool,
    #[serde(default)]
    pub(crate) definitions: Vec<daily_agent_core::AgentDefinition>,
    #[serde(default)]
    pub(crate) model_bindings: std::collections::BTreeMap<String, PluginModelSelection>,
    #[serde(default)]
    pub(crate) run_limits: Option<daily_agent_core::RunLimits>,
    #[serde(default)]
    pub(crate) agent_app_id: Option<u64>,
    /// Provider and model for every segment of the task; overrides the agent app's model.
    #[serde(default)]
    pub(crate) model: Option<PluginModelSelection>,
    pub(crate) system_prompt: String,
    pub(crate) user_prompt: String,
    #[serde(default = "default_plugin_task_final_instructions")]
    pub(crate) final_instructions: String,
    #[serde(default)]
    pub(crate) context: Vec<PluginAgentTaskContextInput>,
    #[serde(default)]
    pub(crate) tools: Vec<DynamicAgentToolDefinition>,
    #[serde(default)]
    pub(crate) max_iterations: Option<usize>,
    #[serde(default)]
    pub(crate) max_output_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) temperature: Option<f32>,
    #[serde(default)]
    pub(crate) require_tool_before_final: bool,
    #[serde(default = "default_continue_on_tool_error")]
    pub(crate) continue_on_tool_error: bool,
    /// Records the task in this agent session and replays the session's history.
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    /// The author's own words, stored in the session instead of `userPrompt`.
    #[serde(default)]
    pub(crate) display_message: Option<String>,
    /// Plugin-owned metadata for the stored author message.
    #[serde(default)]
    pub(crate) message_meta: Value,
    /// Agents the model may call as tools, and whether it may create its own.
    #[serde(default)]
    pub(crate) sub_agents: SubAgentConfig,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAgentTaskResumeInput {
    pub(crate) run_id: String,
    #[serde(default = "default_resume_ok")]
    pub(crate) ok: bool,
    #[serde(default)]
    pub(crate) observation: Option<String>,
    #[serde(default)]
    pub(crate) result: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAgentTaskRetryInput {
    pub(crate) run_id: String,
}

/// Checkpoint of a plugin agent task between segments: written when the task
/// pauses on a plugin tool and when a segment fails, so the task can continue
/// from there instead of starting over.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingPluginAgentTaskRun {
    #[serde(default)]
    pub(crate) runtime_run_id: Option<String>,
    pub(crate) run_id: String,
    pub(crate) plugin_id: String,
    pub(crate) task: PluginAgentTaskStartInput,
    /// Stable across segments so event view ids keep pointing at the same steps.
    #[serde(default)]
    pub(crate) task_id: String,
    /// The plugin tool the task waits on; `None` when the next step is a model request.
    #[serde(default)]
    pub(crate) pending_tool: Option<PendingPluginAgentTool>,
    #[serde(default)]
    pub(crate) observations: Vec<AgentToolObservation>,
    /// The ply to run next when `pending_tool` is `None`.
    #[serde(default)]
    pub(crate) next_ply: usize,
    /// The resume that failed; `agent.task.retry` replays it.
    #[serde(default)]
    pub(crate) failed_resume: Option<PluginAgentTaskResumeInput>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<PluginSessionLink>,
    /// Agents created by the model (in this run or earlier in the session), on top of the plugin's.
    #[serde(default)]
    pub(crate) defined_agents: Vec<AgentToolSpec>,
    /// Set when `pending_tool` was called by a sub-agent: that sub-agent's progress.
    #[serde(default)]
    pub(crate) sub_agent: Option<SubAgentFrame>,
    pub(crate) created_at: u64,
}

impl PendingPluginAgentTaskRun {
    pub(super) fn task_id(&self) -> String {
        if self.task_id.is_empty() {
            format!("plugin-task-{}", self.run_id)
        } else {
            self.task_id.clone()
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum PluginAgentTaskFlowResult {
    Completed {
        text: String,
        value: Value,
    },
    Paused {
        pending_tool: PendingPluginAgentTool,
        sub_agent: Option<SubAgentFrame>,
    },
}

pub(crate) fn default_plugin_task_final_instructions() -> String {
    "Final output must be a plain string unless the plugin asks for structured JSON.".into()
}

pub(crate) fn default_continue_on_tool_error() -> bool {
    true
}

pub(crate) fn default_resume_ok() -> bool {
    true
}
