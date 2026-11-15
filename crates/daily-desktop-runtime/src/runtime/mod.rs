use crate::agent::AgentStore;
use crate::error::{CoreError, CoreResult};
use crate::net::http::{build_http_client, build_plugin_http_client};
use crate::runtime::commands::plugins::{
    disable_plugin, enable_plugin, install_local_dev_plugin, install_plugin, list_plugins,
    plugin_host_rpc, plugin_module, reload_plugins, save_plugin_settings, uninstall_plugin,
};
use crate::runtime::commands::session::{
    continue_session_turn, plugin_rpc_result, resume_run, retry_session_turn, run_session_turn,
    stop_session_turn,
};
use crate::runtime::commands::settings::{
    delete_auth_settings_content, get_antigravity_auth_status, get_antigravity_model_catalog,
    get_app_snapshot, get_codex_auth_status, refresh_antigravity_models, reload_settings,
    save_auth_settings_content, save_codex_auth_settings_content, save_settings_file_content,
    start_antigravity_login, start_codex_login,
};
use crate::runtime::commands::workspace::{
    bind_session_agent, create_agent_app, create_workspace_node, delete_agent_app,
    delete_workspace_node, list_context_roots, move_workspace_node, open_session,
    rename_workspace_node, update_agent_app,
};
use crate::runtime::context::RuntimeContext;
use crate::settings::{default_settings, load_settings, LoadedSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(crate) mod commands;
pub(crate) mod context;
pub(crate) mod events;

#[derive(Clone)]
pub struct DailyRuntime {
    pub(crate) handle: RuntimeContext,
}

impl DailyRuntime {
    pub fn new<F>(project_root: PathBuf, emit_event: F) -> Result<Self, String>
    where
        F: Fn(&str, Value) -> Result<(), String> + Send + Sync + 'static,
    {
        let project_root = project_root.canonicalize().map_err(|error| {
            format!(
                "Failed to resolve workspace directory {}: {error}",
                project_root.display()
            )
        })?;
        if !project_root.is_dir() {
            return Err(format!(
                "Workspace path is not a directory: {}",
                project_root.display()
            ));
        }

        let initial_settings = load_settings(&project_root)
            .or_else(|error| {
                eprintln!("Failed to load settings from disk: {error}");
                default_settings(&project_root)
            })
            .map_err(|error| format!("Failed to initialize settings state: {error}"))?;
        Self::with_settings(project_root, initial_settings, emit_event)
    }

    fn with_settings<F>(
        project_root: PathBuf,
        initial_settings: LoadedSettings,
        emit_event: F,
    ) -> Result<Self, String>
    where
        F: Fn(&str, Value) -> Result<(), String> + Send + Sync + 'static,
    {
        let http = build_http_client()?;
        let plugin_http = build_plugin_http_client()?;
        let app_state = crate::state::AppState::seeded(
            &project_root,
            initial_settings.agent,
            initial_settings.model,
            initial_settings.view,
        )
        .map_err(|error| format!("Failed to initialize workspace state: {error}"))?;
        let store = Arc::new(AgentStore {
            agent_checkpoints: Arc::new(crate::agent_checkpoints::SqliteCheckpointStore::open(
                &project_root,
            )?),
            tasks: crate::tasks::TaskStore::open(&project_root)?,
            state: Mutex::new(app_state),
            plugin_storage_lock: Mutex::new(()),
            plugin_agent_sessions: Mutex::new(Default::default()),
            turn_tasks: Mutex::new(Default::default()),
            http,
            plugin_http,
            project_root,
        });
        Ok(Self {
            handle: RuntimeContext::new(store, Arc::new(emit_event)),
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.handle.store.project_root
    }

    #[cfg(test)]
    pub(crate) fn test_runtime() -> Self {
        // Parallel tests can read the same clock value; the counter keeps roots apart.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "daily-runtime-test-{}-{}",
            crate::util::unique_stamp(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let settings = crate::settings::default_settings(&root).unwrap();
        Self::with_settings(root, settings, |_, _| Ok(())).unwrap()
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> &AgentStore {
        &self.handle.store
    }

    pub async fn invoke(&self, method: &str, params: Value) -> CoreResult<Value> {
        dispatch(self.handle.clone(), method, params).await
    }
}

/// Declares every desktop command once. The macro produces both the dispatcher
/// and [`DESKTOP_COMMANDS`], which is exported to the Electron IPC allowlist so
/// the two can never drift apart.
macro_rules! desktop_commands {
    ($app:ident, $state:ident, $params:ident; $($name:literal => $call:expr,)+) => {
        /// Names of all commands accepted by [`DailyRuntime::invoke`].
        pub const DESKTOP_COMMANDS: &[&str] = &[$($name),+];

        async fn dispatch($app: RuntimeContext, method: &str, $params: Value) -> CoreResult<Value> {
            let $state = $app.store.clone();
            let $state = &*$state;
            match method {
                $($name => Ok(to_value($call)?),)+
                _ => Err(CoreError::UnknownCommand(format!("Unknown desktop command: {method}"))),
            }
        }
    };
}

desktop_commands! { app, state, params;
    "get_app_snapshot" => get_app_snapshot(state)?,
    "get_task_runtime_status" => serde_json::json!({"isRunning": state.tasks.has_active()?}),
    "list_workspace_tasks" => state.tasks.workspace_tasks()?,
    "workspace_task" => state.tasks.workspace_task(parse_input(&params)?)?,
    "reload_settings" => reload_settings(app, state)?,
    "get_codex_auth_status" => get_codex_auth_status(parse_input(&params)?, state)?,
    "save_codex_auth_settings_content" => save_codex_auth_settings_content(app, parse_input(&params)?, state)?,
    "start_codex_login" => start_codex_login(parse_input(&params)?, state)?,
    "get_antigravity_auth_status" => get_antigravity_auth_status().await,
    "start_antigravity_login" => start_antigravity_login().await?,
    "get_antigravity_model_catalog" => get_antigravity_model_catalog(state).await?,
    "refresh_antigravity_models" => refresh_antigravity_models(app, parse_input(&params)?, state).await?,
    "save_auth_settings_content" => save_auth_settings_content(app, parse_input(&params)?, state)?,
    "delete_auth_settings_content" => delete_auth_settings_content(app, parse_input(&params)?, state)?,
    "save_settings_file_content" => save_settings_file_content(app, parse_input(&params)?, state)?,
    "create_workspace_node" => create_workspace_node(app, parse_input(&params)?, state)?,
    "rename_workspace_node" => rename_workspace_node(app, parse_input(&params)?, state)?,
    "move_workspace_node" => move_workspace_node(app, parse_input(&params)?, state)?,
    "delete_workspace_node" => delete_workspace_node(app, parse_input(&params)?, state)?,
    "open_session" => open_session(app, parse_input(&params)?, state)?,
    "create_agent_app" => create_agent_app(app, parse_input(&params)?, state)?,
    "update_agent_app" => update_agent_app(app, parse_input(&params)?, state)?,
    "delete_agent_app" => delete_agent_app(app, parse_input(&params)?, state)?,
    "bind_session_agent" => bind_session_agent(app, parse_input(&params)?, state)?,
    "list_context_roots" => list_context_roots(state)?,
    "list_plugins" => list_plugins(state)?,
    "install_plugin" => install_plugin(app, parse_input(&params)?, state)?,
    "install_local_dev_plugin" => install_local_dev_plugin(app, parse_input(&params)?, state)?,
    "uninstall_plugin" => uninstall_plugin(app, parse_input(&params)?, state)?,
    "enable_plugin" => enable_plugin(app, parse_input(&params)?, state)?,
    "disable_plugin" => disable_plugin(app, parse_input(&params)?, state)?,
    "save_plugin_settings" => save_plugin_settings(app, parse_input(&params)?, state)?,
    "reload_plugins" => reload_plugins(app, state)?,
    "plugin_module" => plugin_module(parse_input(&params)?, state)?,
    "plugin_host_rpc" => plugin_host_rpc(app, parse_input(&params)?, state).await?,
    "plugin_rpc_result" => plugin_rpc_result(app, parse_input(&params)?, state)?,
    "run_session_turn" => run_session_turn(app, parse_input(&params)?, state)?,
    "resume_run" => resume_run(app, parse_input(&params)?, state)?,
    "retry_session_turn" => retry_session_turn(app, parse_input(&params)?, state)?,
    "continue_session_turn" => continue_session_turn(app, parse_input(&params)?, state)?,
    "stop_session_turn" => stop_session_turn(app, parse_input(&params)?, state)?,
}

pub(crate) fn parse_input<T: for<'de> Deserialize<'de>>(params: &Value) -> CoreResult<T> {
    let input = params.get("input").unwrap_or(params).clone();
    serde_json::from_value(input)
        .map_err(|error| CoreError::InvalidParams(format!("Invalid command parameters: {error}")))
}

pub(crate) fn to_value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("Failed to serialize command result: {error}"))
}

#[cfg(all(test, feature = "ts"))]
mod ts_export {
    use super::DESKTOP_COMMANDS;
    use std::path::Path;

    /// Writes the Electron IPC allowlist and a barrel for the ts-rs bindings.
    /// Run through `npm run gen:protocol`.
    #[test]
    fn export_bindings_desktop_commands() {
        let app_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let names = DESKTOP_COMMANDS
            .iter()
            .map(|name| format!("  \"{name}\","))
            .collect::<Vec<_>>()
            .join("\n");
        let commands = format!(
            "// Generated from daily-desktop-runtime `desktop_commands!`. Do not edit; run `npm run gen:protocol`.\n\
             export const DESKTOP_COMMANDS = [\n{names}\n] as const;\n"
        );
        std::fs::write(
            app_root.join("electron/shared/commands.generated.ts"),
            commands,
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::DailyRuntime;
    use crate::settings::default_settings;
    use crate::util::unique_stamp;
    use serde_json::json;

    fn test_runtime() -> DailyRuntime {
        let root = std::env::temp_dir().join(format!("daily-runtime-test-{}", unique_stamp()));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let settings = default_settings(&root).unwrap();
        DailyRuntime::with_settings(root, settings, |_, _| Ok(())).unwrap()
    }

    #[tokio::test]
    async fn invoke_reports_stable_error_codes() {
        let runtime = test_runtime();

        let unknown = runtime
            .invoke("missing_command", json!({}))
            .await
            .unwrap_err();
        assert_eq!(unknown.code(), "unknown_command");

        let invalid = runtime
            .invoke(
                "open_session",
                json!({ "input": { "sessionId": "not a number" } }),
            )
            .await
            .unwrap_err();
        assert_eq!(invalid.code(), "invalid_params");

        let oversized = runtime
            .invoke(
                "plugin_host_rpc",
                json!({ "input": {
                    "pluginId": "builtin.chart",
                    "method": "storage.set",
                    "payload": { "value": "x".repeat(600 * 1024) },
                }}),
            )
            .await
            .unwrap_err();
        assert_eq!(oversized.code(), "payload_too_large");

        let _ = std::fs::remove_dir_all(runtime.project_root());
    }
}
