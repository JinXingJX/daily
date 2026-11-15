use crate::agent::{AgentStore, AppSnapshot};
use crate::error::{CoreError, CoreResult};
use crate::fs_utils::sandbox::resolve_sandboxed_path;
use crate::plugins::rpc::{dispatch_plugin_host_rpc, plugin_rpc_payload_limit};
use crate::plugins::{
    InstallLocalDevPluginInput, InstallPluginInput, PluginIdInput, PluginSnapshot,
    SavePluginSettingsInput,
};
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::mutate_state;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a plugin's built module lives on disk; Electron main serves `root` to the renderer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginModule {
    /// Absolute plugin folder (the one holding `plugin.json`).
    pub(crate) root: String,
    /// Module path relative to `root`.
    pub(crate) entry: String,
    /// Changes when the entry file is rebuilt, so a reload imports the new build.
    pub(crate) revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginHostRpcInput {
    pub(crate) plugin_id: String,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) payload: Value,
}

pub(crate) fn list_plugins(state: &AgentStore) -> Result<PluginSnapshot, String> {
    state
        .state
        .lock()
        .map(|state| state.plugins.snapshot())
        .map_err(|_| "Application state is unavailable.".to_string())
}

pub(crate) fn install_plugin(
    app: RuntimeContext,
    input: InstallPluginInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.install_plugin(input))
}

pub(crate) fn install_local_dev_plugin(
    app: RuntimeContext,
    input: InstallLocalDevPluginInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.install_local_dev_plugin(input))
}

pub(crate) fn uninstall_plugin(
    app: RuntimeContext,
    input: PluginIdInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.uninstall_plugin(input))
}

pub(crate) fn enable_plugin(
    app: RuntimeContext,
    input: PluginIdInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.enable_plugin(input))
}

pub(crate) fn disable_plugin(
    app: RuntimeContext,
    input: PluginIdInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.disable_plugin(input))
}

pub(crate) fn save_plugin_settings(
    app: RuntimeContext,
    input: SavePluginSettingsInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| store.save_plugin_settings(input))
}

pub(crate) fn reload_plugins(
    app: RuntimeContext,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let project_root = state.project_root.clone();
    mutate_state(&app, state, |store| store.reload_plugins(&project_root))
}

pub(crate) fn plugin_module(
    input: PluginIdInput,
    state: &AgentStore,
) -> Result<PluginModule, String> {
    let (manifest, manifest_path) = {
        let app_state = state
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let manifest = app_state.plugins.manifest_for(&input.plugin_id)?;
        if !manifest.enabled {
            return Err(format!("Plugin {} is disabled.", manifest.identifier));
        }
        let manifest_path = app_state
            .plugins
            .local_dev_manifest_path(&manifest.identifier);
        (manifest, manifest_path)
    };

    let entry = manifest
        .entry
        .ok_or_else(|| format!("Plugin {} does not declare an entry.", manifest.identifier))?;
    let manifest_path = manifest_path.ok_or_else(|| {
        format!(
            "Plugin {} does not have a local manifest path.",
            manifest.identifier
        )
    })?;
    let root = manifest_path
        .parent()
        .ok_or_else(|| "Local plugin manifest path is missing a parent directory.".to_string())?;
    let root = root.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve local plugin directory {}: {error}",
            root.display()
        )
    })?;
    let entry_path = resolve_sandboxed_path(&root, &entry)?;
    let metadata = std::fs::metadata(&entry_path).map_err(|error| {
        format!(
            "Plugin entry {} could not be read: {error}",
            entry_path.display()
        )
    })?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_millis());
    Ok(PluginModule {
        root: root.display().to_string(),
        entry,
        revision: format!("{modified}-{}", metadata.len()),
    })
}

pub(crate) async fn plugin_host_rpc(
    app: RuntimeContext,
    input: PluginHostRpcInput,
    _state: &AgentStore,
) -> CoreResult<Value> {
    if serde_json::to_string(&input.payload)
        .map(|payload| payload.len() > plugin_rpc_payload_limit(input.method.as_str()))
        .unwrap_or(true)
    {
        return Err(CoreError::PayloadTooLarge(format!(
            "Plugin RPC payload is too large for {}.",
            input.method
        )));
    }
    dispatch_plugin_host_rpc(app.store.clone(), input, Some(app.emit_event.clone())).await
}
