use crate::fs_utils::atomic_write;
use crate::fs_utils::sandbox::{
    is_app_private_relative_path, plugin_payload_path, read_limited_text, resolve_plugin_file_path,
    resolve_workspace_file_path,
};
use crate::limits::{
    FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES, MAX_DIRECTORY_ENTRIES, MAX_PLUGIN_FILE_COPY_BYTES,
};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub(crate) fn plugin_workspace_read_text(
    project_root: &Path,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_workspace_file_path(project_root, plugin_payload_path(&payload)?)?;
    let content = read_limited_text(
        &path,
        FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES,
        "workspace file",
    )?;
    Ok(json!({ "content": content }))
}

pub(crate) fn plugin_workspace_list(project_root: &Path, payload: Value) -> Result<Value, String> {
    let input_path = payload.get("path").and_then(Value::as_str).unwrap_or(".");
    let path = resolve_workspace_file_path(project_root, input_path)?;
    if !path.exists() {
        return Ok(json!({ "entries": [] }));
    }
    if !path.is_dir() {
        return Err("workspace.list path must be a directory.".into());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path)
        .map_err(|error| format!("Failed to list workspace files {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("Failed to inspect workspace file: {error}"))?;
        let file_name = entry.file_name();
        if is_app_private_relative_path(Path::new(&file_name)) {
            continue;
        }
        if entry
            .file_type()
            .map_err(|error| format!("Failed to inspect workspace file type: {error}"))?
            .is_symlink()
        {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("Failed to inspect workspace file metadata: {error}"))?;
        entries.push(json!({
            "name": file_name.to_string_lossy(),
            "isDirectory": metadata.is_dir(),
            "size": if metadata.is_file() { metadata.len() } else { 0 },
        }));
        if entries.len() > MAX_DIRECTORY_ENTRIES {
            return Err(format!(
                "workspace.list supports at most {MAX_DIRECTORY_ENTRIES} entries."
            ));
        }
    }
    entries.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .cmp(&right.get("name").and_then(Value::as_str))
    });
    Ok(json!({ "entries": entries }))
}

pub(crate) fn plugin_workspace_write_text(
    project_root: &Path,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_workspace_file_path(project_root, plugin_payload_path(&payload)?)?;
    let content = payload
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "workspace.writeText requires payload.content.".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create workspace directory {}: {error}",
                parent.display()
            )
        })?;
    }
    atomic_write(&path, content)
        .map_err(|error| format!("Failed to write workspace file {}: {error}", path.display()))?;
    Ok(json!({ "ok": true }))
}

pub(crate) fn plugin_workspace_ensure_dir(
    project_root: &Path,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_workspace_file_path(project_root, plugin_payload_path(&payload)?)?;
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "Failed to create workspace directory {}: {error}",
            path.display()
        )
    })?;
    Ok(json!({ "ok": true }))
}

pub(crate) fn plugin_workspace_remove_file(
    project_root: &Path,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_workspace_file_path(project_root, plugin_payload_path(&payload)?)?;
    if path.is_dir() {
        return Err("workspace.removeFile only removes files.".into());
    }
    if path.exists() {
        fs::remove_file(&path).map_err(|error| {
            format!(
                "Failed to remove workspace file {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(json!({ "ok": true }))
}

/// Copies a binary file out of the plugin's private folder, e.g. images for an export.
pub(crate) fn plugin_workspace_copy_from_plugin_files(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let field = |name: &str| {
        payload
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("workspace.copyFromPluginFiles requires payload.{name}."))
    };
    let source = resolve_plugin_file_path(project_root, plugin_id, field("from")?)?;
    let target = resolve_workspace_file_path(project_root, field("to")?)?;
    let metadata = fs::metadata(&source)
        .map_err(|error| format!("Failed to read plugin file {}: {error}", source.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_PLUGIN_FILE_COPY_BYTES {
        return Err(format!(
            "Plugin file {} is not a file or exceeds {MAX_PLUGIN_FILE_COPY_BYTES} bytes.",
            source.display()
        ));
    }
    let bytes = fs::read(&source)
        .map_err(|error| format!("Failed to read plugin file {}: {error}", source.display()))?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create workspace directory {}: {error}",
                parent.display()
            )
        })?;
    }
    atomic_write(&target, bytes).map_err(|error| {
        format!(
            "Failed to write workspace file {}: {error}",
            target.display()
        )
    })?;
    Ok(json!({ "ok": true }))
}
