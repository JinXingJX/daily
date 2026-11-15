pub(crate) mod compaction;
mod config;
mod event_views;
mod events;
mod runtime;
pub mod sdk;
pub(crate) mod tools;
pub(crate) mod turn;
pub(crate) mod types;
pub(crate) mod workspace_tools;

use crate::state::AppState;
use reqwest::Client;
use std::sync::Mutex;

pub use config::{AgentConfig, AgentConfigView};
pub use events::{apply_event, apply_event_delta, AgentEvent};
pub(crate) use runtime::plan_history_fold;
pub use runtime::{drive_turn, DriveTurnResult, TurnRuntimeInput};
pub use types::{
    AgentAppConfig, AgentEventView, AgentEventViewKind, AgentEventViewMergeStrategy,
    AgentEventViewPatch, AgentEventViewStatus, AgentHookConfig, AgentHookMode, AgentHookPhase,
    AppEventEnvelope, AppEventPayload, AppSnapshot, BindSessionAgentInput, ContextRootOption,
    CreateAgentAppInput, CreateWorkspaceNodeInput, DecisionStep, DeleteAgentAppInput,
    DeleteWorkspaceNodeInput, DialogueMessage, MemoryEntry, MemoryKind, MessageRole,
    MoveWorkspaceNodeInput, OpenSessionInput, RenameWorkspaceNodeInput, ResumeRunInput,
    RunSessionTurnInput, TurnCheckpoint, TurnDelta, TurnImageInput, TurnPatch, TurnStatus,
    UpdateAgentAppInput, APP_EVENT_NAME,
};

pub struct AgentStore {
    pub(crate) agent_checkpoints: std::sync::Arc<crate::agent_checkpoints::SqliteCheckpointStore>,
    pub(crate) tasks: crate::tasks::TaskStore,
    pub state: Mutex<AppState>,
    /// Serializes the read-modify-write cycle for the per-plugin storage file.
    /// Requests are handled concurrently by the app server, so atomic file
    /// replacement alone cannot prevent two keys from overwriting each other.
    pub plugin_storage_lock: Mutex<()>,
    /// Serializes plugin agent session writes and tracks sessions with a task
    /// currently executing (keys are `<pluginId>/<sessionId>`), each with the
    /// signal `agent.session.stop` uses to end it.
    pub plugin_agent_sessions:
        Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Notify>>>,
    /// Chat turns running in the background, by session, so they can be stopped.
    pub turn_tasks: Mutex<std::collections::HashMap<u64, turn::start::RunningTurn>>,
    pub http: Client,
    pub plugin_http: Client,
    pub project_root: std::path::PathBuf,
}
