use super::{
    canonical_plugin_id, canonical_plugin_ids, APP_DIR_NAME, BUILTIN_AGENT_HOOKS_ID,
    BUILTIN_CHART_ID, BUILTIN_WEATHER_ID, MAX_PLUGIN_CONFIG_BYTES, PLUGINS_FILE_NAME,
};
use crate::fs_utils::{atomic_write, read_limited_utf8};
use crate::plugins::builtin::{
    base_manifest, is_builtin_plugin, known_plugin_ids, marketplace_registry, push_unique,
};
use crate::plugins::manifest::{
    normalize_manifest_path, read_dev_manifest, runtime_error_manifest,
};
use crate::plugins::types::{
    InstallLocalDevPluginInput, LocalDevPluginRef, PluginEventViewParserContribution,
    PluginEventViewParserRegistration, PluginManifestView, PluginMarketplaceEntry,
    PluginRuntimeError, PluginSnapshot, PluginViewContribution,
};
use crate::util::unix_timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginState {
    pub installed_plugin_ids: Vec<String>,
    pub enabled_plugin_ids: Vec<String>,
    pub settings: BTreeMap<String, Value>,
    pub dev_plugins: Vec<LocalDevPluginRef>,
    pub runtime_errors: Vec<PluginRuntimeError>,
}

impl Default for PluginState {
    fn default() -> Self {
        let mut state = Self {
            installed_plugin_ids: Vec::new(),
            enabled_plugin_ids: Vec::new(),
            settings: BTreeMap::new(),
            dev_plugins: Vec::new(),
            runtime_errors: Vec::new(),
        };
        state.ensure_builtin_plugins(true);
        state
    }
}

impl PluginState {
    pub fn load(project_root: &Path) -> Result<Self, String> {
        let path = plugin_state_path(project_root);
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = read_limited_utf8(&path, MAX_PLUGIN_CONFIG_BYTES, "plugin state")?;
        let mut state = serde_json::from_str::<Self>(&raw)
            .map_err(|error| format!("Failed to parse plugin state {}: {error}", path.display()))?;
        state.ensure_builtin_plugins(false);
        Ok(state)
    }

    pub fn persist(&self, project_root: &Path) -> Result<(), String> {
        let path = plugin_state_path(project_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create plugin state directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let payload = serde_json::to_string_pretty(self)
            .map_err(|error| format!("Failed to serialize plugin state: {error}"))?;
        if payload.len() > MAX_PLUGIN_CONFIG_BYTES {
            return Err(format!(
                "Plugin state exceeds the {MAX_PLUGIN_CONFIG_BYTES} byte limit."
            ));
        }
        atomic_write(&path, format!("{payload}\n"))
            .map_err(|error| format!("Failed to write plugin state {}: {error}", path.display()))
    }

    pub fn snapshot(&self) -> PluginSnapshot {
        PluginSnapshot {
            installed: self.installed_manifests(),
            enabled: self.enabled_plugin_ids.clone(),
            marketplace: marketplace_registry()
                .into_iter()
                .map(|entry| {
                    let plugin = self.decorate_manifest(base_manifest(&entry.plugin_id));
                    PluginMarketplaceEntry {
                        installed: self.is_installed(&entry.plugin_id),
                        enabled: self.is_enabled(&entry.plugin_id),
                        latest_version: entry.latest_version,
                        repository: entry.repository,
                        tarball_url: entry.tarball_url,
                        sha256: entry.sha256,
                        plugin,
                    }
                })
                .collect(),
            runtime_errors: self.runtime_errors.clone(),
            event_view_parsers: self
                .event_view_parsers(&self.enabled_plugin_ids)
                .into_iter()
                .map(|(plugin_id, parser)| PluginEventViewParserRegistration { plugin_id, parser })
                .collect(),
        }
    }

    pub fn installed_manifests(&self) -> Vec<PluginManifestView> {
        let mut manifests = known_plugin_ids()
            .into_iter()
            .filter(|plugin_id| self.is_installed(plugin_id))
            .map(|plugin_id| self.decorate_manifest(base_manifest(plugin_id)))
            .collect::<Vec<_>>();

        for dev_plugin in &self.dev_plugins {
            let plugin_id = canonical_plugin_id(&dev_plugin.plugin_id);
            if !self.is_installed(&plugin_id) {
                continue;
            }
            match self.load_dev_manifest(dev_plugin) {
                Ok(manifest) => manifests.push(self.decorate_manifest(manifest)),
                Err(error) => manifests.push(self.decorate_manifest(runtime_error_manifest(
                    &plugin_id,
                    &dev_plugin.manifest_path,
                    error,
                ))),
            }
        }

        manifests.sort_by(|left, right| left.name.cmp(&right.name));
        manifests
    }

    pub fn prompt_manifests(&self, mounted_plugin_ids: &[String]) -> Vec<PluginManifestView> {
        let mounted = mounted_plugin_ids
            .iter()
            .map(|plugin_id| canonical_plugin_id(plugin_id))
            .collect::<BTreeSet<_>>();

        self.installed_manifests()
            .into_iter()
            .filter(|manifest| manifest.enabled && mounted.contains(&manifest.identifier))
            .collect()
    }

    pub fn event_view_parsers(
        &self,
        mounted_plugin_ids: &[String],
    ) -> Vec<(String, PluginEventViewParserContribution)> {
        self.prompt_manifests(mounted_plugin_ids)
            .into_iter()
            .flat_map(|manifest| {
                manifest
                    .contributes
                    .event_view_parsers
                    .into_iter()
                    .map(move |parser| (manifest.identifier.clone(), parser))
            })
            .collect()
    }

    pub fn install(&mut self, plugin_id: &str) -> Result<(), String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        self.ensure_known_plugin(&plugin_id)?;
        push_unique(&mut self.installed_plugin_ids, plugin_id);
        self.ensure_builtin_plugins(false);
        Ok(())
    }

