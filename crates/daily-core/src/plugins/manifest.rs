use super::{canonical_plugin_id, MAX_PLUGIN_CONFIG_BYTES};
use crate::fs_utils::read_limited_utf8;
use crate::plugins::types::{PluginContributions, PluginManifestView, PluginSourceKind};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DevPluginManifestFile {
    pub identifier: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub payload_hint: String,
    #[serde(default = "default_plugin_version")]
    pub version: String,
    #[serde(default = "default_plugin_author")]
    pub author: String,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub contributes: PluginContributions,
    #[serde(default)]
    pub settings_schema: Value,
}

pub(crate) fn normalize_manifest_path(input: &str) -> Result<PathBuf, String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("Local plugin manifest path is required.".into());
    }
    let path = PathBuf::from(raw);
    // Relative paths would resolve against the sidecar's working directory (the workspace),
    // which would let a cloned repository point at plugins it ships itself.
    if !path.is_absolute() {
        return Err("Local plugin manifest path must be absolute.".into());
    }
    let path = if path.is_dir() {
        path.join(".daily-plugin").join("plugin.json")
    } else {
        path
    };
    if !path.is_file() {
        return Err(format!(
            "Local plugin manifest {} was not found.",
            path.display()
        ));
    }
    path.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve local plugin manifest {}: {error}",
            path.display()
        )
    })
}

pub(crate) fn read_dev_manifest(
    path: &Path,
    dev_url: Option<&str>,
) -> Result<PluginManifestView, String> {
    let raw = read_limited_utf8(path, MAX_PLUGIN_CONFIG_BYTES, "local plugin manifest")?;
    let file = serde_json::from_str::<DevPluginManifestFile>(&raw).map_err(|error| {
        format!(
            "Failed to parse local plugin manifest {}: {error}",
            path.display()
        )
    })?;
    let plugin_id = canonical_plugin_id(&file.identifier);
    if !is_valid_contribution_id(&plugin_id) {
        return Err(
            "Local plugin identifier must be 1-128 ASCII letters, numbers, dots, underscores, or hyphens."
                .into(),
        );
    }

    if file.contributes.views.len() > 20 {
        return Err("Local plugin manifest cannot contribute more than 20 views.".into());
    }
    if let Some(entry) = file.entry.as_deref() {
        validate_manifest_entry(entry, "plugin entry")?;
    }
    let normalized_dev_url = dev_url.map(validate_dev_url).transpose()?;

    let mut contributes = file.contributes;
    let mut view_ids = BTreeSet::new();
    for view in &mut contributes.views {
        if !is_valid_contribution_id(&view.id) || !view_ids.insert(view.id.clone()) {
            return Err(format!(
                "Local plugin view id '{}' is invalid or duplicated.",
                view.id
            ));
        }
        if !matches!(view.location.as_str(), "main" | "background") {
            return Err(format!(
                "Local plugin view '{}' has unsupported location '{}'.",
                view.id, view.location
            ));
        }
        if view.runtime.trim().is_empty() {
            view.runtime = "iframe".into();
        }
        if view.runtime != "iframe" {
            return Err(format!(
                "Local plugin view '{}' must use the iframe runtime.",
                view.id
            ));
        }
        if let Some(entry) = view.entry.as_deref() {
            validate_manifest_entry(entry, "plugin view entry")?;
        }
        if let Some(dev_entry) = view.dev_entry.as_deref() {
            validate_manifest_entry(dev_entry, "plugin development entry")?;
        }
        if let Some(dev_url) = normalized_dev_url.as_deref() {
            view.dev_url = Some(dev_url.to_string());
        }
    }

    Ok(PluginManifestView {
        identifier: plugin_id,
        name: normalize_manifest_text(&file.name, "Local Plugin"),
        description: file.description,
        payload_hint: file.payload_hint,
        version: file.version,
        author: file.author,
        source: PluginSourceKind::LocalDev,
        entry: file.entry,
        installed: false,
        enabled: false,
        built_in: false,
        contributes,
        settings_schema: if file.settings_schema.is_null() {
            json!({})
        } else {
            file.settings_schema
        },
        settings: json!({}),
        runtime_errors: Vec::new(),
    })
}

pub(crate) fn is_valid_contribution_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

pub(crate) fn validate_manifest_entry(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value.trim());
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!("Local {label} must be a sandboxed relative path."));
    }
    Ok(())
}

pub(crate) fn validate_dev_url(value: &str) -> Result<String, String> {
    let mut url = Url::parse(value.trim())
        .map_err(|error| format!("Local plugin development URL is invalid: {error}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Local plugin development URL must be an HTTP(S) URL without credentials.".into(),
        );
    }
    // Dev servers run on this machine; remote URLs would load arbitrary code into the host.
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if !loopback {
        return Err(
            "Local plugin development URL must point at localhost, 127.0.0.1, or [::1].".into(),
        );
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

pub(crate) fn runtime_error_manifest(
    plugin_id: &str,
    manifest_path: &str,
    error: String,
) -> PluginManifestView {
    PluginManifestView {
        identifier: plugin_id.into(),
        name: plugin_id.into(),
        description: format!("Local development plugin could not be loaded from {manifest_path}."),
        payload_hint: String::new(),
        version: "0.0.0".into(),
        author: "Local Dev".into(),
        source: PluginSourceKind::LocalDev,
        entry: None,
        installed: true,
        enabled: false,
        built_in: false,
        contributes: PluginContributions::default(),
        settings_schema: json!({}),
        settings: json!({}),
        runtime_errors: vec![error],
    }
}

pub(crate) fn normalize_manifest_text(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.into()
    } else {
        trimmed.into()
    }
}

#[allow(dead_code)]
pub(crate) fn matches_support_filter(filter: &[String], value: &str) -> bool {
    filter.is_empty()
        || filter
            .iter()
            .any(|item| item == "*" || item.eq_ignore_ascii_case(value))
}

pub(crate) fn default_view_runtime() -> String {
    "iframe".into()
}

pub(crate) fn default_plugin_version() -> String {
    "0.1.0".into()
}

pub(crate) fn default_plugin_author() -> String {
    "Local Dev".into()
}
