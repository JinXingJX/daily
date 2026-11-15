use crate::agent::types::{
    AgentAppConfig, AgentTurnView, AppSnapshot, ChatSessionState, ChatSessionView, RuntimeView,
    WorkspaceNode, WorkspaceView,
};
use crate::agent::AgentConfig;
use crate::model::ModelConfig;
use crate::plugins::{
    InstallLocalDevPluginInput, InstallPluginInput, PluginIdInput, PluginState,
    SavePluginSettingsInput,
};
use crate::settings::SettingsView;
use crate::state::persistence::PersistedSessions;
use std::path::Path;
use std::sync::Arc;

pub(crate) mod agent_apps;
pub(crate) mod persistence;
pub(crate) mod sessions;
#[cfg(test)]
mod tests;
pub(crate) mod workspace_tree;

pub(crate) const APP_DIR_NAME: &str = ".daily";
pub(crate) const WORKSPACE_FILE_NAME: &str = "workspace.json";
pub(crate) const SESSIONS_DIR_NAME: &str = "sessions";
pub(crate) const MAX_TURN_IMAGES: usize = 4;
pub(crate) const MAX_STATE_FILE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_WORKSPACE_NODES: usize = 10_000;
pub(crate) const MAX_AGENT_APPS: usize = 100;
pub(crate) const MAX_NAME_CHARACTERS: usize = 128;
pub(crate) const MAX_SYSTEM_PROMPT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_AGENT_REFERENCES: usize = 100;
pub(crate) const MAX_SESSION_MESSAGES: usize = 100_000;
pub(crate) const MAX_SESSION_TURNS: usize = 20_000;
pub(crate) const MAX_SESSION_MEMORY_ENTRIES: usize = 100_000;
pub(crate) const MAX_TURN_STEPS: usize = 2_048;
pub(crate) const MAX_TURN_EVENT_VIEWS: usize = 4_096;
pub(crate) const DEFAULT_SESSION_TITLE: &str = "New task";
pub(crate) const INTRO_MESSAGE: &str =
    "Daily workspace is ready. Pick an agent, attach context, and start the conversation.";
pub(crate) const INTRO_MEMORY: &str =
    "The workspace stores session dialogue, turns, and tool memory in local files.";

#[derive(Clone)]
pub struct AppState {
    pub root_id: u64,
    pub active_session_id: Option<u64>,
    pub nodes: Vec<WorkspaceNode>,
    /// Copy-on-write so cloning the state for rollback is cheap and unchanged
    /// sessions can be detected by pointer identity when persisting.
    pub sessions: Vec<Arc<ChatSessionState>>,
    pub agent_apps: Vec<AgentAppConfig>,
    pub plugins: PluginState,
    /// Workspace directory the agent's read-only workspace tools are bound to; `None` in
    /// in-memory test states.
    pub workspace_root: Option<std::path::PathBuf>,
    pub settings: SettingsView,
    pub agent_config: AgentConfig,
    pub model_config: ModelConfig,
    /// Latest runtime progress message, across sessions.
    pub runtime_status: String,
    pub(crate) next_node_id: u64,
    pub(crate) next_session_id: u64,
    pub(crate) next_agent_app_id: u64,
    pub(crate) persisted_sessions: PersistedSessions,
}

pub(crate) fn normalize_name(input: &str, fallback: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn validate_name(value: &str, label: &str) -> Result<(), String> {
    if value.chars().count() > MAX_NAME_CHARACTERS || value.chars().any(char::is_control) {
        return Err(format!(
            "{label} must be at most {MAX_NAME_CHARACTERS} characters and contain no control characters."
        ));
    }
    Ok(())
}

impl AppState {
    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            workspace: WorkspaceView {
                root_id: self.root_id,
                active_session_id: self.active_session_id,
                nodes: self.nodes.clone(),
            },
            active_session: self.active_session().map(|session| ChatSessionView {
                id: session.id,
                title: session.title.clone(),
                agent_app_id: session.agent_app_id,
                messages: session.messages.clone(),
                turns: session.turns.iter().map(AgentTurnView::from).collect(),
                updated_at: session.updated_at,
                has_running_turn: session.running_turn_id.is_some(),
            }),
            agent_apps: self.agent_apps.clone(),
            plugin_manifests: self.plugins.installed_manifests(),
            plugins: self.plugins.snapshot(),
            settings: self.settings.clone(),
            runtime: self.runtime_view(),
        }
    }

    pub fn runtime_view(&self) -> RuntimeView {
        let running_session_ids = self.running_session_ids();
        RuntimeView {
            is_running: !running_session_ids.is_empty(),
            running_session_ids,
            status: self.runtime_status.clone(),
        }
    }

    pub fn running_session_ids(&self) -> Vec<u64> {
        let mut ids = self
            .sessions
            .iter()
            .filter(|session| session.running_turn_id.is_some())
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub fn is_session_running(&self, session_id: u64) -> bool {
        self.session(session_id)
            .is_some_and(|session| session.running_turn_id.is_some())
    }

    /// Checks that `session_id` may start a turn: it is idle and the workspace is below
    /// [`MAX_CONCURRENT_TURNS`](crate::limits::MAX_CONCURRENT_TURNS).
    pub fn ensure_can_start_turn(&self, session_id: u64) -> Result<(), String> {
        if self.is_session_running(session_id) {
            return Err("This session is already running a turn. Wait for it or stop it.".into());
        }
        if self.running_session_ids().len() >= crate::limits::MAX_CONCURRENT_TURNS {
            return Err(format!(
                "At most {} sessions can run at once. Wait for one to finish.",
                crate::limits::MAX_CONCURRENT_TURNS
            ));
        }
        Ok(())
    }

    pub fn update_settings(
        &mut self,
        agent_config: AgentConfig,
        model_config: ModelConfig,
        settings: SettingsView,
    ) {
        self.agent_config = agent_config;
        self.model_config = model_config;
        self.settings = settings;
    }

    pub fn install_plugin(&mut self, input: InstallPluginInput) -> Result<(), String> {
        self.plugins.install(&input.plugin_id)
    }

    pub fn install_local_dev_plugin(
        &mut self,
        input: InstallLocalDevPluginInput,
    ) -> Result<(), String> {
        self.plugins.install_local_dev(input)
    }

    pub fn uninstall_plugin(&mut self, input: PluginIdInput) -> Result<(), String> {
        self.plugins.uninstall(&input.plugin_id)?;
        for agent in &mut self.agent_apps {
            self.plugins
                .remove_plugin_from_agents(&input.plugin_id, &mut agent.mounted_plugin_ids);
        }
        Ok(())
    }

    pub fn enable_plugin(&mut self, input: PluginIdInput) -> Result<(), String> {
        self.plugins.enable(&input.plugin_id)
    }

    pub fn disable_plugin(&mut self, input: PluginIdInput) -> Result<(), String> {
        self.plugins.disable(&input.plugin_id)
    }

    pub fn save_plugin_settings(&mut self, input: SavePluginSettingsInput) -> Result<(), String> {
        self.plugins.save_settings(&input.plugin_id, input.settings)
    }

    pub fn reload_plugins(&mut self, project_root: &Path) -> Result<(), String> {
        self.plugins = PluginState::load(project_root)?;
        self.canonicalize_agent_plugin_mounts();
        Ok(())
    }
}
