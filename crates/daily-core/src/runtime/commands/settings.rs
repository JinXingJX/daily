use crate::agent::{apply_event, AgentEvent, AgentStore, AppSnapshot};
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::mutate_state;
use crate::settings::{
    delete_auth_settings, load_settings, save_auth_settings, save_codex_auth_settings,
    save_settings_file, DeleteAuthSettingsInput, SaveAuthSettingsInput, SaveCodexAuthSettingsInput,
    SaveSettingsFileInput,
};
use crate::{model, settings};

pub(crate) fn apply_loaded_settings(
    app: &RuntimeContext,
    store: &AgentStore,
    loaded: settings::LoadedSettings,
) -> Result<AppSnapshot, String> {
    mutate_state(app, store, |state| {
        apply_event(
            state,
            AgentEvent::SettingsApplied {
                agent_config: loaded.agent,
                model_config: Box::new(loaded.model),
                settings: Box::new(loaded.view),
            },
        );
        Ok(())
    })
}

pub(crate) fn get_app_snapshot(state: &AgentStore) -> Result<AppSnapshot, String> {
    state
        .state
        .lock()
        .map(|state| state.snapshot())
        .map_err(|_| "Application state is unavailable.".to_string())
}

pub(crate) fn reload_settings(
    app: RuntimeContext,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let loaded = load_settings(&state.project_root)?;
    apply_loaded_settings(&app, state, loaded)
}

pub(crate) fn get_codex_auth_status(
    input: model::codex::StartCodexLoginInput,
    state: &AgentStore,
) -> Result<model::codex::CodexAuthStatusView, String> {
    let codex_home = input.codex_home.or_else(|| {
        state
            .state
            .lock()
            .ok()
            .and_then(|state| state.model_config.selected_auth_config().codex_home)
    });
    Ok(model::codex::codex_auth_status(codex_home.as_deref()))
}

pub(crate) fn save_codex_auth_settings_content(
    app: RuntimeContext,
    input: SaveCodexAuthSettingsInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let loaded = save_codex_auth_settings(&state.project_root, input)?;
    apply_loaded_settings(&app, state, loaded)
}

pub(crate) fn save_auth_settings_content(
    app: RuntimeContext,
    input: SaveAuthSettingsInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let loaded = save_auth_settings(&state.project_root, input)?;
    apply_loaded_settings(&app, state, loaded)
}

pub(crate) fn delete_auth_settings_content(
    app: RuntimeContext,
    input: DeleteAuthSettingsInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let loaded = delete_auth_settings(&state.project_root, input)?;
    apply_loaded_settings(&app, state, loaded)
}

pub(crate) async fn get_antigravity_model_catalog(
    state: &AgentStore,
) -> Result<model::antigravity::AntigravityModelCatalog, String> {
    let selected = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.")?
        .model_config
        .selected_auth_config();
    model::antigravity::fetch_model_catalog(
        &state.http,
        if selected.provider_kind == model::ModelProviderKind::AntigravityLogin {
            &selected.default_model
        } else {
            ""
        },
    )
    .await
}

pub(crate) async fn get_antigravity_auth_status() -> model::antigravity::AntigravityAuthStatusView {
    model::antigravity::antigravity_auth_status().await
}

pub(crate) async fn start_antigravity_login(
) -> Result<model::antigravity::AntigravityLoginStartView, String> {
    model::antigravity::start_antigravity_login().await
}

pub(crate) async fn refresh_antigravity_models(
    app: RuntimeContext,
    input: model::antigravity::RefreshAntigravityModelsInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let config = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.")?
        .model_config
        .auth_configs
        .iter()
        .find(|c| {
            c.id == input.auth_config_id
                && c.provider_kind == model::ModelProviderKind::AntigravityLogin
        })
        .cloned()
        .ok_or("Antigravity auth configuration was not found.")?;
    let catalog =
        model::antigravity::fetch_model_catalog(&state.http, &config.default_model).await?;
    // Reload before saving so a concurrent settings edit is not overwritten by the network result.
    let mut latest = load_settings(&state.project_root)?
        .model
        .auth_configs
        .into_iter()
        .find(|c| {
            c.id == config.id && c.provider_kind == model::ModelProviderKind::AntigravityLogin
        })
        .ok_or("Antigravity auth configuration was removed while refreshing.")?;
    latest.models = catalog.models;
    if !latest.models.contains(&latest.default_model) {
        latest.default_model = catalog.default_model;
    }
    let loaded = settings::save_auth_config(&state.project_root, latest, false)?;
    apply_loaded_settings(&app, state, loaded)
}

pub(crate) fn start_codex_login(
    input: model::codex::StartCodexLoginInput,
    state: &AgentStore,
) -> Result<model::codex::CodexLoginStartView, String> {
    let codex_home = input.codex_home.or_else(|| {
        state
            .state
            .lock()
            .ok()
            .and_then(|state| state.model_config.selected_auth_config().codex_home)
    });
    model::codex::start_codex_login(model::codex::StartCodexLoginInput { codex_home })
}

pub(crate) fn save_settings_file_content(
    app: RuntimeContext,
    input: SaveSettingsFileInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let loaded = save_settings_file(&state.project_root, input)?;
    apply_loaded_settings(&app, state, loaded)
}
