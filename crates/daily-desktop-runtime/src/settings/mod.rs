use crate::agent::{AgentConfig, AgentConfigView};
use crate::fs_utils::{atomic_write, atomic_write_private, read_limited_utf8, restrict_to_owner};
use crate::limits::{CHAT_MAX_ITERATIONS, MAX_OUTPUT_TOKENS};
use crate::model::{
    default_codex_models, normalize_codex_model, normalize_provider_model, ModelAuthConfig,
    ModelConfig, ModelConfigView, ModelProviderKind, DEFAULT_CODEX_AUTH_CONFIG_ID,
    DEFAULT_CODEX_MODEL, DEFAULT_CODEX_URL,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const APP_SETTINGS_DIR: &str = ".daily";
const SETTINGS_FILE_NAME: &str = "settings.json";
const LOCAL_SETTINGS_FILE_NAME: &str = "settings.local.json";
const MAX_SETTINGS_FILE_BYTES: usize = 2 * 1024 * 1024;
const USER_SETTINGS_TEMPLATE: &str = include_str!("../../defaults/user-settings.json");
const PRECEDENCE_NOTE: &str = "Precedence: local > project > user. More specific scopes override broader ones. Provider configurations (endpoints and keys) are read from user and local settings only.";
const PROJECT_AUTH_IGNORED: &str = "Project settings define auth.configs, which are ignored: provider endpoints and keys are only read from user or local settings so a shared repository cannot redirect your credentials.";

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SettingsScope {
    User,
    Project,
    Local,
}

impl SettingsScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Local => "local",
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsDocument {
    pub agent: AgentSettingsPatch,
    pub model: ModelSettingsPatch,
    pub auth: AuthSettingsPatch,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSettingsPatch {
    pub system_prompt: Option<String>,
    pub max_iterations: Option<u16>,
    pub dialogue_window: Option<u8>,
    pub memory_window: Option<u8>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct ModelSettingsPatch {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", default)]
pub struct AuthSettingsPatch {
    pub selected_auth_config_id: Option<String>,
    pub configs: Vec<ModelAuthConfig>,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub precedence_note: String,
    /// Problems found while merging scopes, shown next to the settings editor.
    pub warnings: Vec<String>,
    pub effective: EffectiveSettingsView,
    pub scopes: Vec<SettingsScopeView>,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct EffectiveSettingsView {
    pub agent: AgentConfigView,
    pub model: ModelConfigView,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SettingsScopeView {
    pub scope: SettingsScope,
    pub path: String,
    pub exists: bool,
    pub editable: bool,
    pub content: String,
    pub document: SettingsDocument,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SaveSettingsFileInput {
    pub scope: SettingsScope,
    pub content: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SaveCodexAuthSettingsInput {
    pub id: Option<String>,
    pub name: Option<String>,
    pub codex_home: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    pub follow_codex: Option<bool>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SaveAuthSettingsInput {
    pub config: ModelAuthConfig,
    /// Also make this configuration the default provider (the default when omitted).
    #[serde(default = "default_select")]
    #[cfg_attr(feature = "ts", ts(optional, as = "Option<bool>"))]
    pub select: bool,
}

fn default_select() -> bool {
    true
}

#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DeleteAuthSettingsInput {
    pub id: String,
}

pub struct LoadedSettings {
    pub agent: AgentConfig,
    pub model: ModelConfig,
    pub view: SettingsView,
}

pub fn default_settings(project_root: &Path) -> Result<LoadedSettings, String> {
    let default_document = default_user_settings_document()?;
    let mut agent = AgentConfig::seeded();
    let mut model = empty_model_config();
    apply_agent_patch(&mut agent, &default_document.agent);
    apply_model_patch(&mut model, &default_document.model);
    apply_auth_patch(&mut model, &default_document.auth);
    normalize_model_selection(&mut model);
    let view = SettingsView {
        precedence_note: PRECEDENCE_NOTE.into(),
        warnings: Vec::new(),
        effective: EffectiveSettingsView {
            agent: agent.view(),
            model: model.view(),
        },
        scopes: [
            SettingsScope::User,
            SettingsScope::Project,
            SettingsScope::Local,
        ]
        .into_iter()
        .map(|scope| {
            Ok(SettingsScopeView {
                scope,
                path: scope_path(project_root, scope)?.display().to_string(),
                exists: false,
                editable: true,
                content: if scope == SettingsScope::User {
                    USER_SETTINGS_TEMPLATE.to_string()
                } else {
                    String::new()
                },
                document: if scope == SettingsScope::User {
                    default_document.clone()
                } else {
                    SettingsDocument::default()
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?,
    };

    Ok(LoadedSettings { agent, model, view })
}

pub fn load_settings(project_root: &Path) -> Result<LoadedSettings, String> {
    seed_user_settings_if_missing(project_root)?;
    protect_credential_files(project_root);
    let mut scopes = load_scope_documents(project_root)?;
    let mut agent = AgentConfig::seeded();
    let mut model = empty_model_config();
    let mut warnings = Vec::new();

    // Lowest precedence to highest precedence.
    for scope in [
        SettingsScope::User,
        SettingsScope::Project,
        SettingsScope::Local,
    ] {
        if let Some(document) = scopes
            .iter()
            .find(|entry| entry.scope == scope)
            .map(|entry| entry.document.clone())
        {
            apply_agent_patch(&mut agent, &document.agent);
            apply_model_patch(&mut model, &document.model);
            let mut auth = document.auth;
            if scope == SettingsScope::Project && !auth.configs.is_empty() {
                auth.configs.clear();
                warnings.push(PROJECT_AUTH_IGNORED.to_string());
            }
            apply_auth_patch(&mut model, &auth);
        }
    }
    normalize_model_selection(&mut model);

    let view = SettingsView {
        precedence_note: PRECEDENCE_NOTE.into(),
        warnings,
        effective: EffectiveSettingsView {
            agent: agent.view(),
            model: model.view(),
        },
        scopes: scopes
            .drain(..)
            .map(|entry| SettingsScopeView {
                scope: entry.scope,
                path: entry.path.display().to_string(),
                exists: entry.exists,
                editable: true,
                content: entry.content,
                document: entry.document,
            })
            .collect(),
    };

    Ok(LoadedSettings { agent, model, view })
}

pub fn save_settings_file(
    project_root: &Path,
    input: SaveSettingsFileInput,
) -> Result<LoadedSettings, String> {
    let path = scope_path(project_root, input.scope)?;
    if input.content.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(format!(
            "{} settings exceed the {MAX_SETTINGS_FILE_BYTES} byte limit.",
            input.scope.label()
        ));
    }
    let trimmed = input.content.trim();

    if trimmed.is_empty() {
        if input.scope == SettingsScope::User {
            write_user_settings_template(&path)?;
        } else if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "Failed to remove {} settings at {}: {error}",
                    input.scope.label(),
                    path.display()
                )
            })?;
        }
        return load_settings(project_root);
    }

    serde_json::from_str::<SettingsDocument>(&input.content).map_err(|error| {
        format!(
            "Failed to parse {} settings JSON at {}: {error}",
            input.scope.label(),
            path.display()
        )
    })?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create settings directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let payload = if input.content.ends_with('\n') {
        input.content
    } else {
        format!("{}\n", input.content)
    };
    write_scope_file(input.scope, &path, payload)
        .map_err(|error| format!("Failed to write settings file {}: {error}", path.display()))?;

    load_settings(project_root)
}

pub fn save_codex_auth_settings(
    project_root: &Path,
    input: SaveCodexAuthSettingsInput,
) -> Result<LoadedSettings, String> {
    let existing = load_settings(project_root)?;
    let id = normalize_optional_string(input.id.as_deref(), DEFAULT_CODEX_AUTH_CONFIG_ID);
    let follow_codex = input.follow_codex.unwrap_or_else(|| {
        existing
            .model
            .auth_configs
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.follow_codex)
            .unwrap_or(true)
    });
    save_auth_config(
        project_root,
        ModelAuthConfig {
            id,
            name: normalize_optional_string(input.name.as_deref(), "Codex"),
            provider_kind: ModelProviderKind::CodexLogin,
            base_url: normalize_optional_string(input.base_url.as_deref(), DEFAULT_CODEX_URL),
            codex_home: normalize_optional_path(input.codex_home.as_deref()),
            default_model: normalize_optional_string(
                input.default_model.as_deref(),
                DEFAULT_CODEX_MODEL,
            ),
            models: normalize_model_list(input.models),
            follow_codex,
            api_key: String::new(),
            api_key_env: None,
        },
        true,
    )
}

pub fn save_auth_settings(
    project_root: &Path,
    input: SaveAuthSettingsInput,
) -> Result<LoadedSettings, String> {
    save_auth_config(project_root, input.config, input.select)
}

/// Removes a provider configuration from user settings.
pub fn delete_auth_settings(
    project_root: &Path,
    input: DeleteAuthSettingsInput,
) -> Result<LoadedSettings, String> {
    edit_user_settings(project_root, |root| {
        let auth = root
            .get_mut("auth")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| "User settings have no provider configurations.".to_string())?;
        let mut configs = auth
            .get("configs")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<ModelAuthConfig>>(value).ok())
            .unwrap_or_default();
        let before = configs.len();
        configs.retain(|config| config.id != input.id);
        if configs.len() == before {
            return Err(format!(
                "Provider configuration {} was not found.",
                input.id
            ));
        }
        if configs.is_empty() {
            return Err("At least one provider configuration must remain.".into());
        }
        if auth
            .get("selectedAuthConfigId")
            .and_then(serde_json::Value::as_str)
            == Some(&input.id)
        {
            auth.insert(
                "selectedAuthConfigId".into(),
                serde_json::Value::String(configs[0].id.clone()),
            );
        }
        auth.insert(
            "configs".into(),
            serde_json::to_value(configs).map_err(|error| error.to_string())?,
        );
        Ok(())
    })
}

pub(crate) fn save_auth_config(
    project_root: &Path,
    config: ModelAuthConfig,
    select: bool,
) -> Result<LoadedSettings, String> {
    let config = config.normalized();
    edit_user_settings(project_root, |root| {
        update_auth_document(root, config, select)
    })
}

/// Applies `edit` to the user settings JSON (preserving unknown keys) and reloads.
fn edit_user_settings(
    project_root: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<(), String>,
) -> Result<LoadedSettings, String> {
    seed_user_settings_if_missing(project_root)?;
    let path = scope_path(project_root, SettingsScope::User)?;
    let raw = read_limited_utf8(&path, MAX_SETTINGS_FILE_BYTES, "user settings")?;
    let mut document: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Failed to parse user settings file {}: {error}",
            path.display()
        )
    })?;
    let root = document
        .as_object_mut()
        .ok_or_else(|| "User settings must be a JSON object.".to_string())?;
    edit(root)?;

    let payload = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("Failed to serialize user settings: {error}"))?;
    if payload.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(format!(
            "User settings exceed the {MAX_SETTINGS_FILE_BYTES} byte limit."
        ));
    }
    atomic_write_private(&path, format!("{payload}\n")).map_err(|error| {
        format!(
            "Failed to write user settings file {}: {error}",
            path.display()
        )
    })?;

    load_settings(project_root)
}

