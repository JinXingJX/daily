use crate::agent::{AgentStore, AppSnapshot};
use crate::error::{CoreError, CoreResult};
use crate::fs_utils::sandbox::{read_limited_text, resolve_sandboxed_path};
use crate::limits::PLUGIN_VIEW_ASSET_LIMIT_BYTES;
use crate::plugins::rpc::{dispatch_plugin_host_rpc, plugin_rpc_payload_limit};
use crate::plugins::{
    InstallLocalDevPluginInput, InstallPluginInput, PluginIdInput, PluginSnapshot,
    SavePluginSettingsInput,
};
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::mutate_state;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginViewAssetInput {
    pub(crate) plugin_id: String,
    pub(crate) view_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginViewAsset {
    pub(crate) source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginHostRpcInput {
    pub(crate) plugin_id: String,
    pub(crate) view_id: String,
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

pub(crate) fn plugin_view_asset(
    input: PluginViewAssetInput,
    state: &AgentStore,
) -> Result<PluginViewAsset, String> {
    let (manifest, view, manifest_path) = {
        let app_state = state
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let (manifest, view) = app_state
            .plugins
            .view_for(&input.plugin_id, &input.view_id)?;
        if !manifest.enabled {
            return Err(format!("Plugin {} is disabled.", manifest.identifier));
        }
        let manifest_path = app_state
            .plugins
            .local_dev_manifest_path(&manifest.identifier);
        (manifest, view, manifest_path)
    };

    if view.runtime != "iframe" {
        return Err(format!(
            "Plugin view {} is not an iframe view.",
            input.view_id
        ));
    }
    let entry = view
        .entry
        .or(manifest.entry)
        .ok_or_else(|| format!("Plugin view {} does not declare an entry.", input.view_id))?;
    let manifest_path = manifest_path.ok_or_else(|| {
        format!(
            "Plugin {} does not have a local development manifest path.",
            manifest.identifier
        )
    })?;
    let base_dir = manifest_path
        .parent()
        .ok_or_else(|| "Local plugin manifest path is missing a parent directory.".to_string())?;
    let base_dir = base_dir.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve local plugin directory {}: {error}",
            base_dir.display()
        )
    })?;
    let asset_path = resolve_sandboxed_path(&base_dir, &entry)?;
    let source = read_limited_text(
        &asset_path,
        PLUGIN_VIEW_ASSET_LIMIT_BYTES,
        "plugin view asset",
    )?;
    Ok(PluginViewAsset { source })
}

pub(crate) async fn plugin_host_rpc(
    app: RuntimeContext,
    input: PluginHostRpcInput,
    state: &AgentStore,
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
    dispatch_plugin_host_rpc(state, input, Some(app.emit_event.clone())).await
}
