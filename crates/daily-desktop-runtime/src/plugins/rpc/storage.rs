use crate::fs_utils::atomic_write;
use crate::fs_utils::sandbox::{read_limited_text, reject_symlink_components};
use crate::limits::{APP_PRIVATE_DIR_NAME, PLUGIN_STORAGE_LIMIT_BYTES};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn plugin_storage_get(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let key = plugin_storage_key(&payload, "storage.get")?;
    let storage = read_plugin_storage(project_root, plugin_id)?;
    Ok(json!({
        "value": storage.get(key).cloned().unwrap_or(Value::Null)
    }))
}

pub(crate) fn plugin_storage_set(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let key = plugin_storage_key(&payload, "storage.set")?;
    let value = payload.get("value").cloned().unwrap_or(Value::Null);
    let mut storage = read_plugin_storage(project_root, plugin_id)?;
    storage.insert(key.to_string(), value);
    write_plugin_storage(project_root, plugin_id, &storage)?;
    Ok(json!({ "ok": true }))
}

pub(crate) fn plugin_storage_compare_and_swap(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let key = plugin_storage_key(&payload, "storage.compareAndSwap")?;
    let expected = payload.get("expected").cloned().unwrap_or(Value::Null);
    let remove = payload
        .get("remove")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next = payload.get("value").cloned().unwrap_or(Value::Null);
    let mut storage = read_plugin_storage(project_root, plugin_id)?;
    let current = storage.get(key).cloned().unwrap_or(Value::Null);
    if current != expected {
        return Ok(json!({ "swapped": false, "value": current }));
    }
    if remove {
        storage.remove(key);
    } else {
        storage.insert(key.to_string(), next.clone());
    }
    write_plugin_storage(project_root, plugin_id, &storage)?;
    Ok(json!({
        "swapped": true,
        "value": if remove { Value::Null } else { next },
    }))
}

pub(crate) fn plugin_storage_remove(
    project_root: &Path,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let key = plugin_storage_key(&payload, "storage.remove")?;
    let mut storage = read_plugin_storage(project_root, plugin_id)?;
    storage.remove(key);
    write_plugin_storage(project_root, plugin_id, &storage)?;
    Ok(json!({ "ok": true }))
}

pub(crate) fn plugin_storage_list(project_root: &Path, plugin_id: &str) -> Result<Value, String> {
    Ok(Value::Object(read_plugin_storage(project_root, plugin_id)?))
}

pub(crate) fn plugin_storage_key<'a>(payload: &'a Value, method: &str) -> Result<&'a str, String> {
    let key = payload
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{method} requires payload.key."))?;
    if key.trim().is_empty() {
        return Err(format!("{method} key cannot be empty."));
    }
    if key.len() > 256 || key.chars().any(char::is_control) {
        return Err(format!(
            "{method} key must be at most 256 bytes and contain no control characters."
        ));
    }
    Ok(key)
}

pub(crate) fn read_plugin_storage(
    project_root: &Path,
    plugin_id: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let path = plugin_storage_path(project_root, plugin_id)?;
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let raw = read_limited_text(&path, PLUGIN_STORAGE_LIMIT_BYTES, "plugin storage")?;
    match serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("Failed to parse plugin storage {}: {error}", path.display()))?
    {
        Value::Object(map) => Ok(map),
        _ => Ok(serde_json::Map::new()),
    }
}

pub(crate) fn write_plugin_storage(
    project_root: &Path,
    plugin_id: &str,
    storage: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let path = plugin_storage_path(project_root, plugin_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create plugin storage directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let payload = serde_json::to_string_pretty(storage)
        .map_err(|error| format!("Failed to serialize plugin storage: {error}"))?;
    if payload.len() > PLUGIN_STORAGE_LIMIT_BYTES {
        return Err(format!(
            "Plugin storage exceeds the {PLUGIN_STORAGE_LIMIT_BYTES} byte limit."
        ));
    }
    atomic_write(&path, format!("{payload}\n"))
        .map_err(|error| format!("Failed to write plugin storage {}: {error}", path.display()))
}

pub(crate) fn plugin_storage_path(project_root: &Path, plugin_id: &str) -> Result<PathBuf, String> {
    let relative = PathBuf::from(APP_PRIVATE_DIR_NAME)
        .join("plugin-storage")
        .join(format!("{}.json", sanitize_plugin_file_name(plugin_id)));
    reject_symlink_components(project_root, &relative)?;
    Ok(project_root.join(relative))
}

pub(crate) fn sanitize_plugin_file_name(plugin_id: &str) -> String {
    plugin_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