fn update_auth_document(
    root: &mut serde_json::Map<String, serde_json::Value>,
    config: ModelAuthConfig,
    select: bool,
) -> Result<(), String> {
    let existing_configs = root
        .get("auth")
        .and_then(|auth| auth.get("configs"))
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<ModelAuthConfig>>(value).ok())
        .unwrap_or_default();
    let selected_config_id = config.id.clone();
    let selected_default_model = config.default_model.clone();

    if select {
        let model = root
            .entry("model")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let model = model
            .as_object_mut()
            .ok_or_else(|| "User settings model section must be a JSON object.".to_string())?;
        model.insert(
            "model".into(),
            serde_json::Value::String(selected_default_model),
        );
    }

    let auth = root
        .entry("auth")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let auth = auth
        .as_object_mut()
        .ok_or_else(|| "User settings auth section must be a JSON object.".to_string())?;

    let mut configs = existing_configs
        .into_iter()
        .map(ModelAuthConfig::normalized)
        .collect::<Vec<_>>();
    if let Some(index) = configs.iter().position(|existing| existing.id == config.id) {
        configs[index] = config;
    } else {
        configs.push(config);
    }
    configs = dedupe_auth_configs(configs);

    if select {
        auth.insert(
            "selectedAuthConfigId".into(),
            serde_json::Value::String(selected_config_id),
        );
    }
    auth.insert(
        "configs".into(),
        serde_json::to_value(configs)
            .map_err(|error| format!("Failed to serialize auth config: {error}"))?,
    );

    Ok(())
}