    pub fn install_local_dev(&mut self, input: InstallLocalDevPluginInput) -> Result<(), String> {
        let manifest_path = normalize_manifest_path(&input.manifest_path)?;
        let dev_url = input
            .dev_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let manifest = read_dev_manifest(&manifest_path, dev_url.as_deref())?;
        let plugin_id = canonical_plugin_id(&manifest.identifier);
        if known_plugin_ids().iter().any(|id| *id == plugin_id) {
            return Err(format!(
                "Local dev plugin id {plugin_id} conflicts with a built-in or marketplace plugin."
            ));
        }

        self.dev_plugins.retain(|item| {
            canonical_plugin_id(&item.plugin_id) != plugin_id
                && item.manifest_path != manifest_path.display().to_string()
        });
        self.dev_plugins.push(LocalDevPluginRef {
            plugin_id: plugin_id.clone(),
            manifest_path: manifest_path.display().to_string(),
            dev_url,
        });
        push_unique(&mut self.installed_plugin_ids, plugin_id.clone());
        push_unique(&mut self.enabled_plugin_ids, plugin_id);
        Ok(())
    }

    pub fn uninstall(&mut self, plugin_id: &str) -> Result<(), String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        if is_builtin_plugin(&plugin_id) {
            return Err("Built-in plugins can be disabled but cannot be uninstalled.".into());
        }
        self.installed_plugin_ids.retain(|id| id != &plugin_id);
        self.enabled_plugin_ids.retain(|id| id != &plugin_id);
        self.dev_plugins
            .retain(|item| canonical_plugin_id(&item.plugin_id) != plugin_id);
        self.settings.remove(&plugin_id);
        Ok(())
    }

    pub fn enable(&mut self, plugin_id: &str) -> Result<(), String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        if !self.is_installed(&plugin_id) {
            return Err(format!("Plugin {plugin_id} is not installed."));
        }
        push_unique(&mut self.enabled_plugin_ids, plugin_id);
        Ok(())
    }

    pub fn disable(&mut self, plugin_id: &str) -> Result<(), String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        self.enabled_plugin_ids.retain(|id| id != &plugin_id);
        Ok(())
    }

    pub fn save_settings(&mut self, plugin_id: &str, settings: Value) -> Result<(), String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        self.ensure_known_plugin(&plugin_id)?;
        self.settings.insert(plugin_id, settings);
        Ok(())
    }

    pub fn record_runtime_error(&mut self, plugin_id: &str, message: impl Into<String>) {
        self.runtime_errors.push(PluginRuntimeError {
            plugin_id: canonical_plugin_id(plugin_id),
            message: message.into(),
            timestamp: unix_timestamp(),
        });
        if self.runtime_errors.len() > 50 {
            let overflow = self.runtime_errors.len() - 50;
            self.runtime_errors.drain(0..overflow);
        }
    }

    pub fn remove_plugin_from_agents(&self, plugin_id: &str, mounted_plugin_ids: &mut Vec<String>) {
        let plugin_id = canonical_plugin_id(plugin_id);
        mounted_plugin_ids.retain(|id| canonical_plugin_id(id) != plugin_id);
    }

    pub fn canonicalize_agent_mounts(&self, mounted_plugin_ids: Vec<String>) -> Vec<String> {
        canonical_plugin_ids(mounted_plugin_ids)
            .into_iter()
            .filter(|plugin_id| self.is_installed(plugin_id))
            .collect()
    }

    pub fn manifest_for(&self, plugin_id: &str) -> Result<PluginManifestView, String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        if known_plugin_ids().iter().any(|id| *id == plugin_id) {
            return Ok(self.decorate_manifest(base_manifest(&plugin_id)));
        }
        let Some(dev_plugin) = self
            .dev_plugins
            .iter()
            .find(|item| canonical_plugin_id(&item.plugin_id) == plugin_id)
        else {
            return Err(format!(
                "Plugin {plugin_id} was not found in the configured registry."
            ));
        };
        Ok(self.decorate_manifest(self.load_dev_manifest(dev_plugin)?))
    }

    pub fn view_for(
        &self,
        plugin_id: &str,
        view_id: &str,
    ) -> Result<(PluginManifestView, PluginViewContribution), String> {
        let manifest = self.manifest_for(plugin_id)?;
        let Some(view) = manifest
            .contributes
            .views
            .iter()
            .find(|view| view.id == view_id)
            .cloned()
        else {
            return Err(format!(
                "Plugin {} does not contribute view {}.",
                manifest.identifier, view_id
            ));
        };
        Ok((manifest, view))
    }

    pub fn local_dev_manifest_path(&self, plugin_id: &str) -> Option<PathBuf> {
        let plugin_id = canonical_plugin_id(plugin_id);
        self.dev_plugins
            .iter()
            .find(|item| canonical_plugin_id(&item.plugin_id) == plugin_id)
            .map(|item| PathBuf::from(&item.manifest_path))
    }

    pub(crate) fn is_installed(&self, plugin_id: &str) -> bool {
        self.installed_plugin_ids
            .iter()
            .any(|id| canonical_plugin_id(id) == plugin_id)
    }

    pub(crate) fn is_enabled(&self, plugin_id: &str) -> bool {
        self.enabled_plugin_ids
            .iter()
            .any(|id| canonical_plugin_id(id) == plugin_id)
    }

    pub(crate) fn decorate_manifest(&self, mut manifest: PluginManifestView) -> PluginManifestView {
        let plugin_id = manifest.identifier.clone();
        manifest.installed = self.is_installed(&plugin_id);
        manifest.enabled = self.is_enabled(&plugin_id);
        manifest.settings = self
            .settings
            .get(&plugin_id)
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut runtime_errors = manifest.runtime_errors.clone();
        runtime_errors.extend(
            self.runtime_errors
                .iter()
                .filter(|error| error.plugin_id == plugin_id)
                .map(|error| error.message.clone()),
        );
        manifest.runtime_errors = runtime_errors;
        manifest
    }

    pub(crate) fn ensure_known_plugin(&self, plugin_id: &str) -> Result<(), String> {
        let plugin_id = canonical_plugin_id(plugin_id);
        if known_plugin_ids().iter().any(|id| *id == plugin_id)
            || self
                .dev_plugins
                .iter()
                .any(|item| canonical_plugin_id(&item.plugin_id) == plugin_id)
        {
            Ok(())
        } else {
            Err(format!(
                "Plugin {plugin_id} was not found in the configured registry."
            ))
        }
    }

    pub(crate) fn load_dev_manifest(
        &self,
        dev_plugin: &LocalDevPluginRef,
    ) -> Result<PluginManifestView, String> {
        let path = Path::new(&dev_plugin.manifest_path);
        // Plugin state lives in the workspace; never resolve a path it names relative to it.
        if !path.is_absolute() {
            return Err("Local plugin manifest path must be absolute.".into());
        }
        read_dev_manifest(path, dev_plugin.dev_url.as_deref())
    }

    pub(crate) fn ensure_builtin_plugins(&mut self, enable_missing: bool) {
        for plugin_id in [BUILTIN_CHART_ID, BUILTIN_WEATHER_ID, BUILTIN_AGENT_HOOKS_ID] {
            let was_installed = self.is_installed(plugin_id);
            push_unique(&mut self.installed_plugin_ids, plugin_id.to_string());
            if enable_missing || !was_installed {
                push_unique(&mut self.enabled_plugin_ids, plugin_id.to_string());
            }
        }
        self.installed_plugin_ids = canonical_plugin_ids(self.installed_plugin_ids.clone());
        self.enabled_plugin_ids = canonical_plugin_ids(self.enabled_plugin_ids.clone());
    }
}

pub(crate) fn plugin_state_path(project_root: &Path) -> PathBuf {
    project_root.join(APP_DIR_NAME).join(PLUGINS_FILE_NAME)
}
