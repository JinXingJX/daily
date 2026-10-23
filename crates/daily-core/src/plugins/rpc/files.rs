use crate::fs_utils::atomic_write;
use crate::fs_utils::sandbox::{plugin_payload_path, read_limited_text, resolve_plugin_file_path};
use crate::limits::{FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES, MAX_DIRECTORY_ENTRIES};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub(crate) fn plugin_file_read_text(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_plugin_file_path(project_root, plugin_id, plugin_payload_path(&payload)?)?;
    let content = read_limited_text(
        &path,
        FILE_WRITE_PLUGIN_RPC_PAYLOAD_LIMIT_BYTES,
        "plugin file",
    )?;
    Ok(json!({ "content": content }))
}

pub(crate) fn plugin_file_write_text(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_plugin_file_path(project_root, plugin_id, plugin_payload_path(&payload)?)?;
    let content = payload
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "plugin.files.writeText requires payload.content.".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create plugin file directory {}: {error}",
                parent.display()
            )
        })?;
    }
    atomic_write(&path, content)
        .map_err(|error| format!("Failed to write plugin file {}: {error}", path.display()))?;
    Ok(json!({ "ok": true }))
}

pub(crate) fn plugin_file_remove(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_plugin_file_path(project_root, plugin_id, plugin_payload_path(&payload)?)?;
    if path.is_dir() {
        return Err("plugin.files.remove only removes files.".into());
    }
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| format!("Failed to remove plugin file {}: {error}", path.display()))?;
    }
    Ok(json!({ "ok": true }))
}

pub(crate) fn plugin_file_list(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let input_path = payload.get("path").and_then(Value::as_str).unwrap_or(".");
    let path = resolve_plugin_file_path(project_root, plugin_id, input_path)?;
    if !path.exists() {
        return Ok(json!({ "entries": [] }));
    }
    if !path.is_dir() {
        return Err("plugin.files.list path must be a directory.".into());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path)
        .map_err(|error| format!("Failed to list plugin files {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("Failed to inspect plugin file: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("Failed to inspect plugin file type: {error}"))?
            .is_symlink()
        {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("Failed to inspect plugin file metadata: {error}"))?;
        entries.push(json!({
            "name": entry.file_name().to_string_lossy(),
            "isDirectory": metadata.is_dir(),
            "size": if metadata.is_file() { metadata.len() } else { 0 },
        }));
        if entries.len() > MAX_DIRECTORY_ENTRIES {
            return Err(format!(
                "plugin.files.list supports at most {MAX_DIRECTORY_ENTRIES} entries."
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

pub(crate) fn plugin_file_ensure_dir(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let path = resolve_plugin_file_path(project_root, plugin_id, plugin_payload_path(&payload)?)?;
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "Failed to create plugin directory {}: {error}",
            path.display()
        )
    })?;
    Ok(json!({ "ok": true }))
}