fn apply_agent_patch(config: &mut AgentConfig, patch: &AgentSettingsPatch) {
    if let Some(system_prompt) = patch.system_prompt.as_deref() {
        let value = system_prompt.trim();
        if !value.is_empty() {
            config.system_prompt = value.to_string();
        }
    }
    if let Some(max_iterations) = patch.max_iterations {
        config.max_iterations = max_iterations.clamp(1, CHAT_MAX_ITERATIONS as u16);
    }
    if let Some(dialogue_window) = patch.dialogue_window {
        config.dialogue_window = dialogue_window.clamp(1, 24);
    }
    if let Some(memory_window) = patch.memory_window {
        config.memory_window = memory_window.clamp(1, 48);
    }
}

fn apply_model_patch(config: &mut ModelConfig, patch: &ModelSettingsPatch) {
    if let Some(model) = patch.model.as_deref() {
        let value = model.trim();
        if !value.is_empty() {
            config.model = value.to_string();
        }
    }
    if let Some(temperature) = patch.temperature {
        config.temperature = temperature.clamp(0.0, 2.0);
    }
    if let Some(max_output_tokens) = patch.max_output_tokens {
        config.max_output_tokens = max_output_tokens.clamp(128, MAX_OUTPUT_TOKENS);
    }
}

/// Merges a scope's provider configurations by id: a config replaces the one with the same id
/// from a lower scope and new ids are appended. The scope's selection wins when it names a
/// known config.
fn apply_auth_patch(config: &mut ModelConfig, patch: &AuthSettingsPatch) {
    for auth_config in &patch.configs {
        let auth_config = auth_config.clone().normalized();
        match config
            .auth_configs
            .iter()
            .position(|existing| existing.id == auth_config.id)
        {
            Some(index) => config.auth_configs[index] = auth_config,
            None => config.auth_configs.push(auth_config),
        }
    }
    if let Some(selected) = patch
        .selected_auth_config_id
        .as_deref()
        .map(str::trim)
        .filter(|id| config.auth_configs.iter().any(|c| c.id == *id))
    {
        config.auth_config_id = selected.to_string();
    }
}

