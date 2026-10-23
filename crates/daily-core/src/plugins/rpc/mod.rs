use crate::agent::AgentStore;
use crate::error::{CoreError, CoreResult};
use crate::limits::{
    DEFAULT_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES, FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES,
};
use crate::plugins::canonical_plugin_id;
use crate::plugins::rpc::agent::{
    plugin_agent_list, plugin_agent_run, plugin_agent_task_cancel, plugin_agent_task_resume,
    plugin_agent_task_retry, plugin_agent_task_start, PluginProgressSink,
};
use crate::plugins::rpc::agent_session::{
    plugin_agent_session_append_messages, plugin_agent_session_create, plugin_agent_session_delete,
    plugin_agent_session_get, plugin_agent_session_list, plugin_agent_session_remove_agent_tool,
    plugin_agent_session_stop, plugin_agent_session_update_entry,
};
use crate::plugins::rpc::files::{
    plugin_file_ensure_dir, plugin_file_list, plugin_file_read_text, plugin_file_remove,
    plugin_file_write_text,
};
use crate::plugins::rpc::model::{plugin_model_invoke, plugin_model_list};
use crate::plugins::rpc::network::plugin_network_fetch;
use crate::plugins::rpc::storage::{
    plugin_storage_compare_and_swap, plugin_storage_get, plugin_storage_list,
    plugin_storage_remove, plugin_storage_set,
};
use crate::plugins::rpc::workspace::{
    plugin_workspace_ensure_dir, plugin_workspace_list, plugin_workspace_read_text,
    plugin_workspace_remove_file, plugin_workspace_write_text,
};
use crate::runtime::commands::plugins::PluginHostRpcInput;
use crate::runtime::context::EventCallback;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) mod agent;
pub(crate) mod agent_session;
pub(crate) mod files;
pub(crate) mod model;
pub(crate) mod network;
pub(crate) mod storage;
#[cfg(test)]
mod tests;
pub(crate) mod workspace;

pub(crate) fn plugin_rpc_payload_limit(method: &str) -> usize {
    match method {
        "plugin.files.writeText" | "workspace.writeText" => {
            FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES
        }
        _ => DEFAULT_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES,
    }
}

/// Host-enforced policy for one plugin RPC method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcMethodSpec {
    /// Serializes read-modify-write access to the per-plugin storage file.
    pub(crate) storage_lock: bool,
}

const fn spec(storage_lock: bool) -> RpcMethodSpec {
    RpcMethodSpec { storage_lock }
}

/// Every plugin host RPC method and its policy. Exported to the plugin SDK as
/// `PLUGIN_RPC_METHODS` by `npm run gen:protocol`; `host.setDirty` is handled by the renderer
/// and never reaches the sidecar.
pub(crate) const PLUGIN_RPC_METHODS: &[(&str, RpcMethodSpec)] = &[
    ("host.getContext", spec(false)),
    ("storage.get", spec(true)),
    ("storage.set", spec(true)),
    ("storage.compareAndSwap", spec(true)),
    ("storage.remove", spec(true)),
    ("storage.list", spec(true)),
    ("plugin.files.readText", spec(false)),
    ("plugin.files.writeText", spec(false)),
    ("plugin.files.remove", spec(false)),
    ("plugin.files.list", spec(false)),
    ("plugin.files.ensureDir", spec(false)),
    ("workspace.readText", spec(false)),
    ("workspace.list", spec(false)),
    ("workspace.writeText", spec(false)),
    ("workspace.ensureDir", spec(false)),
    ("workspace.removeFile", spec(false)),
    ("model.invoke", spec(false)),
    ("model.list", spec(false)),
    ("network.fetch", spec(false)),
    ("agent.list", spec(false)),
    ("agent.run", spec(false)),
    ("agent.task.start", spec(false)),
    ("agent.task.resume", spec(false)),
    ("agent.task.retry", spec(false)),
    ("agent.task.cancel", spec(false)),
    ("agent.session.create", spec(false)),
    ("agent.session.list", spec(false)),
    ("agent.session.get", spec(false)),
    ("agent.session.delete", spec(false)),
    ("agent.session.appendMessages", spec(false)),
    ("agent.session.updateEntry", spec(false)),
    ("agent.session.stop", spec(false)),
    ("agent.session.removeAgentTool", spec(false)),
];

pub(crate) fn rpc_method_spec(method: &str) -> Option<RpcMethodSpec> {
    PLUGIN_RPC_METHODS
        .iter()
        .find(|(name, _)| *name == method)
        .map(|(_, spec)| *spec)
}

#[cfg(all(test, feature = "ts"))]
mod ts_export {
    use super::PLUGIN_RPC_METHODS;
    use std::path::Path;

    /// Writes the RPC method list consumed by the plugin SDK. Run through `npm run gen:protocol`.
    #[test]
    fn export_bindings_plugin_rpc_methods() {
        let app_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let names = PLUGIN_RPC_METHODS
            .iter()
            .map(|(name, _)| format!("  \"{name}\","))
            .collect::<Vec<_>>()
            .join("\n");
        let source = format!(
            "// Generated from daily-core `PLUGIN_RPC_METHODS`. Do not edit; run `npm run gen:protocol`.\n\
             export const PLUGIN_RPC_METHODS = [\n{names}\n] as const;\n\n\
             export type PluginRpcMethod = (typeof PLUGIN_RPC_METHODS)[number];\n"
        );
        std::fs::write(
            app_root.join("packages/plugin-sdk/src/methods.generated.ts"),
            source,
        )
        .unwrap();
    }
}

