use crate::fs_utils::read_limited_utf8;
use crate::limits::APP_PRIVATE_DIR_NAME;
use crate::plugins::rpc::storage::sanitize_plugin_file_name;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) fn plugin_payload_path(payload: &Value) -> Result<&str, String> {
    payload
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "Plugin file operation requires payload.path.".to_string())
}

pub(crate) fn resolve_plugin_file_path(
    project_root: &Path,
    plugin_id: &str,
    relative_path: &str,
) -> Result<PathBuf, String> {
    let clean = clean_plugin_relative_path(relative_path)?;
    let relative = PathBuf::from(APP_PRIVATE_DIR_NAME)
        .join("plugin-files")
        .join(sanitize_plugin_file_name(plugin_id))
        .join(clean);
    reject_symlink_components(project_root, &relative)?;
    Ok(project_root.join(relative))
}

pub(crate) fn resolve_workspace_file_path(
    project_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    let clean = clean_plugin_relative_path(relative_path)?;
    if is_app_private_relative_path(&clean) {
        return Err("Workspace paths cannot access Daily private state.".into());
    }
    reject_symlink_components(project_root, &clean)?;
    Ok(project_root.join(clean))
}

pub(crate) fn resolve_sandboxed_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let clean = clean_plugin_relative_path(relative_path)?;
    reject_symlink_components(root, &clean)?;
    Ok(root.join(clean))
}

pub(crate) fn clean_plugin_relative_path(relative_path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(relative_path);
    if candidate.is_absolute() {
        return Err("Plugin paths must be relative.".into());
    }
    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                #[cfg(windows)]
                {
                    let name = part.to_string_lossy();
                    if name.contains(':') || name.ends_with(['.', ' ']) {
                        return Err("Plugin paths cannot use Windows file aliases or alternate data streams.".into());
                    }
                }
                clean.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Plugin paths cannot escape their sandbox.".into());
            }
        }
    }
    if clean.as_os_str().is_empty() {
        clean.push(".");
    }
    Ok(clean)
}

pub(crate) fn is_app_private_relative_path(relative_path: &Path) -> bool {
    matches!(
        relative_path.components().next(),
        Some(Component::Normal(part)) if part.to_string_lossy().eq_ignore_ascii_case(APP_PRIVATE_DIR_NAME)
    )
}

pub(crate) fn reject_symlink_components(root: &Path, relative_path: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative_path.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Plugin paths cannot traverse symbolic links: {}.",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "Failed to inspect plugin path {}: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn read_limited_text(path: &Path, limit: usize, label: &str) -> Result<String, String> {
    read_limited_utf8(path, limit, label)
}