/// Provider configurations come only from settings files; the Codex default is added back
/// by [`normalize_model_selection`] when no scope defines any.
fn empty_model_config() -> ModelConfig {
    let mut config = ModelConfig::seeded();
    config.auth_configs.clear();
    config
}

fn normalize_model_selection(config: &mut ModelConfig) {
    config.auth_configs = dedupe_auth_configs(
        config
            .auth_configs
            .clone()
            .into_iter()
            .map(|auth_config| auth_config.normalized().with_codex_catalog())
            .collect(),
    );
    if config.auth_configs.is_empty() {
        config.auth_configs.push(ModelAuthConfig::codex_default());
    }
    if !config
        .auth_configs
        .iter()
        .any(|auth_config| auth_config.id == config.auth_config_id)
    {
        config.auth_config_id = config.auth_configs[0].id.clone();
    }
    let selected = config.selected_auth_config();
    config.apply_auth_config(&selected);
    let model = config.model.trim();
    // Followed catalogs and user-listed API models drop selections no longer in the list.
    let list_is_authoritative = selected.follow_codex
        || selected.provider_kind == ModelProviderKind::AntigravityLogin
        || (selected.provider_kind.uses_api_key() && !selected.models.is_empty());
    if model.is_empty() || (list_is_authoritative && !selected.models.iter().any(|m| m == model)) {
        config.model = selected.default_model;
    } else {
        config.model = normalize_provider_model(selected.provider_kind, &config.model);
    }
}

fn dedupe_auth_configs(configs: Vec<ModelAuthConfig>) -> Vec<ModelAuthConfig> {
    let mut result = Vec::new();
    for config in configs {
        if !result
            .iter()
            .any(|existing: &ModelAuthConfig| existing.id == config.id)
        {
            result.push(config);
        }
    }
    result
}

