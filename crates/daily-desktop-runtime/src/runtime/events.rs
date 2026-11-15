use crate::agent::{
    AgentStore, AppEventEnvelope, AppEventPayload, AppSnapshot, DialogueMessage, TurnDelta,
    TurnPatch, APP_EVENT_NAME,
};
use crate::runtime::context::RuntimeContext;

pub(crate) fn emit_snapshot(app: &RuntimeContext, snapshot: &AppSnapshot) -> Result<(), String> {
    app.emit(
        APP_EVENT_NAME,
        AppEventEnvelope {
            payload: AppEventPayload::Snapshot {
                snapshot: snapshot.clone(),
            },
        },
    )
    .map_err(|error| format!("Failed to emit app state: {error}"))
}

pub(crate) fn emit_turn_delta(app: &RuntimeContext, delta: TurnDelta) -> Result<(), String> {
    app.emit(
        APP_EVENT_NAME,
        AppEventEnvelope {
            payload: AppEventPayload::TurnDelta { delta },
        },
    )
    .map_err(|error| format!("Failed to emit app state: {error}"))
}

pub(crate) fn build_turn_delta(
    state: &crate::state::AppState,
    session_id: u64,
    turn_id: u64,
    turn_patch: TurnPatch,
    message_appends: Vec<DialogueMessage>,
) -> TurnDelta {
    TurnDelta {
        session_id,
        turn_id,
        active_session_id: state.active_session_id,
        runtime: state.runtime_view(),
        session_updated_at: state.session(session_id).map(|session| session.updated_at),
        message_appends,
        turn_patch,
    }
}

pub(crate) fn mutate_state<F>(
    app: &RuntimeContext,
    store: &AgentStore,
    mutation: F,
) -> Result<AppSnapshot, String>
where
    F: FnOnce(&mut crate::state::AppState) -> Result<(), String>,
{
    let snapshot = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let previous = state.clone();
        mutation(&mut state)?;
        if let Err(error) = state.persist(&store.project_root) {
            *state = previous;
            return Err(error);
        }
        state.snapshot()
    };
    emit_snapshot(app, &snapshot)?;
    Ok(snapshot)
}
