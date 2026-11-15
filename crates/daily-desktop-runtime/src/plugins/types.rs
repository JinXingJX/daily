use crate::plugins::manifest::matches_support_filter;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginManifestView {
    pub identifier: String,
    pub name: String,
    pub description: String,
    pub payload_hint: String,
    pub version: String,
    pub author: String,
    pub source: PluginSourceKind,
    /// ES module the host imports, relative to the manifest directory.
    pub entry: Option<String>,
    /// Module path on the development server, used when `dev_url` is set.
    pub dev_entry: Option<String>,
    pub dev_url: Option<String>,
    pub activation: PluginActivation,
    pub installed: bool,
    pub enabled: bool,
    pub built_in: bool,
    pub contributes: PluginContributions,
    pub settings_schema: Value,
    pub settings: Value,
    pub runtime_errors: Vec<String>,
}

/// When the host loads a plugin module and runs its `activate`.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub enum PluginActivation {
    /// When the workspace opens, for plugins with background work.
    Startup,
    /// When one of its views is first opened.
    #[default]
    OnView,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum PluginSourceKind {
    BuiltIn,
    Marketplace,
    LocalDev,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct PluginContributions {
    pub renderers: Vec<PluginRendererContribution>,
    pub agent_tools: Vec<PluginAgentToolContribution>,
    pub views: Vec<PluginViewContribution>,
    pub themes: Vec<PluginThemeContribution>,
    pub hooks: Vec<PluginHookContribution>,
    pub event_view_parsers: Vec<PluginEventViewParserContribution>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginRendererContribution {
    pub id: String,
    pub name: String,
    pub description: String,
    pub payload_hint: String,
    pub legacy_identifier: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginAgentToolContribution {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_hint: String,
}

/// A page the plugin adds to the app rail; the plugin module mounts it by id.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginViewContribution {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginThemeContribution {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginHookContribution {
    pub id: String,
    pub phase: String,
    pub description: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginEventViewParserContribution {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub support: PluginEventViewParserSupport,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct PluginEventViewParserSupport {
    pub event_kinds: Vec<String>,
    pub tool_ids: Vec<String>,
    pub statuses: Vec<String>,
}

#[allow(dead_code)]
impl PluginEventViewParserSupport {
    pub fn supports(&self, event_kind: &str, tool_id: Option<&str>, status: &str) -> bool {
        matches_support_filter(&self.event_kinds, event_kind)
            && tool_id
                .map(|id| matches_support_filter(&self.tool_ids, id))
                .unwrap_or_else(|| self.tool_ids.is_empty())
            && matches_support_filter(&self.statuses, status)
    }
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginSnapshot {
    pub installed: Vec<PluginManifestView>,
    pub enabled: Vec<String>,
    pub marketplace: Vec<PluginMarketplaceEntry>,
    pub runtime_errors: Vec<PluginRuntimeError>,
    pub event_view_parsers: Vec<PluginEventViewParserRegistration>,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginEventViewParserRegistration {
    pub plugin_id: String,
    pub parser: PluginEventViewParserContribution,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginMarketplaceEntry {
    pub plugin: PluginManifestView,
    pub latest_version: String,
    pub installed: bool,
    pub enabled: bool,
    pub repository: String,
    pub tarball_url: String,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntimeError {
    pub plugin_id: String,
    pub message: String,
    pub timestamp: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct LocalDevPluginRef {
    pub plugin_id: String,
    pub manifest_path: String,
    pub dev_url: Option<String>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct InstallPluginInput {
    pub plugin_id: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PluginIdInput {
    pub plugin_id: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SavePluginSettingsInput {
    pub plugin_id: String,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct InstallLocalDevPluginInput {
    pub manifest_path: String,
    pub dev_url: Option<String>,
}