fn normalize_optional_string(value: Option<&str>, fallback: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn normalize_optional_path(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    for model in models {
        let model = normalize_codex_model(&model);
        if !result.iter().any(|existing: &String| existing == &model) {
            result.push(model);
        }
    }
    if result.is_empty() {
        result = default_codex_models();
    }
    result
}

struct ScopeDocumentEntry {
    scope: SettingsScope,
    path: PathBuf,
    exists: bool,
    content: String,
    document: SettingsDocument,
}

fn load_scope_documents(project_root: &Path) -> Result<Vec<ScopeDocumentEntry>, String> {
    let mut entries = Vec::with_capacity(3);
    for scope in [
        SettingsScope::User,
        SettingsScope::Project,
        SettingsScope::Local,
    ] {
        let path = scope_path(project_root, scope)?;
        let exists = path.exists();
        let (content, document) = if exists {
            let raw = read_limited_utf8(&path, MAX_SETTINGS_FILE_BYTES, "settings file")?;
            let document = match serde_json::from_str::<SettingsDocument>(&raw) {
                Ok(document) => document,
                Err(error) => {
                    eprintln!(
                        "Failed to parse settings file {}: {error}. Falling back to the default template for this scope.",
                        path.display()
                    );
                    if scope == SettingsScope::User {
                        default_user_settings_document()?
                    } else {
                        SettingsDocument::default()
                    }
                }
            };
            (raw, document)
        } else {
            (String::new(), SettingsDocument::default())
        };

        entries.push(ScopeDocumentEntry {
            scope,
            path,
            exists,
            content,
            document,
        });
    }

    Ok(entries)
}

fn default_user_settings_document() -> Result<SettingsDocument, String> {
    serde_json::from_str(USER_SETTINGS_TEMPLATE)
        .map_err(|error| format!("Failed to parse embedded user settings template: {error}"))
}

fn seed_user_settings_if_missing(project_root: &Path) -> Result<(), String> {
    let path = scope_path(project_root, SettingsScope::User)?;
    if path.exists() {
        return Ok(());
    }

    write_user_settings_template(&path)
}

fn write_user_settings_template(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create user settings directory {}: {error}",
                parent.display()
            )
        })?;
    }

    atomic_write_private(path, USER_SETTINGS_TEMPLATE).map_err(|error| {
        format!(
            "Failed to write default user settings file {}: {error}",
            path.display()
        )
    })
}

/// User and local settings can hold provider keys, so they are private to their owner;
/// project settings are meant to be shared.
fn write_scope_file(scope: SettingsScope, path: &Path, payload: String) -> Result<(), String> {
    match scope {
        SettingsScope::User | SettingsScope::Local => atomic_write_private(path, payload),
        SettingsScope::Project => atomic_write(path, payload),
    }
}

/// Tightens settings files written before they were created private, and the user settings
/// directory. Best effort: a failure here must not keep the app from starting.
fn protect_credential_files(project_root: &Path) {
    for scope in [SettingsScope::User, SettingsScope::Local] {
        if let Ok(path) = scope_path(project_root, scope) {
            restrict_to_owner(&path);
            if scope == SettingsScope::User {
                if let Some(directory) = path.parent() {
                    restrict_to_owner(directory);
                }
            }
        }
    }
}

fn scope_path(project_root: &Path, scope: SettingsScope) -> Result<PathBuf, String> {
    match scope {
        SettingsScope::User => Ok(home_dir()?.join(APP_SETTINGS_DIR).join(SETTINGS_FILE_NAME)),
        SettingsScope::Project => Ok(project_root.join(APP_SETTINGS_DIR).join(SETTINGS_FILE_NAME)),
        SettingsScope::Local => Ok(project_root
            .join(APP_SETTINGS_DIR)
            .join(LOCAL_SETTINGS_FILE_NAME)),
    }
}

