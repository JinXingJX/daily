use crate::agent::turn::start::{
    prepare_turn_request, retry_prompt_from_turn, start_continue_turn, start_resume_run,
    start_session_turn,
};
use crate::agent::{AgentStore, AppSnapshot, ResumeRunInput, RunSessionTurnInput};
use crate::limits::MAX_ASYNC_TOOL_RESULT_BYTES;
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::mutate_state;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetrySessionTurnInput {
    pub(crate) session_id: u64,
    pub(crate) message_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinueSessionTurnInput {
    pub(crate) session_id: u64,
    pub(crate) turn_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StopSessionTurnInput {
    pub(crate) session_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginRpcResultInput {
    pub(crate) plugin_id: String,
    pub(crate) request_id: String,
    pub(crate) ok: bool,
    pub(crate) error: Option<String>,
    pub(crate) payload: Option<serde_json::Value>,
}

pub(crate) fn plugin_rpc_result(
    app: RuntimeContext,
    input: PluginRpcResultInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    mutate_state(&app, state, |store| {
        if !input.ok {
            let error = input
                .error
                .clone()
                .unwrap_or_else(|| "Plugin RPC returned an error without details.".into());
            store
                .plugins
                .record_runtime_error(&input.plugin_id, format!("{}: {}", input.request_id, error));
        }
        let _ = input.payload;
        Ok(())
    })
}

pub(crate) fn run_session_turn(
    app: RuntimeContext,
    input: RunSessionTurnInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    start_session_turn(&app, state, input)
}

pub(crate) fn resume_run(
    app: RuntimeContext,
    input: ResumeRunInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    if input.result.len() > MAX_ASYNC_TOOL_RESULT_BYTES {
        return Err(format!(
            "Async tool result exceeds the {MAX_ASYNC_TOOL_RESULT_BYTES} byte limit."
        ));
    }
    if input
        .metadata
        .as_ref()
        .and_then(|value| serde_json::to_vec(value).ok())
        .is_some_and(|value| value.len() > MAX_ASYNC_TOOL_RESULT_BYTES)
    {
        return Err(format!(
            "Async tool metadata exceeds the {MAX_ASYNC_TOOL_RESULT_BYTES} byte limit."
        ));
    }
    start_resume_run(&app, state, input)
}

pub(crate) fn retry_session_turn(
    app: RuntimeContext,
    input: RetrySessionTurnInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    let (retry_text, retry_images) = {
        let mut store = state
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let previous = store.clone();
        let (prompt, images) = store.retry_from_message(input.session_id, input.message_id)?;
        let retry_text = retry_prompt_from_turn(&prompt);
        if let Err(error) = prepare_turn_request(retry_text.clone(), images.clone()) {
            *store = previous;
            return Err(error);
        }
        if let Err(error) = store.persist_session(&state.project_root, input.session_id) {
            *store = previous;
            return Err(error);
        }
        (retry_text, images)
    };

    start_session_turn(
        &app,
        state,
        RunSessionTurnInput {
            session_id: input.session_id,
            text: retry_text,
            images: retry_images,
        },
    )
}

/// Continues a failed or stopped turn from its last checkpoint instead of starting over.
pub(crate) fn continue_session_turn(
    app: RuntimeContext,
    input: ContinueSessionTurnInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    start_continue_turn(&app, state, input.session_id, input.turn_id)
}

pub(crate) fn stop_session_turn(
    app: RuntimeContext,
    input: StopSessionTurnInput,
    state: &AgentStore,
) -> Result<AppSnapshot, String> {
    crate::agent::turn::start::stop_session_turn(&app, state, input.session_id)?;
    let snapshot = state
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?
        .snapshot();
    Ok(snapshot)
}
