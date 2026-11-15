use crate::model::{ModelInlineImage, ModelProviderKind, ModelUsage, DEFAULT_CODEX_AUTH_CONFIG_ID};
use crate::plugins::{PluginManifestView, PluginSnapshot};
use crate::settings::SettingsView;
use crate::util::unix_timestamp;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
pub const APP_EVENT_NAME: &str = "app://state";
const MAX_INLINE_IMAGE_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub workspace: WorkspaceView,
    pub active_session: Option<ChatSessionView>,
    pub agent_apps: Vec<AgentAppConfig>,
    pub plugin_manifests: Vec<PluginManifestView>,
    pub plugins: PluginSnapshot,
    pub settings: SettingsView,
    pub runtime: RuntimeView,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AppEventEnvelope {
    #[serde(flatten)]
    pub payload: AppEventPayload,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppEventPayload {
    Snapshot { snapshot: AppSnapshot },
    TurnDelta { delta: TurnDelta },
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TurnDelta {
    pub session_id: u64,
    pub turn_id: u64,
    pub active_session_id: Option<u64>,
    pub runtime: RuntimeView,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub session_updated_at: Option<u64>,
    pub message_appends: Vec<DialogueMessage>,
    pub turn_patch: TurnPatch,
}

#[derive(Clone, Serialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TurnPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub status: Option<TurnStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub partial_response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub partial_response_append: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_partial_response: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pending_async_tool: Option<PendingAsyncToolState>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clear_pending_async_tool: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub steps: Option<Vec<DecisionStep>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps_append: Vec<DecisionStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_view_patches: Vec<AgentEventViewPatch>,
    /// The turn's token total so far.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub usage: Option<ModelUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error_code: Option<String>,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    pub root_id: u64,
    pub active_session_id: Option<u64>,
    pub nodes: Vec<WorkspaceNode>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceNodeKind {
    Root,
    Folder,
    Session,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNode {
    pub id: u64,
    pub kind: WorkspaceNodeKind,
    pub name: String,
    pub parent_id: Option<u64>,
    pub children: Vec<u64>,
    pub session_id: Option<u64>,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionView {
    pub id: u64,
    pub title: String,
    pub agent_app_id: Option<u64>,
    pub messages: Vec<DialogueMessage>,
    pub turns: Vec<AgentTurnView>,
    pub updated_at: u64,
    pub has_running_turn: bool,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeView {
    /// True while any session has a running turn.
    pub is_running: bool,
    /// Sessions with a queued or running turn, in session-id order.
    pub running_session_ids: Vec<u64>,
    pub status: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AgentHookMode {
    Parallel,
    Serial,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AgentHookPhase {
    OnStart,
    OnFinish,
}

impl AgentHookPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::OnStart => "on_start",
            Self::OnFinish => "on_finish",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentHookConfig {
    pub mode: Option<AgentHookMode>,
    pub append_previous_output: bool,
    pub agent_app_ids: Vec<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentAppConfig {
    pub id: u64,
    pub name: String,
    pub system_prompt: String,
    pub provider_kind: ModelProviderKind,
    #[serde(default = "default_auth_config_id")]
    pub auth_config_id: String,
    pub model: String,
    pub temperature: f32,
    pub mounted_plugin_ids: Vec<String>,
    pub context_roots: Vec<String>,
    #[serde(default)]
    pub on_start_hook: AgentHookConfig,
    #[serde(default)]
    pub on_finish_hook: AgentHookConfig,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DialogueMessage {
    pub id: u64,
    #[serde(default)]
    pub turn_id: Option<u64>,
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentTurn {
    pub id: u64,
    pub prompt: String,
    pub response: String,
    pub strategy: String,
    pub status: TurnStatus,
    pub partial_response: String,
    pub steps: Vec<DecisionStep>,
    #[serde(default)]
    pub event_views: Vec<AgentEventView>,
    #[serde(default)]
    pub images: Vec<TurnImageInput>,
    #[serde(default)]
    pub agent_app_id: Option<u64>,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub pending_async_tool: Option<PendingAsyncToolState>,
    /// Where a failed or stopped turn continues: the tool results so far and the next ply.
    #[serde(default)]
    pub checkpoint: Option<TurnCheckpoint>,
    /// Tokens used by the turn's model requests, when the provider reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelUsage>,
    /// Machine-readable reason of a failed turn (e.g. `model_rate_limited`, `stopped`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentRunRef {
    pub run_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TurnCheckpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub runtime: Option<AgentRunRef>,
    pub observations: Vec<crate::agent::sdk::AgentToolObservation>,
    pub next_ply: usize,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Queued,
    Running,
    WaitingAsyncTool,
    Completed,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PendingAsyncToolState {
    pub ply: usize,
    pub tool_id: String,
    pub tool_name: String,
    pub rationale: String,
    pub input: serde_json::Value,
    pub pending_message: String,
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub observations: Vec<crate::agent::sdk::AgentToolObservation>,
    /// The model's call the tool answers, so its result replays as a native tool result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub call: Option<crate::agent::sdk::AgentToolCallRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DecisionStep {
    pub ply: usize,
    pub selected_action: String,
    pub selected_kind: String,
    pub rationale: String,
    pub model_output: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub observation: Option<String>,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnView {
    pub id: u64,
    pub prompt: String,
    pub response: String,
    pub strategy: String,
    pub status: TurnStatus,
    pub partial_response: String,
    pub steps: Vec<DecisionStep>,
    pub event_views: Vec<AgentEventView>,
    pub agent_app_id: Option<u64>,
    pub agent_name: Option<String>,
    pub pending_async_tool: Option<PendingAsyncToolState>,
    pub usage: Option<ModelUsage>,
    pub error_code: Option<String>,
}

impl From<&AgentTurn> for AgentTurnView {
    fn from(turn: &AgentTurn) -> Self {
        Self {
            id: turn.id,
            prompt: turn.prompt.clone(),
            response: turn.response.clone(),
            strategy: turn.strategy.clone(),
            status: turn.status,
            partial_response: turn.partial_response.clone(),
            steps: turn.steps.clone(),
            event_views: turn.event_views.clone(),
            agent_app_id: turn.agent_app_id,
            agent_name: turn.agent_name.clone(),
            pending_async_tool: turn.pending_async_tool.clone(),
            usage: turn.usage,
            error_code: turn.error_code.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AgentEventViewKind {
    UserInput,
    LlmThought,
    ToolCall,
    ToolResult,
    AssistantText,
    Error,
    Runtime,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AgentEventViewStatus {
    InProgress,
    Waiting,
    Completed,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentEventView {
    pub id: String,
    pub parser_id: String,
    pub kind: AgentEventViewKind,
    pub status: AgentEventViewStatus,
    #[serde(default)]
    pub role: Option<MessageRole>,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub ply: Option<usize>,
    #[serde(default)]
    pub tool_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: u64,
    pub updated_at: u64,
}

impl AgentEventView {
    pub fn new(
        id: impl Into<String>,
        kind: AgentEventViewKind,
        status: AgentEventViewStatus,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let now = unix_timestamp();
        Self {
            id: id.into(),
            parser_id: "builtin.default".into(),
            kind,
            status,
            role: None,
            title: title.into(),
            content: content.into(),
            ply: None,
            tool_id: None,
            tool_name: None,
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn user_input(turn_id: u64, content: impl Into<String>) -> Self {
        Self::new(
            format!("turn-{turn_id}-user-input"),
            AgentEventViewKind::UserInput,
            AgentEventViewStatus::Completed,
            "User input",
            content,
        )
        .with_role(MessageRole::User)
    }

    pub fn with_role(mut self, role: MessageRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn with_ply(mut self, ply: usize) -> Self {
        self.ply = Some(ply);
        self
    }

    pub fn with_tool(mut self, tool_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        self.tool_id = Some(tool_id.into());
        self.tool_name = Some(tool_name.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AgentEventViewMergeStrategy {
    Replace,
    AppendContent,
    Update,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentEventViewPatch {
    pub view: AgentEventView,
    pub merge: AgentEventViewMergeStrategy,
}

impl AgentEventViewPatch {
    pub fn new(view: AgentEventView, merge: AgentEventViewMergeStrategy) -> Self {
        Self { view, merge }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: u64,
    pub turn_id: u64,
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    System,
    UserMessage,
    LlmDecision,
    ToolResult,
    AssistantMessage,
    Error,
}

impl MemoryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::UserMessage => "user_message",
            Self::LlmDecision => "llm_decision",
            Self::ToolResult => "tool_result",
            Self::AssistantMessage => "assistant_message",
            Self::Error => "error",
        }
    }
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceNodeInput {
    pub parent_id: u64,
    pub kind: WorkspaceNodeKind,
    pub name: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RenameWorkspaceNodeInput {
    pub node_id: u64,
    pub name: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MoveWorkspaceNodeInput {
    pub node_id: u64,
    pub parent_id: u64,
    pub index: Option<usize>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkspaceNodeInput {
    pub node_id: u64,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct OpenSessionInput {
    pub session_id: u64,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentAppInput {
    pub name: String,
    pub system_prompt: String,
    #[serde(default)]
    pub auth_config_id: Option<String>,
    pub model: String,
    pub temperature: f32,
    #[serde(default)]
    pub mounted_plugin_ids: Vec<String>,
    #[serde(default)]
    pub context_roots: Vec<String>,
    #[serde(default)]
    pub on_start_hook: AgentHookConfig,
    #[serde(default)]
    pub on_finish_hook: AgentHookConfig,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentAppInput {
    pub id: u64,
    pub name: String,
    pub system_prompt: String,
    #[serde(default)]
    pub auth_config_id: Option<String>,
    pub model: String,
    pub temperature: f32,
    #[serde(default)]
    pub mounted_plugin_ids: Vec<String>,
    #[serde(default)]
    pub context_roots: Vec<String>,
    #[serde(default)]
    pub on_start_hook: AgentHookConfig,
    #[serde(default)]
    pub on_finish_hook: AgentHookConfig,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DeleteAgentAppInput {
    pub id: u64,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct BindSessionAgentInput {
    pub session_id: u64,
    pub agent_app_id: Option<u64>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RunSessionTurnInput {
    pub session_id: u64,
    pub text: String,
    #[serde(default)]
    pub images: Vec<TurnImageInput>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ResumeRunInput {
    pub session_id: u64,
    pub turn_id: u64,
    pub result: String,
    #[serde(default = "default_resume_ok")]
    pub ok: bool,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

fn default_resume_ok() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Deserialize, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TurnImageInput {
    pub name: Option<String>,
    pub mime_type: String,
    pub data_base64: String,
}

impl TurnImageInput {
    pub fn into_model_image(self) -> Result<ModelInlineImage, String> {
        let mime_type = self.mime_type.trim().to_string();
        let data_base64 = self.data_base64.trim().to_string();
        let name = self
            .name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if name
            .as_ref()
            .is_some_and(|value| value.len() > 255 || value.chars().any(char::is_control))
        {
            return Err(
                "Image names must be at most 255 bytes and contain no control characters.".into(),
            );
        }

        if !matches!(
            mime_type.as_str(),
            "image/gif" | "image/jpeg" | "image/png" | "image/webp"
        ) {
            return Err("Only image mime types are supported for multimodal turns.".into());
        }
        if data_base64.is_empty() {
            return Err("Image payload cannot be empty.".into());
        }
        if data_base64.len() > MAX_INLINE_IMAGE_BYTES.saturating_mul(4) / 3 + 4 {
            return Err(format!(
                "Each image must be at most {MAX_INLINE_IMAGE_BYTES} decoded bytes."
            ));
        }
        let decoded = STANDARD
            .decode(&data_base64)
            .map_err(|_| "Image payload is not valid base64.".to_string())?;
        if decoded.len() > MAX_INLINE_IMAGE_BYTES {
            return Err(format!(
                "Each image must be at most {MAX_INLINE_IMAGE_BYTES} decoded bytes."
            ));
        }

        Ok(ModelInlineImage {
            name,
            mime_type,
            data_base64,
        })
    }
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ContextRootOption {
    pub path: String,
    pub label: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionState {
    pub id: u64,
    pub title: String,
    pub agent_app_id: Option<u64>,
    #[serde(default)]
    pub last_agent_app_id: Option<u64>,
    pub messages: Vec<DialogueMessage>,
    /// Older messages folded into a summary; they are no longer replayed to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_summary: Option<ChatHistorySummary>,
    pub turns: Vec<AgentTurn>,
    pub memory: Vec<MemoryEntry>,
    pub updated_at: u64,
    pub running_turn_id: Option<u64>,
    pub next_message_id: u64,
    pub next_memory_id: u64,
    pub next_turn_id: u64,
}

/// An LLM summary of a chat session's messages up to and including `through_message_id`.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatHistorySummary {
    pub content: String,
    pub through_message_id: u64,
}

pub(crate) fn default_auth_config_id() -> String {
    DEFAULT_CODEX_AUTH_CONFIG_ID.to_string()
}