fn home_dir() -> Result<PathBuf, String> {
    crate::util::home_dir()
        .ok_or_else(|| "Failed to resolve the user home directory for settings scope.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(documents: &[(SettingsScope, serde_json::Value)]) -> ModelConfig {
        let mut config = empty_model_config();
        for (scope, document) in documents {
            let document: SettingsDocument = serde_json::from_value(document.clone()).unwrap();
            apply_model_patch(&mut config, &document.model);
            let mut auth = document.auth;
            if *scope == SettingsScope::Project {
                auth.configs.clear();
            }
            apply_auth_patch(&mut config, &auth);
        }
        normalize_model_selection(&mut config);
        config
    }

    #[test]
    fn auth_configs_coexist_and_catalog_refresh_keeps_active_provider() {
        let mut document = json!({"model":{"model":"gpt-5.5"},"custom":{"preserve":true}});
        let root = document.as_object_mut().unwrap();
        let mut agy = ModelAuthConfig::antigravity_default();
        agy.models = vec!["gemini-pro".into(), "claude-sonnet".into()];
        agy.default_model = "gemini-pro".into();
        update_auth_document(root, ModelAuthConfig::codex_default(), false).unwrap();
        update_auth_document(root, agy.clone(), true).unwrap();
        let config = merged(&[(SettingsScope::User, document.clone())]);
        assert!(config.provider_kind == ModelProviderKind::AntigravityLogin);
        assert_eq!(config.model, "gemini-pro");
        assert_eq!(config.auth_configs.len(), 2);

        let root = document.as_object_mut().unwrap();
        update_auth_document(root, ModelAuthConfig::codex_default(), true).unwrap();
        agy.models.push("gemini-flash".into());
        update_auth_document(root, agy, false).unwrap();
        assert_eq!(document["auth"]["selectedAuthConfigId"], "codex-default");
        assert_eq!(document["custom"]["preserve"], true);
        let config = merged(&[(SettingsScope::User, document)]);
        assert!(config.provider_kind == ModelProviderKind::CodexLogin);
    }

    #[test]
    fn api_key_providers_keep_their_kind_endpoint_and_key() {
        let user = json!({"auth":{"selectedAuthConfigId":"openai","configs":[
            {"id":"openai","name":"OpenAI","providerKind":"open_ai_compatible","baseUrl":"",
             "codexHome":"/tmp/ignored","models":["gpt-4.1"],"defaultModel":"","apiKey":" sk-test "},
            {"id":"claude","name":"","providerKind":"anthropic_messages","baseUrl":"",
             "codexHome":null,"models":[],"defaultModel":"claude-sonnet-4-5"},
            {"id":"gemini","name":"Gemini","providerKind":"gemini_api","baseUrl":"https://proxy.example/v1",
             "codexHome":null,"models":["gemini-2.5-pro"],"defaultModel":"gemini-2.5-pro"}
        ]}});
        let config = merged(&[(SettingsScope::User, user)]);
        assert!(config.provider_kind == ModelProviderKind::OpenAiCompatible);
        assert_eq!(config.base_url, crate::model::types::DEFAULT_OPENAI_URL);
        assert_eq!(config.model, "gpt-4.1");
        assert_eq!(config.api_key, "sk-test");
        assert!(config.codex_home.is_none());
        let claude = &config.auth_configs[1];
        assert!(claude.provider_kind == ModelProviderKind::AnthropicMessages);
        assert_eq!(claude.base_url, crate::model::types::DEFAULT_ANTHROPIC_URL);
        assert_eq!(claude.name, "Anthropic Messages");
        assert_eq!(claude.models, vec!["claude-sonnet-4-5".to_string()]);
        assert_eq!(config.auth_configs[2].base_url, "https://proxy.example/v1");
    }

    #[test]
    fn codex_api_settings_preserve_keys_endpoints_and_explicit_models() {
        for endpoint in ["", "https://gateway.example/v1/responses"] {
            let input: SaveAuthSettingsInput = serde_json::from_value(json!({"config":{
                "id":"codex-api", "name":"", "providerKind":"codex_api_key", "baseUrl":endpoint,
                "codexHome":"/tmp/ignored", "followCodex":true, "models":["gpt-5.5-high", "custom-model"],
                "defaultModel":"gpt-5.5-high", "apiKey":" test-key "
            }})).unwrap();
            let mut document = json!({});
            update_auth_document(
                document.as_object_mut().unwrap(),
                input.config.normalized(),
                input.select,
            )
            .unwrap();
            let project = json!({"auth":{"configs":[{
                "id":"codex-api", "name":"Untrusted", "providerKind":"codex_api_key",
                "baseUrl":"https://attacker.example", "codexHome":null, "models":[], "defaultModel":"other", "apiKey":"other"
            }]}});
            let config = merged(&[
                (SettingsScope::User, document),
                (SettingsScope::Project, project),
            ]);
            assert!(config.provider_kind == ModelProviderKind::CodexApiKey);
            assert_eq!(
                config.base_url,
                if endpoint.is_empty() {
                    crate::model::types::DEFAULT_OPENAI_URL
                } else {
                    endpoint
                }
            );
            assert_eq!(config.model, "gpt-5.5-high");
            assert_eq!(config.api_key, "test-key");
            assert!(config.codex_home.is_none());
            assert!(!config.selected_auth_config().follow_codex);
            assert_eq!(config.selected_auth_config().name, "Codex API (Responses)");
            assert_eq!(
                config.selected_auth_config().models,
                vec!["gpt-5.5-high", "custom-model"]
            );
        }
    }

    #[test]
    fn api_key_env_overrides_the_stored_key() {
        let mut config = ModelAuthConfig::codex_default();
        config.provider_kind = ModelProviderKind::OpenAiCompatible;
        config.id = "openai".into();
        config.api_key = "stored".into();
        config.api_key_env = Some("DAILY_TEST_OPENAI_KEY_7F3".into());
        let config = config.normalized();
        assert_eq!(config.resolved_api_key(), "stored");
        std::env::set_var("DAILY_TEST_OPENAI_KEY_7F3", "from-env");
        assert_eq!(config.resolved_api_key(), "from-env");
        std::env::remove_var("DAILY_TEST_OPENAI_KEY_7F3");
        assert!(ModelAuthConfig::codex_default()
            .resolved_api_key()
            .is_empty());
    }

    #[test]
    fn codex_api_uses_explicit_key_environment_variables() {
        let mut auth = ModelAuthConfig::codex_default();
        auth.provider_kind = ModelProviderKind::CodexApiKey;
        auth.api_key = "stored".into();
        auth.api_key_env = Some("DAILY_TEST_CODEX_API_KEY_5A1".into());
        let auth = auth.normalized();
        assert_eq!(auth.resolved_api_key(), "stored");
        std::env::set_var("DAILY_TEST_CODEX_API_KEY_5A1", "from-env");
        let resolved = auth.resolved_api_key();
        std::env::remove_var("DAILY_TEST_CODEX_API_KEY_5A1");
        assert_eq!(resolved, "from-env");
    }

    #[test]
    fn local_overrides_project_overrides_user() {
        let user = json!({"model":{"temperature":0.1,"maxOutputTokens":1000}});
        let project = json!({"model":{"temperature":0.5,"maxOutputTokens":2000}});
        let local = json!({"model":{"temperature":0.9}});
        let config = merged(&[
            (SettingsScope::User, user),
            (SettingsScope::Project, project),
            (SettingsScope::Local, local),
        ]);
        assert!((config.temperature - 0.9).abs() < f32::EPSILON);
        assert_eq!(config.max_output_tokens, 2000);
    }

    #[test]
    fn project_scope_cannot_redirect_credentials_but_local_can_add_providers() {
        let user = json!({"auth":{"selectedAuthConfigId":"codex-default","configs":[
            {"id":"codex-default","name":"Codex","providerKind":"codex_login",
             "baseUrl":"https://chatgpt.com/backend-api/codex","codexHome":null,"models":[],"defaultModel":"gpt-5.5"}
        ]}});
        let project = json!({"auth":{"selectedAuthConfigId":"codex-default","configs":[
            {"id":"codex-default","name":"Codex","providerKind":"codex_login",
             "baseUrl":"https://attacker.example","codexHome":null,"models":[],"defaultModel":"gpt-5.5"}
        ]}});
        let local = json!({"auth":{"selectedAuthConfigId":"openai","configs":[
            {"id":"openai","name":"OpenAI","providerKind":"open_ai_compatible","baseUrl":"",
             "codexHome":null,"models":["gpt-4.1"],"defaultModel":"gpt-4.1","apiKey":"sk-local"}
        ]}});
        let config = merged(&[
            (SettingsScope::User, user.clone()),
            (SettingsScope::Project, project.clone()),
        ]);
        assert_eq!(config.base_url, crate::model::DEFAULT_CODEX_URL);

        let config = merged(&[
            (SettingsScope::User, user),
            (SettingsScope::Project, project),
            (SettingsScope::Local, local),
        ]);
        assert_eq!(config.auth_configs.len(), 2);
        assert_eq!(config.auth_config_id, "openai");
        assert_eq!(config.api_key, "sk-local");
    }

    #[test]
    fn antigravity_never_inherits_codex_catalog_or_credentials_path() {
        let mut agy = ModelAuthConfig::antigravity_default();
        agy.codex_home = Some("/tmp/codex".into());
        agy.follow_codex = true;
        agy.base_url = "https://untrusted.example".into();
        let agy = agy.normalized().with_codex_catalog();
        assert!(agy.models.is_empty());
        assert!(agy.default_model.is_empty());
        assert!(agy.codex_home.is_none());
        assert!(!agy.follow_codex);
        assert_eq!(agy.base_url, crate::model::types::DEFAULT_ANTIGRAVITY_URL);
    }
}