pub(crate) async fn dispatch_plugin_host_rpc(
    state: &AgentStore,
    input: PluginHostRpcInput,
    emit: Option<Arc<EventCallback>>,
) -> CoreResult<Value> {
    let plugin_id = canonical_plugin_id(&input.plugin_id);
    {
        let app_state = state
            .state
            .lock()
            .map_err(|_| CoreError::Unavailable("Application state is unavailable.".into()))?;
        let (manifest, _) = app_state
            .plugins
            .view_for(&plugin_id, &input.view_id)
            .map_err(CoreError::NotFound)?;
        if !manifest.enabled {
            return Err(CoreError::PluginDisabled(format!(
                "Plugin {} is disabled.",
                manifest.identifier
            )));
        }
    }

    let method = input.method.as_str();
    let spec = rpc_method_spec(method)
        .ok_or_else(|| CoreError::NotFound(format!("Unknown plugin host RPC method {method}.")))?;

    let root = state.project_root.as_path();
    let progress = emit.and_then(|emit| {
        let stream_id = input.payload.get("streamId")?.as_str()?;
        (!stream_id.is_empty() && stream_id.len() <= 128)
            .then(|| PluginProgressSink::new(emit, &plugin_id, &input.view_id, stream_id))
    });
    let payload = input.payload;
    if spec.storage_lock {
        let _storage_guard = state
            .plugin_storage_lock
            .lock()
            .map_err(|_| CoreError::Unavailable("Plugin storage is unavailable.".into()))?;
        let result = match method {
            "storage.get" => plugin_storage_get(root, &plugin_id, payload),
            "storage.set" => plugin_storage_set(root, &plugin_id, payload),
            "storage.compareAndSwap" => plugin_storage_compare_and_swap(root, &plugin_id, payload),
            "storage.remove" => plugin_storage_remove(root, &plugin_id, payload),
            "storage.list" => plugin_storage_list(root, &plugin_id),
            _ => unreachable!("storage lock declared for non-storage method {method}"),
        };
        return result.map_err(CoreError::from);
    }

    let result = match method {
        "host.getContext" => plugin_host_context(state, &plugin_id, &input.view_id),
        "plugin.files.readText" => plugin_file_read_text(root, &plugin_id, payload),
        "plugin.files.writeText" => plugin_file_write_text(root, &plugin_id, payload),
        "plugin.files.remove" => plugin_file_remove(root, &plugin_id, payload),
        "plugin.files.list" => plugin_file_list(root, &plugin_id, payload),
        "plugin.files.ensureDir" => plugin_file_ensure_dir(root, &plugin_id, payload),
        "workspace.readText" => plugin_workspace_read_text(root, payload),
        "workspace.list" => plugin_workspace_list(root, payload),
        "workspace.writeText" => plugin_workspace_write_text(root, payload),
        "workspace.ensureDir" => plugin_workspace_ensure_dir(root, payload),
        "workspace.removeFile" => plugin_workspace_remove_file(root, payload),
        "model.invoke" => plugin_model_invoke(state, payload).await,
        "model.list" => plugin_model_list(state),
        "network.fetch" => plugin_network_fetch(state, payload).await,
        "agent.list" => plugin_agent_list(state),
        "agent.run" => plugin_agent_run(state, &plugin_id, payload).await,
        "agent.task.start" => plugin_agent_task_start(state, &plugin_id, payload, progress).await,
        "agent.task.resume" => plugin_agent_task_resume(state, &plugin_id, payload, progress).await,
        "agent.task.retry" => plugin_agent_task_retry(state, &plugin_id, payload, progress).await,
        "agent.task.cancel" => plugin_agent_task_cancel(state, &plugin_id, payload),
        "agent.session.create" => plugin_agent_session_create(state, &plugin_id, payload),
        "agent.session.list" => plugin_agent_session_list(state, &plugin_id, payload),
        "agent.session.get" => plugin_agent_session_get(state, &plugin_id, payload),
        "agent.session.delete" => plugin_agent_session_delete(state, &plugin_id, payload),
        "agent.session.appendMessages" => {
            plugin_agent_session_append_messages(state, &plugin_id, payload)
        }
        "agent.session.stop" => plugin_agent_session_stop(state, &plugin_id, payload),
        "agent.session.removeAgentTool" => {
            plugin_agent_session_remove_agent_tool(state, &plugin_id, payload)
        }
        "agent.session.updateEntry" => {
            plugin_agent_session_update_entry(state, &plugin_id, payload)
        }
        _ => unreachable!("plugin RPC method {method} has a spec but no handler"),
    };
    result.map_err(CoreError::from)
}

pub(crate) fn plugin_host_context(
    state: &AgentStore,
    plugin_id: &str,
    view_id: &str,
) -> Result<Value, String> {
    let app_state = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?;
    let (manifest, view) = app_state.plugins.view_for(plugin_id, view_id)?;
    Ok(json!({
        "plugin": manifest,
        "view": view,
        "agents": app_state.agent_apps.iter().map(|agent| {
            json!({
                "id": agent.id,
                "name": agent.name,
                "providerKind": agent.provider_kind,
                "authConfigId": agent.auth_config_id,
                "model": agent.model,
            })
        }).collect::<Vec<_>>(),
        "runtime": {
            "status": app_state.runtime_status,
            "isRunning": !app_state.running_session_ids().is_empty(),
        }
    }))
}
